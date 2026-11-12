//! HTTP and TLS plumbing.
//!
//! TLS is rustls (with the `ring` provider, which cross-compiles cleanly for Android) and
//! `rustls-platform-verifier`: certificates are checked by the operating system's own
//! verifier and trust store (Android's TrustManager, Apple's Security framework), so
//! user-installed CAs and revocation work as the platform intends (except revocation on
//! Android, see `crate::tls`), and no root list is bundled. Redirects are never followed
//! (a bearer token must not travel anywhere else), and responses are read up to a fixed
//! limit, with time limits on every step and on the whole body.
//!
//! The client is hyper's, used directly rather than through reqwest: the gateway's own
//! reqwest client has no TLS, and Cargo would unify a TLS feature into it.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER,
    USER_AGENT,
};
use hyper::{Method, StatusCode};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rustls::ClientConfig;
use serde_json::Value;
use url::Url;

use crate::error::ApiError;

pub(crate) const CLIENT_HEADER: &str = "x-anywhere-client";
/// JSON answers: the gateway relays LMS answers of up to 16 MiB.
pub(crate) const MAX_JSON: usize = 16 * 1024 * 1024;
const AGENT: &str = concat!("LyrionAnywhere-Mobile/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Until the response headers arrive.
const HEADERS_TIMEOUT: Duration = Duration::from_secs(30);
/// Between two pieces of a response body.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// For a whole response body read with [`Response::read_limited`]: a gateway sending a
/// little at a time, each piece within [`READ_TIMEOUT`], must not hold a read (of up to
/// [`MAX_JSON`]) open indefinitely. Three minutes carry the largest answer over a slow
/// mobile connection (about 1 Mbit/s).
const READ_DEADLINE: Duration = Duration::from_secs(180);

/// An error and its causes, as one short line ("client error (Connect): dns error: …"),
/// for diagnosing a failed connection. Network-layer errors name hosts, addresses and
/// certificate problems, never request contents, so this holds no tokens or passwords.
pub(crate) fn causes(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(c) = cur {
        let t = c.to_string();
        if parts.last() != Some(&t) && parts.len() < 6 {
            parts.push(t);
        }
        cur = c.source();
    }
    let mut out = parts.join(": ");
    if out.len() > 300 {
        let cut = (0..=300)
            .rev()
            .find(|&i| out.is_char_boundary(i))
            .unwrap_or(0);
        out.truncate(cut);
        out.push('…');
    }
    out
}

#[cfg(test)]
mod causes_tests {
    use super::causes;

    #[derive(Debug)]
    struct E(&'static str, Option<Box<E>>);
    impl std::fmt::Display for E {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for E {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|e| e as _)
        }
    }

    #[test]
    fn the_chain_is_joined_short_and_without_repeats() {
        let e = E(
            "client error (Connect)",
            Some(Box::new(E(
                "dns error",
                Some(Box::new(E("dns error", None))),
            ))),
        );
        assert_eq!(causes(&e), "client error (Connect): dns error");
        let long = E(Box::leak("x".repeat(1000).into_boxed_str()), None);
        assert!(causes(&long).chars().count() <= 301);
        let multibyte = E(Box::leak("é".repeat(400).into_boxed_str()), None);
        assert!(causes(&multibyte).ends_with('…'));
    }
}

/// The runtime the shared client's connections and the token rotations run on: one per
/// process, made on first use and never dropped (review 2026-11-12, M17).
///
/// The player engine and the downloader run on private runtimes of their own, which are
/// dropped when they stop or the network changes. A rotation spawned there was cancelled
/// with it, after the gateway had rotated, so the old token stayed stored and the next
/// refresh looked like theft (the device revoked, its downloads wiped); and connections
/// they opened in the shared pool died with them, or stalled whenever their thread was busy.
pub(crate) fn background() -> &'static tokio::runtime::Handle {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("anywhere-net")
            .enable_all()
            .build()
            .expect("the network runtime")
    })
    .handle()
}

/// A task on another runtime that is aborted when whoever awaits it goes away, so a
/// request the caller gave up on stops as it did when it ran in the caller's own task.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

