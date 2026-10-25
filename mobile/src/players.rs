//! Remote control of LMS players. The gateway lets a user control only the players they
//! are permitted (their own devices and home players an admin granted); anything else is
//! refused as `Forbidden`.

use std::sync::Arc;

use serde_json::Value;

use crate::commands::{self, Placement, RepeatMode, Selection, ShuffleMode};
use crate::edits;
use crate::error::ApiError;
use crate::model::{self, Player, PlayerStatus, Track};
use crate::session::Session;
use crate::transfer::{self, TransferOutcome, TransferProgress};

/// The most players the Rooms view reads at once (docs/plans/wave2-design.md §4.3).
pub const ROOMS_MAX: usize = 20;
/// Status reads in flight at once for the Rooms view.
const ROOMS_PARALLEL: usize = 4;

/// Most tracks [`Players::load_track_list`] puts on a queue at once.
pub const MAX_LIST_LOAD: usize = 2000;

/// The commands that put library tracks `ids` on a queue, in order, playing from `start`:
/// the first id list replaces the queue (`load`, starting there when `start` is in it), the
/// rest are added, then a jump when `start` is further on. Every id is checked, and each
/// list fits the gateway's limit; nothing is sent for a bad list.
pub fn list_load_commands(ids: &[i64], start: u32) -> Result<Vec<commands::Command>, ApiError> {
    if ids.is_empty() || ids.len() > MAX_LIST_LOAD {
        return Err(ApiError::invalid("bad track list"));
    }
    let start = start as usize;
    if start >= ids.len() {
        return Err(ApiError::invalid("bad start"));
    }
    let chunks = commands::chunk_ids(ids, commands::ID_LIST_MAX);
    let first = &chunks[0];
    let in_first = start < first.len();
    let mut out = vec![commands::load_tracks(
        first,
        false,
        Some(if in_first { start as u32 } else { 0 }),
    )?];
    for c in &chunks[1..] {
        out.push(commands::load_tracks(c, true, None)?);
    }
    if !in_first {
        out.push(commands::jump(start as u32));
    }
    Ok(out)
}

/// One room's status for the Rooms view, or why it couldn't be read.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RoomStatus {
    pub player_id: String,
    pub status: Option<PlayerStatus>,
    /// Why the status couldn't be read (a refusal, a disconnected player, the network).
    pub error: Option<String>,
}

