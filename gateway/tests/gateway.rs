//! Gateway integration tests against a fake LMS that records everything it receives, so
//! each test can prove what did (and did not) reach LMS.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::routing::any;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tower::ServiceExt;

use anywhere_core::proto::{ClientFrameReader, encode_server_frame, format_mac};
use anywhere_gateway::devices::Devices;
use anywhere_gateway::{Config, Gateway, router};

const MAC_A: [u8; 6] = [0x02, 0x4c, 0x41, 0, 0, 0x0a];
const MAC_B: [u8; 6] = [0x02, 0x4c, 0x41, 0, 0, 0x0b];
const TOK_A: &str = "token-for-device-a-0123456789abcdef";
const TOK_B: &str = "token-for-device-b-0123456789abcdef";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------------------
// Fake LMS
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Recorded {
    frames: Vec<[u8; 4]>,
    http_paths: Vec<String>,
}

struct FakeLms {
    slim_addr: SocketAddr,
    http_addr: SocketAddr,
    rec: Arc<Mutex<Recorded>>,
    conns: mpsc::Receiver<OwnedWriteHalf>,
    audio: Arc<Vec<u8>>,
}

impl FakeLms {
    async fn start() -> FakeLms {
        let rec = Arc::new(Mutex::new(Recorded::default()));
        let audio = Arc::new(b"fLaCAUDIO".repeat(20_000));

        let slim = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slim_addr = slim.local_addr().unwrap();
        let (tx, conns) = mpsc::channel(4);
        let r = rec.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = slim.accept().await {
                let (mut rd, wr) = sock.into_split();
                let _ = tx.send(wr).await;
                let r = r.clone();
                tokio::spawn(async move {
                    let mut reader = ClientFrameReader::default();
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = rd.read(&mut buf).await {
                        if n == 0 {
                            return;
                        }
                        reader.push(&buf[..n]);
                        while let Ok(Some((op, _))) = reader.next_frame() {
                            r.lock().unwrap().frames.push(op);
                        }
                    }
                });
            }
        });

        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_addr = http.local_addr().unwrap();
        let (r, a) = (rec.clone(), audio.clone());
        let app = Router::new().fallback(any(move |req: Request| {
            let (r, a) = (r.clone(), a.clone());
            async move {
                let path = req
                    .uri()
                    .path_and_query()
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                r.lock().unwrap().http_paths.push(path);
                if req.uri().path() == "/stream.mp3" {
                    (
                        [("content-type", "audio/flac")],
                        Body::from(a.as_ref().clone()),
                    )
                        .into_response_parts()
                } else {
                    (
                        [("content-type", "text/plain")],
                        Body::from("LMS internal page"),
                    )
                        .into_response_parts()
                }
            }
        }));
        tokio::spawn(async move { axum::serve(http, app).await.unwrap() });

        FakeLms {
            slim_addr,
            http_addr,
            rec,
            conns,
            audio,
        }
    }

    async fn accept(&mut self) -> OwnedWriteHalf {
        tokio::time::timeout(Duration::from_secs(3), self.conns.recv())
            .await
            .expect("gateway never connected to LMS")
            .unwrap()
    }

    async fn assert_no_conn(&mut self) {
        let r = tokio::time::timeout(Duration::from_millis(150), self.conns.recv()).await;
        assert!(r.is_err(), "gateway connected to LMS but should not have");
    }

    fn frames(&self) -> Vec<String> {
        self.rec
            .lock()
            .unwrap()
            .frames
            .iter()
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect()
    }

    fn http_paths(&self) -> Vec<String> {
        self.rec.lock().unwrap().http_paths.clone()
    }
}

trait IntoParts {
    fn into_response_parts(self) -> axum::response::Response;
}
impl<T: axum::response::IntoResponse> IntoParts for T {
    fn into_response_parts(self) -> axum::response::Response {
        self.into_response()
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    lms: FakeLms,
    addr: SocketAddr,
    gw: anywhere_gateway::AppState,
}

async fn harness_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    let lms = FakeLms::start().await;
    let mut cfg = Config {
        lms_slimproto: lms.slim_addr.to_string(),
        lms_http: lms.http_addr.to_string(),
        lms_ips: vec![Ipv4Addr::LOCALHOST],
        auth_timeout: Duration::from_millis(300),
        ..Config::default()
    };
    mutate(&mut cfg);
    let devices = Devices::from_tokens(&[(TOK_A, "device-a", MAC_A), (TOK_B, "device-b", MAC_B)]);
    let gw = Gateway::new(cfg, devices);
    let app = router(gw.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Harness { lms, addr, gw }
}

async fn harness() -> Harness {
    harness_with(|_| {}).await
}

impl Harness {
    async fn dial(&self) -> Ws {
        self.dial_with(&[]).await.expect("websocket dial")
    }

