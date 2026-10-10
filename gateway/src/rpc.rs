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
/// Devices whose learned menu actions are kept; beyond this the least recently used go.
pub const MAX_ACTION_DEVICES: usize = 1000;
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
        Err(denied) => match gw.menu_action(&principal, &scope, &player, &req.cmd) {
            // Not in the fixed allow-list, but an action LMS offered this device in a menu.
            Some(Ok(c)) => c,
            Some(Err(refusal)) => {
                warn!(user = %principal.username, cmd = %summary(&req.cmd), reason = ?refusal, "menu action denied");
                return error(StatusCode::FORBIDDEN, menu_refusal(&refusal, &denied.0));
            }
            None => {
                warn!(user = %principal.username, cmd = %summary(&req.cmd), reason = %denied.0, "rpc denied");
                return error(StatusCode::FORBIDDEN, &denied.0);
            }
        },
    };

    // Plugins the admin reserved for admins: refused for everyone else, whichever way the
    // command got through (app list or learned menu action).
    let hidden = if principal.role == Role::Admin {
        Default::default()
    } else {
        gw.accounts().admin_only_plugins()
    };
    if checked.cmd.first().is_some_and(|c| hidden.contains(c)) {
        warn!(user = %principal.username, cmd = %summary(&req.cmd), "admin-only plugin denied");
        return error(StatusCode::FORBIDDEN, "this plugin is for admins only");
    }

    match gw.lms_rpc(&checked).await {
        Ok(result) => {
            if checked.kind == Kind::AppMenu {
                gw.add_apps(rpc_policy::learn_apps(&result));
            }
            let mut filtered =
                rpc_policy::filter_response(checked.kind, result, &|p| scope.allows(p));
            rpc_policy::hide_plugins(&mut filtered, &hidden);
            // Remember image URLs LMS handed out, so their artwork can be fetched later: only
            // from image fields of what the client actually receives.
            let mut images = Vec::new();
            crate::art::learn_images(&filtered, &mut images);
            if !images.is_empty() {
                let mut known = gw.known_images.lock().unwrap();
                for url in images {
                    known.insert(url);
                }
            }
            // Learn the menu actions this device has now been shown (after filtering, so only
            // what the client actually received can ever be run).
            let offered = crate::menu_actions::learn(&filtered);
            if !offered.is_empty() {
                gw.remember_menu_actions(principal.device_id, offered);
            }
            Json(json!({ "result": filtered })).into_response()
        }
        Err(e) => {
            warn!(error = %e, "LMS JSON-RPC failed");
            error(StatusCode::BAD_GATEWAY, "music server unavailable")
        }
    }
}

fn menu_refusal<'a>(r: &crate::menu_actions::Refusal, fallback: &'a str) -> &'a str {
    use crate::menu_actions::Refusal;
    match r {
        Refusal::Never => "this command is not available remotely",
        Refusal::AdminOnly => "this action needs the admin role",
        Refusal::Malformed => "malformed command",
        Refusal::NotOffered => fallback,
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
    pub(crate) fn remember_menu_actions(
        &self,
        device: i64,
        actions: Vec<crate::menu_actions::Action>,
    ) {
        let now = std::time::Instant::now();
        let mut all = self.menu_actions.lock().unwrap();
        all.entry(device).or_default().add(actions, now);
        // Bound the number of devices tracked: expire old entries, then drop the least
        // recently used devices.
        if all.len() > MAX_ACTION_DEVICES {
            for d in all.values_mut() {
                d.expire(now);
            }
            all.retain(|_, d| !d.is_empty());
        }
        if all.len() > MAX_ACTION_DEVICES {
            let mut by_use: Vec<(Option<std::time::Instant>, i64)> = all
                .iter()
                .filter(|(id, _)| **id != device)
                .map(|(id, d)| (d.last_used(), *id))
                .collect();
            by_use.sort();
            let excess = all.len() - MAX_ACTION_DEVICES;
            for (_, id) in by_use.into_iter().take(excess) {
                all.remove(&id);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn menu_action_devices(&self) -> Vec<i64> {
        self.menu_actions.lock().unwrap().keys().copied().collect()
    }

    /// A command outside the fixed allow-list: allowed if it is a menu action LMS offered this
    /// device. `None` if it isn't shaped like one at all (the allow-list refusal stands).
    fn menu_action(
        &self,
        principal: &Principal,
        scope: &PlayerScope,
        player: &str,
        cmd: &[Value],
    ) -> Option<Result<Checked, crate::menu_actions::Refusal>> {
        use crate::menu_actions::Refusal;
        if !player.is_empty() && !(rpc_policy::is_player_id(player) && scope.allows(player)) {
            return None;
        }
        let raw: Vec<String> = cmd
            .iter()
            .map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => n.as_u64().map(|n| n.to_string()),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let now = std::time::Instant::now();
        let mut all = self.menu_actions.lock().unwrap();
        let empty = crate::menu_actions::DeviceActions::default();
        let learned = match all.get_mut(&principal.device_id) {
            Some(d) => {
                d.expire(now);
                d.touch(now);
                &*d
            }
            None => &empty,
        };
        let verdict = crate::menu_actions::allowed(learned, &raw, principal.role == Role::Admin);
        drop(all);
        Some(match verdict {
            Ok(()) => Ok(Checked {
                player: player.to_string(),
                cmd: raw,
                kind: Kind::Plain,
            }),
            Err(Refusal::NotOffered) => return None,
            Err(e) => Err(e),
        })
    }

    /// Send a checked command to LMS and return its `result`.
    pub(crate) async fn lms_rpc(&self, c: &Checked) -> Result<Value, String> {
        self.lms_command(&c.player, &c.cmd).await
    }

    pub(crate) async fn lms_command(&self, player: &str, cmd: &[String]) -> Result<Value, String> {
        self.lms_command_within(player, cmd, LMS_TIMEOUT).await
    }

    /// [`Self::lms_command`] with its own time limit for the whole exchange.
    pub(crate) async fn lms_command_within(
        &self,
        player: &str,
        cmd: &[String],
        limit: Duration,
    ) -> Result<Value, String> {
        let body = json!({ "id": 1, "method": "slim.request", "params": [player, cmd] });
        let resp = self
            .http
            .post(format!("http://{}/jsonrpc.js", self.cfg.lms_http))
            .timeout(limit)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu_actions::{Action, PValue};

    fn action(n: i64) -> Action {
        Action {
            cmd: vec!["someplugin".into(), "items".into()],
            params: [("item_id".to_string(), PValue::Exact(n.to_string()))].into(),
            nav: true,
        }
    }

    /// The learned-actions map is bounded however many devices fill it, even when none of
    /// their entries has expired yet: the least recently used devices go.
    #[test]
    fn learned_actions_keep_at_most_the_most_recently_used_devices() {
        let gw = crate::Gateway::new(
            crate::Config::default(),
            std::sync::Arc::new(crate::accounts::Accounts::in_memory().unwrap()),
        );
        let n = MAX_ACTION_DEVICES as i64 + 200;
        for d in 0..n {
            gw.remember_menu_actions(d, vec![action(d)]);
            // Device 1 keeps being used, so it is never the least recently used.
            gw.remember_menu_actions(1, vec![action(1)]);
        }
        let kept = gw.menu_action_devices();
        assert_eq!(kept.len(), MAX_ACTION_DEVICES);
        assert!(kept.contains(&1));
        assert!(kept.contains(&(n - 1)));
        assert!(!kept.contains(&0));
        assert!(!kept.contains(&2));
    }
}
