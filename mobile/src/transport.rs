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

/// The platform-verified TLS configuration shared by HTTP and WebSocket connections.
pub(crate) fn tls_config(alpn: &[&[u8]]) -> Result<ClientConfig, ApiError> {
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
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(unavailable)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(cfg)
}

type Inner = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;

/// The HTTP client (one per process; it owns a connection pool).
#[derive(Clone)]
pub(crate) struct Http(Inner);

impl Http {
    pub(crate) fn shared() -> Result<Http, ApiError> {
        static CLIENT: OnceLock<Http> = OnceLock::new();
        if let Some(c) = CLIENT.get() {
            return Ok(c.clone());
        }
        let client = Http::build()?;
        Ok(CLIENT.get_or_init(|| client).clone())
    }

    /// A client with its own connection pool, for a component that runs on its own
    /// runtime (the player engine): pooled connections belong to the runtime that made
    /// them.
    pub(crate) fn private() -> Result<Http, ApiError> {
        Http::build()
    }

    fn build() -> Result<Http, ApiError> {
        let mut tcp = HttpConnector::new();
        tcp.enforce_http(false);
        tcp.set_connect_timeout(Some(CONNECT_TIMEOUT));
        tcp.set_nodelay(true);
        // Plain HTTP is allowed only for loopback servers; `Server` checks that.
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config(&[])?)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(tcp);
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(30))
            .build(https);
        Ok(Http(client))
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
        match tokio::time::timeout(headers_timeout, self.http.0.request(req)).await {
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

/// The refresh exchange: like [`send_json`], but only the gateway's own 401 (with its JSON
/// `error`) is a refusal. Anything else in the way (a proxy's or CDN's 400/403/404 page
/// during maintenance, a challenge page) is a failure to reach the gateway, so the sign-in,
/// and with it the downloads, is kept.
pub(crate) async fn send_refresh(req: Request) -> Result<Value, ApiError> {
    let mut req = req;
    req.headers
        .insert(ACCEPT, HeaderValue::from_static("application/json"));
    let resp = req.send().await?;
    let status = resp.status();
    if status.is_success() {
        let body = resp.read_limited(MAX_JSON).await?;
        return serde_json::from_slice(&body)
            .map_err(|_| ApiError::bad_response("malformed answer"));
    }
    let headers = resp.headers().clone();
    let body = resp.read_limited(64 * 1024).await.unwrap_or_default();
    Err(refresh_error_of(status.as_u16(), &headers, &body))
}

fn refresh_error_of(status: u16, headers: &HeaderMap, body: &[u8]) -> ApiError {
    let from_gateway = serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|v| v.get("error").is_some_and(Value::is_string));
    match error_of(status, headers, body) {
        ApiError::Unauthorised if from_gateway => ApiError::Unauthorised,
        e @ (ApiError::RateLimited { .. } | ApiError::Network { .. }) => e,
        _ => ApiError::Server {
            status,
            detail: format!("HTTP {status} while renewing the sign-in"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_gateways_own_401_ends_a_sign_in_on_refresh() {
        let h = HeaderMap::new();
        let gw = br#"{"error":"invalid credentials"}"#;
        assert_eq!(refresh_error_of(401, &h, gw), ApiError::Unauthorised);
        // In the way: a proxy, a CDN challenge, maintenance pages. Never a revocation.
        for (status, body) in [
            (401, &b"<html>Unauthorized</html>"[..]),
            (401, &b""[..]),
            (401, &br#"{"message":"nope"}"#[..]),
            (400, &br#"{"error":"malformed request"}"#[..]),
            (403, &br#"{"error":"forbidden"}"#[..]),
            (403, &b"<html>Attention required</html>"[..]),
            (404, &b"Not found"[..]),
            (409, &b""[..]),
            (421, &b""[..]),
            (451, &b""[..]),
            (502, &b"Bad gateway"[..]),
        ] {
            let e = refresh_error_of(status, &h, body);
            assert!(
                matches!(e, ApiError::Server { .. }),
                "{status}: {e:?} must not end the sign-in"
            );
            assert!(!matches!(e, ApiError::Unauthorised));
        }
        assert!(matches!(
            refresh_error_of(429, &h, b""),
            ApiError::RateLimited { .. }
        ));
        assert!(matches!(
            refresh_error_of(408, &h, b""),
            ApiError::Network { .. }
        ));
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
        assert!(!error_of(503, &h, busy).is_refusal());

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
            refresh_error_of(503, &h, busy),
            ApiError::Unauthorised
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
