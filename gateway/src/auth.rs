//! Public authentication endpoints (`/v1/auth/*`, `/v1/me`).
//!
//! Browsers get the refresh token in an `HttpOnly; Secure; SameSite=Strict` cookie scoped
//! to `/v1/auth`, so page scripts can never read it. Native apps ask for it in the response
//! body (`"refresh":"body"`) and keep it in the platform keystore.
//!
//! CSRF: every `/v1/auth/*` request must carry `X-Anywhere-Client` (a custom header that a
//! cross-site form can't send, and a cross-site `fetch` can't send without a CORS preflight
//! the gateway never approves), and any `Origin` must be allowed.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::accounts::{AuthError, Tokens};
use crate::{AppState, origin_allowed};

pub const REFRESH_COOKIE: &str = "anywhere_refresh";
pub const CLIENT_HEADER: &str = "x-anywhere-client";
const MAX_BODY: usize = 4096;
/// Sent when this browser's device is signed out, so cached artwork and other responses
/// go with it. "cache" leaves IndexedDB (offline downloads) alone; browsers may also clear
/// the service worker's caches, which it refills.
const CLEAR_SITE_DATA: &str = "clear-site-data";
const CLEAR_CACHE: &str = "\"cache\"";

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Map account errors to responses that reveal nothing beyond what the client must know.
fn auth_error(e: AuthError) -> Response {
    match e {
        AuthError::Invalid | AuthError::Reused(_) => {
            error(StatusCode::UNAUTHORIZED, "invalid credentials")
        }
        AuthError::Locked(d) => {
            let mut r = error(
                StatusCode::TOO_MANY_REQUESTS,
                "too many attempts; try again later",
            );
            if let Ok(v) = HeaderValue::from_str(&d.as_secs().max(1).to_string()) {
                r.headers_mut().insert(header::RETRY_AFTER, v);
            }
            r
        }
        AuthError::TotpRequired => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "two-step code required", "totp_required": true })),
        )
            .into_response(),
        AuthError::BadInput(m) => error(StatusCode::BAD_REQUEST, m),
        AuthError::Busy => {
            let mut r = error(StatusCode::SERVICE_UNAVAILABLE, "busy; try again shortly");
            r.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            r
        }
        AuthError::Storage(e) => {
            tracing::error!(error = %e, "account store error");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    }
}

/// CSRF and origin checks for cookie-capable endpoints.
fn csrf_ok(gw: &AppState, headers: &HeaderMap) -> bool {
    headers.contains_key(CLIENT_HEADER) && origin_allowed(&gw.cfg, headers)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn refresh_cookie(value: &str, max_age: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{REFRESH_COOKIE}={value}; Path=/v1/auth; Max-Age={max_age}; HttpOnly; Secure; SameSite=Strict"
    ))
    .expect("cookie is ASCII")
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum RefreshIn {
    #[default]
    Cookie,
    Body,
}

fn token_response(t: Tokens, refresh_in: RefreshIn) -> Response {
    let mut body = json!({
        "access_token": t.access,
        "expires_in": t.access_expires_in,
        "user": { "username": t.principal.username, "role": t.principal.role.as_str() },
        "device": {
            "id": t.principal.device_id,
            "name": t.principal.device_name,
            "mac": t.principal.mac_string(),
        },
    });
    let mut resp_headers = HeaderMap::new();
    match refresh_in {
        RefreshIn::Body => body["refresh_token"] = json!(t.refresh),
        RefreshIn::Cookie => {
            resp_headers.insert(
                header::SET_COOKIE,
                refresh_cookie(&t.refresh, t.refresh_expires_in),
            );
        }
    }
    resp_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (StatusCode::OK, resp_headers, Json(body)).into_response()
}

fn client_id(gw: &AppState, peer: SocketAddr, headers: &HeaderMap) -> String {
    crate::client_ip(&gw.cfg, peer, headers)
}

