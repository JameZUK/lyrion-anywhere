//! Save a player's queue as a saved playlist, under a checked name.
//!
//! LMS's `playlist save <name>` writes `<name>.m3u` into its playlist folder, named by the
//! client, and silently overwrites a playlist already saved under that name. The RPC policy
//! never passes it on; this endpoint does the save with the name checked first.
//!
//! - `POST /v1/players/{player}/queue/save` `{"name":"Road trip","replace":false}`:
//!   200 `{"id":<playlist id>,"name":<name saved>}`;
//!   409 `{"error":…,"exists":{"id":…,"name":…}}` when a playlist of that name (compared
//!   ignoring case, since the folder may be on a case-insensitive filesystem) exists and
//!   `replace` isn't true; 409 `{"error":"the queue is empty"}` when there is nothing to save.
//!   With `replace`, the existing playlist is overwritten under its own spelling, so a
//!   different case on a case-sensitive filesystem doesn't leave two playlists of one name.
//!
//! **Who:** admins only (saved playlists are shared household data: listeners may play
//! them, not change them, as for `playlists new|rename|delete|edit`), for a player they may
//! control (as for queue edits). Other players look like unknown ones (403).
//!
//! **What's saved** is LMS's own: library tracks by their file, and radio or service
//! streams in the queue by their URL, as LMS saves them from its own web pages. The
//! gateway never returns those URLs (`playlists tracks` is filtered as ever).

use std::sync::OnceLock;

use anywhere_core::names;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::AppState;
use crate::accounts::Role;
use crate::history::{body_json, caller, error, ok};
use crate::rpc_policy;

/// Largest request body (a 100-character name, every character escaped, fits).
pub const MAX_BODY: usize = 1024;
/// Longest name, in characters.
pub const MAX_NAME: usize = names::PLAYLIST_NAME_MAX;
/// Saved playlists of a similar name looked through for an exact match.
const SEARCH_ROWS: u32 = 1_000;

/// The name a playlist may be saved under (trimmed), or why not: the shared rules
/// ([`names::playlist_name`]: length, control and invisible characters, path separators,
/// `..`, leading or trailing dots, characters filesystems reject, Windows device names),
/// and never anything LMS could take for a location.
pub fn check_name(raw: &str) -> Result<String, &'static str> {
    let name = names::playlist_name(raw)?;
    if !rpc_policy::text_ok(&name) {
        return Err("that name isn't allowed");
    }
    Ok(name)
}

/// A save request: the checked name and whether an existing playlist may be overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub name: String,
    pub replace: bool,
}

/// The save in a request body, or why not.
pub fn parse(body: &Value) -> Result<Save, &'static str> {
    let o = body.as_object().ok_or("expected an object")?;
    if o.keys().any(|k| k != "name" && k != "replace") {
        return Err("unexpected fields");
    }
    let name = o
        .get("name")
        .and_then(Value::as_str)
        .ok_or("expected a name")?;
    let replace = match o.get("replace") {
        None => false,
        Some(v) => v.as_bool().ok_or("replace must be true or false")?,
    };
    Ok(Save {
        name: check_name(name)?,
        replace,
    })
}

/// A number LMS sent as a number or a string.
fn number(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// The saved playlist in LMS's `playlists` answer whose name is `name` ignoring case.
fn same_name(list: &Value, name: &str) -> Option<(u64, String)> {
    let want = name.to_lowercase();
    list.get("playlists_loop")?
        .as_array()?
        .iter()
        .find_map(|p| {
            let n = p.get("playlist")?.as_str()?;
            if n.trim().to_lowercase() != want {
                return None;
            }
            Some((number(p.get("id")?)?, n.to_string()))
        })
}

/// How many entries LMS's `status` answer says the queue holds.
fn queue_len(status: &Value) -> u64 {
    status
        .get("playlist_tracks")
        .and_then(number)
        .unwrap_or_else(|| {
            status
                .get("playlist_loop")
                .and_then(Value::as_array)
                .map_or(0, |l| l.len() as u64)
        })
}

/// LMS couldn't write the playlist file (no playlist folder set, or it isn't writable).
fn write_failed(result: &Value) -> bool {
    result
        .get("writeError")
        .is_some_and(|v| v.as_bool().unwrap_or(true) && number(v) != Some(0))
}

fn unavailable() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "the server didn't answer")
}

