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

/// Regression (issue #37): LMS's own responses name remote artwork as
/// `/imageproxy/<escaped URL>/image.<ext>`, and asked for exactly that LMS answers with a
/// redirect to the external URL (as the fake does), which the gateway never follows. Such a
/// request must reach LMS as `image`, so that the image itself comes back.
#[tokio::test]
async fn unsized_proxied_images_are_fetched_without_a_redirect() {
    let h = harness().await;
    let r = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(&json!({ "cmd": ["favorites", "items", 0, 10] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    // The fake behaves as LMS does: an unsized `image.jpg` is a redirect, not an image.
    let direct = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!(
            "http://{}/imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image.jpg",
            h.lms.http_addr
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(direct.status(), S::MOVED_PERMANENTLY);
    assert_eq!(
        direct.headers()["location"],
        "https://cdn.example.com/a.jpg"
    );

    let seen = h.lms.http_paths().len();
    let r = h
        .art(
            Some(&h.tok_a),
            "imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image.jpg",
        )
        .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"IMAGEDATA");
    let paths = h.lms.http_paths();
    assert_eq!(
        paths[seen..],
        ["/imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image"],
        "{paths:?}"
    );
    // A sized request goes to LMS as asked.
    let r = h
        .art(
            Some(&h.tok_a),
            "imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image_100x100.jpg",
        )
        .await;
    assert_eq!(r.status(), S::OK);
    assert!(
        h.lms
            .http_paths()
            .last()
            .is_some_and(|p| p.ends_with("/image_100x100.jpg")),
        "{:?}",
        h.lms.http_paths()
    );
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
    // The listener connects before it subscribes; wait for the subscription itself.
    wait_for_async(|| async {
        h.lms
            .rec
            .lock()
            .unwrap()
            .cli_in
            .concat()
            .contains("listen 1")
    })
    .await;

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
async fn quiet_events_socket_is_kept_alive_with_pings() {
    // Proxies close idle sockets (Cloudflare after 100 s); the gateway must ping.
    let h = harness_with(|c| c.ws_keepalive = std::time::Duration::from_millis(100)).await;
    let mut ws = h.events(&h.tok_a).await;
    for _ in 0..2 {
        match next(&mut ws).await {
            Message::Ping(_) => {}
            m => panic!("expected a keepalive ping, got {m:?}"),
        }
    }
    // Pongs from the client are accepted, not treated as unexpected messages.
    ws.send(Message::Pong(Default::default())).await.unwrap();
    assert!(
        matches!(next(&mut ws).await, Message::Ping(_)),
        "socket still open"
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

/// Regression (review low): the events socket limited messages to 1 KiB but not frames, so
/// a socket not yet signed in could announce a frame of 16 MiB and the gateway would wait
/// to read it all. A frame over the limit is refused as soon as its header arrives.
#[tokio::test]
async fn an_oversize_frame_on_the_events_socket_is_refused_at_its_header() {
    let h = harness_with(|c| c.auth_timeout = Duration::from_secs(30)).await;
    let mut s = TcpStream::connect(h.addr).await.unwrap();
    s.write_all(
        b"GET /v1/events HTTP/1.1\r\nhost: test\r\nupgrade: websocket\r\nconnection: upgrade\r\n\
          sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\n\r\n",
    )
    .await
    .unwrap();
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut buf).await.unwrap();
        assert!(n > 0);
        head.extend_from_slice(&buf[..n]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"));
    // A masked text frame announcing 1 MiB, and nothing of it.
    let mut frame = vec![0x81, 0x80 | 127];
    frame.extend_from_slice(&(1024u64 * 1024).to_be_bytes());
    frame.extend_from_slice(&[1, 2, 3, 4]);
    s.write_all(&frame).await.unwrap();
    // The gateway gives up on the socket at once: well before the sign-in time limit.
    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the socket waited for the frame's body");
}

/// Regression (review low): revoking a device left its open event sockets running until
/// their next periodic check, up to 30 s (SECURITY.md says "at once"). Told of the
/// revocation, they close at once; other devices' sockets stay.
#[tokio::test]
async fn a_revoked_devices_event_socket_closes_at_once() {
    let h = harness().await;
    let mut a = h.events(&h.tok_a).await;
    let mut b = h.events(&h.tok_b).await;
    assert!(h.accounts.revoke_device(h.dev_a).unwrap());
    h.gw.recheck_event_sockets(&[h.dev_a]);
    let t = std::time::Instant::now();
    expect_close(&mut a, CloseCode::Policy).await;
    assert!(t.elapsed() < Duration::from_secs(2));
    // Bob's socket is untouched (and still answers a recheck with nothing to say).
    h.gw.rescope_event_sockets();
    let quiet = tokio::time::timeout(Duration::from_millis(300), b.next()).await;
    assert!(quiet.is_err(), "bob's socket got {quiet:?}");
}

/// Regression (review low): taking a player away from a user left their open event
/// sockets reporting it until their next periodic check. Told of the change, they stop at
/// once, and go on reporting the players the user may still control.
#[tokio::test]
async fn an_ungranted_player_drops_out_of_open_event_sockets_at_once() {
    let h = harness().await;
    let shared = "aa:bb:cc:dd:ee:02";
    h.accounts.grant_player("alice", shared).unwrap();
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let mut ws = h.events(&h.tok_a).await;
    wait_for_async(|| async {
        h.lms
            .rec
            .lock()
            .unwrap()
            .cli_in
            .concat()
            .contains("listen 1")
    })
    .await;
    let esc = |m: &str| m.replace(':', "%3A");
    h.lms
        .cli_send(&format!("{} mixer volume 10", esc(shared)))
        .await;
    assert_eq!(event_json(&mut ws).await["player"], shared);
    h.accounts.ungrant_player("alice", shared).unwrap();
    h.gw.rescope_event_sockets();
    // Give the socket the moment it needs to re-read its permissions.
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.lms
        .cli_send(&format!("{} mixer volume 20", esc(shared)))
        .await;
    h.lms
        .cli_send(&format!("{} mixer volume 30", esc(&mine)))
        .await;
    let next_event = event_json(&mut ws).await;
    assert_eq!(next_event["player"], mine.as_str(), "{next_event}");
    assert_eq!(next_event["event"], json!(["mixer", "volume", "30"]));
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
