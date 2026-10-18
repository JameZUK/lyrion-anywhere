//! Wave 1 per-user endpoints against a fake LMS: listening history (live and offline),
//! the Home layout, stations (built-in, Random Mix and pinned from menus) and pinned
//! menus. Security first: every endpoint refuses strangers, other users' data, hostile ids
//! and bodies, and pinned menu actions run only while LMS still offers them.

mod common;
use common::*;

use anywhere_core::proto::{ClientMsg, Stat};
use reqwest::StatusCode as S;
use serde_json::{Value, json};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

impl Harness {
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (S, Value) {
        let c = reqwest::Client::new();
        let url = self.url(path);
        let mut req = match method {
            "GET" => c.get(url),
            "PUT" => c.put(url),
            "POST" => c.post(url),
            "PATCH" => c.patch(url),
            "DELETE" => c.delete(url),
            _ => unreachable!(),
        };
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let r = tokio::time::timeout(Duration::from_secs(20), req.send())
            .await
            .expect("request took too long")
            .unwrap();
        let status = r.status();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn raw(&self, method: &str, path: &str, token: &str, body: Vec<u8>) -> S {
        let c = reqwest::Client::new();
        let url = self.url(path);
        let req = match method {
            "PUT" => c.put(url),
            _ => c.post(url),
        };
        req.bearer_auth(token)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
            .status()
    }

    fn a(&self) -> String {
        format_mac(&self.mac_a)
    }

    fn b(&self) -> String {
        format_mac(&self.mac_b)
    }

    async fn rpc(&self, token: &str, player: &str, cmd: Value) -> S {
        reqwest::Client::new()
            .post(self.url("/v1/rpc"))
            .bearer_auth(token)
            .json(&json!({ "player": player, "cmd": cmd }))
            .send()
            .await
            .unwrap()
            .status()
    }

    /// LMS commands the gateway sent for `player` whose first word is `word`.
    fn sent(&self, player: &str, word: &str) -> Vec<Value> {
        self.lms
            .rpc_calls()
            .into_iter()
            .filter(|p| p[0] == player && p[1][0] == word)
            .map(|p| p[1].clone())
            .collect()
    }

    async fn history(&self, token: &str) -> Value {
        let (s, v) = self.call("GET", "/v1/me/history", Some(token), None).await;
        assert_eq!(s, S::OK, "{v}");
        v
    }
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_endpoint_needs_a_valid_token() {
    let h = harness().await;
    let endpoints = [
        ("GET", "/v1/me/history"),
        ("DELETE", "/v1/me/history"),
        ("PATCH", "/v1/me/history"),
        ("GET", "/v1/me/shelves/recent"),
        ("GET", "/v1/me/home"),
        ("PUT", "/v1/me/home"),
        ("DELETE", "/v1/me/home"),
        ("GET", "/v1/me/stations"),
        ("POST", "/v1/me/stations"),
        ("DELETE", "/v1/me/stations/1"),
        ("POST", "/v1/me/stations/1/next"),
        ("GET", "/v1/me/menus"),
        ("POST", "/v1/me/menus"),
        ("DELETE", "/v1/me/menus/1"),
        ("POST", "/v1/me/menus/1/open"),
    ];
    for (m, p) in endpoints {
        let body = (m != "GET" && m != "DELETE").then(|| json!({}));
        for token in [None, Some("garbage"), Some("")] {
            let (s, _) = h.call(m, p, token, body.clone()).await;
            assert_eq!(s, S::UNAUTHORIZED, "{m} {p} with {token:?}");
        }
    }
    // A revoked device's token stops working at once.
    let (_, v) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(v["version"], 1);
    h.accounts.logout(&h.tok_a).unwrap();
    let (s, _) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(s, S::UNAUTHORIZED);
}

#[tokio::test]
async fn requests_are_rate_limited_per_user() {
    let h = harness().await;
    let mut limited = false;
    for _ in 0..anywhere_gateway::history::ME_REQUESTS_PER_MINUTE + 1 {
        let (s, _) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
        if s == S::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
        assert_eq!(s, S::OK);
    }
    assert!(limited, "never rate limited");
    // Bob is unaffected.
    let (s, _) = h.call("GET", "/v1/me/home", Some(&h.tok_b), None).await;
    assert_eq!(s, S::OK);
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// An offline play of fake track 1 (241.2 s) that counts.
fn play(key: &str, ago: u64) -> Value {
    json!({"key": key, "track_id": 1, "played_at": now() - ago, "seconds": 200})
}

async fn send_plays(h: &Harness, token: &str, plays: Value) {
    let (s, v) = h.call("POST", "/v1/plays", Some(token), Some(plays)).await;
    assert_eq!(s, S::OK, "{v}");
}

#[tokio::test]
async fn offline_plays_go_into_the_users_own_history() {
    let h = harness().await;
    send_plays(
        &h,
        &h.tok_a,
        json!([play("offline-0001", 3600), play("offline-0002", 7200)]),
    )
    .await;
    let v = h.history(&h.tok_a).await;
    assert_eq!(v["enabled"], true);
    assert_eq!(v["retention_days"], 396);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["track_id"], 1);
    assert_eq!(items[0]["source"], "offline");
    assert_eq!(items[0]["album_id"], 7);
    assert_eq!(items[0]["seconds"], 200.0);
    let key = items[0]["track_key"].as_str().unwrap();
    assert!(key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(!v.to_string().contains("/srv/music"), "no paths: {v}");
    // Never another user's.
    let bob = h.history(&h.tok_b).await;
    assert_eq!(bob["items"].as_array().unwrap().len(), 0);
    // A duplicate upload adds nothing.
    send_plays(&h, &h.tok_a, json!([play("offline-0001", 3600)])).await;
    assert_eq!(
        h.history(&h.tok_a).await["items"].as_array().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn history_pages_clears_and_switches_off() {
    let h = harness().await;
    let plays: Vec<Value> = (0..5)
        .map(|i| play(&format!("page-key-{i:04}"), 3600 + i * 600))
        .collect();
    send_plays(&h, &h.tok_a, json!(plays)).await;
    send_plays(&h, &h.tok_b, json!([play("bob-key-0001", 3600)])).await;
    let (s, v) = h
        .call("GET", "/v1/me/history?limit=2", Some(&h.tok_a), None)
        .await;
    assert_eq!(s, S::OK);
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    let next = v["next"].as_str().unwrap().to_string();
    let (_, v2) = h
        .call(
            "GET",
            &format!("/v1/me/history?limit=10&before={next}"),
            Some(&h.tok_a),
            None,
        )
        .await;
    assert_eq!(v2["items"].as_array().unwrap().len(), 3);
    assert!(v2["next"].is_null());
    // Bad queries.
    for q in [
        "limit=0",
        "limit=201",
        "limit=abc",
        "limit=-1",
        "limit=1e2",
        "before=x",
        "before=1",
        "before=1-2-3",
        "before=99999999999999999-1",
        "other=1",
        "limit=1&limit=2",
    ] {
        let (s, _) = h
            .call("GET", &format!("/v1/me/history?{q}"), Some(&h.tok_a), None)
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{q}");
    }
    // Switch off: nothing more is recorded (LMS still counts the play).
    for bad in [
        json!({}),
        json!({"enabled": "no"}),
        json!({"enabled": 0}),
        json!({"enabled": false, "x": 1}),
        json!([false]),
    ] {
        let (s, _) = h
            .call("PATCH", "/v1/me/history", Some(&h.tok_a), Some(bad.clone()))
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{bad}");
    }
    let (s, v) = h
        .call(
            "PATCH",
            "/v1/me/history",
            Some(&h.tok_a),
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!((s, v["enabled"].clone()), (S::OK, json!(false)));
    send_plays(&h, &h.tok_a, json!([play("after-off-001", 300)])).await;
    assert_eq!(
        h.lms.plugin.lock().unwrap().recorded.len(),
        7,
        "LMS still counts it"
    );
    let v = h.history(&h.tok_a).await;
    assert_eq!(v["enabled"], false);
    assert_eq!(v["items"].as_array().unwrap().len(), 5);
    // Clear: only alice's.
    let (s, v) = h
        .call("DELETE", "/v1/me/history", Some(&h.tok_a), None)
        .await;
    assert_eq!((s, v["removed"].clone()), (S::OK, json!(5)));
    assert_eq!(
        h.history(&h.tok_a).await["items"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        h.history(&h.tok_b).await["items"].as_array().unwrap().len(),
        1
    );
}

fn stat(event: &[u8; 4], elapsed_ms: u32) -> Vec<u8> {
    ClientMsg::Stat(Stat {
        event: *event,
        elapsed_seconds: elapsed_ms / 1000,
        elapsed_milliseconds: elapsed_ms,
        ..Stat::default()
    })
    .encode()
}

/// Play `track` on alice's own device for `seconds` of reports a second apart (sped up:
/// each report 100 ms of wall time counts at most 100 ms + the slack).
async fn play_live(h: &Harness, track: u64, reports: u32, step_ms: u32) {
    *h.lms.status_track.lock().unwrap() = Some(track);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat(b"STMs", 0).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    for i in 1..=reports {
        tokio::time::sleep(Duration::from_millis(step_ms as u64)).await;
        ws.send(Message::Binary(stat(b"STMt", i * step_ms).into()))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = ws.close(None).await;
}

fn live_harness() -> impl FnOnce(&mut Config) {
    |c: &mut Config| c.history_lookup_delay = Duration::from_millis(20)
}

#[tokio::test]
async fn live_plays_on_the_users_own_player_are_recorded() {
    let h = harness_with(live_harness()).await;
    // A 2-second track: it counts after 1 s heard.
    h.lms.add_track(40, "flc", b"audio".to_vec(), 2.0);
    play_live(&h, 40, 15, 100).await;
    wait_for(|| {
        anywhere_gateway::history::count(
            &h.accounts,
            h.accounts.user_id_by_name("alice").unwrap().unwrap(),
        )
        .unwrap()
            == 1
    })
    .await;
    let v = h.history(&h.tok_a).await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{v}");
    assert_eq!(items[0]["track_id"], 40);
    assert_eq!(items[0]["source"], "live");
    assert!(items[0]["seconds"].as_f64().unwrap() >= 1.0);
    // Bob played nothing: his history stays empty.
    assert_eq!(
        h.history(&h.tok_b).await["items"].as_array().unwrap().len(),
        0
    );
    // LMS was asked about alice's player only.
    assert!(!h.sent(&h.a(), "status").is_empty());
    assert!(h.sent(&h.b(), "status").is_empty());
}

#[tokio::test]
async fn live_plays_too_short_radio_or_switched_off_are_not_recorded() {
    let h = harness_with(live_harness()).await;
    let alice = h.accounts.user_id_by_name("alice").unwrap().unwrap();
    // Too short: 0.5 s of a 60 s track.
    h.lms.add_track(41, "flc", b"audio".to_vec(), 60.0);
    play_live(&h, 41, 5, 100).await;
    // Radio (a remote stream): never.
    play_live(&h, 2, 25, 100).await;
    // Switched off: nothing.
    h.call(
        "PATCH",
        "/v1/me/history",
        Some(&h.tok_a),
        Some(json!({"enabled": false})),
    )
    .await;
    h.lms.add_track(42, "flc", b"audio".to_vec(), 2.0);
    play_live(&h, 42, 15, 100).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        anywhere_gateway::history::count(&h.accounts, alice).unwrap(),
        0
    );
    // Reports claiming far more than wall time (a seek, or a lying client) count nothing.
    h.call(
        "PATCH",
        "/v1/me/history",
        Some(&h.tok_a),
        Some(json!({"enabled": true})),
    )
    .await;
    *h.lms.status_track.lock().unwrap() = Some(41);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat(b"STMs", 0).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    for i in 1..=5u32 {
        ws.send(Message::Binary(stat(b"STMt", i * 50_000).into()))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = ws.close(None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        anywhere_gateway::history::count(&h.accounts, alice).unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------
// Shelves
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shelves_return_album_ids_only() {
    let h = harness().await;
    send_plays(&h, &h.tok_a, json!([play("shelf-key-0001", 3600)])).await;
    for name in ["recent", "most", "on_this_day"] {
        let (s, v) = h
            .call(
                "GET",
                &format!("/v1/me/shelves/{name}?limit=5"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(s, S::OK, "{name}");
        assert_eq!(v["shelf"], name);
        assert!(v["albums"].as_array().unwrap().iter().all(Value::is_i64));
        assert_eq!(v.as_object().unwrap().len(), 2, "ids only: {v}");
    }
    let (_, v) = h
        .call("GET", "/v1/me/shelves/recent", Some(&h.tok_a), None)
        .await;
    assert_eq!(v["albums"][0], 7, "the user's own play first");
    // The shelves are the user's own (review low): LMS's server-wide last-played and
    // play-count orders would show one user what the others played, so LMS isn't asked.
    let sorts: Vec<String> = h
        .sent("", "albums")
        .iter()
        .filter_map(|c| {
            c.as_array()
                .unwrap()
                .iter()
                .find_map(|w| w.as_str()?.strip_prefix("sort:").map(String::from))
        })
        .collect();
    assert!(sorts.is_empty(), "{sorts:?}");
    // Another user who has played nothing sees nothing (not anyone else's albums).
    for name in ["recent", "most", "on_this_day"] {
        let (s, v) = h
            .call(
                "GET",
                &format!("/v1/me/shelves/{name}"),
                Some(&h.tok_b),
                None,
            )
            .await;
        assert_eq!(s, S::OK, "{name}");
        assert_eq!(v["albums"], json!([]), "{name}: {v}");
    }
    for bad in [
        "/v1/me/shelves/new",
        "/v1/me/shelves/..%2Fhistory",
        "/v1/me/shelves/RECENT",
        "/v1/me/shelves/x",
    ] {
        let (s, _) = h.call("GET", bad, Some(&h.tok_a), None).await;
        assert_eq!(s, S::NOT_FOUND, "{bad}");
    }
    for q in ["limit=0", "limit=51", "limit=x", "sort=1"] {
        let (s, _) = h
            .call(
                "GET",
                &format!("/v1/me/shelves/most?{q}"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{q}");
    }
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

#[tokio::test]
async fn home_layouts_are_per_user_validated_and_capped() {
    let h = harness().await;
    let (s, v) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(s, S::OK);
    assert_eq!(v["default"], true);
    let types: Vec<&str> = v["shelves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["recent", "stations", "new", "most", "on_this_day"]);
    // A station of alice's own can be a shelf.
    let (_, st) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    let mine = st["stations"][0]["id"].as_i64().unwrap();
    let layout = json!({"version": 1, "shelves": [{"type": "most"}, {"type": "station", "id": mine}, {"type": "recent", "hidden": true}]});
    let (s, v) = h
        .call("PUT", "/v1/me/home", Some(&h.tok_a), Some(layout.clone()))
        .await;
    assert_eq!(s, S::OK, "{v}");
    let (_, v) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(v["default"], false);
    assert_eq!(v["shelves"], layout["shelves"]);
    // Bob's is untouched, and he can't use alice's station.
    let (_, vb) = h.call("GET", "/v1/me/home", Some(&h.tok_b), None).await;
    assert_eq!(vb["default"], true);
    let steal = json!({"version": 1, "shelves": [{"type": "station", "id": mine}]});
    let (s, _) = h
        .call("PUT", "/v1/me/home", Some(&h.tok_b), Some(steal))
        .await;
    assert_eq!(s, S::BAD_REQUEST);
    for bad in [
        json!({"version": 2, "shelves": []}),
        json!({"version": 1, "shelves": [{"type": "unknown"}]}),
        json!({"version": 1, "shelves": [{"type": "station", "id": 999999}]}),
        json!({"version": 1, "shelves": [{"type": "menu", "id": mine}]}),
        json!({"version": 1, "shelves": [{"type": "station", "id": "1 OR 1=1"}]}),
        json!({"version": 1, "shelves": [{"type": "station", "id": -1}]}),
        json!({"version": 1, "shelves": [{"type": "recent", "label": "<img src=x onerror=alert(1)>"}]}),
        json!({"version": 1, "shelves": [{"type": "recent"}, {"type": "recent"}]}),
        json!([]),
    ] {
        let (s, _) = h
            .call("PUT", "/v1/me/home", Some(&h.tok_a), Some(bad.clone()))
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{bad}");
    }
    // Oversize: refused before it is read.
    let big = format!(
        r#"{{"version":1,"shelves":[],"pad":"{}"}}"#,
        "x".repeat(5000)
    );
    assert_eq!(
        h.raw("PUT", "/v1/me/home", &h.tok_a, big.into_bytes())
            .await,
        S::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        h.raw("PUT", "/v1/me/home", &h.tok_a, b"{not json".to_vec())
            .await,
        S::BAD_REQUEST
    );
    // The saved layout survived all that.
    let (_, v) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(v["shelves"], layout["shelves"]);
    // Deleting the station drops its shelf; resetting restores the default.
    let (s, _) = h
        .call(
            "DELETE",
            &format!("/v1/me/stations/{mine}"),
            Some(&h.tok_a),
            None,
        )
        .await;
    assert_eq!(s, S::OK);
    let (_, v) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(v["shelves"].as_array().unwrap().len(), 2);
    let (s, v) = h.call("DELETE", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!((s, v["default"].clone()), (S::OK, json!(true)));
}

// ---------------------------------------------------------------------------
// Stations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn built_in_stations_are_seeded_per_user_and_owned() {
    let h = harness().await;
    let (s, v) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    assert_eq!(s, S::OK);
    let kinds: Vec<&str> = v["stations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["forgotten", "fresh", "deep_cuts"]);
    let alices: Vec<i64> = v["stations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_i64().unwrap())
        .collect();
    // Deleting one keeps it deleted (it is not seeded again).
    h.call(
        "DELETE",
        &format!("/v1/me/stations/{}", alices[0]),
        Some(&h.tok_a),
        None,
    )
    .await;
    let (_, v) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    assert_eq!(v["stations"].as_array().unwrap().len(), 2);
    // Bob can neither delete nor play alice's stations.
    let (_, vb) = h.call("GET", "/v1/me/stations", Some(&h.tok_b), None).await;
    assert_eq!(vb["stations"].as_array().unwrap().len(), 3);
    let (s, _) = h
        .call(
            "DELETE",
            &format!("/v1/me/stations/{}", alices[1]),
            Some(&h.tok_b),
            None,
        )
        .await;
    assert_eq!(s, S::NOT_FOUND);
    let (s, _) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{}/next", alices[1]),
            Some(&h.tok_b),
            Some(json!({"player": h.b()})),
        )
        .await;
    assert_eq!(s, S::NOT_FOUND);
    // Hostile ids in the path.
    for id in [
        "0",
        "-1",
        "abc",
        "1e3",
        "01",
        "1%20OR%201",
        "99999999999999999999",
        "%2e%2e",
    ] {
        let (s, _) = h
            .call(
                "DELETE",
                &format!("/v1/me/stations/{id}"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(s, S::NOT_FOUND, "{id}");
        let (s, _) = h
            .call(
                "POST",
                &format!("/v1/me/stations/{id}/next"),
                Some(&h.tok_a),
                Some(json!({"player": h.a()})),
            )
            .await;
        assert_eq!(s, S::NOT_FOUND, "{id}");
    }
}

#[tokio::test]
async fn stations_are_created_with_strict_specs() {
    let h = harness().await;
    let (s, v) = h
        .call(
            "POST",
            "/v1/me/stations",
            Some(&h.tok_a),
            Some(json!({"name": "Jazz & Blues", "spec": {"kind": "genre", "genres": [3, 4]}})),
        )
        .await;
    assert_eq!(s, S::CREATED, "{v}");
    assert_eq!(v["kind"], "genre");
    for bad in [
        json!({"name": "x"}),
        json!({"spec": {"kind": "fresh"}}),
        json!({"name": "", "spec": {"kind": "fresh"}}),
        json!({"name": "x".repeat(65), "spec": {"kind": "fresh"}}),
        json!({"name": "x", "spec": {"kind": "sql", "q": "DROP TABLE users"}}),
        json!({"name": "x", "spec": {"kind": "genre", "genres": ["3"]}}),
        json!({"name": "x", "spec": {"kind": "decade", "decade": 1985}}),
        json!({"name": "x", "spec": {"kind": "fresh"}, "user_id": 2}),
        json!({"name": "x", "spec": {"kind": "menu", "steps": [], "play": ["x", "y"]}}),
        json!([1]),
    ] {
        let (s, _) = h
            .call("POST", "/v1/me/stations", Some(&h.tok_a), Some(bad.clone()))
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{bad}");
    }
    let big = format!(
        r#"{{"name":"x","spec":{{"kind":"fresh"}},"pad":"{}"}}"#,
        "y".repeat(5000)
    );
    assert_eq!(
        h.raw("POST", "/v1/me/stations", &h.tok_a, big.into_bytes())
            .await,
        S::PAYLOAD_TOO_LARGE
    );
    // At most 100 stations and pinned menus a user.
    let mut full = false;
    for i in 0..110 {
        let (s, _) = h
            .call(
                "POST",
                "/v1/me/stations",
                Some(&h.tok_a),
                Some(json!({"name": format!("S{i}"), "spec": {"kind": "fresh"}})),
            )
            .await;
        if s == S::CONFLICT {
            full = true;
            break;
        }
        if s == S::TOO_MANY_REQUESTS {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        assert_eq!(s, S::CREATED);
    }
    assert!(full, "never capped");
}

async fn station(h: &Harness, token: &str, spec: Value) -> i64 {
    let (s, v) = h
        .call(
            "POST",
            "/v1/me/stations",
            Some(token),
            Some(json!({"name": "Test", "spec": spec, "player": format_mac(&h.mac_a)})),
        )
        .await;
    assert_eq!(s, S::CREATED, "{v}");
    v["id"].as_i64().unwrap()
}

#[tokio::test]
async fn a_single_genre_station_uses_random_mix_through_the_allow_list() {
    let h = harness().await;
    let id = station(&h, &h.tok_a, json!({"kind": "genre", "genres": [3]})).await;
    let (s, v) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_a),
            Some(json!({"player": h.a(), "start": true})),
        )
        .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["mode"], "randomplay");
    assert_eq!(v["refill"], false);
    assert_eq!(v["command"], json!(["randomplay", "tracks", "genre_id:3"]));
    // The client sends it through /v1/rpc, where the fixed grammar allows exactly that.
    assert_eq!(h.rpc(&h.tok_a, &h.a(), v["command"].clone()).await, S::OK);
    assert_eq!(h.sent(&h.a(), "randomplay").len(), 1);
    // Not on bob's player, and nothing looser.
    assert_eq!(
        h.rpc(&h.tok_a, &h.b(), v["command"].clone()).await,
        S::FORBIDDEN
    );
    for bad in [
        json!(["randomplay", ".*"]),
        json!(["randomplay", "tracks", "genres:Jazz"]),
        json!(["randomplay", "tracks", "genre_id:0"]),
        json!(["randomplaychoosegenre", "Jazz", 0]),
        json!(["randomplaygenreselectall", 0]),
    ] {
        assert_eq!(
            h.rpc(&h.tok_a, &h.a(), bad.clone()).await,
            S::FORBIDDEN,
            "{bad}"
        );
    }
    assert_eq!(h.sent(&h.a(), "randomplay").len(), 1);
    assert!(h.sent(&h.a(), "randomplaychoosegenre").is_empty());
}

#[tokio::test]
async fn built_in_batches_are_track_ids_avoiding_recent_ones() {
    let h = harness().await;
    let (_, v) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    let fresh = v["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "fresh")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let next = |body: Value| {
        let h = &h;
        async move {
            h.call(
                "POST",
                &format!("/v1/me/stations/{fresh}/next"),
                Some(&h.tok_a),
                Some(body),
            )
            .await
        }
    };
    // The fake LMS's newest album lists track 1, never played: fresh.
    let (s, v) = next(json!({"player": h.a(), "start": true, "count": 10})).await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["mode"], "tracks");
    assert_eq!(v["refill"], true);
    assert_eq!(v["tracks"], json!([1]));
    // Served lately: not again straight away.
    let (_, v) = next(json!({"player": h.a()})).await;
    assert_eq!(v["tracks"], json!([]));
    // Bad bodies.
    for bad in [
        json!({}),
        json!({"player": h.b()}),
        json!({"player": "aa:bb:cc:dd:ee:01"}),
        json!({"player": "not-a-mac"}),
        json!({"player": h.a(), "count": 0}),
        json!({"player": h.a(), "count": 51}),
        json!({"player": h.a(), "count": "5"}),
        json!({"player": h.a(), "start": "yes"}),
        json!({"player": h.a(), "exclude": "1"}),
        json!({"player": h.a(), "exclude": [0]}),
        json!({"player": h.a(), "exclude": ["1"]}),
        json!({"player": h.a(), "exclude": vec![1; 201]}),
        json!({"player": h.a(), "extra": 1}),
    ] {
        let (s, _) = next(bad.clone()).await;
        assert!(s == S::BAD_REQUEST || s == S::FORBIDDEN, "{bad}: {s}");
    }
    // A user's own recent plays are left out too.
    let id = station(&h, &h.tok_b, json!({"kind": "fresh"})).await;
    send_plays(&h, &h.tok_b, json!([play("bob-fresh-001", 3600)])).await;
    let (s, v) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_b),
            Some(json!({"player": h.b()})),
        )
        .await;
    assert_eq!(s, S::OK);
    assert_eq!(v["tracks"], json!([]), "bob just played track 1");
}

// ---------------------------------------------------------------------------
// Stations pinned from menus: references to offered actions, re-checked every time
// ---------------------------------------------------------------------------

/// The fake track-info menu for track 9 offers AudioMuse-AI's "Similar tracks".
fn similar_station() -> Value {
    json!({"kind": "menu",
           "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]],
           "play": ["audiomuse", "similar_track", "track_id:9"]})
}

#[tokio::test]
async fn a_pinned_menu_station_runs_only_what_lms_offers() {
    let h = harness().await;
    let id = station(&h, &h.tok_a, similar_station()).await;
    let (s, v) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_a),
            Some(json!({"player": h.a(), "start": true})),
        )
        .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["mode"], "ran");
    assert_eq!(v["refill"], false, "the item offers no add");
    assert_eq!(
        h.sent(&h.a(), "audiomuse"),
        vec![json!(["audiomuse", "similar_track", "track_id:9"])]
    );
    // The menu was read again first, on alice's player.
    assert!(!h.sent(&h.a(), "trackinfo").is_empty());
    // Without `start` there is nothing to add: nothing runs.
    let (_, v) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_a),
            Some(json!({"player": h.a()})),
        )
        .await;
    assert_eq!(v["mode"], "none");
    assert_eq!(h.sent(&h.a(), "audiomuse").len(), 1);
}

#[tokio::test]
async fn pinning_refuses_anything_lms_did_not_offer() {
    let h = harness().await;
    let a = h.a();
    let try_save = |spec: Value, player: String| {
        let h = &h;
        async move {
            h.call(
                "POST",
                "/v1/me/stations",
                Some(&h.tok_a),
                Some(json!({"name": "x", "spec": spec, "player": player})),
            )
            .await
        }
    };
    let cases = [
        // Not offered: another track, extra or missing parameters.
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["audiomuse", "similar_track", "track_id:10"]}),
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["audiomuse", "similar_track", "track_id:9", "x:1"]}),
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["audiomuse", "similar_track"]}),
        // Offered in the menu but never runnable: server settings.
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["pref", "audiodir", "value:x"]}),
        // Dropped from the menu (it carried a URL): never learned.
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["jivefavorites", "add", "title:T"]}),
        // Control commands are never menu actions.
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["playlist", "clear"]}),
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]], "play": ["client", "forget"]}),
        // A root that isn't a menu of the fixed allow-list.
        json!({"kind": "menu", "steps": [["status", 0, 100]], "play": ["audiomuse", "similar_track", "track_id:9"]}),
        json!({"kind": "menu", "steps": [["someplugin", "items", 0, 100]], "play": ["audiomuse", "similar_track", "track_id:9"]}),
        json!({"kind": "menu", "steps": [["readdirectory", 0, 100, "folder:x"]], "play": ["audiomuse", "similar_track", "track_id:9"]}),
        json!({"kind": "menu", "steps": [["anywhere", "items", 0, 100]], "play": ["audiomuse", "similar_track", "track_id:9"]}),
        // A second step LMS never offered.
        json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"], ["remote", "items", 0, 10, "item_id:1"]], "play": ["audiomuse", "similar_track", "track_id:9"]}),
    ];
    for spec in cases {
        let (s, v) = try_save(spec.clone(), a.clone()).await;
        assert!(
            s == S::UNPROCESSABLE_ENTITY || s == S::BAD_REQUEST,
            "{spec}: {s} {v}"
        );
    }
    // Another user's player: refused before LMS is asked.
    let (s, _) = try_save(similar_station(), h.b()).await;
    assert_eq!(s, S::FORBIDDEN);
    let (s, _) = try_save(similar_station(), "aa:bb:cc:dd:ee:01".into()).await;
    assert_eq!(s, S::FORBIDDEN, "a home player alice wasn't granted");
    // Nothing was run, and nothing was saved.
    assert!(h.sent(&a, "audiomuse").is_empty());
    assert!(h.sent(&a, "pref").is_empty());
    let (_, v) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    assert!(
        v["stations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "menu")
    );
}

