//! Plays made offline (SPEC D6): recorded on the device with an idempotency key, kept in a
//! durable queue (`plays.json`), and sent to `POST /v1/plays` once the server is back.
//!
//! The contract (being built on the gateway alongside this): the body is a JSON array of
//! `{track_id, played_at, seconds, key}`; the answer gives, per play, `recorded`,
//! `duplicate`, `refused` or `retry`. Recorded, duplicate and refused plays leave the
//! queue; `retry` (and anything the answer doesn't mention) stays. A server without the
//! endpoint (404) keeps every play for a later attempt. The sender is behind [`PlaysApi`],
//! so the queue logic is tested against a fake.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::error::ApiError;

/// Plays kept at most (the oldest are dropped beyond this: years of listening offline).
pub const MAX_PLAYS: usize = 10_000;
/// Plays sent per request.
pub const BATCH: usize = 100;
/// A play shorter than this isn't recorded (skipping through a list isn't listening).
pub const MIN_SECONDS: u32 = 5;
/// Largest plays file read.
const MAX_FILE: usize = 8 * 1024 * 1024;

/// One play, as queued and sent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlayRecord {
    /// Idempotency key (a random UUID): the server ignores a play it has already seen.
    pub key: String,
    /// The LMS track id when it was played.
    pub track_id: i64,
    /// Unix seconds when it started.
    pub played_at: u64,
    /// Seconds actually heard.
    pub seconds: u32,
}

/// The server's verdict on one play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlayVerdict {
    Recorded,
    Duplicate,
    Refused,
    Retry,
}

/// What a send achieved.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaysSyncOutcome {
    /// Plays the server took (recorded or already had).
    pub sent: u32,
    /// Plays the server refused (dropped: sending them again won't help).
    pub refused: u32,
    /// Plays still waiting.
    pub pending: u32,
    /// The server doesn't have `/v1/plays` yet (an older version): plays are kept.
    pub unsupported: bool,
}

/// Why a send failed as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// 404: the server has no plays endpoint.
    Unsupported,
    /// Anything else (network, sign-in, server failure): try again later.
    Failed(ApiError),
}

