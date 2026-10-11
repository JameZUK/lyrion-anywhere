//! Offline play sync-back (spec D6, §8.4): `POST /v1/plays`.
//!
//! A device that played downloaded tracks with no connection sends those plays here when it
//! is back online. Each one LMS accepts raises the track's play count and last-played time,
//! as if it had played on a player.
//!
//! - **Who:** a valid access token. No download permission is needed: a play is only a
//!   count, and the track must be in the library anyway.
//! - **What:** a JSON array of at most [`MAX_ITEMS`] plays, [`MAX_BODY`] bytes at most. Each
//!   is `{"key", "track_id", "played_at", "seconds"}`, with an optional `"track_key"` (the
//!   stable key from the download's info). Anything else in an item makes that item invalid.
//! - **Checks, per play:**
//!   - `key`: the client's idempotency key, 8-64 characters of `A-Z a-z 0-9 _ -`;
//!   - `track_id`: a JSON integer, 1 to 999 999 999 999 (as in download URLs);
//!   - `played_at` (when playback started, Unix seconds): within the last 90 days, and
//!     neither it nor `played_at + seconds` later than now plus 5 minutes of clock skew;
//!   - `seconds` played: a finite number, 0 to 86 400;
//!   - the track must be a local library file (looked up as for downloads); with a
//!     `track_key`, it must still be the same file (LMS renumbers tracks on a full rescan);
//!   - **the counting rule (as scrobblers use):** a play counts only if `seconds` is at
//!     least half the track's duration or 240 seconds, whichever is less (240 if the
//!     duration is unknown);
//!   - **no overlap:** a play of a track the user already has a recorded play of, starting
//!     less than that counting time before or after it, is refused (`overlap`): it can't
//!     be a separate listen.
//! - **Idempotency:** keys are kept per user (see [`crate::accounts::Accounts::claim_play`])
//!   for as long as their plays could be accepted, so a retried upload is answered
//!   `duplicate`, never counted twice.
//! - **Limits:** one upload at a time per user; [`Config::plays_requests_per_minute`]
//!   uploads a minute and [`Config::plays_per_hour`] recorded plays an hour per user (more
//!   are answered `retry`); at most [`MAX_KEYS_PER_USER`] keys kept per user; and a time
//!   budget per upload, after which the rest are answered `retry`.
//!
//! # Forwarding to LMS
//!
//! LMS has no command that counts a play, so the plugin provides one: `anywhere
//! play_record`, sent over LMS's JSON-RPC on loopback like every other LMS call the gateway
//! makes. LMS's JSON-RPC is open to the LAN, so the command is signed: an HMAC-SHA256 over
//! the play, its key and the time, keyed with the admin secret both sides already share
//! (the gateway writes it to its data directory; the plugin reads it from there to call the
//! admin API). The plugin refuses anything unsigned, wrongly signed or more than 2 minutes
//! old, and also remembers recent keys itself. Clients can never send `anywhere` commands
//! through `/v1/rpc` (it is reserved in the allow-list).
//!
//! [`Config::plays_requests_per_minute`]: crate::Config::plays_requests_per_minute
//! [`Config::plays_per_hour`]: crate::Config::plays_per_hour

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Map, Value, json};
use sha2::Sha256;

use crate::AppState;
use crate::accounts::PlayClaim;
use crate::download::Refusal;

/// Most plays in one upload.
pub const MAX_ITEMS: usize = 200;
/// Largest upload accepted (about 300 bytes a play leaves plenty of room).
pub const MAX_BODY: usize = 96 * 1024;
/// Oldest play accepted.
pub const MAX_AGE: Duration = Duration::from_secs(90 * 86_400);
/// Clock skew allowed between device and gateway.
pub const SKEW: Duration = Duration::from_secs(300);
/// Longest single play.
pub const MAX_SECONDS: f64 = 86_400.0;
/// A play counts once this much has been heard (or half the track, if less).
pub const COUNT_AFTER: f64 = 240.0;
/// Largest track id (12 digits, as in download URLs).
pub const MAX_TRACK_ID: u64 = 999_999_999_999;
/// Keys kept per user, at most (the newest).
pub const MAX_KEYS_PER_USER: usize = 50_000;
/// A claim left pending this long (a crash mid-forward) may be taken over.
const STALE_CLAIM: Duration = Duration::from_secs(120);
/// Time limit for LMS to record one play.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(10);
/// Pause the client should take before retrying.
const RETRY_AFTER: u64 = 60;
/// How often old keys are pruned for everyone.
const PRUNE_EVERY: Duration = Duration::from_secs(3600);
/// What the signature covers comes first in the message: a version, then the fields.
const SIG_CONTEXT: &str = "lyrion-anywhere play_record v1";

