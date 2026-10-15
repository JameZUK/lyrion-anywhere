//! "Your devices": `GET /v1/devices`, `DELETE /v1/devices/{id}` and
//! `POST /v1/devices/sign-out-others` — a signed-in user sees and signs out their own
//! devices, and never anyone else's.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

const CLIENT: (&str, &str) = ("x-anywhere-client", "1");

/// Sign alice (or bob) in on another device; returns its access token and device id.
async fn sign_in(h: &Harness, user: &str, device: &str, extra: Value) -> (String, i64) {
    let mut body = json!({ "username": user, "password": PW, "device_name": device });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/login"))
        .header(CLIENT.0, CLIENT.1)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    let v: Value = r.json().await.unwrap();
    (
        v["access_token"].as_str().unwrap().to_string(),
        v["device"]["id"].as_i64().unwrap(),
    )
}

async fn device_of(h: &Harness, token: &str) -> i64 {
    let v: Value = get(h, "/v1/me", token).await.json().await.unwrap();
    v["device"]["id"].as_i64().unwrap()
}

async fn get(h: &Harness, path: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(h.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn list(h: &Harness, token: &str) -> Vec<Value> {
    let r = get(h, "/v1/devices", token).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let v: Value = r.json().await.unwrap();
    v["devices"].as_array().unwrap().clone()
}

async fn delete(h: &Harness, token: &str, id: &str) -> reqwest::Response {
    reqwest::Client::new()
        .delete(h.url(&format!("/v1/devices/{id}")))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn others(h: &Harness, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(h.url("/v1/devices/sign-out-others"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn raw(
    h: &Harness,
    method: reqwest::Method,
    path: &str,
    headers: &[(&str, &str)],
) -> reqwest::Response {
    let mut req = reqwest::Client::new().request(method, h.url(path));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

/// Open a player session for a device; returns the socket and LMS's end of it.
async fn player(h: &mut Harness, token: &str) -> (Ws, OwnedWriteHalf) {
    let mut ws = h.dial().await;
    ws.send(Message::Text(token.to_string().into()))
        .await
        .unwrap();
    next(&mut ws).await;
    let lms = h.lms.accept().await;
    (ws, lms)
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn me_advertises_the_feature() {
    let h = harness().await;
    let v: Value = get(&h, "/v1/me", &h.tok_a).await.json().await.unwrap();
    assert_eq!(v["features"]["devices"], true);
}

#[tokio::test]
async fn a_user_sees_their_own_devices_only() {
    let h = harness().await;
    let (phone, phone_id) = sign_in(
        &h,
        "alice",
        "Phone",
        json!({ "client_kind": "android", "client_platform": "Pixel 8" }),
    )
    .await;
    let a_id = device_of(&h, &h.tok_a).await;
    let b_id = device_of(&h, &h.tok_b).await;

    let mine = list(&h, &h.tok_a).await;
    let ids: Vec<i64> = mine.iter().map(|d| d["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![a_id, phone_id], "oldest first, alice's only");
    assert!(!ids.contains(&b_id));
    assert_eq!(mine[0]["current"], true);
    assert_eq!(mine[1]["current"], false);
    assert_eq!(mine[1]["name"], "Phone");
    assert_eq!(mine[1]["client_kind"], "android");
    assert_eq!(mine[1]["client_platform"], "Pixel 8");
    assert!(mine[1]["created_at"].as_i64().unwrap() > 0);
    let seen = mine[1]["last_seen"].as_i64().unwrap();
    assert_eq!(seen % 60, 0, "last used is coarse");
    for d in &mine {
        // Exactly these fields: nothing about addresses, players, browsers or tokens.
        let mut keys: Vec<&str> = d.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "client_kind",
                "client_platform",
                "created_at",
                "current",
                "id",
                "last_seen",
                "name"
            ]
        );
    }
    let text = serde_json::to_string(&mine).unwrap();
    for leak in ["device-b", "02:4c:41", "127.0.0.1", &h.tok_a, &phone] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }

    // The same list from the other device marks that one as current.
    let from_phone = list(&h, &phone).await;
    assert_eq!(from_phone[0]["current"], false);
    assert_eq!(from_phone[1]["current"], true);

    // Bob sees only his.
    let bobs = list(&h, &h.tok_b).await;
    assert_eq!(bobs.len(), 1);
    assert_eq!(bobs[0]["id"], b_id);
}

#[tokio::test]
async fn admins_see_only_their_own_devices_here_too() {
    let h = harness().await;
    h.accounts.create_user("root", PW, Role::Admin).unwrap();
    let (admin, admin_id) = sign_in(&h, "root", "Desk", json!({})).await;
    let theirs = list(&h, &admin).await;
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0]["id"], admin_id);
}

#[tokio::test]
async fn paired_devices_are_listed_and_can_be_signed_out() {
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
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/pair"))
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({
            "code": code, "device_name": "Tablet",
            "client_kind": "android", "client_platform": "Pixel Tablet",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    let v: Value = r.json().await.unwrap();
    let id = v["device"]["id"].as_i64().unwrap();
    let paired = v["access_token"].as_str().unwrap().to_string();
    let refresh = v["refresh_token"].as_str().unwrap().to_string();

    let mine = list(&h, &h.tok_a).await;
    let d = mine.iter().find(|d| d["id"] == id).expect("paired device");
    assert_eq!(d["name"], "Tablet");
    assert_eq!(d["client_platform"], "Pixel Tablet");

    assert_eq!(
        delete(&h, &h.tok_a, &id.to_string()).await.status(),
        S::NO_CONTENT
    );
    assert_eq!(get(&h, "/v1/me", &paired).await.status(), S::UNAUTHORIZED);
    // Its refresh token is dead too.
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/refresh"))
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({ "refresh_token": refresh }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Signing out one device
// ---------------------------------------------------------------------------

#[tokio::test]
async fn signing_out_a_device_kills_its_tokens_and_ends_its_session_at_once() {
    let mut h = harness().await;
    let (laptop, laptop_id) = sign_in(&h, "alice", "Laptop", json!({})).await;
    let a_id = device_of(&h, &h.tok_a).await;
    let tok_a = h.tok_a.clone();
    let (mut ws, _lms) = player(&mut h, &tok_a).await;

    let r = delete(&h, &laptop, &a_id.to_string()).await;
    assert_eq!(r.status(), S::NO_CONTENT);
    expect_close(&mut ws, CloseCode::Policy).await;
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::UNAUTHORIZED);
    assert!(h.accounts.authenticate(&h.tok_a).is_none());

    // The caller and bob carry on; the list no longer has it.
    assert_eq!(get(&h, "/v1/me", &laptop).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), S::OK);
    let left = list(&h, &laptop).await;
    assert!(left.iter().all(|d| d["id"] != a_id));

    // Already gone: 404.
    assert_eq!(
        delete(&h, &laptop, &a_id.to_string()).await.status(),
        S::NOT_FOUND
    );

    // Audited as the user's own doing.
    let log = h.accounts.audit_log(100).unwrap();
    let e = log.iter().find(|e| e.event == "device.revoked").unwrap();
    assert_eq!(e.username.as_deref(), Some("alice"));
    assert_eq!(e.device_id, Some(a_id));
    assert_eq!(e.detail, format!("self from {laptop_id}"));
}

#[tokio::test]
async fn another_users_device_is_not_found_and_untouched() {
    let mut h = harness().await;
    let b_id = device_of(&h, &h.tok_b).await;
    let tok_b = h.tok_b.clone();
    let (mut ws, _lms) = player(&mut h, &tok_b).await;
    // Bob's renewable session, to prove his refresh tokens survive.
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/login"))
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({
            "username": "bob", "password": PW, "device_name": "Phone", "refresh": "body"
        }))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let bob_phone = v["device"]["id"].as_i64().unwrap();
    let bob_refresh = v["refresh_token"].as_str().unwrap().to_string();

    for id in [b_id, bob_phone] {
        let r = delete(&h, &h.tok_a, &id.to_string()).await;
        assert_eq!(r.status(), S::NOT_FOUND);
        let body: Value = r.json().await.unwrap();
        assert_eq!(
            body["error"], "no such device",
            "same as one that never existed"
        );
    }
    let never = delete(&h, &h.tok_a, "999999").await;
    assert_eq!(never.status(), S::NOT_FOUND);
    assert_eq!(
        never.json::<Value>().await.unwrap()["error"],
        "no such device"
    );

    // Nothing of bob's changed: his token, his live session, his refresh token.
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), S::OK);
    let quiet = tokio::time::timeout(Duration::from_millis(300), ws.next()).await;
    assert!(quiet.is_err(), "bob's session carried on: {quiet:?}");
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/refresh"))
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({ "refresh_token": bob_refresh }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    assert!(
        !h.accounts
            .audit_log(100)
            .unwrap()
            .iter()
            .any(|e| e.event == "device.revoked")
    );
}

#[tokio::test]
async fn an_admin_cannot_sign_out_a_listeners_device_here() {
    let h = harness().await;
    h.accounts.create_user("root", PW, Role::Admin).unwrap();
    let (admin, _) = sign_in(&h, "root", "Desk", json!({})).await;
    let a_id = device_of(&h, &h.tok_a).await;
    assert_eq!(
        delete(&h, &admin, &a_id.to_string()).await.status(),
        S::NOT_FOUND
    );
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
}

#[tokio::test]
async fn the_device_asking_is_pointed_at_sign_out() {
    let h = harness().await;
    let a_id = device_of(&h, &h.tok_a).await;
    let r = delete(&h, &h.tok_a, &a_id.to_string()).await;
    assert_eq!(r.status(), S::CONFLICT);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("sign out"), "{v}");
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
}

#[tokio::test]
async fn malformed_device_ids_are_refused() {
    let h = harness().await;
    let a_id = device_of(&h, &h.tok_a).await;
    for id in [
        "0",
        "-1",
        "%2B1",
        "1.0",
        "1e3",
        "0x1",
        "abc",
        "%20",
        "9999999999999999999999",
        "%00",
        "1%2F2",
        "self%00",
    ] {
        // Every one decodes as UTF-8, so reaches the handler: the same 404 as a device
        // that doesn't exist, never anything that tells the shapes apart.
        let r = delete(&h, &h.tok_a, id).await;
        assert_eq!(r.status(), S::NOT_FOUND, "{id}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"], "no such device", "{id}");
    }
    // Other shapes never reach a handler that signs anything out.
    for path in ["/v1/devices/1/2", "/v1/devices/", "/v1/devices/self"] {
        let r = raw(
            &h,
            reqwest::Method::DELETE,
            path,
            &[CLIENT, ("authorization", &format!("Bearer {}", h.tok_a))],
        )
        .await;
        assert!(r.status().is_client_error(), "{path}: {}", r.status());
    }
    assert_eq!(list(&h, &h.tok_a).await[0]["id"], a_id);
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), S::OK);
}

// ---------------------------------------------------------------------------
// Signing out every other device
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sign_out_others_keeps_only_the_caller() {
    let mut h = harness().await;
    let (laptop, laptop_id) = sign_in(&h, "alice", "Laptop", json!({})).await;
    let (phone, _) = sign_in(&h, "alice", "Phone", json!({})).await;
    let tok_a = h.tok_a.clone();
    let (mut ws, _lms) = player(&mut h, &tok_a).await;

    let r = others(&h, &laptop).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(r.json::<Value>().await.unwrap()["signed_out"], 2);
    expect_close(&mut ws, CloseCode::Policy).await;
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::UNAUTHORIZED);
    assert_eq!(get(&h, "/v1/me", &phone).await.status(), S::UNAUTHORIZED);
    assert_eq!(get(&h, "/v1/me", &laptop).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), S::OK);
    let left = list(&h, &laptop).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["id"], laptop_id);

    // Again: nothing to do.
    let r = others(&h, &laptop).await;
    assert_eq!(r.json::<Value>().await.unwrap()["signed_out"], 0);
}