/// Sends a batch of plays and returns each one's verdict, by key.
pub trait PlaysApi {
    fn send(
        &self,
        body: Value,
    ) -> impl std::future::Future<Output = Result<Value, SendError>> + Send;
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A random (version 4) UUID.
pub fn new_key() -> String {
    let mut b = [0u8; 16];
    if getrandom::fill(&mut b).is_err() {
        // No randomness: fall back to the clock and a counter (still unique here).
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        b[..8].copy_from_slice(&t.to_le_bytes());
        b[8..].copy_from_slice(&N.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

fn record_json(p: &PlayRecord) -> Value {
    json!({"track_id": p.track_id, "played_at": p.played_at, "seconds": p.seconds, "key": p.key})
}

fn record_from(v: &Value) -> Option<PlayRecord> {
    let key = v.get("key")?.as_str()?;
    if !valid_uuid(key) {
        return None;
    }
    Some(PlayRecord {
        key: key.to_string(),
        track_id: v
            .get("track_id")?
            .as_i64()
            .filter(|n| (1..=999_999_999_999).contains(n))?,
        played_at: v.get("played_at")?.as_u64()?,
        seconds: v
            .get("seconds")?
            .as_u64()
            .filter(|s| *s <= 86_400)
            .map(|s| s as u32)?,
    })
}

/// The queue's file contents, checked (malformed entries dropped). Never panics.
pub fn parse(bytes: &[u8]) -> Vec<PlayRecord> {
    if bytes.len() > MAX_FILE {
        return Vec::new();
    }
    let Ok(Value::Array(list)) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<PlayRecord> = list
        .iter()
        .filter_map(record_from)
        .filter(|p| seen.insert(p.key.clone()))
        .collect();
    if out.len() > MAX_PLAYS {
        out.drain(..out.len() - MAX_PLAYS);
    }
    out
}

pub fn serialise(plays: &[PlayRecord]) -> Vec<u8> {
    serde_json::to_vec(&Value::Array(plays.iter().map(record_json).collect())).expect("JSON")
}

/// Add a play to `queue` (bounded: the oldest go first). `None` if too short to count.
pub fn record(
    queue: &mut Vec<PlayRecord>,
    track_id: i64,
    played_at: u64,
    seconds: u32,
) -> Option<PlayRecord> {
    if seconds < MIN_SECONDS || track_id <= 0 {
        return None;
    }
    let p = PlayRecord {
        key: new_key(),
        track_id,
        played_at,
        seconds: seconds.min(86_400),
    };
    queue.push(p.clone());
    if queue.len() > MAX_PLAYS {
        let extra = queue.len() - MAX_PLAYS;
        queue.drain(..extra);
    }
    Some(p)
}

/// Read the server's answer: `[{key, result}]` or `{"results": [...]}`, where `result`
/// (or `status`) is one of the four verdicts. Entries without a key are matched by
/// position. Anything unrecognised counts as "retry".
pub fn verdicts(answer: &Value, sent: &[PlayRecord]) -> Vec<PlayVerdict> {
    let list = match answer {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get("results").and_then(Value::as_array),
        _ => None,
    };
    let verdict_of = |v: &Value| -> Option<PlayVerdict> {
        let s = v
            .as_str()
            .or_else(|| v.get("result").and_then(Value::as_str))
            .or_else(|| v.get("status").and_then(Value::as_str))?;
        match s {
            "recorded" => Some(PlayVerdict::Recorded),
            "duplicate" => Some(PlayVerdict::Duplicate),
            "refused" => Some(PlayVerdict::Refused),
            "retry" => Some(PlayVerdict::Retry),
            _ => None,
        }
    };
    let mut out = vec![PlayVerdict::Retry; sent.len()];
    let Some(list) = list else { return out };
    for (i, item) in list.iter().enumerate().take(sent.len() * 2) {
        let Some(v) = verdict_of(item) else { continue };
        match item.get("key").and_then(Value::as_str) {
            Some(k) => {
                if let Some(j) = sent.iter().position(|p| p.key == k) {
                    out[j] = v;
                }
            }
            None if list.len() == sent.len() => out[i] = v,
            None => {}
        }
    }
    out
}

/// Send what is queued, in batches, and drop what the server has dealt with. `queue` is
/// read and written through `load`/`save` so concurrent recording isn't lost: the queue
/// is re-read after every batch.
pub async fn flush<A: PlaysApi>(
    api: &A,
    mut load: impl FnMut() -> Vec<PlayRecord>,
    mut drop_keys: impl FnMut(&[String]),
) -> PlaysSyncOutcome {
    let mut outcome = PlaysSyncOutcome {
        sent: 0,
        refused: 0,
        pending: 0,
        unsupported: false,
    };
    // Keys already tried this round and left waiting (`retry`): not sent again now.
    let mut waiting = std::collections::HashSet::new();
    loop {
        let batch: Vec<PlayRecord> = load()
            .into_iter()
            .filter(|p| !waiting.contains(&p.key))
            .take(BATCH)
            .collect();
        if batch.is_empty() {
            break;
        }
        let body = Value::Array(batch.iter().map(record_json).collect());
        let answer = match api.send(body).await {
            Ok(a) => a,
            Err(SendError::Unsupported) => {
                outcome.unsupported = true;
                break;
            }
            Err(SendError::Failed(_)) => break,
        };
        let mut done = Vec::new();
        for (p, v) in batch.iter().zip(verdicts(&answer, &batch)) {
            match v {
                PlayVerdict::Recorded | PlayVerdict::Duplicate => {
                    outcome.sent += 1;
                    done.push(p.key.clone());
                }
                PlayVerdict::Refused => {
                    outcome.refused += 1;
                    done.push(p.key.clone());
                }
                PlayVerdict::Retry => {
                    waiting.insert(p.key.clone());
                }
            }
        }
        drop_keys(&done);
    }
    outcome.pending = load().len() as u32;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn keys_are_v4_uuids_and_unique() {
        let a = new_key();
        let b = new_key();
        assert!(valid_uuid(&a) && valid_uuid(&b));
        assert_ne!(a, b);
        assert_eq!(&a[14..15], "4");
        assert!(!valid_uuid("../../etc/passwd"));
        assert!(!valid_uuid(&a.to_uppercase()));
    }

    #[test]
    fn short_plays_are_not_recorded_and_the_queue_is_bounded() {
        let mut q = Vec::new();
        assert!(record(&mut q, 7, 1, MIN_SECONDS - 1).is_none());
        assert!(record(&mut q, 0, 1, 60).is_none());
        let first = record(&mut q, 7, 1, 60).unwrap();
        assert_eq!(q.len(), 1);
        for i in 0..MAX_PLAYS {
            record(&mut q, 8, i as u64, 30);
        }
        assert_eq!(q.len(), MAX_PLAYS);
        assert!(
            !q.iter().any(|p| p.key == first.key),
            "the oldest went first"
        );
        let huge = record(&mut q, 8, 1, u32::MAX).unwrap();
        assert_eq!(huge.seconds, 86_400);
    }

    #[test]
    fn the_file_round_trips_and_survives_damage() {
        let mut q = Vec::new();
        for i in 0..5 {
            record(&mut q, 10 + i, 1000, 60);
        }
        assert_eq!(parse(&serialise(&q)), q);
        let mut v: Value = serde_json::from_slice(&serialise(&q)).unwrap();
        v[0]["key"] = json!("not-a-uuid");
        v[1]["track_id"] = json!(-3);
        v[2]["seconds"] = json!(1e12);
        v[3] = v[4].clone(); // duplicate key
        let back = parse(&serde_json::to_vec(&v).unwrap());
        assert_eq!(back, vec![q[4].clone()]);
        for junk in [&b""[..], b"{}", b"[1,2", b"\xff", b"null"] {
            assert!(parse(junk).is_empty());
        }
    }

    #[test]
    fn verdicts_by_key_by_position_and_unknown() {
        let mut q = Vec::new();
        for i in 0..3 {
            record(&mut q, 10 + i, 1000, 60);
        }
        let by_key = json!([
            {"key": q[2].key, "result": "duplicate"},
            {"key": q[0].key, "result": "recorded"},
            {"key": "someone-else", "result": "refused"}
        ]);
        assert_eq!(
            verdicts(&by_key, &q),
            vec![
                PlayVerdict::Recorded,
                PlayVerdict::Retry,
                PlayVerdict::Duplicate
            ]
        );
        let wrapped = json!({"results": ["recorded", {"status": "refused"}, "nonsense"]});
        assert_eq!(
            verdicts(&wrapped, &q),
            vec![
                PlayVerdict::Recorded,
                PlayVerdict::Refused,
                PlayVerdict::Retry
            ]
        );
        assert_eq!(verdicts(&json!("ok"), &q), vec![PlayVerdict::Retry; 3]);
        // Positional answers of the wrong length are ignored.
        assert_eq!(
            verdicts(&json!(["recorded"]), &q),
            vec![PlayVerdict::Retry; 3]
        );
    }

    /// A fake `/v1/plays` with a scripted behaviour.
    struct Fake {
        mode: Mode,
        calls: Mutex<Vec<Value>>,
    }

    #[derive(Clone, Copy)]
    enum Mode {
        AllRecorded,
        Unsupported,
        Down,
        /// Every other play: retry; the rest refused.
        Mixed,
    }

    impl PlaysApi for Fake {
        async fn send(&self, body: Value) -> Result<Value, SendError> {
            self.calls.lock().unwrap().push(body.clone());
            let items = body.as_array().unwrap();
            match self.mode {
                Mode::Unsupported => Err(SendError::Unsupported),
                Mode::Down => Err(SendError::Failed(ApiError::network("down"))),
                Mode::AllRecorded => Ok(Value::Array(
                    items
                        .iter()
                        .map(|p| json!({"key": p["key"], "result": "recorded"}))
                        .collect(),
                )),
                Mode::Mixed => Ok(Value::Array(
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            json!({"key": p["key"], "result": if i % 2 == 0 { "retry" } else { "refused" }})
                        })
                        .collect(),
                )),
            }
        }
    }

    fn run(mode: Mode, n: usize) -> (PlaysSyncOutcome, Vec<PlayRecord>, Vec<Value>) {
        let mut q = Vec::new();
        for i in 0..n {
            record(&mut q, 1 + i as i64, 1000, 60);
        }
        let store = Mutex::new(q);
        let fake = Fake {
            mode,
            calls: Mutex::new(Vec::new()),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on(flush(
            &fake,
            || store.lock().unwrap().clone(),
            |keys| store.lock().unwrap().retain(|p| !keys.contains(&p.key)),
        ));
        let left = store.lock().unwrap().clone();
        (out, left, fake.calls.into_inner().unwrap())
    }

    #[test]
    fn everything_sent_in_batches() {
        let (out, left, calls) = run(Mode::AllRecorded, 250);
        assert_eq!(out.sent, 250);
        assert_eq!(out.pending, 0);
        assert!(left.is_empty());
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].as_array().unwrap().len(), BATCH);
        let first = &calls[0][0];
        for k in ["track_id", "played_at", "seconds", "key"] {
            assert!(first.get(k).is_some(), "{k}");
        }
        assert_eq!(first.as_object().unwrap().len(), 4, "no other fields sent");
    }

    #[test]
    fn an_older_server_keeps_every_play() {
        let (out, left, calls) = run(Mode::Unsupported, 30);
        assert!(out.unsupported);
        assert_eq!((out.sent, out.pending), (0, 30));
        assert_eq!(left.len(), 30);
        assert_eq!(calls.len(), 1);
        let (out, left, _) = run(Mode::Down, 30);
        assert_eq!((out.sent, out.pending, left.len()), (0, 30, 30));
        assert!(!out.unsupported);
    }

    #[test]
    fn retry_stays_refused_goes_and_nothing_loops() {
        let (out, left, calls) = run(Mode::Mixed, 10);
        assert_eq!(out.refused, 5);
        assert_eq!(out.pending, 5);
        assert_eq!(left.len(), 5);
        assert_eq!(calls.len(), 1, "plays told to retry aren't resent at once");
    }
}
