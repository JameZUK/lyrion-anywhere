//! Gateway integration tests: tunnel, streams, resume, heartbeats, pacing.

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Reliability: happy path and lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn end_to_end_tunnel_and_stream() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let sid = connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;

    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;

    let frame = strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a));
    lms.write_all(&frame).await.unwrap();
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(b.as_ref(), frame.as_slice(), "strm altered in transit"),
        other => panic!("{other:?}"),
    }

    let resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "audio/flac");
    assert_eq!(resp.bytes().await.unwrap().as_ref(), h.lms.audio.as_slice());
    // The stream, and nothing else of the player's, reached LMS (the gateway's own JSON-RPC
    // commands, such as having LMS proxy internet streams, are not the player's).
    let paths: Vec<String> = h
        .lms
        .http_paths()
        .into_iter()
        .filter(|p| !p.starts_with("/jsonrpc.js"))
        .collect();
    assert_eq!(
        paths,
        [format!("/stream.mp3?player={}", format_mac(&h.mac_a))]
    );

    // Later frames pass through, even when split across WebSocket messages.
    let stat = client_frame(b"STAT", &[0u8; 53]);
    ws.send(Message::Binary(stat[..10].to_vec().into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat[10..].to_vec().into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "STAT"]).await;
}

#[tokio::test]
async fn lms_closing_ends_the_session() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let lms = h.lms.accept().await;
    assert_eq!(h.gw.session_count(), 1);
    drop(lms);
    expect_close(&mut ws, CloseCode::Away).await;
    wait_for(|| h.gw.session_count() == 0).await;
}

#[tokio::test]
async fn abandoned_session_is_released_after_the_grace_period() {
    let mut h = harness_with(|c| c.resume_grace = Duration::from_millis(300)).await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.gw.session_count(), 1, "session held for resume");
    assert_eq!(h.lms.closed(), 0, "LMS connection kept open during grace");
    wait_for(|| h.gw.session_count() == 0).await;
    wait_for(|| h.lms.closed() == 1).await;
}

#[tokio::test]
async fn lms_unreachable_closes_with_try_again() {
    let h = harness_with(|c| {
        c.lms_slimproto = "127.0.0.1:1".into();
        c.dial_timeout = Duration::from_millis(200);
    })
    .await;
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Again).await;
}

// ---------------------------------------------------------------------------
// Security: authentication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn socket_without_token_times_out_and_never_reaches_lms() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    expect_close(&mut ws, CloseCode::Policy).await;
    h.lms.assert_no_conn().await;
}

