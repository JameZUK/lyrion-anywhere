//! Lyrion Anywhere gateway: the only internet-facing component.
//!
//! It authenticates devices, tunnels Slimproto over WebSocket with policy enforcement, and
//! relays audio streams. Nothing reaches LMS unless a handler here explicitly allows it.
//!
//! # `/v1/player`
//!
//! - client → gateway, first message (text), either a bare device token (new session) or
//!   `{"token":"…","resume":{"session":"…","secret":"…","received":n}}` to resume a session
//!   after a
//!   dropped connection. Then binary messages carrying the client → server Slimproto byte
//!   stream (any chunking). A new session must begin with HELO; a resumed one must not.
//!   The only text message allowed afterwards is a stream acknowledgement,
//!   `{"ack":{"stream":n,"bytes":b}}`, which clients send at least every 1 MiB received;
//!   the gateway stays at most [`Config::replay_window`] ahead of it.
//! - gateway → client: a text greeting
//!   `{"session":"…","secret":"…","mac":"02:…","name":"…","resumed":bool}` (the player
//!   identity to use in HELO and the resume secret), then one binary message per
//!   server → client Slimproto frame, length prefix included, exactly as LMS sent it.
//!
//! If the socket drops, the session (and LMS's connection) survives for
//! [`Config::resume_grace`]; frames from LMS are queued and delivered on resume. A new
//! session for the same device ends any previous one.
//!
//! # `/v1/stream/<session>/<n>[?offset=k]`
//!
//! Audio for the n-th `strm s` frame the client received (counting from 1), with the device
//! token as a bearer credential. The gateway replays only the request LMS itself issued,
//! after checking it is this player's stream. `offset` resumes a broken download.

pub mod accounts;
pub mod admin;
pub mod art;
mod auth;
pub mod events;
pub mod menu_actions;
mod relay;
mod rpc;
pub mod rpc_policy;
mod session;
pub mod totp;
mod tunnel;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tracing::warn;

use accounts::Accounts;
use session::Session;

#[derive(Debug, Clone)]
pub struct Config {
    /// LMS Slimproto and HTTP listeners (host:port).
    pub lms_slimproto: String,
    pub lms_http: String,
    /// LMS command-line interface (host:port), for notifications.
    pub lms_cli: String,
    /// Addresses LMS may name in `strm` server_ip (0.0.0.0 is always allowed).
    pub lms_ips: Vec<Ipv4Addr>,
    /// Extra `Origin` values (e.g. `https://music.example.com`) accepted on WebSocket
    /// upgrade. Same-origin and absent Origin (native clients) are always accepted.
    pub allowed_origins: Vec<String>,
    /// Reverse proxies allowed to set X-Forwarded-*. Loopback is always trusted.
    pub trusted_proxies: Vec<IpAddr>,
    pub auth_timeout: Duration,
    pub dial_timeout: Duration,
    /// How long a session (and its LMS connection) survives without a client socket.
    pub resume_grace: Duration,
    /// Bytes of LMS → client frames queued while no socket is attached; beyond this the
    /// session ends.
    pub detached_queue_limit: usize,
    /// Audio bytes kept per stream so a download can resume from a slightly earlier offset.
    pub replay_window: usize,
    /// Web app directory served at `/`, if any.
    pub static_dir: Option<PathBuf>,
    /// Ping interval on quiet WebSockets, so proxies with idle timeouts (Cloudflare drops
    /// after 100 s) keep them open.
    pub ws_keepalive: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            lms_slimproto: "127.0.0.1:3483".into(),
            lms_http: "127.0.0.1:9000".into(),
            lms_cli: "127.0.0.1:9090".into(),
            lms_ips: vec![],
            allowed_origins: vec![],
            trusted_proxies: vec![],
            auth_timeout: Duration::from_secs(5),
            dial_timeout: Duration::from_secs(5),
            resume_grace: Duration::from_secs(60),
            detached_queue_limit: 1024 * 1024,
            replay_window: 4 * 1024 * 1024,
            static_dir: None,
            ws_keepalive: Duration::from_secs(30),
        }
    }
}

