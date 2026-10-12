//! Lyrics (`GET /v1/lyrics/track/{id}`) against a fake LMS: from the file's tags, then from
//! the Music & Artist Information plugin. Security first: strangers, hostile ids and extra
//! parameters are refused, nothing but the id reaches LMS, and lines naming server paths
//! never reach the client.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

async fn get(h: &Harness, path: &str, token: Option<&str>) -> (S, Value) {
    let mut req = reqwest::Client::new().get(h.url(path));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = tokio::time::timeout(Duration::from_secs(20), req.send())
        .await
        .expect("request took too long")
        .unwrap();
    let s = r.status();
    assert_eq!(
        r.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store"),
        "{path}"
    );
    (s, r.json().await.unwrap_or(Value::Null))
}

/// The LMS commands the gateway sent for lyrics.
fn lyrics_calls(h: &Harness) -> Vec<Value> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| {
            let w = &c[1];
            w[0] == "musicartistinfo"
                || (w[0] == "titles" && w.as_array().unwrap().iter().any(|p| p == "tags:w"))
        })
        .collect()
}

#[tokio::test]
async fn synced_lyrics_come_from_the_tags_without_paths() {
    let h = harness().await;
    let (s, v) = get(&h, "/v1/lyrics/track/9", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({"source": "tags", "synced": true, "lines": [
            {"t": 1000, "text": "First line"},
            {"t": 12500, "text": "Second line"}
        ]})
    );
    assert!(!v.to_string().contains("/srv/"));
    // The tags were enough: the plugin wasn't asked.
    assert!(
        lyrics_calls(&h)
            .iter()
            .all(|c| c[1][0] != "musicartistinfo")
    );
}

#[tokio::test]
async fn the_plugin_is_asked_when_the_tags_have_none() {
    let h = harness().await;
    let (s, v) = get(&h, "/v1/lyrics/track/1", Some(&h.tok_b)).await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({"source": "plugin", "synced": false, "lines": [
            {"t": 0, "text": "Plain words"},
            {"t": 0, "text": "from the plugin"}
        ]})
    );
    // Exactly the id, nothing else, as a server query.
    let calls = lyrics_calls(&h);
    let plugin: Vec<&Value> = calls
        .iter()
        .filter(|c| c[1][0] == "musicartistinfo")
        .collect();
    assert_eq!(
        plugin,
        vec![&json!(["", ["musicartistinfo", "lyrics", "track_id:1"]])]
    );
}

#[tokio::test]
async fn no_lyrics_is_404_and_remembered() {
    let h = harness().await;
    for _ in 0..3 {
        let (s, v) = get(&h, "/v1/lyrics/track/7", Some(&h.tok_a)).await;
        assert_eq!(s, S::NOT_FOUND);
        assert_eq!(v, json!({"error": "no lyrics"}));
    }
    // Asked once; the answer was remembered.
    assert_eq!(lyrics_calls(&h).len(), 2);
}

#[tokio::test]
async fn unknown_tracks_are_404_without_asking_the_plugin() {
    let h = harness().await;
    let (s, v) = get(&h, "/v1/lyrics/track/424242", Some(&h.tok_a)).await;
    assert_eq!(s, S::NOT_FOUND);
    assert_eq!(v, json!({"error": "no such track"}));
    assert!(
        lyrics_calls(&h)
            .iter()
            .all(|c| c[1][0] != "musicartistinfo")
    );
}

#[tokio::test]
async fn needs_a_valid_token() {
    let h = harness().await;
    for t in [None, Some("garbage"), Some("Bearer x.y.z")] {
        let (s, _) = get(&h, "/v1/lyrics/track/9", t).await;
        assert_eq!(s, S::UNAUTHORIZED, "{t:?}");
    }
    assert!(lyrics_calls(&h).is_empty());
}

#[tokio::test]
async fn hostile_ids_and_parameters_are_refused_before_lms() {
    let h = harness().await;
    for id in [
        "0",
        "-1",
        "01",
        "1.5",
        "1e3",
        "9999999999999",
        "abc",
        "1%20",
        "%2Fsrv%2Fmusic%2Fa.flac",
        "..%2F..%2Fetc%2Fpasswd",
        "1;rescan",
        "track_id:1",
    ] {
        let (s, _) = get(&h, &format!("/v1/lyrics/track/{id}"), Some(&h.tok_a)).await;
        assert_eq!(s, S::NOT_FOUND, "{id}");
    }
    for q in ["?url=file:///srv/x", "?artist=a&title=b", "?"] {
        let (s, _) = get(&h, &format!("/v1/lyrics/track/9{q}"), Some(&h.tok_a)).await;
        assert!(s == S::BAD_REQUEST || s == S::OK, "{q}: {s}");
        if q != "?" {
            assert_eq!(s, S::BAD_REQUEST, "{q}");
        }
    }
    // Only well-formed ids ever reached LMS.
    for c in lyrics_calls(&h) {
        let words = c[1].as_array().unwrap();
        assert!(
            words.iter().any(|w| w == "track_id:9"),
            "unexpected LMS call {c}"
        );
    }
    // Nothing else is reachable under the path.
    for p in [
        "/v1/lyrics/track/9/extra",
        "/v1/lyrics/track/",
        "/v1/lyrics/9",
    ] {
        let r = reqwest::Client::new()
            .get(h.url(p))
            .bearer_auth(&h.tok_a)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::NOT_FOUND, "{p}");
    }
}

#[tokio::test]
async fn shares_the_per_user_request_allowance() {
    let h = harness().await;
    let mut limited = false;
    for _ in 0..anywhere_gateway::history::ME_REQUESTS_PER_MINUTE + 1 {
        let (s, _) = get(&h, "/v1/lyrics/track/9", Some(&h.tok_a)).await;
        if s == S::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
    }
    assert!(limited);
    // Another user is unaffected.
    let (s, _) = get(&h, "/v1/lyrics/track/9", Some(&h.tok_b)).await;
    assert_eq!(s, S::OK);
}
