//! Sign-in, token rotation and sign-out (docs/PROTOCOL.md, "Authentication").
//!
//! - The **access token** (15 minutes) lives only in memory. It is renewed shortly before
//!   it expires, and once more when the gateway answers 401; the failed call is then
//!   retried once.
//! - The **refresh token** is single use and rotated on every refresh. Each new one is
//!   written to the app's [`SecureStore`] (Keystore-encrypted on Android) before it is used.
//!   A refresh runs to completion in its own task even if the caller is cancelled, so a
//!   rotated token is never lost to a cancelled coroutine (presenting the old one later
//!   would look like theft and revoke the device).
//! - Refreshes are single-flight: concurrent callers share one.
//! - Only a **refusal** (4xx) of a refresh signs the user out. A network failure keeps the
//!   sign-in, and the caller can retry later.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::error::ApiError;
use crate::model::{AccountInfo, Device, SignedIn, User, UserRole};
use crate::server::{Server, normalise_pairing_code};
use crate::transport::{self, CLIENT_HEADER, Http};

/// What the app keeps in secure storage: nothing else.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StoredCredentials {
    /// The current (single-use) refresh token.
    pub refresh_token: String,
    /// The gateway's id for this device, so signing in again keeps the same LMS player.
    pub device_id: i64,
}

/// Secure storage failed (e.g. the Keystore key was invalidated).
#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum StoreError {
    #[error("secure storage failed: {detail}")]
    Failed { detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for StoreError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        StoreError::Failed { detail: e.reason }
    }
}

/// Encrypted storage for the refresh token, implemented by the app (Android: AES-GCM with
/// a key held in the AndroidKeyStore).
#[uniffi::export(with_foreign)]
pub trait SecureStore: Send + Sync {
    fn load(&self) -> Result<Option<StoredCredentials>, StoreError>;
    fn save(&self, credentials: StoredCredentials) -> Result<(), StoreError>;
    fn clear(&self) -> Result<(), StoreError>;
}

/// Told when the sign-in ends.
#[uniffi::export(with_foreign)]
pub trait SessionObserver: Send + Sync {
    /// `revoked`: the server refused the sign-in (revoked, expired or signed out
    /// elsewhere), as opposed to the user signing out here.
    fn on_signed_out(&self, revoked: bool);
}

/// Renew this long before the access token expires.
const RENEW_BEFORE: Duration = Duration::from_secs(60);
const MAX_TOKEN: usize = 1024;

struct Access {
    token: String,
    renew_at: Instant,
}

#[derive(Default)]
struct State {
    access: Option<Access>,
    refresh: Option<String>,
    device_id: Option<i64>,
    who: Option<SignedIn>,
    /// Bumped by every sign-in and sign-out, so a refresh started before one can't apply
    /// its result afterwards.
    generation: u64,
}

pub(crate) struct Core {
    server: Arc<Server>,
    http: Http,
    store: Arc<dyn SecureStore>,
    device_name: String,
    /// What the gateway is told this client runs on (the phone's model, e.g. "Pixel 8").
    platform: String,
    state: Mutex<State>,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
    observer: Mutex<Option<Arc<dyn SessionObserver>>>,
}

/// A sign-in to one server. Create one per server; it restores a saved sign-in from the
/// [`SecureStore`].
#[derive(uniffi::Object)]
pub struct Session {
    core: Arc<Core>,
}

/// Tokens as the gateway issued them.
struct Issued {
    access: String,
    expires_in: u64,
    refresh: String,
    who: SignedIn,
}

fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= MAX_TOKEN && t.bytes().all(|b| b.is_ascii_graphic())
}

fn parse_who(v: &Value) -> Result<SignedIn, ApiError> {
    let user = &v["user"];
    let device = &v["device"];
    let username = user["username"]
        .as_str()
        .ok_or_else(|| ApiError::bad_response("no user in answer"))?;
    let role = match user["role"].as_str() {
        Some("admin") => UserRole::Admin,
        _ => UserRole::Listener,
    };
    let id = device["id"]
        .as_i64()
        .ok_or_else(|| ApiError::bad_response("no device in answer"))?;
    Ok(SignedIn {
        user: User {
            username: crate::model::clean_text(username),
            role,
        },
        device: Device {
            id,
            name: crate::model::clean_text(device["name"].as_str().unwrap_or("")),
            player_id: device["mac"]
                .as_str()
                .filter(|m| crate::commands::is_player_id(m))
                .map(str::to_ascii_lowercase)
                .unwrap_or_default(),
        },
    })
}

