//! Admin API, used by the LMS plugin's settings page (spec D12).
//!
//! Served on a **separate listener bound to loopback**, never put behind the reverse proxy.
//! Every request must also come from a loopback address, carry no proxy headers, and present
//! the admin secret (generated at first start, stored mode 0600 next to the database) as a
//! bearer token.
//!
//! Database work runs on the blocking pool ([`crate::Gateway::db`]), never on an async
//! worker: a write waiting for the disk must not hold up the player sessions sharing it.

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
        .route("/admin/v1/devices/{id}/name", post(rename_device))
        .route("/admin/v1/users/{name}/players/{mac}/remove", post(ungrant))
        .route("/admin/v1/users/{name}/password", post(set_password))
        .route("/admin/v1/users/{name}/disabled", post(set_disabled))
        .route("/admin/v1/users/{name}/pairing", post(pairing_code))
        .route("/admin/v1/users/{name}/totp/reset", post(reset_totp))
        .route("/admin/v1/users/{name}/downloads", post(set_downloads))
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
        .route(
            "/admin/v1/devices/{id}",
            delete(revoke_device).patch(rename_device),
        )
        .route("/admin/v1/audit", get(audit))
        .route("/admin/v1/settings", get(settings).post(set_settings))
        .route("/admin/v1/plugins", get(plugins).post(set_plugins))
        .route("/admin/v1/tunnel", get(tunnel_status))
        .route("/admin/v1/tunnel/log", get(tunnel_log))
        .route("/admin/v1/tunnel/install", post(tunnel_install))
        .route("/admin/v1/tunnel/update", post(tunnel_update))
        .route("/admin/v1/tunnel/register", post(tunnel_register))
        .route("/admin/v1/tunnel/create", post(tunnel_create))
        .route("/admin/v1/tunnel/start", post(tunnel_start))
        .route("/admin/v1/tunnel/stop", post(tunnel_stop))
        .route("/admin/v1/tunnel/quick", post(tunnel_quick))
        .route("/admin/v1/tunnel/remove", post(tunnel_remove))
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
    match st.gw.db(|a| a.list_users()).await {
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
    match st
        .gw
        .db(move |a| a.create_user(&u.username, &u.password, role))
        .await
    {
        Ok(id) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => err(e),
    }
}

