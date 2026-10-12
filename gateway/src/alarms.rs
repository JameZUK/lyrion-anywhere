//! Alarms (docs/plans/wave2-design.md §1): LMS's own alarms for a player, behind typed
//! endpoints so the grammar lives in one place.
//!
//! - `GET /v1/players/{player}/alarms`: the player's alarms, its "all alarms" switch and the
//!   sounds an alarm can play.
//! - `POST /v1/players/{player}/alarms`: add one.
//! - `PATCH /v1/players/{player}/alarms/{id}`: change one; `DELETE` removes it.
//! - `PUT /v1/players/{player}/alarms/enabled`: `{"enabled": bool}` for all of them.
//!
//! **Who:** anyone who may control the player (admins: every player), within the per-user
//! `/v1/me/…` allowance. Other players look exactly like unknown ones (403).
//!
//! **Sounds are never URLs from the client.** LMS lists what an alarm may play
//! (`alarm playlists`: the current queue, favourites, saved playlists, random mixes, natural
//! sounds). The gateway names each by an opaque id (a salted hash of LMS's URL; `current`
//! for the current queue) and maps an id back to a URL only by asking LMS for the list again,
//! so only a URL LMS itself offers can be set. LMS's URLs are never returned: an alarm's
//! sound is reported by id, category and title.
//!
//! **Values:** time 0–86 399 s after midnight; days 0–6 (0 is Sunday), each once, at least
//! one; volume 0–100; shuffle 0 (off), 1 (songs), 2 (albums); ids of 1–32 lowercase hex.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::history::{body_json, caller, error, ok};
use crate::rpc_policy;

/// Largest request body.
pub const MAX_BODY: usize = 2048;
/// Most sounds listed.
pub const MAX_SOUNDS: usize = 500;
/// Most alarms listed for one player.
pub const MAX_ALARMS: usize = 64;
/// The id of "the current queue".
pub const CURRENT: &str = "current";

/// One sound an alarm can play.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub id: String,
    pub category: String,
    pub title: String,
    /// LMS's URL (`None`: the current queue). Never sent to clients.
    url: Option<String>,
}

/// A sound's opaque id: the salted hash of its URL.
pub fn sound_id(salt: &str, url: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b"\0alarm-sound\0");
    h.update(url.as_bytes());
    hex::encode(&h.finalize()[..8])
}

/// A title for clients: control characters dropped, at most 200 characters, and blank if
/// it looks like a server path or a LAN address.
fn clean(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(200).collect();
    let t = t.trim().to_string();
    if rpc_policy::sensitive_text(&t) {
        String::new()
    } else {
        t
    }
}

