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

use crate::accounts::{AuthError, ClientInfo, ClientKind, MAX_CLIENT_PLATFORM, Tokens};
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

/// What the client said about itself (`client_kind`, `client_platform`) plus its
/// User-Agent. A platform longer than [`MAX_CLIENT_PLATFORM`] characters is refused.
fn client_info(
    headers: &HeaderMap,
    kind: Option<ClientKind>,
    platform: Option<String>,
) -> Result<ClientInfo, (StatusCode, &'static str)> {
    if platform
        .as_deref()
        .is_some_and(|p| p.chars().count() > MAX_CLIENT_PLATFORM)
    {
        return Err((StatusCode::BAD_REQUEST, "client_platform too long"));
    }
    Ok(ClientInfo {
        kind,
        platform: platform
            .as_deref()
            .and_then(crate::accounts::clean_platform),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .and_then(crate::accounts::clean_user_agent),
    })
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
    /// `pwa`, `browser` or `android`.
    client_kind: Option<ClientKind>,
    /// E.g. "Safari on iPadOS" or "Pixel 8" (at most 48 characters).
    client_platform: Option<String>,
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
    let mut req: LoginRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let info = match client_info(&headers, req.client_kind, req.client_platform.take()) {
        Ok(i) => i,
        Err((status, msg)) => return error(status, msg),
    };
    let addr = crate::client_addr(&gw.cfg, peer, &headers);
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
            gw.accounts()
                .record_client(t.principal.device_id, addr, &info);
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
    client_kind: Option<ClientKind>,
    client_platform: Option<String>,
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
    let mut req: PairRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let info = match client_info(&headers, req.client_kind, req.client_platform.take()) {
        Ok(i) => i,
        Err((status, msg)) => return error(status, msg),
    };
    let addr = crate::client_addr(&gw.cfg, peer, &headers);
    let client = client_id(&gw, peer, &headers);
    match gw.accounts().pair(&req.code, &req.device_name, &client) {
        Ok(t) => {
            gw.accounts()
                .record_client(t.principal.device_id, addr, &info);
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
    client_kind: Option<ClientKind>,
    client_platform: Option<String>,
}

/// A short client diagnostic header for the log: lower-case letters, digits and `-` only, at
/// most 24 characters, else `-`.
fn diag_header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 24
                && v.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .unwrap_or("-")
        .to_string()
}

/// Rotate the refresh token (from the cookie, or the body for native apps).
pub async fn refresh(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let mut req: RefreshRequest = if body.is_empty() {
        RefreshRequest::default()
    } else {
        match parse(&body) {
            Ok(r) => r,
            Err((status, msg)) => return error(status, msg),
        }
    };
    let info = match client_info(&headers, req.client_kind, req.client_platform.take()) {
        Ok(i) => i,
        Err((status, msg)) => return error(status, msg),
    };
    let (token, refresh_in) = match (req.refresh_token, cookie(&headers, REFRESH_COOKIE)) {
        (Some(t), _) => (t, RefreshIn::Body),
        (None, Some(c)) => (c.to_string(), RefreshIn::Cookie),
        (None, None) => return error(StatusCode::UNAUTHORIZED, "invalid credentials"),
    };
    // Diagnostics: which client code asked, from which page, with which token (fingerprint).
    tracing::info!(
        target: "anywhere_gateway::refresh",
        token = %crate::accounts::fingerprint(&token),
        via = if matches!(refresh_in, RefreshIn::Cookie) { "cookie" } else { "body" },
        reason = %diag_header(&headers, "x-anywhere-refresh-reason"),
        page = %diag_header(&headers, "x-anywhere-page"),
        "refresh requested"
    );
    match gw.accounts().refresh(&token) {
        Ok(t) => {
            let addr = crate::client_addr(&gw.cfg, peer, &headers);
            gw.accounts()
                .record_client(t.principal.device_id, addr, &info);
            token_response(t, refresh_in)
        }
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
        // Optional features this server offers (plan §7).
        "features": {
            "analysis": gw.analysis().is_some(),
            // Lower-data live streams (docs/plans/streaming-quality.md): the levels a player
            // may ask for when it connects, or later with a control message.
            "stream_quality": gw.live_ffmpeg().is_some(),
            // POST /v1/players/{player}/queue/edit (checked queue edits).
            "queue_edit": true,
            // POST /v1/players/{player}/queue/save (save the queue as a playlist; admins).
            "save_queue": true,
            // The player socket's sign-in takes "grace" (seconds to hold the session for a
            // client that lost its connection); the greeting says what was granted.
            "resume_grace": true,
            // POST /v1/account/password (a signed-in user changes their own password).
            "change_password": true,
            // LMS stalls are reported: {"lms":"busy"|"ok"} on /v1/events, 503 {"busy":true}
            // from /v1/rpc while busy, and keepalives on /v1/player ("keepalive" at sign-in).
            "lms_busy": true,
        },
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

// ---------------------------------------------------------------------------
// Account self-service: password
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangePasswordRequest {
    current: String,
    new: String,
    /// The two-step code, when two-step is on.
    code: Option<String>,
    /// Also sign out the user's other devices (they stay signed in otherwise).
    #[serde(default)]
    sign_out_others: bool,
}

/// `POST /v1/account/password` `{"current", "new", "code"?, "sign_out_others"?}`: change
/// the caller's own password. Wrong credentials are 403, not 401, so a client doesn't take
/// them for an expired token and send the same guess again after renewing it.
pub async fn change_password(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !csrf_ok(&gw, &headers) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let Some(p) = principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let req: ChangePasswordRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    let client = client_id(&gw, peer, &headers);
    let accounts = gw.accounts().clone();
    let result = tokio::task::spawn_blocking(move || {
        accounts.change_own_password(
            &crate::accounts::OwnPasswordChange {
                user: p.user_id,
                username: &p.username,
                device: p.device_id,
                current: &req.current,
                new: &req.new,
                code: req.code.as_deref(),
                sign_out_others: req.sign_out_others,
            },
            &client,
        )
    })
    .await;
    match result {
        Ok(Ok(changed)) => {
            gw.end_sessions_for(&changed.revoked).await;
            let mut r = Json(json!({
                "signed_out": changed.revoked.len(),
                "other_devices": changed.others,
            }))
            .into_response();
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        Ok(Err(AuthError::Invalid)) => error(
            StatusCode::FORBIDDEN,
            "the current password or two-step code is wrong",
        ),
        Ok(Err(AuthError::TotpRequired)) => (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "two-step code required", "totp_required": true })),
        )
            .into_response(),
        Ok(Err(e)) => auth_error(e),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

// ---------------------------------------------------------------------------
// Adding another device
// ---------------------------------------------------------------------------

/// `POST /v1/devices/pair` `{"password"}`: a one-time pairing code that signs another
/// device in as the caller (shown as a QR code holding a `lyrion-anywhere://pair` link).
/// There is no user to name: the code is always for the access token's own user. Needs
/// the password, and is limited per user (see [`crate::accounts::Accounts::create_own_pairing_code`]).
pub async fn create_pairing_code(
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
        accounts.create_own_pairing_code(p.user_id, &p.username, p.device_id, &req.password)
    })
    .await;
    match result {
        Ok(Ok(code)) => {
            let mut r = Json(json!({
                "code": code,
                "expires_in": crate::accounts::PAIRING_TTL.as_secs(),
            }))
            .into_response();
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        Ok(Err(e)) => auth_error(e),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

// ---------------------------------------------------------------------------
// This device
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameRequest {
    name: String,
}

/// `PATCH /v1/devices/self`: rename the device the access token belongs to (its LMS player
/// follows). Only ever the caller's own device: there is no device id to name.
pub async fn rename_self(
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
    let req: RenameRequest = match parse(&body) {
        Ok(r) => r,
        Err((status, msg)) => return error(status, msg),
    };
    // Each rename may reach LMS and the audit log: a few a minute per device is plenty.
    if !gw.renames.take(p.device_id, std::time::Instant::now()) {
        return auth_error(AuthError::Locked(crate::RENAME_WINDOW));
    }
    match gw
        .rename_device(p.device_id, Some(p.user_id), &req.name)
        .await
    {
        Ok(name) => Json(json!({ "id": p.device_id, "name": name })).into_response(),
        Err(e) => auth_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(name: &'static str, v: &[u8]) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(name, HeaderValue::from_bytes(v).unwrap());
        h
    }

    #[test]
    fn diagnostic_headers_are_logged_only_when_plain() {
        let n = "x-anywhere-refresh-reason";
        assert_eq!(diag_header(&with(n, b"timer"), n), "timer");
        assert_eq!(diag_header(&with(n, b"k3x9-a"), n), "k3x9-a");
        assert_eq!(diag_header(&HeaderMap::new(), n), "-");
        for bad in [
            &b""[..],
            b"Timer",
            b"a b",
            b"a\"b",
            b"x=1",
            b"\xc3\xa9",
            b"aaaaaaaaaaaaaaaaaaaaaaaaa",
            b"a\tb",
        ] {
            assert_eq!(diag_header(&with(n, bad), n), "-", "{bad:?}");
        }
    }
}
