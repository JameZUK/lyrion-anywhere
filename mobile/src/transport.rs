//! HTTP and TLS plumbing.
//!
//! TLS is rustls (with the `ring` provider, which cross-compiles cleanly for Android) and
//! `rustls-platform-verifier`: certificates are checked by the operating system's own
//! verifier and trust store (Android's TrustManager, Apple's Security framework), so
//! user-installed CAs and revocation work as the platform intends, and no root list is
//! bundled. Redirects are never followed (a bearer token must not travel anywhere else),
//! and responses are read up to a fixed limit, with time limits on every step.
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
use rustls_platform_verifier::BuilderVerifierExt;
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

/// The platform-verified TLS configuration shared by HTTP and WebSocket connections.
pub(crate) fn tls_config(alpn: &[&[u8]]) -> Result<ClientConfig, ApiError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|b| b.with_platform_verifier())
        .map_err(|_| ApiError::network("TLS is not available on this device"))?
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
        Ok(CLIENT.get_or_init(|| Http(client)).clone())
    }

    pub(crate) fn get(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::GET, url)
    }

    pub(crate) fn post(&self, url: Url) -> Request {
        Request::new(self.clone(), Method::POST, url)
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
        let mut req = hyper::Request::builder()
            .method(self.method)
            .uri(self.url.as_str())
            .body(Full::new(self.body))
            .map_err(|_| ApiError::invalid("bad request"))?;
        *req.headers_mut() = self.headers;
        match tokio::time::timeout(HEADERS_TIMEOUT, self.http.0.request(req)).await {
            Ok(Ok(resp)) => Ok(Response(resp)),
            Ok(Err(e)) if e.is_connect() => Err(ApiError::network("could not connect")),
            Ok(Err(_)) => Err(ApiError::network("connection failed")),
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

    /// Read the body up to `limit` bytes; more than that is an error, never a truncated
    /// answer.
    pub(crate) async fn read_limited(self, limit: usize) -> Result<Vec<u8>, ApiError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