/// The sounds in LMS's `alarm playlists` answer.
pub fn parse_sounds(v: &Value, salt: &str) -> Vec<Sound> {
    let mut out = vec![];
    for item in v
        .get("item_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_SOUNDS)
    {
        let category = item.get("category").and_then(Value::as_str).unwrap_or("");
        let category = match category {
            "CURRENT_PLAYLIST" => "current",
            "FAVORITES" => "favourites",
            "PLAYLISTS" => "playlists",
            "RANDOM" => "random",
            "NATURAL" | "SOUNDS" => "sounds",
            _ => "other",
        };
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .map(clean)
            .unwrap_or_default();
        match item
            .get("url")
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
        {
            None if category == "current" => out.push(Sound {
                id: CURRENT.into(),
                category: category.into(),
                title: if title.is_empty() || title.starts_with('{') {
                    "Current queue".into()
                } else {
                    title
                },
                url: None,
            }),
            None => {}
            Some(url) => {
                let id = sound_id(salt, url);
                if out.iter().any(|s: &Sound| s.id == id) {
                    continue;
                }
                out.push(Sound {
                    id,
                    category: category.into(),
                    title: if title.is_empty() {
                        "Untitled".into()
                    } else {
                        title
                    },
                    url: Some(url.to_string()),
                });
            }
        }
    }
    if !out.iter().any(|s| s.id == CURRENT) {
        out.insert(
            0,
            Sound {
                id: CURRENT.into(),
                category: "current".into(),
                title: "Current queue".into(),
                url: None,
            },
        );
    }
    out
}

fn sound_json(s: &Sound) -> Value {
    json!({ "id": s.id, "category": s.category, "title": s.title })
}

/// An alarm as clients see it.
pub fn alarm_json(a: &Value, sounds: &[Sound], salt: &str) -> Option<Value> {
    let id = a.get("id").and_then(Value::as_str).filter(|i| id_ok(i))?;
    let int = |k: &str| match a.get(k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    };
    let days: Vec<u8> = a
        .get("dow")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split(',')
        .filter_map(|d| d.parse::<u8>().ok())
        .filter(|d| *d <= 6)
        .collect();
    let sound = match a.get("url").and_then(Value::as_str) {
        None | Some("") | Some("CURRENT_PLAYLIST") | Some("0") => {
            sounds.iter().find(|s| s.id == CURRENT).map(sound_json)
        }
        Some(url) => {
            let id = sound_id(salt, url);
            Some(
                sounds
                    .iter()
                    .find(|s| s.id == id)
                    .map(sound_json)
                    .unwrap_or_else(|| json!({ "id": null, "category": "other", "title": "Something not listed" })),
            )
        }
    };
    Some(json!({
        "id": id,
        "time": int("time").filter(|t| (0..86_400).contains(t))?,
        "days": days,
        "enabled": int("enabled") == Some(1),
        "repeat": int("repeat") != Some(0),
        "volume": int("volume").filter(|v| (0..=100).contains(v)),
        "shuffle": int("shufflemode").filter(|v| (0..=2).contains(v)).unwrap_or(0),
        "sound": sound,
    }))
}

/// An alarm id: LMS's own (8 hex today), 1–32 lowercase hex accepted.
pub fn id_ok(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The LMS parameters for a new or changed alarm, from a request body. `add` needs a time.
/// The sound (if any) is returned apart: it must be resolved against LMS's own list.
pub fn params(body: &Value, add: bool) -> Result<(Vec<String>, Option<String>), &'static str> {
    let o = body.as_object().ok_or("expected an object")?;
    const KNOWN: [&str; 7] = [
        "time", "days", "enabled", "repeat", "volume", "shuffle", "sound",
    ];
    if o.keys().any(|k| !KNOWN.contains(&k.as_str())) {
        return Err("unknown field");
    }
    let mut p = vec![];
    match o.get("time") {
        Some(Value::Number(n)) => match n.as_u64().filter(|t| *t < 86_400) {
            Some(t) => p.push(format!("time:{t}")),
            None => return Err("bad time"),
        },
        None if !add => {}
        _ => return Err("bad time"),
    }
    match o.get("days") {
        Some(Value::Array(a)) => {
            let mut days = vec![];
            for d in a {
                match d.as_u64().filter(|d| *d <= 6) {
                    Some(d) if !days.contains(&d) => days.push(d),
                    _ => return Err("bad days"),
                }
            }
            if days.is_empty() {
                return Err("bad days");
            }
            days.sort_unstable();
            let s: Vec<String> = days.iter().map(u64::to_string).collect();
            p.push(format!("dow:{}", s.join(",")));
        }
        None if !add => {}
        None => p.push("dow:0,1,2,3,4,5,6".into()),
        _ => return Err("bad days"),
    }
    for (k, lms) in [("enabled", "enabled"), ("repeat", "repeat")] {
        match o.get(k) {
            Some(Value::Bool(b)) => p.push(format!("{lms}:{}", u8::from(*b))),
            None if add => p.push(format!("{lms}:1")),
            None => {}
            _ => return Err("bad flag"),
        }
    }
    match o.get("volume") {
        Some(Value::Number(n)) => match n.as_u64().filter(|v| *v <= 100) {
            Some(v) => p.push(format!("volume:{v}")),
            None => return Err("bad volume"),
        },
        None => {}
        _ => return Err("bad volume"),
    }
    match o.get("shuffle") {
        Some(Value::Number(n)) => match n.as_u64().filter(|v| *v <= 2) {
            Some(v) => p.push(format!("shufflemode:{v}")),
            None => return Err("bad shuffle"),
        },
        None => {}
        _ => return Err("bad shuffle"),
    }
    let sound = match o.get("sound") {
        Some(Value::String(s))
            if s == CURRENT
                || (s.len() == 16
                    && s.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) =>
        {
            Some(s.clone())
        }
        None => None,
        _ => return Err("bad sound"),
    };
    if p.is_empty() && sound.is_none() {
        return Err("nothing to change");
    }
    Ok((p, sound))
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// The caller and the player, if they may control it.
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
fn player_for(
    gw: &AppState,
    headers: &HeaderMap,
    player: &str,
) -> Result<(String, String), Response> {
    let p = caller(gw, headers)?;
    let player = player.to_ascii_lowercase();
    let scope = gw
        .accounts()
        .player_scope(&p)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
    if !rpc_policy::is_player_id(&player) || !scope.allows(&player) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "you may not control this player",
        ));
    }
    let salt = gw
        .download_salt()
        .ok_or_else(|| error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
    Ok((player, salt))
}

// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
async fn lms(gw: &AppState, player: &str, cmd: Vec<String>) -> Result<Value, Response> {
    gw.lms_command(player, &cmd).await.map_err(|e| {
        tracing::warn!(error = %e, "alarm command failed");
        error(StatusCode::SERVICE_UNAVAILABLE, "the server didn't answer")
    })
}

// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
async fn sounds(gw: &AppState, player: &str, salt: &str) -> Result<Vec<Sound>, Response> {
    let v = lms(
        gw,
        player,
        vec![
            "alarm".into(),
            "playlists".into(),
            "0".into(),
            MAX_SOUNDS.to_string(),
        ],
    )
    .await?;
    Ok(parse_sounds(&v, salt))
}

/// The player's alarms, switch and sounds, as clients see them.
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
async fn listing(gw: &AppState, player: &str, salt: &str) -> Result<Value, Response> {
    let sounds = sounds(gw, player, salt).await?;
    let v = lms(
        gw,
        player,
        vec![
            "alarms".into(),
            "0".into(),
            MAX_ALARMS.to_string(),
            "filter:all".into(),
        ],
    )
    .await?;
    let alarms: Vec<Value> = v
        .get("alarms_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ALARMS)
        .filter_map(|a| alarm_json(a, &sounds, salt))
        .collect();
    let all = lms(
        gw,
        player,
        vec!["playerpref".into(), "alarmsEnabled".into(), "?".into()],
    )
    .await?;
    let enabled = match all.get("_p2") {
        Some(Value::String(s)) => s != "0",
        Some(Value::Number(n)) => n.as_i64() != Some(0),
        _ => true,
    };
    Ok(json!({
        "enabled": enabled,
        "alarms": alarms,
        "sounds": sounds.iter().map(sound_json).collect::<Vec<_>>(),
    }))
}

/// `sound` resolved to LMS's own URL parameter (`url:0` for the current queue).
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
async fn sound_param(
    gw: &AppState,
    player: &str,
    salt: &str,
    id: &str,
) -> Result<String, Response> {
    let list = sounds(gw, player, salt).await?;
    match list.iter().find(|s| s.id == id) {
        Some(Sound { url: None, .. }) => Ok("url:0".into()),
        Some(Sound { url: Some(u), .. }) => Ok(format!("url:{u}")),
        None => Err(error(StatusCode::BAD_REQUEST, "no such sound")),
    }
}

/// `GET /v1/players/{player}/alarms`
pub async fn list(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(player): Path<String>,
) -> Response {
    let (player, salt) = match player_for(&gw, &headers, &player) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match listing(&gw, &player, &salt).await {
        Ok(v) => ok(v),
        Err(r) => r,
    }
}

async fn change(
    gw: &AppState,
    player: &str,
    salt: &str,
    first: &[&str],
    body: &Value,
    add: bool,
) -> Response {
    let (mut p, sound) = match params(body, add) {
        Ok(x) => x,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    if let Some(id) = sound {
        match sound_param(gw, player, salt, &id).await {
            Ok(u) => p.push(u),
            Err(r) => return r,
        }
    }
    let mut cmd: Vec<String> = first.iter().map(|s| s.to_string()).collect();
    cmd.extend(p);
    let r = match lms(gw, player, cmd).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let id = r
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| id_ok(i))
        .map(str::to_string);
    match listing(gw, player, salt).await {
        Ok(mut v) => {
            if let Some(id) = id {
                v["id"] = json!(id);
            }
            ok(v)
        }
        Err(r) => r,
    }
}

/// `POST /v1/players/{player}/alarms`
pub async fn add(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(player): Path<String>,
    body: Body,
) -> Response {
    let (player, salt) = match player_for(&gw, &headers, &player) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let body = match body_json(&headers, body, MAX_BODY).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    change(&gw, &player, &salt, &["alarm", "add"], &body, true).await
}

/// `PATCH /v1/players/{player}/alarms/{id}`
pub async fn update(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path((player, id)): Path<(String, String)>,
    body: Body,
) -> Response {
    let (player, salt) = match player_for(&gw, &headers, &player) {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !id_ok(&id) {
        return error(StatusCode::NOT_FOUND, "no such alarm");
    }
    let body = match body_json(&headers, body, MAX_BODY).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let idp = format!("id:{id}");
    change(
        &gw,
        &player,
        &salt,
        &["alarm", "update", &idp],
        &body,
        false,
    )
    .await
}

/// `DELETE /v1/players/{player}/alarms/{id}`
pub async fn delete(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path((player, id)): Path<(String, String)>,
) -> Response {
    let (player, salt) = match player_for(&gw, &headers, &player) {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !id_ok(&id) {
        return error(StatusCode::NOT_FOUND, "no such alarm");
    }
    if let Err(r) = lms(
        &gw,
        &player,
        vec!["alarm".into(), "delete".into(), format!("id:{id}")],
    )
    .await
    {
        return r;
    }
    match listing(&gw, &player, &salt).await {
        Ok(v) => ok(v),
        Err(r) => r,
    }
}

/// `PUT /v1/players/{player}/alarms/enabled` `{"enabled": bool}`
pub async fn set_all(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(player): Path<String>,
    body: Body,
) -> Response {
    let (player, salt) = match player_for(&gw, &headers, &player) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let body = match body_json(&headers, body, MAX_BODY).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let on = match body.as_object().map(|o| (o.len(), o.get("enabled"))) {
        Some((1, Some(Value::Bool(b)))) => *b,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "expected {\"enabled\": true|false}",
            );
        }
    };
    let cmd = if on { "enableall" } else { "disableall" };
    if let Err(r) = lms(&gw, &player, vec!["alarm".into(), cmd.into()]).await {
        return r;
    }
    match listing(&gw, &player, &salt).await {
        Ok(v) => ok(v),
        Err(r) => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: &str = "salt";

    fn lms_sounds() -> Value {
        json!({"item_loop": [
            {"category": "CURRENT_PLAYLIST", "title": "{ALARM_USE_CURRENT_PLAYLIST}"},
            {"category": "FAVORITES", "title": "Morning radio", "url": "http://radio.example.com/stream"},
            {"category": "PLAYLISTS", "title": "Wake up", "url": "file:///srv/music/playlists/wake.m3u"},
            {"category": "PLAYLISTS", "title": "Dupe", "url": "file:///srv/music/playlists/wake.m3u"},
            {"category": "RANDOM", "title": "Random songs", "url": "randomplay://track"},
            {"category": "WEIRD", "title": "/srv/music/secret", "url": "x://y"},
            {"category": "FAVORITES", "title": "No url"}
        ]})
    }

    #[test]
    fn sounds_have_opaque_ids_and_no_urls_or_paths() {
        let s = parse_sounds(&lms_sounds(), SALT);
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids[0], CURRENT);
        assert_eq!(s[0].title, "Current queue");
        assert_eq!(s.len(), 5, "duplicates and url-less entries dropped");
        for x in &s[1..] {
            assert_eq!(x.id.len(), 16);
            assert!(x.id.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        let out = serde_json::to_string(&s.iter().map(sound_json).collect::<Vec<_>>()).unwrap();
        assert!(
            !out.contains("srv") && !out.contains("http") && !out.contains("randomplay"),
            "{out}"
        );
        assert_eq!(s[4].category, "other");
        assert_eq!(
            s[4].title, "Untitled",
            "a title that looks like a path is never shown"
        );
        // Another salt, other ids.
        assert_ne!(parse_sounds(&lms_sounds(), "other")[1].id, s[1].id);
        // An empty answer still offers the current queue.
        assert_eq!(parse_sounds(&json!({}), SALT).len(), 1);
    }

    #[test]
    fn alarms_are_reported_without_urls() {
        let sounds = parse_sounds(&lms_sounds(), SALT);
        let a = alarm_json(
            &json!({"id": "1a2b3c4d", "dow": "1,2,3,4,5", "enabled": 1, "repeat": "1", "time": 25200,
                    "volume": 50, "shufflemode": 0, "url": "http://radio.example.com/stream"}),
            &sounds,
            SALT,
        )
        .unwrap();
        assert_eq!(a["time"], 25200);
        assert_eq!(a["days"], json!([1, 2, 3, 4, 5]));
        assert_eq!(a["sound"]["title"], "Morning radio");
        assert!(!a.to_string().contains("http"));
        let other = alarm_json(
            &json!({"id": "ff", "time": 0, "url": "file:///etc/passwd"}),
            &sounds,
            SALT,
        )
        .unwrap();
        assert_eq!(other["sound"]["id"], Value::Null);
        assert!(!other.to_string().contains("passwd"));
        let cur = alarm_json(
            &json!({"id": "ff", "time": 1, "url": "CURRENT_PLAYLIST"}),
            &sounds,
            SALT,
        )
        .unwrap();
        assert_eq!(cur["sound"]["id"], CURRENT);
        for bad in [
            json!({"id": "XYZ", "time": 1}),
            json!({"id": "1", "time": 86400}),
            json!({"id": "1", "time": -1}),
            json!({"time": 1}),
            json!({"id": "a".repeat(33), "time": 1}),
        ] {
            assert_eq!(alarm_json(&bad, &sounds, SALT), None, "{bad}");
        }
    }

    #[test]
    fn the_parameter_grammar() {
        let (p, s) = params(&json!({"time": 25200, "days": [5, 1, 3]}), true).unwrap();
        assert_eq!(p, ["time:25200", "dow:1,3,5", "enabled:1", "repeat:1"]);
        assert_eq!(s, None);
        let (p, s) = params(
            &json!({"time": 0, "enabled": false, "repeat": false, "volume": 100, "shuffle": 2, "sound": "current"}),
            true,
        )
        .unwrap();
        assert_eq!(
            p,
            [
                "time:0",
                "dow:0,1,2,3,4,5,6",
                "enabled:0",
                "repeat:0",
                "volume:100",
                "shufflemode:2"
            ]
        );
        assert_eq!(s.as_deref(), Some(CURRENT));
        assert_eq!(
            params(&json!({"volume": 5}), false).unwrap().0,
            ["volume:5"]
        );
        assert_eq!(
            params(&json!({"sound": "0123456789abcdef"}), false)
                .unwrap()
                .1
                .as_deref(),
            Some("0123456789abcdef")
        );
        for (bad, add) in [
            (json!({}), true),
            (json!({}), false),
            (json!([]), true),
            (json!({"time": 86400}), true),
            (json!({"time": -1}), true),
            (json!({"time": 1.5}), true),
            (json!({"time": "25200"}), true),
            (json!({"time": 1, "days": []}), true),
            (json!({"time": 1, "days": [7]}), true),
            (json!({"time": 1, "days": [1, 1]}), true),
            (json!({"time": 1, "days": "1,2"}), true),
            (json!({"time": 1, "volume": 101}), true),
            (json!({"time": 1, "volume": -5}), true),
            (json!({"time": 1, "shuffle": 3}), true),
            (json!({"time": 1, "enabled": 1}), true),
            (json!({"time": 1, "sound": "http://evil.example/"}), true),
            (json!({"time": 1, "sound": "file:///etc/passwd"}), true),
            (json!({"time": 1, "sound": "0123456789ABCDEF"}), true),
            (json!({"time": 1, "url": "http://evil.example/"}), true),
            (json!({"time": 1, "playlisturl": "x"}), true),
            (json!({"time": 1, "cmd": "delete"}), true),
            (json!({"time": 1, "id": "1"}), true),
            (json!({"time": "1 dow:1"}), true),
        ] {
            assert!(params(&bad, add).is_err(), "{bad}");
        }
    }

    #[test]
    fn ids() {
        for good in ["1a2b3c4d", "0", "f".repeat(32).as_str()] {
            assert!(id_ok(good), "{good}");
        }
        for bad in [
            "",
            "G1",
            "1A2B",
            "a b",
            "1a2b;delete",
            "../x",
            "f".repeat(33).as_str(),
            "id:1",
        ] {
            assert!(!id_ok(bad), "{bad}");
        }
    }
}