#[tokio::test]
async fn bad_first_messages_are_refused() {
    type Case = (&'static str, fn(&Harness) -> Message);
    let cases: Vec<Case> = vec![
        ("wrong token", |_| Message::Text("nope".into())),
        ("empty token", |_| Message::Text("".into())),
        ("token as binary", |h| {
            Message::Binary(h.tok_a.as_bytes().to_vec().into())
        }),
        ("near-miss token", |h| {
            Message::Text(format!("{}X", &h.tok_a[..h.tok_a.len() - 1]).into())
        }),
        ("oversize token", |_| Message::Text("A".repeat(4096).into())),
        ("HELO instead of token", |h| {
            Message::Binary(helo(h.mac_a).into())
        }),
        ("token with trailing space", |h| {
            Message::Text(format!("{} ", h.tok_a).into())
        }),
        ("refresh-style JSON without token", |_| {
            Message::Text(r#"{"resume":null}"#.into())
        }),
    ];
    for (name, make) in cases {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        ws.send(make(&h)).await.unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
        h.lms.assert_no_conn().await;
        eprintln!("ok: {name}");
    }
}

#[tokio::test]
async fn cross_origin_upgrade_is_refused() {
    let mut h = harness().await;
    let err = h
        .dial_with(&[("origin", "https://evil.example")])
        .await
        .unwrap_err();
    match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => assert_eq!(resp.status(), 403),
        e => panic!("unexpected error {e}"),
    }
    h.lms.assert_no_conn().await;
    // Same-origin and explicitly allowed origins are fine.
    let host = h.addr.to_string();
    assert!(
        h.dial_with(&[("origin", &format!("http://{host}"))])
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn allowed_origin_is_accepted() {
    let h = harness_with(|c| c.allowed_origins = vec!["https://music.example.com".into()]).await;
    assert!(
        h.dial_with(&[("origin", "https://music.example.com")])
            .await
            .is_ok()
    );
    assert!(
        h.dial_with(&[("origin", "https://music.example.com.evil.example")])
            .await
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// Security: Slimproto enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn helo_enforcement() {
    type Case = (&'static str, fn(&Harness) -> Vec<Message>);
    let cases: Vec<Case> = vec![
        ("other device's MAC", |h| {
            vec![Message::Binary(helo(h.mac_b).into())]
        }),
        ("first frame not HELO", |_| {
            vec![Message::Binary(client_frame(b"STAT", &[0; 53]).into())]
        }),
        ("truncated HELO", |_| {
            vec![Message::Binary(client_frame(b"HELO", &[0; 10]).into())]
        }),
        ("oversize frame", |_| {
            vec![Message::Binary(b"RESP\x00\x10\x00\x00".to_vec().into())]
        }),
        ("text after auth", |_| vec![Message::Text("hello".into())]),
    ];
    for (name, make) in cases {
        let mut h = harness().await;
        let msgs = make(&h);
        let mut ws = h.dial().await;
        connect(&mut ws, &h.tok_a).await;
        let _lms = h.lms.accept().await;
        for m in msgs {
            ws.send(m).await.unwrap();
        }
        expect_close(&mut ws, CloseCode::Policy).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            h.lms.frames().is_empty(),
            "{name}: LMS received {:?}",
            h.lms.frames()
        );
        eprintln!("ok: {name}");
    }
}

#[tokio::test]
async fn second_helo_cannot_swap_identity() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(helo(h.mac_b).into()))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.lms.frames(), ["HELO"]);
}

#[tokio::test]
async fn serv_is_never_forwarded() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&encode_server_frame(*b"serv", &[203, 0, 113, 9]))
        .await
        .unwrap();
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(&b[2..6], b"vers", "serv must be dropped"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn stream_requests_are_locked_down() {
    type Case = (&'static str, Ipv4Addr, fn(&Harness) -> String);
    let cases: [Case; 4] = [
        ("other player's stream", Ipv4Addr::UNSPECIFIED, |h| {
            stream_req(h.mac_b)
        }),
        ("LMS JSON-RPC", Ipv4Addr::UNSPECIFIED, |_| {
            "GET /jsonrpc.js HTTP/1.0\r\n\r\n".to_string()
        }),
        ("download path", Ipv4Addr::UNSPECIFIED, |_| {
            "GET /music/1/download HTTP/1.0\r\n\r\n".to_string()
        }),
        (
            "direct stream elsewhere",
            Ipv4Addr::new(203, 0, 113, 7),
            |h| stream_req(h.mac_a),
        ),
    ];
    for (name, ip, make) in cases {
        let mut h = harness().await;
        let req = make(&h);
        let mut ws = h.dial().await;
        let sid = connect(&mut ws, &h.tok_a).await;
        let mut lms = h.lms.accept().await;
        lms.write_all(&strm(b's', ip, &req)).await.unwrap();
        // Still forwarded, so the client stays in step, but without where it pointed.
        match next(&mut ws).await {
            Message::Binary(b) => {
                assert_eq!(&b[2..6], b"strm", "{name}");
                let s = anywhere_core::proto::Strm::parse(&b[6..]).unwrap();
                assert_eq!(s.server_ip, 0, "{name}");
                assert!(s.request.is_empty(), "{name}: {:?}", s.request);
            }
            other => panic!("{name}: {other:?}"),
        }
        let resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
        assert_eq!(resp.status(), 404, "{name}");
        // Nothing of the refused stream reaches LMS. (The gateway's own JSON-RPC commands
        // may: after a direct stream it asks LMS to proxy and replay, see the tests below.)
        let reached: Vec<String> = h
            .lms
            .http_paths()
            .into_iter()
            .filter(|p| !p.starts_with("/jsonrpc.js"))
            .collect();
        assert!(reached.is_empty(), "{name}: LMS reached: {reached:?}");
    }
}

#[tokio::test]
async fn stream_endpoint_authorisation() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let sid = connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;

    let cases = [
        ("no token", h.get_stream(&sid, 1, None).await.status(), 401),
        (
            "bad token",
            h.get_stream(&sid, 1, Some("nope")).await.status(),
            401,
        ),
        (
            "other device's token",
            h.get_stream(&sid, 1, Some(&h.tok_b)).await.status(),
            404,
        ),
        (
            "unknown session",
            h.get_stream(&"0".repeat(32), 1, Some(&h.tok_a))
                .await
                .status(),
            404,
        ),
        (
            "stale sequence",
            h.get_stream(&sid, 0, Some(&h.tok_a)).await.status(),
            404,
        ),
        (
            "future sequence",
            h.get_stream(&sid, 2, Some(&h.tok_a)).await.status(),
            404,
        ),
    ];
    for (name, got, want) in cases {
        assert_eq!(got.as_u16(), want, "{name}");
    }
    let q = reqwest::get(h.url(&format!("/v1/stream/{sid}/1?token={}", h.tok_a)))
        .await
        .unwrap();
    assert_eq!(q.status(), 401, "token in query string must not work");
    let basic = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1")))
        .basic_auth("x", Some(&h.tok_a))
        .send()
        .await
        .unwrap();
    assert_eq!(basic.status(), 401, "basic auth must not work");
    assert!(
        h.lms.http_paths().is_empty(),
        "LMS reached by unauthorised requests"
    );

    assert_eq!(h.get_stream(&sid, 1, Some(&h.tok_a)).await.status(), 200);
}

// ---------------------------------------------------------------------------
// Security: HTTP surface
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nothing_else_is_proxied_to_lms() {
    let h = harness().await;
    for p in [
        "/jsonrpc.js",
        "/music/1/download",
        "/stream.mp3?player=02:4c:41:00:00:0a",
        "/cometd",
        "/settings/index.html",
        "/../jsonrpc.js",
        "/%2e%2e/jsonrpc.js",
        "/v1/stream/../../jsonrpc.js",
        "/plugins/",
        "/server.log?search=x",
        "/imageproxy/x/image.jpg",
        "/v1/rpc",
        "/v1/stream/abc",
    ] {
        let s = reqwest::get(h.url(p)).await.unwrap().status();
        assert!(s == 404 || s == 400 || s == 405, "{p}: {s}");
    }
    for m in [
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::DELETE,
    ] {
        let s = reqwest::Client::new()
            .request(m.clone(), h.url("/v1/stream/x/1"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(s, 405, "{m} accepted");
    }
    assert!(
        h.lms.http_paths().is_empty(),
        "LMS reached: {:?}",
        h.lms.http_paths()
    );
}

#[tokio::test]
async fn security_headers_are_set() {
    let h = harness().await;
    for path in ["/healthz", "/does-not-exist"] {
        let resp = reqwest::get(h.url(path)).await.unwrap();
        let hd = resp.headers();
        assert_eq!(hd["x-content-type-options"], "nosniff");
        assert_eq!(hd["x-frame-options"], "DENY");
        assert_eq!(hd["referrer-policy"], "no-referrer");
        let csp = hd["content-security-policy"].to_str().unwrap();
        let directive = |name: &str| {
            csp.split(';')
                .map(str::trim)
                .find(|d| d.split_whitespace().next() == Some(name))
                .map(str::to_owned)
        };
        assert_eq!(
            directive("frame-ancestors").as_deref(),
            Some("frame-ancestors 'none'")
        );
        assert_eq!(directive("base-uri").as_deref(), Some("base-uri 'none'"));
        // The media-element player plays Media Source object URLs (blob:), nothing remote.
        assert_eq!(
            directive("media-src").as_deref(),
            Some("media-src 'self' blob:")
        );
        // No inline or eval'd script, and no inline style sheets; only style attributes.
        let script = directive("script-src").expect("script-src set");
        assert!(
            !script.contains("unsafe-inline") && !script.contains("'unsafe-eval'"),
            "{csp}"
        );
        assert_eq!(
            directive("style-src").as_deref(),
            Some("style-src 'self'"),
            "{csp}"
        );
        assert!(directive("style-src-elem").is_none(), "{csp}");
        assert_eq!(
            directive("style-src-attr").as_deref(),
            Some("style-src-attr 'unsafe-inline'"),
            "{csp}"
        );
        for d in csp.split(';') {
            let d = d.trim();
            if d.contains("unsafe-inline") {
                assert!(
                    d.starts_with("style-src-attr "),
                    "unsafe-inline outside style attributes: {d}"
                );
            }
        }
    }
}

#[tokio::test]
async fn insecure_transport_is_refused() {
    let gw = Gateway::new(
        Config {
            trusted_proxies: vec!["10.0.0.10".parse().unwrap()],
            ..Config::default()
        },
        Arc::new(Accounts::in_memory().unwrap()),
    );
    let app = router(gw);
    // (name, peer address, request headers, expected status)
    type Case<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)], u16);
    let cases: &[Case] = &[
        ("remote plain http", "203.0.113.5:4444", &[], 403),
        (
            "remote forging X-Forwarded-Proto",
            "203.0.113.5:4444",
            &[("x-forwarded-proto", "https")],
            403,
        ),
        (
            "trusted proxy, http upstream",
            "10.0.0.10:5555",
            &[("x-forwarded-proto", "http")],
            403,
        ),
        (
            "trusted proxy, https",
            "10.0.0.10:5555",
            &[("x-forwarded-proto", "https")],
            200,
        ),
        (
            "trusted proxy, no proto",
            "10.0.0.10:5555",
            &[("x-forwarded-for", "203.0.113.5")],
            403,
        ),
        (
            "loopback proxy, https",
            "127.0.0.1:5555",
            &[("x-forwarded-proto", "https")],
            200,
        ),
        (
            "loopback proxy, http",
            "127.0.0.1:5555",
            &[("x-forwarded-proto", "http")],
            403,
        ),
        (
            "loopback, XFF only",
            "127.0.0.1:5555",
            &[("x-forwarded-for", "203.0.113.5")],
            403,
        ),
        ("loopback direct (dev)", "127.0.0.1:5555", &[], 200),
        ("ipv6 loopback direct", "[::1]:5555", &[], 200),
        (
            "ipv4-mapped remote",
            "[::ffff:203.0.113.5]:4444",
            &[("x-forwarded-proto", "https")],
            403,
        ),
        (
            "ipv4-mapped trusted proxy",
            "[::ffff:10.0.0.10]:5555",
            &[("x-forwarded-proto", "https")],
            200,
        ),
    ];
    for (name, remote, hdrs, want) in cases {
        let mut req = axum::http::Request::get("/healthz")
            .body(Body::empty())
            .unwrap();
        for (k, v) in *hdrs {
            req.headers_mut().insert(*k, v.parse().unwrap());
        }
        let peer: SocketAddr = remote.parse().unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(peer));
        let got = app.clone().oneshot(req).await.unwrap().status();
        assert_eq!(got, StatusCode::from_u16(*want).unwrap(), "{name}");
    }
}

// ---------------------------------------------------------------------------
// Resume after a dropped connection (reliability + security)
// ---------------------------------------------------------------------------

async fn greeting(ws: &mut Ws) -> serde_json::Value {
    match next(ws).await {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("expected greeting, got {other:?}"),
    }
}

