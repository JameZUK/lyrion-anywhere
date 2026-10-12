//! Lyrics (docs/plans/wave1-design.md §9): `GET /v1/lyrics/track/{id}`.
//!
//! - **Who:** any signed-in user, within their `/v1/me/…` allowance ([`history::caller`]).
//! - **Request:** a library track id and nothing else (a positive integer; no query string).
//!   No path, URL, artist or title is ever taken from the client.
//! - **Sources, in order:**
//!   1. the lyrics LMS read from the file's tags (`titles … tags:w`);
//!   2. the Music & Artist Information plugin, when installed
//!      (`musicartistinfo lyrics track_id:<id>`): its local and cached lyrics, and whatever
//!      online services the server's owner has enabled in that plugin. We call no lyrics
//!      service ourselves.
//! - **Answer:** parsed by the core ([`anywhere_core::lyrics`]):
//!   `{"source": "tags"|"plugin", "synced": bool, "lines": [{"t": ms, "text": "…"}]}`, or
//!   404 `{"error": "no lyrics"}`. Lines that look like a server path or a LAN address are
//!   dropped, as everywhere else ([`rpc_policy::sensitive_text`]).
//! - **Limits:** at most [`PLUGIN_LOOKUPS`] plugin lookups at once (the plugin may go online;
//!   beyond that, 429 with `Retry-After`), each within [`PLUGIN_TIMEOUT`]. Answers (and "no
//!   lyrics") are remembered for [`CACHE_TTL`], at most [`CACHE_ENTRIES`] tracks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anywhere_core::lyrics::Lyrics;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::AppState;
use crate::history::{caller, error, ok};
use crate::{download, rpc_policy};

/// Plugin lookups running at once, for everyone.
pub const PLUGIN_LOOKUPS: usize = 4;
/// Time allowed for one plugin lookup.
pub const PLUGIN_TIMEOUT: Duration = Duration::from_secs(12);
/// How long an answer is remembered.
pub const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
/// Tracks remembered, at most.
pub const CACHE_ENTRIES: usize = 512;

/// A remembered answer and when it was made (`None`: no lyrics).
type Entry = (Instant, Option<Arc<Value>>);

/// Remembered answers (`None`: no lyrics) and the plugin lookup limit.
pub struct LyricsCache {
    entries: Mutex<HashMap<u64, Entry>>,
    plugin: Semaphore,
}

impl Default for LyricsCache {
    fn default() -> Self {
        LyricsCache {
            entries: Mutex::new(HashMap::new()),
            plugin: Semaphore::new(PLUGIN_LOOKUPS),
        }
    }
}

impl LyricsCache {
    fn get(&self, id: u64, now: Instant) -> Option<Option<Arc<Value>>> {
        let e = self.entries.lock().unwrap();
        e.get(&id)
            .filter(|(at, _)| now.duration_since(*at) < CACHE_TTL)
            .map(|(_, v)| v.clone())
    }

    fn put(&self, id: u64, v: Option<Arc<Value>>, now: Instant) {
        let mut e = self.entries.lock().unwrap();
        if e.len() >= CACHE_ENTRIES && !e.contains_key(&id) {
            e.retain(|_, (at, _)| now.duration_since(*at) < CACHE_TTL);
            if e.len() >= CACHE_ENTRIES {
                // Still full of fresh entries: drop the oldest.
                if let Some(oldest) = e.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| *k) {
                    e.remove(&oldest);
                }
            }
        }
        e.insert(id, (now, v));
    }
}

/// The answer for lyrics text from `source`, or `None` if it has no words.
pub fn answer(source: &str, text: &str) -> Option<Value> {
    let mut l = Lyrics::parse(text);
    l.lines.retain(|x| !rpc_policy::sensitive_text(&x.text));
    if l.is_empty() {
        return None;
    }
    let lines: Vec<Value> = l
        .lines
        .iter()
        .map(|x| json!({ "t": x.time_ms, "text": x.text }))
        .collect();
    Some(json!({ "source": source, "synced": l.synced, "lines": lines }))
}