fn parse<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, (StatusCode, &'static str)> {
    if body.len() > MAX_BODY {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "request too large"));
    }
    serde_json::from_slice(body).map_err(|_| (StatusCode::BAD_REQUEST, "malformed request"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    username: String,
    password: String,
    /// Two-step code, for users who have it turned on.
    totp: Option<String>,
    device_name: String,
    /// Reuse this (own) device, e.g. a browser logging in again.
    device_id: Option<i64>,
    #[serde(default)]
    refresh: RefreshIn,
}

pub async fn login(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let req: LoginRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let client = client_id(&gw, peer, &headers);
    let accounts = gw.accounts().clone();
    // Password hashing is CPU-heavy: keep it off the async workers.
    let result = tokio::task::spawn_blocking(move || {
        accounts
            .login(
                &req.username,
                &req.password,
                req.totp.as_deref(),
                &req.device_name,
                req.device_id,
                &client,
            )
            .map(|t| (t, req.refresh))
    })
    .await;
    match result {
        Ok(Ok((t, refresh_in))) => {
            // Devices signed out to stay within the per-user limit.
            gw.end_sessions_for(&t.evicted).await;
            token_response(t, refresh_in)
        }
        Ok(Err(e)) => auth_error(e),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairRequest {
    code: String,
    device_name: String,
}

/// Native apps: redeem a pairing code. The refresh token is always returned in the body.
pub async fn pair(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let req: PairRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let client = client_id(&gw, peer, &headers);
    match gw.accounts().pair(&req.code, &req.device_name, &client) {
        Ok(t) => {
            gw.end_sessions_for(&t.evicted).await;
            token_response(t, RefreshIn::Body)
        }
        Err(e) => auth_error(e),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RefreshRequest {
    refresh_token: Option<String>,
}

/// Rotate the refresh token (from the cookie, or the body for native apps).
pub async fn refresh(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let req: RefreshRequest = if body.is_empty() {
        RefreshRequest::default()
    } else {
        match parse(&body) {
            Ok(r) => r,
            Err((status, msg)) => return error(status, msg),
        }
    };
    let (token, refresh_in) = match (req.refresh_token, cookie(&headers, REFRESH_COOKIE)) {
        (Some(t), _) => (t, RefreshIn::Body),
        (None, Some(c)) => (c.to_string(), RefreshIn::Cookie),
        (None, None) => return error(StatusCode::UNAUTHORIZED, "invalid credentials"),
    };
    match gw.accounts().refresh(&token) {
        Ok(t) => token_response(t, refresh_in),
        Err(AuthError::Reused(device)) => {
            // A stolen refresh token: the device is revoked, so end its live session too.
            gw.end_sessions_for(&[device]).await;
            let mut r = auth_error(AuthError::Reused(device));
            r.headers_mut()
                .insert(CLEAR_SITE_DATA, HeaderValue::from_static(CLEAR_CACHE));
            r
        }
        Err(e) => auth_error(e),
    }
}

/// Log out: revoke this device, end its sessions, clear the cookie.
pub async fn logout(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(token) = bearer(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    match gw.accounts().logout(token) {
        Ok(Some(device)) => {
            gw.end_sessions_for(&[device]).await;
            let mut h = HeaderMap::new();
            h.insert(header::SET_COOKIE, refresh_cookie("", 0));
            h.insert(CLEAR_SITE_DATA, HeaderValue::from_static(CLEAR_CACHE));
            (StatusCode::NO_CONTENT, h).into_response()
        }
        Ok(None) => error(StatusCode::UNAUTHORIZED, "invalid credentials"),
        Err(e) => auth_error(e),
    }
}

/// Who am I: the principal behind a bearer access token.
pub async fn me(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let Some(p) = bearer(&headers).and_then(|t| gw.accounts().authenticate(t)) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    Json(json!({
        "user": { "username": p.username, "role": p.role.as_str() },
        "device": { "id": p.device_id, "name": p.device_name, "mac": p.mac_string() },
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Account self-service: two-step verification
// ---------------------------------------------------------------------------

pub(crate) fn principal(gw: &AppState, headers: &HeaderMap) -> Option<crate::accounts::Principal> {
    bearer(headers).and_then(|t| gw.accounts().authenticate(t))
}

/// The signed-in user's account settings.
pub async fn account(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let Some(p) = principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let state = gw
        .accounts()
        .totp_enabled(p.user_id)
        .and_then(|on| Ok((on, gw.accounts().download_policy(p.user_id)?)));
    match state {
        Ok((on, policy)) => {
            // The formats this user may download that this server can produce.
            let formats: Vec<&str> = gw
                .server_formats()
                .into_iter()
                .map(crate::transcode::Format::as_str)
                .filter(|f| policy.allows(f))
                .collect();
            Json(json!({
                "username": p.username,
                "role": p.role.as_str(),
                "totp_enabled": on,
                "downloads": { "enabled": policy.enabled, "formats": formats },
            }))
            .into_response()
        }
        Err(e) => auth_error(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordRequest {
    password: String,
}

/// Start two-step setup: needs the account password; returns the secret and `otpauth://`
/// URI for the app.
pub async fn totp_setup(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(p) = principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let req: PasswordRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let accounts = gw.accounts().clone();
    let result = tokio::task::spawn_blocking(move || {
        accounts.totp_begin(p.user_id, &p.username, &req.password)
    })
    .await;
    match result {
        Ok(Ok(setup)) => {
            let mut r = Json(setup).into_response();
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        Ok(Err(e)) => auth_error(e),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    code: String,
}

/// Finish setup with a code from the app.
pub async fn totp_confirm(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(p) = principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let req: CodeRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    match gw
        .accounts()
        .totp_confirm(p.user_id, &p.username, &req.code)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => auth_error(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DisableRequest {
    password: String,
    code: String,
}

/// Turn two-step off: needs the password and a current code.
pub async fn totp_disable(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(p) = principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let req: DisableRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let accounts = gw.accounts().clone();
    let result = tokio::task::spawn_blocking(move || {
        accounts.totp_disable(p.user_id, &p.username, &req.password, &req.code)
    })
    .await;
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => auth_error(e),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}