/// New session: returns (session id, resume secret).
async fn open_session(ws: &mut Ws, token: &str) -> (String, String) {
    ws.send(Message::Text(token.into())).await.unwrap();
    let g = greeting(ws).await;
    assert_eq!(g["resumed"], false);
    let secret = g["secret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 32);
    (g["session"].as_str().unwrap().to_string(), secret)
}

fn resume_msg(token: &str, sid: &str, secret: &str) -> Message {
    resume_after(token, sid, secret, 0)
}

/// Resume, telling the gateway how many frames this client received before the drop.
fn resume_after(token: &str, sid: &str, secret: &str, received: u64) -> Message {
    Message::Text(
        serde_json::json!({
            "token": token,
            "resume": { "session": sid, "secret": secret, "received": received }
        })
        .to_string()
        .into(),
    )
}

#[tokio::test]
async fn resume_keeps_lms_connected_and_delivers_queued_frames() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;

    // Network drops. LMS keeps talking; frames must be queued, not lost.
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    lms.write_all(&encode_server_frame(*b"vers", b"one"))
        .await
        .unwrap();
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    let g = greeting(&mut ws).await;
    assert_eq!(g["resumed"], true);
    assert_eq!(g["session"], sid.as_str());
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(&b[2..6], b"vers"),
        m => panic!("{m:?}"),
    }
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(&b[2..6], b"strm"),
        m => panic!("{m:?}"),
    }
    // No HELO on resume: LMS still has the original. Traffic flows both ways.
    ws.send(Message::Binary(client_frame(b"STAT", &[0; 53]).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "STAT"]).await;
    lms.write_all(&encode_server_frame(*b"vers", b"two"))
        .await
        .unwrap();
    assert!(matches!(next(&mut ws).await, Message::Binary(_)));
    assert_eq!(h.lms.closed(), 0, "LMS must never see the player leave");
    // The stream announced while detached is fetchable on the resumed session.
    assert_eq!(h.get_stream(&sid, 1, Some(&h.tok_a)).await.status(), 200);
}

#[tokio::test]
async fn resume_is_refused_without_the_right_credentials() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    drop(ws);

    let mut wrong_secret = secret.clone();
    wrong_secret.replace_range(0..1, if secret.starts_with('0') { "1" } else { "0" });
    let attempts = [
        ("wrong secret", resume_msg(&h.tok_a, &sid, &wrong_secret)),
        ("empty secret", resume_msg(&h.tok_a, &sid, "")),
        ("other device's token", resume_msg(&h.tok_b, &sid, &secret)),
        (
            "unknown session",
            resume_msg(&h.tok_a, &"0".repeat(32), &secret),
        ),
        ("secret as token", resume_msg(&secret, &sid, &secret)),
    ];
    for (name, msg) in attempts {
        let mut ws = h.dial().await;
        ws.send(msg).await.unwrap();
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Close(Some(f)))) => {
                let code = u16::from(f.code);
                assert!(code == 4404 || code == 1008, "{name}: close {code}");
            }
            other => panic!("{name}: expected close, got {other:?}"),
        }
    }
    // A wrong guess doesn't destroy the session for its owner.
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], true);
    // Device B never got a session or an LMS connection out of this.
    h.lms.assert_no_conn().await;
}

#[tokio::test]
async fn resume_after_grace_expiry_is_refused() {
    let mut h = harness_with(|c| c.resume_grace = Duration::from_millis(200)).await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    drop(ws);
    wait_for(|| h.lms.closed() == 1).await;
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    expect_close(&mut ws, CloseCode::Library(4404)).await;
}

#[tokio::test]
async fn resume_takes_over_a_stale_socket() {
    // On mobile the old socket often isn't noticed as dead for minutes.
    let mut h = harness().await;
    let mut old = h.dial().await;
    let (sid, secret) = open_session(&mut old, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    let mut new = h.dial().await;
    new.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    assert_eq!(greeting(&mut new).await["resumed"], true);
    expect_close(&mut old, CloseCode::Library(4000)).await;
    lms.write_all(&encode_server_frame(*b"vers", b"x"))
        .await
        .unwrap();
    assert!(matches!(next(&mut new).await, Message::Binary(_)));
    assert_eq!(h.lms.closed(), 0);
}

#[tokio::test]
async fn new_session_for_the_same_device_replaces_the_old_one() {
    let mut h = harness().await;
    let mut first = h.dial().await;
    let (sid1, secret1) = open_session(&mut first, &h.tok_a).await;
    let _lms1 = h.lms.accept().await;
    let mut second = h.dial().await;
    let (sid2, _) = open_session(&mut second, &h.tok_a).await;
    let _lms2 = h.lms.accept().await;
    assert_ne!(sid1, sid2);
    expect_close(&mut first, CloseCode::Library(4000)).await;
    wait_for(|| h.lms.closed() == 1).await; // the old LMS connection is closed
    assert_eq!(h.gw.session_count(), 1);
    let mut again = h.dial().await;
    again
        .send(resume_msg(&h.tok_a, &sid1, &secret1))
        .await
        .unwrap();
    expect_close(&mut again, CloseCode::Library(4404)).await;
}

#[tokio::test]
async fn resumed_session_rejects_a_second_helo() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;
    drop(ws);
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    greeting(&mut ws).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.lms.frames(), ["HELO"]);
}

#[tokio::test]
async fn policy_violation_ends_the_whole_session() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_b).into()))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    wait_for(|| h.lms.closed() == 1).await;
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    expect_close(&mut ws, CloseCode::Library(4404)).await;
}

#[tokio::test]
async fn detached_queue_is_bounded() {
    let mut h = harness_with(|c| c.detached_queue_limit = 4096).await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    for _ in 0..10 {
        let _ = lms
            .write_all(&encode_server_frame(*b"vers", &[b'x'; 1000]))
            .await;
    }
    wait_for(|| h.lms.closed() == 1).await;
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    expect_close(&mut ws, CloseCode::Library(4404)).await;
}

#[tokio::test]
async fn stream_download_resumes_from_an_offset() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;

    // Read part of the stream, then the connection "drops".
    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let mut got = Vec::new();
    while got.len() < 50_000 {
        got.extend_from_slice(&resp.chunk().await.unwrap().unwrap());
    }
    drop(resp);
    let have = got.len();

    let resumed = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1?offset={have}")))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(resumed.status(), 200);
    got.extend_from_slice(&resumed.bytes().await.unwrap());
    assert_eq!(got, *h.lms.audio, "resumed download must be byte-identical");
    assert_eq!(
        h.lms.stream_requests().len(),
        1,
        "LMS must be asked for the stream only once: {:?}",
        h.lms.http_paths()
    );

    let beyond = reqwest::Client::new()
        .get(h.url(&format!(
            "/v1/stream/{sid}/1?offset={}",
            h.lms.audio.len() + 1
        )))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(beyond.status(), 416);
}

/// `?limit=` (the replay cache's look at a stream's first bytes): at most that many bytes,
/// ending cleanly; the rest follows with `?offset=`, byte-identical, and LMS is asked once.
/// A limit that is zero, too large or not a number is refused.
#[tokio::test]
async fn a_limited_request_sends_the_first_bytes_and_the_rest_follows() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;
    let get = |q: String| {
        let url = h.url(&format!("/v1/stream/{sid}/1?{q}"));
        let tok = h.tok_a.clone();
        async move {
            reqwest::Client::new()
                .get(url)
                .bearer_auth(tok)
                .send()
                .await
                .unwrap()
        }
    };
    for bad in [
        "limit=0",
        "limit=-1",
        "limit=abc",
        "limit=16777217",
        "limit=1e3",
    ] {
        assert_eq!(get(bad.into()).await.status(), 400, "{bad}");
    }
    let audio = h.lms.audio.clone();
    assert!(audio.len() > 100_000);
    // Not a multiple of the relay's pieces.
    let first = get("limit=70001".into()).await;
    assert_eq!(first.status(), 200);
    let mut got = first.bytes().await.unwrap().to_vec();
    assert_eq!(got.len(), 70_001, "exactly the limit, ending cleanly");
    // A limited request from an offset.
    let mid = get(format!("offset={}&limit=10", got.len())).await;
    assert_eq!(mid.bytes().await.unwrap().to_vec(), audio[70_001..70_011]);
    let rest = get(format!("offset={}", got.len())).await;
    got.extend_from_slice(&rest.bytes().await.unwrap());
    assert_eq!(got, *audio, "byte-identical");
    assert_eq!(
        h.lms.stream_requests().len(),
        1,
        "LMS asked once: {:?}",
        h.lms.http_paths()
    );
    // A limit beyond the end: what there is, ending cleanly.
    let tail = get(format!("offset={}&limit=1000000", audio.len() - 5)).await;
    assert_eq!(
        tail.bytes().await.unwrap().to_vec(),
        audio[audio.len() - 5..]
    );
}

