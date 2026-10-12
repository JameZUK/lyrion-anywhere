//! Alarms (docs/plans/wave2-design.md §1) against a fake LMS. Security first: only players
//! the caller may control, a strict grammar, and sounds only from LMS's own list (never a
//! URL from the client, and none of LMS's URLs in any answer).

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

async fn call(
    h: &Harness,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (S, Value) {
    let c = reqwest::Client::new();
    let mut req = match method {
        "GET" => c.get(h.url(path)),
        "POST" => c.post(h.url(path)),
        "PATCH" => c.patch(h.url(path)),
        "PUT" => c.put(h.url(path)),
        _ => c.delete(h.url(path)),
    };
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let r = req.send().await.unwrap();
    let s = r.status();
    (s, r.json().await.unwrap_or(Value::Null))
}

/// The alarm commands LMS was sent, as (player, words).
fn alarm_calls(h: &Harness) -> Vec<(String, Vec<String>)> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "alarm" && c[1][1] != "playlists")
        .map(|c| {
            (
                c[0].as_str().unwrap_or("").to_string(),
                c[1].as_array()
                    .unwrap()
                    .iter()
                    .map(|w| w.as_str().unwrap_or("").to_string())
                    .collect(),
            )
        })
        .collect()
}

fn mine(h: &Harness) -> String {
    format_mac(&h.mac_a)
}

fn no_urls(v: &Value) {
    let s = v.to_string();
    for bad in ["http", "file:", "/srv/", "randomplay", "token", "secret"] {
        assert!(!s.contains(bad), "{bad} leaked: {s}");
    }
}

#[tokio::test]
async fn listing_shows_alarms_and_sounds_without_urls() {
    let h = harness().await;
    let (s, v) = call(
        &h,
        "GET",
        &format!("/v1/players/{}/alarms", mine(&h)),
        Some(&h.tok_a),
        None,
    )
    .await;
    assert_eq!(s, S::OK);
    no_urls(&v);
    assert_eq!(v["enabled"], true);
    let sounds = v["sounds"].as_array().unwrap();
    assert_eq!(sounds.len(), 4);
    assert_eq!(
        sounds[0],
        json!({"id": "current", "category": "current", "title": "Current queue"})
    );
    let wake = sounds.iter().find(|x| x["title"] == "Wake up").unwrap();
    assert_eq!(wake["category"], "playlists");
    let a = &v["alarms"][0];
    assert_eq!(a["id"], "1a2b3c4d");
    assert_eq!(a["time"], 25200);
    assert_eq!(a["days"], json!([1, 2, 3, 4, 5]));
    assert_eq!(a["volume"], 40);
    assert_eq!(a["sound"]["id"], wake["id"], "the alarm's sound by id");
}

#[tokio::test]
async fn adding_resolves_the_sound_to_lms_own_url() {
    let h = harness().await;
    let path = format!("/v1/players/{}/alarms", mine(&h));
    let (_, v) = call(&h, "GET", &path, Some(&h.tok_a), None).await;
    let radio = v["sounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["title"] == "Morning radio")
        .unwrap()["id"]
        .clone();
    let (s, v) = call(
        &h,
        "POST",
        &path,
        Some(&h.tok_a),
        Some(json!({"time": 23400, "days": [6, 0], "volume": 30, "shuffle": 1, "sound": radio})),
    )
    .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["id"], "5e6f7a8b");
    no_urls(&v);
    let calls = alarm_calls(&h);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, mine(&h));
    assert_eq!(
        calls[0].1,
        [
            "alarm",
            "add",
            "time:23400",
            "dow:0,6",
            "enabled:1",
            "repeat:1",
            "volume:30",
            "shufflemode:1",
            "url:http://radio.example.com/stream?token=secret"
        ]
    );
}

