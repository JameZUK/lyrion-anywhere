//! Listening stats (`GET /v1/me/stats`) against a fake LMS and a seeded history: the
//! numbers, the caller's own plays only, switched-off history, strict queries, caching and
//! its invalidation, and artist-name lookups that stay bounded and cached.

mod common;
use common::*;

use anywhere_gateway::history::{self, Source, TrackInfo};
use reqwest::StatusCode as S;
use serde_json::{Value, json};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn user(h: &Harness, name: &str) -> i64 {
    h.accounts.user_id_by_name(name).unwrap().unwrap()
}

/// Record a play of track `track` (album `album`, artist `artist`) `ago` seconds ago.
fn seed(h: &Harness, who: i64, track: u64, album: i64, artist: i64, ago: i64, seconds: f64) {
    let t = TrackInfo {
        id: track,
        key: format!("{track:064x}"),
        album_id: Some(album),
        artist_id: Some(artist),
        year: Some(2001),
        duration: Some(300.0),
    };
    history::record(
        &h.accounts,
        who,
        &t,
        (now() - ago) as u64,
        seconds,
        Source::Live,
    )
    .unwrap()
    .expect("history is on");
}

async fn call(h: &Harness, method: &str, path: &str, token: Option<&str>) -> (S, Value) {
    let c = reqwest::Client::new();
    let mut req = match method {
        "GET" => c.get(h.url(path)),
        _ => c.delete(h.url(path)),
    };
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = tokio::time::timeout(Duration::from_secs(20), req.send())
        .await
        .expect("request took too long")
        .unwrap();
    let status = r.status();
    let cache = r
        .headers()
        .get("cache-control")
        .map(|v| v.to_str().unwrap().to_string());
    let v = r.json().await.unwrap_or(Value::Null);
    if status == S::OK {
        assert_eq!(cache.as_deref(), Some("private, no-store"));
    }
    (status, v)
}

async fn stats(h: &Harness, token: &str, query: &str) -> Value {
    let (s, v) = call(h, "GET", &format!("/v1/me/stats{query}"), Some(token)).await;
    assert_eq!(s, S::OK, "{query}: {v}");
    v
}

async fn set_history(h: &Harness, token: &str, on: bool) {
    let body = json!({ "enabled": on });
    let r = reqwest::Client::new()
        .patch(h.url("/v1/me/history"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
}

/// `artists` lookups the gateway sent LMS.
fn artist_lookups(h: &Harness) -> Vec<Value> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|p| p[1][0] == "artists")
        .map(|p| p[1].clone())
        .collect()
}

#[tokio::test]
async fn stats_need_a_valid_token_and_count_under_the_allowance() {
    let h = harness().await;
    for token in [None, Some("garbage"), Some("")] {
        let (s, _) = call(&h, "GET", "/v1/me/stats", token).await;
        assert_eq!(s, S::UNAUTHORIZED, "{token:?}");
    }
    let mut limited = false;
    for _ in 0..history::ME_REQUESTS_PER_MINUTE + 1 {
        let (s, _) = call(&h, "GET", "/v1/me/stats?period=all", Some(&h.tok_a)).await;
        if s == S::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
        assert_eq!(s, S::OK);
    }
    assert!(limited, "never rate limited");
    // Bob is unaffected.
    stats(&h, &h.tok_b, "").await;
}

