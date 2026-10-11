//! Offline play sync-back (`POST /v1/plays`) against a fake LMS whose fake plugin checks
//! the signature and records plays (see `common::FakePlugin`).

mod common;
use common::*;

use anywhere_gateway::plays::plugin_key;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A play of `track` that counts (fake track 1 lasts 241.2 s), started an hour or more ago:
/// each key at its own time (a multiple of five minutes), so plays don't overlap.
fn play(key: &str, track: u64) -> Value {
    let slot = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    }) % 8000;
    json!({"key": key, "track_id": track, "played_at": now() - 3600 - slot * 300, "seconds": 200})
}

impl Harness {
    async fn plays_raw(&self, token: Option<&str>, body: Vec<u8>) -> reqwest::Response {
        let mut req = reqwest::Client::new()
            .post(self.url("/v1/plays"))
            .header("content-type", "application/json")
            .body(body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        tokio::time::timeout(Duration::from_secs(20), req.send())
            .await
            .expect("plays request took too long")
            .unwrap()
    }

    async fn plays(&self, token: &str, items: Value) -> (S, Value) {
        let r = self
            .plays_raw(Some(token), items.to_string().into_bytes())
            .await;
        let status = r.status();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    fn recorded(&self) -> Vec<(u64, u64, String)> {
        self.lms.plugin.lock().unwrap().recorded.clone()
    }

    fn plugin_calls(&self) -> usize {
        self.lms.plugin.lock().unwrap().calls
    }

    fn set_plugin(&self, mode: PluginMode) {
        self.lms.plugin.lock().unwrap().mode = mode;
    }

    fn stored(&self, user: &str) -> usize {
        let id = self.accounts.user_id_by_name(user).unwrap().unwrap();
        self.accounts.stored_plays(id).unwrap()
    }
}

fn results(v: &Value) -> Vec<(String, String)> {
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["result"].as_str().unwrap().to_string(),
                r["reason"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn one(v: &Value) -> (String, String) {
    let r = results(v);
    assert_eq!(r.len(), 1, "{v}");
    r[0].clone()
}

#[tokio::test]
async fn a_play_is_recorded_once_however_often_it_is_sent() {
    let h = harness().await;
    let p = play("play-0001", 1);
    let (s, v) = h.plays(&h.tok_a, json!([p])).await;
    assert_eq!(s, S::OK);
    assert_eq!(v["results"][0]["key"], "play-0001");
    assert_eq!(one(&v), ("recorded".into(), "".into()));
    assert!(v.get("retry_after").is_none());
    let rec = h.recorded();
    assert_eq!(rec.len(), 1);
    assert_eq!(rec[0].0, 1);
    assert_eq!(rec[0].1, p["played_at"].as_u64().unwrap());

    // A retry (the answer was lost): duplicate, and LMS isn't asked again.
    let calls = h.plugin_calls();
    let (s, v) = h.plays(&h.tok_a, json!([p])).await;
    assert_eq!(s, S::OK);
    assert_eq!(one(&v), ("duplicate".into(), "".into()));
    assert_eq!(h.recorded().len(), 1);
    assert_eq!(h.plugin_calls(), calls);

    // The same key twice in one upload: the second is a duplicate.
    let q = play("play-0002", 1);
    let (_, v) = h.plays(&h.tok_a, json!([q, q])).await;
    assert_eq!(
        results(&v),
        vec![
            ("recorded".into(), "".into()),
            ("duplicate".into(), "".into())
        ]
    );
    assert_eq!(h.recorded().len(), 2);
    assert_eq!(h.stored("alice"), 2);

    // Keys belong to a user: bob's play with the same key is his own.
    let (_, v) = h.plays(&h.tok_b, json!([p])).await;
    assert_eq!(one(&v).0, "recorded");
    assert_eq!(h.recorded().len(), 3);

    // The cache headers keep answers out of shared caches.
    let r = h
        .plays_raw(
            Some(&h.tok_a),
            json!([play("play-0003", 1)]).to_string().into_bytes(),
        )
        .await;
    assert_eq!(r.headers()["cache-control"], "private, no-store");
    let entries = h.accounts.audit_log(50).unwrap();
    assert!(entries.iter().any(|e| e.event == "plays"));
}

#[tokio::test]
async fn the_plugin_remembering_a_key_counts_as_a_duplicate() {
    // The gateway gave up waiting (or crashed) after LMS recorded a play: the resend is
    // answered from the plugin's own memory, and the gateway now remembers it too.
    let h = harness().await;
    let alice = h.accounts.user_id_by_name("alice").unwrap().unwrap();
    h.lms
        .plugin
        .lock()
        .unwrap()
        .recorded
        .push((1, 0, plugin_key(alice, "play-lost-1")));
    let (_, v) = h.plays(&h.tok_a, json!([play("play-lost-1", 1)])).await;
    assert_eq!(one(&v).0, "duplicate");
    let calls = h.plugin_calls();
    let (_, v) = h.plays(&h.tok_a, json!([play("play-lost-1", 1)])).await;
    assert_eq!(one(&v).0, "duplicate");
    assert_eq!(h.plugin_calls(), calls);
}

#[tokio::test]
async fn plays_need_a_valid_token() {
    let h = harness().await;
    let body = json!([play("play-0001", 1)]).to_string().into_bytes();
    for token in [None, Some("nonsense"), Some(""), Some(&*"x".repeat(5000))] {
        let r = h.plays_raw(token, body.clone()).await;
        assert_eq!(r.status(), S::UNAUTHORIZED, "{token:?}");
    }
    // A revoked device's token stops working at once.
    let p = h.accounts.authenticate(&h.tok_a).unwrap();
    h.accounts.revoke_device(p.device_id).unwrap();
    let r = h.plays_raw(Some(&h.tok_a), body).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert!(h.recorded().is_empty());
    assert_eq!(h.plugin_calls(), 0);
}

#[tokio::test]
async fn malformed_and_oversize_uploads_are_refused() {
    let h = harness().await;
    for body in [
        &b""[..],
        b"{}",
        b"[]",
        b"null",
        b"\"plays\"",
        b"[{\"key\":",
        b"\xff\xfe",
        b"{\"0\": {\"key\": \"play-0001\"}}",
    ] {
        let r = h.plays_raw(Some(&h.tok_a), body.to_vec()).await;
        assert_eq!(
            r.status(),
            S::BAD_REQUEST,
            "{:?}",
            String::from_utf8_lossy(body)
        );
    }
    // More than 200 plays.
    let many: Vec<Value> = (0..201).map(|i| play(&format!("play-{i:05}"), 1)).collect();
    let r = h
        .plays_raw(Some(&h.tok_a), json!(many).to_string().into_bytes())
        .await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    // A body over the size cap, with or without a declared length.
    let huge = format!("[{}]", " ".repeat(200 * 1024));
    let r = h.plays_raw(Some(&h.tok_a), huge.clone().into_bytes()).await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    let chunks = futures_util::stream::iter(
        huge.into_bytes()
            .chunks(4096)
            .map(|c| Ok::<_, std::io::Error>(c.to_vec()))
            .collect::<Vec<_>>(),
    );
    let r = reqwest::Client::new()
        .post(h.url("/v1/plays"))
        .bearer_auth(&h.tok_a)
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    assert_eq!(h.plugin_calls(), 0);
    assert!(h.lms.rpc_calls().is_empty(), "LMS was never asked");
}

#[tokio::test]
async fn hostile_values_are_refused_item_by_item() {
    let h = harness().await;
    let t = now() - 3600;
    let items = json!([
        {"key": "short", "track_id": 1, "played_at": t, "seconds": 200},
        {"key": "play-0001", "track_id": "1", "played_at": t, "seconds": 200},
        {"key": "play-0002", "track_id": 0, "played_at": t, "seconds": 200},
        {"key": "play-0003", "track_id": -1, "played_at": t, "seconds": 200},
        {"key": "play-0004", "track_id": 1.5, "played_at": t, "seconds": 200},
        {"key": "play-0005", "track_id": 1_000_000_000_000u64, "played_at": t, "seconds": 200},
        {"key": "play-0006", "track_id": 1, "played_at": t, "seconds": -1},
        {"key": "play-0007", "track_id": 1, "played_at": t, "seconds": 86_401},
        {"key": "play-0008", "track_id": 1, "played_at": "yesterday", "seconds": 200},
        {"key": "play-0009", "track_id": 1, "played_at": t, "seconds": 200, "player": "aa:bb:cc:dd:ee:01"},
        {"key": "play-0010", "track_id": 1, "played_at": t, "seconds": 200, "track_key": "../../x"},
        {"key": "play 0011\r\nX-Evil: 1", "track_id": 1, "played_at": t, "seconds": 200},
        {"key": "play-0012\u{0}", "track_id": 1, "played_at": t, "seconds": 200},
        [1, 2, 3],
        "play-0013",
        null,
    ]);
    let (s, v) = h.plays(&h.tok_a, items).await;
    assert_eq!(s, S::OK);
    let r = results(&v);
    assert_eq!(r.len(), 16);
    for (i, (result, reason)) in r.iter().enumerate() {
        assert_eq!(
            (result.as_str(), reason.as_str()),
            ("refused", "invalid"),
            "item {i}"
        );
    }
    // Keys are echoed only when they are themselves valid.
    assert_eq!(v["results"][0]["key"], Value::Null);
    assert_eq!(v["results"][1]["key"], "play-0001");
    assert_eq!(v["results"][11]["key"], Value::Null);
    assert_eq!(h.plugin_calls(), 0);
    assert!(h.lms.rpc_calls().is_empty(), "LMS was never asked");
    assert_eq!(h.stored("alice"), 0);
}

#[tokio::test]
async fn future_and_old_plays_are_refused() {
    let h = harness().await;
    let n = now();
    let items = json!([
        {"key": "future-01", "track_id": 1, "played_at": n + 3600, "seconds": 200},
        {"key": "future-02", "track_id": 1, "played_at": n, "seconds": 3000},
        {"key": "future-03", "track_id": 1, "played_at": u64::MAX, "seconds": 200},
        {"key": "too-old-1", "track_id": 1, "played_at": n - 91 * 86_400, "seconds": 200},
        {"key": "too-old-2", "track_id": 1, "played_at": 0, "seconds": 200},
        {"key": "skew-ok-1", "track_id": 1, "played_at": n + 60, "seconds": 200},
        {"key": "old-ok-01", "track_id": 1, "played_at": n - 89 * 86_400, "seconds": 200},
    ]);
    let (_, v) = h.plays(&h.tok_a, items).await;
    let r = results(&v);
    let reasons: Vec<&str> = r
        .iter()
        .map(|(a, b)| {
            if a == "refused" {
                b.as_str()
            } else {
                a.as_str()
            }
        })
        .collect();
    assert_eq!(
        reasons,
        [
            "in_future",
            "in_future",
            "in_future",
            "too_old",
            "too_old",
            "recorded",
            "recorded"
        ]
    );
    assert_eq!(h.recorded().len(), 2);
}

#[tokio::test]
async fn only_library_tracks_that_were_really_listened_to_count() {
    let h = harness().await;
    let key1: String = {
        let r = reqwest::Client::new()
            .get(h.url("/v1/download/track/1/info"))
            .bearer_auth(&h.tok_a)
            .send()
            .await
            .unwrap();
        let v: Value = r.json().await.unwrap();
        v["key"].as_str().unwrap().to_string()
    };
    let t = now() - 3600;
    let items = json!([
        // A radio stream, and a track LMS doesn't know.
        {"key": "radio-001", "track_id": 2, "played_at": t, "seconds": 600},
        {"key": "unknown-1", "track_id": 99_999, "played_at": t, "seconds": 600},
        // Track 1 lasts 241.2 s: half of it is 120.6 s.
        {"key": "short-001", "track_id": 1, "played_at": t, "seconds": 120},
        {"key": "half-0001", "track_id": 1, "played_at": t, "seconds": 120.6},
        // The stable key: track 7 is track 1's file renumbered (fine); track 3 is another.
        {"key": "renumber1", "track_id": 7, "played_at": t, "seconds": 200, "track_key": key1},
        {"key": "changed01", "track_id": 3, "played_at": t, "seconds": 200, "track_key": key1},
    ]);
    let (_, v) = h.plays(&h.tok_a, items).await;
    assert_eq!(
        results(&v),
        vec![
            ("refused".into(), "not_library".into()),
            ("refused".into(), "not_found".into()),
            ("refused".into(), "too_short".into()),
            ("recorded".into(), "".into()),
            ("recorded".into(), "".into()),
            ("refused".into(), "track_changed".into()),
        ]
    );
    let ids: Vec<u64> = h.recorded().iter().map(|r| r.0).collect();
    assert_eq!(ids, [1, 7]);
    // Refused plays leave no key behind: sent again, they are judged again.
    assert_eq!(h.stored("alice"), 2);

    // A long track counts after four minutes, whatever its length.
    h.lms.add_track(50, "flc", vec![0; 10], 3600.0);
    let items = json!([
        {"key": "long-0001", "track_id": 50, "played_at": t, "seconds": 239},
        {"key": "long-0002", "track_id": 50, "played_at": t, "seconds": 240},
    ]);
    let (_, v) = h.plays(&h.tok_a, items).await;
    assert_eq!(
        results(&v),
        vec![
            ("refused".into(), "too_short".into()),
            ("recorded".into(), "".into())
        ]
    );
}

#[tokio::test]
async fn the_same_listening_under_new_keys_counts_once() {
    // Pentest 2026-10 (A7): every fresh key counted, so one listener could add hundreds of
    // plays of a track an hour by sending the same play again under new keys.
    let h = harness().await;
    let t = now() - 7200;
    // Track 1 lasts 241.2 s, so a play counts after 120.6 s: plays of it can't start
    // closer together than that.
    let items = json!([
        {"key": "first-001", "track_id": 1, "played_at": t, "seconds": 200},
        {"key": "copy-0001", "track_id": 1, "played_at": t, "seconds": 200},
        {"key": "copy-0002", "track_id": 1, "played_at": t + 60, "seconds": 200},
        {"key": "copy-0003", "track_id": 1, "played_at": t - 120, "seconds": 200},
        // Far enough apart: a second listen.
        {"key": "again-001", "track_id": 1, "played_at": t + 121, "seconds": 200},
        // Another track at the same time (another device): fine.
        {"key": "other-001", "track_id": 7, "played_at": t, "seconds": 200},
    ]);
    let (_, v) = h.plays(&h.tok_a, items).await;
    assert_eq!(
        results(&v),
        vec![
            ("recorded".into(), "".into()),
            ("refused".into(), "overlap".into()),
            ("refused".into(), "overlap".into()),
            ("refused".into(), "overlap".into()),
            ("recorded".into(), "".into()),
            ("recorded".into(), "".into()),
        ]
    );
    assert_eq!(h.recorded().len(), 3);
    // In a later upload too; and the original key is still a duplicate, not an overlap.
    let items = json!([
        {"key": "copy-0004", "track_id": 1, "played_at": t + 10, "seconds": 200},
        {"key": "first-001", "track_id": 1, "played_at": t, "seconds": 200},
    ]);
    let (_, v) = h.plays(&h.tok_a, items).await;
    assert_eq!(
        results(&v),
        vec![
            ("refused".into(), "overlap".into()),
            ("duplicate".into(), "".into()),
        ]
    );
    // Another user's listening is their own.
    let (_, v) = h
        .plays(
            &h.tok_b,
            json!([{"key": "bob-00001", "track_id": 1, "played_at": t, "seconds": 200}]),
        )
        .await;
    assert_eq!(one(&v).0, "recorded");
    assert_eq!(h.recorded().len(), 4);
}

#[tokio::test]
async fn the_plugin_refusing_or_missing_is_reported() {
    let h = harness().await;
    for (answer, want) in [
        (json!({"error": "not_library"}), ("refused", "not_library")),
        (json!({"error": "not_found"}), ("refused", "not_found")),
        (json!({"error": "busy"}), ("retry", "server_busy")),
        (
            json!({"error": "refused: bad signature"}),
            ("retry", "server_error"),
        ),
        // LMS without the plugin answers an unknown command with an empty result.
        (json!({}), ("retry", "unsupported")),
    ] {
        h.set_plugin(PluginMode::Answer(answer.clone()));
        let (_, v) = h.plays(&h.tok_a, json!([play("play-0001", 1)])).await;
        assert_eq!(one(&v), (want.0.into(), want.1.into()), "{answer}");
        if want.0 == "retry" {
            assert_eq!(v["retry_after"], 60);
        }
    }
    // Nothing was kept, so once the plugin works the play is recorded.
    assert_eq!(h.stored("alice"), 0);
    h.set_plugin(PluginMode::Record);
    let (_, v) = h.plays(&h.tok_a, json!([play("play-0001", 1)])).await;
    assert_eq!(one(&v).0, "recorded");
}

#[tokio::test]
async fn a_wrong_signing_key_is_refused_by_the_plugin() {
    let h = harness_with(|c| c.plays_secret = Some("f".repeat(64))).await;
    let (_, v) = h.plays(&h.tok_a, json!([play("play-0001", 1)])).await;
    assert_eq!(one(&v), ("retry".into(), "server_error".into()));
    assert!(h.recorded().is_empty());
}

#[tokio::test]
async fn without_a_signing_key_play_sync_is_unavailable() {
    let h = harness_with(|c| c.plays_secret = None).await;
    let (s, _) = h.plays(&h.tok_a, json!([play("play-0001", 1)])).await;
    assert_eq!(s, S::SERVICE_UNAVAILABLE);
    assert!(h.lms.rpc_calls().is_empty());
}

#[tokio::test]
async fn an_unreachable_lms_means_retry_later() {
    // Nothing listens on port 1.
    let h = harness_with(|c| c.lms_http = "127.0.0.1:1".into()).await;
    let items: Vec<Value> = (0..50).map(|i| play(&format!("play-{i:05}"), 1)).collect();
    let started = std::time::Instant::now();
    let (s, v) = h.plays(&h.tok_a, json!(items)).await;
    assert_eq!(s, S::OK);
    assert!(
        results(&v)
            .iter()
            .all(|r| *r == ("retry".into(), "server_unavailable".into())),
        "{v}"
    );
    assert_eq!(v["retry_after"], 60);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the rest weren't each tried"
    );
    assert_eq!(h.stored("alice"), 0, "keys are given back");
}

#[tokio::test]
async fn duplicates_are_answered_even_while_lms_is_down() {
    let h = harness().await;
    let p = play("play-0001", 1);
    let (_, v) = h.plays(&h.tok_a, json!([p])).await;
    assert_eq!(one(&v).0, "recorded");
    // LMS now unreachable: point a second gateway with the same accounts at a closed port.
    let gw = Gateway::new(
        Config {
            lms_http: "127.0.0.1:1".into(),
            plays_secret: Some(PLAYS_SECRET.into()),
            ..Config::default()
        },
        h.accounts.clone(),
    );
    let app = router(gw);
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/plays"))
        .bearer_auth(&h.tok_a)
        .body(json!([p, play("play-0002", 1)]).to_string())
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        results(&v),
        vec![
            ("duplicate".into(), "".into()),
            ("retry".into(), "server_unavailable".into())
        ]
    );
}

#[tokio::test]
async fn a_hung_lms_is_bounded_by_the_upload_budget() {
    let h = harness_with(|c| c.plays_budget = Duration::from_secs(1)).await;
    h.set_plugin(PluginMode::Hang);
    let items: Vec<Value> = (0..5).map(|i| play(&format!("play-{i:05}"), 1)).collect();
    let started = std::time::Instant::now();
    let (s, v) = h.plays(&h.tok_a, json!(items)).await;
    assert_eq!(s, S::OK);
    assert!(results(&v).iter().all(|r| r.0 == "retry"), "{v}");
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(
        h.plugin_calls(),
        1,
        "one hang, then the rest wait for the next upload"
    );
    assert_eq!(h.stored("alice"), 0);
}

#[tokio::test]
async fn one_upload_at_a_time_per_user() {
    let h = harness_with(|c| c.plays_budget = Duration::from_secs(2)).await;
    h.set_plugin(PluginMode::Hang);
    let slow = {
        let (url, tok) = (h.url("/v1/plays"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .body(json!([play("play-slow1", 1)]).to_string())
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    wait_for(|| h.lms.plugin.lock().unwrap().calls == 1).await;
    let r = h
        .plays_raw(
            Some(&h.tok_a),
            json!([play("play-0002", 1)]).to_string().into_bytes(),
        )
        .await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    // Another user isn't held up.
    h.set_plugin(PluginMode::Answer(json!({"ok": 1})));
    let (s, v) = h.plays(&h.tok_b, json!([play("play-0003", 1)])).await;
    assert_eq!(s, S::OK);
    assert_eq!(one(&v).0, "recorded");
    assert_eq!(slow.await.unwrap(), S::OK);
    // Once the first upload is over, the user may upload again.
    let (s, _) = h.plays(&h.tok_a, json!([play("play-0004", 1)])).await;
    assert_eq!(s, S::OK);
}

#[tokio::test]
async fn plays_and_uploads_are_rate_limited_per_user() {
    let h = harness_with(|c| {
        c.plays_per_hour = 3;
        c.plays_requests_per_minute = 3;
    })
    .await;
    let items: Vec<Value> = (0..5).map(|i| play(&format!("play-{i:05}"), 1)).collect();
    let (_, v) = h.plays(&h.tok_a, json!(items)).await;
    assert_eq!(
        results(&v).iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
        ["", "", "", "rate_limited", "rate_limited"]
    );
    assert_eq!(v["retry_after"], 60);
    assert_eq!(h.recorded().len(), 3);
    assert_eq!(
        h.stored("alice"),
        3,
        "keys of plays to retry are given back"
    );
    // Bob has his own allowance.
    let (_, v) = h.plays(&h.tok_b, json!([play("play-bob01", 1)])).await;
    assert_eq!(one(&v).0, "recorded");
    // Uploads: three a minute.
    let (s, _) = h.plays(&h.tok_a, json!([play("play-0001", 1)])).await;
    assert_eq!(s, S::OK);
    let (s, _) = h.plays(&h.tok_a, json!([play("play-0002", 1)])).await;
    assert_eq!(s, S::OK);
    let r = h
        .plays_raw(
            Some(&h.tok_a),
            json!([play("play-0003", 1)]).to_string().into_bytes(),
        )
        .await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert_eq!(r.headers()["retry-after"], "60");
}

#[tokio::test]
async fn clients_cannot_send_the_plugin_command_themselves() {
    let h = harness().await;
    let p = play("play-0001", 1);
    let ts = now();
    let sig = anywhere_gateway::plays::signature(
        PLAYS_SECRET,
        1,
        p["played_at"].as_u64().unwrap(),
        "play-0001",
        ts,
    );
    let cmd = json!([
        "anywhere",
        "play_record",
        "track_id:1",
        format!("played_at:{}", p["played_at"]),
        "key:play-0001",
        format!("ts:{ts}"),
        format!("sig:{sig}")
    ]);
    let r = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(&json!({"player": "", "cmd": cmd}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    assert_eq!(h.plugin_calls(), 0);
}

/// Robustness: random uploads never crash the gateway and never record anything invalid.
#[tokio::test]
async fn random_uploads_are_survived() {
    let h = harness_with(|c| c.plays_requests_per_minute = 10_000).await;
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let n = now();
    for _ in 0..60 {
        let count = (next() % 8) as usize;
        let items: Vec<Value> = (0..count)
            .map(|_| {
                let v = |r: u64| match r % 7 {
                    0 => json!(r % 10),
                    1 => json!(n - r % 200_000),
                    2 => json!(-((r % 5) as i64)),
                    3 => json!(format!("k{:x}", r)),
                    4 => Value::Null,
                    5 => json!((r % 100_000) as f64 / 3.0),
                    _ => json!([r % 3]),
                };
                json!({"key": v(next()), "track_id": v(next()), "played_at": v(next()), "seconds": v(next())})
            })
            .collect();
        let r = h
            .plays_raw(Some(&h.tok_a), json!(items).to_string().into_bytes())
            .await;
        assert!(
            [S::OK, S::BAD_REQUEST].contains(&r.status()),
            "{}",
            r.status()
        );
        // Random bytes too.
        let raw: Vec<u8> = (0..(next() % 300)).map(|_| next() as u8).collect();
        let r = h.plays_raw(Some(&h.tok_a), raw).await;
        assert!([S::OK, S::BAD_REQUEST].contains(&r.status()));
    }
    for (id, at, key) in h.recorded() {
        assert!((1..=999_999_999_999).contains(&id));
        assert!(at <= n + 300 && at + 90 * 86_400 >= n);
        assert!(anywhere_gateway::plays::valid_key(&key));
    }
    let r = reqwest::get(h.url("/healthz")).await.unwrap();
    assert_eq!(r.status(), S::OK);
}