    async fn dial_with(
        &self,
        headers: &[(&str, &str)],
    ) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}/v1/player", self.addr)
            .into_client_request()
            .unwrap();
        for (k, v) in headers {
            let name = axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap();
            req.headers_mut().insert(name, v.parse().unwrap());
        }
        connect_async(req).await.map(|(ws, _)| ws)
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    async fn get_stream(&self, sid: &str, n: u64, token: Option<&str>) -> reqwest::Response {
        let mut req = reqwest::Client::new().get(self.url(&format!("/v1/stream/{sid}/{n}")));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap()
    }
}

async fn connect(ws: &mut Ws, token: &str) -> String {
    ws.send(Message::Text(token.into())).await.unwrap();
    match next(ws).await {
        Message::Text(t) => {
            let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
            let sid = v["session"].as_str().unwrap().to_string();
            assert_eq!(sid.len(), 32);
            sid
        }
        other => panic!("expected session message, got {other:?}"),
    }
}

async fn next(ws: &mut Ws) -> Message {
    tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timed out waiting for message")
        .expect("socket ended")
        .expect("socket error")
}

async fn expect_close(ws: &mut Ws, want: CloseCode) {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no close in time")
        {
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(f.code, want, "close reason: {}", f.reason);
                return;
            }
            Some(Ok(Message::Close(None))) | None => panic!("closed without code, wanted {want:?}"),
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("socket error instead of close {want:?}: {e}"),
        }
    }
}

fn helo(mac: [u8; 6]) -> Vec<u8> {
    let mut p = vec![0u8; 36];
    p[0] = 12;
    p[2..8].copy_from_slice(&mac);
    p[34..36].copy_from_slice(b"en");
    p.extend_from_slice(b"Model=squeezelite,flc");
    client_frame(b"HELO", &p)
}

fn client_frame(op: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = op.to_vec();
    v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

fn strm(cmd: u8, ip: Ipv4Addr, request: &str) -> Vec<u8> {
    let mut p = vec![0u8; 24];
    p[0] = cmd;
    p[1] = b'1';
    p[2] = b'f';
    p[18..20].copy_from_slice(&9000u16.to_be_bytes());
    p[20..24].copy_from_slice(&ip.octets());
    p.extend_from_slice(request.as_bytes());
    encode_server_frame(*b"strm", &p)
}

fn stream_req(mac: [u8; 6]) -> String {
    format!(
        "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
        format_mac(&mac)
    )
}

async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met in time");
}

// ---------------------------------------------------------------------------
// Reliability: happy path and lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn end_to_end_tunnel_and_stream() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let sid = connect(&mut ws, TOK_A).await;
    let mut lms = h.lms.accept().await;

    ws.send(Message::Binary(helo(MAC_A).into())).await.unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;

    let frame = strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(MAC_A));
    lms.write_all(&frame).await.unwrap();
    match next(&mut ws).await {
        Message::Binary(b) => assert_eq!(b.as_ref(), frame.as_slice(), "strm altered in transit"),
        other => panic!("{other:?}"),
    }

    let resp = h.get_stream(&sid, 1, Some(TOK_A)).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "audio/flac");
    assert_eq!(resp.bytes().await.unwrap().as_ref(), h.lms.audio.as_slice());
    assert_eq!(
        h.lms.http_paths(),
        [format!("/stream.mp3?player={}", format_mac(&MAC_A))]
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
    connect(&mut ws, TOK_A).await;
    let lms = h.lms.accept().await;
    assert_eq!(h.gw.session_count(), 1);
    drop(lms);
    expect_close(&mut ws, CloseCode::Away).await;
    wait_for(|| h.gw.session_count() == 0).await;
}

#[tokio::test]
async fn client_disconnect_releases_session_and_lms_connection() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, TOK_A).await;
    let _lms = h.lms.accept().await;
    drop(ws);
    wait_for(|| h.gw.session_count() == 0).await;
}