pub struct Gateway {
    cfg: Config,
    accounts: Arc<Accounts>,
    http: reqwest::Client,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// App and radio commands LMS lists in its menus (the only plugin commands allowed).
    apps: Mutex<std::collections::HashSet<String>>,
    apps_refreshed: Mutex<Option<std::time::Instant>>,
    /// Menu actions LMS offered, per device (see [`menu_actions`]).
    menu_actions: Mutex<HashMap<i64, menu_actions::DeviceActions>>,
    /// Image URLs LMS has handed out (the only ones its image proxy may fetch for clients).
    known_images: Mutex<art::KnownImages>,
    events: Arc<events::Hub>,
}

pub type AppState = Arc<Gateway>;

impl Gateway {
    pub fn new(cfg: Config, accounts: Arc<Accounts>) -> AppState {
        let http = reqwest::Client::builder()
            .connect_timeout(cfg.dial_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("http client");
        Arc::new(Gateway {
            cfg,
            accounts,
            http,
            sessions: Mutex::new(HashMap::new()),
            apps: Mutex::new(Default::default()),
            apps_refreshed: Mutex::new(None),
            menu_actions: Mutex::new(HashMap::new()),
            known_images: Mutex::new(Default::default()),
            events: Arc::new(events::Hub::default()),
        })
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    fn remove_session(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }

    pub fn accounts(&self) -> &Arc<Accounts> {
        &self.accounts
    }

    fn apps(&self) -> std::collections::HashSet<String> {
        self.apps.lock().unwrap().clone()
    }

    fn add_apps(&self, names: Vec<String>) {
        self.apps.lock().unwrap().extend(names);
    }

    /// End any live session for these devices (after revocation).
    pub async fn end_sessions_for(&self, device_ids: &[i64]) {
        let doomed: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| device_ids.contains(&s.device.device_id))
            .cloned()
            .collect();
        for s in doomed {
            s.end(self, session::EndReason::Revoked).await;
        }
    }
}

pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route("/v1/auth/login", post(auth::login))
        .route("/v1/auth/pair", post(auth::pair))
        .route("/v1/auth/refresh", post(auth::refresh))
        .route("/v1/auth/logout", post(auth::logout))
        .route("/v1/me", get(auth::me))
        .route("/v1/account", get(auth::account))
        .route("/v1/account/totp/setup", post(auth::totp_setup))
        .route("/v1/account/totp/confirm", post(auth::totp_confirm))
        .route("/v1/account/totp/disable", post(auth::totp_disable))
        .route("/v1/rpc", post(rpc::rpc))
        .route("/v1/art/{*path}", get(art::art))
        .route("/v1/events", get(events::events_upgrade))
        .route("/v1/player", get(tunnel::player_upgrade))
        .route("/v1/stream/{session}/{n}", get(relay::stream));
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
    // A handler may set a stricter policy of its own (e.g. artwork).
    if h.contains_key("content-security-policy") {
        return resp;
    }
    // Scripts and style sheets come only from this origin. Inline `style` attributes are
    // allowed (the web app's framework uses them); they hold no selectors, so they can't be
    // used to read page content the way injected style sheets can.
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; \
             connect-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; \
             style-src 'self'; \
             style-src-attr 'unsafe-inline'; \
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

/// The client's address for rate limiting and audit: the proxy-reported address when the
/// peer is a trusted proxy, otherwise the peer itself.
fn client_ip(cfg: &Config, peer: SocketAddr, headers: &HeaderMap) -> String {
    let ip = canonical(peer.ip());
    let trusted = ip.is_loopback() || cfg.trusted_proxies.iter().any(|p| canonical(*p) == ip);
    if trusted
        && let Some(first) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
        && let Ok(fwd) = first.trim().parse::<IpAddr>()
    {
        return fwd.to_string();
    }
    ip.to_string()
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

/// A random 128-bit identifier (hex).
pub fn random_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS random source");
    hex::encode(b)
}

/// Constant-time string comparison for secrets.
fn secret_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
