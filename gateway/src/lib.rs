//! Lyrion Anywhere gateway: the only internet-facing component.
//!
//! It authenticates devices, tunnels Slimproto over WebSocket with policy enforcement, and
//! relays audio streams. Nothing reaches LMS unless a handler here explicitly allows it.
//!
//! `/v1/player` messages:
//! - client → gateway: first a text message holding the device token, then binary messages
//!   carrying the client → server Slimproto byte stream (any chunking).
//! - gateway → client: a text message `{"session":"<id>","mac":"02:…","name":"…"}` (the
//!   player identity this device must use in HELO), then one binary message per
//!   server → client Slimproto frame, length prefix included, exactly as LMS sent it.
//!
//! Audio for the n-th `strm s` frame the client receives (counting from 1) is fetched with
//! `GET /v1/stream/<session>/<n>` and the same token as a bearer credential. The gateway
//! replays only the request LMS itself issued, after checking it is this player's stream.

pub mod devices;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};

use anywhere_core::proto::{
    ClientFrameReader, Helo, ServerFrameReader, Strm, encode_server_frame, validate_stream_request,
};
use devices::{Device, Devices, random_id};

/// Bound on a single WebSocket message from a client.
const MAX_WS_MESSAGE: usize = ClientFrameReader::MAX_FRAME + 1024;
const MAX_TOKEN_LEN: usize = 512;

#[derive(Debug, Clone)]
pub struct Config {
    /// LMS Slimproto and HTTP listeners (host:port).
    pub lms_slimproto: String,
    pub lms_http: String,
    /// Addresses LMS may name in `strm` server_ip (0.0.0.0 is always allowed).
    pub lms_ips: Vec<Ipv4Addr>,
    /// Extra `Origin` values (e.g. `https://music.example.com`) accepted on WebSocket
    /// upgrade. Same-origin and absent Origin (native clients) are always accepted.
    pub allowed_origins: Vec<String>,
    /// Reverse proxies allowed to set X-Forwarded-*. Loopback is always trusted.
    pub trusted_proxies: Vec<IpAddr>,
    pub auth_timeout: Duration,
    pub dial_timeout: Duration,
    /// Web app directory served at `/`, if any.
    pub static_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            lms_slimproto: "127.0.0.1:3483".into(),
            lms_http: "127.0.0.1:9000".into(),
            lms_ips: vec![],
            allowed_origins: vec![],
            trusted_proxies: vec![],
            auth_timeout: Duration::from_secs(5),
            dial_timeout: Duration::from_secs(5),
            static_dir: None,
        }
    }
}

struct Session {
    device: Arc<Device>,
    stream_seq: u64,
    /// `None` when the current `strm` was refused by policy.
    stream_path: Option<String>,
}

pub struct Gateway {
    cfg: Config,
    devices: Devices,
    http: reqwest::Client,
    sessions: Mutex<HashMap<String, Session>>,
}

pub type AppState = Arc<Gateway>;

impl Gateway {
    pub fn new(cfg: Config, devices: Devices) -> AppState {
        let http = reqwest::Client::builder()
            .connect_timeout(cfg.dial_timeout)
            .read_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("http client");
        Arc::new(Gateway {
            cfg,
            devices,
            http,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route("/v1/player", get(player_upgrade))
        .route("/v1/stream/{session}/{n}", get(stream));
    app = match &state.cfg.static_dir {
        Some(dir) => app.fallback_service(tower_http::services::ServeDir::new(dir)),
        None => app.fallback(|| async { StatusCode::NOT_FOUND }),
    };
    app.layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_secure_transport,
        ))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; \
             connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    resp
}

/// Refuse remote requests that didn't arrive over TLS: either via a trusted reverse proxy
/// reporting `X-Forwarded-Proto: https`, or plain HTTP from loopback with no proxy headers
/// (local development).
async fn require_secure_transport(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let ip = canonical(peer.ip());
    let trusted = ip.is_loopback() || gw.cfg.trusted_proxies.iter().any(|p| canonical(*p) == ip);
    let proto = req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok());
    let proxied = req.headers().contains_key("x-forwarded-for") || proto.is_some();
    let ok = match proto {
        Some(p) => trusted && p.eq_ignore_ascii_case("https"),
        None => ip.is_loopback() && !proxied,
    };
    if !ok {
        warn!(peer = %ip, path = %req.uri().path(), "refused insecure request");
        return (StatusCode::FORBIDDEN, "TLS required\n").into_response();
    }
    next.run(req).await
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// Same-origin, an explicitly allowed origin, or no Origin header (non-browser clients).
fn origin_allowed(cfg: &Config, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if cfg.allowed_origins.iter().any(|o| o == origin) {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    !host.is_empty() && (origin == format!("https://{host}") || origin == format!("http://{host}"))
}

// ---------------------------------------------------------------------------
// Slimproto tunnel
// ---------------------------------------------------------------------------

async fn player_upgrade(
    State(gw): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&gw.cfg, &headers) {
        warn!("refused cross-origin websocket");
        return (StatusCode::FORBIDDEN, "origin not allowed\n").into_response();
    }
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| tunnel(gw, socket))
}

