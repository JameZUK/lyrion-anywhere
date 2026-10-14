//! Whether LMS is answering. LMS is one process: when something blocks it (typically its
//! music share stalling while network storage spins up its disks) it answers nothing at all
//! for tens of seconds. The gateway notices a request left unanswered for [`crate::Config::lms_stall_after`],
//! tells the apps (`{"lms":"busy"}` on `/v1/events`), refuses new `/v1/rpc` requests at once
//! instead of piling them up inside LMS, and keeps one probe waiting so it notices the moment
//! LMS answers again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tracing::info;

/// How often the watchdog looks, at most.
const WATCH_EVERY: Duration = Duration::from_secs(1);
/// What clients are told to wait before retrying a request refused while busy.
pub const RETRY_AFTER_SECS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Busy since this time (Unix milliseconds).
    Busy {
        since_ms: u64,
    },
}

impl Status {
    /// The `/v1/events` message for this status.
    pub fn message(&self) -> serde_json::Value {
        match self {
            Status::Ok => serde_json::json!({ "lms": "ok" }),
            Status::Busy { since_ms } => serde_json::json!({ "lms": "busy", "since": since_ms }),
        }
    }
}

struct State {
    /// Requests waiting for LMS, by ticket, with when they were sent.
    waiting: HashMap<u64, Instant>,
    next: u64,
    /// When the current stall began (for the log), if busy.
    busy_since: Option<Instant>,
    probing: bool,
}

pub struct LmsHealth {
    state: Mutex<State>,
    tx: watch::Sender<Status>,
    /// A request unanswered this long means LMS is blocked (healthy answers take
    /// milliseconds).
    stall_after: Duration,
}

impl LmsHealth {
    pub fn new(stall_after: Duration) -> Self {
        LmsHealth {
            stall_after,
            state: Mutex::new(State {
                waiting: HashMap::new(),
                next: 0,
                busy_since: None,
                probing: false,
            }),
            tx: watch::channel(Status::Ok).0,
        }
    }
}

/// A request waiting for LMS; dropping it (answered or not) stops it counting.
pub struct Waiting<'a> {
    health: &'a LmsHealth,
    id: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.health.state.lock().unwrap().waiting.remove(&self.id);
    }
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl LmsHealth {
    pub fn status(&self) -> Status {
        *self.tx.borrow()
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.status(), Status::Busy { .. })
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.tx.subscribe()
    }

    /// A request is about to be sent to LMS.
    pub fn sent(&self) -> Waiting<'_> {
        self.sent_at(Instant::now())
    }

    fn sent_at(&self, at: Instant) -> Waiting<'_> {
        let mut st = self.state.lock().unwrap();
        st.next += 1;
        let id = st.next;
        st.waiting.insert(id, at);
        Waiting { health: self, id }
    }

    /// LMS answered something (any HTTP answer, even an error status): it is not blocked.
    pub fn answered(&self) {
        let mut st = self.state.lock().unwrap();
        if let Some(since) = st.busy_since.take() {
            info!(after = ?since.elapsed(), "LMS answering again");
            self.tx.send_replace(Status::Ok);
        }
    }

    /// Mark LMS busy if a request has waited the stall time or longer by `now`.
    fn check_at(&self, now: Instant, wall: SystemTime) {
        let mut st = self.state.lock().unwrap();
        if st.busy_since.is_some() {
            return;
        }
        let Some(oldest) = st.waiting.values().min().copied() else {
            return;
        };
        let waited = now.saturating_duration_since(oldest);
        if waited >= self.stall_after {
            st.busy_since = Some(oldest);
            let since = wall.checked_sub(waited).unwrap_or(wall);
            info!(waited = ?waited, "LMS not answering; telling apps it is busy");
            self.tx.send_replace(Status::Busy {
                since_ms: unix_ms(since),
            });
        }
    }

    /// Whether a probe should be sent now (busy and none waiting); claims the probe slot.
    fn claim_probe(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        if st.busy_since.is_some() && !st.probing {
            st.probing = true;
            true
        } else {
            false
        }
    }

    fn probe_done(&self) {
        self.state.lock().unwrap().probing = false;
    }
}

