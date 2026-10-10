//! Shared test harness: a fake LMS that records everything it receives, and a gateway
//! with two users logged in.
#![allow(dead_code, unused_imports)]

pub use std::net::{Ipv4Addr, SocketAddr};
pub use std::sync::{Arc, Mutex};
pub use std::time::Duration;

pub use axum::Router;
pub use axum::body::Body;
pub use axum::extract::Request;
pub use axum::http::StatusCode;
pub use axum::routing::any;
pub use futures_util::{SinkExt, StreamExt};
pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub use tokio::net::tcp::OwnedWriteHalf;
pub use tokio::net::{TcpListener, TcpStream};
pub use tokio::sync::mpsc;
pub use tokio_tungstenite::tungstenite::Message;
pub use tokio_tungstenite::tungstenite::client::IntoClientRequest;
pub use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
pub use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
pub use tower::ServiceExt;

pub use anywhere_core::proto::{ClientFrameReader, encode_server_frame, format_mac};
pub use anywhere_gateway::accounts::{Accounts, Role};
pub use anywhere_gateway::{Config, Gateway, router};

pub const PW: &str = "correct horse battery";

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------------------
// Fake LMS
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Recorded {
    pub frames: Vec<[u8; 4]>,
    pub http_paths: Vec<String>,
    /// Slimproto connections the gateway has closed.
    pub closed: usize,
    /// STAT payloads received, in order.
    pub stats: Vec<Vec<u8>>,
}

pub struct FakeLms {
    pub slim_addr: SocketAddr,
    pub http_addr: SocketAddr,
    pub rec: Arc<Mutex<Recorded>>,
    pub conns: mpsc::Receiver<OwnedWriteHalf>,
    pub audio: Arc<Vec<u8>>,
}

impl FakeLms {
    pub async fn start() -> FakeLms {
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
                            break;
                        }
                        reader.push(&buf[..n]);
                        while let Ok(Some((op, payload))) = reader.next_frame() {
                            let mut rec = r.lock().unwrap();
                            rec.frames.push(op);
                            if &op == b"STAT" {
                                rec.stats.push(payload);
                            }
                        }
                    }
                    r.lock().unwrap().closed += 1;
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

    pub async fn accept(&mut self) -> OwnedWriteHalf {
        tokio::time::timeout(Duration::from_secs(3), self.conns.recv())
            .await
            .expect("gateway never connected to LMS")
            .unwrap()
    }

    pub async fn assert_no_conn(&mut self) {
        let r = tokio::time::timeout(Duration::from_millis(150), self.conns.recv()).await;
        assert!(r.is_err(), "gateway connected to LMS but should not have");
    }

    pub fn frames(&self) -> Vec<String> {
        self.rec
            .lock()
            .unwrap()
            .frames
            .iter()
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect()
    }

    pub fn stats(&self) -> Vec<anywhere_core::proto::Stat> {
        self.rec
            .lock()
            .unwrap()
            .stats
            .iter()
            .map(|p| anywhere_core::proto::Stat::parse(p).unwrap())
            .collect()
    }

    pub fn closed(&self) -> usize {
        self.rec.lock().unwrap().closed
    }

    pub fn http_paths(&self) -> Vec<String> {
        self.rec.lock().unwrap().http_paths.clone()
    }
}

pub trait IntoParts {
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

pub struct Harness {
    pub lms: FakeLms,
    pub addr: SocketAddr,
    pub gw: anywhere_gateway::AppState,
    pub accounts: Arc<Accounts>,
    /// Access tokens and assigned player MACs for two users' devices.
    pub tok_a: String,
    pub mac_a: [u8; 6],
    pub tok_b: String,
    pub mac_b: [u8; 6],
}

pub async fn harness_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    let lms = FakeLms::start().await;
    let mut cfg = Config {
        lms_slimproto: lms.slim_addr.to_string(),
        lms_http: lms.http_addr.to_string(),
        lms_ips: vec![Ipv4Addr::LOCALHOST],
        auth_timeout: Duration::from_millis(300),
        ..Config::default()
    };
    mutate(&mut cfg);
    let accounts = Arc::new(Accounts::in_memory().unwrap());
    accounts.create_user("alice", PW, Role::Listener).unwrap();
    accounts.create_user("bob", PW, Role::Listener).unwrap();
    let a = accounts
        .login("alice", PW, None, "device-a", None, "test")
        .unwrap();
    let b = accounts
        .login("bob", PW, None, "device-b", None, "test")
        .unwrap();
    let gw = Gateway::new(cfg, accounts.clone());
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
    Harness {
        lms,
        addr,
        gw,
        accounts,
        tok_a: a.access,
        mac_a: a.principal.mac,
        tok_b: b.access,
        mac_b: b.principal.mac,
    }
}

pub async fn harness() -> Harness {
    harness_with(|_| {}).await
}

impl Harness {
    pub async fn dial(&self) -> Ws {
        self.dial_with(&[]).await.expect("websocket dial")
    }

    pub async fn dial_with(
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

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub async fn get_stream(&self, sid: &str, n: u64, token: Option<&str>) -> reqwest::Response {
        let mut req = reqwest::Client::new().get(self.url(&format!("/v1/stream/{sid}/{n}")));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap()
    }
}

pub async fn connect(ws: &mut Ws, token: &str) -> String {
    ws.send(Message::Text(token.into())).await.unwrap();
    match next(ws).await {
        Message::Text(t) => {
            let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
            let sid = v["session"].as_str().unwrap().to_string();
            assert_eq!(sid.len(), 32);
            let mac = v["mac"].as_str().unwrap();
            assert!(
                mac.starts_with("02:4c:41:") && mac.len() == 17,
                "assigned MAC {mac}"
            );
            assert!(v["name"].is_string());
            sid
        }
        other => panic!("expected session message, got {other:?}"),
    }
}

pub async fn next(ws: &mut Ws) -> Message {
    tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timed out waiting for message")
        .expect("socket ended")
        .expect("socket error")
}

pub async fn expect_close(ws: &mut Ws, want: CloseCode) {
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

pub fn helo(mac: [u8; 6]) -> Vec<u8> {
    let mut p = vec![0u8; 36];
    p[0] = 12;
    p[2..8].copy_from_slice(&mac);
    p[34..36].copy_from_slice(b"en");
    p.extend_from_slice(b"Model=squeezelite,flc");
    client_frame(b"HELO", &p)
}

pub fn client_frame(op: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = op.to_vec();
    v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn strm(cmd: u8, ip: Ipv4Addr, request: &str) -> Vec<u8> {
    let mut p = vec![0u8; 24];
    p[0] = cmd;
    p[1] = b'1';
    p[2] = b'f';
    p[18..20].copy_from_slice(&9000u16.to_be_bytes());
    p[20..24].copy_from_slice(&ip.octets());
    p.extend_from_slice(request.as_bytes());
    encode_server_frame(*b"strm", &p)
}

pub fn stream_req(mac: [u8; 6]) -> String {
    format!(
        "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
        format_mac(&mac)
    )
}

pub async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met in time");
}
