//! Account store: users, devices, tokens, pairing codes and the audit log (spec D12).
//!
//! - Passwords are argon2id (OWASP parameters: 19 MiB, 2 passes). An unknown username
//!   still costs a full hash, so response timing doesn't reveal which usernames exist.
//! - Access tokens (15 min) are random and held only in memory, hashed; revocation is
//!   immediate.
//! - Refresh tokens are random, stored hashed, and rotate on every use. Presenting an
//!   already-used refresh token means it was copied, so the whole device is revoked.
//! - Each device has its own player MAC (locally administered, `02:4c:41:…`).
//! - Failed logins and pairing attempts are rate-limited per key with escalating lockouts.
//! - Optional two-step verification (TOTP, RFC 6238) per user: once enabled, a login needs a
//!   current code as well as the password. Codes are single use, and wrong codes count
//!   towards the same lockouts as wrong passwords.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use anywhere_core::proto::format_mac;

pub const ACCESS_TTL: Duration = Duration::from_secs(15 * 60);
pub const REFRESH_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
pub const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
pub const MIN_PASSWORD: usize = 10;
pub const MAX_PASSWORD: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Listener,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Listener => "listener",
            Role::Admin => "admin",
        }
    }
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "listener" => Some(Role::Listener),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// Who a valid access token belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    pub device_id: i64,
    pub device_name: String,
    pub mac: [u8; 6],
}

impl Principal {
    pub fn mac_string(&self) -> String {
        format_mac(&self.mac)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub access_expires_in: u64,
    pub refresh: String,
    pub principal: Principal,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: &'static str,
    pub disabled: bool,
    pub totp_enabled: bool,
}

/// What a user needs to add two-step verification to an authenticator app.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TotpSetup {
    /// Base32 secret, for manual entry.
    pub secret: String,
    /// `otpauth://` URI, for a QR code.
    pub uri: String,
    /// The URI as an SVG QR code.
    pub qr_svg: String,
}

/// The players a principal may control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerScope {
    pub all: bool,
    pub players: HashSet<String>,
}

impl PlayerScope {
    pub fn allows(&self, mac: &str) -> bool {
        self.all || self.players.contains(&mac.to_ascii_lowercase())
    }
}

