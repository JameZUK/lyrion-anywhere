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
use tracing::{info, warn};

use crate::AppState;
use crate::accounts::{PlayerScope, Principal, Role};
use crate::rpc_policy::{self, Caller, Checked, Kind};

pub(crate) const MAX_BODY: usize = 8 * 1024;
/// Devices whose learned menu actions are kept; beyond this the least recently used go.
pub const MAX_ACTION_DEVICES: usize = 1000;
const LMS_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest LMS JSON-RPC answer read (larger ones fail with 502 rather than filling memory).
pub(crate) const MAX_LMS_RESPONSE: usize = 16 * 1024 * 1024;
/// Pause before the one retry of an idempotent LMS request.
const RETRY_PAUSE: Duration = Duration::from_millis(250);
/// An LMS exchange at least this slow is logged with its command and duration. LMS answers
/// in milliseconds when healthy; seconds mean it is blocked (e.g. on slow storage).
const SLOW_LMS: Duration = Duration::from_secs(5);
/// Commands whose second word says what they do ("playlist jump", "mixer volume").
const GROUP_COMMANDS: &[&str] = &[
    "playlist",
    "mixer",
    "favorites",
    "playlists",
    "alarm",
    "alarms",
    "time",
    "power",
    "sync",
];

/// A log-safe label for an LMS command: its first word, plus the second for command groups
/// such as `playlist jump`. Never search terms, ids, URLs or tagged parameters.
fn command_label(cmd: &[String]) -> String {
    fn word(s: &str) -> Option<String> {
        (!s.is_empty() && s.len() <= 24 && s.chars().all(|c| c.is_ascii_lowercase()))
            .then(|| s.to_string())
    }
    let Some(first) = cmd.first().and_then(|s| word(s)) else {
        return "?".into();
    };
    match cmd.get(1).and_then(|s| word(s)) {
        Some(second) if GROUP_COMMANDS.contains(&first.as_str()) => format!("{first} {second}"),
        _ => first,
    }
}

/// What to log about a finished LMS exchange, if anything: failures always, successes only
/// when they took [`SLOW_LMS`] or longer.
fn exchange_note(
    cmd: &[String],
    elapsed: Duration,
    failure: Option<&str>,
) -> Option<(String, String)> {
    match failure {
        Some(f) => Some((
            command_label(cmd),
            format!("failed after {elapsed:.1?}: {f}"),
        )),
        None if elapsed >= SLOW_LMS => Some((
            command_label(cmd),
            format!("answered slowly in {elapsed:.1?}"),
        )),
        None => None,
    }
}

/// A failed LMS exchange.
#[derive(Debug)]
pub(crate) struct LmsFailure {
    /// Worth one more try: a connection-level failure or a 5xx answer (LMS's web server
    /// sometimes refuses a request just after clients abort large downloads).
    pub retryable: bool,
    pub message: String,
}

impl LmsFailure {
    fn fatal(message: impl Into<String>) -> Self {
        LmsFailure {
            retryable: false,
            message: message.into(),
        }
    }

    pub(crate) fn from_reqwest(e: &reqwest::Error) -> Self {
        LmsFailure {
            retryable: !e.is_timeout() && (e.is_connect() || e.is_request()),
            // Without the URL: logs carry no LMS addresses, paths or query strings.
            message: format!("{}", WithoutUrl(e)),
        }
    }
}

/// Display a reqwest error without its URL.
struct WithoutUrl<'a>(&'a reqwest::Error);

impl std::fmt::Display for WithoutUrl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::error::Error;
        let kind = if self.0.is_timeout() {
            "timed out"
        } else if self.0.is_connect() {
            "connection failed"
        } else if self.0.is_body() || self.0.is_decode() {
            "response broke off"
        } else {
            "request failed"
        };
        write!(f, "{kind}")?;
        // The innermost cause (e.g. "connection reset by peer") names no URL.
        let mut cause = self.0.source();
        let mut last = None;
        while let Some(c) = cause {
            last = Some(c);
            cause = c.source();
        }
        if let Some(c) = last {
            write!(f, ": {c}")?;
        }
        Ok(())
    }
}