// ---------------------------------------------------------------------------
// Who may call it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_endpoint_needs_a_live_access_token() {
    let h = harness().await;
    let a_id = device_of(&h, &h.tok_a).await.to_string();
    let b_id = device_of(&h, &h.tok_b).await.to_string();
    let del = format!("/v1/devices/{b_id}");
    let cases = [
        (reqwest::Method::GET, "/v1/devices"),
        (reqwest::Method::DELETE, del.as_str()),
        (reqwest::Method::POST, "/v1/devices/sign-out-others"),
    ];
    let bare = format!("Bearer {}", "x".repeat(300));
    for (m, path) in &cases {
        for headers in [
            vec![CLIENT],
            vec![CLIENT, ("authorization", "Bearer not-a-token")],
            vec![CLIENT, ("authorization", h.tok_a.as_str())],
            vec![CLIENT, ("authorization", bare.as_str())],
            vec![CLIENT, ("cookie", "anywhere_refresh=abc")],
        ] {
            let r = raw(&h, m.clone(), path, &headers).await;
            assert_eq!(r.status(), S::UNAUTHORIZED, "{m} {path} {headers:?}");
        }
    }

    // A signed-out device's token is dead here too.
    let (gone, _) = sign_in(&h, "alice", "Old phone", json!({})).await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/logout"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&gone)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert_eq!(
        get(&h, "/v1/devices", &gone).await.status(),
        S::UNAUTHORIZED
    );
    assert_eq!(others(&h, &gone).await.status(), S::UNAUTHORIZED);
    assert_eq!(delete(&h, &gone, &a_id).await.status(), S::UNAUTHORIZED);

    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), S::OK);
}