#[tokio::test]
async fn stats_count_only_the_callers_own_plays() {
    let h = harness().await;
    let (alice, bob) = (user(&h, "alice"), user(&h, "bob"));
    // Alice: track 1 three times (artist 1), track 2 once (artist 2), and track 3 forty
    // days ago (artist 3: outside 30 days, so 1 and 2 are new while 3 is not).
    seed(&h, alice, 1, 10, 1, 3600, 200.0);
    seed(&h, alice, 1, 10, 1, 7200, 200.0);
    seed(&h, alice, 1, 10, 1, 86_400 + 60, 200.0);
    seed(&h, alice, 2, 20, 2, 7300, 100.4);
    seed(&h, alice, 3, 30, 3, 40 * 86_400, 300.0);
    seed(&h, alice, 4, 30, 3, 2 * 86_400 + 60, 50.0);
    // Bob: lots of a different artist; never in alice's numbers.
    for i in 0..10 {
        seed(&h, bob, 7, 70, 77, 600 + i, 250.0);
    }
    let v = stats(&h, &h.tok_a, "?period=days-30&tz=0").await;
    assert_eq!(v["enabled"], true);
    assert_eq!(v["retention_days"], 396);
    let t = &v["totals"];
    assert_eq!(t["plays"], 5);
    assert_eq!(t["seconds"], 750);
    assert_eq!(
        (
            t["tracks"].clone(),
            t["albums"].clone(),
            t["artists"].clone()
        ),
        (json!(3), json!(3), json!(3))
    );
    assert!((2..=4).contains(&t["days"].as_u64().unwrap()));
    assert_eq!(
        v["top_artists"][0],
        json!({"id": 1, "name": "Invented Artist 1", "plays": 3, "seconds": 600})
    );
    assert_eq!(
        v["top_albums"][0],
        json!({"id": 10, "plays": 3, "seconds": 600})
    );
    assert_eq!(
        v["top_tracks"][0],
        json!({"id": 1, "plays": 3, "seconds": 600})
    );
    let new: Vec<i64> = v["new_artists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_i64().unwrap())
        .collect();
    assert_eq!(new, [1, 2], "artist 3 was first played before the period");
    assert_eq!(v["new_artists"][1]["name"], "Invented Artist 2");
    assert!(v["new_artists"][0]["first_played"].as_i64().unwrap() > now() - 2 * 86_400);
    assert_eq!(v["by_hour"].as_array().unwrap().len(), 24);
    assert_eq!(v["by_weekday"].as_array().unwrap().len(), 7);
    let sum = |k: &str| {
        v[k].as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap())
            .sum::<u64>()
    };
    assert_eq!((sum("by_hour"), sum("by_weekday")), (5, 5));
    assert_eq!(v["by_month"], json!([]));
    assert!(v["longest_streak"]["days"].as_u64().unwrap() >= 2);
    assert!(v["busiest_day"]["seconds"].as_u64().unwrap() >= 200);
    let (from, to) = (
        v["period"]["from"].as_i64().unwrap(),
        v["period"]["to"].as_i64().unwrap(),
    );
    assert!(to <= now() && to > now() - 60 && from <= now() - 29 * 86_400);
    // Nothing of bob's anywhere.
    let text = v.to_string();
    assert!(
        !text.contains("\"id\":77") && !text.contains("\"id\":70"),
        "{text}"
    );
    // The whole kept history: artist 3 too, and months.
    let all = stats(&h, &h.tok_a, "?period=all").await;
    assert_eq!(all["totals"]["plays"], 6);
    assert_eq!(all["new_artists"].as_array().unwrap().len(), 3);
    let months = all["by_month"].as_array().unwrap();
    assert!((13..=15).contains(&months.len()));
    assert_eq!(
        months
            .iter()
            .map(|m| m["plays"].as_u64().unwrap())
            .sum::<u64>(),
        6
    );
    // Bob sees only his own.
    let b = stats(&h, &h.tok_b, "?period=days-7").await;
    assert_eq!(b["totals"]["plays"], 10);
    assert_eq!(b["totals"]["tracks"], 1);
    assert_eq!(b["top_artists"][0]["id"], 77);
    assert_eq!(b["top_artists"].as_array().unwrap().len(), 1);
    // A past year with nothing in it, and a future month: empty, well-formed.
    let y = stats(&h, &h.tok_a, "?period=year-1999").await;
    assert_eq!(y["totals"]["plays"], 0);
    assert_eq!(y["by_month"].as_array().unwrap().len(), 12);
    assert!(y["longest_streak"].is_null() && y["busiest_day"].is_null());
    let m = stats(&h, &h.tok_a, "?period=month-2100-12&tz=-840").await;
    assert_eq!(m["period"]["from"], m["period"]["to"]);
    assert_eq!(m["totals"]["plays"], 0);
}