/// `POST /v1/players/{player}/queue/save`
pub async fn save(
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
    // Saved playlists are shared household data: listeners may play them, not change them.
    if p.role != Role::Admin {
        return error(
            StatusCode::FORBIDDEN,
            "changing playlists needs the admin role",
        );
    }
    let body = match body_json(&headers, body, MAX_BODY).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let save = match parse(&body) {
        Ok(s) => s,
        Err(why) => return error(StatusCode::BAD_REQUEST, why),
    };

    // One save at a time (names are shared by every player), and none while the gateway
    // edits this queue.
    static SAVING: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _saving = SAVING.get_or_init(Default::default).lock().await;
    let lock = crate::queue_edit::lock_for(&player);
    let _held = lock.lock().await;

    let status = match gw
        .lms_command(&player, &["status".into(), "0".into(), "1".into()])
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "queue check failed");
            return unavailable();
        }
    };
    if queue_len(&status) == 0 {
        return error(StatusCode::CONFLICT, "the queue is empty");
    }

    let find = |name: String| {
        let gw = gw.clone();
        async move {
            let cmd = [
                "playlists".to_string(),
                "0".into(),
                SEARCH_ROWS.to_string(),
                format!("search:{name}"),
            ];
            gw.lms_command("", &cmd)
                .await
                .map(|list| same_name(&list, &name))
        }
    };
    let mut name = save.name;
    match find(name.clone()).await {
        Err(e) => {
            tracing::warn!(error = %e, "playlist lookup failed");
            return unavailable();
        }
        Ok(Some((id, existing))) if !save.replace => {
            let mut r = (
                StatusCode::CONFLICT,
                axum::Json(json!({
                    "error": "a playlist with that name already exists",
                    "exists": { "id": id, "name": existing },
                })),
            )
                .into_response();
            r.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("private, no-store"),
            );
            return r;
        }
        // Overwrite the existing file, not a second one differing only in case.
        Ok(Some((_, existing))) => name = existing,
        Ok(None) => {}
    }

    let saved = match gw
        .lms_command(&player, &["playlist".into(), "save".into(), name.clone()])
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "playlist save failed");
            return unavailable();
        }
    };
    if write_failed(&saved) {
        return error(
            StatusCode::BAD_GATEWAY,
            "the server couldn't save the playlist",
        );
    }
    let id = match saved.get("__playlist_id").and_then(number) {
        Some(id) => Some(id),
        None => find(name.clone()).await.ok().flatten().map(|(id, _)| id),
    };
    match id {
        Some(id) => ok(json!({ "id": id, "name": name })),
        None => error(
            StatusCode::BAD_GATEWAY,
            "the server couldn't save the playlist",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        for (raw, want) in [
            ("Road trip", "Road trip"),
            ("  Sunday morning \t", "Sunday morning"),
            ("Sigur Rós – live", "Sigur Rós – live"),
            ("Café", "Café"),
            ("日本語のプレイリスト", "日本語のプレイリスト"),
            ("Mix vol. 2", "Mix vol. 2"),
            ("AC-DC & friends (2026)", "AC-DC & friends (2026)"),
            ("Console", "Console"),
        ] {
            assert_eq!(check_name(raw).as_deref(), Ok(want), "{raw:?}");
        }
        assert!(
            check_name(&"é".repeat(100)).is_ok(),
            "100 characters, not bytes"
        );
        for bad in [
            "",
            "   ",
            "../etc/passwd",
            "..",
            "a..b",
            "a/b",
            "AC/DC",
            "a\\b",
            "C:\\x",
            "name:x",
            "silent:1",
            "a*b",
            "what?",
            "say \"hi\"",
            "<b>",
            "a|b",
            ".hidden",
            "trailing.",
            "line\nbreak",
            "tab\tinside",
            "nul\0",
            "esc\u{1b}[31m",
            "del\u{7f}",
            "rtl\u{202E}gpj.exe",
            "zero\u{200B}width",
            "bom\u{FEFF}",
            "CON",
            "nul.txt",
            "Lpt1",
            "file:///etc/passwd",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name(&"a".repeat(101)).is_err());
    }

    #[test]
    fn requests_are_parsed_strictly() {
        assert_eq!(
            parse(&json!({"name": " Road trip "})),
            Ok(Save {
                name: "Road trip".into(),
                replace: false
            })
        );
        assert_eq!(
            parse(&json!({"name": "x", "replace": true})),
            Ok(Save {
                name: "x".into(),
                replace: true
            })
        );
        for bad in [
            json!(null),
            json!("Road trip"),
            json!([]),
            json!({}),
            json!({"name": 5}),
            json!({"name": ["x"]}),
            json!({"name": "x", "replace": "yes"}),
            json!({"name": "x", "replace": 1}),
            json!({"name": "x", "player": "aa:bb:cc:dd:ee:01"}),
            json!({"name": "x", "cmd": ["playlist", "save", "y"]}),
            json!({"name": "../x"}),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn existing_names_match_ignoring_case() {
        let list = json!({"playlists_loop": [
            {"id": 3, "playlist": "Road trip 2"},
            {"id": "5", "playlist": "ROAD TRIP"},
        ]});
        assert_eq!(same_name(&list, "road trip"), Some((5, "ROAD TRIP".into())));
        assert_eq!(same_name(&list, "Road"), None, "a search hit isn't a match");
        for junk in [
            json!({}),
            json!({"playlists_loop": "x"}),
            json!({"playlists_loop": [{"id": 1}]}),
            json!({"playlists_loop": [{"playlist": "road trip"}]}),
            json!({"playlists_loop": [{"playlist": "road trip", "id": "x"}]}),
        ] {
            assert_eq!(same_name(&junk, "road trip"), None, "{junk}");
        }
    }

    #[test]
    fn queue_length_and_write_errors_are_read() {
        assert_eq!(queue_len(&json!({"playlist_tracks": 12})), 12);
        assert_eq!(queue_len(&json!({"playlist_tracks": "3"})), 3);
        assert_eq!(queue_len(&json!({"playlist_tracks": 0})), 0);
        assert_eq!(queue_len(&json!({"playlist_loop": [{"id": 1}]})), 1);
        assert_eq!(queue_len(&json!({})), 0);
        assert!(write_failed(&json!({"writeError": 1})));
        assert!(write_failed(&json!({"writeError": "1"})));
        assert!(write_failed(&json!({"writeError": true})));
        assert!(!write_failed(&json!({"writeError": 0})));
        assert!(!write_failed(&json!({"__playlist_id": 4})));
    }

    /// The name check is fed arbitrary text and JSON: it must answer, never panic, and
    /// anything it accepts must keep every rule.
    #[test]
    fn the_name_check_never_panics() {
        let mut seed: u64 = 0x5eed_1234_abcd_ef01;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let alphabet: Vec<char> = "ab Z.é/\\:*?\"<>|\0\n\t\u{7f}\u{202E}\u{200B}日 ..-_~%"
            .chars()
            .chain(['\u{10FFFF}', '\u{FFFD}', '\u{0301}', 'C', 'O', 'N'])
            .collect();
        for _ in 0..20_000 {
            let len = (next() % 130) as usize;
            let s: String = (0..len)
                .map(|_| {
                    if next() % 5 == 0 {
                        char::from_u32((next() % 0x11_0000) as u32).unwrap_or('x')
                    } else {
                        alphabet[(next() as usize) % alphabet.len()]
                    }
                })
                .collect();
            if let Ok(n) = check_name(&s) {
                assert!(!n.is_empty() && n.chars().count() <= MAX_NAME, "{n:?}");
                assert!(!n.contains(names::RESERVED) && !n.contains(".."), "{n:?}");
                assert!(!n.starts_with('.') && !n.ends_with('.'), "{n:?}");
                assert!(
                    !n.chars().any(|c| c.is_control() || names::invisible(c)),
                    "{n:?}"
                );
                assert_eq!(n, n.trim());
            }
            let _ = parse(&json!({ "name": s, "replace": next() % 2 == 0 }));
            let _ = parse(&serde_json::from_str::<Value>(&s).unwrap_or(Value::Null));
        }
    }
}