enum End {
    Policy(String),
    LmsClosed,
    ClientGone,
    Error(String),
}

async fn close(mut ws: WebSocket, code: u16, reason: &str) {
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn tunnel(gw: AppState, mut ws: WebSocket) {
    // 1. Authenticate: the first message must be the token, within the time limit.
    let first = tokio::time::timeout(gw.cfg.auth_timeout, ws.recv()).await;
    let device = match first {
        Ok(Some(Ok(Message::Text(t)))) if t.len() <= MAX_TOKEN_LEN => {
            gw.devices.authenticate(t.as_str())
        }
        _ => None,
    };
    let Some(device) = device else {
        warn!("player authentication failed");
        return close(ws, close_code::POLICY, "unauthorised").await;
    };
    let mac = device.mac_string();

    // 2. Connect to LMS.
    let lms = tokio::time::timeout(
        gw.cfg.dial_timeout,
        TcpStream::connect(&gw.cfg.lms_slimproto),
    )
    .await;
    let Ok(Ok(lms)) = lms else {
        warn!(%mac, "cannot reach LMS slimproto");
        return close(ws, close_code::AGAIN, "server unavailable").await;
    };
    let _ = lms.set_nodelay(true);

    let id = random_id();
    gw.sessions.lock().unwrap().insert(
        id.clone(),
        Session {
            device: device.clone(),
            stream_seq: 0,
            stream_path: None,
        },
    );
    let hello = serde_json::json!({ "session": id, "mac": mac, "name": device.name }).to_string();
    if ws.send(Message::Text(hello.into())).await.is_err() {
        gw.sessions.lock().unwrap().remove(&id);
        return;
    }
    info!(device = %device.name, %mac, "player connected");

    let end = pump(&gw, &mut ws, lms, &device, &id).await;
    gw.sessions.lock().unwrap().remove(&id);
    match end {
        End::Policy(why) => {
            warn!(device = %device.name, %mac, reason = %why, "player closed for policy violation");
            close(ws, close_code::POLICY, &why).await
        }
        End::LmsClosed => {
            info!(%mac, "LMS closed the player connection");
            close(ws, close_code::AWAY, "server closed connection").await
        }
        End::ClientGone => info!(%mac, "player disconnected"),
        End::Error(e) => {
            info!(%mac, error = %e, "player connection ended");
            close(ws, close_code::ERROR, "connection error").await
        }
    }
}

async fn pump(gw: &Gateway, ws: &mut WebSocket, lms: TcpStream, device: &Device, id: &str) -> End {
    let (mut lms_rd, mut lms_wr) = lms.into_split();
    let mut from_client = ClientFrameReader::default();
    let mut from_lms = ServerFrameReader::default();
    let mut seen_helo = false;
    let mut buf = vec![0u8; 32 * 1024];

    loop {
        tokio::select! {
            msg = ws.recv() => {
                let data = match msg {
                    Some(Ok(Message::Binary(b))) => b,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Text(_))) => return End::Policy("unexpected text message".into()),
                    Some(Ok(Message::Close(_))) | None => return End::ClientGone,
                    Some(Err(e)) => return End::Error(e.to_string()),
                };
                from_client.push(&data);
                loop {
                    let (op, payload) = match from_client.next_frame() {
                        Ok(Some(f)) => f,
                        Ok(None) => break,
                        Err(e) => return End::Policy(e.to_string()),
                    };
                    if &op == b"HELO" {
                        if seen_helo {
                            return End::Policy("repeated HELO".into());
                        }
                        match Helo::parse(&payload) {
                            Ok(h) if h.mac == device.mac => seen_helo = true,
                            Ok(_) => return End::Policy("HELO MAC does not match device".into()),
                            Err(_) => return End::Policy("malformed HELO".into()),
                        }
                    } else if !seen_helo {
                        return End::Policy("first frame must be HELO".into());
                    }
                    let mut frame = Vec::with_capacity(8 + payload.len());
                    frame.extend_from_slice(&op);
                    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                    frame.extend_from_slice(&payload);
                    if let Err(e) = lms_wr.write_all(&frame).await {
                        return End::Error(e.to_string());
                    }
                }
                if from_client.buffered() > ClientFrameReader::MAX_FRAME + 8 {
                    return End::Policy("oversize frame".into());
                }
            }
            n = lms_rd.read(&mut buf) => {
                let n = match n {
                    Ok(0) => return End::LmsClosed,
                    Ok(n) => n,
                    Err(e) => return End::Error(e.to_string()),
                };
                from_lms.push(&buf[..n]);
                loop {
                    let (op, payload) = match from_lms.next_frame() {
                        Ok(Some(f)) => f,
                        Ok(None) => break,
                        Err(e) => return End::Error(format!("LMS framing: {e}")),
                    };
                    match &op {
                        // Never let a remote device be redirected to another server.
                        b"serv" => {
                            warn!(mac = %device.mac_string(), "dropped serv from LMS");
                            continue;
                        }
                        b"strm" => gw.note_strm(id, device, &payload),
                        _ => {}
                    }
                    let frame = encode_server_frame(op, &payload);
                    if ws.send(Message::Binary(frame.into())).await.is_err() {
                        return End::ClientGone;
                    }
                }
            }
        }
    }
}

