//! Resource limits that protect the gateway itself.
//!
//! - [`serve`]: the HTTP server, with a time limit on request headers (which also ends
//!   idle keep-alive connections), a cap on open connections, and a per-address cap for
//!   peers that aren't the reverse proxy.
//! - [`body_deadline`]: request bodies (all small JSON) must arrive within a time limit.
//! - [`Limiter`]: requests in flight, per user and overall (artwork).
//!
//! WebSockets and streamed responses are not affected by the time limits: the header limit
//! applies only while a request head is awaited, and the body limit only to request bodies.
//! A WebSocket does count against the connection caps for as long as it is open: its
//! connection's [`ConnSlot`] is held by the socket's task (see [`ConnSlot`]).

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;
use tracing::{info, warn};

/// Largest request body read by [`body_deadline`] for any path: what no endpoint exceeds.
/// [`body_limit`] holds most paths to far less.
pub const MAX_BODY: usize = 1024 * 1024;
/// The body cap for paths that take small JSON (sign-in, account changes, devices, player
/// requests) and for any path not listed in [`body_limit`].
pub const SMALL_BODY: usize = 4096;
/// The body cap for `/v1/me/…`: the largest of those endpoints is an offline ratings batch.
pub const ME_BODY: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// Log lines anyone can cause
// ---------------------------------------------------------------------------

/// Log lines of one kind written per [`LOG_WINDOW`] at most, when anyone (an
/// unauthenticated request) can cause them; the rest are counted and the count is written
/// with the next line let through.
pub const LOG_BURST: u32 = 10;
pub const LOG_WINDOW: Duration = Duration::from_secs(60);
/// The longest request path written to the log, in bytes (review low: a refused request's
/// whole path, up to the 64 KiB a request head may hold, went to the log).
pub const LOG_PATH_MAX: usize = 120;

/// A budget for one kind of log line (see [`LOG_BURST`]): `static`, one per call site.
pub struct LogBudget(Mutex<Option<(std::time::Instant, u32, u64)>>);

impl LogBudget {
    pub const fn new() -> Self {
        LogBudget(Mutex::new(None))
    }

    /// Whether to write the line now, at `now`: `Some(n)` with the lines suppressed since
    /// the last one written, `None` to drop it (it is counted).
    pub fn allow(&self, now: std::time::Instant) -> Option<u64> {
        let mut s = self.0.lock().unwrap();
        let (start, written, suppressed) = s.get_or_insert((now, 0, 0));
        if now.saturating_duration_since(*start) >= LOG_WINDOW {
            *start = now;
            *written = 0;
        }
        if *written >= LOG_BURST {
            *suppressed += 1;
            return None;
        }
        *written += 1;
        Some(std::mem::take(suppressed))
    }
}

impl Default for LogBudget {
    fn default() -> Self {
        Self::new()
    }
}

/// A request path cut to [`LOG_PATH_MAX`] bytes for the log (at a character boundary), with
/// `…` when it was cut.
pub fn log_path(path: &str) -> std::borrow::Cow<'_, str> {
    if path.len() <= LOG_PATH_MAX {
        return path.into();
    }
    let mut end = LOG_PATH_MAX;
    while !path.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &path[..end]).into()
}

// ---------------------------------------------------------------------------
// Concurrency limiter
// ---------------------------------------------------------------------------

/// Requests in flight: at most `per_key` for one key (a user) and `total` for everyone.
pub struct Limiter {
    per_key: usize,
    total: usize,
    state: Mutex<LimState>,
    freed: Notify,
}

#[derive(Default)]
struct LimState {
    total: usize,
    per: HashMap<i64, usize>,
}

/// A slot held in a [`Limiter`]; freed when dropped.
pub struct Permit {
    lim: Arc<Limiter>,
    key: i64,
}

impl Limiter {
    pub fn new(per_key: usize, total: usize) -> Arc<Self> {
        Arc::new(Limiter {
            per_key: per_key.max(1),
            total: total.max(1),
            state: Mutex::default(),
            freed: Notify::new(),
        })
    }

    fn try_take(&self, key: i64) -> bool {
        let mut st = self.state.lock().unwrap();
        let mine = st.per.get(&key).copied().unwrap_or(0);
        if st.total >= self.total || mine >= self.per_key {
            return false;
        }
        st.total += 1;
        st.per.insert(key, mine + 1);
        true
    }