#[tokio::test]
async fn a_pinned_station_whose_action_is_withdrawn_is_marked_broken_and_never_runs() {
    let h = harness().await;
    let id = station(&h, &h.tok_a, similar_station()).await;
    // The admin makes AudioMuse-AI admin-only: listeners are no longer offered it.
    h.accounts
        .set_admin_only_plugins(&["audiomuse".to_string()])
        .unwrap();
    let (s, v) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_a),
            Some(json!({"player": h.a(), "start": true})),
        )
        .await;
    assert_eq!(s, S::CONFLICT, "{v}");
    assert_eq!(v["broken"], true);
    assert!(h.sent(&h.a(), "audiomuse").is_empty(), "it never ran");
    let (_, v) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    let st = v["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()
        .clone();
    assert!(
        st["broken"].as_str().unwrap().contains("admins only"),
        "{st}"
    );
    // Offered again: it works, and is no longer broken.
    h.accounts.set_admin_only_plugins(&[]).unwrap();
    let (s, _) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_a),
            Some(json!({"player": h.a(), "start": true})),
        )
        .await;
    assert_eq!(s, S::OK);
    let (_, v) = h.call("GET", "/v1/me/stations", Some(&h.tok_a), None).await;
    let st = v["stations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()
        .clone();
    assert!(st["broken"].is_null());
    // Running it on a player alice may not control: refused, nothing sent.
    let before = h.sent(&h.b(), "audiomuse").len();
    let (s, _) = h
        .call(
            "POST",
            &format!("/v1/me/stations/{id}/next"),
            Some(&h.tok_a),
            Some(json!({"player": h.b(), "start": true})),
        )
        .await;
    assert_eq!(s, S::FORBIDDEN);
    assert_eq!(h.sent(&h.b(), "audiomuse").len(), before);
}