/// The TLS configuration without ALPN, built once per process: the platform verifier and
/// (on Android) the system CA store it falls back on are made once, not on every connect.
fn base_tls() -> Result<ClientConfig, ApiError> {
    static BASE: std::sync::Mutex<Option<ClientConfig>> = std::sync::Mutex::new(None);
    let mut base = BASE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = base.as_ref() {
        return Ok(c.clone());
    }
    // A failure is not kept: the platform may not be ready yet (Android initialises the
    // verifier from the app), and the next connect tries again.
    let cfg = build_tls()?;
    *base = Some(cfg.clone());
    Ok(cfg)
}

fn build_tls() -> Result<ClientConfig, ApiError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let unavailable = |e: rustls::Error| {
        ApiError::network(format!(
            "TLS is not available on this device: {}",
            causes(&e)
        ))
    };
    // The platform's verifier, with a second opinion for Android's unreliable "revoked"
    // verdict (see `crate::tls`).
    let verifier = crate::tls::verifier(provider.clone()).map_err(unavailable)?;
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(unavailable)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

/// The platform-verified TLS configuration shared by HTTP and WebSocket connections.
pub(crate) fn tls_config(alpn: &[&[u8]]) -> Result<ClientConfig, ApiError> {
    let mut cfg = base_tls()?;
    cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(cfg)
}

/// The TLS configuration for WebSocket connections (HTTP/1.1), made once per process
/// (review 2026-11-12: every connect used to rebuild the verifier and re-read the CA store).
pub(crate) fn ws_tls_config() -> Result<Arc<ClientConfig>, ApiError> {
    static WS: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(c) = WS.get() {
        return Ok(c.clone());
    }
    let cfg = Arc::new(tls_config(&[b"http/1.1"])?);
    Ok(WS.get_or_init(|| cfg).clone())
}

type Inner = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// An HTTP client and its connection pool.
#[derive(Clone)]
pub(crate) struct Http {
    inner: Inner,
    /// Requests run on [`background`] (the shared client), so its pooled connections never
    /// belong to a runtime that is dropped or busy; a private client runs them on the
    /// caller's runtime, which owns its pool.
    background: bool,
}

impl Http {
    /// The process's shared client. Its requests run on [`background`], whichever runtime
    /// the caller is on.
    pub(crate) fn shared() -> Result<Http, ApiError> {
        static CLIENT: OnceLock<Http> = OnceLock::new();
        if let Some(c) = CLIENT.get() {
            return Ok(c.clone());
        }
        let client = Http::build(true)?;
        Ok(CLIENT.get_or_init(|| client).clone())
    }

    /// A client with its own connection pool, for a component that runs on its own
    /// runtime (the player engine, the downloader): its pooled connections belong to that
    /// runtime and go with it, and never reach the shared pool.
    pub(crate) fn private() -> Result<Http, ApiError> {
        Http::build(false)
    }

    fn build(background: bool) -> Result<Http, ApiError> {
        let mut tcp = HttpConnector::new();
        tcp.enforce_http(false);
        tcp.set_connect_timeout(Some(CONNECT_TIMEOUT));
        tcp.set_nodelay(true);
        // Plain HTTP is allowed only for loopback servers and literal private addresses;
        // `Server` checks that (`server::parse_server_url`).
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config(&[])?)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(tcp);
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(30))
            .build(https);
        Ok(Http {
            inner: client,
            background,
        })
    }

    pub(crate) fn get(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::GET, url)
    }

    pub(crate) fn post(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::POST, url)
    }

    pub(crate) fn patch(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::PATCH, url)
    }

    pub(crate) fn put(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::PUT, url)
    }

    pub(crate) fn delete(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::DELETE, url)
    }
}

/// A request being built (a deliberately small subset of what HTTP allows).
pub(crate) struct Request {
    http: Http,
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Bytes,
}