/// The signature the plugin checks on `anywhere play_record`: hex HMAC-SHA256, keyed with
/// the admin secret, of `SIG_CONTEXT`, the track id, start time, key and current time,
/// each on its own line.
pub fn signature(secret: &str, track_id: u64, played_at: u64, key: &str, ts: u64) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).expect("any key");
    mac.update(format!("{SIG_CONTEXT}\n{track_id}\n{played_at}\n{key}\n{ts}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The key the plugin remembers a play by: the client's key, scoped to its user (two users'
/// clients may pick the same key) and not revealing either. 32 hex characters.
pub fn plugin_key(user_id: i64, key: &str) -> String {
    use sha2::Digest;
    let mut h = Sha256::new();
    h.update(format!("{user_id}\n{key}").as_bytes());
    hex::encode(h.finalize())[..32].to_string()
}

// ---------------------------------------------------------------------------
// Parsing and checks (pure)
// ---------------------------------------------------------------------------

/// One play, as checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    pub key: String,
    pub track_id: u64,
    pub played_at: u64,
    pub seconds: f64,
    pub track_key: Option<String>,
}

/// An upload that can't be read at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    /// Not a JSON array, or empty.
    Malformed,
    /// More than [`MAX_ITEMS`] plays.
    TooMany,
}

/// An item that isn't a well-formed play: its key (if that much was readable) and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadItem {
    pub key: Option<String>,
    pub reason: &'static str,
}

pub fn valid_key(s: &str) -> bool {
    (8..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_track_key(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

const FIELDS: [&str; 5] = ["key", "track_id", "played_at", "seconds", "track_key"];

/// Check one item's shape and values (not its time: see [`check_time`]).
pub fn parse_item(v: &Value) -> Result<Play, BadItem> {
    let Some(o) = v.as_object() else {
        return Err(BadItem {
            key: None,
            reason: "invalid",
        });
    };
    let key = o
        .get("key")
        .and_then(Value::as_str)
        .filter(|k| valid_key(k))
        .map(str::to_string);
    let bad = |reason| BadItem {
        key: key.clone(),
        reason,
    };
    if key.is_none() || o.len() > FIELDS.len() || o.keys().any(|k| !FIELDS.contains(&k.as_str())) {
        return Err(bad("invalid"));
    }
    let uint = |name: &str| o.get(name).and_then(|n| n.as_u64().filter(|_| n.is_u64()));
    let track_id = uint("track_id")
        .filter(|n| (1..=MAX_TRACK_ID).contains(n))
        .ok_or_else(|| bad("invalid"))?;
    let played_at = uint("played_at").ok_or_else(|| bad("invalid"))?;
    let seconds = o
        .get("seconds")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && (0.0..=MAX_SECONDS).contains(s))
        .ok_or_else(|| bad("invalid"))?;
    let track_key = match o.get("track_key") {
        None => None,
        Some(Value::String(k)) if valid_track_key(k) => Some(k.clone()),
        Some(_) => return Err(bad("invalid")),
    };
    Ok(Play {
        key: key.unwrap(),
        track_id,
        played_at,
        seconds,
        track_key,
    })
}

/// Read an upload: the list of items, each checked on its own.
pub fn parse_body(bytes: &[u8]) -> Result<Vec<Result<Play, BadItem>>, BodyError> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| BodyError::Malformed)?;
    let Value::Array(items) = v else {
        return Err(BodyError::Malformed);
    };
    if items.is_empty() {
        return Err(BodyError::Malformed);
    }
    if items.len() > MAX_ITEMS {
        return Err(BodyError::TooMany);
    }
    Ok(items.iter().map(parse_item).collect())
}

