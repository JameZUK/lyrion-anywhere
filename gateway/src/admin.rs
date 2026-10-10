//! Admin API, used by the LMS plugin's settings page (spec D12).
//!
//! Served on a **separate listener bound to loopback**, never put behind the reverse proxy.
//! Every request must also come from a loopback address, carry no proxy headers, and present
//! the admin secret (generated at first start, stored mode 0600 next to the database) as a
//! bearer token.

use std::net::SocketAddr;

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::Deserialize;
use serde_json::json;

use crate::accounts::{AuthError, Role};
use crate::{AppState, secret_eq};

#[derive(Clone)]
struct AdminState {
    gw: AppState,
    secret: std::sync::Arc<String>,
}

pub fn router(gw: AppState, secret: String) -> Router {
    let state = AdminState {
        gw,
        secret: std::sync::Arc::new(secret),
    };
    Router::new()
        .route("/admin/v1/users", get(list_users).post(create_user))
        .route("/admin/v1/users/{name}", delete(delete_user))
        // POST forms of the deletes, for HTTP clients without DELETE (e.g. LMS's).
        .route("/admin/v1/users/{name}/delete", post(delete_user))
        .route("/admin/v1/devices/{id}/revoke", post(revoke_device))
        .route("/admin/v1/users/{name}/players/{mac}/remove", post(ungrant))
        .route("/admin/v1/users/{name}/password", post(set_password))
        .route("/admin/v1/users/{name}/disabled", post(set_disabled))
        .route("/admin/v1/users/{name}/pairing", post(pairing_code))
        .route("/admin/v1/users/{name}/totp/reset", post(reset_totp))
        .route(
            "/admin/v1/users/{name}/players",
            get(list_grants).post(grant),
        )
        .route("/admin/v1/users/{name}/players/{mac}", delete(ungrant))
        .route("/admin/v1/devices", get(list_devices))
        .route(
            "/admin/v1/devices/{id}/diagnostics",
            get(device_diagnostics),
        )
        .route("/admin/v1/devices/{id}", delete(revoke_device))
        .route("/admin/v1/audit", get(audit))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Loopback only, no proxy headers, correct admin secret.
async fn guard(
    State(st): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let proxied = [
        "x-forwarded-for",
        "x-forwarded-proto",
        "forwarded",
        "x-real-ip",
    ]
    .iter()
    .any(|h| req.headers().contains_key(*h));
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !crate::canonical(peer.ip()).is_loopback() || proxied || !secret_eq(token, &st.secret) {
        tracing::warn!(peer = %peer.ip(), path = %req.uri().path(), "admin API request refused");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

fn err(e: AuthError) -> Response {
    let (status, msg) = match e {
        AuthError::BadInput(m) => (StatusCode::BAD_REQUEST, m.to_string()),
        AuthError::Storage(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        _ => (StatusCode::BAD_REQUEST, "refused".to_string()),
    };
    (status, Json(json!({ "error": msg }))).into_response()
}

async fn list_users(State(st): State<AdminState>) -> Response {
    match st.gw.accounts().list_users() {
        Ok(u) => Json(json!({ "users": u })).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewUser {
    username: String,
    password: String,
    role: String,
}

async fn create_user(State(st): State<AdminState>, Json(u): Json<NewUser>) -> Response {
    let Some(role) = Role::parse(&u.role) else {
        return err(AuthError::BadInput("role must be listener or admin"));
    };
    let accounts = st.gw.accounts().clone();
    match tokio::task::spawn_blocking(move || accounts.create_user(&u.username, &u.password, role))
        .await
    {
        Ok(Ok(id)) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Ok(Err(e)) => err(e),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn delete_user(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.accounts().delete_user(&name) {
        Ok(devices) => {
            st.gw.end_sessions_for(&devices).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPassword {
    password: String,
}

async fn set_password(
    State(st): State<AdminState>,
    Path(name): Path<String>,
    Json(p): Json<NewPassword>,
) -> Response {
    let accounts = st.gw.accounts().clone();
    match tokio::task::spawn_blocking(move || accounts.set_password(&name, &p.password)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => err(e),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Disabled {
    disabled: bool,
}

async fn set_disabled(
    State(st): State<AdminState>,
    Path(name): Path<String>,
    Json(d): Json<Disabled>,
) -> Response {
    match st.gw.accounts().set_disabled(&name, d.disabled) {
        Ok(devices) => {
            st.gw.end_sessions_for(&devices).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => err(e),
    }
}

async fn pairing_code(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.accounts().create_pairing_code(&name) {
        Ok(code) => Json(json!({
            "code": code,
            "expires_in": crate::accounts::PAIRING_TTL.as_secs(),
        }))
        .into_response(),
        Err(e) => err(e),
    }
}

/// Remove a user's two-step verification (e.g. they lost their phone).
async fn reset_totp(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.accounts().totp_reset(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn list_grants(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.accounts().players_granted(&name) {
        Ok(p) => Json(json!({ "players": p })).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    player: String,
}

/// Let a user control a home player (by its LMS player id).
async fn grant(
    State(st): State<AdminState>,
    Path(name): Path<String>,
    Json(g): Json<Grant>,
) -> Response {
    match st.gw.accounts().grant_player(&name, &g.player) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn ungrant(
    State(st): State<AdminState>,
    Path((name, mac)): Path<(String, String)>,
) -> Response {
    match st.gw.accounts().ungrant_player(&name, &mac) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn list_devices(State(st): State<AdminState>) -> Response {
    match st.gw.accounts().list_devices() {
        Ok(d) => Json(json!({ "devices": d })).into_response(),
        Err(e) => err(e),
    }
}

async fn device_diagnostics(State(st): State<AdminState>, Path(id): Path<i64>) -> Response {
    match st.gw.accounts().diagnostics(id) {
        Ok(Some((at, report))) => {
            let report: serde_json::Value =
                serde_json::from_str(&report).unwrap_or(serde_json::Value::Null);
            Json(json!({ "updated_at": at, "report": report })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no diagnostics for this device" })),
        )
            .into_response(),
        Err(e) => err(e),
    }
}

async fn revoke_device(State(st): State<AdminState>, Path(id): Path<i64>) -> Response {
    match st.gw.accounts().revoke_device(id) {
        Ok(true) => {
            st.gw.end_sessions_for(&[id]).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
struct AuditQuery {
    limit: Option<u32>,
}

async fn audit(
    State(st): State<AdminState>,
    Query(q): Query<AuditQuery>,
    _h: HeaderMap,
) -> Response {
    match st.gw.accounts().audit_log(q.limit.unwrap_or(100)) {
        Ok(a) => Json(json!({ "audit": a })).into_response(),
        Err(e) => err(e),
    }
}