impl Request {
    fn new(http: Http, method: Method, url: Url) -> Request {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(AGENT));
        Request {
            http,
            method,
            url,
            headers,
            body: Bytes::new(),
        }
    }

    pub(crate) fn header(mut self, name: &'static str, value: &'static str) -> Request {
        self.headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
        self
    }

    /// A header built at run time (e.g. `Range`); a value that isn't a valid header is
    /// not sent.
    pub(crate) fn header_value(mut self, name: HeaderName, value: &str) -> Request {
        if let Ok(v) = HeaderValue::from_str(value) {
            self.headers.insert(name, v);
        }
        self
    }

    /// `token` has been checked to be printable ASCII; anything else sends no header (and
    /// the server answers 401).
    pub(crate) fn bearer_auth(mut self, token: &str) -> Request {
        if let Ok(mut v) = HeaderValue::from_str(&format!("Bearer {token}")) {
            v.set_sensitive(true);
            self.headers.insert(AUTHORIZATION, v);
        }
        self
    }

    pub(crate) fn json(mut self, body: &Value) -> Request {
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        self.body = Bytes::from(serde_json::to_vec(body).expect("JSON encodes"));
        self
    }

    pub(crate) async fn send(self) -> Result<Response, ApiError> {
        self.send_within(HEADERS_TIMEOUT).await
    }

    /// Send, waiting up to `headers_timeout` for the response headers (a transcoded
    /// download only starts once the server has made it).
    pub(crate) async fn send_within(self, headers_timeout: Duration) -> Result<Response, ApiError> {
        let mut req = hyper::Request::builder()
            .method(self.method)
            .uri(self.url.as_str())
            .body(Full::new(self.body))
            .map_err(|_| ApiError::invalid("bad request"))?;
        *req.headers_mut() = self.headers;
        let client = self.http.inner.clone();
        let exchange =
            async move { tokio::time::timeout(headers_timeout, client.request(req)).await };
        let answer = if self.http.background {
            // The connection (and the task that hands it back to the pool) is made inside
            // this task, so it lives on the long-lived runtime.
            match AbortOnDrop(background().spawn(exchange)).await {
                Ok(a) => a,
                Err(_) => return Err(ApiError::network("connection interrupted")),
            }
        } else {
            exchange.await
        };
        match answer {
            Ok(Ok(resp)) => Ok(Response(resp)),
            Ok(Err(e)) if e.is_connect() => Err(ApiError::network(format!(
                "could not connect: {}",
                causes(&e)
            ))),
            Ok(Err(e)) => Err(ApiError::network(format!(
                "connection failed: {}",
                causes(&e)
            ))),
            Err(_) => Err(ApiError::network("timed out")),
        }
    }
}

pub(crate) struct Response(hyper::Response<Incoming>);

impl Response {
    pub(crate) fn status(&self) -> StatusCode {
        self.0.status()
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        self.0.headers()
    }

    fn content_length(&self) -> Option<u64> {
        self.headers()
            .get(hyper::header::CONTENT_LENGTH)?
            .to_str()
            .ok()?
            .parse()
            .ok()
    }

    /// The body, for reading piece by piece (audio streams).
    pub(crate) fn into_body(self) -> Incoming {
        self.0.into_body()
    }

    /// Read the body up to `limit` bytes; more than that is an error, never a truncated
    /// answer. The whole read must be done within [`READ_DEADLINE`].
    pub(crate) async fn read_limited(self, limit: usize) -> Result<Vec<u8>, ApiError> {
        self.read_limited_within(limit, READ_DEADLINE).await
    }

    /// [`Response::read_limited`], all of it within `deadline`.
    async fn read_limited_within(
        self,
        limit: usize,
        deadline: Duration,
    ) -> Result<Vec<u8>, ApiError> {
        match tokio::time::timeout(deadline, self.read_body(limit)).await {
            Ok(r) => r,
            Err(_) => Err(ApiError::network("timed out")),
        }
    }