#[tokio::test]
async fn malformed_queries_are_refused() {
    let h = harness().await;
    for q in [
        "period=days-6",
        "period=days-401",
        "period=days-030",
        "period=days-30.5",
        "period=month-2026-13",
        "period=month-2026-1",
        "period=year-1969",
        "period=year-2101",
        "period=ALL",
        "period=",
        "period",
        "tz=841",
        "tz=-841",
        "tz=%2B60",
        "tz=+60",
        "tz=1.5",
        "tz=",
        "period=all&period=all",
        "tz=0&tz=60",
        "period=all&limit=5",
        "user=bob",
        "period=all&",
        "period=all%26user%3Dbob",
        "period=days%2D30",
        "period=all;tz=0",
    ] {
        let (s, v) = call(&h, "GET", &format!("/v1/me/stats?{q}"), Some(&h.tok_a)).await;
        assert_eq!(s, S::BAD_REQUEST, "{q}");
        assert_eq!(v["error"], "bad query");
    }
    let long = format!("/v1/me/stats?period=all&tz=0{}", "0".repeat(100));
    let (s, _) = call(&h, "GET", &long, Some(&h.tok_a)).await;
    assert_eq!(s, S::BAD_REQUEST);
    for q in [
        "",
        "?",
        "?period=days-7",
        "?period=days-400&tz=840",
        "?tz=-840",
        "?period=month-1970-01",
        "?period=year-2100",
        "?tz=345&period=all",
    ] {
        stats(&h, &h.tok_a, q).await;
    }
}

#[tokio::test]
async fn switched_off_history_shows_nothing() {
    let h = harness().await;
    let alice = user(&h, "alice");
    seed(&h, alice, 1, 10, 1, 3600, 200.0);
    assert_eq!(stats(&h, &h.tok_a, "").await["totals"]["plays"], 1);
    set_history(&h, &h.tok_a, false).await;
    let v = stats(&h, &h.tok_a, "").await;
    assert_eq!(v["enabled"], false);
    assert_eq!(
        v["totals"],
        json!({"plays": 0, "seconds": 0, "tracks": 0, "albums": 0, "artists": 0, "days": 0})
    );
    for k in [
        "top_artists",
        "top_albums",
        "top_tracks",
        "new_artists",
        "by_month",
    ] {
        assert_eq!(v[k], json!([]), "{k}");
    }
    assert_eq!(v["by_hour"], json!(vec![0; 24]));
    assert_eq!(v["by_weekday"], json!(vec![0; 7]));
    assert!(v["longest_streak"].is_null() && v["busiest_day"].is_null());
    // Switched back on, what was kept shows again (not a remembered empty answer).
    set_history(&h, &h.tok_a, true).await;
    assert_eq!(stats(&h, &h.tok_a, "").await["totals"]["plays"], 1);
}