    /// A slot for `key`, waiting up to `wait` for one to come free.
    pub async fn acquire(self: &Arc<Self>, key: i64, wait: Duration) -> Option<Permit> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Register for wake-ups before looking, so a slot freed in between isn't missed.
            let freed = self.freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();
            if self.try_take(key) {
                return Some(Permit {
                    lim: self.clone(),
                    key,
                });
            }
            if tokio::time::timeout_at(deadline, freed).await.is_err() {
                return None;
            }
        }
    }

    /// A slot for `key` if one is free now (never waits).
    pub fn try_acquire(self: &Arc<Self>, key: i64) -> Option<Permit> {
        self.try_take(key).then(|| Permit {
            lim: self.clone(),
            key,
        })
    }

    /// Slots in use (for tests and diagnostics).
    pub fn in_use(&self) -> usize {
        self.state.lock().unwrap().total
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        {
            let mut st = self.lim.state.lock().unwrap();
            st.total = st.total.saturating_sub(1);
            if let Some(n) = st.per.get_mut(&self.key) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    st.per.remove(&self.key);
                }
            }
        }
        self.lim.freed.notify_waiters();
    }
}

// ---------------------------------------------------------------------------
// Request body deadline
// ---------------------------------------------------------------------------

/// Middleware: a request body must arrive whole within the time limit (the state), or the
/// request gets 408 and the connection is closed. Bodies are at most [`MAX_BODY`] (413
/// beyond). GET, HEAD and OPTIONS requests (including WebSocket upgrades) pass untouched.
pub async fn body_deadline(State(limit): State<Duration>, req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let max = body_limit(req.uri().path());
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > max as u64) {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "request too large\n");
    }
    let (parts, body) = req.into_parts();
    match tokio::time::timeout(limit, axum::body::to_bytes(body, max)).await {
        Err(_) => {
            static SLOW: LogBudget = LogBudget::new();
            if let Some(suppressed) = SLOW.allow(std::time::Instant::now()) {
                warn!(path = %log_path(parts.uri.path()), suppressed, "request body too slow");
            }
            refuse(StatusCode::REQUEST_TIMEOUT, "request body too slow\n")
        }
        Ok(Err(_)) => refuse(StatusCode::PAYLOAD_TOO_LARGE, "request too large\n"),
        Ok(Ok(bytes)) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
    }
}

/// The largest body read for a request to `path`, before any handler (or authentication)
/// runs: what the endpoints there take, so an oversize body is refused as soon as it passes
/// that (review lows: `/v1/rpc`, then every other path, read up to [`MAX_BODY`] first). A
/// path not listed (sign-in, account changes, devices, players, and anything unknown)
/// gets [`SMALL_BODY`].
pub fn body_limit(path: &str) -> usize {
    match path {
        "/v1/rpc" => crate::rpc::MAX_BODY,
        "/v1/diagnostics" => crate::diagnostics::MAX_BODY,
        "/v1/plays" => crate::plays::MAX_BODY,
        p if p.starts_with("/v1/me/") => ME_BODY,
        _ => SMALL_BODY,
    }
}

// Every endpoint's own body cap fits under its path's in `body_limit`, and every path's
// under `MAX_BODY`: checked when the gateway is built, so a handler that comes to take more
// can't be cut short here unnoticed. (`ratings`' batch cap is private to its module; the
// test `body_caps_follow_the_endpoints` checks it.)
const _: () = {
    assert!(crate::auth::MAX_BODY <= SMALL_BODY);
    assert!(crate::alarms::MAX_BODY <= SMALL_BODY);
    assert!(crate::queue_edit::MAX_BODY <= SMALL_BODY);
    assert!(crate::queue_save::MAX_BODY <= SMALL_BODY);
    assert!(crate::home::MAX_BYTES <= ME_BODY);
    assert!(crate::stations::MAX_BODY <= ME_BODY);
    assert!(crate::filters::MAX_BODY <= ME_BODY);
    assert!(crate::rpc::MAX_BODY <= MAX_BODY);
    assert!(crate::diagnostics::MAX_BODY <= MAX_BODY);
    assert!(crate::plays::MAX_BODY <= MAX_BODY);
    assert!(ME_BODY <= MAX_BODY);
};