    async fn read_body(self, limit: usize) -> Result<Vec<u8>, ApiError> {
        let status = self.status().as_u16();
        let too_large = || ApiError::Server {
            status,
            detail: "answer too large".into(),
        };
        if self.content_length().is_some_and(|n| n > limit as u64) {
            return Err(too_large());
        }
        let mut body = self.0.into_body();
        let mut out = Vec::new();
        loop {
            let frame = match tokio::time::timeout(READ_TIMEOUT, body.frame()).await {
                Ok(Some(Ok(f))) => f,
                Ok(None) => return Ok(out),
                Ok(Some(Err(_))) => return Err(ApiError::network("connection interrupted")),
                Err(_) => return Err(ApiError::network("timed out")),
            };
            if let Ok(data) = frame.into_data() {
                if out.len() + data.len() > limit {
                    return Err(too_large());
                }
                out.extend_from_slice(&data);
            }
        }
    }
}

/// The gateway's error text and flags from a failed response.
fn error_of(status: u16, headers: &HeaderMap, body: &[u8]) -> ApiError {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let retry_after = headers
        .get(RETRY_AFTER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    // The music server is busy (the gateway refused at once): only a 503 that says so.
    if status == 503 && v.get("busy") == Some(&Value::Bool(true)) {
        return ApiError::busy(retry_after);
    }
    ApiError::from_status(
        status,
        v.get("error").and_then(Value::as_str),
        v.get("totp_required").and_then(Value::as_bool) == Some(true),
        retry_after,
    )
}

/// Map a failed response (any non-2xx, including redirects, which are never followed).
pub(crate) async fn error_from_response(resp: Response) -> ApiError {
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    // Error bodies are small; don't read a large one.
    let body = resp.read_limited(64 * 1024).await.unwrap_or_default();
    error_of(status, &headers, &body)
}

/// Send a request and decode a JSON answer (`Value::Null` for 204). Failures become the
/// matching [`ApiError`].
pub(crate) async fn send_json(req: Request) -> Result<Value, ApiError> {
    let mut req = req;
    req.headers
        .insert(ACCEPT, HeaderValue::from_static("application/json"));
    let resp = req.send().await?;
    let status = resp.status();
    if status == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    if !status.is_success() {
        return Err(error_from_response(resp).await);
    }
    let body = resp.read_limited(MAX_JSON).await?;
    serde_json::from_slice(&body).map_err(|_| ApiError::bad_response("malformed answer"))
}

/// How a sign-in exchange (refresh, sign-out) failed, read strictly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthFailure {
    /// The gateway itself refused (its own 401, with its JSON `error`): this device's
    /// sign-in has ended there. `expired`: it ran out (the 401 says `"reason": "expired"`),
    /// rather than being revoked; the app then keeps its downloads (SPEC §8.5).
    Ended { expired: bool },
    /// Anything else: the gateway could not be reached or did not answer for itself. The
    /// sign-in is kept.
    Failed(ApiError),
}

/// A sign-in exchange (`/v1/auth/refresh`, `/v1/auth/logout`): like [`send_json`], but only
/// the gateway's own 401 (with its JSON `error`) is a refusal. Anything else in the way (a
/// proxy's or CDN's 400/401/403/404 page during maintenance, a challenge page) is a failure
/// to reach the gateway, so the sign-in, and with it the downloads, is kept.
pub(crate) async fn send_auth(req: Request) -> Result<Value, AuthFailure> {
    let mut req = req;
    req.headers
        .insert(ACCEPT, HeaderValue::from_static("application/json"));
    let resp = req.send().await.map_err(AuthFailure::Failed)?;
    let status = resp.status();
    if status == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    if status.is_success() {
        let body = resp
            .read_limited(MAX_JSON)
            .await
            .map_err(AuthFailure::Failed)?;
        return serde_json::from_slice(&body)
            .map_err(|_| AuthFailure::Failed(ApiError::bad_response("malformed answer")));
    }
    let headers = resp.headers().clone();
    let body = resp.read_limited(64 * 1024).await.unwrap_or_default();
    Err(auth_error_of(status.as_u16(), &headers, &body))
}

