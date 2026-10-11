//! Transfer: move what one player is playing (its queue, position and play state) to
//! another, start it there and stop the first. The recipe is in docs/PROTOCOL.md
//! ("Transfer"), the same as the web app's (`web/app/src/lib/lms/transfer.ts`).
//!
//! 1. Group, then leave (preferred). `<source> sync <target>` makes the target join the
//!    source's group and take its queue (radio and app items included), in step;
//!    `<source> sync -` then takes the source out: it stops, the target carries on.
//! 2. Copy the list (fallback, when LMS won't group the two): load the source's library
//!    tracks on the target by id, jump to the track and time, then stop the source.
//!
//! Planning is pure (status in, steps out) and unit-tested; [`transfer`] runs a plan
//! through anything that implements [`Rpc`], so every step goes through the gateway's
//! policy: the caller must be allowed to control both players.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::commands::{self, Command, RepeatMode, ShuffleMode};
use crate::error::ApiError;
use crate::model::float_of;
use crate::session::Session;

/// The gateway pages at most this many queue entries at once; longer queues can't be copied.
pub const MAX_COPY: u32 = 500;
/// How often (and how far apart) the target is checked for having joined the group.
pub const GROUP_CHECKS: u32 = 6;
pub const GROUP_CHECK_INTERVAL: Duration = Duration::from_millis(250);

/// How playback was moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransferMethod {
    /// Grouped, then the source left (everything moves, radio included).
    Group,
    /// The list of library tracks was copied.
    Copy,
}

/// Why a transfer wasn't done (nothing was changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransferRefusal {
    /// The source and target are the same player.
    Same,
    /// The source's queue is empty: nothing to move.
    Empty,
    /// LMS wouldn't group them, and the queue has radio or app items, which can't be copied.
    Remote,
    /// LMS wouldn't group them, and the queue is too long to copy (over 500 entries).
    TooLong,
    /// LMS wouldn't group them, and the list couldn't be copied either.
    NotGrouped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransferOutcome {
    Moved { method: TransferMethod },
    Refused { reason: TransferRefusal },
}

/// Where a transfer has got to, for a progress line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransferStage {
    /// Reading what the source plays.
    Reading,
    /// Asking LMS to group the two.
    Grouping,
    /// Starting the target and stopping the source.
    Starting,
    /// LMS wouldn't group them: copying the list instead.
    Copying,
}

/// Told about each stage of a transfer (called on a background thread).
#[uniffi::export(with_foreign)]
pub trait TransferProgress: Send + Sync {
    fn on_stage(&self, stage: TransferStage);
}

/// Which player a step is sent to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Source,
    Target,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Send(Side, Command),
    /// Check that the target has joined the source's group before going on.
    VerifyGrouped,
}

pub type Plan = Result<Vec<Step>, TransferRefusal>;

/// What the source says about itself (from `status - 1`), read leniently.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceStatus {
    pub mode: Option<String>,
    pub time: Option<f64>,
    pub tracks: u32,
    pub index: u32,
    pub repeat: u32,
}

/// A count LMS sends as a number or a string; anything else (or negative) is 0.
fn count(v: &Value) -> u32 {
    float_of(v)
        .filter(|n| *n >= 0.0)
        .map(|n| n.floor().min(f64::from(u32::MAX)) as u32)
        .unwrap_or(0)
}

impl SourceStatus {
    pub fn from_lms(v: &Value) -> SourceStatus {
        SourceStatus {
            mode: v["mode"].as_str().map(str::to_owned),
            time: float_of(&v["time"]),
            tracks: count(&v["playlist_tracks"]),
            index: count(&v["playlist_cur_index"]),
            repeat: count(&v["playlist repeat"]),
        }
    }
}

/// How the target should end up playing, given how the source was.
fn start_steps(mode: Option<&str>) -> Vec<Step> {
    match mode {
        Some("pause") => vec![Step::Send(Side::Target, commands::pause(Some(false)))],
        Some("play") => vec![],
        // Stopped: start it, as asked.
        _ => vec![Step::Send(Side::Target, commands::play())],
    }
}

