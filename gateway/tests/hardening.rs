//! Regression tests for the sign-in and session hardening: client addresses behind
//! appending proxies, lockouts that can't be used against someone else, concurrent
//! refreshes, device and player clean-up, and response headers.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

const CLIENT: (&str, &str) = ("x-anywhere-client", "1");

async fn post(h: &Harness, path: &str, body: Value, extra: &[(&str, &str)]) -> reqwest::Response {
    let mut req = reqwest::Client::new().post(h.url(path)).json(&body);
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

async fn login_via(h: &Harness, user: &str, password: &str, xff: &str) -> reqwest::Response {
    post(
        h,
        "/v1/auth/login",
        json!({ "username": user, "password": password, "device_name": "Test browser" }),
        &[
            CLIENT,
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", xff),
        ],
    )
    .await
}

fn cookie_of(resp: &reqwest::Response) -> String {
    let set = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    set.split(';').next().unwrap().to_string()
}

async fn refresh_with_cookie(h: &Harness, cookie: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(h.url("/v1/auth/refresh"))
        .header(CLIENT.0, CLIENT.1)
        .header("cookie", cookie)
        .send()
        .await
        .unwrap()
}

/// `client forget` calls LMS received, by player.
fn forgotten(h: &Harness) -> Vec<String> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|p| p[1] == json!(["client", "forget"]))
        .map(|p| p[0].as_str().unwrap_or("").to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// M2: the client address behind an appending proxy
// ---------------------------------------------------------------------------

/// Regression: the leftmost `X-Forwarded-For` entry was the lockout key. Behind a proxy
/// that appends (Cloudflare Tunnel, nginx `$proxy_add_x_forwarded_for`), the client writes
/// that entry, so it could pick a fresh key for every guess.
#[tokio::test]
async fn an_appended_forwarded_for_cannot_dodge_the_lockout() {
    let h = harness().await;
    for i in 0..5 {
        let spoofed = format!("192.0.2.{i}, 198.51.100.7");
        login_via(&h, "nobody", "wrong password!", &spoofed).await;
    }
    let r = login_via(&h, "alice", PW, "192.0.2.99, 198.51.100.7").await;
    assert_eq!(
        r.status(),
        S::TOO_MANY_REQUESTS,
        "the real address is locked"
    );
    // A different real client behind the same proxy is unaffected.
    let r = login_via(&h, "alice", PW, "192.0.2.1, 198.51.100.8").await;
    assert_eq!(r.status(), S::OK);
}

// ---------------------------------------------------------------------------
// M3: lockouts can't be turned against someone else
// ---------------------------------------------------------------------------

/// Regression: five wrong passwords for a username from anywhere locked out the right
/// password from everywhere, escalating to a day.
#[tokio::test]
async fn an_attacker_cannot_lock_a_user_out() {
    let h = harness().await;
    for _ in 0..15 {
        login_via(&h, "alice", "wrong password!", "198.51.100.66").await;
    }
    let attacker = login_via(&h, "alice", PW, "198.51.100.66").await;
    assert_eq!(attacker.status(), S::TOO_MANY_REQUESTS);
    let alice = login_via(&h, "alice", PW, "203.0.113.20").await;
    assert_eq!(alice.status(), S::OK, "the owner signs in from elsewhere");
}

#[tokio::test]
async fn guesses_from_many_addresses_share_one_small_budget() {
    let h = harness().await;
    let before = h.accounts.password_checks();
    let mut statuses = vec![];
    for i in 0..60 {
        let ip = format!("198.51.100.{}", i + 1);
        statuses.push(
            login_via(&h, "alice", "wrong password!", &ip)
                .await
                .status(),
        );
    }
    let checked = h.accounts.password_checks() - before;
    let budget = 5 + u64::from(anywhere_gateway::accounts::THROTTLED_ATTEMPTS);
    assert_eq!(checked, budget, "{statuses:?}");
    assert_eq!(
        statuses.iter().filter(|s| **s == S::UNAUTHORIZED).count() as u64,
        budget
    );
    assert!(
        statuses[budget as usize..]
            .iter()
            .all(|s| *s == S::TOO_MANY_REQUESTS)
    );
}

// ---------------------------------------------------------------------------
// L4: concurrent refreshes
// ---------------------------------------------------------------------------

/// Regression: two tabs restoring at once both presented the same cookie. The second
/// refresh discarded the first one's replacement, so if the browser kept that cookie, its
/// next refresh looked like theft and revoked the device.
#[tokio::test]
async fn two_tabs_refreshing_at_once_keep_the_device() {
    let mut h = harness().await;
    let resp = post(
        &h,
        "/v1/auth/login",
        json!({ "username": "alice", "password": PW, "device_name": "Laptop" }),
        &[CLIENT],
    )
    .await;
    let a = cookie_of(&resp);
    let token = resp.json::<Value>().await.unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = h.dial().await;
    connect(&mut ws, &token).await;
    let _lms = h.lms.accept().await;

    let first = refresh_with_cookie(&h, &a).await;
    let second = refresh_with_cookie(&h, &a).await;
    assert_eq!(first.status(), S::OK);
    assert_eq!(second.status(), S::OK);
    // The responses arrived in the other order: the browser kept the first replacement.
    let kept = cookie_of(&first);
    let next = refresh_with_cookie(&h, &kept).await;
    assert_eq!(next.status(), S::OK, "the device must survive");
    assert_eq!(
        refresh_with_cookie(&h, &cookie_of(&next)).await.status(),
        S::OK
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), ws.next())
            .await
            .is_err(),
        "the player session stays open"
    );
}

