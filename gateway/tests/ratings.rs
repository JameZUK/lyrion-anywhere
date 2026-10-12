//! Ratings (docs/plans/wave2-design.md §2) against a fake LMS. Each user's own rating,
//! blended with LMS's shared rating; written through to Ratings Light; offline batches
//! checked against LMS's stable keys. Security first: per user only, strict values, track
//! ids checked, keys verified, hostile bodies refused.

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

async fn key_of(h: &Harness, id: u64) -> String {
    let (s, v) = call(
        h,
        "GET",
        &format!("/v1/download/track/{id}/info"),
        Some(&h.tok_a),
        None,
    )
    .await;
    assert_eq!(s, S::OK, "{v}");
    v["key"].as_str().unwrap().to_string()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// What was sent to Ratings Light, as (track, rating).
fn shared_writes(h: &Harness) -> Vec<(String, String)> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "ratingslight")
        .map(|c| {
            (
                c[1][2].as_str().unwrap().to_string(),
                c[1][3].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn rating_a_track_blends_with_the_shared_rating() {
    let h = harness().await;
    // Track 1 has LMS's shared rating 60; track 3 none; track 2 is a stream (never rated).
    let (s, v) = call(
        &h,
        "GET",
        "/v1/me/ratings?tracks=1,3,2",
        Some(&h.tok_a),
        None,
    )
    .await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v["ratings"],
        json!([
            {"track": 1, "mine": null, "shared": 60, "rating": 60},
            {"track": 3, "mine": null, "shared": null, "rating": null}
        ])
    );
    let (s, v) = call(
        &h,
        "PUT",
        "/v1/me/ratings/1",
        Some(&h.tok_a),
        Some(json!({"rating": 100})),
    )
    .await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({"track": 1, "mine": 100, "shared": 60, "rating": 90}),
        "own ×3, shared ×1"
    );
    assert_eq!(
        shared_writes(&h),
        [("1".to_string(), "100".to_string())],
        "written through to Ratings Light"
    );
    // Private: bob sees only the shared rating.
    let (_, v) = call(&h, "GET", "/v1/me/ratings?tracks=1", Some(&h.tok_b), None).await;
    assert_eq!(v["ratings"][0]["mine"], Value::Null);
    // 0 clears.
    let (_, v) = call(
        &h,
        "PUT",
        "/v1/me/ratings/1",
        Some(&h.tok_a),
        Some(json!({"rating": 0})),
    )
    .await;
    assert_eq!(v["mine"], Value::Null);
    // A stream or an unknown track can't be rated.
    for id in [2, 99_999] {
        let (s, _) = call(
            &h,
            "PUT",
            &format!("/v1/me/ratings/{id}"),
            Some(&h.tok_a),
            Some(json!({"rating": 50})),
        )
        .await;
        assert_eq!(s, S::NOT_FOUND, "{id}");
    }
}

#[tokio::test]
async fn ratings_follow_the_file_not_the_track_number() {
    let h = harness().await;
    // Track 7 is track 1's file under another id (a rescan renumbered it).
    call(
        &h,
        "PUT",
        "/v1/me/ratings/1",
        Some(&h.tok_a),
        Some(json!({"rating": 80})),
    )
    .await;
    let (_, v) = call(&h, "GET", "/v1/me/ratings?tracks=7", Some(&h.tok_a), None).await;
    assert_eq!(v["ratings"][0]["mine"], 80);
}

#[tokio::test]
async fn offline_ratings_are_checked_and_the_newest_wins() {
    let h = harness().await;
    let (k1, k3) = (key_of(&h, 1).await, key_of(&h, 3).await);
    let t = now();
    // Rated online just now.
    call(
        &h,
        "PUT",
        "/v1/me/ratings/3",
        Some(&h.tok_a),
        Some(json!({"rating": 90})),
    )
    .await;
    let (s, v) = call(
        &h,
        "POST",
        "/v1/me/ratings",
        Some(&h.tok_a),
        Some(json!({"ratings": [
            {"track": 1, "key": k1, "rating": 70, "at": t - 600},
            {"track": 3, "key": k3, "rating": 20, "at": t - 3600},   // older than the online one
            {"track": 3, "key": k1, "rating": 50, "at": t - 60},     // the key of another file
            {"track": 99999, "key": k1, "rating": 50, "at": t - 60}, // unknown to LMS
        ]})),
    )
    .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v, json!({"applied": 1, "skipped": 3}));
    let (_, v) = call(&h, "GET", "/v1/me/ratings?tracks=1,3", Some(&h.tok_a), None).await;
    assert_eq!(v["ratings"][0]["mine"], 70);
    assert_eq!(
        v["ratings"][1]["mine"], 90,
        "the newer online rating was kept"
    );
    // Sending the same batch again changes nothing.
    let (_, v) = call(
        &h,
        "POST",
        "/v1/me/ratings",
        Some(&h.tok_a),
        Some(json!({"ratings": [{"track": 1, "key": k1, "rating": 70, "at": t - 600}]})),
    )
    .await;
    assert_eq!(v["applied"], 1, "same value, same time: harmless");
    // Forget everything.
    let (_, v) = call(&h, "DELETE", "/v1/me/ratings", Some(&h.tok_a), None).await;
    assert_eq!(v["removed"], 2);
}