fn text_of(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// `GET /v1/lyrics/track/{id}`
pub async fn track(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Err(r) = caller(&gw, &headers) {
        return r;
    }
    let Some(id) = download::parse_track_id(&id) else {
        return error(StatusCode::NOT_FOUND, "no such track");
    };
    if query.is_some() {
        return error(StatusCode::BAD_REQUEST, "unknown parameter");
    }
    let now = Instant::now();
    if let Some(hit) = gw.lyrics.get(id, now) {
        return respond(hit);
    }

    // 1. The file's tags.
    let cmd = [
        "titles".to_string(),
        "0".into(),
        "1".into(),
        format!("track_id:{id}"),
        "tags:w".into(),
    ];
    let item = match gw
        .lms_query_retrying("", &cmd, gw.cfg.download_lookup_timeout)
        .await
    {
        Ok(r) => r
            .get("titles_loop")
            .and_then(Value::as_array)
            .and_then(|l| l.first())
            .cloned(),
        Err(e) => {
            tracing::warn!(error = %e, "lyrics lookup failed");
            return error(StatusCode::SERVICE_UNAVAILABLE, "the server didn't answer");
        }
    };
    let Some(item) = item.filter(|t| download::int(t, "id") == Some(id as i64)) else {
        return error(StatusCode::NOT_FOUND, "no such track");
    };
    if let Some(v) = text_of(&item, "lyrics").and_then(|t| answer("tags", &t)) {
        let v = Arc::new(v);
        gw.lyrics.put(id, Some(v.clone()), now);
        return respond(Some(v));
    }

    // 2. The Music & Artist Information plugin, if it is there.
    let Ok(_permit) = gw.lyrics.plugin.try_acquire() else {
        let mut r = error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many lyrics lookups; try again shortly",
        );
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        return r;
    };
    let cmd = [
        "musicartistinfo".to_string(),
        "lyrics".into(),
        format!("track_id:{id}"),
    ];
    let found = match gw.lms_command_within("", &cmd, PLUGIN_TIMEOUT).await {
        Ok(r) => text_of(&r, "lyrics").and_then(|t| answer("plugin", &t)),
        Err(e) => {
            // Not installed answers with nothing; this is a failure or a timeout, so don't
            // remember it.
            tracing::debug!(error = %e, "lyrics plugin lookup failed");
            return respond(None);
        }
    };
    let found = found.map(Arc::new);
    gw.lyrics.put(id, found.clone(), Instant::now());
    respond(found)
}

fn respond(v: Option<Arc<Value>>) -> Response {
    match v {
        Some(v) => ok((*v).clone()),
        None => error(StatusCode::NOT_FOUND, "no lyrics"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_shapes() {
        let v = answer("tags", "[00:01.00]One\n[00:02.50]Two\n").unwrap();
        assert_eq!(
            v,
            json!({"source": "tags", "synced": true,
                   "lines": [{"t": 1000, "text": "One"}, {"t": 2500, "text": "Two"}]})
        );
        let v = answer("plugin", "Plain\nwords").unwrap();
        assert_eq!(v["synced"], json!(false));
        assert_eq!(v["lines"][1], json!({"t": 0, "text": "words"}));
        assert_eq!(answer("tags", "  \n[ar:x]\n"), None);
    }

    #[test]
    fn paths_and_addresses_are_dropped() {
        let v = answer(
            "tags",
            "Fine line\nfile:///srv/music/a.flac\nsee http://10.0.0.10:9000/x\n/srv/music/Artist/x.lrc\nAC/DC rocks",
        )
        .unwrap();
        let texts: Vec<&str> = v["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, vec!["Fine line", "AC/DC rocks"]);
        assert_eq!(answer("tags", "/srv/music/a.flac"), None);
    }

    #[test]
    fn cache_expires_and_is_bounded() {
        let c = LyricsCache::default();
        let t0 = Instant::now();
        c.put(1, None, t0);
        assert_eq!(c.get(1, t0), Some(None));
        assert_eq!(c.get(1, t0 + CACHE_TTL), None);
        for i in 0..(CACHE_ENTRIES as u64 * 2) {
            c.put(
                i + 10,
                Some(Arc::new(json!(i))),
                t0 + Duration::from_millis(i),
            );
        }
        assert!(c.entries.lock().unwrap().len() <= CACHE_ENTRIES);
        // The newest is kept.
        let last = CACHE_ENTRIES as u64 * 2 + 9;
        assert!(c.get(last, t0 + Duration::from_secs(1)).is_some());
    }
}