#[tokio::test]
async fn stream_resume_outside_the_replay_window_is_gone() {
    let mut h = harness_with(|c| c.replay_window = 1000).await;
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;
    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let mut n = 0;
    while n < 50_000 {
        n += resp.chunk().await.unwrap().unwrap().len();
        ws.send(ack(1, n as u64)).await.unwrap(); // keep the paced relay flowing
    }
    drop(resp);
    let old = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1?offset=10")))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), 410);
    let first_without_offset_mid_stream = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1?offset=abc")))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(
        first_without_offset_mid_stream.status(),
        400,
        "non-numeric offset"
    );
}

// ---------------------------------------------------------------------------
// Heartbeats on behalf of a detached client (found in the live dropout test: LMS drops a
// player it hasn't heard from for ~15 s, which defeated resume)
// ---------------------------------------------------------------------------

fn stat_frame(event: &[u8; 4], jiffies: u32, elapsed_ms: u32) -> Vec<u8> {
    use anywhere_core::proto::{ClientMsg, Stat};
    ClientMsg::Stat(Stat {
        event: *event,
        jiffies,
        elapsed_seconds: elapsed_ms / 1000,
        elapsed_milliseconds: elapsed_ms,
        ..Default::default()
    })
    .encode()
}

async fn attached_with_stat(
    h: &mut Harness,
    event: &[u8; 4],
) -> (Ws, String, String, OwnedWriteHalf) {
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat_frame(event, 1_000, 5_000).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "STAT"]).await;
    (ws, sid, secret, lms)
}

#[tokio::test]
async fn detached_session_keeps_reporting_to_lms() {
    let mut h = harness().await;
    let (ws, _, _, _lms) = attached_with_stat(&mut h, b"STMs").await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(2600)).await;
    let stats = h.lms.stats();
    assert!(
        stats.len() >= 3,
        "expected heartbeats while detached, got {}",
        stats.len() - 1
    );
    let last = stats.last().unwrap();
    assert_eq!(&last.event, b"STMt");
    assert!(
        last.jiffies >= 1_000 + 1_500,
        "jiffies advance: {}",
        last.jiffies
    );
    assert!(
        last.elapsed_milliseconds >= 5_000 + 1_500,
        "playing: elapsed advances"
    );
    assert_eq!(last.elapsed_seconds, last.elapsed_milliseconds / 1000);
    assert_eq!(h.lms.closed(), 0);
}

#[tokio::test]
async fn detached_heartbeat_freezes_elapsed_when_paused() {
    let mut h = harness().await;
    let (ws, _, _, _lms) = attached_with_stat(&mut h, b"STMp").await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let last = h.lms.stats().last().cloned().unwrap();
    assert_eq!(&last.event, b"STMt");
    assert_eq!(
        last.elapsed_milliseconds, 5_000,
        "paused: elapsed must not advance"
    );
}

#[tokio::test]
async fn detached_session_answers_status_polls_and_does_not_queue_them() {
    let mut h = harness().await;
    let (ws, sid, secret, mut lms) = attached_with_stat(&mut h, b"STMs").await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut poll = vec![0u8; 24];
    poll[0] = b't';
    poll[14..18].copy_from_slice(&0xABCD_1234u32.to_be_bytes());
    lms.write_all(&encode_server_frame(*b"strm", &poll))
        .await
        .unwrap();
    wait_for(|| {
        h.lms
            .stats()
            .iter()
            .any(|s| s.server_timestamp == 0xABCD_1234)
    })
    .await;

    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    greeting(&mut ws).await;
    lms.write_all(&encode_server_frame(*b"vers", b"after"))
        .await
        .unwrap();
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(&b[2..6], b"vers", "the stale poll must not be replayed"),
        m => panic!("{m:?}"),
    }
}

#[tokio::test]
async fn no_heartbeats_before_helo_or_after_resume() {
    let mut h = harness().await;
    // Before HELO the gateway has no player identity to speak for.
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(1600)).await;
    assert!(
        h.lms.stats().is_empty(),
        "no STAT may be invented before HELO"
    );

    // After resume the client reports for itself; the gateway stops.
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    greeting(&mut ws).await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap(); // first HELO: allowed
    ws.send(Message::Binary(stat_frame(b"STMs", 1, 1).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.stats().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(1600)).await;
    assert_eq!(h.lms.stats().len(), 1, "no synthesised STAT while attached");
}

/// Regression: a socket that died without either side noticing (a phone changing networks)
/// stayed "attached", so nobody answered LMS; LMS dropped the player after about 15 s, which
/// ended the session before the client could resume, and it had to start afresh.
#[tokio::test]
async fn a_silent_attached_client_is_reported_for_until_it_speaks() {
    let mut h = harness_with(|c| c.client_silence = Duration::from_millis(700)).await;
    let (mut ws, _, _, _lms) = attached_with_stat(&mut h, b"STMs").await;
    // Silent, but the socket stays open.
    tokio::time::sleep(Duration::from_millis(2900)).await;
    let stats = h.lms.stats();
    assert!(
        stats.len() >= 2,
        "expected heartbeats for a silent client, got {}",
        stats.len() - 1
    );
    let last = stats.last().unwrap();
    assert_eq!(&last.event, b"STMt");
    assert!(
        last.elapsed_milliseconds >= 5_000 + 1_000,
        "playing: elapsed advances"
    );
    assert_eq!(h.lms.closed(), 0);

    // Once it speaks, it reports for itself again.
    ws.send(Message::Binary(stat_frame(b"STMt", 9_000, 9_000).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.stats().last().is_some_and(|s| s.jiffies == 9_000)).await;
    let n = h.lms.stats().len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(h.lms.stats().len(), n, "no synthesised STAT once it speaks");
}

// ---------------------------------------------------------------------------
// Redelivery of frames lost in flight (found by a race in the resume test: frames written
// into a dying socket were silently lost)
// ---------------------------------------------------------------------------

async fn vers_payload(ws: &mut Ws) -> Vec<u8> {
    match next(ws).await {
        Message::Binary(b) => {
            assert_eq!(&b[2..6], b"vers");
            b[6..].to_vec()
        }
        m => panic!("{m:?}"),
    }
}

#[tokio::test]
async fn frames_lost_in_flight_are_redelivered_on_resume() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    for v in [b"1", b"2", b"3"] {
        lms.write_all(&encode_server_frame(*b"vers", v))
            .await
            .unwrap();
    }
    for want in [b"1", b"2", b"3"] {
        assert_eq!(vers_payload(&mut ws).await, want);
    }
    // The client only really processed frame 1 before its network died.
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    lms.write_all(&encode_server_frame(*b"vers", b"4"))
        .await
        .unwrap();
    let mut ws = h.dial().await;
    ws.send(resume_after(&h.tok_a, &sid, &secret, 1))
        .await
        .unwrap();
    greeting(&mut ws).await;
    for want in [b"2", b"3", b"4"] {
        assert_eq!(
            vers_payload(&mut ws).await,
            want,
            "in order, nothing missing or repeated"
        );
    }
}

