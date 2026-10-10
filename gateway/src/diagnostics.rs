//! `POST /v1/diagnostics`: a device reports how it plays (browser capabilities, chosen
//! playback mode, player status, recent connection log), for troubleshooting by an admin.
//!
//! Reports are untrusted. They are size-capped, then rebuilt: only bounded objects, arrays,
//! numbers, booleans and short printable strings survive, under simple keys. The latest report
//! per device is kept, and removed with the device.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::AppState;

/// Largest report accepted, before sanitising.
pub const MAX_BODY: usize = 32 * 1024;
const MAX_DEPTH: usize = 4;
const MAX_ITEMS: usize = 200; // keys per object, entries per array
const MAX_STRING: usize = 600;
const MAX_NODES: usize = 3000;
/// At most one stored update per device in this time.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);

fn key_ok(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 48
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// Rebuild an untrusted report from safe pieces. `None` if it isn't a JSON object.
pub fn sanitise(v: &Value) -> Option<Value> {
    let mut nodes = 0;
    match v {
        Value::Object(_) => Some(clean(v, 0, &mut nodes)),
        _ => None,
    }
}

fn clean(v: &Value, depth: usize, nodes: &mut usize) -> Value {
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Value::Null;
    }
    match v {
        Value::Null | Value::Bool(_) => v.clone(),
        Value::Number(n) => {
            if n.as_f64().is_some_and(f64::is_finite) {
                v.clone()
            } else {
                Value::Null
            }
        }
        Value::String(s) => {
            let mut t: String = s
                .chars()
                .filter(|c| !c.is_control() || *c == ' ')
                .take(MAX_STRING)
                .collect();
            if s.chars().count() > MAX_STRING {
                t.push('…');
            }
            Value::String(t)
        }
        Value::Array(a) if depth < MAX_DEPTH => Value::Array(
            a.iter()
                .take(MAX_ITEMS)
                .map(|x| clean(x, depth + 1, nodes))
                .collect(),
        ),
        Value::Object(o) if depth < MAX_DEPTH => {
            let mut m = Map::new();
            for (k, x) in o.iter().filter(|(k, _)| key_ok(k)).take(MAX_ITEMS) {
                m.insert(k.clone(), clean(x, depth + 1, nodes));
            }
            Value::Object(m)
        }
        _ => Value::Null, // too deep
    }
}

/// Per-device rate limit for stored updates.
#[derive(Default)]
pub struct Throttle(Mutex<HashMap<i64, Instant>>);

impl Throttle {
    /// May this device store a report now?
    pub fn allow(&self, device: i64, now: Instant) -> bool {
        let mut m = self.0.lock().unwrap();
        if m.len() > 10_000 {
            m.retain(|_, t| now.duration_since(*t) < MIN_INTERVAL);
        }
        match m.get(&device) {
            Some(t) if now.duration_since(*t) < MIN_INTERVAL => false,
            _ => {
                m.insert(device, now);
                true
            }
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub async fn report(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(p) = crate::auth::principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    if body.len() > MAX_BODY {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "report too large");
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return error(StatusCode::BAD_REQUEST, "malformed report");
    };
    let Some(clean) = sanitise(&v) else {
        return error(StatusCode::BAD_REQUEST, "malformed report");
    };
    if !gw.diagnostics_throttle.allow(p.device_id, Instant::now()) {
        // Accepted but not stored: the device reports again later.
        return StatusCode::ACCEPTED.into_response();
    }
    match gw
        .accounts()
        .save_diagnostics(p.device_id, &clean.to_string())
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_keep_their_shape_and_values() {
        let r = json!({"playback": {"mode": "media", "reason": "iPad"}, "caps": {"mms": true, "types": [{"type": "audio/mp4; codecs=\"fLaC\"", "managed": true}]}, "memory": 48.5});
        assert_eq!(sanitise(&r).unwrap(), r);
    }

    #[test]
    fn hostile_reports_are_cut_down() {
        let deep = json!({"a": {"b": {"c": {"d": {"e": "too deep"}}}}});
        assert_eq!(sanitise(&deep).unwrap()["a"]["b"]["c"]["d"], Value::Null);
        let long = json!({"s": "x".repeat(10_000)});
        assert!(
            sanitise(&long).unwrap()["s"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= MAX_STRING + 1
        );
        let ctrl = json!({"s": "a\u{0}b\u{1b}[31mc\nd"});
        assert_eq!(sanitise(&ctrl).unwrap()["s"], "ab[31mcd");
        let keys = json!({"<script>": 1, "ok_key": 2, "": 3, "a b": 4});
        assert_eq!(sanitise(&keys).unwrap(), json!({"ok_key": 2}));
        let wide: Value = (0..1000).map(Value::from).collect::<Vec<_>>().into();
        assert_eq!(
            sanitise(&json!({"w": wide})).unwrap()["w"]
                .as_array()
                .unwrap()
                .len(),
            MAX_ITEMS
        );
        for not_object in [json!([]), json!("x"), json!(1), Value::Null] {
            assert!(sanitise(&not_object).is_none());
        }
    }

    #[test]
    fn many_nodes_are_capped() {
        let big: Value = (0..150).map(|_| json!({"a": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]})).collect::<Vec<_>>().into();
        let out = sanitise(&json!({"x": big})).unwrap().to_string();
        assert!(
            out.matches("null").count() > 0,
            "nodes beyond the cap are dropped"
        );
    }

    #[test]
    fn throttle_allows_one_update_per_interval() {
        let t = Throttle::default();
        let t0 = Instant::now();
        assert!(t.allow(1, t0));
        assert!(!t.allow(1, t0 + Duration::from_secs(3)));
        assert!(t.allow(2, t0), "other devices are independent");
        assert!(t.allow(1, t0 + MIN_INTERVAL));
    }
}
