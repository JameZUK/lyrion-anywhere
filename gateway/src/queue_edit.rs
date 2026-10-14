//! Checked queue edits: remove or move a queue entry only if it is still the track the client
//! saw there.
//!
//! LMS's queue commands take a position (`playlist delete <index>`, `playlist move <from>
//! <to>`); by the time a client's command arrives, another controller may have changed the
//! queue, and the position may hold a different track. LMS has no "delete this entry" by
//! identity (`playlistcontrol cmd:delete track_id:` removes every copy of a track). So the
//! gateway looks the entry up and changes it in one step, holding a per-player lock, so
//! edits made through it can't interleave, and the window left for other controllers is
//! the few milliseconds between two local LMS commands rather than a network round trip.
//!
//! - `POST /v1/players/{player}/queue/edit`
//!   `{"op":"remove","index":i,"track":id}` or `{"op":"move","index":i,"to":j,"track":id}`:
//!   200 `{"done":true}`, or 409 `{"error":"the queue changed"}` when the entry at `index`
//!   isn't `track` (nothing is changed).
//!
//! **Who:** anyone who may control the player (as for alarms), within the per-user `/v1/me`
//! allowance. Other players look like unknown ones (403).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};

use crate::AppState;
use crate::history::{body_json, caller, error, ok};
use crate::rpc_policy;

/// Largest request body.
pub const MAX_BODY: usize = 256;
/// Highest queue position accepted (LMS queues are far shorter).
pub const MAX_INDEX: u64 = 100_000;
/// Locks kept before unused ones are dropped.
const MAX_LOCKS: usize = 1_000;

type Locks = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

/// One lock per player: edits through the gateway to one queue run one at a time.
pub(crate) fn lock_for(player: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    let mut m = LOCKS.get_or_init(Default::default).lock().unwrap();
    if m.len() >= MAX_LOCKS {
        m.retain(|_, l| Arc::strong_count(l) > 1);
    }
    m.entry(player.to_string()).or_default().clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Remove { index: u64, track: i64 },
    Move { index: u64, to: u64, track: i64 },
}

/// The edit in a request body, or why not.
pub fn parse(body: &Value) -> Result<Edit, &'static str> {
    let o = body.as_object().ok_or("expected an object")?;
    let index = |k: &str| {
        o.get(k)
            .and_then(Value::as_u64)
            .filter(|n| *n <= MAX_INDEX)
            .ok_or("positions are 0–100000")
    };
    let track = o
        .get("track")
        .and_then(Value::as_i64)
        .filter(|t| *t != 0 && t.unsigned_abs() <= 999_999_999_999)
        .ok_or("expected the track id seen at that position")?;
    match o.get("op").and_then(Value::as_str) {
        Some("remove") if o.len() == 3 => Ok(Edit::Remove {
            index: index("index")?,
            track,
        }),
        Some("move") if o.len() == 4 => {
            let (index, to) = (index("index")?, index("to")?);
            if index == to {
                return Err("nothing to move");
            }
            Ok(Edit::Move { index, to, track })
        }
        Some("remove" | "move") => Err("unexpected fields"),
        _ => Err("op must be remove or move"),
    }
}

/// The track id at `index` in LMS's answer to `status <index> 1`, if that entry is there.
fn track_at(status: &Value, index: u64) -> Option<i64> {
    let e = status.get("playlist_loop")?.as_array()?.first()?;
    let at = e.get("playlist index").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })?;
    if at != index {
        return None;
    }
    e.get("id").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

/// `POST /v1/players/{player}/queue/edit`
pub async fn edit(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(player): Path<String>,
    body: Body,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let player = player.to_ascii_lowercase();
    let allowed = gw
        .accounts()
        .player_scope(&p)
        .is_ok_and(|s| rpc_policy::is_player_id(&player) && s.allows(&player));
    if !allowed {
        return error(StatusCode::FORBIDDEN, "you may not control this player");
    }
    let body = match body_json(&headers, body, MAX_BODY).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let edit = match parse(&body) {
        Ok(e) => e,
        Err(why) => return error(StatusCode::BAD_REQUEST, why),
    };
    let (index, track) = match edit {
        Edit::Remove { index, track } | Edit::Move { index, track, .. } => (index, track),
    };
    let lock = lock_for(&player);
    let _held = lock.lock().await;
    let lms = |cmd: Vec<String>| {
        let gw = gw.clone();
        let player = player.clone();
        async move { gw.lms_command(&player, &cmd).await }
    };
    let status = match lms(vec!["status".into(), index.to_string(), "1".into()]).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "queue check failed");
            return error(StatusCode::SERVICE_UNAVAILABLE, "the server didn't answer");
        }
    };
    if track_at(&status, index) != Some(track) {
        return error(StatusCode::CONFLICT, "the queue changed");
    }
    let cmd = match edit {
        Edit::Remove { index, .. } => vec!["playlist".into(), "delete".into(), index.to_string()],
        Edit::Move { index, to, .. } => vec![
            "playlist".into(),
            "move".into(),
            index.to_string(),
            to.to_string(),
        ],
    };
    match lms(cmd).await {
        Ok(_) => ok(json!({ "done": true })),
        Err(e) => {
            tracing::warn!(error = %e, "queue edit failed");
            error(StatusCode::SERVICE_UNAVAILABLE, "the server didn't answer")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_are_parsed_strictly() {
        assert_eq!(
            parse(&json!({"op":"remove","index":3,"track":42})),
            Ok(Edit::Remove {
                index: 3,
                track: 42
            })
        );
        assert_eq!(
            parse(&json!({"op":"move","index":5,"to":1,"track":-7})),
            Ok(Edit::Move {
                index: 5,
                to: 1,
                track: -7
            })
        );
        for bad in [
            json!(null),
            json!([]),
            json!({"op":"remove","index":3}),
            json!({"op":"remove","index":-1,"track":4}),
            json!({"op":"remove","index":"3","track":4}),
            json!({"op":"remove","index":3.5,"track":4}),
            json!({"op":"remove","index":100_001,"track":4}),
            json!({"op":"remove","index":3,"track":0}),
            json!({"op":"remove","index":3,"track":"4"}),
            json!({"op":"remove","index":3,"track":1_000_000_000_000_i64}),
            json!({"op":"remove","index":3,"track":4,"extra":1}),
            json!({"op":"move","index":3,"track":4}),
            json!({"op":"move","index":3,"to":3,"track":4}),
            json!({"op":"clear","index":3,"track":4}),
            json!({"op":["remove"],"index":3,"track":4}),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_entry_must_be_the_one_asked_for() {
        let s = json!({"playlist_loop": [{"playlist index": 4, "id": 9}]});
        assert_eq!(track_at(&s, 4), Some(9));
        assert_eq!(track_at(&s, 3), None, "LMS answered a different position");
        let strs = json!({"playlist_loop": [{"playlist index": "4", "id": "-9"}]});
        assert_eq!(track_at(&strs, 4), Some(-9));
        for junk in [
            json!({}),
            json!({"playlist_loop": []}),
            json!({"playlist_loop": [{"id": 9}]}),
            json!({"playlist_loop": [{"playlist index": 4}]}),
            json!({"playlist_loop": "x"}),
        ] {
            assert_eq!(track_at(&junk, 4), None, "{junk}");
        }
    }
}
