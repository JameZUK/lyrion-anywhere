//! Adding another device: `POST /v1/devices/pair` (a signed-in user's own pairing code)
//! and redeeming codes with `POST /v1/auth/pair`.

mod common;
use common::*;

use anywhere_gateway::accounts::SELF_PAIRING_PER_HOUR;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

const CLIENT: (&str, &str) = ("x-anywhere-client", "1");

/// Headers for a request arriving through the (loopback) reverse proxy from `ip`.
fn via(ip: &str) -> [(&str, &str); 3] {
    [
        CLIENT,
        ("x-forwarded-proto", "https"),
        ("x-forwarded-for", ip),
    ]
}

async fn create(
    h: &Harness,
    token: Option<&str>,
    body: &str,
    headers: &[(&str, &str)],
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(h.url("/v1/devices/pair"))
        .header("content-type", "application/json")
        .body(body.to_string());
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

/// A code for alice, made with her own access token and password.
async fn alice_code(h: &Harness) -> String {
    let r = create(
        h,
        Some(&h.tok_a),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let v: Value = r.json().await.unwrap();
    v["code"].as_str().unwrap().to_string()
}

async fn redeem(h: &Harness, code: &str, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(h.url("/v1/auth/pair"))
        .json(&json!({ "code": code, "device_name": "New phone", "client_kind": "android" }));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn a_signed_in_user_creates_a_code_that_signs_in_as_them() {
    let h = harness().await;
    let r = create(
        &h,
        Some(&h.tok_a),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let v: Value = r.json().await.unwrap();
    let code = v["code"].as_str().unwrap();
    assert_eq!(code.len(), 10);
    assert!(
        code.bytes()
            .all(|b| b.is_ascii_uppercase() || (b'2'..=b'9').contains(&b))
    );
    assert_eq!(v["expires_in"], 600);
    assert_eq!(v.as_object().unwrap().len(), 2, "nothing else: {v}");

    let r = redeem(&h, code, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    let t: Value = r.json().await.unwrap();
    assert_eq!(t["user"]["username"], "alice");
    assert_eq!(t["device"]["name"], "New phone");
    assert!(t["refresh_token"].as_str().is_some_and(|s| !s.is_empty()));
    // A new device, not the one that made the code.
    let me: Value = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(me["device"]["id"], t["device"]["id"]);

    // Bob's code is bob's.
    let r = create(
        &h,
        Some(&h.tok_b),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    let code = r.json::<Value>().await.unwrap()["code"]
        .as_str()
        .unwrap()
        .to_string();
    let t: Value = redeem(&h, &code, &[CLIENT]).await.json().await.unwrap();
    assert_eq!(t["user"]["username"], "bob");
}

#[tokio::test]
async fn creating_a_code_needs_a_token_the_client_header_and_an_allowed_origin() {
    let h = harness().await;
    let body = json!({ "password": PW }).to_string();
    for (token, headers, want) in [
        (None, vec![CLIENT], S::UNAUTHORIZED),
        (Some("not-a-token"), vec![CLIENT], S::UNAUTHORIZED),
        (Some(h.tok_a.as_str()), vec![], S::FORBIDDEN),
        (
            Some(h.tok_a.as_str()),
            vec![CLIENT, ("origin", "https://evil.example")],
            S::FORBIDDEN,
        ),
    ] {
        let r = create(&h, token, &body, &headers).await;
        assert_eq!(r.status(), want, "{token:?} {headers:?}");
        let v: Value = r.json().await.unwrap();
        assert!(v.get("code").is_none());
    }
    // GET is not a way in either.
    let r = reqwest::Client::new()
        .get(h.url("/v1/devices/pair"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::METHOD_NOT_ALLOWED);
}

/// A listener can't make a code for anyone else: there is no field to name a user, and
/// trying to add one is refused outright.
#[tokio::test]
async fn nobody_can_create_a_code_for_another_user() {
    let h = harness().await;
    for body in [
        json!({ "password": PW, "username": "bob" }).to_string(),
        json!({ "password": PW, "user_id": 2 }).to_string(),
        json!({ "password": PW, "device_id": 1 }).to_string(),
        json!({}).to_string(),
        json!({ "password": 12 }).to_string(),
        "not json".to_string(),
        "".to_string(),
        format!("{{\"password\":\"{}\"}}", "x".repeat(5000)),
    ] {
        let r = create(&h, Some(&h.tok_a), &body, &[CLIENT]).await;
        assert!(
            r.status() == S::BAD_REQUEST || r.status() == S::PAYLOAD_TOO_LARGE,
            "{body:.40}: {}",
            r.status()
        );
    }
    // Bob's password with alice's token is just a wrong password.
    let r = create(
        &h,
        Some(&h.tok_a),
        &json!({ "password": "bob's password!" }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    // And a code alice makes never signs in as bob.
    let code = alice_code(&h).await;
    let t: Value = redeem(&h, &code, &[CLIENT]).await.json().await.unwrap();
    assert_eq!(t["user"]["username"], "alice");
}

#[tokio::test]
async fn codes_are_single_use_and_expire() {
    let h = harness().await;
    let code = alice_code(&h).await;
    // Typed by hand: lower case with a dash still works, once.
    let typed = format!("{}-{}", code[..5].to_lowercase(), &code[5..]);
    assert_eq!(redeem(&h, &typed, &[CLIENT]).await.status(), S::OK);
    assert_eq!(redeem(&h, &code, &[CLIENT]).await.status(), S::UNAUTHORIZED);

    let code = alice_code(&h).await;
    h.accounts.expire_pairing_codes_for_test();
    let r = redeem(&h, &code, &[CLIENT]).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert!(
        r.json::<Value>()
            .await
            .unwrap()
            .get("access_token")
            .is_none()
    );
}

#[tokio::test]
async fn codes_are_limited_per_user_and_hour() {
    let h = harness().await;
    for _ in 0..SELF_PAIRING_PER_HOUR {
        alice_code(&h).await;
    }
    let r = create(
        &h,
        Some(&h.tok_a),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    let wait: u64 = r.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=3600).contains(&wait), "{wait}");
    // Others are unaffected.
    let r = create(
        &h,
        Some(&h.tok_b),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
}

#[tokio::test]
async fn wrong_passwords_lock_code_creation() {
    let h = harness().await;
    let wrong = json!({ "password": "wrong password!" }).to_string();
    for _ in 0..5 {
        let r = create(&h, Some(&h.tok_a), &wrong, &[CLIENT]).await;
        assert_eq!(r.status(), S::UNAUTHORIZED);
    }
    let r = create(
        &h,
        Some(&h.tok_a),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    assert!(r.json::<Value>().await.unwrap().get("code").is_none());
}

/// Redeeming a user's own code goes through the same lockouts as any pairing: an address
/// that has been guessing is refused even with a valid code, while others still pair.
#[tokio::test]
async fn pairing_lockouts_still_apply_to_own_codes() {
    let h = harness().await;
    let code = alice_code(&h).await;
    for _ in 0..5 {
        let r = redeem(&h, "AAAAAAAAAA", &via("198.51.100.66")).await;
        assert_eq!(r.status(), S::UNAUTHORIZED);
    }
    let r = redeem(&h, &code, &via("198.51.100.66")).await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    // The refused attempt didn't use the code up.
    assert_eq!(
        redeem(&h, &code, &via("203.0.113.20")).await.status(),
        S::OK
    );
}

/// A revoked device's token can no longer make codes.
#[tokio::test]
async fn a_signed_out_device_cannot_create_codes() {
    let h = harness().await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/logout"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    let r = create(
        &h,
        Some(&h.tok_a),
        &json!({ "password": PW }).to_string(),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
}