// ---------------------------------------------------------------------------
// Pinned menus
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replaying_unknown_menus_never_hammers_lms() {
    // Regression: each replay of a menu whose app LMS doesn't list re-read LMS's app
    // lists, unthrottled (as /v1/rpc is, to once a minute).
    let h = harness().await;
    let a = h.a();
    for _ in 0..5 {
        let (s, _) = h
            .call(
                "POST",
                "/v1/me/menus",
                Some(&h.tok_a),
                Some(
                    json!({"name": "Gone", "steps": [["goneplugin", "items", 0, 50]], "player": a}),
                ),
            )
            .await;
        assert_eq!(s, S::UNPROCESSABLE_ENTITY);
    }
    let refreshes = h
        .lms
        .rpc_calls()
        .iter()
        .filter(|c| c[1][0] == "apps")
        .count();
    assert!(refreshes <= 1, "{refreshes} app-list refreshes");
}

#[tokio::test]
async fn pinned_menus_reopen_what_lms_offers_now() {
    let h = harness().await;
    let a = h.a();
    // Before the menu was shown, its actions can't be run.
    assert_eq!(
        h.rpc(&h.tok_a, &a, json!(["someplugin", "menu", 0, 50]))
            .await,
        S::FORBIDDEN
    );
    let (s, v) = h
        .call(
            "POST",
            "/v1/me/menus",
            Some(&h.tok_a),
            Some(json!({"name": "My Music", "steps": [["menu", 0, 100, "direct:1"]], "player": a})),
        )
        .await;
    assert_eq!(s, S::CREATED, "{v}");
    let id = v["id"].as_i64().unwrap();
    let (s, v) = h
        .call(
            "POST",
            &format!("/v1/me/menus/{id}/open"),
            Some(&h.tok_a),
            Some(json!({"player": a})),
        )
        .await;
    assert_eq!(s, S::OK, "{v}");
    let text = v.to_string();
    assert!(text.contains("Some Plugin"));
    assert!(
        !text.contains("crossfadesettings") && !text.contains("\"power\""),
        "settings are filtered: {text}"
    );
    // Now the device may run what it was shown.
    assert_eq!(
        h.rpc(&h.tok_a, &a, json!(["someplugin", "menu", 0, 50]))
            .await,
        S::OK
    );
    // Bob can't open alice's pinned menu, nor list it.
    let (s, _) = h
        .call(
            "POST",
            &format!("/v1/me/menus/{id}/open"),
            Some(&h.tok_b),
            Some(json!({"player": h.b()})),
        )
        .await;
    assert_eq!(s, S::NOT_FOUND);
    let (_, v) = h.call("GET", "/v1/me/menus", Some(&h.tok_b), None).await;
    assert_eq!(v["menus"], json!([]));
    // Hostile pins are refused.
    for bad in [
        json!({"name": "x", "steps": [["pref", 0, 100]], "player": a}),
        json!({"name": "x", "steps": [["menu"]], "player": a}),
        json!({"name": "x", "steps": [["menu", 0, 100]], "player": h.b()}),
        json!({"name": "x", "steps": [["menu", 0, 100]]}),
        json!({"name": "x", "steps": [["menu", 0, 100], ["settings", "items", 0, 10]], "player": a}),
        json!({"name": "x", "steps": [["menu", 0, 100]], "player": a, "play": ["x"]}),
    ] {
        let (s, _) = h
            .call("POST", "/v1/me/menus", Some(&h.tok_a), Some(bad.clone()))
            .await;
        assert!(
            matches!(s, S::BAD_REQUEST | S::FORBIDDEN | S::UNPROCESSABLE_ENTITY),
            "{bad}: {s}"
        );
    }
    for bad in [
        json!({}),
        json!({"player": a, "x": 1}),
        json!({"player": "nope"}),
    ] {
        let (s, _) = h
            .call(
                "POST",
                &format!("/v1/me/menus/{id}/open"),
                Some(&h.tok_a),
                Some(bad.clone()),
            )
            .await;
        assert!(matches!(s, S::BAD_REQUEST | S::FORBIDDEN), "{bad}: {s}");
    }
    let (s, _) = h
        .call(
            "DELETE",
            &format!("/v1/me/menus/{id}"),
            Some(&h.tok_b),
            None,
        )
        .await;
    assert_eq!(s, S::NOT_FOUND);
    let (s, _) = h
        .call(
            "DELETE",
            &format!("/v1/me/menus/{id}"),
            Some(&h.tok_a),
            None,
        )
        .await;
    assert_eq!(s, S::OK);
    let (s, _) = h
        .call(
            "POST",
            &format!("/v1/me/menus/{id}/open"),
            Some(&h.tok_a),
            Some(json!({"player": a})),
        )
        .await;
    assert_eq!(s, S::NOT_FOUND);
}