#[tokio::test]
async fn lms_unreachable_closes_with_try_again() {
    let h = harness_with(|c| {
        c.lms_slimproto = "127.0.0.1:1".into();
        c.dial_timeout = Duration::from_millis(200);
    })
    .await;
    let mut ws = h.dial().await;
    ws.send(Message::Text(TOK_A.into())).await.unwrap();
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
    let cases: Vec<(&str, Message)> = vec![
        ("wrong token", Message::Text("nope".into())),
        ("empty token", Message::Text("".into())),
        (
            "token as binary",
            Message::Binary(TOK_A.as_bytes().to_vec().into()),
        ),
        (
            "near-miss token",
            Message::Text(format!("{}X", &TOK_A[..TOK_A.len() - 1]).into()),
        ),
        ("oversize token", Message::Text("A".repeat(4096).into())),
        ("HELO instead of token", Message::Binary(helo(MAC_A).into())),
        (
            "token with trailing space",
            Message::Text(format!("{TOK_A} ").into()),
        ),
    ];
    for (name, msg) in cases {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        ws.send(msg).await.unwrap();
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
    let cases: Vec<(&str, Vec<Message>)> = vec![
        (
            "other device's MAC",
            vec![Message::Binary(helo(MAC_B).into())],
        ),
        (
            "first frame not HELO",
            vec![Message::Binary(client_frame(b"STAT", &[0; 53]).into())],
        ),
        (
            "truncated HELO",
            vec![Message::Binary(client_frame(b"HELO", &[0; 10]).into())],
        ),
        (
            "oversize frame",
            vec![Message::Binary(b"RESP\x00\x10\x00\x00".to_vec().into())],
        ),
        ("text after auth", vec![Message::Text("hello".into())]),
    ];
    for (name, msgs) in cases {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        connect(&mut ws, TOK_A).await;
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
    connect(&mut ws, TOK_A).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(MAC_A).into())).await.unwrap();
    ws.send(Message::Binary(helo(MAC_B).into())).await.unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.lms.frames(), ["HELO"]);
}

#[tokio::test]
async fn serv_is_never_forwarded() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, TOK_A).await;
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
    let cases = [
        (
            "other player's stream",
            Ipv4Addr::UNSPECIFIED,
            stream_req(MAC_B),
        ),
        (
            "LMS JSON-RPC",
            Ipv4Addr::UNSPECIFIED,
            "GET /jsonrpc.js HTTP/1.0\r\n\r\n".to_string(),
        ),
        (
            "download path",
            Ipv4Addr::UNSPECIFIED,
            "GET /music/1/download HTTP/1.0\r\n\r\n".to_string(),
        ),
        (
            "direct stream elsewhere",
            Ipv4Addr::new(203, 0, 113, 7),
            stream_req(MAC_A),
        ),
    ];
    for (name, ip, req) in cases {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        let sid = connect(&mut ws, TOK_A).await;
        let mut lms = h.lms.accept().await;
        lms.write_all(&strm(b's', ip, &req)).await.unwrap();
        next(&mut ws).await; // still forwarded, so the client stays in step
        let resp = h.get_stream(&sid, 1, Some(TOK_A)).await;
        assert_eq!(resp.status(), 404, "{name}");
        assert!(
            h.lms.http_paths().is_empty(),
            "{name}: LMS reached: {:?}",
            h.lms.http_paths()
        );
    }
}

#[tokio::test]
async fn stream_endpoint_authorisation() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let sid = connect(&mut ws, TOK_A).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(MAC_A)))
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
            h.get_stream(&sid, 1, Some(TOK_B)).await.status(),
            404,
        ),
        (
            "unknown session",
            h.get_stream(&"0".repeat(32), 1, Some(TOK_A)).await.status(),
            404,
        ),
        (
            "stale sequence",
            h.get_stream(&sid, 0, Some(TOK_A)).await.status(),
            404,
        ),
        (
            "future sequence",
            h.get_stream(&sid, 2, Some(TOK_A)).await.status(),
            404,
        ),
    ];
    for (name, got, want) in cases {
        assert_eq!(got.as_u16(), want, "{name}");
    }
    let q = reqwest::get(h.url(&format!("/v1/stream/{sid}/1?token={TOK_A}")))
        .await
        .unwrap();
    assert_eq!(q.status(), 401, "token in query string must not work");
    let basic = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1")))
        .basic_auth("x", Some(TOK_A))
        .send()
        .await
        .unwrap();
    assert_eq!(basic.status(), 401, "basic auth must not work");
    assert!(
        h.lms.http_paths().is_empty(),
        "LMS reached by unauthorised requests"
    );

    assert_eq!(h.get_stream(&sid, 1, Some(TOK_A)).await.status(), 200);
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
        assert!(
            csp.contains("frame-ancestors 'none'") && !csp.contains("unsafe-inline"),
            "{csp}"
        );
    }
}

#[tokio::test]
async fn insecure_transport_is_refused() {
    let gw = Gateway::new(
        Config {
            trusted_proxies: vec!["10.0.0.10".parse().unwrap()],
            ..Config::default()
        },
        Devices::default(),
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