#[tokio::test]
async fn resume_claiming_unsent_frames_is_refused_but_session_survives() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&encode_server_frame(*b"vers", b"1"))
        .await
        .unwrap();
    vers_payload(&mut ws).await;
    drop(ws);
    let mut bad = h.dial().await;
    bad.send(resume_after(&h.tok_a, &sid, &secret, 99))
        .await
        .unwrap();
    expect_close(&mut bad, CloseCode::Library(4404)).await;
    let mut ws = h.dial().await;
    ws.send(resume_after(&h.tok_a, &sid, &secret, 1))
        .await
        .unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], true);
    assert_eq!(h.lms.closed(), 0);
}

#[tokio::test]
async fn resume_after_the_redelivery_window_starts_a_new_session() {
    // A tiny log: only the most recent frames are kept once delivered.
    let mut h = harness_with(|c| c.detached_queue_limit = 64).await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    for i in 0..20u8 {
        lms.write_all(&encode_server_frame(*b"vers", &[b'a' + i; 10]))
            .await
            .unwrap();
        vers_payload(&mut ws).await;
    }
    drop(ws);
    let mut ws = h.dial().await;
    ws.send(resume_after(&h.tok_a, &sid, &secret, 0))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Library(4404)).await;
    wait_for(|| h.lms.closed() == 1).await; // a client that missed frames can't continue
}

// ---------------------------------------------------------------------------
// Ack-paced streaming (found in the live dropout test: network buffers absorbed a whole
// track, so a resume could start before the replay window)
// ---------------------------------------------------------------------------

fn ack(stream: u64, bytes: u64) -> Message {
    Message::Text(
        serde_json::json!({ "ack": { "stream": stream, "bytes": bytes } })
            .to_string()
            .into(),
    )
}

async fn read_for(resp: &mut reqwest::Response, dur: Duration) -> (usize, bool) {
    let mut n = 0;
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        match tokio::time::timeout_at(deadline, resp.chunk()).await {
            Ok(Ok(Some(c))) => n += c.len(),
            Ok(Ok(None)) => return (n, true),
            Ok(Err(e)) => panic!("{e}"),
            Err(_) => return (n, false),
        }
    }
}

#[tokio::test]
async fn relay_stays_within_a_window_of_the_last_ack() {
    let mut h = harness_with(|c| c.replay_window = 8 * 1024).await;
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;

    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let (got, done) = read_for(&mut resp, Duration::from_millis(800)).await;
    assert!(!done, "must not run to the end without acknowledgements");
    assert!(
        got < 8 * 1024 + 64 * 1024,
        "sent {got} bytes unacknowledged"
    );

    // Acknowledging lets it carry on to the end.
    let mut total = got;
    loop {
        ws.send(ack(1, total as u64)).await.unwrap();
        let (more, done) = read_for(&mut resp, Duration::from_millis(300)).await;
        total += more;
        if done {
            break;
        }
    }
    assert_eq!(total, h.lms.audio.len());
}

#[tokio::test]
async fn acks_for_another_stream_do_not_release_data() {
    let mut h = harness_with(|c| c.replay_window = 8 * 1024).await;
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;
    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let (first, _) = read_for(&mut resp, Duration::from_millis(500)).await;
    ws.send(ack(7, 10_000_000)).await.unwrap();
    let (more, done) = read_for(&mut resp, Duration::from_millis(500)).await;
    assert!(
        !done && more == 0,
        "an ack for stream 7 released {more} bytes of stream 1 (first {first})"
    );
}

#[tokio::test]
async fn malformed_control_messages_are_policy_violations() {
    for bad in [
        r#"{"ack":{"stream":1}}"#.to_string(),
        r#"{"ack":{"stream":1,"bytes":5,"x":1}}"#.to_string(),
        r#"{"ack":{"stream":-1,"bytes":5}}"#.to_string(),
        r#"{"resume":{}}"#.to_string(),
        format!(r#"{{"ack":{{"stream":1,"bytes":{}}}}}"#, "9".repeat(300)),
        "hello".to_string(),
    ] {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        open_session(&mut ws, &h.tok_a).await;
        let _lms = h.lms.accept().await;
        ws.send(Message::Text(bad.clone().into())).await.unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
        wait_for(|| h.lms.closed() == 1).await;
    }
}

async fn cache_control(h: &Harness, path: &str) -> String {
    let r = reqwest::get(h.url(path)).await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK, "{path}");
    r.headers()["cache-control"].to_str().unwrap().to_string()
}