/// Watch LMS for as long as the gateway exists: mark it busy when a request stalls, and
/// while busy keep one cheap request waiting so recovery is noticed at once.
pub(crate) async fn watch(gw: Weak<crate::Gateway>) {
    let every = match gw.upgrade() {
        Some(g) => (g.lms_health().stall_after / 4).clamp(Duration::from_millis(10), WATCH_EVERY),
        None => return,
    };
    let mut tick = tokio::time::interval(every);
    loop {
        tick.tick().await;
        let Some(g) = gw.upgrade() else { return };
        g.lms_health().check_at(Instant::now(), SystemTime::now());
        if g.lms_health().claim_probe() {
            let g: Arc<crate::Gateway> = g.clone();
            tokio::spawn(async move {
                let _ = g.lms_probe().await;
                g.lms_health().probe_done();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn a_quick_answer_never_marks_busy() {
        let h = LmsHealth::new(Duration::from_secs(4));
        let base = Instant::now();
        let wall = SystemTime::now();
        {
            let _w = h.sent_at(base);
            h.check_at(at(base, 3), wall);
            assert_eq!(h.status(), Status::Ok);
            h.answered();
        }
        // Nothing waiting: however late the check, not busy.
        h.check_at(at(base, 100), wall);
        assert_eq!(h.status(), Status::Ok);
    }

    #[test]
    fn a_stalled_request_marks_busy_until_lms_answers() {
        let h = LmsHealth::new(Duration::from_secs(4));
        let mut rx = h.subscribe();
        let base = Instant::now();
        let wall = UNIX_EPOCH + Duration::from_secs(1_000);
        let w = h.sent_at(base);
        h.check_at(at(base, 4), wall);
        // Busy since the stalled request was sent (4 s before the check).
        assert_eq!(h.status(), Status::Busy { since_ms: 996_000 });
        assert!(h.is_busy());
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        // Later checks change nothing while busy.
        h.check_at(at(base, 10), wall);
        assert!(!rx.has_changed().unwrap());
        // The request times out (dropped unanswered): still busy.
        drop(w);
        h.check_at(at(base, 40), wall);
        assert!(h.is_busy());
        h.answered();
        assert_eq!(h.status(), Status::Ok);
        assert!(rx.has_changed().unwrap());
    }

    #[test]
    fn answers_while_healthy_send_no_change() {
        let h = LmsHealth::new(Duration::from_secs(4));
        let rx = h.subscribe();
        h.answered();
        h.answered();
        assert!(!rx.has_changed().unwrap());
    }

    #[test]
    fn one_probe_at_a_time_and_only_while_busy() {
        let h = LmsHealth::new(Duration::from_secs(4));
        assert!(!h.claim_probe());
        let base = Instant::now();
        let _w = h.sent_at(base);
        h.check_at(at(base, 5), SystemTime::now());
        assert!(h.claim_probe());
        assert!(!h.claim_probe());
        h.probe_done();
        assert!(h.claim_probe());
        h.probe_done();
        h.answered();
        assert!(!h.claim_probe());
    }

    #[test]
    fn the_oldest_waiting_request_decides() {
        let h = LmsHealth::new(Duration::from_secs(4));
        let base = Instant::now();
        let _new = h.sent_at(at(base, 3));
        let old = h.sent_at(base);
        h.check_at(at(base, 5), SystemTime::now());
        assert!(h.is_busy());
        h.answered();
        drop(old);
        // Only the newer one (2 s old) is left: fine.
        h.check_at(at(base, 5), SystemTime::now());
        assert!(!h.is_busy());
    }

    #[test]
    fn messages() {
        assert_eq!(Status::Ok.message().to_string(), r#"{"lms":"ok"}"#);
        assert_eq!(
            Status::Busy { since_ms: 5 }.message().to_string(),
            r#"{"lms":"busy","since":5}"#
        );
    }
}