fn ids(source: &str, target: &str) -> Result<(String, String), ApiError> {
    Ok((commands::player_id(source)?, commands::player_id(target)?))
}

/// Recipe 1: group, then leave. Malformed player ids are an error: nothing is sent for them.
pub fn plan_group(source: &str, target: &str, s: Option<&SourceStatus>) -> Result<Plan, ApiError> {
    let (from, to) = ids(source, target)?;
    if from == to {
        return Ok(Err(TransferRefusal::Same));
    }
    let Some(s) = s.filter(|s| s.tracks > 0) else {
        return Ok(Err(TransferRefusal::Empty));
    };
    let mut steps = vec![
        // A target that is off joins the group but stays silent: switch it on first.
        Step::Send(Side::Target, commands::power(true)),
        // The target joins the source's group (leaving any it was in) and takes its queue.
        Step::Send(Side::Source, commands::sync_with(&to)?),
        Step::VerifyGrouped,
        // The source leaves: it stops, and the target carries on.
        Step::Send(Side::Source, commands::unsync()),
        Step::Send(Side::Source, commands::stop()),
    ];
    steps.extend(start_steps(s.mode.as_deref()));
    Ok(Ok(steps))
}

/// A queue entry's library id, or none for radio, app and other remote items (LMS gives
/// those negative ids, or marks them `remote`).
fn library_id(e: &Value) -> Option<i64> {
    let remote = matches!(&e["remote"], Value::Number(n) if n.as_i64() == Some(1))
        || e["remote"].as_str() == Some("1");
    if remote {
        return None;
    }
    let id = match &e["id"] {
        Value::Number(n) => n.as_i64()?,
        Value::String(s)
            if (1..=15).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            s.parse().ok()?
        }
        _ => return None,
    };
    commands::id(id).ok()
}

/// Recipe 2: copy the list. `queue` is the source's whole queue in play order
/// (`status 0 N`), `s` its status read just before (with `time` advanced to now if playing).
pub fn plan_copy(
    source: &str,
    target: &str,
    s: Option<&SourceStatus>,
    queue: &[Value],
) -> Result<Plan, ApiError> {
    let (from, to) = ids(source, target)?;
    if from == to {
        return Ok(Err(TransferRefusal::Same));
    }
    let Some(s) = s.filter(|s| s.tracks > 0) else {
        return Ok(Err(TransferRefusal::Empty));
    };
    let total = s.tracks as usize;
    if s.tracks > MAX_COPY || queue.len() < total {
        return Ok(Err(TransferRefusal::TooLong));
    }
    let mut list = Vec::with_capacity(total);
    for e in &queue[..total] {
        match library_id(e) {
            Some(id) => list.push(id),
            None => return Ok(Err(TransferRefusal::Remote)),
        }
    }
    let index = (s.index as usize).min(list.len() - 1);
    let repeat = match s.repeat {
        1 => RepeatMode::One,
        2 => RepeatMode::All,
        _ => RepeatMode::Off,
    };
    let chunks = commands::chunk_ids(&list, commands::ID_LIST_MAX);
    let first = &chunks[0];
    let start = if index < first.len() { index } else { 0 };
    let mut steps = vec![
        Step::Send(Side::Target, commands::power(true)),
        // The list is copied in play order, so the target must not shuffle it again.
        Step::Send(Side::Target, commands::shuffle(ShuffleMode::Off)),
        Step::Send(Side::Target, commands::repeat(repeat)),
        Step::Send(
            Side::Target,
            commands::load_tracks(first, false, Some(start as u32))?,
        ),
    ];
    for c in &chunks[1..] {
        steps.push(Step::Send(
            Side::Target,
            commands::load_tracks(c, true, None)?,
        ));
    }
    if index >= first.len() {
        steps.push(Step::Send(Side::Target, commands::jump(index as u32)));
    }
    // A second or two in is as good as the start.
    if let Some(t) = s.time.filter(|t| *t >= 2.0) {
        steps.push(Step::Send(Side::Target, seek_whole(t)));
    }
    steps.push(Step::Send(Side::Source, commands::stop()));
    Ok(Ok(steps))
}