/// Is the play's time plausible? `Err` with the reason it is refused.
pub fn check_time(played_at: u64, seconds: f64, now: u64) -> Result<(), &'static str> {
    let latest = now.saturating_add(SKEW.as_secs());
    if played_at > latest || played_at as f64 + seconds > latest as f64 {
        return Err("in_future");
    }
    if now.saturating_sub(played_at) > MAX_AGE.as_secs() {
        return Err("too_old");
    }
    Ok(())
}

/// The counting rule: at least half the track, or [`COUNT_AFTER`] seconds if that is less.
/// Without a known duration, [`COUNT_AFTER`].
pub fn counts(seconds: f64, duration: Option<f64>) -> bool {
    seconds >= needed(duration)
}

/// Seconds of a track that must be heard for a play to count.
fn needed(duration: Option<f64>) -> f64 {
    match duration {
        Some(d) if d.is_finite() && d > 0.0 => (d / 2.0).min(COUNT_AFTER),
        _ => COUNT_AFTER,
    }
}

/// What became of one play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Recorded,
    Duplicate,
    Refused(&'static str),
    Retry(&'static str),
}

impl Outcome {
    fn json(self, key: Option<&str>) -> Value {
        let (result, reason) = match self {
            Outcome::Recorded => ("recorded", None),
            Outcome::Duplicate => ("duplicate", None),
            Outcome::Refused(r) => ("refused", Some(r)),
            Outcome::Retry(r) => ("retry", Some(r)),
        };
        let mut m = Map::new();
        m.insert("key".into(), key.map_or(Value::Null, Value::from));
        m.insert("result".into(), result.into());
        if let Some(r) = reason {
            m.insert("reason".into(), r.into());
        }
        Value::Object(m)
    }
}

// ---------------------------------------------------------------------------
// Rate limits
// ---------------------------------------------------------------------------

/// A sliding-window count per user.
pub struct Window {
    limit: usize,
    span: Duration,
    seen: Mutex<HashMap<i64, VecDeque<Instant>>>,
}

impl Window {
    pub fn new(limit: usize, span: Duration) -> Self {
        Window {
            limit,
            span,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Take one, if the user has any left in the window.
    pub fn take(&self, user: i64, now: Instant) -> bool {
        let mut m = self.seen.lock().unwrap();
        if m.len() > 10_000 {
            m.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < self.span));
        }
        let q = m.entry(user).or_default();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) >= self.span)
        {
            q.pop_front();
        }
        if q.len() >= self.limit {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// Per-gateway state for play uploads.
pub struct PlayLimits {
    busy: Mutex<HashSet<i64>>,
    requests: Window,
    plays: Window,
    pruned: Mutex<Option<Instant>>,
}

impl PlayLimits {
    pub fn new(requests_per_minute: usize, plays_per_hour: usize) -> Self {
        PlayLimits {
            busy: Mutex::new(HashSet::new()),
            requests: Window::new(requests_per_minute, Duration::from_secs(60)),
            plays: Window::new(plays_per_hour, Duration::from_secs(3600)),
            pruned: Mutex::new(None),
        }
    }
}

/// The user's one upload slot, freed when dropped.
struct Busy<'a> {
    limits: &'a PlayLimits,
    user: i64,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.limits.busy.lock().unwrap().remove(&self.user);
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

fn error(status: StatusCode, message: &str) -> Response {
    let mut r = (status, Json(json!({ "error": message }))).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

fn too_many(message: &str, after: u64) -> Response {
    let mut r = error(StatusCode::TOO_MANY_REQUESTS, message);
    r.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(after));
    r
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `POST /v1/plays`
pub async fn record(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let Some(p) = crate::auth::principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_BODY as u64) {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "too many plays");
    }
    let Some(secret) = gw.cfg.plays_secret.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "play sync is not available",
        );
    };
    let limits = &gw.plays;
    if !limits.busy.lock().unwrap().insert(p.user_id) {
        return too_many("another upload of plays is running", 5);
    }
    let _busy = Busy {
        limits,
        user: p.user_id,
    };
    if !limits.requests.take(p.user_id, Instant::now()) {
        return too_many("too many uploads; try again shortly", RETRY_AFTER);
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "too many plays");
    };
    let items = match parse_body(&bytes) {
        Ok(i) => i,
        Err(BodyError::TooMany) => {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "too many plays");
        }
        Err(BodyError::Malformed) => return error(StatusCode::BAD_REQUEST, "malformed plays"),
    };

    let now = unix_now();
    let oldest = now.saturating_sub(MAX_AGE.as_secs() + 86_400);
    {
        let mut pruned = limits.pruned.lock().unwrap();
        if pruned.is_none_or(|t| t.elapsed() >= PRUNE_EVERY) {
            *pruned = Some(Instant::now());
            let _ = gw.accounts().prune_plays(None, oldest, MAX_KEYS_PER_USER);
        }
    }

    let started = Instant::now();
    let mut lookups: HashMap<u64, Result<crate::download::Track, Outcome>> = HashMap::new();
    let mut lms_down = false;
    let mut recorded = 0usize;
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let play = match item {
            Ok(play) => play,
            Err(bad) => {
                results.push(Outcome::Refused(bad.reason).json(bad.key.as_deref()));
                continue;
            }
        };
        let outcome = gw
            .record_one(
                &p,
                &play,
                &secret,
                now,
                started,
                &mut lookups,
                &mut lms_down,
            )
            .await;
        if outcome == Outcome::Recorded {
            recorded += 1;
        }
        results.push(outcome.json(Some(&play.key)));
    }
    let _ = gw
        .accounts()
        .prune_plays(Some(p.user_id), oldest, MAX_KEYS_PER_USER);
    if recorded > 0 {
        gw.accounts().audit(
            "plays",
            Some(&p.username),
            Some(p.device_id),
            &format!("recorded {recorded}"),
        );
    }
    let retry = results.iter().any(|r| r["result"] == "retry");
    let mut out = json!({ "results": results });
    if retry {
        out["retry_after"] = RETRY_AFTER.into();
    }
    let mut r = Json(out).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

