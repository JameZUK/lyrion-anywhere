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

/// Largest request body read by [`body_deadline`]. Every endpoint takes far less.
pub const MAX_BODY: usize = 1024 * 1024;

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
            warn!(path = %parts.uri.path(), "request body too slow");
            refuse(StatusCode::REQUEST_TIMEOUT, "request body too slow\n")
        }
        Ok(Err(_)) => refuse(StatusCode::PAYLOAD_TOO_LARGE, "request too large\n"),
        Ok(Ok(bytes)) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
    }
}

/// The largest body read for a request to `path`: [`MAX_BODY`], or less where the handler
/// takes less, so an oversize body is refused as soon as it passes that (review low:
/// `/v1/rpc` read up to [`MAX_BODY`] before refusing anything over its 8 KiB).
pub fn body_limit(path: &str) -> usize {
    match path {
        "/v1/rpc" => crate::rpc::MAX_BODY,
        _ => MAX_BODY,
    }
}

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
                    warn!(peer = %ip, "too many connections from one address");
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
