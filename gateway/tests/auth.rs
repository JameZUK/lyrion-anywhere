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
        json!("not an object"),
    ];
    for body in cases {
        let r = h.post("/v1/auth/login", body.clone(), &[CLIENT]).await;
        assert_eq!(r.status(), S::BAD_REQUEST, "{body}");
    }
    // Hostile device names are cleaned, not refused (finding L7).
    for (name, want) in [
        ("line\nbreak", "line break"),
        ("<img src=x onerror=alert(1)>", "img srcx onerroralert(1)"),
        ("\u{202e}\u{200b}", "Browser"),
    ] {
        let r = h
            .post(
                "/v1/auth/login",
                json!({ "username": "alice", "password": PW, "device_name": name }),
                &[CLIENT],
            )
            .await;
        assert_eq!(r.status(), S::OK, "{name:?}");
        assert_eq!(json_of(r).await["device"]["name"], want, "{name:?}");
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
    // The new token is used (by whoever holds it)…
    let refresh = |c: String| {
        let url = h.url("/v1/auth/refresh");
        async move {
            reqwest::Client::new()
                .post(url)
                .header(CLIENT.0, CLIENT.1)
                .header("cookie", c)
                .send()
                .await
                .unwrap()
        }
    };
    let r = refresh(cookie2.clone()).await;
    assert_eq!(r.status(), S::OK);
    let cookie3 = cookie_value(&cookie_from(&r).unwrap());

    // …and the old cookie comes back: two parties hold tokens, so it was stolen. The device
    // is revoked and its session ended.
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
        .header("cookie", &cookie3)
        .send()
        .await
        .unwrap();
    assert_eq!(
        again.status(),
        S::UNAUTHORIZED,
        "the whole device is revoked"
    );
}

/// Regression: a phone suspended mid-refresh never received the new cookie and presented the
/// old one again. That looked like theft, so the device was revoked and the user signed out.
#[tokio::test]
async fn a_lost_refresh_response_does_not_sign_the_device_out() {
    let mut h = harness().await;
    let resp = h.login("alice", PW, &[CLIENT]).await;
    let cookie1 = cookie_value(&cookie_from(&resp).unwrap());
    let token = json_of(resp).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = h.dial().await;
    ws.send(Message::Text(token.into())).await.unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;

    let refresh = |c: String| {
        let url = h.url("/v1/auth/refresh");
        async move {
            reqwest::Client::new()
                .post(url)
                .header(CLIENT.0, CLIENT.1)
                .header("cookie", c)
                .send()
                .await
                .unwrap()
        }
    };
    let lost = refresh(cookie1.clone()).await; // the browser never sees this response
    assert_eq!(lost.status(), S::OK);
    let retry = refresh(cookie1.clone()).await;
    assert_eq!(retry.status(), S::OK, "the retry is accepted");
    let cookie2 = cookie_value(&cookie_from(&retry).unwrap());
    assert!(cookie_from(&retry).unwrap().contains("Max-Age=2592000"));
    // The device and its player session carry on.
    assert_eq!(refresh(cookie2).await.status(), S::OK);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), ws.next())
            .await
            .is_err(),
        "the player session stays open"
    );
}

