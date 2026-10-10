//! Account endpoints (`/v1/auth/*`, `/v1/me`) and the loopback admin API.

mod common;
use common::*;

use anywhere_gateway::admin;
use axum::extract::ConnectInfo;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

const CLIENT: (&str, &str) = ("x-anywhere-client", "1");

impl Harness {
    async fn post(&self, path: &str, body: Value, extra: &[(&str, &str)]) -> reqwest::Response {
        let mut req = reqwest::Client::new().post(self.url(path)).json(&body);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        req.send().await.unwrap()
    }

    async fn login(&self, user: &str, password: &str, extra: &[(&str, &str)]) -> reqwest::Response {
        self.post(
            "/v1/auth/login",
            json!({ "username": user, "password": password, "device_name": "Test browser" }),
            extra,
        )
        .await
    }

    /// Start the admin API on its own loopback listener.
    async fn admin(&self) -> (String, String) {
        let secret = "s".repeat(64);
        let app = admin::router(self.gw.clone(), secret.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap()
        });
        (format!("http://{addr}"), secret)
    }
}

fn cookie_from(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().to_string())
}

fn cookie_value(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

async fn json_of(resp: reqwest::Response) -> Value {
    resp.json().await.unwrap()
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

#[tokio::test]
async fn browser_login_sets_a_locked_down_refresh_cookie() {
    let h = harness().await;
    let resp = h.login("alice", PW, &[CLIENT]).await;
    assert_eq!(resp.status(), S::OK);
    let set = cookie_from(&resp).expect("refresh cookie");
    for attr in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/v1/auth"] {
        assert!(set.contains(attr), "cookie lacks {attr}: {set}");
    }
    assert_eq!(resp.headers()["cache-control"], "no-store");
    let body = json_of(resp).await;
    assert!(
        body.get("refresh_token").is_none(),
        "browsers must not see the refresh token"
    );
    assert_eq!(body["user"]["username"], "alice");
    assert!(
        body["device"]["mac"]
            .as_str()
            .unwrap()
            .starts_with("02:4c:41:")
    );

    // The access token works for /v1/me and the player socket.
    let token = body["access_token"].as_str().unwrap();
    let me = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(json_of(me).await["device"]["mac"], body["device"]["mac"]);
}

#[tokio::test]
async fn app_login_can_take_the_refresh_token_in_the_body() {
    let h = harness().await;
    let resp = h
        .post(
            "/v1/auth/login",
            json!({ "username": "alice", "password": PW, "device_name": "App", "refresh": "body" }),
            &[CLIENT],
        )
        .await;
    assert_eq!(resp.status(), S::OK);
    assert!(cookie_from(&resp).is_none());
    assert_eq!(
        json_of(resp).await["refresh_token"].as_str().unwrap().len(),
        64
    );
}

#[tokio::test]
async fn auth_endpoints_require_the_csrf_header_and_an_allowed_origin() {
    let h = harness().await;
    assert_eq!(
        h.login("alice", PW, &[]).await.status(),
        S::FORBIDDEN,
        "no client header"
    );
    assert_eq!(
        h.login("alice", PW, &[CLIENT, ("origin", "https://evil.example")])
            .await
            .status(),
        S::FORBIDDEN,
        "cross-origin"
    );
    let same = format!("http://{}", h.addr);
    assert_eq!(
        h.login("alice", PW, &[CLIENT, ("origin", &same)])
            .await
            .status(),
        S::OK
    );
    for path in ["/v1/auth/refresh", "/v1/auth/logout", "/v1/auth/pair"] {
        let r = h.post(path, json!({}), &[]).await;
        assert_eq!(r.status(), S::FORBIDDEN, "{path} without client header");
    }
}

#[tokio::test]
async fn failures_are_generic_and_lock_out() {
    let h = harness().await;
    let wrong = h.login("alice", "not the password", &[CLIENT]).await;
    assert_eq!(wrong.status(), S::UNAUTHORIZED);
    let wrong_body = json_of(wrong).await;
    let unknown = json_of(h.login("nobody", PW, &[CLIENT]).await).await;
    assert_eq!(
        wrong_body, unknown,
        "unknown user and wrong password look identical"
    );

    for _ in 0..5 {
        h.login("alice", "not the password", &[CLIENT]).await;
    }
    let locked = h.login("alice", PW, &[CLIENT]).await;
    assert_eq!(locked.status(), S::TOO_MANY_REQUESTS);
    assert!(locked.headers().contains_key("retry-after"));
}

#[tokio::test]
async fn malformed_login_requests_are_rejected() {
    let h = harness().await;
    let cases = [
        json!({ "username": "alice", "password": PW }),
        json!({ "username": "alice", "password": PW, "device_name": "x", "admin": true }),
        json!({ "username": "alice", "password": PW, "device_name": "x", "refresh": "url" }),
        json!({ "username": "alice", "password": PW, "device_name": "line\nbreak" }),
        json!("not an object"),
    ];
    for body in cases {
        let r = h.post("/v1/auth/login", body.clone(), &[CLIENT]).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "{body}");
    }
    let huge = json!({ "username": "alice", "password": "x".repeat(10_000), "device_name": "x" });
    assert_eq!(
        h.post("/v1/auth/login", huge, &[CLIENT]).await.status(),
        S::PAYLOAD_TOO_LARGE
    );
}

