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
}

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

    pub async fn status(&self, player: String) -> Result<PlayerStatus, ApiError> {
        Ok(model::status_from(
            &self.send(&player, commands::status()).await?,
        ))
    }

    /// A page of the player's queue (`count` at most 500).
    pub async fn queue(
        &self,
        player: String,
        start: u32,
        count: u32,
    ) -> Result<Vec<Track>, ApiError> {
        let v = self.send(&player, commands::queue(start, count)).await?;
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