/// Read a response body of at most `max` bytes, chunk by chunk: a larger one is refused as
/// soon as it says so (Content-Length) or passes the limit, never read whole.
pub(crate) async fn read_capped(resp: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err("LMS response over the size limit".into());
    }
    let mut out = Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| WithoutUrl(&e).to_string())?;
        if out.len() + chunk.len() > max {
            return Err("LMS response over the size limit".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

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

/// 503 while LMS is blocked (see [`crate::lms_health`]): the client retries shortly.
pub(crate) fn busy() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(
            header::RETRY_AFTER,
            crate::lms_health::RETRY_AFTER_SECS.to_string(),
        )],
        Json(json!({ "error": "music server is busy", "busy": true })),
    )
        .into_response()
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
    let (checked, via_menu) = match checked {
        Ok(c) => (c, false),
        Err(denied) => match gw.menu_action(&principal, &scope, &player, &req.cmd) {
            // Not in the fixed allow-list, but an action LMS offered this device in a menu.
            Some(Ok(c)) => (c, true),
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
    if checked.cmd.first().is_some_and(|c| hidden.contains(c))
        || rpc_policy::chooses_hidden_dstm_provider(&checked.cmd, &hidden)
    {
        warn!(user = %principal.username, cmd = %summary(&req.cmd), "admin-only plugin denied");
        return error(StatusCode::FORBIDDEN, "this plugin is for admins only");
    }

    // A URL in free text whose name resolves into the home network or to the server: LMS or
    // a plugin would fetch it. Refused as the name checks refuse a local URL written out.
    if let Some(i) = crate::art::first_resolving_local(&checked.cmd, &gw.cfg.host_answers).await {
        warn!(user = %principal.username, cmd = %summary(&req.cmd), "free-text URL resolves locally");
        let message = match checked.cmd[i].split_once(':') {
            _ if via_menu => "malformed command".to_string(),
            Some((k, _)) => format!("bad value for {k}"),
            None => "bad value".to_string(),
        };
        return error(StatusCode::FORBIDDEN, &message);
    }

    // LMS is blocked: say so at once rather than pile another request up inside it.
    if gw.lms_health().is_busy() {
        return busy();
    }

    match gw.lms_rpc(&checked).await {
        Ok(mut result) => {
            if let Some(cur) = crate::status_fix::needs_repair(&checked.cmd, &result)
                && let Some(entry) = gw.current_entry(&checked.player, cur, &checked.cmd).await
            {
                crate::status_fix::replace_current(&mut result, entry);
            }
            if checked.kind == Kind::AppMenu {
                gw.add_apps(rpc_policy::learn_apps(&result));
            }
            let mut filtered =
                rpc_policy::filter_response(checked.kind, result, &|p| scope.allows(p));
            rpc_policy::hide_plugins(&mut filtered, &hidden);
            if checked
                .cmd
                .first()
                .is_some_and(|c| c == "dontstopthemusicsetting")
            {
                rpc_policy::hide_dstm_providers(&mut filtered, &hidden);
            }
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
            warn!(error = %e, cmd = %summary(&req.cmd), "LMS JSON-RPC failed");
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
    /// The track really at queue position `index` of `player`, as a status entry with the
    /// tags of `cmd` (see [`crate::status_fix`]). `None` on any failure.
    async fn current_entry(&self, player: &str, index: u64, cmd: &[String]) -> Option<Value> {
        let limit = Duration::from_secs(2);
        let path = ["playlist", "path", &index.to_string(), "?"].map(String::from);
        let answer = self.lms_command_within(player, &path, limit).await.ok()?;
        let url = crate::status_fix::path_of(&answer)?;
        let mut info = vec![
            "songinfo".to_string(),
            "0".into(),
            "100".into(),
            format!("url:{url}"),
        ];
        if let Some(t) = crate::status_fix::tags(cmd) {
            info.push(t.to_string());
        }
        let answer = self.lms_command_within("", &info, limit).await.ok()?;
        let entry = crate::status_fix::entry_from_songinfo(&answer, index);
        if entry.is_some() {
            info!(
                player,
                index, "status skipped the playing track; repaired from songinfo"
            );
        }
        entry
    }

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
        self.lms_command_once(player, cmd, limit)
            .await
            .map_err(|f| f.message)
    }

    /// [`Self::lms_command_within`] for a query that changes nothing: tried once more after
    /// a short pause if the first attempt failed at the connection level or with a 5xx.
    pub(crate) async fn lms_query_retrying(
        &self,
        player: &str,
        cmd: &[String],
        limit: Duration,
    ) -> Result<Value, String> {
        match self.lms_command_once(player, cmd, limit).await {
            Err(f) if f.retryable => {
                warn!(error = %f.message, "LMS JSON-RPC failed; retrying once");
                tokio::time::sleep(RETRY_PAUSE).await;
                self.lms_command_once(player, cmd, limit)
                    .await
                    .map_err(|f| f.message)
            }
            r => r.map_err(|f| f.message),
        }
    }

    async fn lms_command_once(
        &self,
        player: &str,
        cmd: &[String],
        limit: Duration,
    ) -> Result<Value, LmsFailure> {
        let body = json!({ "id": 1, "method": "slim.request", "params": [player, cmd] });
        let _waiting = self.lms_health.sent();
        let exchange = async {
            let resp = self
                .http
                .post(format!("http://{}/jsonrpc.js", self.cfg.lms_http))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
                .send()
                .await
                .map_err(|e| LmsFailure::from_reqwest(&e))?;
            // Any answer at all means LMS isn't blocked.
            self.lms_health.answered();
            let status = resp.status();
            if !status.is_success() {
                return Err(LmsFailure {
                    retryable: status.is_server_error(),
                    message: format!("LMS status {status}"),
                });
            }
            read_capped(resp, MAX_LMS_RESPONSE)
                .await
                .map_err(LmsFailure::fatal)
        };
        // One limit for the whole exchange, body included.
        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(limit, exchange)
            .await
            .map_err(|_| LmsFailure::fatal("LMS timed out"))
            .and_then(|r| r);
        if let Some((cmd, note)) = exchange_note(
            cmd,
            started.elapsed(),
            outcome.as_ref().err().map(|f| f.message.as_str()),
        ) {
            warn!(cmd = %cmd, "LMS request {note}");
        }
        let bytes = outcome?;
        let v: Value =
            serde_json::from_slice(&bytes).map_err(|e| LmsFailure::fatal(e.to_string()))?;
        Ok(v.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    /// The cheapest question for LMS, asked while it seems blocked: its answer ends the stall.
    pub(crate) async fn lms_probe(&self) -> Result<Value, String> {
        self.lms_command(
            "",
            &["serverstatus".to_string(), "0".to_string(), "0".to_string()],
        )
        .await
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
    /// Whether the app menus may be re-read now: not more than once a minute, and never
    /// while LMS is blocked (the refusal it would delay is given at once instead).
    pub(crate) fn may_refresh_apps(&self) -> bool {
        !self.lms_health.is_busy()
            && self
                .apps_refreshed
                .lock()
                .unwrap()
                .is_none_or(|t| t.elapsed() > Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu_actions::{Action, PValue};

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| s.to_string()).collect()
    }

    /// The label names the command and, for groups, what it does, and nothing else: no
    /// search terms, ids, tagged parameters or anything that isn't a plain lowercase word.
    #[test]
    fn command_labels_are_log_safe() {
        assert_eq!(
            command_label(&words(&["playlist", "jump", "3"])),
            "playlist jump"
        );
        assert_eq!(
            command_label(&words(&["mixer", "volume", "+5"])),
            "mixer volume"
        );
        assert_eq!(
            command_label(&words(&["albums", "0", "50", "tags:al"])),
            "albums"
        );
        assert_eq!(
            command_label(&words(&["search", "0", "10", "term:private words"])),
            "search"
        );
        // A second word only for known groups, so free text in that slot never appears.
        assert_eq!(
            command_label(&words(&["songinfo", "secretword", "x"])),
            "songinfo"
        );
        assert_eq!(
            command_label(&words(&["playlist", "Some Title", "x"])),
            "playlist"
        );
        assert_eq!(
            command_label(&words(&["playlist", "file:///srv/music/a.flac"])),
            "playlist"
        );
        assert_eq!(command_label(&words(&["Albums"])), "?");
        assert_eq!(command_label(&words(&["status\n", "x"])), "?");
        assert_eq!(command_label(&words(&[&"a".repeat(25)])), "?");
        assert_eq!(command_label(&[]), "?");
    }

    /// Healthy, fast answers log nothing; slow answers and every failure are logged with
    /// the command and how long it took.
    #[test]
    fn slow_or_failed_exchanges_are_noted() {
        let cmd = words(&["playlist", "jump", "3"]);
        assert_eq!(exchange_note(&cmd, Duration::from_millis(40), None), None);
        assert_eq!(
            exchange_note(&cmd, SLOW_LMS - Duration::from_millis(1), None),
            None
        );
        let (label, note) = exchange_note(&cmd, Duration::from_secs(70), None).unwrap();
        assert_eq!(label, "playlist jump");
        assert!(note.contains("slowly") && note.contains("70.0s"), "{note}");
        let (label, note) =
            exchange_note(&cmd, Duration::from_secs(30), Some("LMS timed out")).unwrap();
        assert_eq!(label, "playlist jump");
        assert!(
            note.contains("30.0s") && note.contains("LMS timed out"),
            "{note}"
        );
        // A quick failure (connection refused) is still logged.
        assert!(exchange_note(&cmd, Duration::from_millis(2), Some("connection failed")).is_some());
    }

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

    /// Regression: a revoked or deleted device's learned actions stayed, keyed by its id,
    /// for whichever device was given that id next. Ending its sessions (as every
    /// revocation does) now forgets them; other devices keep theirs.
    #[tokio::test]
    async fn a_revoked_device_forgets_its_learned_actions() {
        let gw = crate::Gateway::new(
            crate::Config::default(),
            std::sync::Arc::new(crate::accounts::Accounts::in_memory().unwrap()),
        );
        gw.remember_menu_actions(7, vec![action(7)]);
        gw.remember_menu_actions(8, vec![action(8)]);
        gw.end_sessions_for(&[7]).await;
        assert_eq!(gw.menu_action_devices(), vec![8]);
    }
}
