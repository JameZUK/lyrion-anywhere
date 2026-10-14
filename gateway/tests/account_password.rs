//! `POST /v1/account/password`: a signed-in user changes their own password.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

const CLIENT: (&str, &str) = ("x-anywhere-client", "1");
const NEW_PW: &str = "a much better passphrase";
const PATH: &str = "/v1/account/password";

async fn login(h: &Harness, user: &str, password: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(h.url("/v1/auth/login"))
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({ "username": user, "password": password, "device_name": "Test browser" }))
        .send()
        .await
        .unwrap()
}

/// A fresh access token for another of alice's devices.
async fn second_device(h: &Harness) -> String {
    let r = login(h, "alice", PW).await;
    assert_eq!(r.status(), S::OK);
    let v: Value = r.json().await.unwrap();
    v["access_token"].as_str().unwrap().to_string()
}

async fn change(h: &Harness, token: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(h.url(PATH))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn raw(
    h: &Harness,
    headers: &[(&str, &str)],
    body: impl Into<reqwest::Body>,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(h.url(PATH))
        .header("content-type", "application/json")
        .body(body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

async fn get(h: &Harness, path: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(h.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn error_of(r: reqwest::Response) -> String {
    let v: Value = r.json().await.unwrap();
    v["error"].as_str().unwrap_or_default().to_string()
}

fn base32_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u64, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("bad base32"),
        } as u64;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

fn code_for(secret_b32: &str, steps_ahead: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let step = now / anywhere_gateway::totp::STEP_SECS + steps_ahead;
    format!(
        "{:06}",
        anywhere_gateway::totp::code_at(&base32_decode(secret_b32), step)
    )
}

/// Turn two-step on for alice; returns the base32 secret.
async fn enrol_alice(h: &Harness) -> String {
    let setup = reqwest::Client::new()
        .post(h.url("/v1/account/totp/setup"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .json(&json!({ "password": PW }))
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), S::OK);
    let secret = setup.json::<Value>().await.unwrap()["secret"]
        .as_str()
        .unwrap()
        .to_string();
    let ok = reqwest::Client::new()
        .post(h.url("/v1/account/totp/confirm"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .json(&json!({ "code": code_for(&secret, 0) }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), S::NO_CONTENT);
    secret
}

fn no_password_in_audit(h: &Harness, secrets: &[&str]) {
    for e in h.accounts.audit_log(1000).unwrap() {
        let row = format!("{e:?}");
        for s in secrets {
            assert!(!row.contains(s), "audit row holds a password: {row}");
        }
    }
}

// ---------------------------------------------------------------------------
// Success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn me_advertises_the_feature() {
    let h = harness().await;
    let v: Value = get(&h, "/v1/me", &h.tok_a).await.json().await.unwrap();
    assert_eq!(v["features"]["change_password"], true);
}

#[tokio::test]
async fn a_user_changes_their_password_and_keeps_their_devices() {
    let h = harness().await;
    let other = second_device(&h).await;
    let r = change(&h, &h.tok_a, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["signed_out"], 0);
    let others = v["other_devices"].as_array().unwrap();
    assert_eq!(others.len(), 1, "{v}");
    assert_eq!(others[0]["name"], "Test browser");
    // Nothing a user shouldn't see about their devices.
    assert!(others[0].get("mac").is_none() && others[0].get("last_addr").is_none());

    // The old password no longer signs in; the new one does.
    assert_eq!(login(&h, "alice", PW).await.status(), S::UNAUTHORIZED);
    assert_eq!(login(&h, "alice", NEW_PW).await.status(), S::OK);

    // The caller's own session and the other device both carry on.
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/account", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &other).await.status(), S::OK);
    // Bob is untouched.
    assert_eq!(login(&h, "bob", PW).await.status(), S::OK);

    // Audited like an admin's reset, saying who did it, never with the password.
    let log = h.accounts.audit_log(1000).unwrap();
    let e = log
        .iter()
        .find(|e| e.event == "user.password_changed")
        .expect("audit entry");
    assert_eq!(e.username.as_deref(), Some("alice"));
    assert_eq!(e.detail, "self");
    assert!(e.device_id.is_some());
    no_password_in_audit(&h, &[PW, NEW_PW]);
}

#[tokio::test]
async fn other_devices_can_be_signed_out_with_it() {
    let h = harness().await;
    let other = second_device(&h).await;
    let bob_before = get(&h, "/v1/me", &h.tok_b).await.status();
    let r = change(
        &h,
        &h.tok_a,
        json!({ "current": PW, "new": NEW_PW, "sign_out_others": true }),
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["signed_out"], 1);
    assert_eq!(v["other_devices"], json!([]));
    assert_eq!(get(&h, "/v1/me", &other).await.status(), S::UNAUTHORIZED);
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), bob_before);
}

#[tokio::test]
async fn outstanding_pairing_codes_are_dropped() {
    let h = harness().await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/devices/pair"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .json(&json!({ "password": PW }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    let code = r.json::<Value>().await.unwrap()["code"]
        .as_str()
        .unwrap()
        .to_string();
    // Bob's code is not alice's to drop.
    let bob_code = h.accounts.create_pairing_code("bob").unwrap();
    let r = change(&h, &h.tok_a, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::OK);
    let pair = |code: String| {
        reqwest::Client::new()
            .post(h.url("/v1/auth/pair"))
            .header(CLIENT.0, CLIENT.1)
            .json(&json!({ "code": code, "device_name": "Phone" }))
            .send()
    };
    assert_eq!(pair(code).await.unwrap().status(), S::UNAUTHORIZED);
    assert_eq!(pair(bob_code).await.unwrap().status(), S::OK);
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_wrong_current_password_is_refused_without_a_401() {
    let h = harness().await;
    let r = change(
        &h,
        &h.tok_a,
        json!({ "current": "not my password", "new": NEW_PW }),
    )
    .await;
    // 403, not 401: a client must not take it for an expired token and retry.
    assert_eq!(r.status(), S::FORBIDDEN);
    assert_eq!(
        error_of(r).await,
        "the current password or two-step code is wrong"
    );
    assert_eq!(login(&h, "alice", PW).await.status(), S::OK);
    assert_eq!(login(&h, "alice", NEW_PW).await.status(), S::UNAUTHORIZED);
    assert!(
        h.accounts
            .audit_log(100)
            .unwrap()
            .iter()
            .any(|e| e.event == "password.change_failed")
    );
    no_password_in_audit(&h, &["not my password", NEW_PW]);
}

#[tokio::test]
async fn two_step_users_need_a_code() {
    let h = harness().await;
    let secret = enrol_alice(&h).await;

    let r = change(&h, &h.tok_a, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::FORBIDDEN);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["totp_required"], true);

    for bad in [json!("000000"), json!(""), json!("12345678901")] {
        let r = change(
            &h,
            &h.tok_a,
            json!({ "current": PW, "new": NEW_PW, "code": bad }),
        )
        .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{bad}");
        assert!(
            r.json::<Value>()
                .await
                .unwrap()
                .get("totp_required")
                .is_none()
        );
    }
    // A right code with a wrong password is still refused.
    let r = change(
        &h,
        &h.tok_a,
        json!({ "current": "not my password", "new": NEW_PW, "code": code_for(&secret, 1) }),
    )
    .await;
    assert_eq!(r.status(), S::FORBIDDEN);

    let r = change(
        &h,
        &h.tok_a,
        json!({ "current": PW, "new": NEW_PW, "code": code_for(&secret, 1) }),
    )
    .await;
    assert_eq!(r.status(), S::OK);
    // Two-step is still on for the new password.
    let r = login(&h, "alice", NEW_PW).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert_eq!(r.json::<Value>().await.unwrap()["totp_required"], true);
}

#[tokio::test]
async fn guesses_here_lock_like_sign_in_failures_and_block_sign_in() {
    let h = harness().await;
    for i in 0..5 {
        let r = change(
            &h,
            &h.tok_a,
            json!({ "current": format!("wrong guess {i}"), "new": NEW_PW }),
        )
        .await;
        assert_eq!(r.status(), S::FORBIDDEN, "guess {i}");
    }
    // Locked: even the right password is refused now, with a time to wait.
    let r = change(&h, &h.tok_a, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    let wait: u64 = r.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(wait >= 60, "{wait}");
    // And sign-in from the same place is locked too.
    assert_eq!(login(&h, "alice", PW).await.status(), S::TOO_MANY_REQUESTS);
    // So are the other account changes that take the password.
    let r = reqwest::Client::new()
        .post(h.url("/v1/account/totp/setup"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .json(&json!({ "password": PW }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    // Nothing changed.
    assert_eq!(
        h.accounts
            .login("alice", PW, None, "x", None, "elsewhere")
            .map(|_| ()),
        Ok(())
    );
}

#[tokio::test]
async fn sign_in_failures_count_here_too() {
    let h = harness().await;
    for _ in 0..5 {
        assert_eq!(
            login(&h, "alice", "wrong password!").await.status(),
            S::UNAUTHORIZED
        );
    }
    let r = change(&h, &h.tok_a, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
}

// ---------------------------------------------------------------------------
// The new password
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_new_password_must_pass_the_policy_and_differ() {
    let h = harness().await;
    let long = "x".repeat(1025);
    for (new, want) in [
        ("short", "password must be at least 10 characters"),
        ("", "password must be at least 10 characters"),
        ("123456789", "password must be at least 10 characters"),
        (long.as_str(), "password too long"),
        (
            PW,
            "the new password must be different from the current one",
        ),
    ] {
        let r = change(&h, &h.tok_a, json!({ "current": PW, "new": new })).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "{new}");
        assert_eq!(error_of(r).await, want);
    }
    // Refused before any password check, so none of it counted towards a lock.
    let r = change(&h, &h.tok_a, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::OK);
}

// ---------------------------------------------------------------------------
// Who may call it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn it_needs_a_live_access_token() {
    let h = harness().await;
    let body = json!({ "current": PW, "new": NEW_PW }).to_string();
    let r = raw(&h, &[CLIENT], body.clone()).await;
    assert_eq!(r.status(), S::UNAUTHORIZED, "no token");
    let r = raw(
        &h,
        &[CLIENT, ("authorization", "Bearer not-a-token")],
        body.clone(),
    )
    .await;
    assert_eq!(r.status(), S::UNAUTHORIZED, "bogus token");
    let r = raw(&h, &[CLIENT, ("authorization", &h.tok_a)], body.clone()).await;
    assert_eq!(r.status(), S::UNAUTHORIZED, "not a bearer token");

    // A revoked device's token is dead here too.
    let other = second_device(&h).await;
    let me: Value = get(&h, "/v1/me", &other).await.json().await.unwrap();
    let id = me["device"]["id"].as_i64().unwrap();
    assert!(h.accounts.revoke_device(id).unwrap());
    let r = change(&h, &other, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    // A signed-out one too.
    let out = second_device(&h).await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/logout"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&out)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    let r = change(&h, &out, json!({ "current": PW, "new": NEW_PW })).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);

    assert_eq!(login(&h, "alice", PW).await.status(), S::OK, "unchanged");
}

#[tokio::test]
async fn cross_site_requests_are_refused() {
    let h = harness().await;
    let body = json!({ "current": PW, "new": NEW_PW }).to_string();
    let auth = format!("Bearer {}", h.tok_a);
    // No X-Anywhere-Client: what a cross-site form would send.
    let r = raw(&h, &[("authorization", &auth)], body.clone()).await;
    assert_eq!(r.status(), S::FORBIDDEN);
    // A foreign Origin.
    let r = raw(
        &h,
        &[
            CLIENT,
            ("authorization", &auth),
            ("origin", "https://evil.example"),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(r.status(), S::FORBIDDEN);
    // A cookie is no credential here.
    let r = raw(
        &h,
        &[CLIENT, ("cookie", "anywhere_refresh=abc")],
        body.clone(),
    )
    .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert_eq!(login(&h, "alice", PW).await.status(), S::OK, "unchanged");
    // The same origin is fine.
    let same = format!("http://{}", h.addr);
    let r = raw(
        &h,
        &[CLIENT, ("authorization", &auth), ("origin", &same)],
        body,
    )
    .await;
    assert_eq!(r.status(), S::OK);
}

#[tokio::test]
async fn malformed_bodies_are_refused() {
    let h = harness().await;
    let auth = format!("Bearer {}", h.tok_a);
    let headers = [CLIENT, ("authorization", auth.as_str())];
    for body in [
        String::new(),
        "not json".into(),
        "[]".into(),
        "null".into(),
        json!({ "current": PW }).to_string(),
        json!({ "new": NEW_PW }).to_string(),
        json!({ "current": PW, "new": NEW_PW, "extra": 1 }).to_string(),
        json!({ "current": PW, "new": NEW_PW, "password": NEW_PW }).to_string(),
        json!({ "current": 1, "new": NEW_PW }).to_string(),
        json!({ "current": PW, "new": ["a"] }).to_string(),
        json!({ "current": PW, "new": NEW_PW, "code": 123456 }).to_string(),
        json!({ "current": PW, "new": NEW_PW, "sign_out_others": "yes" }).to_string(),
        format!("{{\"current\":\"{PW}\",\"new\":\"{NEW_PW}\",\"current\":\"x\"}}"),
    ] {
        let r = raw(&h, &headers, body.clone()).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "{body}");
    }
    let big = json!({ "current": PW, "new": "y".repeat(5000) }).to_string();
    let r = raw(&h, &headers, big).await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    // Not a byte of it counted towards a lock or changed anything.
    assert_eq!(login(&h, "alice", PW).await.status(), S::OK);
    let r = change(
        &h,
        &h.tok_a,
        json!({ "current": PW, "new": NEW_PW, "code": null }),
    )
    .await;
    assert_eq!(r.status(), S::OK);
}

#[tokio::test]
async fn arbitrary_bodies_never_break_it() {
    let h = harness().await;
    let auth = format!("Bearer {}", h.tok_a);
    let headers = [CLIENT, ("authorization", auth.as_str())];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for i in 0..64 {
        let len = (i * 37) % 300;
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 24) as u8
            })
            .collect();
        let r = raw(&h, &headers, bytes).await;
        assert!(
            r.status() == S::BAD_REQUEST || r.status() == S::PAYLOAD_TOO_LARGE,
            "{}",
            r.status()
        );
    }
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
}