#[tokio::test]
async fn cross_site_sign_outs_are_refused() {
    let h = harness().await;
    let (phone, phone_id) = sign_in(&h, "alice", "Phone", json!({})).await;
    let auth = format!("Bearer {}", h.tok_a);
    let del = format!("/v1/devices/{phone_id}");
    for (m, path) in [
        (reqwest::Method::DELETE, del.as_str()),
        (reqwest::Method::POST, "/v1/devices/sign-out-others"),
    ] {
        // No X-Anywhere-Client: what a cross-site form would send.
        let r = raw(&h, m.clone(), path, &[("authorization", &auth)]).await;
        assert_eq!(r.status(), S::FORBIDDEN, "{m} {path}");
        // A foreign Origin.
        let r = raw(
            &h,
            m.clone(),
            path,
            &[
                CLIENT,
                ("authorization", &auth),
                ("origin", "https://evil.example"),
            ],
        )
        .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{m} {path}");
    }
    assert_eq!(
        get(&h, "/v1/me", &phone).await.status(),
        S::OK,
        "still signed in"
    );
    // The same origin is fine.
    let same = format!("http://{}", h.addr);
    let r = raw(
        &h,
        reqwest::Method::DELETE,
        &del,
        &[CLIENT, ("authorization", &auth), ("origin", &same)],
    )
    .await;
    assert_eq!(r.status(), S::NO_CONTENT);
}