impl crate::Gateway {
    #[allow(clippy::too_many_arguments)]
    async fn record_one(
        &self,
        p: &crate::accounts::Principal,
        play: &Play,
        secret: &str,
        now: u64,
        started: Instant,
        lookups: &mut HashMap<u64, Result<crate::download::Track, Outcome>>,
        lms_down: &mut bool,
    ) -> Outcome {
        if let Err(reason) = check_time(play.played_at, play.seconds, now) {
            return Outcome::Refused(reason);
        }
        let accounts = self.accounts();
        match accounts.claim_play(
            p.user_id,
            p.device_id,
            &play.key,
            play.track_id,
            play.played_at,
            STALE_CLAIM,
        ) {
            Ok(PlayClaim::Claimed) => {}
            Ok(PlayClaim::Recorded) => return Outcome::Duplicate,
            Ok(PlayClaim::InFlight) => return Outcome::Retry("in_progress"),
            Err(_) => return Outcome::Retry("server_error"),
        }
        // Duplicates are answered even when LMS is down or the budget is spent.
        let outcome = if *lms_down {
            Outcome::Retry("server_unavailable")
        } else if started.elapsed() >= self.cfg.plays_budget {
            Outcome::Retry("busy")
        } else {
            self.forward(p, play, secret, started, lookups, lms_down)
                .await
        };
        let _ = match outcome {
            Outcome::Recorded | Outcome::Duplicate => accounts.finish_play(p.user_id, &play.key),
            _ => accounts.release_play(p.user_id, &play.key),
        };
        outcome
    }