/// The players to read for the Rooms view: every id well-formed (else nothing is sent),
/// at most [`ROOMS_MAX`] after repeats are dropped, in the order given.
pub fn room_ids(players: &[String]) -> Result<Vec<String>, ApiError> {
    if players.len() > ROOMS_MAX * 4 {
        return Err(ApiError::invalid("too many players"));
    }
    let mut out: Vec<String> = Vec::new();
    for p in players {
        let id = commands::player_id(p)?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.len() > ROOMS_MAX {
        return Err(ApiError::invalid("at most 20 players at once"));
    }
    Ok(out)
}

#[derive(uniffi::Object)]
pub struct Players {
    session: Arc<Session>,
}

impl Players {
    async fn send(&self, player: &str, cmd: commands::Command) -> Result<Value, ApiError> {
        let p = commands::player_id(player)?;
        self.session.core().rpc(&p, cmd).await
    }

    async fn run(&self, player: &str, cmd: commands::Command) -> Result<(), ApiError> {
        self.send(player, cmd).await.map(|_| ())
    }

    /// The gateway's checked edit (docs/PROTOCOL.md "Checked queue edits").
    async fn checked_edit(&self, player: &str, body: Value) -> Result<Checked, ApiError> {
        let p = commands::player_id(player)?;
        let core = self.session.core();
        let url = core
            .server()
            .endpoint(&format!("v1/players/{p}/queue/edit"));
        core.authed(|token| {
            let req = core
                .http()
                .post(url.clone())
                .bearer_auth(&token)
                .json(&body);
            async move {
                let resp = req.send().await?;
                match resp.status().as_u16() {
                    s if (200..300).contains(&s) => Ok(Checked::Done),
                    409 => Ok(Checked::Changed),
                    404 | 405 => Ok(Checked::Unsupported),
                    _ => Err(crate::transport::error_from_response(resp).await),
                }
            }
        })
        .await
    }

    /// Whether `track_id` is (still) the entry at `index`: the app's own check, for a gateway
    /// without checked edits.
    async fn still_at(&self, player: &str, index: u32, track_id: i64) -> Result<bool, ApiError> {
        let v = self.send(player, commands::queue_list(index, 1)).await?;
        let at = model::tracks_from(&v, "playlist_loop");
        Ok(at
            .first()
            .is_some_and(|t| t.id == Some(track_id) && t.queue_index.is_none_or(|i| i == index)))
    }
}

enum Checked {
    Done,
    Changed,
    Unsupported,
}

/// What saving the queue as a playlist came to (docs/PROTOCOL.md "Save the queue as a
/// playlist").
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SaveQueueOutcome {
    /// Saved: the playlist's id and the name it was saved under.
    Saved { id: i64, name: String },
    /// Not saved: a playlist of that name (ignoring case) exists. Save again with
    /// `replace` to overwrite it.
    Exists { id: i64, name: String },
    /// Not saved: the queue is empty.
    Empty,
}

/// A saved playlist named in the gateway's answer (`{"id":…,"name":…}`).
fn named_playlist(v: &Value) -> Option<(i64, String)> {
    let id = v.get("id")?.as_i64().filter(|n| *n > 0)?;
    let name = model::clean_text(v.get("name")?.as_str()?);
    Some((id, name))
}

/// The gateway's answer to `POST /v1/players/{player}/queue/save`, as an outcome.
fn save_outcome(status: u16, v: &Value) -> Result<SaveQueueOutcome, ApiError> {
    if status == 409 {
        return Ok(match v.get("exists").and_then(named_playlist) {
            Some((id, name)) => SaveQueueOutcome::Exists { id, name },
            None => SaveQueueOutcome::Empty,
        });
    }
    named_playlist(v)
        .map(|(id, name)| SaveQueueOutcome::Saved { id, name })
        .ok_or_else(|| ApiError::bad_response("no playlist in answer"))
}

fn edit_track(track_id: i64) -> Result<i64, ApiError> {
    if track_id == 0 || track_id.unsigned_abs() > 999_999_999_999 {
        return Err(ApiError::invalid("not a track id"));
    }
    Ok(track_id)
}

const MAX_QUEUE_INDEX: u32 = 100_000;

#[uniffi::export(async_runtime = "tokio")]
impl Players {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Players { session })
    }

    /// The players this account may control.
    pub async fn list(&self) -> Result<Vec<Player>, ApiError> {
        let v = self.session.core().rpc("", commands::players()).await?;
        Ok(model::players_from(&v))
    }

    /// Sync groups among the players this account may control.
    pub async fn sync_groups(&self) -> Result<Vec<model::SyncGroup>, ApiError> {
        let v = self.session.core().rpc("", commands::sync_groups()).await?;
        Ok(model::sync_groups_from(&v))
    }

    /// Several players' statuses at once, for the Rooms view: at most [`ROOMS_MAX`]
    /// players; every id is checked before anything is sent. One player's failure is
    /// reported in its entry and doesn't stop the others.
    pub async fn statuses(&self, players: Vec<String>) -> Result<Vec<RoomStatus>, ApiError> {
        use futures_util::StreamExt;
        let ids = room_ids(&players)?;
        let reads = ids.into_iter().map(|id| async move {
            match self.send(&id, commands::status()).await {
                Ok(v) => RoomStatus {
                    player_id: id,
                    status: Some(model::status_from(&v)),
                    error: None,
                },
                Err(e) => RoomStatus {
                    player_id: id,
                    status: None,
                    error: Some(e.to_string()),
                },
            }
        });
        Ok(futures_util::stream::iter(reads)
            .buffered(ROOMS_PARALLEL)
            .collect()
            .await)
    }

    pub async fn status(&self, player: String) -> Result<PlayerStatus, ApiError> {
        Ok(model::status_from(
            &self.send(&player, commands::status()).await?,
        ))
    }

    /// A page of the player's queue (`count` at most 500), with album ids and track and disc
    /// numbers (for gapless markers).
    pub async fn queue(
        &self,
        player: String,
        start: u32,
        count: u32,
    ) -> Result<Vec<Track>, ApiError> {
        let v = self
            .send(&player, commands::queue_list(start, count))
            .await?;
        Ok(model::tracks_from(&v, "playlist_loop"))
    }

    pub async fn play(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::play()).await
    }

    pub async fn set_paused(&self, player: String, paused: bool) -> Result<(), ApiError> {
        self.run(&player, commands::pause(Some(paused))).await
    }

    pub async fn toggle_pause(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::pause(None)).await
    }

    pub async fn stop(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::stop()).await
    }

    pub async fn next(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::skip(true)).await
    }

    pub async fn previous(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::skip(false)).await
    }

    /// Play the queue entry at `index`.
    pub async fn jump_to(&self, player: String, index: u32) -> Result<(), ApiError> {
        self.run(&player, commands::jump(index)).await
    }

    pub async fn seek(&self, player: String, seconds: f64) -> Result<(), ApiError> {
        self.run(&player, commands::seek(seconds)?).await
    }

    /// 0–100.
    pub async fn set_volume(&self, player: String, volume: u32) -> Result<(), ApiError> {
        self.run(&player, commands::volume(volume)).await
    }

    /// Set LMS's "Output level is fixed at 100%" for `player` (on), or give it its volume
    /// control back (off). The gateway allows this only for the caller's own player.
    pub async fn set_fixed_output_level(&self, player: String, on: bool) -> Result<(), ApiError> {
        self.run(&player, commands::fixed_output_level(on)).await
    }

    pub async fn toggle_mute(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::mute_toggle()).await
    }

    pub async fn set_shuffle(&self, player: String, mode: ShuffleMode) -> Result<(), ApiError> {
        self.run(&player, commands::shuffle(mode)).await
    }

    pub async fn set_repeat(&self, player: String, mode: RepeatMode) -> Result<(), ApiError> {
        self.run(&player, commands::repeat(mode)).await
    }

    pub async fn set_power(&self, player: String, on: bool) -> Result<(), ApiError> {
        self.run(&player, commands::power(on)).await
    }

    /// Set the sleep timer: stop (and turn off) after `seconds`, at most a day; 0 cancels.
    pub async fn set_sleep(&self, player: String, seconds: u32) -> Result<(), ApiError> {
        self.run(&player, commands::sleep(seconds)).await
    }

    /// The queue from `from` on, with durations and albums only (for "sleep at the end of
    /// this album or the queue"): up to 500 entries.
    pub async fn queue_times(&self, player: String, from: u32) -> Result<Vec<Track>, ApiError> {
        let v = self.send(&player, commands::queue_times(from)).await?;
        Ok(model::tracks_from(&v, "playlist_loop"))
    }

    /// Take the queue entry at `index` out.
    pub async fn remove_from_queue(&self, player: String, index: u32) -> Result<(), ApiError> {
        self.run(&player, commands::remove_from_queue(index)).await
    }

    /// Take the entry at `index` out, only if it is still `track_id`: the gateway checks and
    /// removes in one step (an older gateway: checked here first). False, with nothing
    /// changed, if another track is there now.
    pub async fn remove_if_still(
        &self,
        player: String,
        index: u32,
        track_id: i64,
    ) -> Result<bool, ApiError> {
        let track = edit_track(track_id)?;
        if index > MAX_QUEUE_INDEX {
            return Err(ApiError::invalid("not a queue position"));
        }
        let body = serde_json::json!({"op": "remove", "index": index, "track": track});
        match self.checked_edit(&player, body).await? {
            Checked::Done => Ok(true),
            Checked::Changed => Ok(false),
            Checked::Unsupported => {
                if !self.still_at(&player, index, track).await? {
                    return Ok(false);
                }
                self.run(&player, commands::remove_from_queue(index))
                    .await?;
                Ok(true)
            }
        }
    }

    /// Move the entry at `from` to `to`, only if it is still `track_id` (as
    /// [`Self::remove_if_still`]).
    pub async fn move_if_still(
        &self,
        player: String,
        from: u32,
        to: u32,
        track_id: i64,
    ) -> Result<bool, ApiError> {
        let track = edit_track(track_id)?;
        if from > MAX_QUEUE_INDEX || to > MAX_QUEUE_INDEX || from == to {
            return Err(ApiError::invalid("not a queue move"));
        }
        let body = serde_json::json!({"op": "move", "index": from, "to": to, "track": track});
        match self.checked_edit(&player, body).await? {
            Checked::Done => Ok(true),
            Checked::Changed => Ok(false),
            Checked::Unsupported => {
                if !self.still_at(&player, from, track).await? {
                    return Ok(false);
                }
                self.run(&player, commands::move_in_queue(from, to)).await?;
                Ok(true)
            }
        }
    }

    /// Move the queue entry at `from` so that it ends up at `to`.
    pub async fn move_in_queue(&self, player: String, from: u32, to: u32) -> Result<(), ApiError> {
        self.run(&player, commands::move_in_queue(from, to)).await
    }

    pub async fn clear_queue(&self, player: String) -> Result<(), ApiError> {
        self.run(&player, commands::clear_queue()).await
    }

    /// Undo a removal: put library track `track_id` back at `index`. Fails (`Server`, 409)
    /// if the queue changed meanwhile; the track is then at the end.
    pub async fn restore_to_queue(
        &self,
        player: String,
        track_id: i64,
        index: u32,
    ) -> Result<(), ApiError> {
        edits::restore(
            self.session.as_ref(),
            &player,
            track_id,
            index,
            edits::RESTORE_CHECKS,
            edits::RESTORE_CHECK_INTERVAL,
        )
        .await
    }

    /// Put `other` in `current`'s sync group (`on`), or take `other` out of its group.
    pub async fn set_synced(
        &self,
        current: String,
        other: String,
        on: bool,
    ) -> Result<(), ApiError> {
        let (player, cmd) = edits::sync_toggle(&current, &other, on)?;
        self.run(&player, cmd).await
    }

    /// Whether this account may save the queue as a playlist here: an admin, on a gateway
    /// that offers it (`GET /v1/me`: role and `features.save_queue`).
    pub async fn can_save_queue(&self) -> Result<bool, ApiError> {
        let (_, v) = crate::me::exchange(self.session.core(), "GET", "v1/me", None).await?;
        Ok(v["user"]["role"].as_str() == Some("admin")
            && v["features"]["save_queue"].as_bool() == Some(true))
    }

    /// Save the player's queue as a playlist called `name` (admins only; the name is
    /// checked here first, by the rules the gateway enforces). A playlist of that name is
    /// overwritten only with `replace`.
    pub async fn save_queue(
        &self,
        player: String,
        name: String,
        replace: bool,
    ) -> Result<SaveQueueOutcome, ApiError> {
        let p = commands::player_id(&player)?;
        let name = anywhere_core::names::playlist_name(&name).map_err(ApiError::invalid)?;
        let body = serde_json::json!({ "name": name, "replace": replace });
        let (status, v) = crate::me::exchange(
            self.session.core(),
            "POST",
            &format!("v1/players/{p}/queue/save"),
            Some(&body),
        )
        .await?;
        save_outcome(status, &v)
    }

    /// Move playback from `source` to `target` (docs/PROTOCOL.md › Transfer): group then
    /// leave, or copy the list if LMS won't group them. Both must be players this account
    /// may control; the gateway refuses anything else.
    pub async fn transfer(
        &self,
        source: String,
        target: String,
        progress: Arc<dyn TransferProgress>,
    ) -> Result<TransferOutcome, ApiError> {
        transfer::transfer(
            self.session.as_ref(),
            &source,
            &target,
            transfer::Pace::default(),
            &|stage| progress.on_stage(stage),
        )
        .await
    }

    /// Replace the queue with library tracks `ids`, in order, and play from `start` (an
    /// index into them): "Continue on" for the offline queue (SPEC §8.4). Long lists go in
    /// several commands, each through the gateway's policy (see [`list_load_commands`]).
    pub async fn load_track_list(
        &self,
        player: String,
        ids: Vec<i64>,
        start: u32,
    ) -> Result<(), ApiError> {
        let player = commands::player_id(&player)?;
        for c in list_load_commands(&ids, start)? {
            self.run(&player, c).await?;
        }
        Ok(())
    }

    /// Play a favourite (a station, track or app item) now, next, or at the end.
    pub async fn play_favourite(
        &self,
        player: String,
        item_id: String,
        placement: Placement,
    ) -> Result<(), ApiError> {
        self.run(&player, commands::play_favourite(&item_id, placement)?)
            .await
    }

    /// Play an album, track, artist, playlist, genre, year or folder now, next, or at the
    /// end of the queue. With
    /// `PlayNow`, `play_index` starts at that track of the selection.
    pub async fn enqueue(
        &self,
        player: String,
        selection: Selection,
        placement: Placement,
        play_index: Option<u32>,
    ) -> Result<(), ApiError> {
        self.run(
            &player,
            commands::enqueue(selection, placement, play_index)?,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: usize) -> String {
        format!("02:00:00:00:00:{n:02x}")
    }

    #[test]
    fn a_list_load_keeps_order_and_starts_where_asked() {
        // Short: one load, starting at the track.
        let c = list_load_commands(&[11, 12, 13], 1).unwrap();
        assert_eq!(
            c,
            vec![commands::load_tracks(&[11, 12, 13], false, Some(1)).unwrap()]
        );
        // Long: the first list loads, the rest are added in order, then a jump past the first.
        let ids: Vec<i64> = (100_000..100_100).collect();
        let chunks = commands::chunk_ids(&ids, commands::ID_LIST_MAX);
        assert!(chunks.len() > 2);
        let c = list_load_commands(&ids, 90).unwrap();
        assert_eq!(c.len(), chunks.len() + 1);
        assert_eq!(
            c[0],
            commands::load_tracks(&chunks[0], false, Some(0)).unwrap()
        );
        for (i, ch) in chunks[1..].iter().enumerate() {
            assert_eq!(c[i + 1], commands::load_tracks(ch, true, None).unwrap());
        }
        assert_eq!(*c.last().unwrap(), commands::jump(90));
        // Starting inside the first list: no jump.
        let c = list_load_commands(&ids, 3).unwrap();
        assert_eq!(c.len(), chunks.len());
        assert_eq!(
            c[0],
            commands::load_tracks(&chunks[0], false, Some(3)).unwrap()
        );
    }

    #[test]
    fn a_bad_list_load_sends_nothing() {
        assert!(list_load_commands(&[], 0).is_err());
        assert!(list_load_commands(&[1, 2], 2).is_err());
        assert!(list_load_commands(&[1, 0, 3], 0).is_err());
        assert!(list_load_commands(&[1, -5], 0).is_err());
        assert!(list_load_commands(&[i64::MAX], 0).is_err());
        let many = vec![1i64; MAX_LIST_LOAD + 1];
        assert!(list_load_commands(&many, 0).is_err());
        assert!(list_load_commands(&vec![1i64; MAX_LIST_LOAD], 0).is_ok());
    }

    #[test]
    fn room_ids_are_checked_deduplicated_and_capped() {
        let ids: Vec<String> = (0..20).map(id).collect();
        assert_eq!(room_ids(&ids).unwrap(), ids);
        // Repeats count once, case-insensitively, in the order first given.
        let twice = vec![id(1), id(2), id(1).to_uppercase(), id(2)];
        assert_eq!(room_ids(&twice).unwrap(), vec![id(1), id(2)]);
        assert!(room_ids(&[]).unwrap().is_empty());
        // Twenty-one players: refused.
        let many: Vec<String> = (0..21).map(id).collect();
        assert!(matches!(
            room_ids(&many),
            Err(ApiError::InvalidInput { .. })
        ));
        // A huge list is refused before anything is looked at.
        let huge = vec![id(1); 10_000];
        assert!(matches!(
            room_ids(&huge),
            Err(ApiError::InvalidInput { .. })
        ));
    }

    #[test]
    fn save_answers_are_read() {
        use serde_json::json;
        assert_eq!(
            save_outcome(200, &json!({"id": 12, "name": "Road trip"})),
            Ok(SaveQueueOutcome::Saved {
                id: 12,
                name: "Road trip".into()
            })
        );
        assert_eq!(
            save_outcome(
                409,
                &json!({"error": "exists", "exists": {"id": 5, "name": "Road Trip"}})
            ),
            Ok(SaveQueueOutcome::Exists {
                id: 5,
                name: "Road Trip".into()
            })
        );
        assert_eq!(
            save_outcome(409, &json!({"error": "the queue is empty"})),
            Ok(SaveQueueOutcome::Empty)
        );
        for junk in [
            json!(null),
            json!({}),
            json!({"id": "12", "name": "x"}),
            json!({"id": 0, "name": "x"}),
            json!({"id": 12}),
        ] {
            assert!(save_outcome(200, &junk).is_err(), "{junk}");
        }
    }

    #[test]
    fn one_bad_id_refuses_the_lot() {
        for bad in [
            "",
            "../../x",
            "aa:bb:cc:dd:ee",
            "AA-BB-CC-DD-EE-01",
            "02:00:00:00:00:0g",
        ] {
            let ids = vec![id(1), bad.to_string()];
            assert!(
                matches!(room_ids(&ids), Err(ApiError::InvalidInput { .. })),
                "{bad:?} accepted"
            );
        }
    }
}