fn refuse(status: StatusCode, msg: &'static str) -> Response {
    let mut r = (status, msg).into_response();
    r.headers_mut().insert(
        header::CONNECTION,
        header::HeaderValue::from_static("close"),
    );
    r
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Connection limits for [`serve`].
#[derive(Debug, Clone)]
pub struct ServeLimits {
    /// Longest a client may take to send a request head, and longest an idle keep-alive
    /// connection stays open.
    pub header_timeout: Duration,
    /// Open connections at once; beyond this, new ones wait in the listen queue.
    pub max_connections: usize,
    /// Open connections from one address, except loopback and trusted proxies (behind a
    /// proxy every connection comes from the proxy, so the proxy must limit its clients).
    pub per_ip_connections: usize,
    /// Peers exempt from the per-address cap.
    pub trusted_proxies: Vec<IpAddr>,
    /// How long shutdown waits for open connections to finish.
    pub shutdown_grace: Duration,
}

impl Default for ServeLimits {
    fn default() -> Self {
        ServeLimits {
            header_timeout: Duration::from_secs(15),
            max_connections: 1024,
            per_ip_connections: 64,
            trusted_proxies: vec![],
            shutdown_grace: Duration::from_secs(10),
        }
    }
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

/// One connection counted against its address; uncounted when dropped.
/// A connection's places under the connection caps, put in each request's extensions by
/// [`serve`]. Hyper hands an upgraded connection (a WebSocket) to its handler and ends the
/// HTTP connection, so an upgrade handler keeps a clone of this for the life of the socket;
/// otherwise WebSockets would escape the caps.
#[derive(Clone)]
pub struct ConnSlot(#[allow(dead_code)] Arc<(tokio::sync::OwnedSemaphorePermit, Option<IpSlot>)>);

struct IpSlot {
    map: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl IpSlot {
    fn take(map: &Arc<Mutex<HashMap<IpAddr, usize>>>, ip: IpAddr, cap: usize) -> Option<Self> {
        let mut m = map.lock().unwrap();
        let n = m.entry(ip).or_default();
        if *n >= cap {
            return None;
        }
        *n += 1;
        Some(IpSlot {
            map: map.clone(),
            ip,
        })
    }
}

impl Drop for IpSlot {
    fn drop(&mut self) {
        let mut m = self.map.lock().unwrap();
        if let Some(n) = m.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                m.remove(&self.ip);
            }
        }
    }
}

/// Serve `app` on `listener` (HTTP/1.1, with WebSocket upgrades) until `shutdown`
/// completes, within `limits`. Handlers see the peer address as `ConnectInfo<SocketAddr>`.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    limits: ServeLimits,
    shutdown: impl Future<Output = ()> + Send + 'static,
) {
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_timeout)
        .max_buf_size(64 * 1024);
    let max = limits.max_connections.max(1);
    let conns = Arc::new(Semaphore::new(max));
    let per_ip: Arc<Mutex<HashMap<IpAddr, usize>>> = Default::default();
    let trusted: Vec<IpAddr> = limits
        .trusted_proxies
        .iter()
        .map(|i| canonical(*i))
        .collect();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::pin!(shutdown);
    let mut full_warned = false;
    loop {
        if conns.available_permits() == 0 && !full_warned {
            warn!(
                max = limits.max_connections,
                "connection limit reached; new connections wait"
            );
            full_warned = true;
        }
        let permit = tokio::select! {
            p = conns.clone().acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => break,
            },
            _ = &mut shutdown => break,
        };
        let (stream, peer): (tokio::net::TcpStream, SocketAddr) = tokio::select! {
            r = listener.accept() => match r {
                Ok(x) => x,
                Err(e) => {
                    // Out of file descriptors and the like: back off rather than spin.
                    warn!(error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            _ = &mut shutdown => break,
        };
        if conns.available_permits() > 0 {
            full_warned = false;
        }
        let ip = canonical(peer.ip());
        let ip_slot = if ip.is_loopback() || trusted.contains(&ip) {
            None
        } else {
            match IpSlot::take(&per_ip, ip, limits.per_ip_connections.max(1)) {
                Some(s) => Some(s),
                None => {
                    static CROWDED: LogBudget = LogBudget::new();
                    if let Some(suppressed) = CROWDED.allow(std::time::Instant::now()) {
                        warn!(peer = %ip, suppressed, "too many connections from one address");
                    }
                    drop(stream);
                    continue;
                }
            }
        };
        let _ = stream.set_nodelay(true);
        let app = app.clone();
        let slot = ConnSlot(Arc::new((permit, ip_slot)));
        let held = slot.clone();
        let svc = hyper::service::service_fn(move |mut req: Request<hyper::body::Incoming>| {
            req.extensions_mut().insert(ConnectInfo(peer));
            req.extensions_mut().insert(held.clone());
            app.clone().oneshot(req.map(Body::new))
        });
        let conn = builder
            .serve_connection(TokioIo::new(stream), svc)
            .with_upgrades();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            tokio::pin!(conn);
            tokio::select! {
                _ = conn.as_mut() => {}
                _ = async { let _ = stop.wait_for(|s| *s).await; } => {
                    conn.as_mut().graceful_shutdown();
                    let _ = conn.await;
                }
            }
            drop(slot);
        });
    }
    drop(listener);
    let _ = stop_tx.send(true);
    info!("shutting down: waiting for open connections");
    if tokio::time::timeout(limits.shutdown_grace, conns.acquire_many(max as u32))
        .await
        .is_err()
    {
        info!("open connections cut at shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn limiter_caps_per_key_and_total() {
        let lim = Limiter::new(2, 3);
        let w = Duration::from_millis(20);
        let a1 = lim.acquire(1, w).await.unwrap();
        let _a2 = lim.acquire(1, w).await.unwrap();
        assert!(lim.acquire(1, w).await.is_none(), "per-key cap");
        let _b1 = lim.acquire(2, w).await.unwrap();
        assert!(lim.acquire(3, w).await.is_none(), "total cap");
        assert_eq!(lim.in_use(), 3);
        drop(a1);
        assert!(lim.acquire(3, w).await.is_some(), "a freed slot is reused");
    }

    #[tokio::test]
    async fn a_waiter_gets_the_next_free_slot() {
        let lim = Limiter::new(1, 1);
        let held = lim.acquire(1, Duration::ZERO).await.unwrap();
        let l2 = lim.clone();
        let waiter = tokio::spawn(async move { l2.acquire(1, Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(held);
        let got = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter woke")
            .unwrap();
        assert!(got.is_some());
    }

    /// Review low: a body was read up to 1 MiB for any path before authentication. Each
    /// path's cap is now what its endpoints take, and never less.
    #[test]
    fn body_caps_follow_the_endpoints() {
        assert_eq!(body_limit("/v1/rpc"), crate::rpc::MAX_BODY);
        assert_eq!(body_limit("/v1/diagnostics"), crate::diagnostics::MAX_BODY);
        assert_eq!(body_limit("/v1/plays"), crate::plays::MAX_BODY);
        for small in [
            "/v1/auth/login",
            "/v1/auth/pair",
            "/v1/auth/refresh",
            "/v1/account/password",
            "/v1/account/totp/setup",
            "/v1/devices/pair",
            "/v1/devices/self",
            "/v1/players/aa:bb:cc:dd:ee:ff/alarms",
            "/v1/players/aa:bb:cc:dd:ee:ff/queue/edit",
            "/v1/players/aa:bb:cc:dd:ee:ff/queue/save",
            "/nothing/here",
            "/",
            "/v1/rpc/",
            "/v1/me",
        ] {
            assert_eq!(body_limit(small), SMALL_BODY, "{small}");
        }
        assert_eq!(body_limit("/v1/me/ratings"), ME_BODY);
        // The other endpoints' own caps are checked against these when the gateway is built
        // (the `const _` beside `body_limit`); the offline ratings batch's is private to its
        // module.
        let ratings_batch = 128 * 1024;
        assert!(ratings_batch <= body_limit("/v1/me/ratings"));
    }

    #[test]
    fn log_lines_anyone_can_cause_are_rationed() {
        let b = LogBudget::new();
        let t0 = std::time::Instant::now();
        for _ in 0..LOG_BURST {
            assert_eq!(b.allow(t0), Some(0));
        }
        for _ in 0..1000 {
            assert_eq!(b.allow(t0 + Duration::from_secs(1)), None);
        }
        // The next window writes again, saying how many were dropped.
        assert_eq!(b.allow(t0 + LOG_WINDOW), Some(1000));
        assert_eq!(b.allow(t0 + LOG_WINDOW), Some(0));
    }

    #[test]
    fn logged_paths_are_cut_short() {
        assert_eq!(log_path("/v1/rpc"), "/v1/rpc");
        let long = format!("/{}", "a".repeat(10_000));
        let cut = log_path(&long);
        assert_eq!(cut.len(), LOG_PATH_MAX + '…'.len_utf8());
        assert!(cut.ends_with('…'));
        // Never mid-character, whatever sits at the cut.
        for pad in 0..4 {
            let s = format!("{}{}", "x".repeat(LOG_PATH_MAX - pad), "é".repeat(10));
            let cut = log_path(&s);
            assert!(cut.len() <= LOG_PATH_MAX + '…'.len_utf8(), "{pad}");
        }
    }

    #[test]
    fn ip_slots_are_capped_and_released() {
        let map = Arc::new(Mutex::new(HashMap::new()));
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let a = IpSlot::take(&map, ip, 2).unwrap();
        let _b = IpSlot::take(&map, ip, 2).unwrap();
        assert!(IpSlot::take(&map, ip, 2).is_none());
        let other: IpAddr = "192.0.2.2".parse().unwrap();
        assert!(IpSlot::take(&map, other, 2).is_some());
        drop(a);
        assert!(IpSlot::take(&map, ip, 2).is_some());
    }
}