// ---------------------------------------------------------------------------
// L8: devices and LMS players
// ---------------------------------------------------------------------------

/// Regression: every sign-in without a remembered device made a new device and LMS player.
#[tokio::test]
async fn sign_ins_cannot_create_unlimited_devices() {
    let h = harness().await;
    let login = |device_id: Option<i64>| {
        let mut body = json!({ "username": "bob", "password": PW, "device_name": "Phone" });
        if let Some(id) = device_id {
            body["device_id"] = json!(id);
        }
        post(&h, "/v1/auth/login", body, &[CLIENT])
    };
    // The harness signed bob in once already.
    let mut last = None;
    for _ in 1..anywhere_gateway::accounts::NEW_DEVICES_PER_HOUR {
        let r = login(None).await;
        assert_eq!(r.status(), S::OK);
        last = Some(
            r.json::<Value>().await.unwrap()["device"]["id"]
                .as_i64()
                .unwrap(),
        );
    }
    let over = login(None).await;
    assert_eq!(over.status(), S::TOO_MANY_REQUESTS);
    assert!(over.headers().contains_key("retry-after"));
    // A browser signing in again with its remembered device is never limited.
    for _ in 0..3 {
        let r = login(last).await;
        assert_eq!(r.status(), S::OK);
        assert_eq!(
            r.json::<Value>().await.unwrap()["device"]["id"].as_i64(),
            last
        );
    }
}

/// Regression: revoked devices' players stayed in LMS for ever.
#[tokio::test]
async fn signing_out_removes_the_lms_player() {
    let h = harness().await;
    let mac_a = format_mac(&h.mac_a);
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/logout"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert_eq!(r.headers().get("clear-site-data").unwrap(), "\"cache\"");
    wait_for(|| forgotten(&h) == vec![mac_a.clone()]).await;
    // Only that player.
    assert!(!forgotten(&h).contains(&format_mac(&h.mac_b)));
}