async fn delete_user(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    let user = name.clone();
    match st.gw.db(move |a| a.delete_user(&user)).await {
        Ok(devices) => {
            // Its remembered stats go with it (a later user could get the same id).
            st.gw.stats.forget_name(&name);
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
    match st.gw.db(move |a| a.set_password(&name, &p.password)).await {
        Ok(devices) => {
            // A password reset signs the user out everywhere.
            st.gw.end_sessions_for(&devices).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => err(e),
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
    match st.gw.db(move |a| a.set_disabled(&name, d.disabled)).await {
        Ok(devices) => {
            st.gw.end_sessions_for(&devices).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    session_days: u32,
}

/// Gateway settings: the sign-in lifetime (days an unused device stays signed in), and the
/// download formats this server can produce (transcodes need ffmpeg).
async fn settings(State(st): State<AdminState>) -> Response {
    let formats: Vec<&str> = st
        .gw
        .server_formats()
        .into_iter()
        .map(crate::transcode::Format::as_str)
        .collect();
    let days = st.gw.db(|a| a.session_days()).await;
    Json(json!({
        "session_days": days,
        "session_days_default": crate::accounts::DEFAULT_SESSION_DAYS,
        "session_days_min": crate::accounts::SESSION_DAYS.start(),
        "session_days_max": crate::accounts::SESSION_DAYS.end(),
        "download_formats": formats,
    }))
    .into_response()
}

async fn set_settings(State(st): State<AdminState>, Json(s): Json<Settings>) -> Response {
    match st.gw.db(move |a| a.set_session_days(s.session_days)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminOnly {
    admin_only: Vec<String>,
}

/// The plugins LMS offers (apps, radio, and entries in the home menu such as My Music), by
/// command word, each with whether it is reserved for admins.
async fn plugins(State(st): State<AdminState>) -> Response {
    use std::collections::BTreeMap;
    let gw = &st.gw;
    let cmd = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    let mut found: BTreeMap<String, (String, &'static str)> = BTreeMap::new();
    let mut add = |c: &str, name: &str, kind: &'static str| {
        if crate::accounts::valid_plugin_command(c) && !crate::rpc_policy::core_command(c) {
            let name: String = name
                .chars()
                .filter(|ch| !ch.is_control())
                .take(100)
                .collect();
            found
                .entry(c.to_string())
                .or_insert((if name.is_empty() { c.to_string() } else { name }, kind));
        }
    };
    for (menu, key, kind) in [
        ("apps", "appss_loop", "app"),
        ("radios", "radioss_loop", "radio"),
    ] {
        if let Ok(v) = gw.lms_command("", &cmd(&[menu, "0", "500"])).await {
            for item in v.get(key).and_then(|x| x.as_array()).into_iter().flatten() {
                if let Some(c) = item.get("cmd").and_then(|x| x.as_str()) {
                    add(
                        c,
                        item.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                        kind,
                    );
                }
            }
        }
    }
    // The home menu belongs to a player: any will do, it is the same for all.
    let player = gw
        .lms_command("", &cmd(&["players", "0", "50"]))
        .await
        .ok()
        .and_then(|v| {
            v.get("players_loop")?
                .as_array()?
                .iter()
                .filter_map(|p| p.get("playerid")?.as_str().map(str::to_owned))
                .find(|p| crate::rpc_policy::is_player_id(p))
        });
    if let Some(player) = player
        && let Ok(v) = gw
            .lms_command(&player, &cmd(&["menu", "0", "500", "direct:1"]))
            .await
    {
        let menu =
            crate::rpc_policy::filter_response(crate::rpc_policy::Kind::HomeMenu, v, &|_| true);
        let items: Vec<&serde_json::Value> = menu
            .get("item_loop")
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .collect();
        let text = |i: &serde_json::Value| {
            i.get("text")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        };
        for item in &items {
            let Some(c) = item.pointer("/actions/go/cmd/0").and_then(|x| x.as_str()) else {
                continue;
            };
            // Entries inside a group (e.g. Random Mix's mixes) take the group's name.
            let node = item.get("node").and_then(|x| x.as_str()).unwrap_or("");
            let name = items
                .iter()
                .find(|g| g.get("id").and_then(|x| x.as_str()) == Some(node))
                .map(|g| text(g))
                .unwrap_or_else(|| text(item));
            add(c, &name, "menu");
        }
    }
    let admin_only = gw.db(|a| a.admin_only_plugins()).await;
    for c in &admin_only {
        found
            .entry(c.clone())
            .or_insert((c.clone(), "not installed"));
    }
    // One entry per plugin: commands shown under the same name (e.g. Random Mix's mixes and
    // its genre and library choosers) are one plugin, reserved or opened together.
    let mut groups: BTreeMap<(String, &'static str), Vec<String>> = BTreeMap::new();
    for (c, (name, kind)) in found {
        groups.entry((name, kind)).or_default().push(c);
    }
    let list: Vec<_> = groups
        .into_iter()
        .map(|((name, kind), commands)| {
            let reserved = commands.iter().all(|c| admin_only.contains(c));
            json!({ "command": commands[0], "commands": commands, "name": name, "kind": kind, "admin_only": reserved })
        })
        .collect();
    Json(json!({ "plugins": list })).into_response()
}

async fn set_plugins(State(st): State<AdminState>, Json(a): Json<AdminOnly>) -> Response {
    match st
        .gw
        .db(move |acc| acc.set_admin_only_plugins(&a.admin_only))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn pairing_code(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.db(move |a| a.create_pairing_code(&name)).await {
        Ok(code) => Json(json!({
            "code": code,
            "expires_in": crate::accounts::PAIRING_TTL.as_secs(),
        }))
        .into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Downloads {
    enabled: bool,
    #[serde(default)]
    formats: Option<Vec<String>>,
}

/// Turn a user's downloads on or off, optionally with the formats they may use.
async fn set_downloads(
    State(st): State<AdminState>,
    Path(name): Path<String>,
    Json(d): Json<Downloads>,
) -> Response {
    match st
        .gw
        .db(move |a| a.set_downloads(&name, d.enabled, d.formats.as_deref()))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

/// Remove a user's two-step verification (e.g. they lost their phone).
async fn reset_totp(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.db(move |a| a.totp_reset(&name)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn list_grants(State(st): State<AdminState>, Path(name): Path<String>) -> Response {
    match st.gw.db(move |a| a.players_granted(&name)).await {
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
    match st.gw.db(move |a| a.grant_player(&name, &g.player)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn ungrant(
    State(st): State<AdminState>,
    Path((name, mac)): Path<(String, String)>,
) -> Response {
    match st.gw.db(move |a| a.ungrant_player(&name, &mac)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

async fn list_devices(State(st): State<AdminState>) -> Response {
    match st.gw.db(|a| a.list_devices()).await {
        Ok(d) => Json(json!({ "devices": d })).into_response(),
        Err(e) => err(e),
    }
}

async fn device_diagnostics(State(st): State<AdminState>, Path(id): Path<i64>) -> Response {
    match st.gw.db(move |a| a.diagnostics(id)).await {
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
    match st.gw.db(move |a| a.revoke_device(id)).await {
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
    let limit = q.limit.unwrap_or(100);
    match st.gw.db(move |a| a.audit_log(limit)).await {
        Ok(a) => Json(json!({ "audit": a })).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    name: String,
}

/// Rename any device (its LMS player follows if it is connected).
async fn rename_device(
    State(st): State<AdminState>,
    Path(id): Path<i64>,
    Json(r): Json<Rename>,
) -> Response {
    match st.gw.rename_device(id, None, &r.name).await {
        Ok(name) => Json(json!({ "id": id, "name": name })).into_response(),
        Err(AuthError::BadInput("no such device")) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => err(e),
    }
}

// ---------------------------------------------------------------------------
// The built-in Cloudflare Tunnel (docs/plans/cloudflare-tunnel.md, "Admin API")
// ---------------------------------------------------------------------------

use crate::cftunnel::{Accepted, Refusal};

fn refused(r: Refusal) -> Response {
    let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(json!({ "error": r.message }))).into_response()
}

fn accepted(r: Result<Accepted, Refusal>) -> Response {
    match r {
        Ok(Accepted::Started) => (StatusCode::ACCEPTED, Json(json!({}))).into_response(),
        Ok(Accepted::Done) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => refused(e),
    }
}

fn done(r: Result<(), Refusal>) -> Response {
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => refused(e),
    }
}

/// A JSON body that doesn't parse (or has unknown fields) is a 400 with a sentence, like
/// every other refusal of the tunnel's routes.
fn bad_body() -> Response {
    refused(Refusal {
        status: 400,
        message: "The request body is not what this request takes.".into(),
    })
}

async fn tunnel_status(State(st): State<AdminState>) -> Response {
    Json(st.gw.tunnel().status()).into_response()
}

async fn tunnel_log(State(st): State<AdminState>) -> Response {
    Json(json!({ "log": st.gw.tunnel().log() })).into_response()
}

async fn tunnel_install(State(st): State<AdminState>) -> Response {
    accepted(st.gw.tunnel().install().await)
}

async fn tunnel_update(State(st): State<AdminState>) -> Response {
    accepted(st.gw.tunnel().update().await)
}

/// The tunnel token is a secret: no `Debug`, never logged.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    token: String,
    hostname: String,
}

async fn tunnel_register(
    State(st): State<AdminState>,
    body: Result<Json<Register>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Ok(Json(r)) = body else {
        return bad_body();
    };
    done(st.gw.tunnel().register(&r.token, &r.hostname).await)
}

/// The API token is a secret: no `Debug`, never logged, dropped after the exchange.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTunnel {
    api_token: String,
    hostname: String,
    #[serde(default)]
    replace_dns: bool,
}

async fn tunnel_create(
    State(st): State<AdminState>,
    body: Result<Json<CreateTunnel>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Ok(Json(c)) = body else {
        return bad_body();
    };
    accepted(
        st.gw
            .tunnel()
            .create(c.api_token, &c.hostname, c.replace_dns)
            .await,
    )
}

async fn tunnel_start(State(st): State<AdminState>) -> Response {
    done(st.gw.tunnel().start().await)
}

async fn tunnel_stop(State(st): State<AdminState>) -> Response {
    done(st.gw.tunnel().stop().await)
}

async fn tunnel_quick(State(st): State<AdminState>) -> Response {
    accepted(st.gw.tunnel().quick().await)
}

async fn tunnel_remove(State(st): State<AdminState>) -> Response {
    done(st.gw.tunnel().remove().await)
}