/// Regression: a thief replaying a copied refresh token within the grace window got the
/// device, and the real client's token then looked merely invalid. Changed deliberately
/// (finding L4): replacements issued within the grace window are siblings, all valid, so
/// concurrent refreshes don't revoke the device. The first sibling used supersedes the
/// others (their access tokens die at once), and a superseded one coming back is reuse:
/// the device is revoked and its live session ended.
#[tokio::test]
async fn a_replay_within_the_grace_window_is_caught_when_the_owner_returns() {
    let mut h = harness().await;
    let resp = h
        .post(
            "/v1/auth/login",
            json!({ "username": "alice", "password": PW, "device_name": "Phone", "refresh": "body" }),
            &[CLIENT],
        )
        .await;
    let body = json_of(resp).await;
    let stolen = body["refresh_token"].as_str().unwrap().to_string();
    let access = body["access_token"].as_str().unwrap().to_string();
    let mut ws = h.dial().await;
    ws.send(Message::Text(access.into())).await.unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;

    let refresh = |t: &str| h.post("/v1/auth/refresh", json!({ "refresh_token": t }), &[CLIENT]);
    let owner = json_of(refresh(&stolen).await).await; // the owner refreshes: T → S
    let owner_next = owner["refresh_token"].as_str().unwrap().to_string();
    let thief = refresh(&stolen).await; // a copy of T, inside the grace window
    assert_eq!(thief.status(), S::OK);
    let thief = json_of(thief).await;
    let thief_access = thief["access_token"].as_str().unwrap().to_string();
    let thief_next = thief["refresh_token"].as_str().unwrap().to_string();
    // The owner comes back with S: it still works, and the thief's access dies.
    assert_eq!(refresh(&owner_next).await.status(), S::OK);
    let me = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&thief_access)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), S::UNAUTHORIZED, "the thief is signed out");
    // The thief tries its (superseded) token: two parties held tokens for this device.
    let caught = refresh(&thief_next).await;
    assert_eq!(caught.status(), S::UNAUTHORIZED);
    assert_eq!(
        caught.headers().get("clear-site-data").unwrap(),
        "\"cache\""
    );
    expect_close(&mut ws, CloseCode::Policy).await;
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
async fn admin_lists_plugins_and_reserves_them_for_admins() {
    let h = harness().await;
    let (base, secret) = h.admin().await;
    let c = reqwest::Client::new();
    let get = || {
        c.get(format!("{base}/admin/v1/plugins"))
            .bearer_auth(&secret)
    };
    let set = |body: Value| {
        c.post(format!("{base}/admin/v1/plugins"))
            .bearer_auth(&secret)
            .json(&body)
    };
    let list = json_of(get().send().await.unwrap()).await;
    let plugins = list["plugins"].as_array().unwrap();
    let find = |cmd: &str| plugins.iter().find(|p| p["command"] == cmd).cloned();
    assert_eq!(find("spotty").unwrap()["kind"], "app");
    assert_eq!(find("bbcsounds").unwrap()["kind"], "radio");
    let sp = find("someplugin").expect("home-menu plugin listed");
    assert_eq!(sp["name"], "Some Plugin");
    assert_eq!(sp["admin_only"], false);
    assert!(
        find("crossfadesettings").is_none(),
        "settings entries are not plugins"
    );
    let group = plugins
        .iter()
        .find(|p| p["name"] == "Some Group")
        .expect("grouped plugin");
    assert_eq!(
        group["commands"],
        json!(["groupchooser", "groupmix"]),
        "one entry for the group's commands"
    );
    assert_eq!(
        plugins.iter().filter(|p| p["name"] == "Some Group").count(),
        1
    );
    assert!(plugins.iter().all(|p| {
        !["favorites", "browselibrary", "anywhere"].contains(&p["command"].as_str().unwrap())
    }));

    assert_eq!(
        set(json!({"admin_only": ["someplugin", "gone"]}))
            .send()
            .await
            .unwrap()
            .status(),
        S::NO_CONTENT
    );
    let list = json_of(get().send().await.unwrap()).await;
    let plugins = list["plugins"].as_array().unwrap();
    assert_eq!(
        plugins
            .iter()
            .find(|p| p["command"] == "someplugin")
            .unwrap()["admin_only"],
        true
    );
    let gone = plugins.iter().find(|p| p["command"] == "gone").unwrap();
    assert_eq!(
        gone["kind"], "not installed",
        "a reserved plugin that is no longer installed still shows"
    );
    for bad in [
        json!({"admin_only": ["a b"]}),
        json!({"admin_only": "x"}),
        json!({"admin_only": [], "x": 1}),
        json!({}),
    ] {
        assert!(
            set(bad.clone())
                .send()
                .await
                .unwrap()
                .status()
                .is_client_error(),
            "{bad}"
        );
    }
    let r = c
        .get(format!("{base}/admin/v1/plugins"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    assert_ne!(
        c.get(h.url("/admin/v1/plugins"))
            .send()
            .await
            .unwrap()
            .status(),
        S::OK
    );
}

#[tokio::test]
async fn admin_sets_the_sign_in_lifetime() {
    let h = harness().await;
    let (base, secret) = h.admin().await;
    let c = reqwest::Client::new();
    let get = || {
        c.get(format!("{base}/admin/v1/settings"))
            .bearer_auth(&secret)
    };
    let set = |body: Value| {
        c.post(format!("{base}/admin/v1/settings"))
            .bearer_auth(&secret)
            .json(&body)
    };
    let s = json_of(get().send().await.unwrap()).await;
    assert_eq!(s["session_days"], 30);
    assert_eq!(
        set(json!({"session_days": 120}))
            .send()
            .await
            .unwrap()
            .status(),
        S::NO_CONTENT
    );
    assert_eq!(
        json_of(get().send().await.unwrap()).await["session_days"],
        120
    );
    // New sign-ins get the new lifetime in their cookie.
    let resp = h.login("alice", PW, &[CLIENT]).await;
    assert!(cookie_from(&resp).unwrap().contains("Max-Age=10368000"));
    // Out of range, wrong types and unknown fields are refused, and change nothing.
    for bad in [
        json!({"session_days": 0}),
        json!({"session_days": 366}),
        json!({"session_days": -1}),
        json!({"session_days": "30"}),
        json!({"session_days": 30, "admin": true}),
        json!({}),
    ] {
        let st = set(bad.clone()).send().await.unwrap().status();
        assert!(st.is_client_error(), "{bad} gave {st}");
    }
    assert_eq!(
        json_of(get().send().await.unwrap()).await["session_days"],
        120
    );
    // Without the admin secret: refused.
    let r = c
        .get(format!("{base}/admin/v1/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    let r = c
        .post(format!("{base}/admin/v1/settings"))
        .bearer_auth("not-the-secret")
        .json(&json!({"session_days": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    // The public listener doesn't serve it at all.
    let r = c.get(h.url("/admin/v1/settings")).send().await.unwrap();
    assert_ne!(r.status(), S::OK);
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

/// Regression: one subscriber holds a whole IPv6 /64, so keying by full address let them
/// rotate addresses past the lockout.
#[tokio::test]
async fn ipv6_clients_are_rate_limited_per_64() {
    let h = harness().await;
    fn via(ip: &str) -> [(&str, &str); 3] {
        [
            CLIENT,
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", ip),
        ]
    }
    for i in 0..5 {
        let ip = format!("2001:db8:5:6::{:x}", i + 1);
        h.login("nobody", "wrong password!", &via(&ip)).await;
    }
    let same_64 = h.login("alice", PW, &via("2001:db8:5:6:ffff::1")).await;
    assert_eq!(same_64.status(), S::TOO_MANY_REQUESTS);
    let other_64 = h.login("alice", PW, &via("2001:db8:5:7::1")).await;
    assert_eq!(other_64.status(), S::OK);
}

/// Regression: parallel wrong guesses were all checked before any was counted.
#[tokio::test]
async fn parallel_wrong_logins_are_cut_off_at_the_limit() {
    let h = Arc::new(harness().await);
    let before = h.accounts.password_checks();
    let tasks: Vec<_> = (0..30)
        .map(|_| {
            let h = h.clone();
            tokio::spawn(async move {
                h.login("alice", "wrong password!", &[CLIENT])
                    .await
                    .status()
            })
        })
        .collect();
    let mut statuses = vec![];
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    let unauthorised = statuses.iter().filter(|s| **s == S::UNAUTHORIZED).count();
    assert!(unauthorised <= 5, "{statuses:?}");
    assert!(
        statuses.iter().all(|s| matches!(
            *s,
            S::UNAUTHORIZED | S::TOO_MANY_REQUESTS | S::SERVICE_UNAVAILABLE
        )),
        "{statuses:?}"
    );
    assert!(h.accounts.password_checks() - before <= 5);
    assert_eq!(
        h.login("alice", PW, &[CLIENT]).await.status(),
        S::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn an_admin_password_reset_signs_the_user_out() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a.clone()).await;
    let _lms = h.lms.accept().await;
    let (base, secret) = h.admin().await;
    let r = reqwest::Client::new()
        .post(format!("{base}/admin/v1/users/alice/password"))
        .bearer_auth(&secret)
        .json(&json!({ "password": "a brand new password" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    expect_close(&mut ws, CloseCode::Policy).await;
    let me = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), S::UNAUTHORIZED);
    // Bob is untouched.
    let me = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_b)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), S::OK);
}

// ---------------------------------------------------------------------------
// Two-step verification (TOTP)
// ---------------------------------------------------------------------------

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

async fn bearer_post(
    h: &Harness,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(h.url(path))
        .header(CLIENT.0, CLIENT.1)
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    req.send().await.unwrap()
}

/// Turn two-step on for alice through the API; returns the base32 secret.
async fn enrol_alice(h: &Harness) -> String {
    let token = h.tok_a.clone();
    let setup = bearer_post(
        h,
        "/v1/account/totp/setup",
        &token,
        Some(json!({ "password": PW })),
    )
    .await;
    assert_eq!(setup.status(), S::OK);
    assert_eq!(setup.headers()["cache-control"], "no-store");
    let setup = json_of(setup).await;
    let secret = setup["secret"].as_str().unwrap().to_string();
    assert!(setup["uri"].as_str().unwrap().contains(&secret));
    let wrong = bearer_post(
        h,
        "/v1/account/totp/confirm",
        &token,
        Some(json!({ "code": "000000" })),
    )
    .await;
    assert_eq!(wrong.status(), S::BAD_REQUEST);
    let ok = bearer_post(
        h,
        "/v1/account/totp/confirm",
        &token,
        Some(json!({ "code": code_for(&secret, 0) })),
    )
    .await;
    assert_eq!(ok.status(), S::NO_CONTENT);
    secret
}

#[tokio::test]
async fn totp_login_flow() {
    let h = harness().await;
    let secret = enrol_alice(&h).await;
    let account = reqwest::Client::new()
        .get(h.url("/v1/account"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(json_of(account).await["totp_enabled"], true);

    // Password alone: the client is told to ask for the code.
    let r = h.login("alice", PW, &[CLIENT]).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert_eq!(json_of(r).await["totp_required"], true);

    // A wrong password never mentions two-step.
    let r = h.login("alice", "wrong password!", &[CLIENT]).await;
    assert!(json_of(r).await.get("totp_required").is_none());

    let with_code = |code: String| json!({ "username": "alice", "password": PW, "totp": code, "device_name": "Test browser" });
    let r = h
        .post("/v1/auth/login", with_code("000000".into()), &[CLIENT])
        .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    let code = code_for(&secret, 1);
    let r = h
        .post("/v1/auth/login", with_code(code.clone()), &[CLIENT])
        .await;
    assert_eq!(r.status(), S::OK);
    let r = h.post("/v1/auth/login", with_code(code), &[CLIENT]).await;
    assert_eq!(r.status(), S::UNAUTHORIZED, "codes are single use");
}

#[tokio::test]
async fn totp_endpoints_require_auth_and_csrf_header() {
    let h = harness().await;
    for path in [
        "/v1/account/totp/setup",
        "/v1/account/totp/confirm",
        "/v1/account/totp/disable",
    ] {
        let no_auth = reqwest::Client::new()
            .post(h.url(path))
            .header(CLIENT.0, CLIENT.1)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(no_auth.status(), S::UNAUTHORIZED, "{path} without token");
        let no_header = reqwest::Client::new()
            .post(h.url(path))
            .bearer_auth(&h.tok_a)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            no_header.status(),
            S::FORBIDDEN,
            "{path} without client header"
        );
    }
    let r = reqwest::Client::new()
        .get(h.url("/v1/account"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::UNAUTHORIZED);
}

/// Regression: an access token alone (e.g. one lifted from a shared browser) could turn on
/// two-step with the thief's authenticator, locking the owner out.
#[tokio::test]
async fn totp_setup_needs_the_password() {
    let h = harness().await;
    let token = h.tok_a.clone();
    for (body, want) in [
        (None, S::BAD_REQUEST),
        (Some(json!({})), S::BAD_REQUEST),
        (
            Some(json!({ "password": "wrong password!" })),
            S::UNAUTHORIZED,
        ),
        (Some(json!({ "password": PW, "extra": 1 })), S::BAD_REQUEST),
    ] {
        let r = bearer_post(&h, "/v1/account/totp/setup", &token, body.clone()).await;
        assert_eq!(r.status(), want, "{body:?}");
        let v: Value = r.json().await.unwrap();
        assert!(v.get("secret").is_none(), "{v}");
    }
    // Nothing was started, so there is nothing to confirm.
    let r = bearer_post(
        &h,
        "/v1/account/totp/confirm",
        &token,
        Some(json!({ "code": "123456" })),
    )
    .await;
    assert_eq!(r.status(), S::BAD_REQUEST);
    enrol_alice(&h).await;
}

#[tokio::test]
async fn totp_disable_needs_password_and_code() {
    let h = harness().await;
    let secret = enrol_alice(&h).await;
    let token = h.tok_a.clone();
    let bad = bearer_post(
        &h,
        "/v1/account/totp/disable",
        &token,
        Some(json!({ "password": "wrong password!", "code": code_for(&secret, 1) })),
    )
    .await;
    assert_eq!(bad.status(), S::UNAUTHORIZED);
    let ok = bearer_post(
        &h,
        "/v1/account/totp/disable",
        &token,
        Some(json!({ "password": PW, "code": code_for(&secret, 1) })),
    )
    .await;
    assert_eq!(ok.status(), S::NO_CONTENT);
    assert_eq!(h.login("alice", PW, &[CLIENT]).await.status(), S::OK);
}

#[tokio::test]
async fn admin_can_reset_a_lost_authenticator() {
    let h = harness().await;
    enrol_alice(&h).await;
    let (base, secret) = h.admin().await;
    let r = reqwest::Client::new()
        .post(format!("{base}/admin/v1/users/alice/totp/reset"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert_eq!(h.login("alice", PW, &[CLIENT]).await.status(), S::OK);
    let users = json_of(
        reqwest::Client::new()
            .get(format!("{base}/admin/v1/users"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert!(
        users["users"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["totp_enabled"] == false)
    );
}

#[tokio::test]
async fn admin_grants_and_removes_home_players() {
    let h = harness().await;
    let (base, secret) = h.admin().await;
    let c = reqwest::Client::new();
    let grant = |mac: &str| {
        c.post(format!("{base}/admin/v1/users/alice/players"))
            .bearer_auth(&secret)
            .json(&json!({ "player": mac }))
            .send()
    };
    assert_eq!(grant("not-a-mac").await.unwrap().status(), S::BAD_REQUEST);
    assert_eq!(
        grant("AA:BB:CC:DD:EE:01").await.unwrap().status(),
        S::NO_CONTENT
    );
    let list = json_of(
        c.get(format!("{base}/admin/v1/users/alice/players"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list["players"], json!(["aa:bb:cc:dd:ee:01"]));
    let principal = h.accounts.authenticate(&h.tok_a).unwrap();
    assert!(
        h.accounts
            .player_scope(&principal)
            .unwrap()
            .allows("aa:bb:cc:dd:ee:01")
    );
    let removed = c
        .delete(format!(
            "{base}/admin/v1/users/alice/players/aa:bb:cc:dd:ee:01"
        ))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), S::NO_CONTENT);
    assert!(
        !h.accounts
            .player_scope(&principal)
            .unwrap()
            .allows("aa:bb:cc:dd:ee:01")
    );
    // Without the secret, nothing changes.
    let anon = c
        .post(format!("{base}/admin/v1/users/alice/players"))
        .json(&json!({ "player": "aa:bb:cc:dd:ee:09" }))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), S::FORBIDDEN);
}

#[tokio::test]
async fn admin_post_forms_of_deletes_work() {
    let mut h = harness().await;
    let (base, secret) = h.admin().await;
    let c = reqwest::Client::new();
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;
    let devices = json_of(
        c.get(format!("{base}/admin/v1/devices"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap(),
    )
    .await;
    let id = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["username"] == "alice")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let r = c
        .post(format!("{base}/admin/v1/devices/{id}/revoke"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    expect_close(&mut ws, CloseCode::Policy).await;
    c.post(format!("{base}/admin/v1/users/alice/players"))
        .bearer_auth(&secret)
        .json(&json!({"player": "aa:bb:cc:dd:ee:01"}))
        .send()
        .await
        .unwrap();
    let r = c
        .post(format!(
            "{base}/admin/v1/users/alice/players/aa:bb:cc:dd:ee:01/remove"
        ))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert!(h.accounts.players_granted("alice").unwrap().is_empty());
    let r = c
        .post(format!("{base}/admin/v1/users/bob/delete"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert!(
        h.accounts
            .list_users()
            .unwrap()
            .iter()
            .all(|u| u.username != "bob")
    );
    let anon = c
        .post(format!("{base}/admin/v1/users/alice/delete"))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), S::FORBIDDEN);
}

// --- device diagnostics ----------------------------------------------------------------

async fn post_diagnostics(h: &Harness, token: Option<&str>, body: String) -> reqwest::Response {
    let mut r = reqwest::Client::new()
        .post(h.url("/v1/diagnostics"))
        .header("content-type", "application/json")
        .body(body);
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    r.send().await.unwrap()
}

async fn device_id(admin: &str, secret: &str, user: &str) -> i64 {
    let devices: Value = reqwest::Client::new()
        .get(format!("{admin}/admin/v1/devices"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["username"] == user)
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

async fn read_diagnostics(admin: &str, secret: &str, id: i64) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{admin}/admin/v1/devices/{id}/diagnostics"))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn devices_report_diagnostics_that_admins_can_read() {
    let h = harness().await;
    let (admin, secret) = h.admin().await;
    let report = json!({
        "playback": {"mode": "media", "mime": "audio/mp4; codecs=\"fLaC\"", "reason": "iPad"},
        "capabilities": {"managedMediaSource": true},
        "log": ["16:00:01  connected", "16:00:05  page hidden"]
    });
    let r = post_diagnostics(&h, Some(&h.tok_a), report.to_string()).await;
    assert_eq!(r.status(), S::NO_CONTENT);
    let id = device_id(&admin, &secret, "alice").await;
    let got: Value = read_diagnostics(&admin, &secret, id)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(got["report"], report);
    assert!(got["updated_at"].as_i64().unwrap() > 0);
    // Bob has none, and can't see Alice's (no such public endpoint at all).
    let bob = device_id(&admin, &secret, "bob").await;
    assert_eq!(
        read_diagnostics(&admin, &secret, bob).await.status(),
        S::NOT_FOUND
    );
    let public = reqwest::Client::new()
        .get(h.url(&format!("/admin/v1/devices/{id}/diagnostics")))
        .bearer_auth(&h.tok_b)
        .send()
        .await
        .unwrap();
    assert_eq!(public.status(), S::NOT_FOUND);
}

#[tokio::test]
async fn diagnostics_reports_are_authenticated_bounded_and_sanitised() {
    let h = harness().await;
    let (admin, secret) = h.admin().await;
    assert_eq!(
        post_diagnostics(&h, None, "{}".into()).await.status(),
        S::UNAUTHORIZED
    );
    assert_eq!(
        post_diagnostics(&h, Some("forged"), "{}".into())
            .await
            .status(),
        S::UNAUTHORIZED
    );
    let big = json!({ "x": "y".repeat(40 * 1024) }).to_string();
    assert_eq!(
        post_diagnostics(&h, Some(&h.tok_a), big).await.status(),
        S::PAYLOAD_TOO_LARGE
    );
    for bad in ["not json", "[1,2]", "\"string\""] {
        assert_eq!(
            post_diagnostics(&h, Some(&h.tok_a), bad.into())
                .await
                .status(),
            S::BAD_REQUEST,
            "{bad}"
        );
    }
    let hostile = json!({
        "<img src=x onerror=alert(1)>": 1,
        "log": ["ok", "a\u{1b}[31mred\u{0}"],
        "deep": {"a": {"b": {"c": {"d": {"e": 1}}}}}
    });
    assert_eq!(
        post_diagnostics(&h, Some(&h.tok_a), hostile.to_string())
            .await
            .status(),
        S::NO_CONTENT
    );
    let id = device_id(&admin, &secret, "alice").await;
    let got: Value = read_diagnostics(&admin, &secret, id)
        .await
        .json()
        .await
        .unwrap();
    let s = got["report"].to_string();
    assert!(
        !s.contains("<img") && !s.contains('\u{1b}') && !s.contains('\u{0}'),
        "{s}"
    );
    assert_eq!(got["report"]["deep"]["a"]["b"]["c"]["d"], Value::Null);
}

#[tokio::test]
async fn diagnostics_are_rate_limited_and_removed_with_the_device() {
    let h = harness().await;
    let (admin, secret) = h.admin().await;
    assert_eq!(
        post_diagnostics(&h, Some(&h.tok_a), r#"{"n":1}"#.into())
            .await
            .status(),
        S::NO_CONTENT
    );
    // Too soon: accepted, not stored.
    assert_eq!(
        post_diagnostics(&h, Some(&h.tok_a), r#"{"n":2}"#.into())
            .await
            .status(),
        S::ACCEPTED
    );
    let id = device_id(&admin, &secret, "alice").await;
    let got: Value = read_diagnostics(&admin, &secret, id)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(got["report"]["n"], 1);
    // Revoking the device deletes its diagnostics.
    let r = reqwest::Client::new()
        .post(format!("{admin}/admin/v1/devices/{id}/revoke"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert_eq!(
        read_diagnostics(&admin, &secret, id).await.status(),
        S::NOT_FOUND
    );
}
