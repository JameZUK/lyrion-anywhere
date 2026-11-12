//! A blocked LMS (e.g. its music share stalled on storage spinning up): the gateway notices,
//! tells the apps, refuses new `/v1/rpc` requests at once instead of piling them up, notices
//! recovery, and keeps player sockets alive meanwhile.

mod common;
use common::*;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use reqwest::StatusCode as S;
use serde_json::{Value, json};

impl Harness {
    async fn rpc(&self, token: &str, cmd: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(self.url("/v1/rpc"))
            .bearer_auth(token)
            .json(&json!({ "player": "", "cmd": cmd }))
            .send()
            .await
            .unwrap()
    }

    async fn events(&self, token: &str) -> Ws {
        let req = format!("ws://{}/v1/events", self.addr)
            .into_client_request()
            .unwrap();
        let (mut ws, _) = connect_async(req).await.unwrap();
        ws.send(Message::Text(token.to_string().into()))
            .await
            .unwrap();
        match next(&mut ws).await {
            Message::Text(t) => assert!(t.as_str().contains("subscribed")),
            m => panic!("{m:?}"),
        }
        ws
    }

    fn freeze(&self, on: bool) {
        self.lms.freeze_rpc.store(on, Ordering::SeqCst);
    }
}

/// The next JSON text message, skipping pings.
async fn text_json(ws: &mut Ws) -> Value {
    loop {
        match next(ws).await {
            Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            m => panic!("{m:?}"),
        }
    }
}

async fn busy_harness() -> Harness {
    harness_with(|c| c.lms_stall_after = Duration::from_millis(200)).await
}

#[tokio::test]
async fn a_blocked_lms_is_reported_requests_are_refused_at_once_and_recovery_is_noticed() {
    let h = busy_harness().await;
    let mut events = h.events(&h.tok_a).await;
    assert_eq!(
        h.rpc(&h.tok_a, json!(["serverstatus", 0, 0]))
            .await
            .status(),
        S::OK
    );

    h.freeze(true);
    let stuck = {
        let (url, tok) = (h.url("/v1/rpc"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .json(&json!({ "player": "", "cmd": ["albums", 0, 5] }))
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    let busy = text_json(&mut events).await;
    assert_eq!(busy["lms"], "busy");
    assert!(
        busy["since"].as_u64().unwrap() > 1_600_000_000_000,
        "{busy}"
    );

    // New requests are refused at once, without reaching LMS.
    let sent_before = h.lms.rpc_calls().len();
    let t = Instant::now();
    let r = h.rpc(&h.tok_a, json!(["albums", 0, 5])).await;
    assert!(t.elapsed() < Duration::from_secs(1), "refused at once");
    assert_eq!(r.status(), S::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        body,
        json!({ "error": "music server is busy", "busy": true })
    );

    // A client subscribing now hears at once that LMS is busy.
    let mut late = h.events(&h.tok_b).await;
    assert_eq!(text_json(&mut late).await["lms"], "busy");

    // LMS answers again: everyone is told, the waiting request completes, and new ones go
    // through.
    h.freeze(false);
    assert_eq!(text_json(&mut events).await, json!({ "lms": "ok" }));
    assert_eq!(text_json(&mut late).await, json!({ "lms": "ok" }));
    assert_eq!(stuck.await.unwrap(), S::OK);
    assert_eq!(
        h.rpc(&h.tok_a, json!(["albums", 0, 5])).await.status(),
        S::OK
    );
    // Only the stuck request, the watchdog's probe and the last request reached LMS: the
    // refused one never did. (The fake records a call once it answers it, and the probe may
    // still be waking from the freeze when the others are done.)
    let commands = |from: usize| -> Vec<String> {
        h.lms.rpc_calls()[from..]
            .iter()
            .map(|p| p[1][0].as_str().unwrap_or("").to_string())
            .collect()
    };
    wait_for(|| commands(sent_before).iter().any(|c| c == "serverstatus")).await;
    let reached = commands(sent_before);
    assert_eq!(
        reached.iter().filter(|c| *c == "albums").count(),
        2,
        "{reached:?}"
    );
    assert!(reached.iter().any(|c| c == "serverstatus"), "{reached:?}");
}

#[tokio::test]
async fn only_one_probe_waits_on_a_blocked_lms() {
    let h = busy_harness().await;
    h.freeze(true);
    let _stuck = {
        let (url, tok) = (h.url("/v1/rpc"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .json(&json!({ "player": "", "cmd": ["albums", 0, 5] }))
                .send()
                .await
        })
    };
    wait_for(|| h.gw.lms_health().is_busy()).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let waiting = h
        .lms
        .http_paths()
        .iter()
        .filter(|p| *p == "/jsonrpc.js")
        .count();
    // The stuck request and a single probe, however long the stall.
    assert!(waiting <= 2 + 1, "requests piling up on LMS: {waiting}");
    h.freeze(false);
    wait_for(|| !h.gw.lms_health().is_busy()).await;
}

#[tokio::test]
async fn refusals_while_busy_still_check_who_is_asking() {
    let h = busy_harness().await;
    h.freeze(true);
    let _stuck = {
        let (url, tok) = (h.url("/v1/rpc"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .json(&json!({ "player": "", "cmd": ["albums", 0, 5] }))
                .send()
                .await
        })
    };
    wait_for(|| h.gw.lms_health().is_busy()).await;
    // No token, a bad token, a malformed body and a forbidden command are refused as ever,
    // and at once: "busy" is never an answer to a request that would have been refused
    // anyway, and refusing an unknown command doesn't wait to re-read LMS's app menus.
    let t = Instant::now();
    let anon = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .json(&json!({ "player": "", "cmd": ["albums", 0, 5] }))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), S::UNAUTHORIZED);
    assert_eq!(
        h.rpc("not-a-token", json!(["albums", 0, 5])).await.status(),
        S::UNAUTHORIZED
    );
    let bad = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .body("{nope")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), S::BAD_REQUEST);
    assert_eq!(
        h.rpc(&h.tok_a, json!(["rescan"])).await.status(),
        S::FORBIDDEN
    );
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    h.freeze(false);
}