/// `time <whole seconds>`, as the web app sends it.
fn seek_whole(t: f64) -> Command {
    vec![json!("time"), json!(t.floor().max(0.0) as u64)]
}

/// Has the target joined `source`'s group, with the same number of tracks?
pub fn grouped(source: &str, target_status: &Value, tracks: u32) -> bool {
    let me = source.to_ascii_lowercase();
    let member = [&target_status["sync_master"], &target_status["sync_slaves"]]
        .into_iter()
        .filter_map(Value::as_str)
        .filter(|s| s.len() <= 4096)
        .flat_map(|s| s.split(','))
        .any(|id| id.trim().to_ascii_lowercase() == me);
    member && count(&target_status["playlist_tracks"]) == tracks
}

/// Sends a command for a player and returns LMS's answer.
pub(crate) trait Rpc: Sync {
    fn call(
        &self,
        player: &str,
        cmd: Command,
    ) -> impl std::future::Future<Output = Result<Value, ApiError>> + Send;
}

impl Rpc for Session {
    async fn call(&self, player: &str, cmd: Command) -> Result<Value, ApiError> {
        self.core().rpc(player, cmd).await
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Pace {
    pub checks: u32,
    pub interval: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Pace {
            checks: GROUP_CHECKS,
            interval: GROUP_CHECK_INTERVAL,
        }
    }
}

fn is_join(cmd: &Command) -> bool {
    cmd.first() == Some(&json!("sync")) && cmd.get(1) != Some(&json!("-"))
}

/// Move playback from `source` to `target`. Tries recipe 1, then recipe 2 if LMS won't
/// group them. On an error after the target joined the group, the target is taken out
/// again (best effort), so the two are never left grouped by accident.
pub(crate) async fn transfer<R: Rpc>(
    rpc: &R,
    source: &str,
    target: &str,
    pace: Pace,
    progress: &(dyn Fn(TransferStage) + Sync),
) -> Result<TransferOutcome, ApiError> {
    let (from, to) = ids(source, target)?;
    if from == to {
        return Ok(TransferOutcome::Refused {
            reason: TransferRefusal::Same,
        });
    }
    let mut joined = false;
    let r = run(rpc, &from, &to, pace, progress, &mut joined).await;
    if r.is_err() && joined {
        let _ = rpc.call(&to, commands::unsync()).await;
    }
    r
}

async fn run<R: Rpc>(
    rpc: &R,
    from: &str,
    to: &str,
    pace: Pace,
    progress: &(dyn Fn(TransferStage) + Sync),
    joined: &mut bool,
) -> Result<TransferOutcome, ApiError> {
    progress(TransferStage::Reading);
    let s = SourceStatus::from_lms(&rpc.call(from, commands::status()).await?);
    let steps = match plan_group(from, to, Some(&s))? {
        Ok(steps) => steps,
        Err(reason) => return Ok(TransferOutcome::Refused { reason }),
    };
    progress(TransferStage::Grouping);
    let mut grouped_ok = true;
    for step in steps {
        match step {
            Step::VerifyGrouped => {
                if !wait_grouped(rpc, from, to, s.tracks, pace).await? {
                    grouped_ok = false;
                    break;
                }
                progress(TransferStage::Starting);
            }
            Step::Send(side, cmd) => {
                if is_join(&cmd) {
                    *joined = true;
                }
                rpc.call(if side == Side::Source { from } else { to }, cmd)
                    .await?;
            }
        }
    }
    if grouped_ok {
        return Ok(TransferOutcome::Moved {
            method: TransferMethod::Group,
        });
    }

    // LMS didn't group them: take the target out again, and copy the list instead.
    let _ = rpc.call(to, commands::unsync()).await;
    *joined = false;
    progress(TransferStage::Copying);
    let fresh = SourceStatus::from_lms(&rpc.call(from, commands::status()).await?);
    let read_at = Instant::now();
    let q = rpc.call(from, commands::queue(0, MAX_COPY)).await?;
    let queue = q["playlist_loop"].as_array().cloned().unwrap_or_default();
    let time = match (fresh.mode.as_deref(), fresh.time) {
        (Some("play"), Some(t)) => Some(t + read_at.elapsed().as_secs_f64()),
        (_, t) => t,
    };
    let plan = plan_copy(from, to, Some(&SourceStatus { time, ..fresh }), &queue)?;
    let steps = match plan {
        Ok(steps) => steps,
        Err(reason @ (TransferRefusal::Remote | TransferRefusal::TooLong)) => {
            return Ok(TransferOutcome::Refused { reason });
        }
        Err(_) => {
            return Ok(TransferOutcome::Refused {
                reason: TransferRefusal::NotGrouped,
            });
        }
    };
    for step in steps {
        if let Step::Send(side, cmd) = step {
            rpc.call(if side == Side::Source { from } else { to }, cmd)
                .await?;
        }
    }
    Ok(TransferOutcome::Moved {
        method: TransferMethod::Copy,
    })
}

async fn wait_grouped<R: Rpc>(
    rpc: &R,
    from: &str,
    to: &str,
    tracks: u32,
    pace: Pace,
) -> Result<bool, ApiError> {
    for i in 0..pace.checks {
        if grouped(from, &rpc.call(to, commands::status()).await?, tracks) {
            return Ok(true);
        }
        if i + 1 < pace.checks {
            tokio::time::sleep(pace.interval).await;
        }
    }
    Ok(false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    const S: &str = "aa:bb:cc:dd:ee:01";
    const T: &str = "aa:bb:cc:dd:ee:02";

    fn status(mode: &str, tracks: u32) -> SourceStatus {
        SourceStatus {
            mode: Some(mode.into()),
            time: Some(31.5),
            tracks,
            index: 0,
            repeat: 0,
        }
    }

    fn sends(plan: &[Step]) -> Vec<(Side, Value)> {
        plan.iter()
            .filter_map(|s| match s {
                Step::Send(side, c) => Some((*side, Value::from(c.clone()))),
                Step::VerifyGrouped => None,
            })
            .collect()
    }

    #[test]
    fn group_plan_follows_the_recipe() {
        let plan = plan_group(S, T, Some(&status("play", 3))).unwrap().unwrap();
        assert_eq!(
            plan,
            vec![
                Step::Send(Side::Target, commands::power(true)),
                Step::Send(Side::Source, vec![json!("sync"), json!(T)]),
                Step::VerifyGrouped,
                Step::Send(Side::Source, commands::unsync()),
                Step::Send(Side::Source, commands::stop()),
            ]
        );
        // The direction matters: the source names the target, never the other way round.
        assert!(
            !sends(&plan).contains(&(Side::Target, json!(["sync", S]))),
            "the target must never sync to the source"
        );
        // Paused: the target is unpaused; stopped: started.
        let paused = plan_group(S, T, Some(&status("pause", 3)))
            .unwrap()
            .unwrap();
        assert_eq!(
            paused.last(),
            Some(&Step::Send(Side::Target, commands::pause(Some(false))))
        );
        let stopped = plan_group(S, T, Some(&status("stop", 3))).unwrap().unwrap();
        assert_eq!(
            stopped.last(),
            Some(&Step::Send(Side::Target, commands::play()))
        );
        // Upper-case ids are the same player.
        assert_eq!(
            plan_group(S, &T.to_uppercase(), Some(&status("play", 1)))
                .unwrap()
                .unwrap()[1],
            Step::Send(Side::Source, vec![json!("sync"), json!(T)])
        );
    }

    #[test]
    fn refusals_send_nothing() {
        assert_eq!(
            plan_group(S, &S.to_uppercase(), Some(&status("play", 3))).unwrap(),
            Err(TransferRefusal::Same)
        );
        assert_eq!(
            plan_group(S, T, Some(&status("play", 0))).unwrap(),
            Err(TransferRefusal::Empty)
        );
        assert_eq!(plan_group(S, T, None).unwrap(), Err(TransferRefusal::Empty));
        for bad in [
            "",
            "-",
            "aa:bb:cc:dd:ee",
            "../x",
            "aa:bb:cc:dd:ee:0g",
            "all",
        ] {
            assert!(plan_group(bad, T, Some(&status("play", 1))).is_err());
            assert!(plan_group(S, bad, Some(&status("play", 1))).is_err());
            assert!(plan_copy(S, bad, Some(&status("play", 1)), &[]).is_err());
        }
    }

    #[test]
    fn source_status_is_read_leniently() {
        let s = SourceStatus::from_lms(&json!({
            "mode": "pause", "time": "12.5", "playlist_tracks": "4",
            "playlist_cur_index": 2, "playlist repeat": "2"
        }));
        assert_eq!(
            s,
            SourceStatus {
                mode: Some("pause".into()),
                time: Some(12.5),
                tracks: 4,
                index: 2,
                repeat: 2
            }
        );
        let junk = SourceStatus::from_lms(&json!({
            "playlist_tracks": -3, "playlist_cur_index": "x", "time": "NaN", "mode": 5
        }));
        assert_eq!(junk, SourceStatus::default());
        assert_eq!(
            SourceStatus::from_lms(&json!(null)),
            SourceStatus::default()
        );
        assert_eq!(
            SourceStatus::from_lms(&json!([1, 2])),
            SourceStatus::default()
        );
    }

    fn queue(ids: impl IntoIterator<Item = i64>) -> Vec<Value> {
        ids.into_iter().map(|i| json!({"id": i})).collect()
    }

    #[test]
    fn copy_plan_follows_the_recipe() {
        let s = SourceStatus {
            mode: Some("play".into()),
            time: Some(83.7),
            tracks: 3,
            index: 1,
            repeat: 2,
        };
        let plan = plan_copy(S, T, Some(&s), &queue([11, 12, 13]))
            .unwrap()
            .unwrap();
        assert_eq!(
            sends(&plan),
            vec![
                (Side::Target, json!(["power", 1])),
                (Side::Target, json!(["playlist", "shuffle", 0])),
                (Side::Target, json!(["playlist", "repeat", 2])),
                (
                    Side::Target,
                    json!([
                        "playlistcontrol",
                        "cmd:load",
                        "track_id:11,12,13",
                        "play_index:1"
                    ])
                ),
                (Side::Target, json!(["time", 83])),
                (Side::Source, json!(["stop"])),
            ]
        );
        // Near the start: no seek.
        let early = SourceStatus {
            time: Some(1.9),
            ..s.clone()
        };
        assert!(
            !sends(
                &plan_copy(S, T, Some(&early), &queue([11, 12, 13]))
                    .unwrap()
                    .unwrap()
            )
            .iter()
            .any(|(_, c)| c[0] == "time")
        );
        // A repeat value LMS shouldn't send is treated as off.
        let odd = SourceStatus {
            repeat: 7,
            ..s.clone()
        };
        assert_eq!(
            sends(
                &plan_copy(S, T, Some(&odd), &queue([11, 12, 13]))
                    .unwrap()
                    .unwrap()
            )[2],
            (Side::Target, json!(["playlist", "repeat", 0]))
        );
    }

    #[test]
    fn long_queues_are_copied_in_chunks_and_the_index_is_kept() {
        let ids: Vec<i64> = (0..60).map(|i| 9_000_000 + i).collect();
        let s = SourceStatus {
            mode: Some("play".into()),
            time: Some(0.0),
            tracks: 60,
            index: 40,
            repeat: 0,
        };
        let plan = plan_copy(S, T, Some(&s), &queue(ids.clone()))
            .unwrap()
            .unwrap();
        let sent = sends(&plan);
        let loads: Vec<&Value> = sent
            .iter()
            .map(|(_, c)| c)
            .filter(|c| c[0] == "playlistcontrol")
            .collect();
        assert!(loads.len() > 1);
        assert_eq!(loads[0][1], "cmd:load");
        // Index 40 isn't in the first list: start at 0, then jump.
        assert_eq!(loads[0][3], "play_index:0");
        assert!(loads[1..].iter().all(|c| c[1] == "cmd:add"));
        let copied: Vec<i64> = loads
            .iter()
            .flat_map(|c| {
                c[2].as_str()
                    .unwrap()
                    .strip_prefix("track_id:")
                    .unwrap()
                    .split(',')
                    .map(|v| v.parse::<i64>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(copied, ids);
        for c in &loads {
            assert!(c[2].as_str().unwrap().len() <= "track_id:".len() + commands::ID_LIST_MAX);
        }
        assert!(sent.contains(&(Side::Target, json!(["playlist", "index", 40]))));
        // An index past the end is clamped to the last track.
        let past = SourceStatus {
            index: 99,
            tracks: 2,
            ..s
        };
        assert_eq!(
            sends(
                &plan_copy(S, T, Some(&past), &queue([5, 6]))
                    .unwrap()
                    .unwrap()
            )[3]
            .1[3],
            "play_index:1"
        );
    }

    #[test]
    fn copy_refuses_what_it_cannot_move() {
        let s = |tracks| SourceStatus {
            mode: Some("play".into()),
            time: None,
            tracks,
            index: 0,
            repeat: 0,
        };
        let refusal =
            |st: SourceStatus, q: Vec<Value>| plan_copy(S, T, Some(&st), &q).unwrap().err();
        assert_eq!(refusal(s(0), vec![]), Some(TransferRefusal::Empty));
        assert_eq!(
            refusal(s(501), queue(1..=501)),
            Some(TransferRefusal::TooLong)
        );
        // Fewer entries came back than LMS says there are.
        assert_eq!(refusal(s(3), queue([1, 2])), Some(TransferRefusal::TooLong));
        for remote in [
            json!({"id": -108_699_300_821_096_i64}),
            json!({"id": 7, "remote": 1}),
            json!({"id": 7, "remote": "1"}),
            json!({"title": "No id"}),
            json!({"id": "12,34"}),
            json!({"id": "http://radio.example.com/stream"}),
            json!({"id": 0}),
            json!("not an object"),
        ] {
            assert_eq!(
                refusal(s(2), vec![json!({"id": 1}), remote.clone()]),
                Some(TransferRefusal::Remote),
                "{remote}"
            );
        }
        // String ids of digits are fine (LMS sometimes sends them so).
        assert!(refusal(s(2), vec![json!({"id": "11"}), json!({"id": 12})]).is_none());
        // Extra entries beyond the count are ignored.
        assert!(refusal(s(1), vec![json!({"id": 1}), json!({"id": -5})]).is_none());
        assert_eq!(
            plan_copy(S, S, Some(&s(1)), &queue([1])).unwrap(),
            Err(TransferRefusal::Same)
        );
    }

    #[test]
    fn grouping_is_recognised() {
        assert!(grouped(
            S,
            &json!({"sync_master": S, "playlist_tracks": 3}),
            3
        ));
        assert!(grouped(
            S,
            &json!({"sync_master": "aa:bb:cc:dd:ee:09", "sync_slaves": format!("{T},{}", S.to_uppercase()), "playlist_tracks": "3"}),
            3
        ));
        // Not grouped yet, or the queue hasn't arrived yet.
        assert!(!grouped(S, &json!({"playlist_tracks": 3}), 3));
        assert!(!grouped(
            S,
            &json!({"sync_master": S, "playlist_tracks": 0}),
            3
        ));
        assert!(!grouped(
            S,
            &json!({"sync_master": "aa:bb:cc:dd:ee:011"}),
            0
        ));
        assert!(!grouped(S, &json!(null), 0));
        assert!(!grouped(
            S,
            &json!({"sync_master": 5, "sync_slaves": [S]}),
            0
        ));
    }

    /// A scripted LMS: records what is sent, and answers `status` per player.
    #[derive(Default)]
    pub(crate) struct Script {
        pub sent: Mutex<Vec<(String, Value)>>,
        /// Answers to `status` for the target, in order (the last repeats).
        pub target_status: Mutex<Vec<Value>>,
        pub source_status: Value,
        pub queue: Value,
        /// Fail the n-th send (0-based) with this error.
        pub fail_at: Option<(usize, ApiError)>,
    }

    impl Rpc for Script {
        async fn call(&self, player: &str, cmd: Command) -> Result<Value, ApiError> {
            let n = {
                let mut sent = self.sent.lock().unwrap();
                sent.push((player.to_string(), Value::from(cmd.clone())));
                sent.len() - 1
            };
            if let Some((at, e)) = &self.fail_at
                && *at == n
            {
                return Err(e.clone());
            }
            if cmd[0] == "status" && cmd[1] == "-" {
                if player == S {
                    return Ok(self.source_status.clone());
                }
                let mut q = self.target_status.lock().unwrap();
                return Ok(if q.len() > 1 {
                    q.remove(0)
                } else {
                    q.first().cloned().unwrap_or_default()
                });
            }
            if cmd[0] == "status" {
                return Ok(self.queue.clone());
            }
            Ok(json!({}))
        }
    }

    impl Script {
        pub(crate) fn sent(&self) -> Vec<(String, Value)> {
            self.sent.lock().unwrap().clone()
        }
    }

    const FAST: Pace = Pace {
        checks: GROUP_CHECKS,
        interval: Duration::ZERO,
    };

    fn run_transfer(script: &Script) -> (Result<TransferOutcome, ApiError>, Vec<TransferStage>) {
        let stages = Mutex::new(Vec::new());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let r = rt.block_on(transfer(script, S, T, FAST, &|s| {
            stages.lock().unwrap().push(s)
        }));
        (r, stages.into_inner().unwrap())
    }

    #[test]
    fn group_transfer_runs_in_order() {
        let script = Script {
            source_status: json!({"mode": "pause", "playlist_tracks": 3}),
            target_status: Mutex::new(vec![
                json!({"playlist_tracks": 0}),
                json!({"sync_master": S, "playlist_tracks": 3}),
            ]),
            ..Default::default()
        };
        let (r, stages) = run_transfer(&script);
        assert_eq!(
            r.unwrap(),
            TransferOutcome::Moved {
                method: TransferMethod::Group
            }
        );
        assert_eq!(
            stages,
            vec![
                TransferStage::Reading,
                TransferStage::Grouping,
                TransferStage::Starting
            ]
        );
        let status = Value::from(commands::status());
        assert_eq!(
            script.sent(),
            vec![
                (S.into(), status.clone()),
                (T.into(), json!(["power", 1])),
                (S.into(), json!(["sync", T])),
                (T.into(), status.clone()),
                (T.into(), status),
                (S.into(), json!(["sync", "-"])),
                (S.into(), json!(["stop"])),
                (T.into(), json!(["pause", 0])),
            ]
        );
    }

    #[test]
    fn an_empty_queue_is_refused_after_one_read() {
        let script = Script {
            source_status: json!({"mode": "stop", "playlist_tracks": 0}),
            ..Default::default()
        };
        let (r, _) = run_transfer(&script);
        assert_eq!(
            r.unwrap(),
            TransferOutcome::Refused {
                reason: TransferRefusal::Empty
            }
        );
        assert_eq!(script.sent().len(), 1);
    }

    #[test]
    fn same_or_malformed_players_send_nothing() {
        let script = Script::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        assert_eq!(
            rt.block_on(transfer(&script, S, &S.to_uppercase(), FAST, &|_| {}))
                .unwrap(),
            TransferOutcome::Refused {
                reason: TransferRefusal::Same
            }
        );
        assert!(matches!(
            rt.block_on(transfer(&script, S, "aa:bb", FAST, &|_| {})),
            Err(ApiError::InvalidInput { .. })
        ));
        assert!(script.sent().is_empty());
    }

    #[test]
    fn when_lms_will_not_group_the_list_is_copied() {
        let script = Script {
            source_status: json!({"mode": "play", "time": 40, "playlist_tracks": 2, "playlist_cur_index": 1}),
            target_status: Mutex::new(vec![json!({"playlist_tracks": 0})]),
            queue: json!({"playlist_loop": [{"id": 11}, {"id": 12}]}),
            ..Default::default()
        };
        let (r, stages) = run_transfer(&script);
        assert_eq!(
            r.unwrap(),
            TransferOutcome::Moved {
                method: TransferMethod::Copy
            }
        );
        assert_eq!(stages.last(), Some(&TransferStage::Copying));
        let sent = script.sent();
        let checks = sent
            .iter()
            .filter(|(p, c)| p == T && c[0] == "status")
            .count();
        assert_eq!(checks, GROUP_CHECKS as usize);
        // The target was taken out of the group before copying, and the source never left
        // (it was never grouped).
        let unsync_at = sent
            .iter()
            .position(|(p, c)| p == T && *c == json!(["sync", "-"]))
            .unwrap();
        let load_at = sent
            .iter()
            .position(|(_, c)| c[0] == "playlistcontrol")
            .unwrap();
        assert!(unsync_at < load_at);
        assert!(!sent.contains(&(S.into(), json!(["sync", "-"]))));
        assert!(sent.contains(&(
            T.into(),
            json!([
                "playlistcontrol",
                "cmd:load",
                "track_id:11,12",
                "play_index:1"
            ])
        )));
        assert!(
            sent.iter()
                .any(|(p, c)| p == T && c[0] == "time" && c[1].as_u64() >= Some(40))
        );
        assert_eq!(sent.last(), Some(&(S.into(), json!(["stop"]))));
    }

    #[test]
    fn radio_that_will_not_group_is_refused() {
        let script = Script {
            source_status: json!({"mode": "play", "playlist_tracks": 1}),
            target_status: Mutex::new(vec![json!({})]),
            queue: json!({"playlist_loop": [{"id": -12345, "remote": 1}]}),
            ..Default::default()
        };
        let (r, _) = run_transfer(&script);
        assert_eq!(
            r.unwrap(),
            TransferOutcome::Refused {
                reason: TransferRefusal::Remote
            }
        );
        // Nothing was loaded, and the source plays on.
        assert!(
            !script
                .sent()
                .iter()
                .any(|(_, c)| c[0] == "playlistcontrol" || c[0] == "stop")
        );
    }

    #[test]
    fn a_failure_after_joining_takes_the_target_out_again() {
        // Sends: status(S), power(T), sync(S), status(T), sync -(S) <- fails here.
        let script = Script {
            source_status: json!({"mode": "play", "playlist_tracks": 1}),
            target_status: Mutex::new(vec![json!({"sync_master": S, "playlist_tracks": 1})]),
            fail_at: Some((4, ApiError::network("connection reset"))),
            ..Default::default()
        };
        let (r, _) = run_transfer(&script);
        assert_eq!(r, Err(ApiError::network("connection reset")));
        assert_eq!(
            script.sent().last(),
            Some(&(T.into(), json!(["sync", "-"])))
        );
    }

    #[test]
    fn a_refused_target_stops_at_the_first_step() {
        // The gateway refuses the target (not this account's): power is the first step that
        // names it, and nothing more is sent (no clean-up either: it never joined).
        let script = Script {
            source_status: json!({"mode": "play", "playlist_tracks": 1}),
            fail_at: Some((
                1,
                ApiError::Forbidden {
                    detail: "player not permitted".into(),
                },
            )),
            ..Default::default()
        };
        let (r, _) = run_transfer(&script);
        assert!(matches!(r, Err(ApiError::Forbidden { .. })));
        assert_eq!(script.sent().len(), 2);
    }
}