fn auth_error_of(status: u16, headers: &HeaderMap, body: &[u8]) -> AuthFailure {
    let v = serde_json::from_slice::<Value>(body).ok();
    let from_gateway = v
        .as_ref()
        .is_some_and(|v| v.get("error").is_some_and(Value::is_string));
    match error_of(status, headers, body) {
        ApiError::Unauthorised if from_gateway => AuthFailure::Ended {
            expired: v
                .as_ref()
                .and_then(|v| v.get("reason"))
                .and_then(Value::as_str)
                == Some("expired"),
        },
        e @ (ApiError::RateLimited { .. } | ApiError::Network { .. }) => AuthFailure::Failed(e),
        _ => AuthFailure::Failed(ApiError::Server {
            status,
            detail: format!("HTTP {status} from the sign-in service"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_gateways_own_401_ends_a_sign_in_on_refresh() {
        let h = HeaderMap::new();
        let gw = br#"{"error":"invalid credentials"}"#;
        assert_eq!(
            auth_error_of(401, &h, gw),
            AuthFailure::Ended { expired: false }
        );
        // In the way: a proxy, a CDN challenge, maintenance pages. Never a revocation.
        for (status, body) in [
            (401, &b"<html>Unauthorized</html>"[..]),
            (401, &b""[..]),
            (401, &br#"{"message":"nope"}"#[..]),
            (401, &br#"{"reason":"expired"}"#[..]),
            (401, &br#"{"error":7,"reason":"expired"}"#[..]),
            (400, &br#"{"error":"malformed request"}"#[..]),
            (403, &br#"{"error":"forbidden"}"#[..]),
            (403, &br#"{"error":"forbidden","reason":"expired"}"#[..]),
            (403, &b"<html>Attention required</html>"[..]),
            (404, &b"Not found"[..]),
            (409, &b""[..]),
            (421, &b""[..]),
            (451, &b""[..]),
            (502, &b"Bad gateway"[..]),
        ] {
            let e = auth_error_of(status, &h, body);
            assert!(
                matches!(e, AuthFailure::Failed(ApiError::Server { .. })),
                "{status}: {e:?} must not end the sign-in"
            );
        }
        assert!(matches!(
            auth_error_of(429, &h, b""),
            AuthFailure::Failed(ApiError::RateLimited { .. })
        ));
        assert!(matches!(
            auth_error_of(408, &h, b""),
            AuthFailure::Failed(ApiError::Network { .. })
        ));
    }

    /// M18: the gateway names an expired sign-in (`"reason": "expired"` on its own 401);
    /// every other refusal of its own, whatever else it says, is a revocation.
    #[test]
    fn an_expired_sign_in_is_told_apart_from_a_revoked_one() {
        let h = HeaderMap::new();
        assert_eq!(
            auth_error_of(
                401,
                &h,
                br#"{"error":"invalid credentials","reason":"expired"}"#
            ),
            AuthFailure::Ended { expired: true }
        );
        for revoked in [
            &br#"{"error":"invalid credentials"}"#[..],
            br#"{"error":"invalid credentials","reason":"revoked"}"#,
            br#"{"error":"invalid credentials","reason":"EXPIRED"}"#,
            br#"{"error":"invalid credentials","reason":" expired"}"#,
            br#"{"error":"invalid credentials","reason":["expired"]}"#,
            br#"{"error":"invalid credentials","reason":{"expired":true}}"#,
            br#"{"error":"invalid credentials","reason":true}"#,
            br#"{"error":"invalid credentials","reason":null}"#,
            br#"{"error":"invalid credentials","expired":true}"#,
        ] {
            assert_eq!(
                auth_error_of(401, &h, revoked),
                AuthFailure::Ended { expired: false },
                "{}",
                String::from_utf8_lossy(revoked)
            );
        }
    }

    /// Arbitrary refusal bodies never panic and only ever end a sign-in on a 401.
    #[test]
    fn arbitrary_refusals_never_panic() {
        let alphabet = br#"{}[]":,errorasonxpid 0123456789\"#;
        let mut seed = 0x0bad_5eed_1234_5678u64;
        for round in 0..20_000u32 {
            let len = (seed % 80) as usize;
            let mut body = Vec::with_capacity(len);
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                body.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let status = [200, 204, 302, 400, 401, 403, 404, 429, 500, 503][round as usize % 10];
            let e = auth_error_of(status, &HeaderMap::new(), &body);
            if matches!(e, AuthFailure::Ended { .. }) {
                assert_eq!(status, 401);
            }
        }
    }

    /// Regression (review, 15 October): a gateway that answers at once and then sends its
    /// body a byte at a time, each piece well within the per-piece limit, held the read
    /// open indefinitely. The whole read now has a deadline.
    #[tokio::test]
    async fn a_trickling_answer_is_cut_off_at_the_overall_deadline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf).await;
                    let head = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                                 transfer-encoding: chunked\r\n\r\n";
                    if s.write_all(head).await.is_err() {
                        return;
                    }
                    while s.write_all(b"1\r\n \r\n").await.is_ok() {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                });
            }
        });
        let http = Http::private().unwrap();
        let url = Url::parse(&format!("http://{addr}/v1/me")).unwrap();
        let resp = http.get(url.clone()).send().await.unwrap();
        let t = std::time::Instant::now();
        let r = resp
            .read_limited_within(MAX_JSON, Duration::from_millis(500))
            .await;
        assert!(matches!(r, Err(ApiError::Network { .. })), "{r:?}");
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        // The deadline is far longer than a piece may take, and bounded.
        const { assert!(READ_DEADLINE.as_secs() > READ_TIMEOUT.as_secs()) };
        const { assert!(READ_DEADLINE.as_secs() <= 300) };
    }

    #[test]
    fn platform_tls_config_builds() {
        let cfg = tls_config(&[b"http/1.1"]).unwrap();
        assert_eq!(cfg.alpn_protocols, vec![b"http/1.1".to_vec()]);
        assert!(tls_config(&[]).unwrap().alpn_protocols.is_empty());
    }

    /// Review 2026-11-12: every WebSocket connect rebuilt the platform verifier (and on
    /// Android re-read the system CA store). It is built once per process now.
    #[test]
    fn the_tls_verifier_is_built_once_per_process() {
        for _ in 0..5 {
            tls_config(&[b"http/1.1"]).unwrap();
            tls_config(&[]).unwrap();
            Http::private().unwrap();
        }
        let (a, b) = (ws_tls_config().unwrap(), ws_tls_config().unwrap());
        assert!(Arc::ptr_eq(&a, &b), "one configuration for every connect");
        assert_eq!(a.alpn_protocols, vec![b"http/1.1".to_vec()]);
        assert_eq!(
            crate::tls::VERIFIERS_BUILT.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    /// M17: the shared client's requests run on the long-lived runtime, whatever runtime
    /// the caller is on; a private client's run on the caller's.
    #[test]
    fn shared_requests_run_on_the_long_lived_runtime() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let caller = rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            // A server on the caller's runtime that answers each request once.
            tokio::spawn(async move {
                while let Ok((mut s, _)) = l.accept().await {
                    tokio::spawn(async move {
                        let mut buf = [0u8; 4096];
                        while s.read(&mut buf).await.is_ok_and(|n| n > 0) {
                            let ok = b"HTTP/1.1 204 No Content\r\n\r\n";
                            if s.write_all(ok).await.is_err() {
                                return;
                            }
                        }
                    });
                }
            });
            let url = Url::parse(&format!("http://{addr}/v1/me")).unwrap();
            let shared = Http::shared().unwrap();
            assert!(shared.background && !Http::private().unwrap().background);
            let r = shared.get(url.clone()).send().await.unwrap();
            assert_eq!(r.status(), StatusCode::NO_CONTENT);
            let r = Http::private().unwrap().get(url).send().await.unwrap();
            assert_eq!(r.status(), StatusCode::NO_CONTENT);
            tokio::runtime::Handle::current().id()
        });
        assert_ne!(caller, background().id());
        // Dropping the caller's runtime leaves the long-lived one running.
        drop(rt);
        let (tx, rx) = std::sync::mpsc::channel();
        background().spawn(async move { tx.send(7).unwrap() });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(7),
            "the long-lived runtime still runs tasks"
        );
    }

    #[tokio::test]
    async fn client_builds_and_marks_tokens_sensitive() {
        let http = Http::shared().unwrap();
        let req = http
            .get(Url::parse("https://music.example.com/v1/me").unwrap())
            .bearer_auth("abc");
        assert!(req.headers[AUTHORIZATION].is_sensitive());
        // A token that can't be a header value is never sent.
        let req = http
            .get(Url::parse("https://music.example.com/v1/me").unwrap())
            .bearer_auth("a\r\nX-Evil: 1");
        assert!(!req.headers.contains_key(AUTHORIZATION));
    }

    #[test]
    fn a_503_saying_busy_is_the_music_server_being_busy() {
        let busy = br#"{"error":"music server is busy","busy":true}"#;
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(
            error_of(503, &h, busy),
            ApiError::ServerBusy {
                retry_after_secs: 2
            }
        );
        // Without Retry-After: a short default, never zero; an absurd one is bounded.
        assert_eq!(
            error_of(503, &HeaderMap::new(), busy),
            ApiError::ServerBusy {
                retry_after_secs: 2
            }
        );
        let mut zero = HeaderMap::new();
        zero.insert(RETRY_AFTER, HeaderValue::from_static("0"));
        assert_eq!(
            error_of(503, &zero, busy),
            ApiError::ServerBusy {
                retry_after_secs: 1
            }
        );
        let mut huge = HeaderMap::new();
        huge.insert(RETRY_AFTER, HeaderValue::from_static("99999999"));
        assert_eq!(
            error_of(503, &huge, busy),
            ApiError::ServerBusy {
                retry_after_secs: 300
            }
        );
        // A busy answer is never a refusal: the sign-in is kept.
        assert!(matches!(
            auth_error_of(503, &h, busy),
            AuthFailure::Failed(ApiError::ServerBusy { .. } | ApiError::Server { .. })
        ));

        // A 503 without `busy: true` keeps its old meaning.
        assert_eq!(
            error_of(503, &h, br#"{"error":"unavailable"}"#),
            ApiError::RateLimited {
                retry_after_secs: 2
            }
        );
        assert!(matches!(
            error_of(503, &HeaderMap::new(), br#"{"error":"unavailable"}"#),
            ApiError::Server { status: 503, .. }
        ));
        for not_busy in [
            &br#"{"busy":"true"}"#[..],
            br#"{"busy":1}"#,
            br#"{"busy":false}"#,
            br#"[{"busy":true}]"#,
            br#"{"error":{"busy":true}}"#,
            b"\xff\xfe<html>busy</html>",
            b"",
        ] {
            let e = error_of(503, &HeaderMap::new(), not_busy);
            assert!(
                matches!(e, ApiError::Server { status: 503, .. }),
                "{not_busy:?}: {e:?}"
            );
        }
        // Only a 503: `busy` on any other status changes nothing.
        assert_eq!(
            error_of(429, &h, busy),
            ApiError::RateLimited {
                retry_after_secs: 2
            }
        );
        assert!(matches!(
            error_of(502, &h, busy),
            ApiError::Server { status: 502, .. }
        ));
        assert!(matches!(
            error_of(500, &h, busy),
            ApiError::Server { status: 500, .. }
        ));
        assert_eq!(error_of(401, &h, busy), ApiError::Unauthorised);
        // Renewing a sign-in never ends it on a busy answer.
        assert!(!matches!(
            auth_error_of(503, &h, busy),
            AuthFailure::Ended { .. }
        ));
    }

    #[test]
    fn errors_carry_retry_after_and_two_step() {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_static("120"));
        assert_eq!(
            error_of(429, &h, br#"{"error":"too many attempts"}"#),
            ApiError::RateLimited {
                retry_after_secs: 120
            }
        );
        assert_eq!(
            error_of(
                401,
                &HeaderMap::new(),
                br#"{"error":"x","totp_required":true}"#
            ),
            ApiError::TwoStepRequired
        );
        // Garbage bodies still map by status.
        assert_eq!(
            error_of(401, &HeaderMap::new(), b"\xff\xfe<html>"),
            ApiError::Unauthorised
        );
        assert!(matches!(
            error_of(302, &HeaderMap::new(), b""),
            ApiError::Server { status: 302, .. }
        ));
    }
}