/// Artwork honours the busy state as `/v1/rpc` does (review low): refused at once with the
/// same 503, without reaching LMS, and only for a request that would otherwise be served.
#[tokio::test]
async fn artwork_is_refused_at_once_while_lms_is_busy() {
    let h = busy_harness().await;
    let art = |token: Option<String>, path: &'static str| {
        let url = h.url(path);
        async move {
            let mut req = reqwest::Client::new().get(url);
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            req.send().await.unwrap()
        }
    };
    let cover = "/v1/art/music/1/cover_300x300.jpg";
    assert_eq!(art(Some(h.tok_a.clone()), cover).await.status(), S::OK);
    h.freeze(true);
    let _stuck = {
        let (url, tok) = (h.url("/v1/rpc"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .json(&json!({ "player": "", "cmd": ["albums", 0, 5] }))
                .send()
                .await
        })
    };
    wait_for(|| h.gw.lms_health().is_busy()).await;
    let fetched = |h: &Harness| {
        h.lms
            .http_paths()
            .iter()
            .filter(|p| p.starts_with("/music/"))
            .count()
    };
    let before = fetched(&h);
    let t = Instant::now();
    let r = art(Some(h.tok_a.clone()), cover).await;
    assert!(t.elapsed() < Duration::from_secs(1), "refused at once");
    assert_eq!(r.status(), S::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        body,
        json!({ "error": "music server is busy", "busy": true })
    );
    // Refusals that don't depend on LMS are unchanged.
    assert_eq!(art(None, cover).await.status(), S::UNAUTHORIZED);
    assert_eq!(
        art(Some(h.tok_a.clone()), "/v1/art/etc/passwd")
            .await
            .status(),
        S::NOT_FOUND
    );
    assert_eq!(fetched(&h), before, "LMS was never asked");
    h.freeze(false);
    wait_for(|| !h.gw.lms_health().is_busy()).await;
    assert_eq!(art(Some(h.tok_a.clone()), cover).await.status(), S::OK);
}