#[tokio::test]
async fn the_rating_style_is_per_user() {
    let h = harness().await;
    let (_, v) = call(&h, "GET", "/v1/me/ratings/style", Some(&h.tok_a), None).await;
    assert_eq!(v, json!({"style": "stars", "count": 0}));
    let (s, _) = call(
        &h,
        "PUT",
        "/v1/me/ratings/style",
        Some(&h.tok_a),
        Some(json!({"style": "heart"})),
    )
    .await;
    assert_eq!(s, S::OK);
    assert_eq!(
        call(&h, "GET", "/v1/me/ratings/style", Some(&h.tok_a), None)
            .await
            .1["style"],
        "heart"
    );
    assert_eq!(
        call(&h, "GET", "/v1/me/ratings/style", Some(&h.tok_b), None)
            .await
            .1["style"],
        "stars"
    );
    for bad in [
        json!({"style": "thumbs"}),
        json!({"style": 1}),
        json!({}),
        json!({"style": "heart", "x": 1}),
    ] {
        let (s, _) = call(
            &h,
            "PUT",
            "/v1/me/ratings/style",
            Some(&h.tok_a),
            Some(bad.clone()),
        )
        .await;
        assert_eq!(s, S::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn hostile_requests_are_refused() {
    let h = harness().await;
    for (m, path) in [
        ("GET", "/v1/me/ratings?tracks=1"),
        ("PUT", "/v1/me/ratings/1"),
        ("POST", "/v1/me/ratings"),
        ("DELETE", "/v1/me/ratings"),
        ("GET", "/v1/me/ratings/style"),
    ] {
        let (s, _) = call(&h, m, path, None, Some(json!({"rating": 50}))).await;
        assert_eq!(s, S::UNAUTHORIZED, "{m} {path}");
    }
    for q in [
        "",
        "?tracks=",
        "?tracks=0",
        "?tracks=1;2",
        "?tracks=a",
        "?tracks=1&x=1",
        "?track=1",
    ] {
        let (s, _) = call(
            &h,
            "GET",
            &format!("/v1/me/ratings{q}"),
            Some(&h.tok_a),
            None,
        )
        .await;
        assert_eq!(s, S::BAD_REQUEST, "{q}");
    }
    let many: Vec<String> = (1..=201).map(|n| n.to_string()).collect();
    let (s, _) = call(
        &h,
        "GET",
        &format!("/v1/me/ratings?tracks={}", many.join(",")),
        Some(&h.tok_a),
        None,
    )
    .await;
    assert_eq!(s, S::BAD_REQUEST);
    for body in [
        json!({"rating": 55}),
        json!({"rating": 110}),
        json!({"rating": -10}),
        json!({"rating": "50"}),
        json!({"rating": 50.5}),
        json!({"rating": 50, "track": 3}),
        json!({}),
    ] {
        let (s, _) = call(
            &h,
            "PUT",
            "/v1/me/ratings/1",
            Some(&h.tok_a),
            Some(body.clone()),
        )
        .await;
        assert_eq!(s, S::BAD_REQUEST, "{body}");
    }
    for id in ["0", "-1", "01", "1.5", "abc", "1;rescan"] {
        let (s, _) = call(
            &h,
            "PUT",
            &format!("/v1/me/ratings/{id}"),
            Some(&h.tok_a),
            Some(json!({"rating": 50})),
        )
        .await;
        assert_eq!(s, S::NOT_FOUND, "{id}");
    }
    let k = "a".repeat(64);
    let t = now();
    for item in [
        json!({"track": 1, "key": "../../x", "rating": 50, "at": t}),
        json!({"track": 1, "key": k.to_uppercase(), "rating": 50, "at": t}),
        json!({"track": 1, "key": k, "rating": 55, "at": t}),
        json!({"track": 1, "key": k, "rating": 50, "at": t + 86_400}), // in the future
        json!({"track": 1, "key": k, "rating": 50, "at": 1}),          // before 2020
        json!({"track": 0, "key": k, "rating": 50, "at": t}),
        json!({"track": 1, "key": k, "rating": 50}),
        json!({"track": 1, "key": k, "rating": 50, "at": t, "user": "bob"}),
    ] {
        let (s, _) = call(
            &h,
            "POST",
            "/v1/me/ratings",
            Some(&h.tok_a),
            Some(json!({"ratings": [item.clone()]})),
        )
        .await;
        assert_eq!(s, S::BAD_REQUEST, "{item}");
    }
    let too_many: Vec<Value> = (0..501)
        .map(|_| json!({"track": 1, "key": k, "rating": 50, "at": t}))
        .collect();
    let (s, _) = call(
        &h,
        "POST",
        "/v1/me/ratings",
        Some(&h.tok_a),
        Some(json!({"ratings": too_many})),
    )
    .await;
    assert_eq!(s, S::PAYLOAD_TOO_LARGE);
    assert!(
        shared_writes(&h).is_empty(),
        "nothing reached Ratings Light"
    );
}

#[tokio::test]
async fn the_rpc_endpoint_still_refuses_ratings_light() {
    let h = harness().await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(
            &json!({"player": "", "cmd": ["ratingslight", "setratingpercentnoclient", "1", "100"]}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
}
