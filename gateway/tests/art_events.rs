//! Artwork relay and the events stream, against the fake LMS.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

impl Harness {
    async fn art(&self, token: Option<&str>, path: &str) -> reqwest::Response {
        let mut r = reqwest::Client::new().get(self.url(&format!("/v1/art/{path}")));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        r.send().await.unwrap()
    }

    async fn events(&self, token: &str) -> Ws {
        let mut req = format!("ws://{}/v1/events", self.addr)
            .into_client_request()
            .unwrap();
        let _ = req.headers_mut();
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
}

async fn event_json(ws: &mut Ws) -> Value {
    match next(ws).await {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        m => panic!("{m:?}"),
    }
}

// --- artwork ---------------------------------------------------------------

#[tokio::test]
async fn covers_are_relayed_as_inert_images() {
    let h = harness().await;
    let r = h.art(Some(&h.tok_a), "music/2109213/cover_150x150_o").await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(
        csp.contains("sandbox") && csp.contains("default-src 'none'"),
        "{csp}"
    );
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"IMAGEDATA");
    let svg = h.art(Some(&h.tok_a), "html/images/icon.svg").await;
    assert_eq!(svg.status(), S::OK);
    assert!(
        svg.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
}

#[tokio::test]
async fn art_needs_a_token_and_a_valid_path() {
    let h = harness().await;
    assert_eq!(
        h.art(None, "music/1/cover.jpg").await.status(),
        S::UNAUTHORIZED
    );
    assert_eq!(
        h.art(Some("nope"), "music/1/cover.jpg").await.status(),
        S::UNAUTHORIZED
    );
    for p in [
        "music/1/download",
        "jsonrpc.js",
        "music/1/cover.jpg/../../jsonrpc.js",
        "settings/index.html",
        "plugins/X/settings/basic.html",
        "music/1/cover.jpg?x=1",
        "music/%2e%2e/jsonrpc.js",
        "music/1/cover%2Ejpg",
        "plugins/X/html/%2e%2e/%2e%2e/prefs.png",
    ] {
        assert_eq!(h.art(Some(&h.tok_a), p).await.status(), S::NOT_FOUND, "{p}");
    }
    let paths = h.lms.http_paths();
    assert!(paths.is_empty(), "LMS reached: {paths:?}");
}

#[tokio::test]
async fn non_images_from_lms_are_not_relayed() {
    let h = harness().await;
    assert_eq!(
        h.art(Some(&h.tok_a), "music/666/cover.jpg").await.status(),
        S::BAD_GATEWAY
    );
}

#[tokio::test]
async fn image_proxy_only_fetches_urls_lms_handed_out() {
    let h = harness().await;
    let internal = "imageproxy/http%3A%2F%2F10.0.0.1%2Fadmin/image.png";
    let learned = "imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image_100x100_o";
    assert_eq!(
        h.art(Some(&h.tok_a), internal).await.status(),
        S::NOT_FOUND,
        "SSRF must be refused"
    );
    assert_eq!(
        h.art(Some(&h.tok_a), learned).await.status(),
        S::NOT_FOUND,
        "not handed out yet"
    );
    assert!(h.lms.http_paths().is_empty());

    // LMS mentions the image in a response (its stream URL is stripped on the way out)...
    let r = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(&json!({ "cmd": ["favorites", "items", 0, 10] }))
        .send()
        .await
        .unwrap();
    let body: Value = r.json().await.unwrap();
    assert!(!body.to_string().contains("secret"));
    assert_eq!(
        body["result"]["loop_loop"][0]["image"],
        "https://cdn.example.com/a.jpg"
    );

    // ...so now its image can be fetched through LMS's proxy, and still nothing else.
    assert_eq!(h.art(Some(&h.tok_a), learned).await.status(), S::OK);
    assert_eq!(h.art(Some(&h.tok_a), internal).await.status(), S::NOT_FOUND);
}

#[tokio::test]
async fn current_cover_requires_permission_for_the_player() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let ok = h
        .art(
            Some(&h.tok_a),
            &format!("music/current/cover.jpg?player={mine}"),
        )
        .await;
    assert_eq!(ok.status(), S::OK);
    assert_eq!(ok.headers()["cache-control"], "no-store");
    let theirs = anywhere_core::proto::format_mac(&h.mac_b);
    for q in [
        format!("player={theirs}"),
        "player=aa:bb:cc:dd:ee:01".into(),
        "".into(),
        "player=x".into(),
    ] {
        let r = h
            .art(Some(&h.tok_a), &format!("music/current/cover.jpg?{q}"))
            .await;
        assert_eq!(r.status(), S::NOT_FOUND, "{q}");
    }
}

// --- events ----------------------------------------------------------------

#[tokio::test]
async fn events_are_filtered_to_permitted_players_and_redacted() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let theirs = anywhere_core::proto::format_mac(&h.mac_b);
    let mut ws = h.events(&h.tok_a).await;
    wait_for_async(|| async { h.lms.cli_listener_count().await == 1 }).await;
    assert!(
        h.lms
            .rec
            .lock()
            .unwrap()
            .cli_in
            .concat()
            .contains("listen 1")
    );

    let esc = |m: &str| m.replace(':', "%3A");
    h.lms
        .cli_send(&format!(
            "{} playlist newsong Secret%20Song 1",
            esc(&theirs)
        ))
        .await;
    h.lms
        .cli_send(&format!("{} prefset server audiodir %2Fsrv", esc(&mine)))
        .await;
    h.lms
        .cli_send(&format!(
            "{} playlist open file%3A%2F%2F%2Fsrv%2Fmusic%2Fa.flac",
            esc(&mine)
        ))
        .await;
    h.lms.cli_send("rescan done").await;

    let first = event_json(&mut ws).await;
    assert_eq!(first["player"], mine.as_str());
    assert_eq!(
        first["event"],
        json!(["playlist", "open", ""]),
        "path redacted; other player's and prefset events dropped"
    );
    let second = event_json(&mut ws).await;
    assert_eq!(
        second,
        json!({ "player": null, "event": ["rescan", "done"] })
    );
}

#[tokio::test]
async fn events_socket_requires_a_token_and_closes_on_bad_messages() {
    let h = harness().await;
    let req = format!("ws://{}/v1/events", h.addr)
        .into_client_request()
        .unwrap();
    let (mut ws, _) = connect_async(req).await.unwrap();
    ws.send(Message::Text("nope".into())).await.unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;

    let mut ws = h.events(&h.tok_a).await;
    ws.send(Message::Text("anything".into())).await.unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;

    let cross = format!("ws://{}/v1/events", h.addr)
        .into_client_request()
        .map(|mut r| {
            r.headers_mut()
                .insert("origin", "https://evil.example".parse().unwrap());
            r
        });
    assert!(
        connect_async(cross.unwrap()).await.is_err(),
        "cross-origin upgrade must fail"
    );
}

async fn wait_for_async<F, Fut>(cond: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if cond().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met in time");
}