/// Regression (review low): queue edits, queue saves and alarms never looked at the busy
/// state, so while LMS was blocked each request waited on it (a save up to four times 30 s,
/// holding the one save lock meanwhile). They answer 503 busy at once, with Retry-After,
/// without asking LMS, and only once every other check has passed.
#[tokio::test]
async fn queue_edits_saves_and_alarms_are_refused_at_once_while_lms_is_busy() {
    let h = busy_harness().await;
    h.accounts.create_user("root", PW, Role::Admin).unwrap();
    let admin = h
        .accounts
        .login("root", PW, None, "admin-device", None, "test")
        .unwrap();
    // (An admin may control every player, alice's included.)
    let mine = format_mac(&h.mac_a);
    let theirs = format_mac(&h.mac_b);
    h.freeze(true);
    let _stuck = {
        let (url, tok) = (h.url("/v1/rpc"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .json(&json!({ "player": "", "cmd": ["albums", 0, 5] }))
                .send()
                .await
        })
    };
    wait_for(|| h.gw.lms_health().is_busy()).await;
    let asked = h.lms.rpc_calls().len();
    let call = |method: reqwest::Method, path: String, token: String, body: Option<Value>| {
        let url = h.url(&path);
        async move {
            let mut r = reqwest::Client::new()
                .request(method, url)
                .bearer_auth(token);
            if let Some(b) = body {
                r = r.json(&b);
            }
            r.send().await.unwrap()
        }
    };
    use reqwest::Method as M;
    let t = Instant::now();
    for (method, path, token, body) in [
        (
            M::POST,
            format!("/v1/players/{mine}/queue/edit"),
            h.tok_a.clone(),
            Some(json!({"op": "remove", "index": 1, "track": 3})),
        ),
        (
            M::POST,
            format!("/v1/players/{mine}/queue/save"),
            admin.access.clone(),
            Some(json!({"name": "Road trip"})),
        ),
        (
            M::GET,
            format!("/v1/players/{mine}/alarms"),
            h.tok_a.clone(),
            None,
        ),
        (
            M::PUT,
            format!("/v1/players/{mine}/alarms/enabled"),
            h.tok_a.clone(),
            Some(json!({"enabled": false})),
        ),
        (
            M::DELETE,
            format!("/v1/players/{mine}/alarms/abcdef01"),
            h.tok_a.clone(),
            None,
        ),
    ] {
        let r = call(method, path.clone(), token, body).await;
        assert_eq!(r.status(), S::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(r.headers()["retry-after"], "2", "{path}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v, json!({ "error": "music server is busy", "busy": true }));
    }
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(h.lms.rpc_calls().len(), asked, "LMS was never asked");
    // Refusals that don't depend on LMS are given as ever.
    let r = call(
        M::POST,
        format!("/v1/players/{theirs}/queue/edit"),
        h.tok_a.clone(),
        Some(json!({"op": "remove", "index": 1, "track": 3})),
    )
    .await;
    assert_eq!(r.status(), S::FORBIDDEN);
    let r = call(
        M::POST,
        format!("/v1/players/{mine}/queue/edit"),
        h.tok_a.clone(),
        Some(json!({"op": "clear"})),
    )
    .await;
    assert_eq!(r.status(), S::BAD_REQUEST);
    let r = call(
        M::POST,
        format!("/v1/players/{mine}/queue/save"),
        h.tok_a.clone(),
        Some(json!({"name": "Road trip"})),
    )
    .await;
    assert_eq!(r.status(), S::FORBIDDEN, "listeners may not save");
    h.freeze(false);
    wait_for(|| !h.gw.lms_health().is_busy()).await;
    let r = call(
        M::GET,
        format!("/v1/players/{mine}/alarms"),
        h.tok_a.clone(),
        None,
    )
    .await;
    assert_eq!(r.status(), S::OK);
}

/// Regression (review low): a save held the one save lock across up to four LMS requests of
/// 30 s each, and every other save waited behind it for as long. Another save now waits a
/// few seconds at most, then gets 503 busy (with Retry-After), and never reaches LMS.
/// (Here rather than with the other save tests: the save lock is the process's, and this
/// test holds it for seconds.)
#[tokio::test]
async fn a_save_stuck_on_a_slow_lms_holds_others_up_only_briefly() {
    // LMS slow on one request but not judged blocked: only the save lock is in the way.
    let h = harness_with(|c| c.lms_stall_after = Duration::from_secs(60)).await;
    h.accounts.create_user("carol", PW, Role::Admin).unwrap();
    let admin = h
        .accounts
        .login("carol", PW, None, "Laptop", None, "test")
        .unwrap()
        .access;
    h.lms.set_saved_playlists(FakeSaved {
        playlists: vec![],
        queue_len: 3,
    });
    h.lms.hang_rpc.lock().unwrap().insert("status".into());
    let player = format_mac(&h.mac_a);
    let save = |name: &'static str| {
        let (url, tok) = (
            h.url(&format!("/v1/players/{player}/queue/save")),
            admin.clone(),
        );
        async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(tok)
                .json(&json!({ "name": name }))
                .send()
                .await
                .unwrap()
        }
    };
    let stuck = tokio::spawn(save("First"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let asked = h.lms.rpc_calls().len();
    let t = Instant::now();
    let r = save("Second").await;
    assert_eq!(r.status(), S::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    assert!(
        t.elapsed() < Duration::from_secs(8),
        "waited {:?}",
        t.elapsed()
    );
    assert_eq!(
        h.lms.rpc_calls().len(),
        asked,
        "the second never reached LMS"
    );
    stuck.abort();
}

#[tokio::test]
async fn me_advertises_lms_busy_reporting() {
    let h = harness().await;
    let me: Value = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["features"]["lms_busy"], true);
}

// --- player keepalives -------------------------------------------------------

async fn sign_in(h: &Harness, auth: Value) -> Ws {
    let mut ws = h.dial().await;
    ws.send(Message::Text(auth.to_string().into()))
        .await
        .unwrap();
    match next(&mut ws).await {
        Message::Text(t) => {
            let g: Value = serde_json::from_str(t.as_str()).unwrap();
            assert!(g["session"].is_string(), "greeting: {g}");
        }
        m => panic!("{m:?}"),
    }
    ws
}

/// Everything the socket receives in `d` (tungstenite answers pings itself).
async fn collect(ws: &mut Ws, d: Duration) -> Vec<Message> {
    let mut out = vec![];
    let end = tokio::time::Instant::now() + d;
    while let Ok(Some(Ok(m))) = tokio::time::timeout_at(end, ws.next()).await {
        out.push(m);
    }
    out
}

fn keepalive_harness(
    mutate: impl FnOnce(&mut anywhere_gateway::Config),
) -> impl Future<Output = Harness> {
    harness_with(|c| {
        c.player_keepalive = Duration::from_millis(100);
        mutate(c);
    })
}

#[tokio::test]
async fn player_sockets_get_pings_and_text_keepalives_only_when_asked() {
    let mut h = keepalive_harness(|_| {}).await;
    let mut asked = sign_in(
        &h,
        json!({ "token": h.tok_a, "protocol": 2, "keepalive": true }),
    )
    .await;
    let _lms_a = h.lms.accept().await;
    let got = collect(&mut asked, Duration::from_millis(550)).await;
    let pings = got.iter().filter(|m| matches!(m, Message::Ping(_))).count();
    let texts: Vec<&Message> = got
        .iter()
        .filter(|m| matches!(m, Message::Text(_)))
        .collect();
    assert!(pings >= 3, "{got:?}");
    assert!(texts.len() >= 3, "{got:?}");
    for t in &texts {
        let Message::Text(t) = t else { unreachable!() };
        assert_eq!(t.as_str(), r#"{"keepalive":true}"#);
    }
    // Never a binary message: frame numbering for resume is untouched.
    assert!(
        !got.iter().any(|m| matches!(m, Message::Binary(_))),
        "{got:?}"
    );

    // Not asked (absent, false, or not a boolean): pings only.
    for auth in [
        json!({ "token": h.tok_b, "protocol": 2 }),
        json!({ "token": h.tok_b, "protocol": 2, "keepalive": false }),
        json!({ "token": h.tok_b, "protocol": 2, "keepalive": "yes" }),
    ] {
        let mut ws = sign_in(&h, auth).await;
        let got = collect(&mut ws, Duration::from_millis(350)).await;
        assert!(got.iter().any(|m| matches!(m, Message::Ping(_))), "{got:?}");
        assert!(
            !got.iter().any(|m| matches!(m, Message::Text(_))),
            "{got:?}"
        );
    }
}

/// A pong answers the gateway's own ping (browsers send it even while their scripts are
/// suspended), so it must not count as the player speaking: the gateway keeps reporting to
/// LMS for a client that is otherwise silent.
#[tokio::test]
async fn pongs_do_not_count_as_the_player_speaking() {
    let mut h = keepalive_harness(|c| c.client_silence = Duration::from_millis(400)).await;
    let mut ws = sign_in(&h, json!({ "token": h.tok_a, "protocol": 2 })).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    let stat = anywhere_core::proto::ClientMsg::Stat(anywhere_core::proto::Stat {
        event: *b"STMs",
        jiffies: 1_000,
        ..Default::default()
    })
    .encode();
    ws.send(Message::Binary(stat.into())).await.unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "STAT"]).await;
    // Reading answers each ping with a pong; the client itself says nothing.
    let got = collect(&mut ws, Duration::from_millis(2_300)).await;
    assert!(got.iter().any(|m| matches!(m, Message::Ping(_))));
    assert!(
        h.lms.stats().len() >= 2,
        "expected the gateway to report for a client that only ponged"
    );
}