    /// Look the track up, apply the counting rule and the rate limit, then have LMS record
    /// it. The key has been claimed.
    async fn forward(
        &self,
        p: &crate::accounts::Principal,
        play: &Play,
        secret: &str,
        started: Instant,
        lookups: &mut HashMap<u64, Result<crate::download::Track, Outcome>>,
        lms_down: &mut bool,
    ) -> Outcome {
        if let std::collections::hash_map::Entry::Vacant(slot) = lookups.entry(play.track_id) {
            let found = match self.library_track(play.track_id).await {
                Ok(t) => Ok(t),
                Err(Refusal::NotFound) => Err(Outcome::Refused("not_found")),
                Err(Refusal::NotDownloadable) => Err(Outcome::Refused("not_library")),
                Err(_) => {
                    *lms_down = true;
                    return Outcome::Retry("server_unavailable");
                }
            };
            slot.insert(found);
        }
        let track = match &lookups[&play.track_id] {
            Ok(t) => t,
            Err(o) => return *o,
        };
        if let Some(want) = &play.track_key {
            let Some(salt) = self.download_salt() else {
                return Outcome::Retry("server_error");
            };
            if crate::download::stable_key(&salt, &track.url) != *want {
                return Outcome::Refused("track_changed");
            }
        }
        if !counts(play.seconds, track.duration) {
            return Outcome::Refused("too_short");
        }
        // Two counted plays of one track can't start closer together than the time a play
        // needs to count: anything closer is the same listening sent twice under new keys
        // (pentest 2026-10: fresh keys could inflate a track's play count at will).
        let within = needed(track.duration).ceil() as u64;
        match self.accounts().play_overlaps(
            p.user_id,
            &play.key,
            play.track_id,
            play.played_at,
            within,
        ) {
            Ok(false) => {}
            Ok(true) => return Outcome::Refused("overlap"),
            Err(_) => return Outcome::Retry("server_error"),
        }
        if !self.plays.plays.take(p.user_id, Instant::now()) {
            return Outcome::Retry("rate_limited");
        }
        let ts = unix_now();
        let plugin_key = plugin_key(p.user_id, &play.key);
        let cmd = [
            "anywhere".to_string(),
            "play_record".into(),
            format!("track_id:{}", play.track_id),
            format!("played_at:{}", play.played_at),
            format!("key:{plugin_key}"),
            format!("ts:{ts}"),
            format!(
                "sig:{}",
                signature(secret, play.track_id, play.played_at, &plugin_key, ts)
            ),
        ];
        // Within what is left of the upload's budget.
        let limit = FORWARD_TIMEOUT
            .min(self.cfg.plays_budget.saturating_sub(started.elapsed()))
            .max(Duration::from_millis(200));
        let answer = match self.lms_command_within("", &cmd, limit).await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(error = %e, "could not record a play in LMS");
                *lms_down = true;
                return Outcome::Retry("server_unavailable");
            }
        };
        plugin_outcome(&answer)
    }
}

