//! `POST /v1/rpc`: browse and control through LMS's JSON-RPC, within the allow-list
//! ([`crate::rpc_policy`]) and the caller's player permissions.
//!
//! Request: `{"player": "<mac or empty>", "cmd": ["albums", 0, 50, "tags:al"]}`.
//! Response: `{"result": {...}}` (filtered), 403 with a reason if the policy refuses, or 502
//! if LMS can't be reached.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::AppState;
use crate::accounts::{PlayerScope, Principal, Role};
use crate::rpc_policy::{self, Caller, Checked, Kind};

const MAX_BODY: usize = 8 * 1024;
const LMS_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    #[serde(default)]
    player: String,
    cmd: Vec<Value>,
}

struct Who<'a> {
    principal: &'a Principal,
    scope: &'a PlayerScope,
}

impl Caller for Who<'_> {
    fn role(&self) -> Role {
        self.principal.role
    }
    fn may_control(&self, player: &str) -> bool {
        self.scope.allows(player)
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub async fn rpc(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let principal = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| gw.accounts().authenticate(t));
    let Some(principal) = principal else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    if body.len() > MAX_BODY {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "request too large");
    }
    let Ok(req) = serde_json::from_slice::<RpcRequest>(&body) else {
        return error(StatusCode::BAD_REQUEST, "malformed request");
    };
    let scope = match gw.accounts().player_scope(&principal) {
        Ok(s) => s,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    };
    let who = Who {
        principal: &principal,
        scope: &scope,
    };
    let player = req.player.to_ascii_lowercase();

    let mut checked = rpc_policy::check(&who, &player, &req.cmd, &gw.apps());
    if checked.is_err() && names_unknown_app(&gw, &req.cmd) {
        // Perhaps an app installed since we last looked: refresh the list once.
        gw.refresh_apps().await;
        checked = rpc_policy::check(&who, &player, &req.cmd, &gw.apps());
    }
    let checked = match checked {
        Ok(c) => c,
        Err(denied) => {
            warn!(user = %principal.username, cmd = %summary(&req.cmd), reason = %denied.0, "rpc denied");
            return error(StatusCode::FORBIDDEN, &denied.0);
        }
    };

    match gw.lms_rpc(&checked).await {
        Ok(result) => {
            if checked.kind == Kind::AppMenu {
                gw.add_apps(rpc_policy::learn_apps(&result));
            }
            // Remember image URLs LMS handed out, so their artwork can be fetched later.
            let mut images = Vec::new();
            crate::art::learn_images(&result, &mut images);
            if !images.is_empty() {
                let mut known = gw.known_images.lock().unwrap();
                for url in images {
                    known.insert(url);
                }
            }
            let filtered = rpc_policy::filter_response(checked.kind, result, &|p| scope.allows(p));
            Json(json!({ "result": filtered })).into_response()
        }
        Err(e) => {
            warn!(error = %e, "LMS JSON-RPC failed");
            error(StatusCode::BAD_GATEWAY, "music server unavailable")
        }
    }
}

/// A short, log-safe description of a command (first word only; no search terms or ids).
fn summary(cmd: &[Value]) -> String {
    cmd.first()
        .and_then(Value::as_str)
        .map(|s| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(32)
                .collect()
        })
        .unwrap_or_else(|| "?".into())
}

fn names_unknown_app(gw: &AppState, cmd: &[Value]) -> bool {
    match cmd.first().and_then(Value::as_str) {
        Some(c) => {
            c.len() <= 64
                && c.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && !gw.apps().contains(c)
                && gw.may_refresh_apps()
        }
        None => false,
    }
}

impl crate::Gateway {
    /// Send a checked command to LMS and return its `result`.
    pub(crate) async fn lms_rpc(&self, c: &Checked) -> Result<Value, String> {
        self.lms_command(&c.player, &c.cmd).await
    }

    pub(crate) async fn lms_command(&self, player: &str, cmd: &[String]) -> Result<Value, String> {
        let body = json!({ "id": 1, "method": "slim.request", "params": [player, cmd] });
        let resp = self
            .http
            .post(format!("http://{}/jsonrpc.js", self.cfg.lms_http))
            .timeout(LMS_TIMEOUT)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("LMS status {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        Ok(v.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    /// Re-read the app and radio menus from LMS to learn their commands.
    pub(crate) async fn refresh_apps(&self) {
        *self.apps_refreshed.lock().unwrap() = Some(std::time::Instant::now());
        for menu in ["apps", "radios"] {
            let cmd = [menu.to_string(), "0".into(), "500".into()];
            if let Ok(v) = self.lms_command("", &cmd).await {
                self.add_apps(rpc_policy::learn_apps(&v));
            }
        }
    }

    /// At most one refresh a minute, so unknown commands can't be used to hammer LMS.
    fn may_refresh_apps(&self) -> bool {
        self.apps_refreshed
            .lock()
            .unwrap()
            .is_none_or(|t| t.elapsed() > Duration::from_secs(60))
    }
}