impl Gateway {
    /// Record the stream a `strm s` asks the player to fetch, if policy allows it.
    fn note_strm(&self, id: &str, device: &Device, payload: &[u8]) {
        let Ok(s) = Strm::parse(payload) else { return };
        if s.command != b's' {
            return;
        }
        let ip = Ipv4Addr::from(s.server_ip);
        let path = if ip.is_unspecified() || self.cfg.lms_ips.contains(&ip) {
            match validate_stream_request(&s.request, &device.mac) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(mac = %device.mac_string(), error = %e, "refused stream request");
                    None
                }
            }
        } else {
            warn!(mac = %device.mac_string(), %ip, "refused direct stream to non-LMS address");
            None
        };
        if let Some(sess) = self.sessions.lock().unwrap().get_mut(id) {
            sess.stream_seq += 1;
            sess.stream_path = path;
        }
    }
}

// ---------------------------------------------------------------------------
// Audio relay
// ---------------------------------------------------------------------------

async fn stream(
    State(gw): State<AppState>,
    Path((session, n)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(device) = token.and_then(|t| gw.devices.authenticate(t)) else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"lyrion-anywhere\"")],
            "unauthorised\n",
        )
            .into_response();
    };
    let path = {
        let sessions = gw.sessions.lock().unwrap();
        // Another device's session looks exactly like a missing one.
        let Some(sess) = sessions
            .get(&session)
            .filter(|s| Arc::ptr_eq(&s.device, &device))
        else {
            return StatusCode::NOT_FOUND.into_response();
        };
        match (n.parse::<u64>(), &sess.stream_path) {
            (Ok(n), Some(p)) if n == sess.stream_seq => p.clone(),
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    };

    let url = format!("http://{}{}", gw.cfg.lms_http, path);
    let resp = match gw.http.get(&url).send().await {
        Ok(r) if r.status() == reqwest::StatusCode::OK => r,
        Ok(r) => {
            warn!(status = %r.status(), "LMS stream status");
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(e) => {
            warn!(error = %e, "LMS stream fetch failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|ct| ct.starts_with("audio/") || *ct == "application/octet-stream")
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = Body::from_stream(
        resp.bytes_stream()
            .map(|r| r.map_err(std::io::Error::other)),
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap()
}