// ---------------------------------------------------------------------------
// Refresh, logout, revocation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refresh_rotates_the_cookie_and_reuse_revokes_the_device() {
    let mut h = harness().await;
    let resp = h.login("alice", PW, &[CLIENT]).await;
    let cookie1 = cookie_value(&cookie_from(&resp).unwrap());
    let token = json_of(resp).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // A live player session on this device.
    let mut ws = h.dial().await;
    ws.send(Message::Text(token.clone().into())).await.unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;

    let r = h
        .post(
            "/v1/auth/refresh",
            Value::Null,
            &[CLIENT, ("cookie", &cookie1)],
        )
        .await;
    // An empty JSON body ("null") is not a valid refresh request body.
    assert_eq!(r.status(), S::BAD_REQUEST);
    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/refresh"))
        .header(CLIENT.0, CLIENT.1)
        .header("cookie", &cookie1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    let cookie2 = cookie_value(&cookie_from(&r).unwrap());
    assert_ne!(cookie1, cookie2);

    // Replaying the old cookie: theft detected, device revoked, session ended.
    let replay = reqwest::Client::new()
        .post(h.url("/v1/auth/refresh"))
        .header(CLIENT.0, CLIENT.1)
        .header("cookie", &cookie1)
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), S::UNAUTHORIZED);
    expect_close(&mut ws, CloseCode::Policy).await;
    let again = reqwest::Client::new()
        .post(h.url("/v1/auth/refresh"))
        .header(CLIENT.0, CLIENT.1)
        .header("cookie", &cookie2)
        .send()
        .await
        .unwrap();
    assert_eq!(
        again.status(),
        S::UNAUTHORIZED,
        "the whole device is revoked"
    );
}

#[tokio::test]
async fn logout_revokes_the_device_and_clears_the_cookie() {
    let mut h = harness().await;
    let token = json_of(h.login("alice", PW, &[CLIENT]).await).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = h.dial().await;
    ws.send(Message::Text(token.clone().into())).await.unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;

    let r = reqwest::Client::new()
        .post(h.url("/v1/auth/logout"))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert!(cookie_from(&r).unwrap().contains("Max-Age=0"));
    expect_close(&mut ws, CloseCode::Policy).await;
    let me = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), S::UNAUTHORIZED);
}

