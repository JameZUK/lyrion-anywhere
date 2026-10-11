//! Queue editing and sync groups, as the web app does them (`queue-edit.ts`, `sync.ts`):
//! what to send is planned here, purely, and run through [`Rpc`] so the gateway's policy
//! applies to every command.

use std::time::Duration;

use serde_json::Value;

use crate::commands::{self, Command, Placement, Selection};
use crate::error::ApiError;
use crate::model::float_of;
use crate::transfer::Rpc;

/// Undo waits up to this many checks, this far apart, for LMS to add the track back.
pub const RESTORE_CHECKS: u32 = 10;
pub const RESTORE_CHECK_INTERVAL: Duration = Duration::from_millis(150);

/// What to send to put `other` in the same group as `current`, or take it out.
///
/// `<X> sync <Y>` makes Y join X's group (leaving any it was in), and Y takes the queue X's
/// group plays. So joining is always sent for the selected player, naming the one that
/// joins it; the other way round would pull the selected player out of its group and onto
/// the other player's queue. Leaving is sent for the player that leaves.
pub fn sync_toggle(current: &str, other: &str, on: bool) -> Result<(String, Command), ApiError> {
    let other = commands::player_id(other)?;
    if !on {
        return Ok((other, commands::unsync()));
    }
    let current = commands::player_id(current)?;
    if current == other {
        return Err(ApiError::invalid("a player can't join its own group"));
    }
    let cmd = commands::sync_with(&other)?;
    Ok((current, cmd))
}

fn queue_length(v: &Value) -> Option<u32> {
    float_of(&v["playlist_tracks"])
        .filter(|n| *n >= 0.0 && *n <= f64::from(u32::MAX))
        .map(|n| n as u32)
}

/// Undo a removal: add the track to the end, wait until the queue is exactly one longer
/// (LMS may apply it a moment later), then move it back to `index`.
pub(crate) async fn restore<R: Rpc>(
    rpc: &R,
    player: &str,
    track_id: i64,
    index: u32,
    checks: u32,
    interval: Duration,
) -> Result<(), ApiError> {
    let p = commands::player_id(player)?;
    let add = commands::enqueue(
        Selection::Track { id: track_id },
        Placement::AddToQueue,
        None,
    )?;
    let before = queue_length(&rpc.call(&p, commands::status()).await?);
    rpc.call(&p, add).await?;
    let mut after = queue_length(&rpc.call(&p, commands::status()).await?);
    for _ in 0..checks {
        if after != before {
            break;
        }
        tokio::time::sleep(interval).await;
        after = queue_length(&rpc.call(&p, commands::status()).await?);
    }
    match (before, after) {
        (Some(b), Some(a)) if a == b.saturating_add(1) => {
            if b > index {
                rpc.call(&p, commands::move_in_queue(b, index)).await?;
            }
            Ok(())
        }
        _ => Err(ApiError::Server {
            status: 409,
            detail: "the queue changed meanwhile, so the track was added at the end".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    const A: &str = "aa:bb:cc:dd:ee:01";
    const B: &str = "aa:bb:cc:dd:ee:02";

    #[test]
    fn sync_is_sent_for_the_selected_player_naming_the_other() {
        let (p, c) = sync_toggle(A, B, true).unwrap();
        assert_eq!(p, A);
        assert_eq!(Value::from(c), json!(["sync", B]));
        // Leaving is sent for the one that leaves.
        let (p, c) = sync_toggle(A, &B.to_uppercase(), false).unwrap();
        assert_eq!(p, B);
        assert_eq!(Value::from(c), json!(["sync", "-"]));
    }

    #[test]
    fn sync_refuses_bad_or_same_players() {
        assert!(sync_toggle(A, A, true).is_err());
        assert!(sync_toggle(A, &A.to_uppercase(), true).is_err());
        for bad in [
            "",
            "-",
            "aa:bb:cc:dd:ee",
            "x;y",
            "aa:bb:cc:dd:ee:01,aa:bb:cc:dd:ee:02",
        ] {
            assert!(sync_toggle(A, bad, true).is_err(), "{bad:?}");
            assert!(sync_toggle(bad, B, true).is_err(), "{bad:?}");
            assert!(sync_toggle(A, bad, false).is_err(), "{bad:?}");
        }
    }

    /// A queue whose length grows only after `lag` status reads following the add.
    struct Queue {
        sent: Mutex<Vec<Value>>,
        len: Mutex<u32>,
        lag: Mutex<u32>,
        added: Mutex<bool>,
        /// Someone else adds a track meanwhile.
        meddle: bool,
    }

    impl Queue {
        fn new(len: u32, lag: u32, meddle: bool) -> Queue {
            Queue {
                sent: Mutex::new(vec![]),
                len: Mutex::new(len),
                lag: Mutex::new(lag),
                added: Mutex::new(false),
                meddle,
            }
        }
    }

    impl Rpc for Queue {
        async fn call(&self, _player: &str, cmd: Command) -> Result<Value, ApiError> {
            let v = Value::from(cmd);
            self.sent.lock().unwrap().push(v.clone());
            if v[0] == "playlistcontrol" {
                *self.added.lock().unwrap() = true;
            }
            if v[0] == "status" {
                if *self.added.lock().unwrap() {
                    let mut lag = self.lag.lock().unwrap();
                    if *lag == 0 {
                        *self.added.lock().unwrap() = false;
                        *self.len.lock().unwrap() += if self.meddle { 2 } else { 1 };
                    } else {
                        *lag -= 1;
                    }
                }
                return Ok(json!({"playlist_tracks": *self.len.lock().unwrap()}));
            }
            Ok(json!({}))
        }
    }

    fn block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn undo_adds_then_moves_back_into_place() {
        let q = Queue::new(4, 2, false);
        block(restore(&q, A, 7, 1, RESTORE_CHECKS, Duration::ZERO)).unwrap();
        let sent = q.sent.lock().unwrap().clone();
        assert_eq!(sent[1], json!(["playlistcontrol", "cmd:add", "track_id:7"]));
        assert_eq!(sent.last().unwrap(), &json!(["playlist", "move", 4, 1]));
    }

    #[test]
    fn undo_of_the_last_track_needs_no_move() {
        let q = Queue::new(4, 0, false);
        block(restore(&q, A, 7, 4, RESTORE_CHECKS, Duration::ZERO)).unwrap();
        assert!(!q.sent.lock().unwrap().iter().any(|c| c[1] == "move"));
    }

    #[test]
    fn undo_reports_a_queue_that_changed_meanwhile() {
        let q = Queue::new(4, 0, true);
        let r = block(restore(&q, A, 7, 1, RESTORE_CHECKS, Duration::ZERO));
        assert!(matches!(r, Err(ApiError::Server { status: 409, .. })));
        assert!(!q.sent.lock().unwrap().iter().any(|c| c[1] == "move"));
        // LMS never adds it: also reported, after a bounded number of checks.
        let q = Queue::new(4, 1000, false);
        assert!(block(restore(&q, A, 7, 1, 3, Duration::ZERO)).is_err());
        assert_eq!(q.sent.lock().unwrap().len(), 1 + 1 + 1 + 3);
    }

    #[test]
    fn undo_refuses_bad_input_before_sending() {
        let q = Queue::new(4, 0, false);
        for (player, id) in [(A, 0), (A, -7), ("nope", 7)] {
            assert!(block(restore(&q, player, id, 0, 1, Duration::ZERO)).is_err());
        }
        assert!(q.sent.lock().unwrap().is_empty());
    }
}