#[tokio::test]
async fn changing_deleting_and_the_all_switch() {
    let h = harness().await;
    let base = format!("/v1/players/{}/alarms", mine(&h));
    let (s, _) = call(
        &h,
        "PATCH",
        &format!("{base}/1a2b3c4d"),
        Some(&h.tok_a),
        Some(json!({"enabled": false, "sound": "current"})),
    )
    .await;
    assert_eq!(s, S::OK);
    let (s, _) = call(
        &h,
        "DELETE",
        &format!("{base}/1a2b3c4d"),
        Some(&h.tok_a),
        None,
    )
    .await;
    assert_eq!(s, S::OK);
    let (s, _) = call(
        &h,
        "PUT",
        &format!("{base}/enabled"),
        Some(&h.tok_a),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(s, S::OK);
    let words: Vec<Vec<String>> = alarm_calls(&h).into_iter().map(|c| c.1).collect();
    assert_eq!(
        words,
        [
            vec!["alarm", "update", "id:1a2b3c4d", "enabled:0", "url:0"],
            vec!["alarm", "delete", "id:1a2b3c4d"],
            vec!["alarm", "disableall"],
        ]
    );
}

#[tokio::test]
async fn only_players_the_caller_may_control() {
    let h = harness().await;
    let others = [
        format_mac(&h.mac_b),            // another user's phone
        "aa:bb:cc:dd:ee:01".to_string(), // a home player not granted
        "02:4c:41:aa:aa:aa".to_string(),
    ];
    for p in &others {
        let base = format!("/v1/players/{p}/alarms");
        for (m, path, body) in [
            ("GET", base.clone(), None),
            ("POST", base.clone(), Some(json!({"time": 1}))),
            (
                "PATCH",
                format!("{base}/1a2b3c4d"),
                Some(json!({"volume": 1})),
            ),
            ("DELETE", format!("{base}/1a2b3c4d"), None),
            (
                "PUT",
                format!("{base}/enabled"),
                Some(json!({"enabled": true})),
            ),
        ] {
            let (s, _) = call(&h, m, &path, Some(&h.tok_a), body).await;
            assert_eq!(s, S::FORBIDDEN, "{m} {path}");
        }
    }
    for p in ["not-a-mac", "AA:BB:CC:DD:EE:0G", "..%2F..%2Fetc"] {
        let (s, _) = call(
            &h,
            "GET",
            &format!("/v1/players/{p}/alarms"),
            Some(&h.tok_a),
            None,
        )
        .await;
        assert!(s == S::FORBIDDEN || s == S::NOT_FOUND, "{p}: {s}");
    }
    assert!(
        h.lms
            .rpc_calls()
            .iter()
            .all(|c| c[1][0] != "alarm" && c[1][0] != "alarms"),
        "nothing reached LMS"
    );
    // Once an admin grants the home player, it may.
    h.accounts
        .grant_player("alice", "aa:bb:cc:dd:ee:01")
        .unwrap();
    let (s, _) = call(
        &h,
        "GET",
        "/v1/players/aa:bb:cc:dd:ee:01/alarms",
        Some(&h.tok_a),
        None,
    )
    .await;
    assert_eq!(s, S::OK);
    // No token at all.
    let (s, _) = call(
        &h,
        "GET",
        &format!("/v1/players/{}/alarms", mine(&h)),
        None,
        None,
    )
    .await;
    assert_eq!(s, S::UNAUTHORIZED);
}

#[tokio::test]
async fn hostile_bodies_and_ids_never_reach_lms() {
    let h = harness().await;
    let base = format!("/v1/players/{}/alarms", mine(&h));
    for body in [
        json!({"time": 1, "url": "http://evil.example/"}),
        json!({"time": 1, "playlisturl": "file:///etc/passwd"}),
        json!({"time": 1, "sound": "file:///etc/passwd"}),
        json!({"time": 1, "sound": "0123456789abcdef"}), // not one LMS offers
        json!({"time": 86400}),
        json!({"time": 1, "days": [9]}),
        json!({"time": 1, "volume": 1000}),
        json!({"time": "1 dow:0"}),
        json!({"time": 1, "cmd": "delete"}),
        json!([]),
    ] {
        let (s, _) = call(&h, "POST", &base, Some(&h.tok_a), Some(body.clone())).await;
        assert_eq!(s, S::BAD_REQUEST, "{body}");
    }
    for id in ["XYZ", "1a2b;delete", "a".repeat(40).as_str(), "id:1"] {
        let (s, _) = call(
            &h,
            "PATCH",
            &format!("{base}/{id}"),
            Some(&h.tok_a),
            Some(json!({"volume": 1})),
        )
        .await;
        assert_eq!(s, S::NOT_FOUND, "{id}");
    }
    for body in [
        json!({"enabled": 1}),
        json!({}),
        json!({"enabled": true, "x": 1}),
    ] {
        let (s, _) = call(
            &h,
            "PUT",
            &format!("{base}/enabled"),
            Some(&h.tok_a),
            Some(body.clone()),
        )
        .await;
        assert_eq!(s, S::BAD_REQUEST, "{body}");
    }
    // Oversized.
    let big = json!({"time": 1, "pad": "x".repeat(5000)});
    let (s, _) = call(&h, "POST", &base, Some(&h.tok_a), Some(big)).await;
    assert!(s == S::PAYLOAD_TOO_LARGE || s == S::BAD_REQUEST);
    assert!(
        alarm_calls(&h).is_empty(),
        "nothing changed in LMS: {:?}",
        alarm_calls(&h)
    );
}

#[tokio::test]
async fn the_rpc_endpoint_still_refuses_alarm_commands() {
    let h = harness().await;
    for cmd in [
        json!(["alarm", "add", "time:1"]),
        json!(["alarms", 0, 10]),
        json!(["alarm", "playlists", 0, 10]),
    ] {
        let r = reqwest::Client::new()
            .post(h.url("/v1/rpc"))
            .bearer_auth(&h.tok_a)
            .json(&json!({"player": mine(&h), "cmd": cmd}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::FORBIDDEN, "{cmd}");
    }
}