fn parse_issued(v: &Value) -> Result<Issued, ApiError> {
    let access = v["access_token"].as_str().filter(|t| valid_token(t));
    let refresh = v["refresh_token"].as_str().filter(|t| valid_token(t));
    let (Some(access), Some(refresh)) = (access, refresh) else {
        return Err(ApiError::bad_response("no tokens in answer"));
    };
    Ok(Issued {
        access: access.to_string(),
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
        refresh: refresh.to_string(),
        who: parse_who(v)?,
    })
}

/// The name this device shows in LMS and the device list: printable, at most 64 characters
/// (the gateway cleans it again).
fn clean_device_name(name: &str) -> String {
    let n: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string();
    if n.is_empty() { "Android".into() } else { n }
}

/// A device name the gateway takes: 1 to 64 characters once trimmed, no control characters
/// (it cleans names further). The trimmed name, or `None`.
fn valid_device_name(name: &str) -> Option<String> {
    let n = name.trim();
    let count = n.chars().count();
    ((1..=64).contains(&count) && !n.chars().any(char::is_control)).then(|| n.to_string())
}

/// The client platform sent at sign-in: the device name as the app first gave it, cut to
/// the gateway's 48 characters.
fn client_platform(device_name: &str) -> String {
    device_name
        .chars()
        .take(48)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A gateway older than client types refuses the unknown fields as a malformed request.
fn refused_as_malformed(e: &ApiError) -> bool {
    matches!(e, ApiError::InvalidInput { detail } if detail.contains("malformed request"))
}

impl Core {
    /// Add what kind of client this is to a sign-in body (shown to the server's admins).
    fn with_client(&self, mut body: Value) -> Value {
        body["client_kind"] = json!("android");
        body["client_platform"] = json!(self.platform);
        body
    }

    /// Sign in (login or pairing), telling the gateway what this client is. A gateway from
    /// before client types refuses the extra fields; then it is asked again without them.
    async fn sign_in(&self, path: &str, body: Value) -> Result<Value, ApiError> {
        match transport::send_json(self.post(path, &self.with_client(body.clone()))).await {
            Err(e) if refused_as_malformed(&e) => {
                transport::send_json(self.post(path, &body)).await
            }
            other => other,
        }
    }

    fn post(&self, path: &str, body: &Value) -> transport::Request {
        self.http
            .post(self.server.endpoint(path))
            .header(CLIENT_HEADER, "1")
            .json(body)
    }

    fn notify_signed_out(&self, revoked: bool) {
        let observer = self.observer.lock().unwrap().clone();
        if let Some(o) = observer {
            o.on_signed_out(revoked);
        }
    }

    /// Adopt newly issued tokens: persist the refresh token, then use the access token.
    /// `generation`: for a refresh, the sign-in it belongs to (refused if it has ended);
    /// `None` for a new sign-in, which supersedes everything before it.
    fn adopt(&self, t: Issued, generation: Option<u64>) -> Result<SignedIn, ApiError> {
        let mut st = self.state.lock().unwrap();
        match generation {
            Some(g) if g != st.generation => return Err(ApiError::Unauthorised),
            Some(_) => {}
            None => st.generation += 1,
        }
        // A store failure keeps the sign-in for this run of the app only.
        let _ = self.store.save(StoredCredentials {
            refresh_token: t.refresh.clone(),
            device_id: t.who.device.id,
        });
        let lifetime = Duration::from_secs(t.expires_in);
        st.access = Some(Access {
            token: t.access,
            renew_at: Instant::now()
                + lifetime
                    .saturating_sub(RENEW_BEFORE)
                    .max(Duration::from_secs(5)),
        });
        st.refresh = Some(t.refresh);
        st.device_id = Some(t.who.device.id);
        st.who = Some(t.who.clone());
        Ok(t.who)
    }

    /// End the sign-in locally (`generation`: only if it is still that sign-in).
    fn forget(&self, revoked: bool, generation: Option<u64>) {
        {
            let mut st = self.state.lock().unwrap();
            if generation.is_some_and(|g| g != st.generation) {
                return;
            }
            st.generation += 1;
            st.access = None;
            st.refresh = None;
            st.who = None;
            // device_id is kept in memory, so signing in again keeps the same player.
            let _ = self.store.clear();
        }
        self.notify_signed_out(revoked);
    }

    fn fresh_access(&self, stale: Option<&str>) -> Option<String> {
        let st = self.state.lock().unwrap();
        st.access
            .as_ref()
            .filter(|a| a.renew_at > Instant::now() && Some(a.token.as_str()) != stale)
            .map(|a| a.token.clone())
    }

    /// A usable access token, renewing it if it is missing or about to expire.
    pub(crate) async fn access_token(self: &Arc<Self>) -> Result<String, ApiError> {
        match self.fresh_access(None) {
            Some(t) => Ok(t),
            None => self.renew(None).await,
        }
    }

    /// Get a new access token, unless someone else already replaced `stale`.
    pub(crate) async fn renew(self: &Arc<Self>, stale: Option<&str>) -> Result<String, ApiError> {
        // Held by the refresh task itself, so a cancelled caller can't let a second refresh
        // of the same token start while the first is still running.
        let guard = self.refresh_lock.clone().lock_owned().await;
        if let Some(t) = self.fresh_access(stale) {
            return Ok(t);
        }
        let (refresh, generation) = {
            let st = self.state.lock().unwrap();
            (st.refresh.clone(), st.generation)
        };
        let refresh = refresh.ok_or(ApiError::Unauthorised)?;
        // Run the rotation in its own task: if our caller is cancelled half-way, the new
        // refresh token is still stored.
        let core = self.clone();
        let task = tokio::spawn(async move {
            let _guard = guard;
            let answer = transport::send_json(
                core.post("v1/auth/refresh", &json!({ "refresh_token": refresh })),
            )
            .await
            .and_then(|v| parse_issued(&v));
            match answer {
                Ok(t) => {
                    let token = t.access.clone();
                    core.adopt(t, Some(generation)).map(|_| token)
                }
                Err(e) if e.is_refusal() => {
                    core.forget(true, Some(generation));
                    Err(ApiError::Unauthorised)
                }
                Err(e) => Err(e),
            }
        });
        task.await
            .unwrap_or_else(|_| Err(ApiError::network("refresh interrupted")))
    }

    /// Run an authenticated call; on 401, renew the access token once and retry.
    pub(crate) async fn authed<T, F, Fut>(self: &Arc<Self>, call: F) -> Result<T, ApiError>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = Result<T, ApiError>>,
    {
        let token = self.access_token().await?;
        match call(token.clone()).await {
            Err(ApiError::Unauthorised) => {
                let fresh = self.renew(Some(&token)).await?;
                call(fresh).await
            }
            other => other,
        }
    }

    /// `POST /v1/rpc`: run an LMS command through the gateway's allow-list.
    pub(crate) async fn rpc(
        self: &Arc<Self>,
        player: &str,
        cmd: Vec<Value>,
    ) -> Result<Value, ApiError> {
        let body = json!({ "player": player, "cmd": cmd });
        if body.to_string().len() > 8 * 1024 {
            return Err(ApiError::invalid("command too long"));
        }
        let url = self.server.endpoint("v1/rpc");
        let v = self
            .authed(|token| {
                let req = self.http.post(url.clone()).bearer_auth(&token).json(&body);
                transport::send_json(req)
            })
            .await?;
        Ok(match v.get("result") {
            Some(r) if r.is_object() => r.clone(),
            _ => json!({}),
        })
    }

    /// An authenticated GET of a `/v1/…` JSON resource.
    async fn get_json(self: &Arc<Self>, path: &str) -> Result<Value, ApiError> {
        let url = self.server.endpoint(path);
        self.authed(|token| transport::send_json(self.http.get(url.clone()).bearer_auth(&token)))
            .await
    }

    pub(crate) fn server(&self) -> &Arc<Server> {
        &self.server
    }

    pub(crate) fn http(&self) -> &Http {
        &self.http
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Session {
    /// A session for `server`. A sign-in saved in `store` is picked up (use [`Self::restore`]).
    /// `device_name` is what this phone is called in LMS (e.g. "Pixel 8").
    #[uniffi::constructor]
    pub fn new(
        server: Arc<Server>,
        store: Arc<dyn SecureStore>,
        device_name: String,
    ) -> Result<Arc<Self>, ApiError> {
        let saved = store.load().ok().flatten();
        let state = State {
            refresh: saved
                .as_ref()
                .map(|c| c.refresh_token.clone())
                .filter(|t| valid_token(t)),
            device_id: saved.map(|c| c.device_id),
            ..State::default()
        };
        Ok(Arc::new(Session {
            core: Arc::new(Core {
                server,
                http: Http::shared()?,
                store,
                platform: client_platform(&clean_device_name(&device_name)),
                device_name: clean_device_name(&device_name),
                state: Mutex::new(state),
                refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
                observer: Mutex::new(None),
            }),
        }))
    }

    pub fn server(&self) -> Arc<Server> {
        self.core.server.clone()
    }

    /// Be told when the sign-in ends (replaces any earlier observer).
    pub fn set_observer(&self, observer: Arc<dyn SessionObserver>) {
        *self.core.observer.lock().unwrap() = Some(observer);
    }

    /// Whether a sign-in is saved or active (it may still turn out to be revoked).
    pub fn has_sign_in(&self) -> bool {
        self.core.state.lock().unwrap().refresh.is_some()
    }

    /// Who is signed in, if anyone (known after a sign-in or a successful restore).
    pub fn current(&self) -> Option<SignedIn> {
        self.core.state.lock().unwrap().who.clone()
    }

    /// Resume a saved sign-in. `Unauthorised` if there is none or it was refused (then
    /// it is forgotten); `Network` if the server can't be reached (it is kept).
    pub async fn restore(&self) -> Result<SignedIn, ApiError> {
        self.core.renew(None).await?;
        if let Some(who) = self.current() {
            return Ok(who);
        }
        self.me().await
    }

    /// Sign in with a user name and password, plus a two-step code when the server asked
    /// for one (`TwoStepRequired`).
    pub async fn login(
        &self,
        username: String,
        password: String,
        two_step_code: Option<String>,
    ) -> Result<SignedIn, ApiError> {
        let username = username.trim();
        if username.is_empty() || username.len() > 128 {
            return Err(ApiError::invalid("enter your user name"));
        }
        if password.is_empty() || password.len() > 1024 {
            return Err(ApiError::invalid("enter your password"));
        }
        let mut body = json!({
            "username": username,
            "password": password,
            "device_name": self.core.device_name,
            "refresh": "body",
        });
        if let Some(code) = two_step_code {
            let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
            if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
                return Err(ApiError::invalid(
                    "the code is the 6 digits from your authenticator app",
                ));
            }
            body["totp"] = json!(code);
        }
        if let Some(id) = self.core.state.lock().unwrap().device_id {
            body["device_id"] = json!(id);
        }
        let v = self.core.sign_in("v1/auth/login", body).await?;
        self.core.adopt(parse_issued(&v)?, None)
    }

    /// Sign in with a one-time pairing code (from the web app or the LMS plugin settings).
    pub async fn pair(&self, code: String) -> Result<SignedIn, ApiError> {
        let code = normalise_pairing_code(&code)?;
        let body = json!({ "code": code, "device_name": self.core.device_name });
        let v = self.core.sign_in("v1/auth/pair", body).await?;
        self.core.adopt(parse_issued(&v)?, None)
    }

    /// Get a new access token now (rotating the refresh token).
    pub async fn refresh(&self) -> Result<(), ApiError> {
        let stale = self.core.fresh_access(None);
        self.core
            .renew(Some(stale.as_deref().unwrap_or("")))
            .await
            .map(|_| ())
    }

    /// Sign out: revoke this device on the server, and forget the sign-in here whatever
    /// happens. Returns whether the server confirmed it (false when it couldn't be reached;
    /// the device then stays listed until it expires or an admin revokes it).
    pub async fn logout(&self) -> Result<bool, ApiError> {
        let confirmed = match self.core.access_token().await {
            Ok(token) => {
                let req = self
                    .core
                    .post("v1/auth/logout", &json!({}))
                    .bearer_auth(&token);
                match transport::send_json(req).await {
                    Ok(_) => true,
                    // Already revoked: as good as done.
                    Err(e) => e.is_refusal(),
                }
            }
            Err(ApiError::Unauthorised) => true,
            Err(_) => false,
        };
        self.core.forget(false, None);
        Ok(confirmed)
    }

    /// Rename this device (`PATCH /v1/devices/self`): its player in LMS follows. Returns the
    /// name the server stored (it cleans names: no markup or control characters).
    pub async fn rename_device(&self, name: String) -> Result<String, ApiError> {
        let name = valid_device_name(&name)
            .ok_or_else(|| ApiError::invalid("device names are 1 to 64 characters"))?;
        let url = self.core.server.endpoint("v1/devices/self");
        let core = &self.core;
        let v = core
            .authed(|token| {
                transport::send_json(
                    core.http
                        .patch(url.clone())
                        .header(CLIENT_HEADER, "1")
                        .bearer_auth(&token)
                        .json(&json!({ "name": name })),
                )
            })
            .await?;
        let stored = v["name"]
            .as_str()
            .map(crate::model::clean_text)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| ApiError::bad_response("no name in answer"))?;
        if let Some(who) = core.state.lock().unwrap().who.as_mut() {
            who.device.name = stored.clone();
        }
        Ok(stored)
    }

    /// `GET /v1/me`: who this token belongs to.
    pub async fn me(&self) -> Result<SignedIn, ApiError> {
        let v = self.core.get_json("v1/me").await?;
        let who = parse_who(&v)?;
        self.core.state.lock().unwrap().who = Some(who.clone());
        Ok(who)
    }

    /// `GET /v1/account`: account settings (two-step, downloads).
    pub async fn account(&self) -> Result<AccountInfo, ApiError> {
        let v = self.core.get_json("v1/account").await?;
        Ok(AccountInfo {
            username: crate::model::clean_text(v["username"].as_str().unwrap_or("")),
            is_admin: v["role"].as_str() == Some("admin"),
            two_step_enabled: v["totp_enabled"].as_bool().unwrap_or(false),
            downloads_enabled: v["downloads"]["enabled"].as_bool().unwrap_or(false),
        })
    }
}

impl Session {
    pub(crate) fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// Run any command array through `/v1/rpc` (for tests of the gateway's policy; the app
    /// uses the typed [`crate::MusicLibrary`] and [`crate::Players`] calls).
    #[doc(hidden)]
    pub async fn rpc_raw(&self, player: &str, cmd: Vec<Value>) -> Result<Value, ApiError> {
        self.core.rpc(player, cmd).await
    }

    /// Replace the access token with an invalid one, as if it had expired (tests only).
    #[doc(hidden)]
    pub fn debug_invalidate_access_token(&self) {
        if let Some(a) = self.core.state.lock().unwrap().access.as_mut() {
            a.token = "expired-token".into();
        }
    }

    /// The refresh token held in memory (tests only: rotation and reuse).
    #[doc(hidden)]
    pub fn debug_refresh_token(&self) -> Option<String> {
        self.core.state.lock().unwrap().refresh.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_must_be_header_safe() {
        assert!(valid_token("abc123_-.~"));
        assert!(!valid_token(""));
        assert!(!valid_token("a b"));
        assert!(!valid_token("a\r\nX-Evil: 1"));
        assert!(!valid_token(&"a".repeat(MAX_TOKEN + 1)));
        assert!(!valid_token("é"));
    }

    #[test]
    fn issued_tokens_are_validated() {
        let good = json!({
            "access_token": "acc", "refresh_token": "ref", "expires_in": 900,
            "user": {"username": "alice", "role": "listener"},
            "device": {"id": 4, "name": "Phone", "mac": "02:4C:41:00:00:01"}
        });
        let t = parse_issued(&good).unwrap();
        assert_eq!(t.who.device.player_id, "02:4c:41:00:00:01");
        assert_eq!(t.who.user.role, UserRole::Listener);

        let mut no_refresh = good.clone();
        no_refresh.as_object_mut().unwrap().remove("refresh_token");
        assert!(parse_issued(&no_refresh).is_err());
        let mut evil = good.clone();
        evil["access_token"] = json!("a\r\nb");
        assert!(parse_issued(&evil).is_err());
        let mut bad_mac = good.clone();
        bad_mac["device"]["mac"] = json!("not-a-mac");
        assert_eq!(parse_issued(&bad_mac).unwrap().who.device.player_id, "");
        assert!(parse_issued(&json!([])).is_err());
    }

    #[test]
    fn renamed_device_names_are_checked_before_sending() {
        assert_eq!(valid_device_name("  Kitchen  "), Some("Kitchen".into()));
        assert_eq!(valid_device_name(&"é".repeat(64)), Some("é".repeat(64)));
        for bad in ["", "   ", "a\nb", "a\u{0}b", "a\u{7f}b", "a\u{85}b"] {
            assert_eq!(valid_device_name(bad), None, "{bad:?}");
        }
        assert_eq!(valid_device_name(&"x".repeat(65)), None);
    }

    #[test]
    fn the_client_platform_fits_the_gateway() {
        assert_eq!(client_platform("Google Pixel 8"), "Google Pixel 8");
        assert_eq!(client_platform(&"x".repeat(100)).chars().count(), 48);
        assert!(refused_as_malformed(&ApiError::invalid(
            "malformed request"
        )));
        assert!(!refused_as_malformed(&ApiError::invalid(
            "enter your password"
        )));
        assert!(!refused_as_malformed(&ApiError::Unauthorised));
    }

    #[test]
    fn device_names_are_printable_and_short() {
        assert_eq!(clean_device_name("  Pixel\u{7} 8 \n"), "Pixel 8");
        assert_eq!(clean_device_name(""), "Android");
        assert_eq!(clean_device_name(&"x".repeat(200)).chars().count(), 64);
    }
}