/// Regression: a phone kept running an old cached player engine after an update, because the
/// worker and WebAssembly files keep their names. Only content-hashed files may be cached.
#[tokio::test]
async fn app_files_are_revalidated_except_hashed_ones() {
    let dir = std::env::temp_dir().join(format!("anywhere-static-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("_app/immutable")).unwrap();
    std::fs::create_dir_all(dir.join("player/pkg")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html>").unwrap();
    std::fs::write(dir.join("_app/immutable/start.abc123.js"), "x").unwrap();
    std::fs::write(dir.join("player/pkg/anywhere_wasm.js"), "x").unwrap();
    let d = dir.clone();
    let h = harness_with(move |c| c.static_dir = Some(d)).await;
    assert_eq!(
        cache_control(&h, "/_app/immutable/start.abc123.js").await,
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        cache_control(&h, "/player/pkg/anywhere_wasm.js").await,
        "no-cache"
    );
    assert_eq!(cache_control(&h, "/").await, "no-cache");
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Static files: revalidation by content (static_files.rs)
// ---------------------------------------------------------------------------

/// A static directory for one test, removed when dropped.
struct StaticDir(std::path::PathBuf);

impl StaticDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("anywhere-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("_app/immutable")).unwrap();
        Self(dir)
    }

    fn write(&self, rel: &str, content: &[u8]) {
        std::fs::write(self.0.join(rel), content).unwrap();
    }

    /// Give a file the date every file in a release zip has (2020-01-01).
    fn date_like_a_release(&self, rel: &str) {
        let epoch_2020 = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
        std::fs::File::options()
            .write(true)
            .open(self.0.join(rel))
            .unwrap()
            .set_modified(epoch_2020)
            .unwrap();
    }
}

impl Drop for StaticDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn fetch(
    h: &Harness,
    method: reqwest::Method,
    path: &str,
    headers: &[(&str, &[u8])],
) -> reqwest::Response {
    let mut r = reqwest::Client::new().request(method, h.url(path));
    for (k, v) in headers {
        r = r.header(*k, reqwest::header::HeaderValue::from_bytes(v).unwrap());
    }
    r.send().await.unwrap()
}

fn etag(r: &reqwest::Response) -> String {
    r.headers()[reqwest::header::ETAG]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn revalidated_files_carry_a_content_etag() {
    use anywhere_gateway::static_files::content_tag;
    use reqwest::Method;
    let dir = StaticDir::new("static-etag");
    let page = b"<!doctype html><script src=\"/_app/immutable/start.abc123.js\"></script>";
    dir.write("index.html", page);
    dir.write(
        "service-worker.js",
        b"self.addEventListener('fetch', () => {})",
    );
    dir.write("_app/immutable/start.abc123.js", b"console.log('start')");
    let d = dir.0.clone();
    let h = harness_with(move |c| c.static_dir = Some(d)).await;

    // The page: a strong tag that is a hash of the content, the same at both its paths.
    let r = fetch(&h, Method::GET, "/", &[]).await;
    assert_eq!(r.status(), 200);
    let tag = etag(&r);
    assert_eq!(tag, content_tag(page).to_str().unwrap());
    assert_eq!(r.headers()["cache-control"], "no-cache");
    assert_eq!(r.bytes().await.unwrap().as_ref(), page);
    assert_eq!(etag(&fetch(&h, Method::GET, "/index.html", &[]).await), tag);
    assert_ne!(
        etag(&fetch(&h, Method::GET, "/service-worker.js", &[]).await),
        tag
    );

    // If-None-Match: 304 for the tag, in any of its forms, with the tag and the policy.
    for inm in [
        tag.clone(),
        format!("W/{tag}"),
        format!("\"other\", {tag}, \"more\""),
        "*".to_string(),
    ] {
        let r = fetch(&h, Method::GET, "/", &[("if-none-match", inm.as_bytes())]).await;
        assert_eq!(r.status(), 304, "{inm}");
        assert_eq!(etag(&r), tag, "{inm}");
        assert_eq!(r.headers()["cache-control"], "no-cache", "{inm}");
        assert!(r.bytes().await.unwrap().is_empty(), "{inm}");
    }
    let r = fetch(&h, Method::GET, "/", &[("if-none-match", b"\"other\"")]).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.bytes().await.unwrap().as_ref(), page);
    // Dates do not revalidate these files: every file in a release zip has the same date.
    let future = b"Fri, 01 Jan 2100 00:00:00 GMT";
    let r = fetch(&h, Method::GET, "/", &[("if-modified-since", future)]).await;
    assert_eq!(r.status(), 200);

    // HEAD: the headers of the GET, the tag included, and no body; 304 the same way.
    let r = fetch(&h, Method::HEAD, "/index.html", &[]).await;
    assert_eq!(r.status(), 200);
    assert_eq!(etag(&r), tag);
    assert_eq!(r.headers()["content-length"], page.len().to_string());
    assert!(r.bytes().await.unwrap().is_empty());
    let r = fetch(
        &h,
        Method::HEAD,
        "/index.html",
        &[("if-none-match", tag.as_bytes())],
    )
    .await;
    assert_eq!(r.status(), 304);
    assert_eq!(
        fetch(&h, Method::HEAD, "/missing.js", &[]).await.status(),
        404
    );

    // Content-hashed files are left to ServeDir: its own tag and date logic still apply.
    let r = fetch(&h, Method::GET, "/_app/immutable/start.abc123.js", &[]).await;
    assert_eq!(r.status(), 200);
    let served = etag(&r);
    assert_ne!(
        served,
        content_tag(b"console.log('start')").to_str().unwrap()
    );
    assert!(
        r.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    let r = fetch(
        &h,
        Method::GET,
        "/_app/immutable/start.abc123.js",
        &[("if-none-match", served.as_bytes())],
    )
    .await;
    assert_eq!(r.status(), 304);
    let r = fetch(
        &h,
        Method::GET,
        "/_app/immutable/start.abc123.js",
        &[("if-modified-since", future)],
    )
    .await;
    assert_eq!(r.status(), 304);

    // Range requests pass through as ServeDir answers them; other methods are refused; a
    // route is not a static file and carries no tag.
    let r = fetch(&h, Method::GET, "/index.html", &[("range", b"bytes=0-4")]).await;
    assert_eq!(r.status(), 206);
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"<!doc");
    let r = fetch(&h, Method::POST, "/index.html", &[]).await;
    assert_eq!(r.status(), 405);
    assert!(!r.headers().contains_key("etag"));
    let r = fetch(&h, Method::GET, "/healthz", &[]).await;
    assert_eq!(r.status(), 200);
    assert!(!r.headers().contains_key("etag"));
}

/// Regression: the release zip gives every file the date 2020-01-01, and the page's size
/// survives most releases (only the hashes it references change), so ServeDir's date-and-size
/// tag was the same for two releases' pages. A browser revalidating the previous release's
/// page got 304 and kept it, with scripts the server no longer had: a white screen.
#[tokio::test]
async fn a_file_changed_without_its_date_or_size_changing_is_served_afresh() {
    use reqwest::Method;
    let dir = StaticDir::new("static-redeploy");
    let release_a = b"<!doctype html><script src=\"/_app/immutable/start.aaaaaa.js\"></script>";
    let release_b = b"<!doctype html><script src=\"/_app/immutable/start.bbbbbb.js\"></script>";
    assert_eq!(release_a.len(), release_b.len());
    dir.write("index.html", release_a);
    dir.date_like_a_release("index.html");
    let d = dir.0.clone();
    let h = harness_with(move |c| c.static_dir = Some(d)).await;

    let r = fetch(&h, Method::GET, "/", &[]).await;
    assert_eq!(r.status(), 200);
    let tag_a = etag(&r);
    let date_a = r.headers()["last-modified"].to_str().unwrap().to_string();
    assert_eq!(date_a, "Wed, 01 Jan 2020 00:00:00 GMT");

    // The next release is deployed: same name, same size, same date, other content.
    dir.write("index.html", release_b);
    dir.date_like_a_release("index.html");

    let r = fetch(&h, Method::GET, "/", &[("if-none-match", tag_a.as_bytes())]).await;
    assert_eq!(
        r.status(),
        200,
        "the changed page is served, not a stale 304"
    );
    assert_ne!(etag(&r), tag_a);
    assert_eq!(r.bytes().await.unwrap().as_ref(), release_b);
    let r = fetch(
        &h,
        Method::GET,
        "/",
        &[("if-modified-since", date_a.as_bytes())],
    )
    .await;
    assert_eq!(r.status(), 200, "a date cannot tell the two releases apart");
    assert_eq!(r.bytes().await.unwrap().as_ref(), release_b);
    // A browser holding a page from a release before 0.7.2 revalidates with ServeDir's
    // date-and-size tag; it gets the new page too.
    let old_style = format!("\"5e0be100.00000000-{:x}\"", release_b.len());
    let r = fetch(
        &h,
        Method::GET,
        "/",
        &[("if-none-match", old_style.as_bytes())],
    )
    .await;
    assert_eq!(r.status(), 200);
}

/// Conditional headers of any shape, traversal attempts and oversize files through the new
/// layer: nothing panics, nothing leaks, big files still stream.
#[tokio::test]
async fn static_revalidation_is_robust() {
    use anywhere_gateway::static_files::{MAX_HASHED, content_tag};
    use reqwest::Method;
    let dir = StaticDir::new("static-robust");
    dir.write("index.html", b"<!doctype html>");
    let big = vec![b'x'; MAX_HASHED + 1];
    dir.write("big.bin", &big);
    let d = dir.0.clone();
    let h = harness_with(move |c| c.static_dir = Some(d)).await;

    let mut junk = b"\"".repeat(4000);
    junk.extend(b", W/\xff\xfe\x80, *garbage, \"\"\"");
    let r = fetch(&h, Method::GET, "/", &[("if-none-match", &junk)]).await;
    assert_eq!(r.status(), 200);
    let r = fetch(
        &h,
        Method::GET,
        "/",
        &[("if-modified-since", b"not a date")],
    )
    .await;
    assert_eq!(r.status(), 200);
    let r = fetch(
        &h,
        Method::GET,
        "/",
        &[("if-none-match", b""), ("if-modified-since", b"")],
    )
    .await;
    assert_eq!(r.status(), 200);

    for p in [
        "/%2e%2e/Cargo.toml",
        "/..%2f..%2fCargo.toml",
        "/%2e%2e%2f%2e%2e%2fetc%2fpasswd",
    ] {
        let r = fetch(&h, Method::GET, p, &[]).await;
        assert_eq!(r.status(), 404, "{p}");
        assert!(!r.headers().contains_key("etag"), "{p}");
    }

    // Beyond the hashing limit the file streams as ServeDir serves it, with its own tag.
    let r = fetch(&h, Method::GET, "/big.bin", &[]).await;
    assert_eq!(r.status(), 200);
    assert_ne!(etag(&r), content_tag(&big).to_str().unwrap());
    assert_eq!(r.bytes().await.unwrap().len(), big.len());
}

// ---------------------------------------------------------------------------
// Resume grace negotiated at sign-in (rolling cache, docs/plans/wave3-design.md A3)
// ---------------------------------------------------------------------------

fn sign_in_with(token: &str, grace: serde_json::Value) -> Message {
    Message::Text(
        serde_json::json!({ "token": token, "grace": grace })
            .to_string()
            .into(),
    )
}

fn resume_with(token: &str, sid: &str, secret: &str, grace: serde_json::Value) -> Message {
    Message::Text(
        serde_json::json!({
            "token": token,
            "resume": { "session": sid, "secret": secret, "received": 0 },
            "grace": grace,
        })
        .to_string()
        .into(),
    )
}

#[tokio::test]
async fn grace_is_granted_up_to_the_servers_maximum() {
    use serde_json::json;
    // Not asked: today's 60 s, in either form of the sign-in.
    let mut h = harness().await;
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    assert_eq!(greeting(&mut ws).await["grace"], 60);
    let _lms = h.lms.accept().await;
    let mut ws = h.dial().await;
    ws.send(Message::Text(
        json!({ "token": h.tok_b }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(greeting(&mut ws).await["grace"], 60);

    // Asked: granted up to the maximum (600 s by default).
    for (asked, granted) in [
        (0, 0),
        (1, 1),
        (300, 300),
        (600, 600),
        (601, 600),
        (3600, 600),
    ] {
        let h = harness().await;
        let mut ws = h.dial().await;
        ws.send(sign_in_with(&h.tok_a, json!(asked))).await.unwrap();
        let g = greeting(&mut ws).await;
        assert_eq!(g["resumed"], false);
        assert_eq!(g["grace"], granted, "asked {asked}");
    }
    // A server set to a lower maximum.
    let h = harness_with(|c| c.max_resume_grace = Duration::from_secs(120)).await;
    let mut ws = h.dial().await;
    ws.send(sign_in_with(&h.tok_a, json!(3600))).await.unwrap();
    assert_eq!(greeting(&mut ws).await["grace"], 120);
}

#[tokio::test]
async fn malformed_grace_is_a_policy_violation_and_opens_nothing() {
    use serde_json::json;
    let bad = [
        json!(-1),
        json!(3601),
        json!(1_000_000_000_000u64),
        json!(60.5),
        json!(60.0),
        json!("600"),
        json!(null),
        json!(true),
        json!([600]),
        json!({ "seconds": 600 }),
    ];
    for grace in bad {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        ws.send(sign_in_with(&h.tok_a, grace.clone()))
            .await
            .unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
        h.lms.assert_no_conn().await;
        assert_eq!(h.gw.session_count(), 0, "{grace}");
    }
}

#[tokio::test]
async fn malformed_grace_on_resume_ends_the_session() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut ws = h.dial().await;
    ws.send(resume_with(&h.tok_a, &sid, &secret, serde_json::json!("x")))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    wait_for(|| h.lms.closed() == 1).await;
    assert_eq!(h.gw.session_count(), 0);
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    expect_close(&mut ws, CloseCode::Library(4404)).await;
}

#[tokio::test]
async fn malformed_grace_with_a_wrong_secret_does_not_end_the_owners_session() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    drop(ws);
    // Another device, or a wrong secret, can't end A's session with a bad grace.
    let wrong = "0".repeat(32);
    for (tok, sec) in [(&h.tok_b, secret.as_str()), (&h.tok_a, wrong.as_str())] {
        let mut ws = h.dial().await;
        ws.send(resume_with(tok, &sid, sec, serde_json::json!(-5)))
            .await
            .unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
    }
    assert_eq!(h.lms.closed(), 0);
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], true);
    h.lms.assert_no_conn().await;
}

#[tokio::test]
async fn the_granted_grace_is_the_one_applied() {
    use serde_json::json;
    // The default is short here; the session asked for 2 s and outlives it.
    let mut h = harness_with(|c| c.resume_grace = Duration::from_millis(200)).await;
    let mut ws = h.dial().await;
    ws.send(sign_in_with(&h.tok_a, json!(2))).await.unwrap();
    let g = greeting(&mut ws).await;
    assert_eq!(g["grace"], 2);
    let (sid, secret) = (
        g["session"].as_str().unwrap().to_string(),
        g["secret"].as_str().unwrap().to_string(),
    );
    let _lms = h.lms.accept().await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(h.gw.session_count(), 1, "held for the granted grace");
    assert_eq!(h.lms.closed(), 0);

    // A resume without "grace" keeps what was granted; one with it changes it.
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    let g = greeting(&mut ws).await;
    assert_eq!((&g["resumed"], &g["grace"]), (&json!(true), &json!(2)));
    drop(ws);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut ws = h.dial().await;
    ws.send(resume_with(&h.tok_a, &sid, &secret, json!(0)))
        .await
        .unwrap();
    let g = greeting(&mut ws).await;
    assert_eq!((&g["resumed"], &g["grace"]), (&json!(true), &json!(0)));
    // Grace 0: the session goes as soon as the socket does.
    drop(ws);
    wait_for(|| h.lms.closed() == 1).await;
    assert_eq!(h.gw.session_count(), 0);
}

#[tokio::test]
async fn a_long_grace_still_holds_one_lms_connection_per_device() {
    use serde_json::json;
    let mut h = harness().await;
    let mut ws = h.dial().await;
    ws.send(sign_in_with(&h.tok_a, json!(600))).await.unwrap();
    assert_eq!(greeting(&mut ws).await["grace"], 600);
    let _lms1 = h.lms.accept().await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(50)).await;
    // A new session for the same device (not a resume) ends the detached one at once.
    let mut ws = h.dial().await;
    ws.send(sign_in_with(&h.tok_a, json!(600))).await.unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], false);
    let _lms2 = h.lms.accept().await;
    wait_for(|| h.lms.closed() == 1).await;
    assert_eq!(h.gw.session_count(), 1);
    drop(ws);
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Revoking the device ends a detached session at once, whatever its grace.
    let dev = h.accounts.authenticate(&h.tok_a).unwrap().device_id;
    h.accounts.revoke_device(dev).unwrap();
    h.gw.end_sessions_for(&[dev]).await;
    wait_for(|| h.lms.closed() == 2).await;
    assert_eq!(h.gw.session_count(), 0);
}