fn stat_at(event: &[u8; 4], elapsed_ms: u32, jiffies: u32) -> Vec<u8> {
    ClientMsg::Stat(Stat {
        event: *event,
        jiffies,
        elapsed_seconds: elapsed_ms / 1000,
        elapsed_milliseconds: elapsed_ms,
        ..Stat::default()
    })
    .encode()
}

/// Regression (rolling cache, A4): a phone that lost its connection queues its reports and
/// sends them together on resume. They are judged by the player's own clock, so the
/// listening they describe counts, though they all arrive at once.
#[tokio::test]
async fn reports_sent_together_after_a_gap_count_by_the_players_clock() {
    let h = harness_with(live_harness()).await;
    let alice = h.accounts.user_id_by_name("alice").unwrap().unwrap();
    // A 20 s track: it counts after 10 s heard.
    h.lms.add_track(43, "flc", b"audio".to_vec(), 20.0);
    *h.lms.status_track.lock().unwrap() = Some(43);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat_at(b"STMs", 0, 50_000).into()))
        .await
        .unwrap();
    // The "gap": 2.5 s of wall time, then three reports made 4 s apart, sent at once.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let mut burst = Vec::new();
    for i in 1..=3u32 {
        burst.extend(stat_at(b"STMt", i * 4000, 50_000 + i * 4000));
    }
    ws.send(Message::Binary(burst.into())).await.unwrap();
    wait_for(|| anywhere_gateway::history::count(&h.accounts, alice).unwrap() == 1).await;
    let _ = ws.close(None).await;
}