#[tokio::test]
async fn requests_are_rate_limited_per_device() {
    let h = harness().await;
    let (phone, _) = sign_in(&h, "alice", "Phone", json!({})).await;
    let limit = anywhere_gateway::DEVICE_REQUESTS_PER_WINDOW;
    // Shared across the three endpoints, for one device.
    for i in 0..limit {
        assert_eq!(get(&h, "/v1/devices", &phone).await.status(), S::OK, "{i}");
    }
    let r = get(&h, "/v1/devices", &phone).await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    assert_eq!(others(&h, &phone).await.status(), S::TOO_MANY_REQUESTS);
    assert_eq!(delete(&h, &phone, "1").await.status(), S::TOO_MANY_REQUESTS);
    // Refused requests sign nothing out.
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &phone).await.status(), S::OK);
    // Bob has his own allowance.
    assert_eq!(get(&h, "/v1/devices", &h.tok_b).await.status(), S::OK);
}

/// Regression (M5): the allowance was per user, so whoever held a stolen device's access
/// token could loop `GET /v1/devices` and keep its owner, on another device, from signing
/// it out (429). Each device now has its own.
#[tokio::test]
async fn a_stolen_device_cannot_starve_its_owners_sign_out() {
    let h = harness().await;
    let (stolen, stolen_id) = sign_in(&h, "alice", "Phone", json!({})).await;
    for _ in 0..anywhere_gateway::DEVICE_REQUESTS_PER_WINDOW + 5 {
        get(&h, "/v1/devices", &stolen).await;
    }
    assert_eq!(
        get(&h, "/v1/devices", &stolen).await.status(),
        S::TOO_MANY_REQUESTS
    );
    // The owner, on another device, still lists and signs the stolen one out.
    let mine = list(&h, &h.tok_a).await;
    assert!(mine.iter().any(|d| d["id"] == stolen_id), "{mine:?}");
    let r = delete(&h, &h.tok_a, &stolen_id.to_string()).await;
    assert_eq!(r.status(), S::NO_CONTENT);
    assert_eq!(get(&h, "/v1/me", &stolen).await.status(), S::UNAUTHORIZED);
    // As does signing out every other device.
    let (stolen2, _) = sign_in(&h, "alice", "Tablet", json!({})).await;
    for _ in 0..anywhere_gateway::DEVICE_REQUESTS_PER_WINDOW + 5 {
        get(&h, "/v1/devices", &stolen2).await;
    }
    let r = others(&h, &h.tok_a).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &stolen2).await.status(), S::UNAUTHORIZED);
}