/// Ten minutes' worth of LMS status polls while detached (one every 5 s) are answered by
/// the gateway and never fill the detached frame log, however small it is.
#[tokio::test]
async fn a_long_detached_spell_of_status_polls_never_fills_the_log() {
    let mut h = harness_with(|c| c.detached_queue_limit = 4096).await;
    let mut ws = h.dial().await;
    ws.send(sign_in_with(&h.tok_a, serde_json::json!(600)))
        .await
        .unwrap();
    let g = greeting(&mut ws).await;
    let (sid, secret) = (
        g["session"].as_str().unwrap().to_string(),
        g["secret"].as_str().unwrap().to_string(),
    );
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    ws.send(Message::Binary(stat_frame(b"STMs", 1_000, 5_000).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "STAT"]).await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    for i in 1..=120u32 {
        let mut poll = vec![0u8; 24];
        poll[0] = b't';
        poll[14..18].copy_from_slice(&i.to_be_bytes());
        lms.write_all(&encode_server_frame(*b"strm", &poll))
            .await
            .unwrap();
    }
    wait_for(|| h.lms.stats().iter().any(|s| s.server_timestamp == 120)).await;
    assert_eq!(h.lms.closed(), 0);
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], true);
    lms.write_all(&encode_server_frame(*b"vers", b"after"))
        .await
        .unwrap();
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(&b[2..6], b"vers", "no poll was queued"),
        m => panic!("{m:?}"),
    }
}