#[tokio::test]
async fn me_requires_a_valid_bearer_token() {
    let h = harness().await;
    for auth in [
        None,
        Some("Bearer nope"),
        Some("Basic abc"),
        Some(h.tok_a.as_str()),
    ] {
        let mut req = reqwest::Client::new().get(h.url("/v1/me"));
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        assert_eq!(
            req.send().await.unwrap().status(),
            S::UNAUTHORIZED,
            "{auth:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Admin API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_api_requires_the_secret_and_is_not_on_the_public_listener() {
    let h = harness().await;
    let (base, secret) = h.admin().await;
    let c = reqwest::Client::new();
    assert_eq!(
        c.get(format!("{base}/admin/v1/users"))
            .send()
            .await
            .unwrap()
            .status(),
        S::FORBIDDEN
    );
    let wrong = c
        .get(format!("{base}/admin/v1/users"))
        .bearer_auth("s".repeat(63) + "t")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), S::FORBIDDEN);
    let proxied = c
        .get(format!("{base}/admin/v1/users"))
        .bearer_auth(&secret)
        .header("x-forwarded-for", "203.0.113.5")
        .send()
        .await
        .unwrap();
    assert_eq!(
        proxied.status(),
        S::FORBIDDEN,
        "a proxied request must never reach the admin API"
    );
    let ok = c
        .get(format!("{base}/admin/v1/users"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), S::OK);
    // Not routed on the public listener at all, secret or not.
    let public = c
        .get(h.url("/admin/v1/users"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(public.status(), S::NOT_FOUND);
}

#[tokio::test]
async fn admin_api_refuses_non_loopback_peers() {
    let h = harness().await;
    let secret = "s".repeat(64);
    let app = admin::router(h.gw.clone(), secret.clone());
    let mut req = axum::http::Request::get("/admin/v1/users")
        .header("authorization", format!("Bearer {secret}"))
        .body(axum::body::Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "203.0.113.5:5555".parse::<SocketAddr>().unwrap(),
    ));
    assert_eq!(
        app.oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_manages_users_devices_and_pairing() {
    let mut h = harness().await;
    let (base, secret) = h.admin().await;
    let c = reqwest::Client::new();
    let admin =
        |m: reqwest::Method, path: &str| c.request(m, format!("{base}{path}")).bearer_auth(&secret);

    // Create a user; weak passwords and bad roles are refused.
    let bad = admin(reqwest::Method::POST, "/admin/v1/users")
        .json(&json!({ "username": "carol", "password": "short", "role": "listener" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), S::BAD_REQUEST);
    let bad_role = admin(reqwest::Method::POST, "/admin/v1/users")
        .json(&json!({ "username": "carol", "password": PW, "role": "root" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_role.status(), S::BAD_REQUEST);
    let created = admin(reqwest::Method::POST, "/admin/v1/users")
        .json(&json!({ "username": "carol", "password": PW, "role": "listener" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), S::CREATED);

    // Pairing: a code the new app redeems for its own device.
    let code = json_of(
        admin(reqwest::Method::POST, "/admin/v1/users/carol/pairing")
            .send()
            .await
            .unwrap(),
    )
    .await["code"]
        .as_str()
        .unwrap()
        .to_string();
    let paired = h
        .post(
            "/v1/auth/pair",
            json!({ "code": code, "device_name": "Carol's phone" }),
            &[CLIENT],
        )
        .await;
    assert_eq!(paired.status(), S::OK);
    let paired = json_of(paired).await;
    assert!(
        paired["refresh_token"].is_string(),
        "apps get the refresh token in the body"
    );
    let again = h
        .post(
            "/v1/auth/pair",
            json!({ "code": code, "device_name": "x" }),
            &[CLIENT],
        )
        .await;
    assert_eq!(again.status(), S::UNAUTHORIZED, "single use");

    // Devices are listed; revoking one ends its live session.
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;
    let devices = json_of(
        admin(reqwest::Method::GET, "/admin/v1/devices")
            .send()
            .await
            .unwrap(),
    )
    .await;
    let alice_device = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["username"] == "alice")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let revoked = admin(
        reqwest::Method::DELETE,
        &format!("/admin/v1/devices/{alice_device}"),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(revoked.status(), S::NO_CONTENT);
    expect_close(&mut ws, CloseCode::Policy).await;
    assert!(h.accounts.authenticate(&h.tok_a).is_none());

    // Disabling a user ends their sessions too.
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_b.clone().into()))
        .await
        .unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;
    let disabled = admin(reqwest::Method::POST, "/admin/v1/users/bob/disabled")
        .json(&json!({ "disabled": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), S::NO_CONTENT);
    expect_close(&mut ws, CloseCode::Policy).await;
    assert_eq!(
        h.login("bob", PW, &[CLIENT]).await.status(),
        S::UNAUTHORIZED
    );

    // The audit log records it all.
    let audit = json_of(
        admin(reqwest::Method::GET, "/admin/v1/audit?limit=100")
            .send()
            .await
            .unwrap(),
    )
    .await;
    let events: Vec<&str> = audit["audit"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    for e in [
        "user.created",
        "pairing.created",
        "pairing.ok",
        "device.revoked",
        "user.disabled",
    ] {
        assert!(events.contains(&e), "audit missing {e}: {events:?}");
    }
}

#[tokio::test]
async fn rate_limit_key_uses_forwarded_address_only_from_trusted_proxies() {
    let h = harness().await;
    // Five failures from one forwarded client address (via the loopback "proxy")...
    for _ in 0..5 {
        h.login(
            "nobody",
            "wrong password!",
            &[
                CLIENT,
                ("x-forwarded-proto", "https"),
                ("x-forwarded-for", "198.51.100.7"),
            ],
        )
        .await;
    }
    // ...lock that client out...
    let locked = h
        .login(
            "alice",
            PW,
            &[
                CLIENT,
                ("x-forwarded-proto", "https"),
                ("x-forwarded-for", "198.51.100.7"),
            ],
        )
        .await;
    assert_eq!(locked.status(), S::TOO_MANY_REQUESTS);
    // ...but not a different client behind the same proxy.
    let other = h
        .login(
            "alice",
            PW,
            &[
                CLIENT,
                ("x-forwarded-proto", "https"),
                ("x-forwarded-for", "198.51.100.8"),
            ],
        )
        .await;
    assert_eq!(other.status(), S::OK);
}