/// The audit log says which device did the signing out (M5): `self from <device id>`.
#[tokio::test]
async fn the_audit_log_names_the_device_that_signed_out() {
    let h = harness().await;
    let a_id = device_of(&h, &h.tok_a).await;
    let (_, phone_id) = sign_in(&h, "alice", "Phone", json!({})).await;
    let (_, tablet_id) = sign_in(&h, "alice", "Tablet", json!({})).await;
    assert_eq!(
        delete(&h, &h.tok_a, &phone_id.to_string()).await.status(),
        S::NO_CONTENT
    );
    assert_eq!(others(&h, &h.tok_a).await.status(), S::OK);
    let log = h.accounts.audit_log(100).unwrap();
    for id in [phone_id, tablet_id] {
        let e = log
            .iter()
            .find(|e| e.event == "device.revoked" && e.device_id == Some(id))
            .unwrap_or_else(|| panic!("no audit entry for {id}: {log:?}"));
        assert_eq!(e.username.as_deref(), Some("alice"));
        assert_eq!(e.detail, format!("self from {a_id}"));
    }
}

#[tokio::test]
async fn racing_sign_outs_of_one_device_succeed_once() {
    let h = harness().await;
    let (phone, phone_id) = sign_in(&h, "alice", "Phone", json!({})).await;
    let (laptop, _) = sign_in(&h, "alice", "Laptop", json!({})).await;
    let id = phone_id.to_string();
    let mut tasks = Vec::new();
    for i in 0..8 {
        let token = if i % 2 == 0 {
            h.tok_a.clone()
        } else {
            laptop.clone()
        };
        let url = h.url(&format!("/v1/devices/{id}"));
        tasks.push(tokio::spawn(async move {
            reqwest::Client::new()
                .delete(url)
                .header(CLIENT.0, CLIENT.1)
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    let mut statuses = Vec::new();
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    assert_eq!(statuses.iter().filter(|s| **s == S::NO_CONTENT).count(), 1);
    assert!(
        statuses
            .iter()
            .all(|s| *s == S::NO_CONTENT || *s == S::NOT_FOUND)
    );
    assert_eq!(get(&h, "/v1/me", &phone).await.status(), S::UNAUTHORIZED);
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &laptop).await.status(), S::OK);
    let revocations = h
        .accounts
        .audit_log(100)
        .unwrap()
        .into_iter()
        .filter(|e| e.event == "device.revoked")
        .count();
    assert_eq!(revocations, 1);

    // Two devices each signing out the other at once: whatever the order, no third
    // party is touched and each request either worked or was refused cleanly.
    let r1 = tokio::spawn({
        let (url, t) = (h.url("/v1/devices/sign-out-others"), laptop.clone());
        async move {
            reqwest::Client::new()
                .post(url)
                .header(CLIENT.0, CLIENT.1)
                .bearer_auth(t)
                .send()
                .await
                .unwrap()
                .status()
        }
    });
    let r2 = tokio::spawn({
        let (url, t) = (h.url("/v1/devices/sign-out-others"), h.tok_a.clone());
        async move {
            reqwest::Client::new()
                .post(url)
                .header(CLIENT.0, CLIENT.1)
                .bearer_auth(t)
                .send()
                .await
                .unwrap()
                .status()
        }
    });
    let (s1, s2) = (r1.await.unwrap(), r2.await.unwrap());
    for s in [s1, s2] {
        assert!(s == S::OK || s == S::UNAUTHORIZED, "{s}");
    }
    assert!(list(&h, &h.tok_b).await.len() == 1, "bob untouched");
}

#[tokio::test]
async fn arbitrary_paths_never_break_it() {
    let h = harness().await;
    let auth = format!("Bearer {}", h.tok_a);
    let mut seed: u64 = 0x5eed_1234_abcd;
    for _ in 0..25 {
        let mut id = String::new();
        for _ in 0..(seed % 24) {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let c = (seed >> 33) as u8;
            // Percent-encode everything so any byte reaches the router.
            id.push_str(&format!("%{c:02X}"));
        }
        seed = seed.wrapping_add(7);
        let r = raw(
            &h,
            reqwest::Method::DELETE,
            &format!("/v1/devices/{id}"),
            &[CLIENT, ("authorization", &auth)],
        )
        .await;
        // The rate limit may cut in; nothing else but refusals.
        assert!(
            matches!(
                r.status(),
                S::NOT_FOUND | S::BAD_REQUEST | S::METHOD_NOT_ALLOWED | S::TOO_MANY_REQUESTS
            ),
            "{id}: {}",
            r.status()
        );
    }
    assert_eq!(get(&h, "/v1/me", &h.tok_a).await.status(), S::OK);
    assert_eq!(get(&h, "/v1/me", &h.tok_b).await.status(), S::OK);
}
