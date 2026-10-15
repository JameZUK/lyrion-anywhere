//! Devices: last-seen address and client type (shown to admins only), and renaming
//! (`PATCH /v1/devices/self`, and the admin API), with the LMS player name following.

mod common;
use common::*;

use anywhere_gateway::admin;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

const CLIENT: (&str, &str) = ("x-anywhere-client", "1");
const SECRET_LEN: usize = 64;

/// Headers for a request arriving through the (loopback) reverse proxy from `ip`.
fn via(ip: &str) -> [(&str, &str); 3] {
    [
        CLIENT,
        ("x-forwarded-proto", "https"),
        ("x-forwarded-for", ip),
    ]
}

async fn admin_api(h: &Harness) -> (String, String) {
    let secret = "s".repeat(SECRET_LEN);
    let app = admin::router(h.gw.clone(), secret.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    (format!("http://{addr}"), secret)
}

async fn devices(base: &str, secret: &str) -> Vec<Value> {
    let v: Value = reqwest::Client::new()
        .get(format!("{base}/admin/v1/devices"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["devices"].as_array().unwrap().clone()
}

fn device_of<'a>(list: &'a [Value], user: &str) -> &'a Value {
    list.iter().find(|d| d["username"] == user).unwrap()
}

async fn post(h: &Harness, path: &str, body: Value, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut req = reqwest::Client::new().post(h.url(path)).json(&body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

async fn rename_self(
    h: &Harness,
    token: Option<&str>,
    body: &str,
    headers: &[(&str, &str)],
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .patch(h.url("/v1/devices/self"))
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

async fn me_name(h: &Harness, token: &str) -> String {
    let v: Value = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["device"]["name"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Address and client type
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sign_in_records_the_proxied_address_and_client_type() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let mut headers = via("198.51.100.7").to_vec();
    headers.push(("user-agent", "Mozilla/5.0 (iPad; <script>) Safari/605.1.15"));
    let r = post(
        &h,
        "/v1/auth/login",
        json!({
            "username": "alice", "password": PW, "device_name": "Tablet",
            "client_kind": "pwa", "client_platform": "Safari on iPadOS",
        }),
        &headers,
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let id = r.json::<Value>().await.unwrap()["device"]["id"]
        .as_i64()
        .unwrap();
    let list = devices(&base, &secret).await;
    let d = list.iter().find(|d| d["id"] == id).unwrap();
    assert_eq!(d["last_addr"], "198.51.100.7");
    assert!(d["last_seen"].as_i64().unwrap() > 0);
    assert_eq!(d["client_kind"], "pwa");
    assert_eq!(d["client_platform"], "Safari on iPadOS");
    let ua = d["user_agent"].as_str().unwrap();
    assert!(ua.starts_with("Mozilla/5.0 (iPad;"), "{ua}");
    assert!(!ua.contains('<') && !ua.contains('>'), "{ua}");
}

#[tokio::test]
async fn a_client_cannot_choose_its_recorded_address_without_a_trusted_proxy() {
    // The gateway trusts only loopback here, and the peer *is* loopback, so a forwarded
    // address is believed; from an untrusted peer it would be ignored (see lib.rs tests).
    // What a client can't do is pick one through X-Forwarded-For's left-hand entries.
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let r = post(
        &h,
        "/v1/auth/login",
        json!({ "username": "alice", "password": PW, "device_name": "Laptop" }),
        &via("203.0.113.66, 198.51.100.9"),
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let id = r.json::<Value>().await.unwrap()["device"]["id"]
        .as_i64()
        .unwrap();
    let list = devices(&base, &secret).await;
    let d = list.iter().find(|d| d["id"] == id).unwrap();
    assert_eq!(d["last_addr"], "198.51.100.9", "rightmost untrusted entry");
    // No client type was sent: nothing is recorded.
    assert!(d["client_kind"].is_null());
    assert!(d["client_platform"].is_null());
}

#[tokio::test]
async fn bad_client_type_fields_are_refused() {
    let h = harness().await;
    for (field, value) in [
        ("client_kind", json!("ios")),
        ("client_kind", json!("PWA")),
        ("client_kind", json!("<script>")),
        ("client_kind", json!(1)),
        ("client_kind", json!(["pwa"])),
        ("client_platform", json!("x".repeat(49))),
        ("client_platform", json!(42)),
        ("client_platform", json!({"a": 1})),
        ("client_extra", json!("x")),
    ] {
        let mut body = json!({ "username": "alice", "password": PW, "device_name": "D" });
        body[field] = value.clone();
        let r = post(&h, "/v1/auth/login", body, &[CLIENT]).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "login {field}={value}");
        let mut body = json!({ "code": "ABCDEFGHJK", "device_name": "D" });
        body[field] = value.clone();
        let r = post(&h, "/v1/auth/pair", body, &[CLIENT]).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "pair {field}={value}");
        let mut body = json!({ "refresh_token": "x" });
        body[field] = value.clone();
        let r = post(&h, "/v1/auth/refresh", body, &[CLIENT]).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "refresh {field}={value}");
    }
}

#[tokio::test]
async fn hostile_platform_strings_are_cleaned() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let r = post(
        &h,
        "/v1/auth/login",
        json!({
            "username": "bob", "password": PW, "device_name": "D",
            "client_kind": "browser", "client_platform": "<b>Chrome</b>\u{202e} on\nLinux",
        }),
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let id = r.json::<Value>().await.unwrap()["device"]["id"]
        .as_i64()
        .unwrap();
    let list = devices(&base, &secret).await;
    let p = list.iter().find(|d| d["id"] == id).unwrap()["client_platform"]
        .as_str()
        .unwrap()
        .to_string();
    for bad in ["<", ">", "/", "\u{202e}", "\n"] {
        assert!(!p.contains(bad), "{bad:?} survived in {p:?}");
    }
    assert_eq!(p, "bChromeb on Linux");
}

#[tokio::test]
async fn refresh_updates_the_client_type_and_address() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let r = post(
        &h,
        "/v1/auth/login",
        json!({
            "username": "alice", "password": PW, "device_name": "Phone",
            "refresh": "body", "client_kind": "browser", "client_platform": "Chrome on Android",
        }),
        &via("198.51.100.20"),
    )
    .await;
    let v: Value = r.json().await.unwrap();
    let id = v["device"]["id"].as_i64().unwrap();
    let refresh = v["refresh_token"].as_str().unwrap();
    let r = post(
        &h,
        "/v1/auth/refresh",
        json!({ "refresh_token": refresh, "client_kind": "pwa" }),
        &via("198.51.100.21"),
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let list = devices(&base, &secret).await;
    let d = list.iter().find(|d| d["id"] == id).unwrap();
    assert_eq!(d["client_kind"], "pwa");
    assert_eq!(
        d["client_platform"], "Chrome on Android",
        "kept when not sent"
    );
    assert_eq!(d["last_addr"], "198.51.100.21");
}

#[tokio::test]
async fn api_use_records_the_address_at_most_once_a_minute() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    // The harness signed alice in directly: no address yet.
    assert!(device_of(&devices(&base, &secret).await, "alice")["last_addr"].is_null());
    let me = |ip: &'static str| {
        let h = &h;
        async move {
            let mut req = reqwest::Client::new()
                .get(h.url("/v1/me"))
                .bearer_auth(&h.tok_a);
            for (k, v) in via(ip) {
                req = req.header(k, v);
            }
            req.send().await.unwrap().status()
        }
    };
    assert_eq!(me("198.51.100.30").await, S::OK);
    assert_eq!(
        device_of(&devices(&base, &secret).await, "alice")["last_addr"],
        "198.51.100.30"
    );
    // Within the minute, further use isn't written.
    assert_eq!(me("198.51.100.31").await, S::OK);
    assert_eq!(
        device_of(&devices(&base, &secret).await, "alice")["last_addr"],
        "198.51.100.30"
    );
    // An invalid token records nothing.
    let r = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth("not-a-token")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-for", "198.51.100.32")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert!(device_of(&devices(&base, &secret).await, "bob")["last_addr"].is_null());
}

#[tokio::test]
async fn attaching_a_player_records_the_address() {
    let mut h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let mut ws = h
        .dial_with(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "198.51.100.40"),
        ])
        .await
        .unwrap();
    connect(&mut ws, &h.tok_b).await;
    let _lms = h.lms.accept().await;
    // Recorded just after the greeting is sent.
    let mut seen = Value::Null;
    for _ in 0..50 {
        seen = device_of(&devices(&base, &secret).await, "bob")["last_addr"].clone();
        if !seen.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(seen, "198.51.100.40");
}

#[tokio::test]
async fn addresses_are_only_in_the_admin_api() {
    let h = harness().await;
    let r = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    let body = r.text().await.unwrap();
    for field in ["last_addr", "user_agent", "client_platform", "127.0.0.1"] {
        assert!(!body.contains(field), "/v1/me exposes {field}: {body}");
    }
    // The user's own device list ("Your devices") carries no addresses or user agents.
    let r = reqwest::Client::new()
        .get(h.url("/v1/devices"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body = r.text().await.unwrap();
    for field in ["last_addr", "user_agent", "mac", "username", "127.0.0.1"] {
        assert!(!body.contains(field), "/v1/devices exposes {field}: {body}");
    }
    // The admin list isn't routed on the public listener.
    let r = reqwest::Client::new()
        .get(h.url("/admin/v1/devices"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error(), "{}", r.status());
}

#[tokio::test]
async fn revoking_a_device_clears_its_address() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let r = post(
        &h,
        "/v1/auth/login",
        json!({ "username": "alice", "password": PW, "device_name": "Old laptop" }),
        &via("198.51.100.50"),
    )
    .await;
    let id = r.json::<Value>().await.unwrap()["device"]["id"]
        .as_i64()
        .unwrap();
    let r = reqwest::Client::new()
        .post(format!("{base}/admin/v1/devices/{id}/revoke"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert!(!devices(&base, &secret).await.iter().any(|d| d["id"] == id));
    assert_eq!(h.accounts.stored_address_for_test(id), None);
}

// ---------------------------------------------------------------------------
// Renaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_user_renames_their_own_device() {
    let h = harness().await;
    let r = rename_self(
        &h,
        Some(&h.tok_a),
        r#"{"name":"  Kitchen   iPad "}"#,
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["name"], "Kitchen iPad");
    assert_eq!(me_name(&h, &h.tok_a).await, "Kitchen iPad");
    // Bob's device is untouched.
    assert_eq!(me_name(&h, &h.tok_b).await, "device-b");
    // Letters of any script are fine.
    let r = rename_self(
        &h,
        Some(&h.tok_a),
        r#"{"name":"Küche — Señora's"}"#,
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(me_name(&h, &h.tok_a).await, "Küche Señora's");
}

#[tokio::test]
async fn renaming_needs_a_token_and_the_client_header() {
    let h = harness().await;
    let body = r#"{"name":"New"}"#;
    assert_eq!(
        rename_self(&h, None, body, &[CLIENT]).await.status(),
        S::UNAUTHORIZED
    );
    assert_eq!(
        rename_self(&h, Some("garbage"), body, &[CLIENT])
            .await
            .status(),
        S::UNAUTHORIZED
    );
    assert_eq!(
        rename_self(&h, Some(&h.tok_a), body, &[]).await.status(),
        S::FORBIDDEN,
        "no client header (CSRF)"
    );
    assert_eq!(
        rename_self(
            &h,
            Some(&h.tok_a),
            body,
            &[CLIENT, ("origin", "https://evil.example")]
        )
        .await
        .status(),
        S::FORBIDDEN,
        "cross-origin"
    );
    assert_eq!(me_name(&h, &h.tok_a).await, "device-a");
    // Only PATCH, only "self".
    let r = reqwest::Client::new()
        .post(h.url("/v1/devices/self"))
        .bearer_auth(&h.tok_a)
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({"name": "New"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error());
}

#[tokio::test]
async fn bad_names_are_refused_and_hostile_ones_cleaned() {
    let h = harness().await;
    let too_long = format!(r#"{{"name":"{}"}}"#, "x".repeat(65));
    let huge = format!(r#"{{"name":"{}"}}"#, "x".repeat(100_000));
    for body in [
        r#"{"name":""}"#,
        r#"{"name":"   "}"#,
        r#"{"name":"<>/\\"}"#,
        r#"{"name":"\u202e\u200b"}"#,
        r#"{"name":null}"#,
        r#"{"name":42}"#,
        r#"{}"#,
        r#"{"name":"ok","id":2}"#,
        r#"not json"#,
        too_long.as_str(),
        huge.as_str(),
    ] {
        let r = rename_self(&h, Some(&h.tok_a), body, &[CLIENT]).await;
        assert!(
            r.status() == S::BAD_REQUEST || r.status() == S::PAYLOAD_TOO_LARGE,
            "{} gave {}",
            &body[..body.len().min(40)],
            r.status()
        );
    }
    assert_eq!(me_name(&h, &h.tok_a).await, "device-a");
    // 64 characters is the limit.
    let max = format!(r#"{{"name":"{}"}}"#, "é".repeat(64));
    assert_eq!(
        rename_self(&h, Some(&h.tok_a), &max, &[CLIENT])
            .await
            .status(),
        S::OK
    );
    // Markup, controls and bidi characters are removed.
    let r = rename_self(
        &h,
        Some(&h.tok_a),
        r#"{"name":"<img src=x onerror=alert(1)>Den\u202e\u0007 TV"}"#,
        &[CLIENT],
    )
    .await;
    assert_eq!(r.status(), S::OK);
    let name = me_name(&h, &h.tok_a).await;
    for bad in ['<', '>', '=', '\u{202e}', '\u{7}'] {
        assert!(!name.contains(bad), "{bad:?} survived in {name:?}");
    }
}

#[tokio::test]
async fn names_are_unique_per_user() {
    let h = harness().await;
    let other = h
        .accounts
        .login("alice", PW, None, "Phone", None, "test")
        .unwrap();
    let r = rename_self(&h, Some(&h.tok_a), r#"{"name":"phone"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::BAD_REQUEST, "alice already has a Phone");
    // Another user may use the same name.
    let r = rename_self(&h, Some(&h.tok_b), r#"{"name":"Phone"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    // Renaming a device to its own name is fine.
    let r = rename_self(&h, Some(&other.access), r#"{"name":"PHONE"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
}

#[tokio::test]
async fn the_lms_player_name_follows_a_rename() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    let r = rename_self(&h, Some(&h.tok_a), r#"{"name":"Living room"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    let mac = format_mac(&h.mac_a);
    let sent = h.lms.rec.lock().unwrap().rpc.clone();
    assert!(
        sent.contains(&json!([mac, ["name", "Living room"]])),
        "LMS was not told: {sent:?}"
    );
    // A later name query with no usable answer from the client gets the new name.
    lms.write_all(&encode_server_frame(*b"setd", &[0]))
        .await
        .unwrap();
    next(&mut ws).await;
    ws.send(Message::Binary(client_frame(b"SETD", b"\0\0").into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "SETD"]).await;
    assert_eq!(h.lms.payloads(b"SETD")[0], b"\0Living room\0");
    // A new connection is greeted with the new name.
    drop(ws);
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    match next(&mut ws).await {
        Message::Text(t) => {
            let g: Value = serde_json::from_str(t.as_str()).unwrap();
            assert_eq!(g["name"], "Living room");
        }
        other => panic!("expected greeting, got {other:?}"),
    }
}

#[tokio::test]
async fn renaming_a_device_without_a_player_sends_nothing_to_lms() {
    let h = harness().await;
    let r = rename_self(&h, Some(&h.tok_b), r#"{"name":"Study"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    let sent = h.lms.rec.lock().unwrap().rpc.clone();
    assert!(
        !sent.iter().any(|p| p[1][0] == "name"),
        "nothing to rename in LMS: {sent:?}"
    );
}

#[tokio::test]
async fn renaming_is_rate_limited_per_device() {
    // Pentest 2026-10 (A7): renames were unlimited, each writing an audit entry and
    // telling LMS, so a listener could flood the admin's audit view and LMS.
    let h = harness().await;
    let limit = anywhere_gateway::RENAMES_PER_WINDOW;
    for i in 0..limit {
        let body = format!(r#"{{"name":"Name {i}"}}"#);
        let r = rename_self(&h, Some(&h.tok_a), &body, &[CLIENT]).await;
        assert_eq!(r.status(), S::OK, "rename {i}");
    }
    let r = rename_self(&h, Some(&h.tok_a), r#"{"name":"One more"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    assert_eq!(me_name(&h, &h.tok_a).await, format!("Name {}", limit - 1));
    // Per device: Bob's device can still be renamed.
    let r = rename_self(&h, Some(&h.tok_b), r#"{"name":"Study"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    // And the audit log holds no more of Alice's renames than the limit allowed.
    let renames = h
        .accounts
        .audit_log(1000)
        .unwrap()
        .iter()
        .filter(|e| e.event == "device.renamed" && e.username.as_deref() == Some("alice"))
        .count();
    assert_eq!(renames, limit);
}

#[tokio::test]
async fn renaming_to_the_same_name_writes_nothing() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    let r = rename_self(&h, Some(&h.tok_a), r#"{"name":"Den"}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    let audited = |h: &Harness| {
        h.accounts
            .audit_log(1000)
            .unwrap()
            .iter()
            .filter(|e| e.event == "device.renamed")
            .count()
    };
    let names_sent = |h: &Harness| {
        h.lms
            .rec
            .lock()
            .unwrap()
            .rpc
            .iter()
            .filter(|p| p[1][0] == "name")
            .count()
    };
    let (a, n) = (audited(&h), names_sent(&h));
    assert_eq!((a, n), (1, 1));
    // The same name again (after cleaning): answered, but no audit entry, nothing to LMS.
    let r = rename_self(&h, Some(&h.tok_a), r#"{"name":"  Den "}"#, &[CLIENT]).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.json::<Value>().await.unwrap()["name"], "Den");
    assert_eq!((audited(&h), names_sent(&h)), (a, n));
    // The entry says whose device it is and that its user renamed it.
    let log = h.accounts.audit_log(1).unwrap();
    assert_eq!(log[0].username.as_deref(), Some("alice"));
    assert_eq!(log[0].detail, "Den (by the user)");
}

#[tokio::test]
async fn admins_rename_any_device() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let c = reqwest::Client::new();
    let bob = device_of(&devices(&base, &secret).await, "bob")["id"]
        .as_i64()
        .unwrap();
    // POST form (for LMS's HTTP client) and PATCH.
    let r = c
        .post(format!("{base}/admin/v1/devices/{bob}/name"))
        .bearer_auth(&secret)
        .json(&json!({"name": "Bob's <b>laptop</b>"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    assert_eq!(me_name(&h, &h.tok_b).await, "Bob's blaptopb");
    let r = c
        .patch(format!("{base}/admin/v1/devices/{bob}"))
        .bearer_auth(&secret)
        .json(&json!({"name": "Desk"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    assert_eq!(
        device_of(&devices(&base, &secret).await, "bob")["name"],
        "Desk"
    );
    // Unknown device, bad names, unknown fields, no secret.
    let r = c
        .post(format!("{base}/admin/v1/devices/999999/name"))
        .bearer_auth(&secret)
        .json(&json!({"name": "X"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NOT_FOUND);
    for bad in [
        json!({"name": ""}),
        json!({"name": "x".repeat(65)}),
        json!({"name": "<>"}),
        json!({"name": "ok", "user": "alice"}),
        json!({}),
    ] {
        let r = c
            .post(format!("{base}/admin/v1/devices/{bob}/name"))
            .bearer_auth(&secret)
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert!(r.status().is_client_error(), "{bad}: {}", r.status());
    }
    for path in [
        format!("/admin/v1/devices/{bob}/name"),
        "/admin/v1/devices/abc/name".into(),
    ] {
        let r = c
            .post(format!("{base}{path}"))
            .json(&json!({"name": "Hacked"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::FORBIDDEN, "{path} without the secret");
    }
    // A listener's access token is not the admin secret.
    let r = c
        .post(format!("{base}/admin/v1/devices/{bob}/name"))
        .bearer_auth(&h.tok_a)
        .json(&json!({"name": "Hacked"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    // And the admin route isn't served on the public listener.
    let r = c
        .post(h.url(&format!("/admin/v1/devices/{bob}/name")))
        .bearer_auth(&h.tok_a)
        .header(CLIENT.0, CLIENT.1)
        .json(&json!({"name": "Hacked"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NOT_FOUND);
    assert_eq!(me_name(&h, &h.tok_b).await, "Desk");
    // The audit log records renames.
    let audit: Value = c
        .get(format!("{base}/admin/v1/audit?limit=50"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        audit["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["event"] == "device.renamed")
    );
}

#[tokio::test]
async fn the_default_sign_in_lifetime_is_90_days() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let s: Value = reqwest::Client::new()
        .get(format!("{base}/admin/v1/settings"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["session_days"], 90);
    assert_eq!(s["session_days_default"], 90);
    assert_eq!(s["session_days_min"], 1);
    assert_eq!(s["session_days_max"], 365);
    let r = post(
        &h,
        "/v1/auth/login",
        json!({ "username": "alice", "password": PW, "device_name": "D" }),
        &[CLIENT],
    )
    .await;
    let cookie = r.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(cookie.contains("Max-Age=7776000"), "{cookie}");
}