/// What the plugin's answer means.
pub fn plugin_outcome(answer: &Value) -> Outcome {
    let ok = match answer.get("ok") {
        Some(Value::Number(n)) => n.as_u64() == Some(1),
        Some(Value::String(s)) => s == "1",
        _ => false,
    };
    if ok {
        return Outcome::Recorded;
    }
    match answer.get("error").and_then(Value::as_str) {
        Some("duplicate") => Outcome::Duplicate,
        Some("not_found") => Outcome::Refused("not_found"),
        Some("not_library") => Outcome::Refused("not_library"),
        Some("busy") => Outcome::Retry("server_busy"),
        Some(e) => {
            let e: String = e.chars().filter(|c| !c.is_control()).take(100).collect();
            tracing::warn!(error = %e, "LMS refused to record a play");
            Outcome::Retry("server_error")
        }
        // No answer at all: the plugin (or this version of it) isn't installed.
        None => Outcome::Retry("unsupported"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn item(v: Value) -> Result<Play, BadItem> {
        parse_item(&v)
    }

    #[test]
    fn a_good_item_parses() {
        let p = item(json!({"key": "0f1e2d3c-aaaa", "track_id": 12, "played_at": NOW - 60, "seconds": 200.5})).unwrap();
        assert_eq!(p.key, "0f1e2d3c-aaaa");
        assert_eq!(p.track_id, 12);
        assert_eq!(p.seconds, 200.5);
        assert_eq!(p.track_key, None);
        let k = "a".repeat(64);
        let p = item(
            json!({"key": "abcdefgh", "track_id": 1, "played_at": 1, "seconds": 0, "track_key": k}),
        )
        .unwrap();
        assert_eq!(p.track_key.as_deref(), Some(k.as_str()));
    }

    #[test]
    fn hostile_items_are_invalid() {
        let base = json!({"key": "abcdefgh", "track_id": 1, "played_at": NOW, "seconds": 10});
        let with = |k: &str, v: Value| {
            let mut b = base.clone();
            b[k] = v;
            b
        };
        let cases = [
            with("key", json!("short")),
            with("key", json!("x".repeat(65))),
            with("key", json!("abc def gh")),
            with("key", json!("abcdefgh\n")),
            with("key", json!("../../etc/x")),
            with("key", json!(12345678)),
            with("track_id", json!(0)),
            with("track_id", json!(-1)),
            with("track_id", json!(1.5)),
            with("track_id", json!(1.0)),
            with("track_id", json!("1")),
            with("track_id", json!(1_000_000_000_000u64)),
            with("track_id", json!(u64::MAX)),
            with("track_id", Value::Null),
            with("played_at", json!(-5)),
            with("played_at", json!("now")),
            with("played_at", json!(1.5e9)),
            with("seconds", json!(-0.1)),
            with("seconds", json!(86_400.5)),
            with("seconds", json!("10")),
            with("seconds", json!(1e308)),
            with("track_key", json!("A".repeat(64))),
            with("track_key", json!("a".repeat(63))),
            with("track_key", json!(null)),
            with("extra", json!(1)),
            with("__proto__", json!({})),
            json!([]),
            json!("abcdefgh"),
            json!(null),
            json!({}),
        ];
        for c in cases {
            assert!(item(c.clone()).is_err(), "accepted {c}");
        }
        // The key is echoed only when it is itself valid.
        assert_eq!(
            item(with("seconds", json!(-1))).unwrap_err().key.as_deref(),
            Some("abcdefgh")
        );
        assert_eq!(item(with("key", json!("<script>"))).unwrap_err().key, None);
        let mut missing = base.clone();
        missing.as_object_mut().unwrap().remove("seconds");
        assert!(item(missing).is_err());
    }

    #[test]
    fn bodies_are_arrays_of_at_most_200() {
        assert_eq!(parse_body(b"{}").unwrap_err(), BodyError::Malformed);
        assert_eq!(parse_body(b"[]").unwrap_err(), BodyError::Malformed);
        assert_eq!(parse_body(b"").unwrap_err(), BodyError::Malformed);
        assert_eq!(parse_body(b"[1,").unwrap_err(), BodyError::Malformed);
        let many = format!("[{}]", vec!["1"; MAX_ITEMS + 1].join(","));
        assert_eq!(parse_body(many.as_bytes()).unwrap_err(), BodyError::TooMany);
        let ok = format!("[{}]", vec!["1"; MAX_ITEMS].join(","));
        assert_eq!(parse_body(ok.as_bytes()).unwrap().len(), MAX_ITEMS);
    }

    #[test]
    fn times_must_be_recent_and_not_in_the_future() {
        assert_eq!(check_time(NOW - 600, 300.0, NOW), Ok(()));
        assert_eq!(check_time(NOW + 299, 0.0, NOW), Ok(()), "skew allowed");
        assert_eq!(check_time(NOW + 301, 0.0, NOW), Err("in_future"));
        assert_eq!(
            check_time(NOW, 400.0, NOW),
            Err("in_future"),
            "ends in the future"
        );
        assert_eq!(check_time(u64::MAX, 0.0, NOW), Err("in_future"));
        let limit = MAX_AGE.as_secs();
        assert_eq!(check_time(NOW - limit, 0.0, NOW), Ok(()));
        assert_eq!(check_time(NOW - limit - 1, 0.0, NOW), Err("too_old"));
        assert_eq!(check_time(0, 0.0, NOW), Err("too_old"));
    }

    #[test]
    fn the_counting_rule_is_half_or_four_minutes() {
        assert!(counts(240.0, Some(3600.0)));
        assert!(!counts(239.9, Some(3600.0)));
        assert!(counts(60.0, Some(120.0)));
        assert!(!counts(59.0, Some(120.0)));
        assert!(counts(240.0, None));
        assert!(!counts(200.0, None));
        assert!(!counts(200.0, Some(0.0)), "unknown duration: four minutes");
        assert!(!counts(200.0, Some(f64::NAN)));
    }

    #[test]
    fn plugin_answers_are_mapped() {
        assert_eq!(plugin_outcome(&json!({"ok": 1})), Outcome::Recorded);
        assert_eq!(plugin_outcome(&json!({"ok": "1"})), Outcome::Recorded);
        assert_eq!(
            plugin_outcome(&json!({"ok": 2})),
            Outcome::Retry("unsupported")
        );
        assert_eq!(
            plugin_outcome(&json!({"error": "duplicate"})),
            Outcome::Duplicate
        );
        assert_eq!(
            plugin_outcome(&json!({"error": "not_found"})),
            Outcome::Refused("not_found")
        );
        assert_eq!(
            plugin_outcome(&json!({"error": "not_library"})),
            Outcome::Refused("not_library")
        );
        assert_eq!(
            plugin_outcome(&json!({"error": "busy"})),
            Outcome::Retry("server_busy")
        );
        assert_eq!(
            plugin_outcome(&json!({"error": "bad signature"})),
            Outcome::Retry("server_error")
        );
        assert_eq!(plugin_outcome(&json!({})), Outcome::Retry("unsupported"));
    }

    /// The same vector is checked by the plugin's tests (plugin/t/09-plays.t), so both sides
    /// agree on the signed message byte for byte.
    #[test]
    fn signature_matches_the_shared_test_vector() {
        let secret = "0123456789abcdef".repeat(4);
        assert_eq!(
            signature(&secret, 42, 1_790_000_000, "key-0001", 1_790_000_100),
            "de7d809202cfbfb878c1db65e10cbdb75f1885b20e7e4a25e05b5d73d2393806"
        );
    }

    #[test]
    fn windows_limit_per_user() {
        let w = Window::new(2, Duration::from_secs(60));
        let t = Instant::now();
        assert!(w.take(1, t));
        assert!(w.take(1, t));
        assert!(!w.take(1, t));
        assert!(w.take(2, t), "users are independent");
        assert!(w.take(1, t + Duration::from_secs(60)));
    }

    /// Robustness: arbitrary bytes and arbitrary JSON never panic the parser, and whatever
    /// it accepts obeys every rule.
    #[test]
    fn fuzz_the_parser() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let atoms = [
            "0",
            "1",
            "-1",
            "1.5",
            "1e400",
            "\"abcdefgh\"",
            "null",
            "true",
            "{}",
            "[]",
            "\"key\"",
            "\"track_id\"",
            "\"seconds\"",
            "\"played_at\"",
            "18446744073709551616",
            "999999999999",
            "1000000000000",
            "\"\\u0000\"",
            ":",
            ",",
            "[",
            "]",
            "{",
            "}",
        ];
        for _ in 0..20_000 {
            let n = (next() % 40) as usize;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(atoms[(next() % atoms.len() as u64) as usize]);
            }
            check(s.as_bytes());
            let raw: Vec<u8> = (0..(next() % 64)).map(|_| next() as u8).collect();
            check(&raw);
        }
        // Structured: well-formed items with random values.
        for _ in 0..20_000 {
            let pick = |r: u64| -> Value {
                match r % 9 {
                    0 => json!(r),
                    1 => json!(-(r as i64 % 1000)),
                    2 => json!((r % 100_000) as f64 / 7.0),
                    3 => json!("x".repeat((r % 70) as usize)),
                    4 => Value::Null,
                    5 => json!(r % 1_000_000_000_000),
                    6 => json!(format!("{:x}", r)),
                    7 => json!([r]),
                    _ => json!(true),
                }
            };
            let v = json!([{"key": pick(next()), "track_id": pick(next()), "played_at": pick(next()), "seconds": pick(next())}]);
            check(v.to_string().as_bytes());
        }
    }

    fn check(bytes: &[u8]) {
        if let Ok(items) = parse_body(bytes) {
            assert!(!items.is_empty() && items.len() <= MAX_ITEMS);
            for p in items.into_iter().flatten() {
                assert!(valid_key(&p.key));
                assert!((1..=MAX_TRACK_ID).contains(&p.track_id));
                assert!(p.seconds.is_finite() && (0.0..=MAX_SECONDS).contains(&p.seconds));
            }
        }
    }
}