/// Security: a player whose clock races ahead with its elapsed time (claiming minutes in
/// a fraction of a second) is held to the wall time since its first report.
#[tokio::test]
async fn a_racing_player_clock_cannot_inflate_listening() {
    let h = harness_with(live_harness()).await;
    let alice = h.accounts.user_id_by_name("alice").unwrap().unwrap();
    // A 60 s track: it would count after 30 s.
    h.lms.add_track(44, "flc", b"audio".to_vec(), 60.0);
    *h.lms.status_track.lock().unwrap() = Some(44);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat_at(b"STMs", 0, 0).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mut burst = Vec::new();
    for i in 1..=60u32 {
        burst.extend(stat_at(b"STMt", i * 1000, i * 1000));
    }
    ws.send(Message::Binary(burst.into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = ws.close(None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        anywhere_gateway::history::count(&h.accounts, alice).unwrap(),
        0
    );
}

/// Regression (M2): a pinned menu replays typed search text, so a URL in it whose name
/// resolves to the server itself must not reach LMS on replay either.
#[tokio::test]
async fn pinned_menus_never_replay_urls_that_resolve_locally() {
    let h = harness_with(|c| {
        c.host_answers.insert(
            "loop.example.com".into(),
            vec!["127.0.0.1".parse().unwrap()],
        );
    })
    .await;
    let a = h.a();
    let steps = json!([
        ["menu", 0, 100, "direct:1"],
        ["someplugin", "menu", 0, 50],
        [
            "someplugin",
            "items",
            0,
            20,
            "search:http://loop.example.com/",
            "menu:someplugin"
        ]
    ]);
    let (s, v) = h
        .call(
            "POST",
            "/v1/me/menus",
            Some(&h.tok_a),
            Some(json!({"name": "Feed", "steps": steps, "player": a})),
        )
        .await;
    if s == S::CREATED {
        let id = v["id"].as_i64().unwrap();
        let (s, v) = h
            .call(
                "POST",
                &format!("/v1/me/menus/{id}/open"),
                Some(&h.tok_a),
                Some(json!({"player": a})),
            )
            .await;
        assert_ne!(s, S::OK, "{v}");
    }
    assert!(
        !h.lms
            .rpc_calls()
            .iter()
            .any(|c| c.to_string().contains("loop.example.com")),
        "reached LMS: {:?}",
        h.lms.rpc_calls()
    );
}

/// Regression (M3): every `STMs` (track started) spawned an LMS `status` lookup, so a client
/// looping them made the gateway query LMS once per frame. Starts that come faster than the
/// lookup delay are coalesced: only the latest is looked up.
#[tokio::test]
async fn a_storm_of_track_starts_makes_a_bounded_number_of_lms_lookups() {
    let h = harness_with(|c| c.history_lookup_delay = Duration::from_millis(300)).await;
    h.lms.add_track(45, "flc", b"audio".to_vec(), 2.0);
    *h.lms.status_track.lock().unwrap() = Some(45);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    // 200 track starts in 2 s.
    for _ in 0..20 {
        let mut burst = Vec::new();
        for _ in 0..10 {
            burst.extend(stat(b"STMs", 0));
        }
        ws.send(Message::Binary(burst.into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let n = h.sent(&h.a(), "status").len();
    assert!(n <= 3, "{n} LMS lookups for 200 track starts");
    assert!(n >= 1, "the last start is still looked up");
    let _ = ws.close(None).await;
}

/// Regression (M3): starts spaced just past the lookup delay each get their lookup, but no
/// more than the per-session cap (30 a minute) reach LMS.
#[tokio::test]
async fn track_start_lookups_are_capped_per_session() {
    let h = harness_with(live_harness()).await;
    h.lms.add_track(46, "flc", b"audio".to_vec(), 2.0);
    *h.lms.status_track.lock().unwrap() = Some(46);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    for _ in 0..45 {
        ws.send(Message::Binary(stat(b"STMs", 0).into()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let n = h.sent(&h.a(), "status").len();
    assert!(n <= 30, "{n} LMS lookups: over the cap");
    assert!(n >= 20, "{n} LMS lookups: spaced starts were coalesced");
    let _ = ws.close(None).await;
}

/// Ordinary track changes, spaced apart, each still get their own lookup (M3).
#[tokio::test]
async fn spaced_track_changes_each_get_their_lookup() {
    let h = harness_with(live_harness()).await;
    h.lms.add_track(47, "flc", b"audio".to_vec(), 2.0);
    *h.lms.status_track.lock().unwrap() = Some(47);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    for _ in 0..3 {
        ws.send(Message::Binary(stat(b"STMs", 0).into()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert_eq!(h.sent(&h.a(), "status").len(), 3);
    let _ = ws.close(None).await;
}