/// LMS's recorded JSON-RPC calls for `player` that equal `cmd`.
fn calls_for(h: &Harness, player: &str, cmd: &serde_json::Value) -> usize {
    h.lms
        .rpc_calls()
        .iter()
        .filter(|c| {
            c[0].as_str()
                .is_some_and(|p| p.eq_ignore_ascii_case(player))
                && &c[1] == cmd
        })
        .count()
}

/// Poll until at least `want` such calls have been made (five seconds at most).
async fn wait_calls(h: &Harness, player: &str, cmd: &serde_json::Value, want: usize) -> usize {
    let mut n = 0;
    for _ in 0..100 {
        n = calls_for(h, player, cmd);
        if n >= want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    n
}

/// Internet radio (2026-10-18): LMS tells a capable player to fetch a station from the
/// broadcaster itself, which these players never do (they only ever talk to the gateway),
/// so the gateway has LMS proxy for each player, as soon as LMS knows it, once a session.
#[tokio::test]
async fn lms_is_told_to_proxy_once_the_player_is_known() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    let me = format_mac(&h.mac_a);
    let proxied = serde_json::json!(["playerpref", "mp3StreamingMethod", "1"]);
    // Nothing before LMS has spoken to the player: it doesn't know it yet.
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(calls_for(&h, &me, &proxied), 0, "not before LMS has spoken");
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    assert_eq!(wait_calls(&h, &me, &proxied, 1).await, 1);
    // Once per session, not once per frame.
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(calls_for(&h, &me, &proxied), 1);
}

/// `mixer volume 100`, as the gateway sends it (LMS's JSON-RPC takes the words as strings).
fn initial_level() -> serde_json::Value {
    serde_json::json!(["mixer", "volume", "100"])
}

/// Open a session for device A, send its HELO and have LMS speak once; returns the
/// websocket and LMS's side of the connection.
async fn attach_and_let_lms_speak(h: &mut Harness) -> (Ws, tokio::net::tcp::OwnedWriteHalf) {
    fn helos(h: &Harness) -> usize {
        h.lms.frames().iter().filter(|f| *f == "HELO").count()
    }
    let before = helos(h);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| helos(h) > before).await;
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    (ws, lms)
}

/// A brand-new device's player starts at full volume, not LMS's 50: the gateway sets it to
/// 100 once LMS knows the player, exactly once, and never again for that device, not on a
/// resume and not on a later session.
#[tokio::test]
async fn a_new_devices_player_is_set_to_full_volume_once() {
    let mut h = harness().await;
    let me = format_mac(&h.mac_a);
    assert!(h.accounts.needs_initial_level(h.dev_a));
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        calls_for(&h, &me, &initial_level()),
        0,
        "not before LMS has spoken"
    );
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    assert_eq!(wait_calls(&h, &me, &initial_level(), 1).await, 1);
    wait_for(|| !h.accounts.needs_initial_level(h.dev_a)).await;
    // Sent for this player only, never for anyone else's.
    let theirs = format_mac(&h.mac_b);
    assert_eq!(calls_for(&h, &theirs, &initial_level()), 0);
    // Once per session, not once per frame.
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    // A resume of the same session: nothing again.
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut ws = h.dial().await;
    ws.send(resume_msg(&h.tok_a, &sid, &secret)).await.unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], true);
    lms.write_all(&encode_server_frame(*b"vers", b"9.1.1"))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    drop(ws);
    drop(lms);
    // A new session for the same device (a reconnect): nothing again either.
    let (_ws, _lms) = attach_and_let_lms_speak(&mut h).await;
    let proxied = serde_json::json!(["playerpref", "mp3StreamingMethod", "1"]);
    assert_eq!(
        wait_calls(&h, &me, &proxied, 2).await,
        2,
        "second session ran"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(calls_for(&h, &me, &initial_level()), 1);
}

/// LMS failing the command: the device is not marked done, and the next session tries
/// again; only once LMS accepts it is the device done for good.
#[tokio::test]
async fn a_failed_initial_volume_is_tried_again_next_session() {
    let mut h = harness().await;
    let me = format_mac(&h.mac_a);
    h.lms.fail_rpc.lock().unwrap().insert("mixer volume".into());
    {
        let (_ws, _lms) = attach_and_let_lms_speak(&mut h).await;
        assert_eq!(wait_calls(&h, &me, &initial_level(), 1).await, 1);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            h.accounts.needs_initial_level(h.dev_a),
            "not done until LMS accepts it"
        );
    }
    h.lms.fail_rpc.lock().unwrap().clear();
    let (_ws, _lms) = attach_and_let_lms_speak(&mut h).await;
    assert_eq!(wait_calls(&h, &me, &initial_level(), 2).await, 2);
    wait_for(|| !h.accounts.needs_initial_level(h.dev_a)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(calls_for(&h, &me, &initial_level()), 2);
}

/// A device that already had its level (every device from before the gateway tracked it is
/// migrated as done): its player's volume is never touched.
#[tokio::test]
async fn an_existing_devices_volume_is_left_alone() {
    let mut h = harness().await;
    let me = format_mac(&h.mac_a);
    h.accounts.mark_initial_level_set(h.dev_a);
    let (_ws, _lms) = attach_and_let_lms_speak(&mut h).await;
    let proxied = serde_json::json!(["playerpref", "mp3StreamingMethod", "1"]);
    assert_eq!(wait_calls(&h, &me, &proxied, 1).await, 1, "LMS has spoken");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(calls_for(&h, &me, &initial_level()), 0);
    assert!(
        h.lms.rpc_calls().iter().all(|c| c[1][0] != "mixer"),
        "no mixer command at all"
    );
}

/// A direct stream that still arrives (the pref not yet applied, or ignored): the player
/// is told nothing of where it pointed, and LMS is asked to proxy and then to replay the
/// entry, which it then serves itself. At most three times in a row; an accepted stream
/// resets that.
#[tokio::test]
async fn a_refused_direct_stream_is_replayed_through_lms_up_to_three_times() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    let me = format_mac(&h.mac_a);
    let proxied = serde_json::json!(["playerpref", "mp3StreamingMethod", "1"]);
    let replay = serde_json::json!(["playlist", "jump", "+0"]);
    let elsewhere = Ipv4Addr::new(203, 0, 113, 7);
    let direct = || {
        strm_format(
            b's',
            b'm',
            elsewhere,
            "GET /radio.mp3 HTTP/1.0\r\nHost: radio.example.com\r\n\r\n",
        )
    };
    for attempt in 1..=4usize {
        lms.write_all(&direct()).await.unwrap();
        match next(&mut ws).await {
            Message::Binary(b) => {
                let s = anywhere_core::proto::Strm::parse(&b[6..]).unwrap();
                assert_eq!(s.server_ip, 0, "attempt {attempt}");
                assert!(s.request.is_empty(), "attempt {attempt}");
            }
            other => panic!("attempt {attempt}: {other:?}"),
        }
        let want = attempt.min(3);
        assert_eq!(
            wait_calls(&h, &me, &replay, want).await,
            want,
            "attempt {attempt}"
        );
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(calls_for(&h, &me, &replay), 3, "capped");
    assert!(calls_for(&h, &me, &proxied) >= 1);
    // The pref is set before each replay is asked for.
    let order: Vec<String> = h
        .lms
        .rpc_calls()
        .iter()
        .filter(|c| c[1] == proxied || c[1] == replay)
        .map(|c| c[1][0].as_str().unwrap().to_string())
        .collect();
    assert_eq!(order.first().map(String::as_str), Some("playerpref"));
    assert!(
        order
            .windows(2)
            .all(|w| !(w[0] == "playlist" && w[1] == "playlist")),
        "{order:?}"
    );
    // An accepted stream resets the count: a later direct one is replayed again.
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    let _ = next(&mut ws).await;
    lms.write_all(&direct()).await.unwrap();
    let _ = next(&mut ws).await;
    assert_eq!(wait_calls(&h, &me, &replay, 4).await, 4);
}