fn normalise_mac(mac: &str) -> Result<String, AuthError> {
    anywhere_core::proto::parse_mac(mac.trim())
        .map(|m| format_mac(&m))
        .ok_or(AuthError::BadInput(
            "player id must be a MAC address like aa:bb:cc:dd:ee:ff",
        ))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceInfo {
    pub id: i64,
    pub username: String,
    pub name: String,
    pub mac: String,
    pub created_at: i64,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuditEntry {
    pub at: i64,
    pub event: String,
    pub username: Option<String>,
    pub device_id: Option<i64>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Wrong credentials (deliberately unspecific).
    Invalid,
    /// Too many failures; retry after this long.
    Locked(Duration),
    /// A used refresh token was replayed; this device has been revoked.
    Reused(i64),
    /// The password was right; this user also needs a two-step code.
    TotpRequired,
    /// Input rejected (bad username, weak password, …).
    BadInput(&'static str),
    Storage(String),
}

impl From<rusqlite::Error> for AuthError {
    fn from(e: rusqlite::Error) -> Self {
        AuthError::Storage(e.to_string())
    }
}

struct AccessEntry {
    principal: Principal,
    expires: Instant,
}

struct Lock {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
    lockouts: u32,
}

/// Failures allowed per key within the window before a lockout.
const MAX_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const FIRST_LOCKOUT: Duration = Duration::from_secs(15 * 60);
const MAX_LOCKOUT: Duration = Duration::from_secs(24 * 3600);

pub struct Accounts {
    db: Mutex<Connection>,
    access: Mutex<HashMap<[u8; 32], AccessEntry>>,
    limits: Mutex<HashMap<String, Lock>>,
    /// A real argon2id hash, verified against for unknown usernames.
    dummy_hash: String,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn sha256(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS random source");
    b
}

fn new_secret() -> String {
    hex::encode(random_bytes::<32>())
}

fn hash_password(password: &str) -> Result<String, AuthError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Storage(e.to_string()))
}

fn valid_username(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 64
        && u.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn valid_device_name(n: &str) -> bool {
    !n.trim().is_empty() && n.chars().count() <= 64 && !n.chars().any(char::is_control)
}

/// Pairing codes: 10 characters from an unambiguous alphabet (~50 bits), easy to type.
const PAIRING_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

impl Accounts {
    pub fn open(path: &Path) -> Result<Accounts, AuthError> {
        Self::with_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Accounts, AuthError> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(db: Connection) -> Result<Accounts, AuthError> {
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS users (
               id INTEGER PRIMARY KEY,
               username TEXT NOT NULL UNIQUE,
               password_hash TEXT NOT NULL,
               role TEXT NOT NULL,
               disabled INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS devices (
               id INTEGER PRIMARY KEY,
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               name TEXT NOT NULL,
               mac TEXT NOT NULL UNIQUE,
               created_at INTEGER NOT NULL,
               last_seen INTEGER,
               revoked INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS refresh_tokens (
               token_hash BLOB PRIMARY KEY,
               device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
               expires_at INTEGER NOT NULL,
               used INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS pairing_codes (
               code_hash BLOB PRIMARY KEY,
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               expires_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS audit (
               id INTEGER PRIMARY KEY,
               at INTEGER NOT NULL,
               event TEXT NOT NULL,
               username TEXT,
               device_id INTEGER,
               detail TEXT NOT NULL);",
        )?;
        migrate(&db)?;
        Ok(Accounts {
            db: Mutex::new(db),
            access: Mutex::new(HashMap::new()),
            limits: Mutex::new(HashMap::new()),
            dummy_hash: hash_password(&new_secret())?,
        })
    }

    // -----------------------------------------------------------------------
    // Users (admin)
    // -----------------------------------------------------------------------

    pub fn create_user(
        &self,
        username: &str,
        password: &str,
        role: Role,
    ) -> Result<i64, AuthError> {
        if !valid_username(username) {
            return Err(AuthError::BadInput(
                "username must be 1–64 of a-z 0-9 . _ -",
            ));
        }
        check_password(password)?;
        let hash = hash_password(password)?;
        let db = self.db.lock().unwrap();
        let exists: Option<i64> = db
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                [username],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(AuthError::BadInput("username already exists"));
        }
        db.execute(
            "INSERT INTO users (username, password_hash, role, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![username, hash, role.as_str(), now_secs()],
        )?;
        let id = db.last_insert_rowid();
        drop(db);
        self.audit("user.created", Some(username), None, role.as_str());
        Ok(id)
    }

    pub fn set_password(&self, username: &str, password: &str) -> Result<(), AuthError> {
        check_password(password)?;
        let hash = hash_password(password)?;
        let n = self.db.lock().unwrap().execute(
            "UPDATE users SET password_hash = ?1 WHERE username = ?2",
            params![hash, username],
        )?;
        if n == 0 {
            return Err(AuthError::BadInput("no such user"));
        }
        self.audit("user.password_changed", Some(username), None, "");
        Ok(())
    }

    /// Disable or enable a user. Disabling revokes all their devices.
    pub fn set_disabled(&self, username: &str, disabled: bool) -> Result<Vec<i64>, AuthError> {
        let n = self.db.lock().unwrap().execute(
            "UPDATE users SET disabled = ?1 WHERE username = ?2",
            params![disabled as i64, username],
        )?;
        if n == 0 {
            return Err(AuthError::BadInput("no such user"));
        }
        self.audit(
            if disabled {
                "user.disabled"
            } else {
                "user.enabled"
            },
            Some(username),
            None,
            "",
        );
        if disabled {
            self.revoke_user_devices(username)
        } else {
            Ok(vec![])
        }
    }

    /// Delete a user and revoke their devices. Returns the revoked device ids.
    pub fn delete_user(&self, username: &str) -> Result<Vec<i64>, AuthError> {
        let revoked = self.revoke_user_devices(username)?;
        let n = self
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM users WHERE username = ?1", [username])?;
        if n == 0 {
            return Err(AuthError::BadInput("no such user"));
        }
        self.audit("user.deleted", Some(username), None, "");
        Ok(revoked)
    }

    pub fn list_users(&self) -> Result<Vec<UserInfo>, AuthError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT id, username, role, disabled, totp_enabled FROM users ORDER BY username",
        )?;
        let rows = st.query_map([], |r| {
            let role: String = r.get(2)?;
            Ok(UserInfo {
                id: r.get(0)?,
                username: r.get(1)?,
                role: Role::parse(&role).unwrap_or(Role::Listener).as_str(),
                disabled: r.get::<_, i64>(3)? != 0,
                totp_enabled: r.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn user_count(&self) -> Result<i64, AuthError> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)
    }

    // -----------------------------------------------------------------------
    // Login, pairing, refresh
    // -----------------------------------------------------------------------

    /// Check a username/password and issue tokens for a device: `device_id` reuses one of
    /// the user's own existing devices (a browser logging in again), otherwise a new one is
    /// created. Rate-limited per username and per `client` (e.g. IP address).
    pub fn login(
        &self,
        username: &str,
        password: &str,
        totp: Option<&str>,
        device_name: &str,
        device_id: Option<i64>,
        client: &str,
    ) -> Result<Tokens, AuthError> {
        let keys = [format!("user:{username}"), format!("client:{client}")];
        for k in &keys {
            self.check_lock(k)?;
        }
        if !valid_device_name(device_name) {
            return Err(AuthError::BadInput(
                "device name must be 1–64 printable characters",
            ));
        }
        let row: Option<(i64, String, String, i64)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id, password_hash, role, disabled FROM users WHERE username = ?1",
                [username],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        // Verify even for unknown users, so timing doesn't reveal which usernames exist.
        let hash = row
            .as_ref()
            .map_or(self.dummy_hash.as_str(), |r| r.1.as_str());
        let ok = password.len() <= MAX_PASSWORD
            && Argon2::default()
                .verify_password(password.as_bytes(), hash)
                .is_ok();
        let user = match row {
            Some((id, _, role, disabled)) if ok && disabled == 0 => {
                (id, Role::parse(&role).unwrap_or(Role::Listener))
            }
            _ => {
                for k in &keys {
                    self.record_failure(k);
                }
                self.audit("login.failed", Some(username), None, client);
                return Err(AuthError::Invalid);
            }
        };
        self.check_second_factor(user.0, username, totp, &keys, client)?;
        for k in &keys {
            self.limits.lock().unwrap().remove(k);
        }
        let device = match device_id {
            Some(id) => match self.own_device(user.0, id)? {
                Some(d) => d,
                None => self.create_device(user.0, device_name)?,
            },
            None => self.create_device(user.0, device_name)?,
        };
        self.audit("login.ok", Some(username), Some(device), client);
        self.issue(device)
    }

    /// Create a one-time pairing code for a user's new app (admin).
    pub fn create_pairing_code(&self, username: &str) -> Result<String, AuthError> {
        let db = self.db.lock().unwrap();
        let user: Option<i64> = db
            .query_row(
                "SELECT id FROM users WHERE username = ?1 AND disabled = 0",
                [username],
                |r| r.get(0),
            )
            .optional()?;
        let user = user.ok_or(AuthError::BadInput("no such enabled user"))?;
        let raw = random_bytes::<10>();
        let code: String = raw
            .iter()
            .map(|b| PAIRING_ALPHABET[(*b as usize) % 32] as char)
            .collect();
        db.execute(
            "INSERT INTO pairing_codes (code_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![
                sha256(&code).to_vec(),
                user,
                now_secs() + PAIRING_TTL.as_secs() as i64
            ],
        )?;
        drop(db);
        self.audit("pairing.created", Some(username), None, "");
        Ok(code)
    }

    /// Redeem a pairing code (single use) for a new device's tokens.
    pub fn pair(&self, code: &str, device_name: &str, client: &str) -> Result<Tokens, AuthError> {
        let key = format!("client:{client}");
        self.check_lock(&key)?;
        if !valid_device_name(device_name) {
            return Err(AuthError::BadInput(
                "device name must be 1–64 printable characters",
            ));
        }
        let normalised: String = code
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect::<String>()
            .to_uppercase();
        let found: Option<(i64, i64)> = {
            let db = self.db.lock().unwrap();
            let row = db
                .query_row(
                    "SELECT user_id, expires_at FROM pairing_codes WHERE code_hash = ?1",
                    [sha256(&normalised).to_vec()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            // Single use, whatever the outcome.
            db.execute(
                "DELETE FROM pairing_codes WHERE code_hash = ?1",
                [sha256(&normalised).to_vec()],
            )?;
            row
        };
        let user = match found {
            Some((user, expires)) if expires > now_secs() => user,
            _ => {
                self.record_failure(&key);
                self.audit("pairing.failed", None, None, client);
                return Err(AuthError::Invalid);
            }
        };
        let disabled: i64 = self.db.lock().unwrap().query_row(
            "SELECT disabled FROM users WHERE id = ?1",
            [user],
            |r| r.get(0),
        )?;
        if disabled != 0 {
            return Err(AuthError::Invalid);
        }
        let device = self.create_device(user, device_name)?;
        self.audit("pairing.ok", None, Some(device), client);
        self.issue(device)
    }

    /// Rotate a refresh token. Replaying a used one revokes the device.
    pub fn refresh(&self, refresh_token: &str) -> Result<Tokens, AuthError> {
        let h = sha256(refresh_token).to_vec();
        let row: Option<(i64, i64, i64)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT device_id, expires_at, used FROM refresh_tokens WHERE token_hash = ?1",
                [&h],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((device, expires, used)) = row else {
            return Err(AuthError::Invalid);
        };
        if used != 0 {
            self.revoke_device(device)?;
            self.audit("refresh.reused", None, Some(device), "device revoked");
            return Err(AuthError::Reused(device));
        }
        if expires <= now_secs() {
            return Err(AuthError::Invalid);
        }
        self.db.lock().unwrap().execute(
            "UPDATE refresh_tokens SET used = 1 WHERE token_hash = ?1",
            [&h],
        )?;
        self.issue(device)
    }

    /// The principal for a valid, unexpired access token.
    pub fn authenticate(&self, access_token: &str) -> Option<Principal> {
        if access_token.is_empty() || access_token.len() > 256 {
            return None;
        }
        let key = sha256(access_token);
        let mut map = self.access.lock().unwrap();
        match map.get(&key) {
            Some(e) if e.expires > Instant::now() => Some(e.principal.clone()),
            Some(_) => {
                map.remove(&key);
                None
            }
            None => None,
        }
    }

    /// Log out: revoke the device the access token belongs to.
    pub fn logout(&self, access_token: &str) -> Result<Option<i64>, AuthError> {
        match self.authenticate(access_token) {
            Some(p) => {
                self.revoke_device(p.device_id)?;
                self.audit("logout", Some(&p.username), Some(p.device_id), "");
                Ok(Some(p.device_id))
            }
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // Two-step verification
    // -----------------------------------------------------------------------

    fn totp_state(&self, user: i64) -> Result<(Option<Vec<u8>>, bool, u64), AuthError> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT totp_secret, totp_enabled, totp_last_step FROM users WHERE id = ?1",
            [user],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, i64>(2)? as u64,
                ))
            },
        )?)
    }

    /// Accept a code (single use) and record its step. Returns false if it was wrong.
    fn consume_code(
        &self,
        user: i64,
        secret: &[u8],
        code: &str,
        last_step: u64,
    ) -> Result<bool, AuthError> {
        match crate::totp::verify(secret, code, unix_now(), last_step) {
            Some(step) => {
                self.db.lock().unwrap().execute(
                    "UPDATE users SET totp_last_step = ?1 WHERE id = ?2",
                    params![step as i64, user],
                )?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn check_second_factor(
        &self,
        user: i64,
        username: &str,
        code: Option<&str>,
        keys: &[String],
        client: &str,
    ) -> Result<(), AuthError> {
        let (secret, enabled, last_step) = self.totp_state(user)?;
        let Some(secret) = secret.filter(|_| enabled) else {
            return Ok(());
        };
        let Some(code) = code else {
            return Err(AuthError::TotpRequired);
        };
        if self.consume_code(user, &secret, code, last_step)? {
            return Ok(());
        }
        for k in keys {
            self.record_failure(k);
        }
        self.audit("login.totp_failed", Some(username), None, client);
        Err(AuthError::Invalid)
    }

    /// Start enrolment: a new secret for the user's authenticator app. Not active until
    /// confirmed with a code from the app.
    pub fn totp_begin(&self, user: i64, username: &str) -> Result<TotpSetup, AuthError> {
        let (_, enabled, _) = self.totp_state(user)?;
        if enabled {
            return Err(AuthError::BadInput("two-step verification is already on"));
        }
        let secret = crate::totp::new_secret();
        self.db.lock().unwrap().execute(
            "UPDATE users SET totp_secret = ?1, totp_enabled = 0, totp_last_step = 0 WHERE id = ?2",
            params![secret.to_vec(), user],
        )?;
        let uri = crate::totp::otpauth_uri(&secret, "Lyrion Anywhere", username);
        Ok(TotpSetup {
            secret: crate::totp::base32(&secret),
            qr_svg: crate::totp::qr_svg(&uri),
            uri,
        })
    }

    /// Finish enrolment with a code from the app.
    pub fn totp_confirm(&self, user: i64, username: &str, code: &str) -> Result<(), AuthError> {
        let (secret, enabled, _) = self.totp_state(user)?;
        let secret = match (secret, enabled) {
            (_, true) => return Err(AuthError::BadInput("two-step verification is already on")),
            (Some(s), false) => s,
            (None, false) => return Err(AuthError::BadInput("start two-step setup first")),
        };
        if !self.consume_code(user, &secret, code, 0)? {
            return Err(AuthError::BadInput(
                "that code is not right; check the app and try again",
            ));
        }
        self.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET totp_enabled = 1 WHERE id = ?1", [user])?;
        self.audit("totp.enabled", Some(username), None, "");
        Ok(())
    }

    /// Turn two-step verification off: needs the password and a current code.
    pub fn totp_disable(
        &self,
        user: i64,
        username: &str,
        password: &str,
        code: &str,
    ) -> Result<(), AuthError> {
        let key = format!("user:{username}");
        self.check_lock(&key)?;
        let hash: String = self.db.lock().unwrap().query_row(
            "SELECT password_hash FROM users WHERE id = ?1",
            [user],
            |r| r.get(0),
        )?;
        let password_ok = password.len() <= MAX_PASSWORD
            && Argon2::default()
                .verify_password(password.as_bytes(), hash.as_str())
                .is_ok();
        let (secret, enabled, last_step) = self.totp_state(user)?;
        let code_ok = match (secret, enabled) {
            (Some(s), true) => password_ok && self.consume_code(user, &s, code, last_step)?,
            _ => return Err(AuthError::BadInput("two-step verification is not on")),
        };
        if !code_ok {
            self.record_failure(&key);
            self.audit("totp.disable_failed", Some(username), None, "");
            return Err(AuthError::Invalid);
        }
        self.clear_totp(user)?;
        self.audit("totp.disabled", Some(username), None, "");
        Ok(())
    }

    /// Admin: remove a user's two-step verification (e.g. a lost phone).
    pub fn totp_reset(&self, username: &str) -> Result<(), AuthError> {
        let user: Option<i64> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                [username],
                |r| r.get(0),
            )
            .optional()?;
        let user = user.ok_or(AuthError::BadInput("no such user"))?;
        self.clear_totp(user)?;
        self.audit("totp.reset", Some(username), None, "by admin");
        Ok(())
    }

    pub fn totp_enabled(&self, user: i64) -> Result<bool, AuthError> {
        Ok(self.totp_state(user)?.1)
    }

    fn clear_totp(&self, user: i64) -> Result<(), AuthError> {
        self.db.lock().unwrap().execute(
            "UPDATE users SET totp_secret = NULL, totp_enabled = 0, totp_last_step = 0 WHERE id = ?1",
            [user],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Player permissions
    // -----------------------------------------------------------------------

    /// Which LMS players this principal may control: admins all of them; listeners their own
    /// devices plus home players an admin has granted.
    pub fn player_scope(&self, p: &Principal) -> Result<PlayerScope, AuthError> {
        if p.role == Role::Admin {
            return Ok(PlayerScope {
                all: true,
                players: HashSet::new(),
            });
        }
        let db = self.db.lock().unwrap();
        let mut players: HashSet<String> = HashSet::new();
        let mut st = db.prepare("SELECT mac FROM devices WHERE user_id = ?1 AND revoked = 0")?;
        for mac in st.query_map([p.user_id], |r| r.get::<_, String>(0))? {
            players.insert(mac?);
        }
        let mut st = db.prepare("SELECT mac FROM user_players WHERE user_id = ?1")?;
        for mac in st.query_map([p.user_id], |r| r.get::<_, String>(0))? {
            players.insert(mac?);
        }
        Ok(PlayerScope {
            all: false,
            players,
        })
    }

    /// Admin: let a user control a home player.
    pub fn grant_player(&self, username: &str, mac: &str) -> Result<(), AuthError> {
        let mac = normalise_mac(mac)?;
        let n = self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO user_players (user_id, mac)
             SELECT id, ?2 FROM users WHERE username = ?1",
            params![username, mac],
        )?;
        if n == 0 && !self.players_granted(username)?.contains(&mac) {
            return Err(AuthError::BadInput("no such user"));
        }
        self.audit("player.granted", Some(username), None, &mac);
        Ok(())
    }

    pub fn ungrant_player(&self, username: &str, mac: &str) -> Result<(), AuthError> {
        let mac = normalise_mac(mac)?;
        self.db.lock().unwrap().execute(
            "DELETE FROM user_players WHERE mac = ?2 AND user_id = (SELECT id FROM users WHERE username = ?1)",
            params![username, mac],
        )?;
        self.audit("player.ungranted", Some(username), None, &mac);
        Ok(())
    }

    pub fn players_granted(&self, username: &str) -> Result<Vec<String>, AuthError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT p.mac FROM user_players p JOIN users u ON u.id = p.user_id
             WHERE u.username = ?1 ORDER BY p.mac",
        )?;
        let rows = st.query_map([username], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // -----------------------------------------------------------------------
    // Devices
    // -----------------------------------------------------------------------

    pub fn list_devices(&self) -> Result<Vec<DeviceInfo>, AuthError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT d.id, u.username, d.name, d.mac, d.created_at, d.last_seen
             FROM devices d JOIN users u ON u.id = d.user_id WHERE d.revoked = 0 ORDER BY d.id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(DeviceInfo {
                id: r.get(0)?,
                username: r.get(1)?,
                name: r.get(2)?,
                mac: r.get(3)?,
                created_at: r.get(4)?,
                last_seen: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    #[cfg(test)]
    fn devices_for_test(&self) -> i64 {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO devices (user_id, name, mac, created_at)
             VALUES ((SELECT id FROM users ORDER BY id LIMIT 1), 'test', '02:4c:41:00:00:99', 0)",
            [],
        )
        .unwrap();
        db.last_insert_rowid()
    }

    /// Store a device's latest diagnostics report (already sanitised JSON).
    pub fn save_diagnostics(&self, device_id: i64, report: &str) -> Result<(), AuthError> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO device_diagnostics (device_id, updated_at, report) VALUES (?1, ?2, ?3)
             ON CONFLICT(device_id) DO UPDATE SET updated_at = excluded.updated_at, report = excluded.report",
            rusqlite::params![device_id, unix_now() as i64, report],
        )?;
        Ok(())
    }

    /// A device's latest diagnostics: (updated_at, report JSON).
    pub fn diagnostics(&self, device_id: i64) -> Result<Option<(i64, String)>, AuthError> {
        let db = self.db.lock().unwrap();
        let mut st =
            db.prepare("SELECT updated_at, report FROM device_diagnostics WHERE device_id = ?1")?;
        let mut rows = st.query([device_id])?;
        Ok(match rows.next()? {
            Some(r) => Some((r.get(0)?, r.get(1)?)),
            None => None,
        })
    }

    /// Revoke a device: its refresh tokens are deleted and its access tokens dropped.
    pub fn revoke_device(&self, device_id: i64) -> Result<bool, AuthError> {
        let n = {
            let db = self.db.lock().unwrap();
            let n = db.execute(
                "UPDATE devices SET revoked = 1 WHERE id = ?1 AND revoked = 0",
                [device_id],
            )?;
            db.execute(
                "DELETE FROM refresh_tokens WHERE device_id = ?1",
                [device_id],
            )?;
            db.execute(
                "DELETE FROM device_diagnostics WHERE device_id = ?1",
                [device_id],
            )?;
            n
        };
        self.access
            .lock()
            .unwrap()
            .retain(|_, e| e.principal.device_id != device_id);
        if n > 0 {
            self.audit("device.revoked", None, Some(device_id), "");
        }
        Ok(n > 0)
    }

    fn revoke_user_devices(&self, username: &str) -> Result<Vec<i64>, AuthError> {
        let ids: Vec<i64> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare(
                "SELECT d.id FROM devices d JOIN users u ON u.id = d.user_id WHERE u.username = ?1 AND d.revoked = 0",
            )?;
            st.query_map([username], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for id in &ids {
            self.revoke_device(*id)?;
        }
        Ok(ids)
    }

    /// The device exists, isn't revoked, and its user isn't disabled.
    pub fn device_active(&self, device_id: i64) -> bool {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT 1 FROM devices d JOIN users u ON u.id = d.user_id
                 WHERE d.id = ?1 AND d.revoked = 0 AND u.disabled = 0",
                [device_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn touch_device(&self, device_id: i64) {
        let _ = self.db.lock().unwrap().execute(
            "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
            params![now_secs(), device_id],
        );
    }

    fn own_device(&self, user: i64, device: i64) -> Result<Option<i64>, AuthError> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM devices WHERE id = ?1 AND user_id = ?2 AND revoked = 0",
                params![device, user],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn create_device(&self, user: i64, name: &str) -> Result<i64, AuthError> {
        let db = self.db.lock().unwrap();
        for _ in 0..16 {
            let r = random_bytes::<3>();
            let mac = format_mac(&[0x02, 0x4c, 0x41, r[0], r[1], r[2]]);
            let taken: Option<i64> = db
                .query_row("SELECT id FROM devices WHERE mac = ?1", [&mac], |r| {
                    r.get(0)
                })
                .optional()?;
            if taken.is_none() {
                db.execute(
                    "INSERT INTO devices (user_id, name, mac, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![user, name.trim(), mac, now_secs()],
                )?;
                return Ok(db.last_insert_rowid());
            }
        }
        Err(AuthError::Storage("could not allocate a player MAC".into()))
    }

    /// Issue a fresh access + refresh pair for a device.
    fn issue(&self, device: i64) -> Result<Tokens, AuthError> {
        let row: (i64, String, String, i64, String, String, i64, i64) =
            self.db.lock().unwrap().query_row(
                "SELECT u.id, u.username, u.role, d.id, d.name, d.mac, d.revoked, u.disabled
             FROM devices d JOIN users u ON u.id = d.user_id WHERE d.id = ?1",
                [device],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )?;
        if row.6 != 0 || row.7 != 0 {
            return Err(AuthError::Invalid);
        }
        let mac = anywhere_core::proto::parse_mac(&row.5)
            .ok_or(AuthError::Storage("bad stored MAC".into()))?;
        let principal = Principal {
            user_id: row.0,
            username: row.1,
            role: Role::parse(&row.2).unwrap_or(Role::Listener),
            device_id: row.3,
            device_name: row.4,
            mac,
        };
        let access = new_secret();
        let refresh = new_secret();
        self.db.lock().unwrap().execute(
            "INSERT INTO refresh_tokens (token_hash, device_id, expires_at) VALUES (?1, ?2, ?3)",
            params![
                sha256(&refresh).to_vec(),
                device,
                now_secs() + REFRESH_TTL.as_secs() as i64
            ],
        )?;
        let mut map = self.access.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, e| e.expires > now);
        map.insert(
            sha256(&access),
            AccessEntry {
                principal: principal.clone(),
                expires: now + ACCESS_TTL,
            },
        );
        Ok(Tokens {
            access,
            access_expires_in: ACCESS_TTL.as_secs(),
            refresh,
            principal,
        })
    }

    // -----------------------------------------------------------------------
    // Rate limiting and audit
    // -----------------------------------------------------------------------

    fn check_lock(&self, key: &str) -> Result<(), AuthError> {
        let limits = self.limits.lock().unwrap();
        if let Some(l) = limits.get(key)
            && let Some(until) = l.locked_until
        {
            let now = Instant::now();
            if until > now {
                return Err(AuthError::Locked(until - now));
            }
        }
        Ok(())
    }

    fn record_failure(&self, key: &str) {
        let mut limits = self.limits.lock().unwrap();
        let now = Instant::now();
        let l = limits.entry(key.to_string()).or_insert(Lock {
            failures: 0,
            window_start: now,
            locked_until: None,
            lockouts: 0,
        });
        if now.duration_since(l.window_start) > FAILURE_WINDOW {
            l.failures = 0;
            l.window_start = now;
        }
        l.failures += 1;
        if l.failures >= MAX_FAILURES {
            let lock = FIRST_LOCKOUT
                .saturating_mul(1 << l.lockouts.min(10))
                .min(MAX_LOCKOUT);
            l.locked_until = Some(now + lock);
            l.lockouts += 1;
            l.failures = 0;
        }
    }

    pub fn audit(&self, event: &str, username: Option<&str>, device: Option<i64>, detail: &str) {
        let _ = self.db.lock().unwrap().execute(
            "INSERT INTO audit (at, event, username, device_id, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now_secs(), event, username, device, detail],
        );
    }

    pub fn audit_log(&self, limit: u32) -> Result<Vec<AuditEntry>, AuthError> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT at, event, username, device_id, detail FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit.min(1000)], |r| {
            Ok(AuditEntry {
                at: r.get(0)?,
                event: r.get(1)?,
                username: r.get(2)?,
                device_id: r.get(3)?,
                detail: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    #[cfg(test)]
    fn expire_access_tokens(&self) {
        for e in self.access.lock().unwrap().values_mut() {
            e.expires = Instant::now() - Duration::from_secs(1);
        }
    }
}

/// Schema upgrades, tracked with `PRAGMA user_version`.
fn migrate(db: &Connection) -> Result<(), AuthError> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        db.execute_batch(
            "ALTER TABLE users ADD COLUMN totp_secret BLOB;
             ALTER TABLE users ADD COLUMN totp_enabled INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE users ADD COLUMN totp_last_step INTEGER NOT NULL DEFAULT 0;
             PRAGMA user_version = 1;",
        )?;
    }
    if version < 2 {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_players (
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               mac TEXT NOT NULL,
               PRIMARY KEY (user_id, mac));
             PRAGMA user_version = 2;",
        )?;
    }
    if version < 3 {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS device_diagnostics (
               device_id INTEGER PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
               updated_at INTEGER NOT NULL,
               report TEXT NOT NULL);
             PRAGMA user_version = 3;",
        )?;
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn check_password(p: &str) -> Result<(), AuthError> {
    if p.chars().count() < MIN_PASSWORD {
        return Err(AuthError::BadInput(
            "password must be at least 10 characters",
        ));
    }
    if p.len() > MAX_PASSWORD {
        return Err(AuthError::BadInput("password too long"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PW: &str = "correct horse battery";

    fn store() -> Accounts {
        let a = Accounts::in_memory().unwrap();
        a.create_user("alice", PW, Role::Listener).unwrap();
        a.create_user("admin", PW, Role::Admin).unwrap();
        a
    }

    #[test]
    fn login_issues_tokens_for_a_new_device_with_its_own_mac() {
        let a = store();
        let t = a
            .login("alice", PW, None, "Laptop", None, "10.0.0.1")
            .unwrap();
        assert_eq!(t.principal.username, "alice");
        assert_eq!(&t.principal.mac[..3], &[0x02, 0x4c, 0x41]);
        assert_eq!(a.authenticate(&t.access).unwrap(), t.principal);
        let t2 = a
            .login("alice", PW, None, "Phone", None, "10.0.0.1")
            .unwrap();
        assert_ne!(
            t.principal.mac, t2.principal.mac,
            "each device gets its own player"
        );
        // Logging in again from the same browser reuses its device.
        let t3 = a
            .login(
                "alice",
                PW,
                None,
                "Laptop",
                Some(t.principal.device_id),
                "10.0.0.1",
            )
            .unwrap();
        assert_eq!(t3.principal.device_id, t.principal.device_id);
        assert_eq!(t3.principal.mac, t.principal.mac);
    }

    #[test]
    fn device_reuse_is_limited_to_the_users_own_devices() {
        let a = store();
        let admins = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        let t = a
            .login(
                "alice",
                PW,
                None,
                "Laptop",
                Some(admins.principal.device_id),
                "c",
            )
            .unwrap();
        assert_ne!(t.principal.device_id, admins.principal.device_id);
    }

    #[test]
    fn wrong_credentials_are_indistinguishable() {
        let a = store();
        assert_eq!(
            a.login("alice", "wrong password!", None, "x", None, "c1"),
            Err(AuthError::Invalid)
        );
        assert_eq!(
            a.login("nobody", PW, None, "x", None, "c2"),
            Err(AuthError::Invalid)
        );
        assert_eq!(
            a.login("alice", &"x".repeat(5000), None, "x", None, "c3"),
            Err(AuthError::Invalid)
        );
    }

    #[test]
    fn repeated_failures_lock_the_username_even_from_new_clients() {
        let a = store();
        for i in 0..MAX_FAILURES {
            assert_eq!(
                a.login(
                    "alice",
                    "wrong password!",
                    None,
                    "x",
                    None,
                    &format!("ip{i}")
                ),
                Err(AuthError::Invalid)
            );
        }
        // Locked: even the right password from a fresh address is refused for now.
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "ip-new"),
            Err(AuthError::Locked(_))
        ));
        // Other users are unaffected.
        assert!(a.login("admin", PW, None, "x", None, "ip-other").is_ok());
    }

    #[test]
    fn repeated_failures_lock_the_client_across_usernames() {
        let a = store();
        for i in 0..MAX_FAILURES {
            let _ = a.login(
                &format!("guess{i}"),
                "whatever!!",
                None,
                "x",
                None,
                "attacker",
            );
        }
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "attacker"),
            Err(AuthError::Locked(_))
        ));
    }

    #[test]
    fn disabled_users_cannot_log_in_and_lose_their_devices() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let revoked = a.set_disabled("alice", true).unwrap();
        assert_eq!(revoked, vec![t.principal.device_id]);
        assert!(
            a.authenticate(&t.access).is_none(),
            "access tokens die immediately"
        );
        assert_eq!(a.refresh(&t.refresh), Err(AuthError::Invalid));
        assert_eq!(
            a.login("alice", PW, None, "Laptop", None, "c"),
            Err(AuthError::Invalid)
        );
        assert!(
            a.list_devices()
                .unwrap()
                .iter()
                .all(|d| d.username != "alice"),
            "a refused login must not create a device"
        );
    }

    #[test]
    fn refresh_rotates_and_detects_reuse() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let t2 = a.refresh(&t.refresh).unwrap();
        assert_ne!(t2.refresh, t.refresh);
        assert_ne!(t2.access, t.access);
        assert_eq!(t2.principal.device_id, t.principal.device_id);
        // The old refresh token was stolen and replayed: revoke the device entirely.
        assert_eq!(
            a.refresh(&t.refresh),
            Err(AuthError::Reused(t.principal.device_id))
        );
        assert!(a.authenticate(&t2.access).is_none());
        assert_eq!(a.refresh(&t2.refresh), Err(AuthError::Invalid));
        assert!(a.list_devices().unwrap().is_empty());
    }

    #[test]
    fn access_tokens_expire() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        a.expire_access_tokens();
        assert!(a.authenticate(&t.access).is_none());
        // The refresh token still works to get a new one.
        assert!(a.refresh(&t.refresh).is_ok());
    }

    #[test]
    fn garbage_tokens_are_rejected() {
        let a = store();
        for t in ["", "x", &"a".repeat(64), &"a".repeat(10_000)] {
            assert!(a.authenticate(t).is_none());
            assert_eq!(a.refresh(t), Err(AuthError::Invalid));
        }
    }

    #[test]
    fn logout_revokes_the_device() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        assert_eq!(a.logout(&t.access).unwrap(), Some(t.principal.device_id));
        assert!(a.authenticate(&t.access).is_none());
        assert_eq!(a.refresh(&t.refresh), Err(AuthError::Invalid));
    }

    #[test]
    fn pairing_codes_are_single_use_and_expire() {
        let a = store();
        let code = a.create_pairing_code("alice").unwrap();
        assert_eq!(code.len(), 10);
        // Typed with a dash, spaces and lower case: still accepted.
        let typed = format!("{}-{} ", code[..5].to_lowercase(), &code[5..]);
        let t = a.pair(&typed, "Phone", "c").unwrap();
        assert_eq!(t.principal.username, "alice");
        assert_eq!(
            a.pair(&code, "Phone 2", "c"),
            Err(AuthError::Invalid),
            "single use"
        );
        assert!(a.create_pairing_code("nobody").is_err());
        // Expired codes are refused.
        let code = a.create_pairing_code("alice").unwrap();
        a.db.lock()
            .unwrap()
            .execute("UPDATE pairing_codes SET expires_at = 0", [])
            .unwrap();
        assert_eq!(a.pair(&code, "Phone", "c"), Err(AuthError::Invalid));
    }

    #[test]
    fn pairing_guesses_are_rate_limited() {
        let a = store();
        let code = a.create_pairing_code("alice").unwrap();
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                a.pair("AAAAAAAAAA", "x", "guesser"),
                Err(AuthError::Invalid)
            );
        }
        assert!(matches!(
            a.pair(&code, "x", "guesser"),
            Err(AuthError::Locked(_))
        ));
    }

    #[test]
    fn input_validation() {
        let a = store();
        for bad in ["", "Alice", "a b", "a/b", &"a".repeat(65), "ünïcode"] {
            assert!(
                matches!(
                    a.create_user(bad, PW, Role::Listener),
                    Err(AuthError::BadInput(_))
                ),
                "{bad:?}"
            );
        }
        assert!(matches!(
            a.create_user("bob", "short", Role::Listener),
            Err(AuthError::BadInput(_))
        ));
        assert!(
            matches!(
                a.create_user("alice", PW, Role::Listener),
                Err(AuthError::BadInput(_))
            ),
            "duplicate"
        );
        for bad in ["", "   ", "bad\nname", &"x".repeat(65)] {
            assert!(
                matches!(
                    a.login("alice", PW, None, bad, None, "c"),
                    Err(AuthError::BadInput(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn passwords_are_stored_as_argon2id_hashes() {
        let a = store();
        let hash: String =
            a.db.lock()
                .unwrap()
                .query_row(
                    "SELECT password_hash FROM users WHERE username = 'alice'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert!(hash.starts_with("$argon2id$"), "{hash}");
        assert!(!hash.contains(PW));
    }

    #[test]
    fn tokens_are_stored_only_as_hashes() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let stored: Vec<u8> =
            a.db.lock()
                .unwrap()
                .query_row("SELECT token_hash FROM refresh_tokens", [], |r| r.get(0))
                .unwrap();
        assert_eq!(stored, sha256(&t.refresh).to_vec());
        assert!(
            !a.access
                .lock()
                .unwrap()
                .keys()
                .any(|k| k == t.access.as_bytes())
        );
    }

    #[test]
    fn audit_log_records_security_events() {
        let a = store();
        let _ = a.login("alice", "wrong password!", None, "x", None, "10.0.0.9");
        let t = a
            .login("alice", PW, None, "Laptop", None, "10.0.0.9")
            .unwrap();
        let _ = a.refresh(&t.refresh);
        let _ = a.refresh(&t.refresh); // reuse
        let events: Vec<String> = a
            .audit_log(50)
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect();
        for e in [
            "login.failed",
            "login.ok",
            "refresh.reused",
            "device.revoked",
        ] {
            assert!(events.contains(&e.to_string()), "missing {e}: {events:?}");
        }
    }

    // -- two-step verification --------------------------------------------

    /// The current code for a user's (pending or active) secret.
    fn current_code(a: &Accounts, username: &str, steps_ahead: u64) -> String {
        let secret: Vec<u8> =
            a.db.lock()
                .unwrap()
                .query_row(
                    "SELECT totp_secret FROM users WHERE username = ?1",
                    [username],
                    |r| r.get(0),
                )
                .unwrap();
        let step = unix_now() / crate::totp::STEP_SECS + steps_ahead;
        format!("{:06}", crate::totp::code_at(&secret, step))
    }

    fn user_id(a: &Accounts, username: &str) -> i64 {
        a.db.lock()
            .unwrap()
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                [username],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn enrol(a: &Accounts, username: &str) {
        let id = user_id(a, username);
        let setup = a.totp_begin(id, username).unwrap();
        assert_eq!(setup.secret.len(), 32, "160-bit secret in base32");
        assert!(setup.uri.starts_with("otpauth://totp/Lyrion%20Anywhere:"));
        a.totp_confirm(id, username, &current_code(a, username, 0))
            .unwrap();
    }

    #[test]
    fn enrolment_needs_a_correct_code_from_the_app() {
        let a = store();
        let id = user_id(&a, "alice");
        assert!(
            matches!(
                a.totp_confirm(id, "alice", "123456"),
                Err(AuthError::BadInput(_))
            ),
            "nothing pending"
        );
        a.totp_begin(id, "alice").unwrap();
        assert!(matches!(
            a.totp_confirm(id, "alice", "000000"),
            Err(AuthError::BadInput(_))
        ));
        assert!(
            !a.totp_enabled(id).unwrap(),
            "a wrong code must not enable it"
        );
        // Until confirmed, logging in still needs only the password.
        assert!(a.login("alice", PW, None, "x", None, "c").is_ok());
        a.totp_confirm(id, "alice", &current_code(&a, "alice", 0))
            .unwrap();
        assert!(a.totp_enabled(id).unwrap());
        assert!(
            matches!(a.totp_begin(id, "alice"), Err(AuthError::BadInput(_))),
            "already on"
        );
    }

    #[test]
    fn login_needs_a_fresh_code_once_enabled() {
        let a = store();
        enrol(&a, "alice");
        assert_eq!(
            a.login("alice", PW, None, "x", None, "c"),
            Err(AuthError::TotpRequired)
        );
        assert_eq!(
            a.login("alice", PW, Some("000000"), "x", None, "c"),
            Err(AuthError::Invalid)
        );
        // The enrolment code was consumed; the next step's code is fresh.
        let code = current_code(&a, "alice", 1);
        assert!(a.login("alice", PW, Some(&code), "x", None, "c").is_ok());
        assert_eq!(
            a.login("alice", PW, Some(&code), "x", None, "c"),
            Err(AuthError::Invalid),
            "a code works once"
        );
    }

    #[test]
    fn totp_status_is_not_revealed_without_the_password() {
        let a = store();
        enrol(&a, "alice");
        // A wrong password must look the same whether or not two-step is on.
        assert_eq!(
            a.login("alice", "wrong password!", None, "x", None, "c"),
            Err(AuthError::Invalid)
        );
        assert_eq!(
            a.login("admin", "wrong password!", None, "x", None, "c"),
            Err(AuthError::Invalid)
        );
    }

    #[test]
    fn wrong_codes_count_towards_lockout() {
        let a = store();
        enrol(&a, "alice");
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                a.login("alice", PW, Some("000000"), "x", None, "c"),
                Err(AuthError::Invalid)
            );
        }
        let code = current_code(&a, "alice", 1);
        assert!(matches!(
            a.login("alice", PW, Some(&code), "x", None, "c2"),
            Err(AuthError::Locked(_))
        ));
    }

    #[test]
    fn turning_it_off_needs_password_and_code() {
        let a = store();
        enrol(&a, "alice");
        let id = user_id(&a, "alice");
        let code = current_code(&a, "alice", 1);
        assert_eq!(
            a.totp_disable(id, "alice", "wrong password!", &code),
            Err(AuthError::Invalid)
        );
        assert_eq!(
            a.totp_disable(id, "alice", PW, "000000"),
            Err(AuthError::Invalid)
        );
        assert!(a.totp_enabled(id).unwrap());
        a.totp_disable(id, "alice", PW, &code).unwrap();
        assert!(!a.totp_enabled(id).unwrap());
        assert!(a.login("alice", PW, None, "x", None, "c").is_ok());
        assert!(matches!(
            a.totp_disable(id, "alice", PW, &code),
            Err(AuthError::BadInput(_))
        ));
    }

    #[test]
    fn admin_reset_removes_two_step() {
        let a = store();
        enrol(&a, "alice");
        a.totp_reset("alice").unwrap();
        assert!(a.login("alice", PW, None, "x", None, "c").is_ok());
        assert!(matches!(
            a.totp_reset("nobody"),
            Err(AuthError::BadInput(_))
        ));
        let users = a.list_users().unwrap();
        assert!(users.iter().all(|u| !u.totp_enabled));
        let events: Vec<String> = a
            .audit_log(50)
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect();
        for e in ["totp.enabled", "totp.reset"] {
            assert!(events.contains(&e.to_string()), "{e}");
        }
    }

    #[test]
    fn migration_adds_totp_columns_to_an_existing_database() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT NOT NULL UNIQUE,
               password_hash TEXT NOT NULL, role TEXT NOT NULL,
               disabled INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);",
        )
        .unwrap();
        let a = Accounts::with_connection(db).unwrap();
        a.create_user("alice", PW, Role::Listener).unwrap();
        enrol(&a, "alice");
        let version: i64 =
            a.db.lock()
                .unwrap()
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
        assert_eq!(version, 3);
        // Version 3 added per-device diagnostics.
        let d = a.devices_for_test();
        a.save_diagnostics(d, r#"{"ok":true}"#).unwrap();
        assert_eq!(a.diagnostics(d).unwrap().unwrap().1, r#"{"ok":true}"#);
    }

    // -- player permissions --------------------------------------------------

    #[test]
    fn listeners_control_their_own_devices_and_granted_players_only() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let b = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        let scope = a.player_scope(&t.principal).unwrap();
        assert!(scope.allows(&t.principal.mac_string()));
        assert!(
            !scope.allows(&b.principal.mac_string()),
            "another user's device"
        );
        assert!(
            !scope.allows("aa:bb:cc:dd:ee:01"),
            "an ungranted home player"
        );

        a.grant_player("alice", "AA:BB:CC:DD:EE:01").unwrap();
        let scope = a.player_scope(&t.principal).unwrap();
        assert!(scope.allows("aa:bb:cc:dd:ee:01"));
        assert!(scope.allows("AA:BB:CC:DD:EE:01"));
        assert_eq!(
            a.players_granted("alice").unwrap(),
            vec!["aa:bb:cc:dd:ee:01"]
        );

        a.ungrant_player("alice", "aa:bb:cc:dd:ee:01").unwrap();
        assert!(
            !a.player_scope(&t.principal)
                .unwrap()
                .allows("aa:bb:cc:dd:ee:01")
        );
    }

    #[test]
    fn admins_control_everything() {
        let a = store();
        let b = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        assert!(
            a.player_scope(&b.principal)
                .unwrap()
                .allows("aa:bb:cc:dd:ee:99")
        );
    }

    #[test]
    fn revoked_devices_drop_out_of_scope() {
        let a = store();
        let t1 = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let t2 = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        a.revoke_device(t2.principal.device_id).unwrap();
        assert!(
            !a.player_scope(&t1.principal)
                .unwrap()
                .allows(&t2.principal.mac_string())
        );
    }

    #[test]
    fn grants_validate_input() {
        let a = store();
        for bad in [
            "",
            "nope",
            "aa:bb:cc",
            "aa:bb:cc:dd:ee:ff:00",
            "zz:bb:cc:dd:ee:ff",
        ] {
            assert!(
                matches!(a.grant_player("alice", bad), Err(AuthError::BadInput(_))),
                "{bad:?}"
            );
        }
        assert!(matches!(
            a.grant_player("nobody", "aa:bb:cc:dd:ee:01"),
            Err(AuthError::BadInput(_))
        ));
    }
}