#[tokio::test]
async fn every_kind_of_revocation_removes_the_lms_player() {
    let h = harness().await;
    let mac_a = format_mac(&h.mac_a);
    let mac_b = format_mac(&h.mac_b);
    // Admin revocation (as the admin API does).
    let a_dev = h.accounts.authenticate(&h.tok_a).unwrap().device_id;
    h.accounts.revoke_device(a_dev).unwrap();
    h.gw.end_sessions_for(&[a_dev]).await;
    wait_for(|| forgotten(&h).contains(&mac_a)).await;
    // User deletion.
    let devices = h.accounts.delete_user("bob").unwrap();
    h.gw.end_sessions_for(&devices).await;
    wait_for(|| forgotten(&h).contains(&mac_b)).await;
    // Refresh-token reuse.
    let resp = post(
        &h,
        "/v1/auth/login",
        json!({ "username": "alice", "password": PW, "device_name": "Phone", "refresh": "body" }),
        &[CLIENT],
    )
    .await;
    let body = resp.json::<Value>().await.unwrap();
    let t = body["refresh_token"].as_str().unwrap().to_string();
    let mac = body["device"]["mac"].as_str().unwrap().to_string();
    let next = post(
        &h,
        "/v1/auth/refresh",
        json!({ "refresh_token": t }),
        &[CLIENT],
    )
    .await;
    let next = next.json::<Value>().await.unwrap()["refresh_token"]
        .as_str()
        .unwrap()
        .to_string();
    post(
        &h,
        "/v1/auth/refresh",
        json!({ "refresh_token": next }),
        &[CLIENT],
    )
    .await;
    // Replaying the original after its replacement was used: theft.
    let r = post(
        &h,
        "/v1/auth/refresh",
        json!({ "refresh_token": t }),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    wait_for(|| forgotten(&h).contains(&mac)).await;
    assert_eq!(forgotten(&h).len(), 3, "{:?}", forgotten(&h));
}

#[tokio::test]
async fn a_failing_lms_does_not_stop_sign_out() {
    let h = harness_with(|c| c.lms_http = "127.0.0.1:9".into()).await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/logout"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert!(h.accounts.authenticate(&h.tok_a).is_none());
}

// ---------------------------------------------------------------------------
// Headers and origins
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_responses_are_never_stored_and_deny_browser_features() {
    let h = harness().await;
    let c = reqwest::Client::new();
    for (path, token) in [
        ("/v1/me", Some(&h.tok_a)),
        ("/v1/account", Some(&h.tok_a)),
        ("/v1/me", None),
    ] {
        let mut req = c.get(h.url(path));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let r = req.send().await.unwrap();
        assert_eq!(
            r.headers().get("cache-control").unwrap(),
            "no-store",
            "{path}"
        );
        let pp = r
            .headers()
            .get("permissions-policy")
            .unwrap()
            .to_str()
            .unwrap();
        for denied in [
            "camera=()",
            "microphone=()",
            "geolocation=()",
            "payment=()",
            "usb=()",
        ] {
            assert!(pp.contains(denied), "{pp}");
        }
    }
    let r = c
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(&json!({ "player": "", "cmd": ["serverstatus", "0", "1"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers().get("cache-control").unwrap(), "no-store");
    // Web app paths keep their own policy.
    let r = c.get(h.url("/healthz")).send().await.unwrap();
    assert_eq!(r.headers().get("cache-control").unwrap(), "no-cache");
    assert!(r.headers().contains_key("permissions-policy"));
}

/// Regression: the "TLS required" refusal was produced outside the security headers layer.
#[tokio::test]
async fn the_tls_refusal_carries_the_security_headers() {
    let h = harness().await;
    let r = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .header("x-forwarded-proto", "http")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    for name in [
        "content-security-policy",
        "x-frame-options",
        "x-content-type-options",
        "referrer-policy",
        "permissions-policy",
    ] {
        assert!(r.headers().contains_key(name), "{name}");
    }
}

/// Regression: `http://{Host}` was accepted as same-origin even through the TLS proxy.
#[tokio::test]
async fn plain_http_origins_are_refused_through_the_proxy() {
    let h = harness().await;
    for (origin, want) in [
        ("http://music.example.com", S::FORBIDDEN),
        ("https://music.example.com", S::OK),
    ] {
        let r = post(
            &h,
            "/v1/auth/login",
            json!({ "username": "alice", "password": PW, "device_name": "x" }),
            &[
                CLIENT,
                ("host", "music.example.com"),
                ("x-forwarded-proto", "https"),
                ("x-forwarded-for", "198.51.100.7"),
                ("origin", origin),
            ],
        )
        .await;
        assert_eq!(r.status(), want, "{origin}");
    }
}