#[tokio::test]
async fn answers_are_cached_and_dropped_when_history_is_cleared() {
    let h = harness().await;
    let (alice, bob) = (user(&h, "alice"), user(&h, "bob"));
    seed(&h, alice, 1, 10, 1, 3600, 200.0);
    seed(&h, bob, 2, 20, 2, 3600, 200.0);
    assert_eq!(
        stats(&h, &h.tok_a, "?period=days-7").await["totals"]["plays"],
        1
    );
    assert_eq!(
        stats(&h, &h.tok_b, "?period=days-7").await["totals"]["plays"],
        1
    );
    // A play recorded meanwhile shows only once the answer is worked out again.
    seed(&h, alice, 1, 10, 1, 1800, 200.0);
    assert_eq!(
        stats(&h, &h.tok_a, "?period=days-7").await["totals"]["plays"],
        1,
        "cached"
    );
    assert_eq!(
        stats(&h, &h.tok_a, "?period=days-7&tz=60").await["totals"]["plays"],
        2
    );
    assert_eq!(
        stats(&h, &h.tok_a, "?period=days-8").await["totals"]["plays"],
        2
    );
    // Clearing drops alice's answers at once; bob's are his own.
    let (s, _) = call(&h, "DELETE", "/v1/me/history", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    for q in ["?period=days-7", "?period=days-7&tz=60", "?period=days-8"] {
        let v = stats(&h, &h.tok_a, q).await;
        assert_eq!(v["totals"]["plays"], 0, "{q}");
        assert_eq!(v["top_artists"], json!([]));
    }
    assert_eq!(
        stats(&h, &h.tok_b, "?period=days-7").await["totals"]["plays"],
        1
    );
}

#[tokio::test]
async fn artist_names_are_looked_up_one_by_one_bounded_and_cached() {
    let h = harness().await;
    let alice = user(&h, "alice");
    // Fifty artists, artist n played n times (so the top twenty are 31–50), all new.
    for artist in 1..=50i64 {
        for i in 0..artist {
            seed(&h, alice, artist as u64, artist, artist, 3600 + i, 60.0);
        }
    }
    let v = stats(&h, &h.tok_a, "?period=all").await;
    let top: Vec<i64> = v["top_artists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_i64().unwrap())
        .collect();
    assert_eq!(top, (31..=50).rev().collect::<Vec<_>>());
    assert_eq!(v["new_artists"].as_array().unwrap().len(), 20);
    assert_eq!(v["top_artists"][0]["name"], "Invented Artist 50");
    let lookups = artist_lookups(&h);
    // Top and new artists are the same twenty here: twenty lookups, never more than 40.
    assert_eq!(lookups.len(), 20, "{lookups:?}");
    for l in &lookups {
        let words = l.as_array().unwrap();
        assert_eq!(words.len(), 4, "{l}");
        assert_eq!(
            (&words[0], &words[1], &words[2]),
            (&json!("artists"), &json!("0"), &json!("1"))
        );
        let id = words[3]
            .as_str()
            .unwrap()
            .strip_prefix("artist_id:")
            .unwrap();
        assert!(
            id.bytes().all(|b| b.is_ascii_digit()),
            "one id per lookup: {l}"
        );
    }
    // Another answer (a different offset) uses the remembered names.
    let v2 = stats(&h, &h.tok_a, "?period=all&tz=120").await;
    assert_eq!(v2["top_artists"][19]["name"], "Invented Artist 31");
    assert_eq!(artist_lookups(&h).len(), 20, "names were remembered");
}

#[tokio::test]
async fn lookups_stay_within_forty_when_top_and_new_differ() {
    let h = harness().await;
    let alice = user(&h, "alice");
    // Artists 101–125 were first played long ago and played most; 201–225 are new.
    for artist in 101..=125i64 {
        seed(&h, alice, artist as u64, 1, artist, 300 * 86_400, 60.0);
        for i in 0..5 {
            seed(&h, alice, artist as u64, 1, artist, 3600 + i, 60.0);
        }
    }
    for artist in 201..=225i64 {
        seed(&h, alice, artist as u64, 2, artist, 7200, 60.0);
    }
    let v = stats(&h, &h.tok_a, "?period=days-30").await;
    assert_eq!(v["top_artists"].as_array().unwrap().len(), 20);
    assert_eq!(v["new_artists"].as_array().unwrap().len(), 20);
    assert!(
        v["new_artists"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["id"].as_i64().unwrap() > 200
                && a["name"].as_str().unwrap().starts_with("Invented Artist"))
    );
    assert_eq!(artist_lookups(&h).len(), 40);
}

#[tokio::test]
async fn names_lms_cannot_give_safely_are_empty() {
    let h = harness().await;
    let alice = user(&h, "alice");
    for artist in [13, 14, 15, 16] {
        seed(&h, alice, artist as u64, artist, artist, 3600, 60.0);
    }
    let v = stats(&h, &h.tok_a, "").await;
    let names: std::collections::HashMap<i64, String> = v["top_artists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["id"].as_i64().unwrap(),
                a["name"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        names[&13], "",
        "a name naming a server path is never handed out"
    );
    assert_eq!(names[&14], "Oddly Spaced Band");
    assert_eq!(names[&15], "", "another artist's name is not this one's");
    assert_eq!(names[&16], "");
    assert!(!v.to_string().contains("/srv/music"));
    assert!(!v.to_string().contains("Somebody Else"));
}

#[tokio::test]
async fn a_stalled_name_lookup_does_not_hold_answers_up() {
    let h = harness_with(|c| c.stats_lookup_timeout = Duration::from_millis(300)).await;
    let alice = user(&h, "alice");
    seed(&h, alice, 1, 1, 950, 3600, 60.0);
    seed(&h, alice, 1, 1, 950, 3601, 60.0);
    seed(&h, alice, 2, 2, 2, 3600, 60.0);
    let started = std::time::Instant::now();
    let v = stats(&h, &h.tok_a, "").await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        v["top_artists"][0],
        json!({"id": 950, "name": "", "plays": 2, "seconds": 120})
    );
    assert_eq!(v["totals"]["plays"], 3);
    let asked = artist_lookups(&h).len();
    assert!(asked <= 2, "{asked}");
    // The failure is remembered for a while: the next answer doesn't wait for LMS again.
    let started = std::time::Instant::now();
    stats(&h, &h.tok_a, "?tz=60").await;
    assert!(started.elapsed() < Duration::from_millis(300));
    assert_eq!(
        artist_lookups(&h)
            .iter()
            .filter(|l| l[3] == json!("artist_id:950"))
            .count(),
        1
    );
    // An answer missing a name LMS failed to give is not remembered.
    assert_eq!(h.gw.stats_answers(), 0);
}
