//! Account store: users, devices, tokens, pairing codes and the audit log (spec D12).
//!
//! - Passwords are argon2id (OWASP parameters: 19 MiB, 2 passes). An unknown username
//!   still costs a full hash, so response timing doesn't reveal which usernames exist.
//! - Access tokens (15 min) are random and held only in memory, hashed; revocation is
//!   immediate.
//! - Refresh tokens are random, stored hashed, and rotate on every use. Presenting an
//!   already-used refresh token means it was copied, so the whole device is revoked (with
//!   a short grace for lost responses and concurrent refreshes; see [`Accounts::refresh`]).
//! - Each device has its own player MAC (locally administered, `02:4c:41:…`).
//! - Failed logins and pairing attempts are rate-limited with escalating lockouts:
//!   - per client address, a hard lock;
//!   - per username (and for pairing, across all clients), a soft one: once it trips, the
//!     clients that failed are refused, and the others share a small budget of
//!     [`THROTTLED_ATTEMPTS`] a minute. So an attacker can't lock a user out from
//!     elsewhere, and spreading guesses over many addresses doesn't buy more of them.
//!
//!   An attempt is counted before its password is checked, so parallel guesses can't get
//!   past the limit, and at most [`HASH_PERMITS`] password checks run at once.
//! - Each user has at most [`MAX_DEVICES_PER_USER`] devices; signing in another revokes the
//!   least recently seen. At most [`NEW_DEVICES_PER_HOUR`] new devices a user an hour (a
//!   browser signing in again reuses its device). Revoked devices' LMS players are
//!   forgotten, and their rows pruned after [`REVOKED_DEVICE_RETENTION`].
//! - Optional two-step verification (TOTP, RFC 6238) per user: once enabled, a login needs a
//!   current code as well as the password. Codes are single use, and wrong codes count
//!   towards the same lockouts as wrong passwords.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use anywhere_core::proto::format_mac;

pub const ACCESS_TTL: Duration = Duration::from_secs(15 * 60);
/// Default sign-in lifetime: unused this long, a device must sign in again. Every use
/// renews it (refresh tokens rotate, each with a fresh lifetime). Admins can change it.
pub const REFRESH_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
/// Bounds for the configurable sign-in lifetime, in days.
pub const SESSION_DAYS: std::ops::RangeInclusive<u32> = 1..=365;
/// At most this many plugins can be marked admin-only.
pub const MAX_ADMIN_ONLY_PLUGINS: usize = 200;

/// A plugin's command word, as LMS menus name it (e.g. `audiomuseai`, `selectVirtualLibrary`).
pub fn valid_plugin_command(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
/// A refresh token presented again this soon after it was used is a lost response (a phone
/// suspended mid-request), not theft, provided its replacement hasn't been used either.
pub const REUSE_GRACE_SECS: i64 = 60;
pub const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
pub const MIN_PASSWORD: usize = 10;
pub const MAX_PASSWORD: usize = 1024;
/// Password checks (argon2id, 19 MiB each) that may run at once. Others wait up to
/// [`HASH_WAIT`], then are refused as busy.
pub const HASH_PERMITS: usize = 4;
pub const HASH_WAIT: Duration = Duration::from_secs(2);
/// Active devices per user. Signing in one more revokes the least recently seen.
pub const MAX_DEVICES_PER_USER: usize = 20;
/// Rate-limit entries kept (per username and per client address). Expired ones go first,
/// then the oldest unlocked ones.
pub const LIMITS_CAP: usize = 10_000;
/// While a username (or pairing) is throttled, attempts allowed per [`THROTTLE_WINDOW`] from
/// clients that haven't failed for it, across all of them.
pub const THROTTLED_ATTEMPTS: u32 = 2;
pub const THROTTLE_WINDOW: Duration = Duration::from_secs(60);
/// Failing clients remembered per throttled username.
const FAILED_CLIENTS_CAP: usize = 1000;
/// New devices (sign-ins without a reusable device, pairings) per user per hour.
pub const NEW_DEVICES_PER_HOUR: i64 = 10;
/// Revoked devices are kept this long (for the audit trail), then deleted.
pub const REVOKED_DEVICE_RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);
/// Siblings one refresh token may have (lost responses and concurrent refreshes).
const MAX_REFRESH_RETRIES: i64 = 8;
/// Revoked devices' players waiting to be forgotten by LMS (beyond this, oldest dropped).
const FORGET_QUEUE_CAP: usize = 1000;
/// A device name that has nothing usable left after cleaning.
pub const DEFAULT_DEVICE_NAME: &str = "Browser";
pub const MAX_DEVICE_NAME: usize = 64;

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
    /// Seconds until the refresh token expires (the sign-in lifetime).
    pub refresh_expires_in: u64,
    pub principal: Principal,
    /// Devices revoked to make room for this one (see [`MAX_DEVICES_PER_USER`]); their live
    /// sessions should be ended.
    pub evicted: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: &'static str,
    pub disabled: bool,
    pub totp_enabled: bool,
    /// May this user download tracks for offline use?
    pub downloads: bool,
    /// The download formats this user may choose (see [`DOWNLOAD_FORMATS`]).
    pub download_formats: Vec<String>,
}

/// Download formats an admin can allow: the original file, or a transcode.
pub const DOWNLOAD_FORMATS: [&str; 3] = ["original", "mp3", "opus"];

/// A user's download permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub enabled: bool,
    /// Allowed formats, in [`DOWNLOAD_FORMATS`] order.
    pub formats: Vec<&'static str>,
}

impl DownloadPolicy {
    pub fn allows(&self, format: &str) -> bool {
        self.enabled && self.formats.contains(&format)
    }
}

/// Stored formats (comma-separated) as a clean list: unknown words are dropped.
fn parse_formats(stored: &str) -> Vec<&'static str> {
    let words: Vec<&str> = stored.split(',').map(str::trim).collect();
    DOWNLOAD_FORMATS
        .into_iter()
        .filter(|f| words.contains(f))
        .collect()
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

/// What [`Accounts::claim_play`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayClaim {
    /// The key is ours to forward.
    Claimed,
    /// LMS already recorded a play with this key.
    Recorded,
    /// Another request is forwarding it right now.
    InFlight,
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
    /// Too many password checks running at once; retry shortly.
    Busy,
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
    /// The refresh token minted with this access token: if that token is superseded (see
    /// [`Accounts::refresh`]), this access token dies with it.
    refresh: [u8; 32],
}

struct Lock {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
    lockouts: u32,
    /// Attempts being checked right now: they count towards the limit until they finish.
    in_flight: u32,
    /// Soft keys (per username, pairing): the clients that failed while counting. They are
    /// refused while the key is throttled; other clients share [`THROTTLED_ATTEMPTS`].
    failed_clients: HashSet<String>,
    budget_start: Instant,
    budget_used: u32,
}

impl Lock {
    fn new(now: Instant) -> Lock {
        Lock {
            failures: 0,
            window_start: now,
            locked_until: None,
            lockouts: 0,
            in_flight: 0,
            failed_clients: HashSet::new(),
            budget_start: now,
            budget_used: 0,
        }
    }

    fn roll_window(&mut self, now: Instant) {
        if now.duration_since(self.window_start) > FAILURE_WINDOW {
            self.failures = 0;
            self.window_start = now;
            if !self.locked(now) {
                self.failed_clients.clear();
            }
        }
    }

    fn locked(&self, now: Instant) -> bool {
        self.locked_until.is_some_and(|u| u > now)
    }

    /// Too many failures (or attempts in flight) to let just anyone try.
    fn throttled(&self, now: Instant) -> bool {
        self.locked(now) || self.failures + self.in_flight >= MAX_FAILURES
    }

    /// How long until this key stops refusing, for `Retry-After`.
    fn retry_after(&self, now: Instant) -> Duration {
        self.locked_until
            .filter(|u| *u > now)
            .map_or(Duration::from_secs(1), |u| u - now)
    }

    /// Worth keeping: something is pending, locked, or still counting.
    fn live(&self, now: Instant) -> bool {
        self.in_flight > 0
            || self.locked(now)
            || (self.failures > 0 && now.duration_since(self.window_start) <= FAILURE_WINDOW)
    }
}

/// A counting semaphore for password checks (they run on blocking threads).
struct HashGate {
    busy: Mutex<usize>,
    freed: Condvar,
    wait: Duration,
}

struct HashPermit<'a>(&'a HashGate);

impl HashGate {
    fn acquire(&self) -> Option<HashPermit<'_>> {
        let deadline = Instant::now() + self.wait;
        let mut busy = self.busy.lock().unwrap();
        while *busy >= HASH_PERMITS {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            busy = self.freed.wait_timeout(busy, deadline - now).unwrap().0;
        }
        *busy += 1;
        Some(HashPermit(self))
    }
}

impl Drop for HashPermit<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock().unwrap() -= 1;
        self.0.freed.notify_one();
    }
}

/// A login (or similar) attempt, counted against its rate-limit keys from the start. It
/// must end as [`Attempt::failed`] or [`Attempt::succeeded`]; dropped otherwise (bad input,
/// busy, storage error), it counts as neither.
struct Attempt<'a> {
    accounts: &'a Accounts,
    /// Hard keys: refused outright once locked.
    keys: Vec<String>,
    /// A soft key and the client making the attempt (see [`Accounts::begin_attempt`]).
    soft: Option<(String, String)>,
    done: bool,
}

impl Attempt<'_> {
    fn all_keys(&self) -> impl Iterator<Item = &String> {
        self.keys.iter().chain(self.soft.as_ref().map(|(k, _)| k))
    }

    fn release(&self, limits: &mut HashMap<String, Lock>) {
        for k in self.all_keys() {
            if let Some(l) = limits.get_mut(k) {
                l.in_flight = l.in_flight.saturating_sub(1);
            }
        }
    }

    fn failed(mut self) {
        self.done = true;
        let mut limits = self.accounts.limits.lock().unwrap();
        let now = Instant::now();
        self.release(&mut limits);
        for k in self.all_keys() {
            record_failure_in(&mut limits, k, now);
        }
        if let Some((k, client)) = &self.soft
            && let Some(l) = limits.get_mut(k)
            && l.failed_clients.len() < FAILED_CLIENTS_CAP
        {
            l.failed_clients.insert(client.clone());
        }
    }

    /// Clears the client's own record. A soft key stays locked if it is (the attacker's
    /// failures still count), but this client is no longer one that failed.
    fn succeeded(mut self) {
        self.done = true;
        let mut limits = self.accounts.limits.lock().unwrap();
        self.release(&mut limits);
        for k in &self.keys {
            limits.remove(k);
        }
        if let Some((k, client)) = &self.soft
            && let Some(l) = limits.get_mut(k)
        {
            l.failed_clients.remove(client);
            if !l.locked(Instant::now()) {
                l.failures = 0;
            }
        }
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let mut limits = self.accounts.limits.lock().unwrap();
        self.release(&mut limits);
    }
}

/// Make room for a new rate-limit entry: drop dead entries, then the oldest unlocked ones.
fn make_room(limits: &mut HashMap<String, Lock>, now: Instant) {
    if limits.len() < LIMITS_CAP {
        return;
    }
    limits.retain(|_, l| l.live(now));
    if limits.len() < LIMITS_CAP {
        return;
    }
    // Still full: evict down to 90%, oldest first, sparing locks and pending attempts as long
    // as anything else is left.
    let mut order: Vec<(bool, Instant, String)> = limits
        .iter()
        .filter(|(_, l)| l.in_flight == 0)
        .map(|(k, l)| (l.locked(now), l.window_start, k.clone()))
        .collect();
    order.sort();
    let excess = limits.len() + 1 - LIMITS_CAP * 9 / 10;
    for (_, _, k) in order.into_iter().take(excess) {
        limits.remove(&k);
    }
}

fn record_failure_in(limits: &mut HashMap<String, Lock>, key: &str, now: Instant) {
    if !limits.contains_key(key) {
        make_room(limits, now);
    }
    let l = limits
        .entry(key.to_string())
        .or_insert_with(|| Lock::new(now));
    l.roll_window(now);
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

/// Failures allowed per key within the window before a lockout.
const MAX_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const FIRST_LOCKOUT: Duration = Duration::from_secs(15 * 60);
const MAX_LOCKOUT: Duration = Duration::from_secs(24 * 3600);

pub struct Accounts {
    db: Mutex<Connection>,
    access: Mutex<HashMap<[u8; 32], AccessEntry>>,
    limits: Mutex<HashMap<String, Lock>>,
    hash_gate: HashGate,
    /// Password checks performed (for tests and diagnostics).
    password_checks: AtomicU64,
    /// A real argon2id hash, verified against for unknown usernames.
    dummy_hash: String,
    /// Player MACs of revoked devices, for the gateway to have LMS forget.
    forget: Mutex<std::collections::VecDeque<String>>,
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

/// A device (player) name safe to show anywhere and hand to LMS: letters of any script,
/// digits, spaces and `. , - _ ' ’ ( ) &`, with combining accents after a letter. Anything
/// else (markup, controls, bidi and other format characters such as U+202E or U+200B,
/// symbols) is removed, runs of whitespace become one space, and the result is cut to
/// [`MAX_DEVICE_NAME`] characters. Nothing left: [`DEFAULT_DEVICE_NAME`].
pub fn clean_device_name(raw: &str) -> String {
    clean_name(raw).unwrap_or_else(|| DEFAULT_DEVICE_NAME.to_string())
}

/// [`clean_device_name`] without the default: `None` if nothing usable is left.
pub fn clean_name(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut count = 0;
    let mut space = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            space = count > 0;
            continue;
        }
        let combining = ('\u{300}'..='\u{36f}').contains(&c);
        let ok = c.is_alphanumeric() || ".,-_'’()&".contains(c) || (combining && count > 0);
        if !ok {
            continue;
        }
        let need = if space && !combining { 2 } else { 1 };
        if count + need > MAX_DEVICE_NAME {
            break;
        }
        if space && !combining {
            out.push(' ');
            count += 1;
        }
        space = false;
        out.push(c);
        count += 1;
    }
    (!out.is_empty()).then_some(out)
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
            hash_gate: HashGate {
                busy: Mutex::new(0),
                freed: Condvar::new(),
                wait: HASH_WAIT,
            },
            password_checks: AtomicU64::new(0),
            dummy_hash: hash_password(&new_secret())?,
            forget: Mutex::new(Default::default()),
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

    /// Set a user's password (admin). Their devices are revoked: whoever knew the old
    /// password may have signed in with it. Returns the revoked device ids.
    pub fn set_password(&self, username: &str, password: &str) -> Result<Vec<i64>, AuthError> {
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
        self.revoke_user_devices(username)
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
            "SELECT id, username, role, disabled, totp_enabled, downloads, download_formats
             FROM users ORDER BY username",
        )?;
        let rows = st.query_map([], |r| {
            let role: String = r.get(2)?;
            Ok(UserInfo {
                id: r.get(0)?,
                username: r.get(1)?,
                role: Role::parse(&role).unwrap_or(Role::Listener).as_str(),
                disabled: r.get::<_, i64>(3)? != 0,
                totp_enabled: r.get::<_, i64>(4)? != 0,
                downloads: r.get::<_, i64>(5)? != 0,
                download_formats: parse_formats(&r.get::<_, String>(6)?)
                    .into_iter()
                    .map(String::from)
                    .collect(),
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
    /// created (its name cleaned by [`clean_device_name`]). Rate-limited per `client` (e.g.
    /// IP address, a hard lock) and per username (a soft one: see the module notes).
    pub fn login(
        &self,
        username: &str,
        password: &str,
        totp: Option<&str>,
        device_name: &str,
        device_id: Option<i64>,
        client: &str,
    ) -> Result<Tokens, AuthError> {
        // A name that can't exist gets no rate-limit entry of its own (it would only grow
        // the table), but still counts against the client.
        let known_form = valid_username(username);
        let client_key = format!("client:{client}");
        let soft = known_form.then(|| (format!("user:{username}"), client_key.clone()));
        let attempt = self.begin_attempt(vec![client_key], soft)?;
        if !known_form {
            attempt.failed();
            self.audit("login.failed", None, None, client);
            return Err(AuthError::Invalid);
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
        let ok = self.check_password_hash(password, hash)?;
        let user = match row {
            Some((id, _, role, disabled)) if ok && disabled == 0 => {
                (id, Role::parse(&role).unwrap_or(Role::Listener))
            }
            _ => {
                attempt.failed();
                self.audit("login.failed", Some(username), None, client);
                return Err(AuthError::Invalid);
            }
        };
        match self.second_factor(user.0, totp)? {
            SecondFactor::Off | SecondFactor::Accepted => {}
            SecondFactor::Missing => return Err(AuthError::TotpRequired),
            SecondFactor::Wrong => {
                attempt.failed();
                self.audit("login.totp_failed", Some(username), None, client);
                return Err(AuthError::Invalid);
            }
        }
        attempt.succeeded();
        let (device, evicted) = match device_id.map(|id| self.own_device(user.0, id)) {
            Some(Err(e)) => return Err(e),
            Some(Ok(Some(d))) => (d, vec![]),
            _ => self.create_device(user.0, device_name)?,
        };
        self.audit("login.ok", Some(username), Some(device), client);
        let mut t = self.issue(device)?;
        t.evicted = evicted;
        Ok(t)
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
    /// Pairing is rate-limited per client (hard) and across all clients (soft), since codes
    /// aren't tied to a username an attacker would have to name.
    pub fn pair(&self, code: &str, device_name: &str, client: &str) -> Result<Tokens, AuthError> {
        let client_key = format!("client:{client}");
        let soft = Some(("pairing".to_string(), client_key.clone()));
        let attempt = self.begin_attempt(vec![client_key], soft)?;
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
                attempt.failed();
                self.audit("pairing.failed", None, None, client);
                return Err(AuthError::Invalid);
            }
        };
        attempt.succeeded();
        let disabled: i64 = self.db.lock().unwrap().query_row(
            "SELECT disabled FROM users WHERE id = ?1",
            [user],
            |r| r.get(0),
        )?;
        if disabled != 0 {
            return Err(AuthError::Invalid);
        }
        let (device, evicted) = self.create_device(user, device_name)?;
        self.audit("pairing.ok", None, Some(device), client);
        let mut t = self.issue(device)?;
        t.evicted = evicted;
        Ok(t)
    }

    /// Rotate a refresh token. Every token is used once; each replacement records the token
    /// it replaced (its parent).
    ///
    /// - An unused token is marked used and replaced. Its *siblings* (other replacements of
    ///   the same parent, from the grace rule below) are superseded: marked used, with no
    ///   replacement of their own, and their access tokens dropped. Presenting one later is
    ///   theft.
    /// - A used token presented again within [`REUSE_GRACE_SECS`] of its use, while none of
    ///   its replacements has been used: a lost response (a phone suspended mid-request) or
    ///   a concurrent refresh (two tabs). Another replacement is issued *alongside* the
    ///   first, which stays valid, so whichever one the client ends up keeping works.
    /// - Any other used token (too late, or a replacement already used, or superseded): two
    ///   parties hold tokens for this device, so the device is revoked.
    ///
    /// The trade-off: a thief replaying a copied token within the grace window also gets a
    /// valid replacement. The first of the two to be used supersedes the other, and the
    /// loser presenting theirs revokes the device for both (see docs/PROTOCOL.md).
    pub fn refresh(&self, refresh_token: &str) -> Result<Tokens, AuthError> {
        self.refresh_at(refresh_token, now_secs())
    }

    /// One transaction under one lock: look up, mark used, supersede siblings, mint the
    /// replacement and record its parent. A concurrent refresh of the same token therefore
    /// always sees either an unused token or a used one with its replacement recorded.
    fn refresh_at(&self, refresh_token: &str, now: i64) -> Result<Tokens, AuthError> {
        let h = sha256(refresh_token).to_vec();
        type Row = (i64, i64, i64, Option<i64>, Option<Vec<u8>>);
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let row: Option<Row> = tx
            .query_row(
                "SELECT device_id, expires_at, used, used_at, parent
                 FROM refresh_tokens WHERE token_hash = ?1",
                [&h],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((device, expires, used, used_at, parent)) = row else {
            return Err(AuthError::Invalid);
        };
        if used != 0 {
            let (children, children_used): (i64, i64) = tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(used), 0) FROM refresh_tokens WHERE parent = ?1",
                [&h],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let in_grace = used_at.is_some_and(|at| (0..=REUSE_GRACE_SECS).contains(&(now - at)));
            if in_grace && children > 0 && children_used == 0 {
                if children >= MAX_REFRESH_RETRIES {
                    return Err(AuthError::Invalid);
                }
                let minted = mint(&tx, device, now)?;
                tx.execute(
                    "UPDATE refresh_tokens SET parent = ?1 WHERE token_hash = ?2",
                    params![h, sha256(&minted.refresh).to_vec()],
                )?;
                tx.commit()?;
                let t = self.activate(minted);
                drop(db);
                self.audit(
                    "refresh.retried",
                    None,
                    Some(device),
                    "lost response or concurrent refresh",
                );
                return Ok(t);
            }
            drop(tx);
            drop(db);
            self.revoke_device(device)?;
            self.audit("refresh.reused", None, Some(device), "device revoked");
            return Err(AuthError::Reused(device));
        }
        if expires <= now {
            return Err(AuthError::Invalid);
        }
        let n = tx.execute(
            "UPDATE refresh_tokens SET used = 1, used_at = ?2 WHERE token_hash = ?1 AND used = 0",
            params![h, now],
        )?;
        if n == 0 {
            return Err(AuthError::Invalid);
        }
        // This replacement won: its unused siblings can only be in another party's hands now.
        let mut superseded: Vec<Vec<u8>> = vec![];
        if let Some(p) = &parent {
            let mut st = tx.prepare(
                "SELECT token_hash FROM refresh_tokens
                 WHERE parent = ?1 AND token_hash != ?2 AND used = 0",
            )?;
            superseded = st
                .query_map(params![p, h], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            drop(st);
            tx.execute(
                "UPDATE refresh_tokens SET used = 1, used_at = ?3
                 WHERE parent = ?1 AND token_hash != ?2 AND used = 0",
                params![p, h, now],
            )?;
        }
        let minted = mint(&tx, device, now)?;
        tx.execute(
            "UPDATE refresh_tokens SET parent = ?1 WHERE token_hash = ?2",
            params![h, sha256(&minted.refresh).to_vec()],
        )?;
        tx.commit()?;
        if !superseded.is_empty() {
            self.access
                .lock()
                .unwrap()
                .retain(|_, e| !superseded.iter().any(|s| s[..] == e.refresh[..]));
        }
        // Still under the database lock, so a revocation can't slip in between.
        Ok(self.activate(minted))
    }

    /// The sign-in lifetime in days (how long an unused device stays signed in).
    pub fn session_days(&self) -> u32 {
        session_days_on(&self.db.lock().unwrap())
    }

    /// Change the sign-in lifetime. It applies from each device's next sign-in or renewal.
    pub fn set_session_days(&self, days: u32) -> Result<(), AuthError> {
        if !SESSION_DAYS.contains(&days) {
            return Err(AuthError::BadInput("session lifetime must be 1–365 days"));
        }
        self.db.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES ('session_days', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [days.to_string()],
        )?;
        self.audit("settings.session_days", None, None, &days.to_string());
        Ok(())
    }

    /// Plugin commands (e.g. `audiomuseai`) only admins may use. Listeners neither see nor
    /// run them. A stored value that doesn't parse counts as none.
    pub fn admin_only_plugins(&self) -> HashSet<String> {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM settings WHERE key = 'admin_only_plugins'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
            .map(|v| v.into_iter().filter(|c| valid_plugin_command(c)).collect())
            .unwrap_or_default()
    }

    /// Replace the set of admin-only plugin commands.
    pub fn set_admin_only_plugins(&self, commands: &[String]) -> Result<(), AuthError> {
        if commands.len() > MAX_ADMIN_ONLY_PLUGINS
            || !commands.iter().all(|c| valid_plugin_command(c))
        {
            return Err(AuthError::BadInput(
                "plugin commands are 1–64 of A-Z a-z 0-9 _ - (at most 200)",
            ));
        }
        let mut list: Vec<&String> = commands.iter().collect();
        list.sort();
        list.dedup();
        let value = serde_json::to_string(&list).map_err(|e| AuthError::Storage(e.to_string()))?;
        self.db.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES ('admin_only_plugins', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&value],
        )?;
        let joined = list
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",");
        self.audit("settings.admin_only_plugins", None, None, &joined);
        Ok(())
    }

    /// A user's download permission, read afresh each time so an admin's change applies to
    /// the next request. A disabled or unknown user may download nothing.
    pub fn download_policy(&self, user_id: i64) -> Result<DownloadPolicy, AuthError> {
        let row: Option<(i64, String)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT downloads, download_formats FROM users WHERE id = ?1 AND disabled = 0",
                [user_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((on, formats)) => DownloadPolicy {
                enabled: on != 0,
                formats: parse_formats(&formats),
            },
            None => DownloadPolicy {
                enabled: false,
                formats: vec![],
            },
        })
    }

    /// Turn downloads on or off for a user, and optionally set the formats they may use
    /// (one or more of [`DOWNLOAD_FORMATS`]).
    pub fn set_downloads(
        &self,
        username: &str,
        enabled: bool,
        formats: Option<&[String]>,
    ) -> Result<(), AuthError> {
        let formats = match formats {
            Some(list) => {
                if list.is_empty()
                    || list.len() > DOWNLOAD_FORMATS.len()
                    || !list.iter().all(|f| DOWNLOAD_FORMATS.contains(&f.as_str()))
                {
                    return Err(AuthError::BadInput(
                        "formats must be one or more of original, mp3, opus",
                    ));
                }
                let set: Vec<&str> = DOWNLOAD_FORMATS
                    .into_iter()
                    .filter(|f| list.iter().any(|x| x == f))
                    .collect();
                Some(set.join(","))
            }
            None => None,
        };
        let n = {
            let db = self.db.lock().unwrap();
            match &formats {
                Some(f) => db.execute(
                    "UPDATE users SET downloads = ?1, download_formats = ?2 WHERE username = ?3",
                    params![enabled as i64, f, username],
                )?,
                None => db.execute(
                    "UPDATE users SET downloads = ?1 WHERE username = ?2",
                    params![enabled as i64, username],
                )?,
            }
        };
        if n == 0 {
            return Err(AuthError::BadInput("no such user"));
        }
        let state = if enabled { "on" } else { "off" };
        let detail = match &formats {
            Some(f) => format!("{state} {f}"),
            None => state.to_string(),
        };
        self.audit("user.downloads", Some(username), None, &detail);
        Ok(())
    }

    /// A random secret, made once and kept with the accounts, that salts stable download
    /// keys (so a key never reveals the file path it was derived from).
    pub fn download_key_salt(&self) -> Result<String, AuthError> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('download_key_salt', ?1)",
            [new_secret()],
        )?;
        Ok(db.query_row(
            "SELECT value FROM settings WHERE key = 'download_key_salt'",
            [],
            |r| r.get(0),
        )?)
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
            // Atomic: of two requests with the same code, only one moves the step on.
            Some(step) => Ok(self.db.lock().unwrap().execute(
                "UPDATE users SET totp_last_step = ?1 WHERE id = ?2 AND totp_last_step < ?1",
                params![step as i64, user],
            )? == 1),
            None => Ok(false),
        }
    }

    fn second_factor(&self, user: i64, code: Option<&str>) -> Result<SecondFactor, AuthError> {
        let (secret, enabled, last_step) = self.totp_state(user)?;
        let Some(secret) = secret.filter(|_| enabled) else {
            return Ok(SecondFactor::Off);
        };
        let Some(code) = code else {
            return Ok(SecondFactor::Missing);
        };
        Ok(if self.consume_code(user, &secret, code, last_step)? {
            SecondFactor::Accepted
        } else {
            SecondFactor::Wrong
        })
    }

    /// Check a signed-in user's own password (for account changes), rate-limited per user
    /// with a lock of its own: only the user's own devices can trip it, and sign-in failures
    /// (which anyone can cause) don't block it.
    fn confirm_password(
        &self,
        user: i64,
        username: &str,
        password: &str,
    ) -> Result<Attempt<'_>, AuthError> {
        let attempt = self.begin_attempt(vec![format!("confirm:{username}")], None)?;
        let hash: String = self.db.lock().unwrap().query_row(
            "SELECT password_hash FROM users WHERE id = ?1",
            [user],
            |r| r.get(0),
        )?;
        if self.check_password_hash(password, &hash)? {
            Ok(attempt)
        } else {
            attempt.failed();
            Err(AuthError::Invalid)
        }
    }

    /// Start enrolment: a new secret for the user's authenticator app. Not active until
    /// confirmed with a code from the app.
    /// Needs the account password, so a stolen access token alone can't add a second factor
    /// the owner doesn't have.
    pub fn totp_begin(
        &self,
        user: i64,
        username: &str,
        password: &str,
    ) -> Result<TotpSetup, AuthError> {
        match self.confirm_password(user, username, password) {
            Ok(attempt) => drop(attempt),
            Err(e) => {
                if e == AuthError::Invalid {
                    self.audit("totp.setup_failed", Some(username), None, "");
                }
                return Err(e);
            }
        }
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
        let attempt = match self.confirm_password(user, username, password) {
            Ok(a) => a,
            Err(e) => {
                if e == AuthError::Invalid {
                    self.audit("totp.disable_failed", Some(username), None, "");
                }
                return Err(e);
            }
        };
        let (secret, enabled, last_step) = self.totp_state(user)?;
        let code_ok = match (secret, enabled) {
            (Some(s), true) => self.consume_code(user, &s, code, last_step)?,
            _ => return Err(AuthError::BadInput("two-step verification is not on")),
        };
        if !code_ok {
            attempt.failed();
            self.audit("totp.disable_failed", Some(username), None, "");
            return Err(AuthError::Invalid);
        }
        drop(attempt);
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

    // -----------------------------------------------------------------------
    // Offline plays (dedupe keys; see crate::plays)
    // -----------------------------------------------------------------------

    /// Claim a play's idempotency key for forwarding to LMS. A pending claim older than
    /// `stale` (a crash mid-forward) is taken over.
    pub fn claim_play(
        &self,
        user: i64,
        device: i64,
        key: &str,
        track_id: u64,
        played_at: u64,
        stale: Duration,
    ) -> Result<PlayClaim, AuthError> {
        let now = now_secs();
        let db = self.db.lock().unwrap();
        let n = db.execute(
            "INSERT OR IGNORE INTO plays (user_id, key, device_id, track_id, played_at, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
            params![user, key, device, track_id as i64, played_at as i64, now],
        )?;
        if n == 1 {
            return Ok(PlayClaim::Claimed);
        }
        let (state, created): (i64, i64) = db.query_row(
            "SELECT state, created_at FROM plays WHERE user_id = ?1 AND key = ?2",
            params![user, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if state != 0 {
            return Ok(PlayClaim::Recorded);
        }
        if now.saturating_sub(created) < stale.as_secs() as i64 {
            return Ok(PlayClaim::InFlight);
        }
        let n = db.execute(
            "UPDATE plays SET device_id = ?3, track_id = ?4, played_at = ?5, created_at = ?6
             WHERE user_id = ?1 AND key = ?2 AND state = 0 AND created_at = ?7",
            params![
                user,
                key,
                device,
                track_id as i64,
                played_at as i64,
                now,
                created
            ],
        )?;
        Ok(if n == 1 {
            PlayClaim::Claimed
        } else {
            PlayClaim::InFlight
        })
    }

    /// LMS recorded the play: its key now marks it as done.
    pub fn finish_play(&self, user: i64, key: &str) -> Result<(), AuthError> {
        self.db.lock().unwrap().execute(
            "UPDATE plays SET state = 1 WHERE user_id = ?1 AND key = ?2",
            params![user, key],
        )?;
        Ok(())
    }

    /// The play was not recorded (refused, or to be retried): give its key back.
    pub fn release_play(&self, user: i64, key: &str) -> Result<(), AuthError> {
        self.db.lock().unwrap().execute(
            "DELETE FROM plays WHERE user_id = ?1 AND key = ?2 AND state = 0",
            params![user, key],
        )?;
        Ok(())
    }

    /// Forget keys of plays from before `oldest` (such plays are refused anyway, so their
    /// keys can't matter), then keep at most `cap` recorded keys for `user`, the newest.
    pub fn prune_plays(&self, user: Option<i64>, oldest: u64, cap: usize) -> Result<(), AuthError> {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM plays WHERE played_at < ?1", [oldest as i64])?;
        if let Some(user) = user {
            db.execute(
                "DELETE FROM plays WHERE user_id = ?1 AND state = 1 AND rowid NOT IN (
                   SELECT rowid FROM plays WHERE user_id = ?1
                   ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
                params![user, cap as i64],
            )?;
        }
        Ok(())
    }

    /// How many play keys are kept for this user (recorded or being forwarded).
    pub fn stored_plays(&self, user: i64) -> Result<usize, AuthError> {
        let n: i64 = self.db.lock().unwrap().query_row(
            "SELECT count(*) FROM plays WHERE user_id = ?1",
            [user],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// A user's id, by name.
    pub fn user_id_by_name(&self, username: &str) -> Result<Option<i64>, AuthError> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                [username],
                |r| r.get(0),
            )
            .optional()?)
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
    /// Its player MAC is queued for LMS to forget (see [`Accounts::take_players_to_forget`]).
    pub fn revoke_device(&self, device_id: i64) -> Result<bool, AuthError> {
        let n = {
            let db = self.db.lock().unwrap();
            let n = db.execute(
                "UPDATE devices SET revoked = 1, revoked_at = ?2 WHERE id = ?1 AND revoked = 0",
                params![device_id, now_secs()],
            )?;
            if n > 0 {
                let mac: String =
                    db.query_row("SELECT mac FROM devices WHERE id = ?1", [device_id], |r| {
                        r.get(0)
                    })?;
                let mut q = self.forget.lock().unwrap();
                if q.len() >= FORGET_QUEUE_CAP {
                    q.pop_front();
                }
                q.push_back(mac);
            }
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

    /// Create a device for a user, first revoking the least recently seen ones beyond
    /// [`MAX_DEVICES_PER_USER`]. Returns the new id and the revoked ones. Refused (as
    /// `Locked`) beyond [`NEW_DEVICES_PER_HOUR`]: each device becomes an LMS player, so
    /// signing in over and over must not be able to flood LMS with them.
    fn create_device(&self, user: i64, name: &str) -> Result<(i64, Vec<i64>), AuthError> {
        let now = now_secs();
        let active: Vec<i64> = {
            let db = self.db.lock().unwrap();
            // Revoked devices are kept a while for the audit trail, then deleted.
            db.execute(
                "DELETE FROM devices WHERE revoked = 1 AND revoked_at < ?1",
                [now - REVOKED_DEVICE_RETENTION.as_secs() as i64],
            )?;
            let (recent, oldest): (i64, Option<i64>) = db.query_row(
                "SELECT COUNT(*), MIN(created_at) FROM devices
                 WHERE user_id = ?1 AND created_at > ?2",
                params![user, now - 3600],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if recent >= NEW_DEVICES_PER_HOUR {
                drop(db);
                let wait = oldest.map_or(3600, |t| (t + 3600 - now).clamp(1, 3600));
                self.audit("device.rate_limited", None, None, &format!("user {user}"));
                return Err(AuthError::Locked(Duration::from_secs(wait as u64)));
            }
            let mut st = db.prepare(
                "SELECT id FROM devices WHERE user_id = ?1 AND revoked = 0
                 ORDER BY COALESCE(last_seen, created_at), id",
            )?;
            st.query_map([user], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        let excess = (active.len() + 1).saturating_sub(MAX_DEVICES_PER_USER);
        let evicted: Vec<i64> = active.into_iter().take(excess).collect();
        for id in &evicted {
            self.revoke_device(*id)?;
            self.audit("device.evicted", None, Some(*id), "device limit reached");
        }
        Ok((self.insert_device(user, name)?, evicted))
    }

    fn insert_device(&self, user: i64, name: &str) -> Result<i64, AuthError> {
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
                    params![user, clean_device_name(name), mac, now_secs()],
                )?;
                return Ok(db.last_insert_rowid());
            }
        }
        Err(AuthError::Storage("could not allocate a player MAC".into()))
    }

    /// Issue a fresh access + refresh pair for a device.
    fn issue(&self, device: i64) -> Result<Tokens, AuthError> {
        let db = self.db.lock().unwrap();
        let minted = mint(&db, device, now_secs())?;
        Ok(self.activate(minted))
    }

    /// Make minted tokens live: register the access token in memory.
    fn activate(&self, m: Minted) -> Tokens {
        let mut map = self.access.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, e| e.expires > now);
        map.insert(
            sha256(&m.access),
            AccessEntry {
                principal: m.principal.clone(),
                expires: now + ACCESS_TTL,
                refresh: sha256(&m.refresh),
            },
        );
        Tokens {
            access: m.access,
            access_expires_in: ACCESS_TTL.as_secs(),
            refresh: m.refresh,
            refresh_expires_in: m.refresh_ttl,
            principal: m.principal,
            evicted: vec![],
        }
    }

    // -----------------------------------------------------------------------
    // Rate limiting and audit
    // -----------------------------------------------------------------------

    /// Start an attempt. Each hard key refuses it once locked, or once its failures plus the
    /// attempts already in flight reach the limit (so parallel guesses can't all be
    /// checked). The soft key `(key, client)`, once throttled the same way, refuses only
    /// clients that failed for it; other clients share [`THROTTLED_ATTEMPTS`] per
    /// [`THROTTLE_WINDOW`], so someone else's failures can't lock a user out, and many
    /// addresses don't buy many guesses.
    fn begin_attempt(
        &self,
        keys: Vec<String>,
        soft: Option<(String, String)>,
    ) -> Result<Attempt<'_>, AuthError> {
        let mut limits = self.limits.lock().unwrap();
        let now = Instant::now();
        for k in &keys {
            if let Some(l) = limits.get_mut(k) {
                l.roll_window(now);
                if l.throttled(now) {
                    return Err(AuthError::Locked(l.retry_after(now)));
                }
            }
        }
        if let Some((k, client)) = &soft
            && let Some(l) = limits.get_mut(k)
        {
            l.roll_window(now);
            if l.throttled(now) {
                if l.failed_clients.contains(client) {
                    return Err(AuthError::Locked(l.retry_after(now)));
                }
                if now.duration_since(l.budget_start) >= THROTTLE_WINDOW {
                    l.budget_start = now;
                    l.budget_used = 0;
                }
                if l.budget_used >= THROTTLED_ATTEMPTS {
                    let wait = (l.budget_start + THROTTLE_WINDOW).saturating_duration_since(now);
                    return Err(AuthError::Locked(wait.max(Duration::from_secs(1))));
                }
                l.budget_used += 1;
            }
        }
        for k in keys.iter().chain(soft.as_ref().map(|(k, _)| k)) {
            if !limits.contains_key(k) {
                make_room(&mut limits, now);
            }
            limits
                .entry(k.clone())
                .or_insert_with(|| Lock::new(now))
                .in_flight += 1;
        }
        Ok(Attempt {
            accounts: self,
            keys,
            soft,
            done: false,
        })
    }

    /// Player MACs of devices revoked since the last call, for LMS to forget.
    pub fn take_players_to_forget(&self) -> Vec<String> {
        self.forget.lock().unwrap().drain(..).collect()
    }

    /// Verify a password against a stored hash, within the concurrency limit.
    fn check_password_hash(&self, password: &str, hash: &str) -> Result<bool, AuthError> {
        if password.len() > MAX_PASSWORD {
            return Ok(false);
        }
        let _permit = self.hash_gate.acquire().ok_or(AuthError::Busy)?;
        self.password_checks.fetch_add(1, Ordering::Relaxed);
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), hash)
            .is_ok())
    }

    /// How many password checks have run.
    pub fn password_checks(&self) -> u64 {
        self.password_checks.load(Ordering::Relaxed)
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

enum SecondFactor {
    /// Two-step isn't on for this user.
    Off,
    Accepted,
    /// On, but no code was given.
    Missing,
    Wrong,
}

/// Tokens minted in the database but not yet live.
struct Minted {
    principal: Principal,
    access: String,
    refresh: String,
    refresh_ttl: u64,
}

fn session_days_on(db: &Connection) -> u32 {
    db.query_row(
        "SELECT value FROM settings WHERE key = 'session_days'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse().ok())
    .filter(|d| SESSION_DAYS.contains(d))
    .unwrap_or((REFRESH_TTL.as_secs() / 86_400) as u32)
}

/// Store a new refresh token for an active device (on the caller's connection or
/// transaction) and pick a new access token.
fn mint(db: &Connection, device: i64, now: i64) -> Result<Minted, AuthError> {
    type Row = (i64, String, String, i64, String, String, i64, i64);
    let row: Row = db.query_row(
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
        // Cleaned again: names stored before cleaning existed may hold anything.
        device_name: clean_device_name(&row.4),
        mac,
    };
    let access = new_secret();
    let refresh = new_secret();
    let refresh_ttl = u64::from(session_days_on(db)) * 86_400;
    db.execute(
        "INSERT INTO refresh_tokens (token_hash, device_id, expires_at) VALUES (?1, ?2, ?3)",
        params![sha256(&refresh).to_vec(), device, now + refresh_ttl as i64],
    )?;
    Ok(Minted {
        principal,
        access,
        refresh,
        refresh_ttl,
    })
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
    if version < 4 {
        // Refresh tokens remember when they were used and what replaced them (for the
        // lost-response grace); settings hold admin choices such as the sign-in lifetime.
        db.execute_batch(
            "ALTER TABLE refresh_tokens ADD COLUMN used_at INTEGER;
             ALTER TABLE refresh_tokens ADD COLUMN successor BLOB;
             CREATE TABLE IF NOT EXISTS settings (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL);
             PRAGMA user_version = 4;",
        )?;
    }
    if version < 5 {
        // Offline downloads: on by default for every user, in every format. Admins can turn
        // them off or limit the formats per user.
        db.execute_batch(
            "ALTER TABLE users ADD COLUMN downloads INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE users ADD COLUMN download_formats TEXT NOT NULL
               DEFAULT 'original,mp3,opus';
             PRAGMA user_version = 5;",
        )?;
    }
    if version < 6 {
        // Refresh tokens record their parent, so a token can have several replacements
        // (lost responses, concurrent refreshes); existing chains carry over. Revoked devices
        // record when, so they can be pruned (those revoked before now count from now).
        db.execute_batch(&format!(
            "ALTER TABLE refresh_tokens ADD COLUMN parent BLOB;
             UPDATE refresh_tokens SET parent = (
               SELECT p.token_hash FROM refresh_tokens p
               WHERE p.successor = refresh_tokens.token_hash);
             CREATE INDEX IF NOT EXISTS refresh_tokens_parent ON refresh_tokens(parent);
             ALTER TABLE devices ADD COLUMN revoked_at INTEGER;
             UPDATE devices SET revoked_at = {} WHERE revoked = 1;
             PRAGMA user_version = 6;",
            now_secs()
        ))?;
    }
    if version < 7 {
        // Offline plays sent back by devices (`POST /v1/plays`): the client's idempotency
        // key per user, so a retried upload never counts a play twice. `state` 0 is a play
        // being forwarded to LMS right now, 1 one LMS has recorded. Kept per user, not per
        // device, so a device that signs in again (a new device record) still dedupes. No
        // reference to the device: its record may be pruned while the key must stay.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS plays (
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               key TEXT NOT NULL,
               device_id INTEGER NOT NULL,
               track_id INTEGER NOT NULL,
               played_at INTEGER NOT NULL,
               state INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               PRIMARY KEY (user_id, key));
             CREATE INDEX IF NOT EXISTS plays_played_at ON plays(played_at);
             CREATE INDEX IF NOT EXISTS plays_user_created ON plays(user_id, created_at);
             PRAGMA user_version = 7;",
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

    /// Changed deliberately (finding M3): failures spread over many addresses throttle the
    /// username, but no longer lock its owner out. The addresses that failed are refused;
    /// the others share a small budget, which the right password from a clean address uses.
    #[test]
    fn repeated_failures_throttle_the_username_across_clients() {
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
        // The addresses that failed are refused for this user, even with the right password.
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "ip0"),
            Err(AuthError::Locked(_))
        ));
        // A fresh address still gets in with the right password.
        assert!(a.login("alice", PW, None, "x", None, "ip-new").is_ok());
        // Other users are unaffected.
        assert!(a.login("admin", PW, None, "x", None, "ip-other").is_ok());
    }

    /// Regression (M3): five failures for a username from one address locked every address
    /// out of it, the right password included, for up to a day: anyone could lock anyone.
    #[test]
    fn an_attacker_cannot_lock_a_user_out_from_elsewhere() {
        let a = store();
        // The attacker locks alice from address A, again and again (escalating).
        for _ in 0..3 * MAX_FAILURES {
            let _ = a.login("alice", "wrong password!", None, "x", None, "198.51.100.66");
        }
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "198.51.100.66"),
            Err(AuthError::Locked(_))
        ));
        // Alice, from address B, signs in.
        let t = a.login("alice", PW, None, "Phone", None, "203.0.113.20");
        assert!(t.is_ok(), "{t:?}");
        // Pairing codes likewise: one address's wrong guesses don't stop another's code.
        let code = a.create_pairing_code("alice").unwrap();
        for _ in 0..3 * MAX_FAILURES {
            let _ = a.pair("AAAAAAAAAA", "x", "198.51.100.67");
        }
        assert!(matches!(
            a.pair(&code, "x", "198.51.100.67"),
            Err(AuthError::Locked(_))
        ));
        assert!(a.pair(&code, "Tablet", "203.0.113.20").is_ok());
    }

    /// M3: spreading guesses over many addresses buys no more than the shared budget.
    #[test]
    fn many_addresses_cannot_exceed_the_guess_budget() {
        let a = store();
        let before = a.password_checks();
        let mut invalid = 0;
        for i in 0..200 {
            if a.login(
                "alice",
                "wrong password!",
                None,
                "x",
                None,
                &format!("bot{i}"),
            ) == Err(AuthError::Invalid)
            {
                invalid += 1;
            }
        }
        let budget = MAX_FAILURES + THROTTLED_ATTEMPTS;
        assert_eq!(
            invalid, budget as usize,
            "guesses checked in the first minute"
        );
        assert_eq!(a.password_checks() - before, u64::from(budget));
        // Within the same minute, even a fresh address waits: the budget is shared.
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "fresh"),
            Err(AuthError::Locked(d)) if d <= THROTTLE_WINDOW && d >= Duration::from_secs(1)
        ));
        // A minute later the budget refills (two more checks), and the right password works.
        {
            let mut limits = a.limits.lock().unwrap();
            let l = limits.get_mut("user:alice").unwrap();
            l.budget_start -= THROTTLE_WINDOW;
        }
        assert!(a.login("alice", PW, None, "x", None, "fresh").is_ok());
        // Pairing: the same shared budget across all addresses.
        let before = a.password_checks();
        let mut invalid = 0;
        for i in 0..200 {
            if a.pair("AAAAAAAAAA", "x", &format!("pbot{i}")) == Err(AuthError::Invalid) {
                invalid += 1;
            }
        }
        assert_eq!(invalid, budget as usize);
        assert_eq!(
            a.password_checks(),
            before,
            "pairing never hashes passwords"
        );
    }

    #[test]
    fn the_soft_lock_still_escalates_and_refuses_failed_clients() {
        let a = store();
        for _ in 0..MAX_FAILURES {
            let _ = a.login("alice", "wrong password!", None, "x", None, "a1");
        }
        let first = match a.login("alice", PW, None, "x", None, "a1") {
            Err(AuthError::Locked(d)) => d,
            other => panic!("{other:?}"),
        };
        assert!(first > Duration::from_secs(60), "{first:?}");
        // A budgeted failure from another address counts too, and that address is refused
        // from then on.
        assert_eq!(
            a.login("alice", "wrong password!", None, "x", None, "a2"),
            Err(AuthError::Invalid)
        );
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "a2"),
            Err(AuthError::Locked(_))
        ));
        // A success from a clean address doesn't lift the lock for the failed ones.
        assert!(a.login("alice", PW, None, "x", None, "a3").is_ok());
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "a1"),
            Err(AuthError::Locked(_))
        ));
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
        // Someone else used the new token (t2 → t3), and the old one comes back: two parties
        // hold tokens for this device, so it was stolen. Revoke the device entirely.
        let t3 = a.refresh(&t2.refresh).unwrap();
        assert_eq!(
            a.refresh(&t.refresh),
            Err(AuthError::Reused(t.principal.device_id))
        );
        assert!(a.authenticate(&t3.access).is_none());
        assert_eq!(a.refresh(&t3.refresh), Err(AuthError::Invalid));
        assert!(a.list_devices().unwrap().is_empty());
    }

    /// A phone suspended mid-refresh never got the new token, and tries the old one again.
    #[test]
    fn a_lost_refresh_response_can_be_retried_once_quickly() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let now = now_secs();
        let _lost = a.refresh_at(&t.refresh, now).unwrap(); // the response is lost
        let retry = a.refresh_at(&t.refresh, now + 5).unwrap();
        assert_eq!(retry.principal.device_id, t.principal.device_id);
        // The device carries on with the retried token.
        let next = a.refresh(&retry.refresh).unwrap();
        assert!(a.authenticate(&next.access).is_some());
        assert_eq!(a.list_devices().unwrap().len(), 1);
    }

    /// Regression (L4): a second refresh of the same token within the grace window (two tabs
    /// restoring at once) discarded the first replacement, so a client that kept that one
    /// revoked its own device on its next refresh.
    #[test]
    fn concurrent_refreshes_keep_both_replacements_usable() {
        for keep_first in [true, false] {
            let a = store();
            let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
            let now = now_secs();
            let b = a.refresh_at(&t.refresh, now).unwrap();
            let c = a.refresh_at(&t.refresh, now + 2).unwrap();
            let (kept, other) = if keep_first { (b, c) } else { (c, b) };
            let next = a.refresh_at(&kept.refresh, now + 600);
            assert!(next.is_ok(), "keep_first={keep_first}: {next:?}");
            assert!(a.device_active(t.principal.device_id));
            assert!(a.authenticate(&next.unwrap().access).is_some());
            // The one not kept is superseded: its access token is dropped at once...
            assert!(a.authenticate(&other.access).is_none());
            // ...and presenting it later means another party had it.
            assert_eq!(
                a.refresh_at(&other.refresh, now + 700),
                Err(AuthError::Reused(t.principal.device_id))
            );
            assert!(a.list_devices().unwrap().is_empty());
        }
    }

    /// The trade-off of the grace rule: a thief replaying a copied token within the window
    /// gets a replacement too, but as soon as both sides use theirs the device is revoked,
    /// and the side that uses its replacement first cuts the other's access off at once.
    #[test]
    fn a_thief_replaying_within_grace_is_caught_when_both_use_their_tokens() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let now = now_secs();
        let legit = a.refresh_at(&t.refresh, now).unwrap(); // the client gets S
        let thief = a.refresh_at(&t.refresh, now + 5).unwrap(); // a copy of T, within grace
        // The owner refreshes with S: the thief's sibling is superseded, access dropped.
        let owner = a.refresh_at(&legit.refresh, now + 10).unwrap();
        assert!(a.authenticate(&thief.access).is_none());
        // The thief tries its token: theft, the device is revoked for everyone.
        assert_eq!(
            a.refresh_at(&thief.refresh, now + 20),
            Err(AuthError::Reused(t.principal.device_id))
        );
        assert!(a.authenticate(&owner.access).is_none());
        assert_eq!(a.refresh(&owner.refresh), Err(AuthError::Invalid));
        assert!(a.list_devices().unwrap().is_empty());
    }

    /// Presenting the original after one of its replacements was used is theft, whichever.
    #[test]
    fn the_parent_after_any_replacement_was_used_is_theft() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let now = now_secs();
        let _b = a.refresh_at(&t.refresh, now).unwrap();
        let c = a.refresh_at(&t.refresh, now + 1).unwrap();
        a.refresh_at(&c.refresh, now + 2).unwrap();
        assert!(matches!(
            a.refresh_at(&t.refresh, now + 3),
            Err(AuthError::Reused(_))
        ));
        assert!(a.list_devices().unwrap().is_empty());
    }

    #[test]
    fn grace_retries_are_bounded() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let now = now_secs();
        for i in 0..MAX_REFRESH_RETRIES {
            a.refresh_at(&t.refresh, now + i).unwrap();
        }
        assert_eq!(
            a.refresh_at(&t.refresh, now + MAX_REFRESH_RETRIES),
            Err(AuthError::Invalid)
        );
        assert!(
            a.device_active(t.principal.device_id),
            "not theft, just refused"
        );
    }

    #[test]
    fn migration_carries_refresh_chains_and_dates_revocations() {
        // The two tables version 6 changes, as version 5 left them.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE devices (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL,
               name TEXT NOT NULL, mac TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL,
               last_seen INTEGER, revoked INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE refresh_tokens (token_hash BLOB PRIMARY KEY,
               device_id INTEGER NOT NULL, expires_at INTEGER NOT NULL,
               used INTEGER NOT NULL DEFAULT 0, used_at INTEGER, successor BLOB);
             PRAGMA user_version = 5;
             INSERT INTO devices VALUES (1, 1, 'Old', '02:4c:41:00:00:01', 0, NULL, 1);
             INSERT INTO devices VALUES (2, 1, 'New', '02:4c:41:00:00:02', 0, NULL, 0);
             INSERT INTO refresh_tokens VALUES (x'01', 2, 9999999999, 1, 5, x'02');
             INSERT INTO refresh_tokens VALUES (x'02', 2, 9999999999, 0, NULL, NULL);",
        )
        .unwrap();
        migrate(&db).unwrap();
        let parent: Option<Vec<u8>> = db
            .query_row(
                "SELECT parent FROM refresh_tokens WHERE token_hash = x'02'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent, Some(vec![1]));
        let revoked_at: Option<i64> = db
            .query_row("SELECT revoked_at FROM devices WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(revoked_at.is_some_and(|t| t > 0));
        let active_revoked_at: Option<i64> = db
            .query_row("SELECT revoked_at FROM devices WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(active_revoked_at, None);
    }

    /// Regression: rotation took the lock three times (mark used, issue, record successor),
    /// so a second refresh of the same token in between saw "used, no successor" and
    /// revoked a legitimate device.
    #[test]
    fn simultaneous_refreshes_of_one_token_never_revoke() {
        let a = std::sync::Arc::new(store());
        for _ in 0..200 {
            // Each round makes a device: keep them outside the new-devices-per-hour limit.
            a.db.lock()
                .unwrap()
                .execute("UPDATE devices SET created_at = created_at - 3601", [])
                .unwrap();
            let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let results: Vec<Result<Tokens, AuthError>> = (0..2)
                .map(|_| {
                    let (a, b, r) = (a.clone(), barrier.clone(), t.refresh.clone());
                    std::thread::spawn(move || {
                        b.wait();
                        a.refresh(&r)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect();
            assert!(
                results
                    .iter()
                    .all(|r| !matches!(r, Err(AuthError::Reused(_)))),
                "{results:?}"
            );
            assert!(results.iter().any(Result::is_ok), "{results:?}");
            assert!(a.device_active(t.principal.device_id));
            a.revoke_device(t.principal.device_id).unwrap();
        }
    }

    #[test]
    fn replays_outside_the_grace_window_still_revoke() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let now = now_secs();
        a.refresh_at(&t.refresh, now).unwrap();
        assert_eq!(
            a.refresh_at(&t.refresh, now + REUSE_GRACE_SECS + 1),
            Err(AuthError::Reused(t.principal.device_id))
        );
        assert!(a.list_devices().unwrap().is_empty());
        // A clock running backwards doesn't open the window either.
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        a.refresh_at(&t.refresh, now).unwrap();
        assert!(matches!(
            a.refresh_at(&t.refresh, now - 10),
            Err(AuthError::Reused(_))
        ));
    }

    #[test]
    fn a_retried_token_cannot_be_retried_again_after_its_replacement_is_used() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let now = now_secs();
        a.refresh_at(&t.refresh, now).unwrap();
        let retry = a.refresh_at(&t.refresh, now + 1).unwrap();
        a.refresh_at(&retry.refresh, now + 2).unwrap(); // the retried token is used
        assert!(matches!(
            a.refresh_at(&t.refresh, now + 3),
            Err(AuthError::Reused(_))
        ));
        assert!(a.list_devices().unwrap().is_empty());
    }

    #[test]
    fn admin_only_plugins_are_stored_checked_and_deduplicated() {
        let a = store();
        assert!(a.admin_only_plugins().is_empty());
        a.set_admin_only_plugins(&[
            "audiomuseai".into(),
            "selectVirtualLibrary".into(),
            "audiomuseai".into(),
        ])
        .unwrap();
        let got = a.admin_only_plugins();
        assert_eq!(got.len(), 2);
        assert!(got.contains("audiomuseai") && got.contains("selectVirtualLibrary"));
        for bad in [
            vec!["".to_string()],
            vec!["a b".into()],
            vec!["x/../y".into()],
            vec!["a".repeat(65)],
            vec!["x;rm".into()],
        ] {
            assert!(a.set_admin_only_plugins(&bad).is_err(), "{bad:?}");
        }
        let many: Vec<String> = (0..=MAX_ADMIN_ONLY_PLUGINS)
            .map(|i| format!("p{i}"))
            .collect();
        assert!(a.set_admin_only_plugins(&many).is_err());
        assert_eq!(
            a.admin_only_plugins().len(),
            2,
            "a refused change keeps the old set"
        );
        a.set_admin_only_plugins(&[]).unwrap();
        assert!(a.admin_only_plugins().is_empty());
        // A tampered stored value counts as none, and bad entries in it are skipped.
        let db = a.db.lock().unwrap();
        db.execute(
            "UPDATE settings SET value = '[\"ok\", \"bad word\"]' WHERE key = 'admin_only_plugins'",
            [],
        )
        .unwrap();
        drop(db);
        assert_eq!(
            a.admin_only_plugins(),
            ["ok".to_string()].into_iter().collect()
        );
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE settings SET value = 'nonsense' WHERE key = 'admin_only_plugins'",
                [],
            )
            .unwrap();
        assert!(a.admin_only_plugins().is_empty());
    }

    #[test]
    fn the_sign_in_lifetime_is_configurable_and_bounded() {
        let a = store();
        assert_eq!(a.session_days(), 30);
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        assert_eq!(t.refresh_expires_in, 30 * 86_400);
        a.set_session_days(90).unwrap();
        assert_eq!(a.session_days(), 90);
        // Applies from the next renewal: the lifetime slides with use.
        let t = a.refresh(&t.refresh).unwrap();
        assert_eq!(t.refresh_expires_in, 90 * 86_400);
        for bad in [0, 366, u32::MAX] {
            assert!(a.set_session_days(bad).is_err());
        }
        assert_eq!(a.session_days(), 90);
        // A bad stored value falls back to the default.
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE settings SET value = 'x' WHERE key = 'session_days'",
                [],
            )
            .unwrap();
        assert_eq!(a.session_days(), 30);
    }

    #[test]
    fn expired_refresh_tokens_are_refused() {
        let a = store();
        a.set_session_days(1).unwrap();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        assert_eq!(
            a.refresh_at(&t.refresh, now_secs() + 86_400 + 1),
            Err(AuthError::Invalid)
        );
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
        // Device names are cleaned rather than refused (see `hostile_device_names_are_cleaned`).
    }

    /// Regression (L7): names reached LMS (and admin pages) with markup and bidi controls.
    #[test]
    fn hostile_device_names_are_cleaned() {
        for (raw, want) in [
            ("", DEFAULT_DEVICE_NAME),
            ("   ", DEFAULT_DEVICE_NAME),
            ("\u{202e}\u{200b}\u{2066}", DEFAULT_DEVICE_NAME),
            ("<script>alert(1)</script>", "scriptalert(1)script"),
            ("bad\nname", "bad name"),
            ("  Firefox \t on   Linux ", "Firefox on Linux"),
            ("exe.\u{202e}gpj", "exe.gpj"),
            ("Zero\u{200b}Width\u{feff}", "ZeroWidth"),
            (
                "Bob's (spare) phone & co., v1.2_x-y",
                "Bob's (spare) phone & co., v1.2_x-y",
            ),
            ("James’s iPad", "James’s iPad"),
            ("Ταμπλέτα 台所 Küche", "Ταμπλέτα 台所 Küche"),
            ("Cafe\u{301}", "Cafe\u{301}"),
            ("\u{301}x", "x"),
            ("a\"b;c|d$e`f%g{h}", "abcdefgh"),
            ("http://evil.example/x", "httpevil.examplex"),
        ] {
            assert_eq!(clean_device_name(raw), want, "{raw:?}");
        }
        let long = clean_device_name(&"word ".repeat(40));
        assert!(long.chars().count() <= MAX_DEVICE_NAME, "{long}");
        assert!(!long.ends_with(' '));
        assert_eq!(clean_device_name(&"x".repeat(100)).len(), MAX_DEVICE_NAME);
        // Login and pairing store the cleaned name; refusing nothing.
        let a = store();
        let t = a
            .login("alice", PW, None, "<b>Phone\u{202e}</b>", None, "c")
            .unwrap();
        assert_eq!(t.principal.device_name, "bPhoneb");
        let code = a.create_pairing_code("alice").unwrap();
        let p = a.pair(&code, "\u{200b}", "c").unwrap();
        assert_eq!(p.principal.device_name, DEFAULT_DEVICE_NAME);
        assert!(
            a.list_devices()
                .unwrap()
                .iter()
                .all(|d| d.name == clean_device_name(&d.name))
        );
        // A name stored before cleaning existed is cleaned when it is used.
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE devices SET name = 'Old\u{202e}<i>' WHERE id = ?1",
                [t.principal.device_id],
            )
            .unwrap();
        let r = a.refresh(&t.refresh).unwrap();
        assert_eq!(r.principal.device_name, "Oldi");
    }

    /// Regression (L8): every sign-in made a new device (and LMS player), without limit.
    #[test]
    fn new_devices_per_user_are_rate_limited_but_reuse_is_not() {
        let a = store();
        let first = a.login("alice", PW, None, "D0", None, "c").unwrap();
        for i in 1..NEW_DEVICES_PER_HOUR {
            a.login("alice", PW, None, &format!("D{i}"), None, "c")
                .unwrap();
        }
        let over = a.login("alice", PW, None, "One more", None, "c");
        assert!(
            matches!(over, Err(AuthError::Locked(d)) if d > Duration::from_secs(60)),
            "{over:?}"
        );
        // Pairing counts too.
        let code = a.create_pairing_code("alice").unwrap();
        assert!(matches!(a.pair(&code, "x", "c"), Err(AuthError::Locked(_))));
        // A browser signing in again with its remembered device is never limited.
        for _ in 0..5 {
            let again = a
                .login(
                    "alice",
                    PW,
                    None,
                    "D0",
                    Some(first.principal.device_id),
                    "c",
                )
                .unwrap();
            assert_eq!(again.principal.device_id, first.principal.device_id);
        }
        // Another user is unaffected.
        assert!(a.login("admin", PW, None, "Desk", None, "c").is_ok());
        // An hour on, new devices are allowed again.
        a.db.lock()
            .unwrap()
            .execute("UPDATE devices SET created_at = created_at - 3601", [])
            .unwrap();
        assert!(a.login("alice", PW, None, "Later", None, "c").is_ok());
        assert!(
            a.audit_log(100)
                .unwrap()
                .iter()
                .any(|e| e.event == "device.rate_limited")
        );
    }

    #[test]
    fn revoked_devices_are_queued_for_lms_to_forget_and_pruned_later() {
        let a = store();
        let t1 = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let t2 = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        assert!(a.take_players_to_forget().is_empty());
        a.logout(&t1.access).unwrap();
        assert_eq!(a.take_players_to_forget(), vec![t1.principal.mac_string()]);
        assert!(a.take_players_to_forget().is_empty(), "drained");
        // Revoking twice queues once.
        a.revoke_device(t2.principal.device_id).unwrap();
        a.revoke_device(t2.principal.device_id).unwrap();
        assert_eq!(a.take_players_to_forget(), vec![t2.principal.mac_string()]);
        // Deleting a user queues its devices' players (before the rows go).
        let t3 = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        a.delete_user("admin").unwrap();
        assert_eq!(a.take_players_to_forget(), vec![t3.principal.mac_string()]);
        // Revoked rows are pruned once they are old enough.
        let count = |a: &Accounts| -> i64 {
            a.db.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&a), 2);
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE devices SET revoked_at = ?1 WHERE id = ?2",
                params![
                    now_secs() - REVOKED_DEVICE_RETENTION.as_secs() as i64 - 1,
                    t1.principal.device_id
                ],
            )
            .unwrap();
        a.login("alice", PW, None, "New", None, "c").unwrap();
        assert_eq!(
            count(&a),
            2,
            "the old revoked row is gone, the new device added"
        );
        assert!(
            !a.list_devices()
                .unwrap()
                .iter()
                .any(|d| d.id == t1.principal.device_id)
        );
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
        let t2 = a.refresh(&t.refresh).unwrap();
        let _ = a.refresh(&t.refresh); // a lost response, retried
        let _ = a.refresh(&t2.refresh); // the first replacement is still good
        let t3 = a
            .login("alice", PW, None, "Phone", None, "10.0.0.9")
            .unwrap();
        let t4 = a.refresh(&t3.refresh).unwrap();
        let _ = a.refresh(&t4.refresh); // the replacement is used…
        let _ = a.refresh(&t3.refresh); // …then the old one comes back: theft
        let events: Vec<String> = a
            .audit_log(50)
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect();
        for e in [
            "login.failed",
            "login.ok",
            "refresh.retried",
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
        let setup = a.totp_begin(id, username, PW).unwrap();
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
        a.totp_begin(id, "alice", PW).unwrap();
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
            matches!(a.totp_begin(id, "alice", PW), Err(AuthError::BadInput(_))),
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
        // The client that failed is locked out, even with a good code...
        let code = current_code(&a, "alice", 1);
        assert!(matches!(
            a.login("alice", PW, Some(&code), "x", None, "c"),
            Err(AuthError::Locked(_))
        ));
        // ...but the user, elsewhere, with the password and a good code, gets in.
        assert!(a.login("alice", PW, Some(&code), "x", None, "c2").is_ok());
    }

    #[test]
    fn setup_needs_the_password() {
        let a = store();
        let id = user_id(&a, "alice");
        assert_eq!(
            a.totp_begin(id, "alice", "wrong password!"),
            Err(AuthError::Invalid)
        );
        assert_eq!(a.totp_begin(id, "alice", ""), Err(AuthError::Invalid));
        // Nothing pending: confirming has nothing to confirm.
        assert!(matches!(
            a.totp_confirm(id, "alice", "123456"),
            Err(AuthError::BadInput(_))
        ));
        // Wrong passwords here count towards a lockout of their own.
        for _ in 0..MAX_FAILURES - 2 {
            let _ = a.totp_begin(id, "alice", "wrong password!");
        }
        assert!(matches!(
            a.totp_begin(id, "alice", PW),
            Err(AuthError::Locked(_))
        ));
    }

    /// Regression: the step was read, checked and written separately, so two requests
    /// carrying the same code could both pass.
    #[test]
    fn a_code_is_consumed_once_even_by_racing_requests() {
        let a = store();
        enrol(&a, "alice");
        let id = user_id(&a, "alice");
        let (secret, _, last_step) = a.totp_state(id).unwrap();
        let secret = secret.unwrap();
        let code = current_code(&a, "alice", 1);
        // Both requests read the same last step before either records its use.
        assert!(a.consume_code(id, &secret, &code, last_step).unwrap());
        assert!(!a.consume_code(id, &secret, &code, last_step).unwrap());
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
        assert_eq!(version, 7);
        // Version 3 added per-device diagnostics.
        let d = a.devices_for_test();
        a.save_diagnostics(d, r#"{"ok":true}"#).unwrap();
        assert_eq!(a.diagnostics(d).unwrap().unwrap().1, r#"{"ok":true}"#);
        // Version 4: refresh-token history and settings.
        a.set_session_days(7).unwrap();
        assert_eq!(a.session_days(), 7);
        let t = a.login("alice", PW, None, "Phone", None, "c");
        assert!(t.is_err() || a.refresh(&t.unwrap().refresh).is_ok());
        // Version 5: download permissions, on by default in every format.
        let users = a.list_users().unwrap();
        assert!(users[0].downloads);
        assert_eq!(users[0].download_formats, ["original", "mp3", "opus"]);
    }

    // -- downloads -------------------------------------------------------------

    #[test]
    fn downloads_are_on_by_default_and_can_be_limited() {
        let a = store();
        let alice = user_id(&a, "alice");
        let p = a.download_policy(alice).unwrap();
        assert!(p.enabled);
        assert_eq!(p.formats, ["original", "mp3", "opus"]);
        assert!(p.allows("opus"));

        a.set_downloads("alice", true, Some(&["opus".into(), "original".into()]))
            .unwrap();
        let p = a.download_policy(alice).unwrap();
        assert_eq!(p.formats, ["original", "opus"], "stored in canonical order");
        assert!(!p.allows("mp3"));

        a.set_downloads("alice", false, None).unwrap();
        let p = a.download_policy(alice).unwrap();
        assert!(!p.enabled);
        assert_eq!(
            p.formats,
            ["original", "opus"],
            "formats kept when only toggled"
        );
        assert!(!p.allows("original"), "off means nothing is allowed");

        let listed = a.list_users().unwrap();
        let al = listed.iter().find(|u| u.username == "alice").unwrap();
        assert!(!al.downloads);
        assert_eq!(al.download_formats, ["original", "opus"]);
        let events: Vec<String> = a
            .audit_log(10)
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect();
        assert!(events.contains(&"user.downloads".to_string()));
    }

    #[test]
    fn download_settings_validate_input() {
        let a = store();
        let bad: [Vec<String>; 6] = [
            vec![],
            vec!["flac".into()],
            vec!["original".into(), "original,mp3".into()],
            vec!["MP3".into()],
            vec!["original".into(), "mp3".into(), "opus".into(), "mp3".into()],
            vec!["../x".into()],
        ];
        for b in bad {
            assert!(
                matches!(
                    a.set_downloads("alice", true, Some(&b)),
                    Err(AuthError::BadInput(_))
                ),
                "{b:?}"
            );
        }
        assert!(matches!(
            a.set_downloads("nobody", true, None),
            Err(AuthError::BadInput(_))
        ));
        // A disabled or unknown user may download nothing.
        assert!(!a.download_policy(9999).unwrap().enabled);
        a.set_disabled("alice", true).unwrap();
        assert!(!a.download_policy(user_id(&a, "alice")).unwrap().enabled);
    }

    #[test]
    fn stored_formats_are_cleaned_when_read() {
        assert_eq!(
            parse_formats("original,mp3,opus"),
            ["original", "mp3", "opus"]
        );
        assert_eq!(parse_formats("opus, evil ,original"), ["original", "opus"]);
        assert!(parse_formats("").is_empty());
        assert!(parse_formats("flac;mp3").is_empty());
    }

    #[test]
    fn the_download_key_salt_is_random_and_stable() {
        let a = store();
        let s = a.download_key_salt().unwrap();
        assert_eq!(s.len(), 64);
        assert_eq!(a.download_key_salt().unwrap(), s);
        assert_ne!(store().download_key_salt().unwrap(), s);
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

    // -- abuse limits ----------------------------------------------------------

    /// Regression: the lockout was checked before hashing and counted after, so many
    /// parallel wrong guesses were all checked.
    #[test]
    fn parallel_wrong_guesses_are_limited_before_hashing() {
        let a = std::sync::Arc::new(store());
        let before = a.password_checks();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(40));
        let results: Vec<Result<Tokens, AuthError>> = (0..40)
            .map(|_| {
                let (a, b) = (a.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    a.login("alice", "wrong password!", None, "x", None, "203.0.113.5")
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let checked = a.password_checks() - before;
        assert!(
            checked <= u64::from(MAX_FAILURES),
            "{checked} guesses checked"
        );
        let invalid = results
            .iter()
            .filter(|r| **r == Err(AuthError::Invalid))
            .count();
        assert!(invalid <= MAX_FAILURES as usize, "{invalid} evaluated");
        assert!(
            results.iter().all(|r| matches!(
                r,
                Err(AuthError::Invalid | AuthError::Locked(_) | AuthError::Busy)
            )),
            "{results:?}"
        );
        // The guessing address is locked out now, even with the right password; the user,
        // elsewhere, is not (changed deliberately, finding M3).
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "203.0.113.5"),
            Err(AuthError::Locked(_))
        ));
        assert!(
            a.login("alice", PW, None, "x", None, "198.51.100.1")
                .is_ok()
        );
    }

    #[test]
    fn password_checks_beyond_the_limit_are_refused_as_busy() {
        let mut a = store();
        a.hash_gate.wait = Duration::from_millis(50);
        let held: Vec<_> = (0..HASH_PERMITS)
            .map(|_| a.hash_gate.acquire().unwrap())
            .collect();
        assert_eq!(
            a.login("alice", PW, None, "x", None, "c"),
            Err(AuthError::Busy)
        );
        // A busy refusal is not a failure: it doesn't count towards a lockout.
        for _ in 0..MAX_FAILURES {
            let _ = a.login("alice", PW, None, "x", None, "c");
        }
        drop(held);
        assert!(a.login("alice", PW, None, "x", None, "c").is_ok());
    }

    #[test]
    fn malformed_usernames_get_no_lockout_entry() {
        let a = store();
        for i in 0..50 {
            let _ = a.login(
                &format!("Bad Name {i}"),
                "whatever!!",
                None,
                "x",
                None,
                &format!("c{i}"),
            );
        }
        let limits = a.limits.lock().unwrap();
        assert!(
            limits.keys().all(|k| !k.starts_with("user:")),
            "{:?}",
            limits.keys()
        );
    }

    #[test]
    fn the_lockout_table_is_bounded_and_keeps_active_locks() {
        let a = store();
        // A locked-out client...
        for _ in 0..MAX_FAILURES {
            let _ = a.login("alice", "wrong password!", None, "x", None, "victim-lock");
        }
        // ...then a flood of one-off failures from other addresses.
        {
            let mut limits = a.limits.lock().unwrap();
            let now = Instant::now();
            for i in 0..LIMITS_CAP + 500 {
                record_failure_in(&mut limits, &format!("client:flood{i}"), now);
            }
            assert!(limits.len() <= LIMITS_CAP, "{}", limits.len());
            assert!(limits.contains_key("client:victim-lock"));
        }
        assert!(matches!(
            a.login("admin", PW, None, "x", None, "victim-lock"),
            Err(AuthError::Locked(_))
        ));
        // Expired entries are cleared first.
        let mut limits = a.limits.lock().unwrap();
        let later = Instant::now() + FAILURE_WINDOW + MAX_LOCKOUT;
        let len = limits.len();
        limits.retain(|_, l| l.live(later));
        assert!(limits.len() < len / 2, "{}", limits.len());
    }

    #[test]
    fn devices_per_user_are_capped_by_revoking_the_least_recently_seen() {
        let a = store();
        // Spread over time, as new devices per hour are limited.
        let age = |a: &Accounts| {
            a.db.lock()
                .unwrap()
                .execute("UPDATE devices SET created_at = created_at - 3601", [])
                .unwrap();
        };
        let first = a.login("alice", PW, None, "Old", None, "c").unwrap();
        let mut rest = vec![];
        for i in 1..MAX_DEVICES_PER_USER {
            age(&a);
            rest.push(
                a.login("alice", PW, None, &format!("D{i}"), None, "c")
                    .unwrap(),
            );
        }
        // The first device is the one in use; the second has been idle the longest.
        a.touch_device(first.principal.device_id);
        let idle = rest[0].principal.device_id;
        a.db.lock()
            .unwrap()
            .execute("UPDATE devices SET last_seen = 1 WHERE id = ?1", [idle])
            .unwrap();
        let t = a.login("alice", PW, None, "New", None, "c").unwrap();
        assert_eq!(t.evicted, vec![idle]);
        assert!(!a.device_active(idle));
        assert!(a.device_active(first.principal.device_id));
        let alice = a
            .list_devices()
            .unwrap()
            .into_iter()
            .filter(|d| d.username == "alice")
            .count();
        assert_eq!(alice, MAX_DEVICES_PER_USER);
        // Pairing is subject to the same limit.
        let code = a.create_pairing_code("alice").unwrap();
        let p = a.pair(&code, "Paired", "c").unwrap();
        assert_eq!(p.evicted.len(), 1);
        // Signing in again on an existing device doesn't evict anything.
        let again = a
            .login("alice", PW, None, "New", Some(t.principal.device_id), "c")
            .unwrap();
        assert!(again.evicted.is_empty());
    }

    #[test]
    fn a_password_reset_signs_the_user_out_everywhere() {
        let a = store();
        let t1 = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let t2 = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let other = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        let mut revoked = a.set_password("alice", "a brand new password").unwrap();
        revoked.sort();
        assert_eq!(
            revoked,
            vec![t1.principal.device_id, t2.principal.device_id]
        );
        assert!(a.authenticate(&t1.access).is_none());
        assert_eq!(a.refresh(&t2.refresh), Err(AuthError::Invalid));
        assert!(a.authenticate(&other.access).is_some());
        assert!(
            a.login("alice", "a brand new password", None, "x", None, "c")
                .is_ok()
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

    #[test]
    fn play_keys_are_claimed_once_and_given_back_on_failure() {
        let a = store();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let admin = a.user_id_by_name("admin").unwrap().unwrap();
        let stale = Duration::from_secs(120);
        let at = unix_now() - 60;
        assert_eq!(
            a.claim_play(alice, 1, "key-0001", 5, at, stale).unwrap(),
            PlayClaim::Claimed
        );
        assert_eq!(
            a.claim_play(alice, 2, "key-0001", 5, at, stale).unwrap(),
            PlayClaim::InFlight
        );
        assert_eq!(
            a.claim_play(admin, 3, "key-0001", 5, at, stale).unwrap(),
            PlayClaim::Claimed,
            "keys are per user"
        );
        a.release_play(alice, "key-0001").unwrap();
        assert_eq!(
            a.claim_play(alice, 1, "key-0001", 5, at, stale).unwrap(),
            PlayClaim::Claimed
        );
        a.finish_play(alice, "key-0001").unwrap();
        assert_eq!(
            a.claim_play(alice, 1, "key-0001", 5, at, stale).unwrap(),
            PlayClaim::Recorded
        );
        // A recorded key is never given back.
        a.release_play(alice, "key-0001").unwrap();
        assert_eq!(
            a.claim_play(alice, 1, "key-0001", 5, at, stale).unwrap(),
            PlayClaim::Recorded
        );
        // A claim left pending (a crash mid-forward) is taken over once stale.
        assert_eq!(
            a.claim_play(alice, 1, "key-0002", 5, at, stale).unwrap(),
            PlayClaim::Claimed
        );
        assert_eq!(
            a.claim_play(alice, 1, "key-0002", 5, at, Duration::ZERO)
                .unwrap(),
            PlayClaim::Claimed
        );
        // Deleting the user deletes their keys.
        assert_eq!(a.stored_plays(admin).unwrap(), 1);
        a.delete_user("admin").unwrap();
        assert_eq!(a.stored_plays(admin).unwrap(), 0);
        assert_eq!(a.user_id_by_name("admin").unwrap(), None);
    }

    #[test]
    fn play_keys_are_pruned_by_age_and_capped_per_user() {
        let a = store();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let stale = Duration::from_secs(120);
        let now = unix_now();
        for i in 0..10u64 {
            let key = format!("key-{i:04}");
            a.claim_play(
                alice,
                1,
                &key,
                5,
                now - 100 * 86_400 + i * 20 * 86_400,
                stale,
            )
            .unwrap();
            a.finish_play(alice, &key).unwrap();
        }
        // Plays from before the cut-off go.
        a.prune_plays(None, now - 91 * 86_400, 1000).unwrap();
        assert_eq!(a.stored_plays(alice).unwrap(), 9);
        // At most `cap` recorded keys are kept, the newest.
        a.prune_plays(Some(alice), 0, 4).unwrap();
        assert_eq!(a.stored_plays(alice).unwrap(), 4);
        assert_eq!(
            a.claim_play(alice, 1, "key-0009", 5, now, stale).unwrap(),
            PlayClaim::Recorded
        );
    }

    #[test]
    fn the_plays_table_is_added_to_an_existing_database() {
        let db = Connection::open_in_memory().unwrap();
        let a = Accounts::with_connection(db).unwrap();
        let v: i64 =
            a.db.lock()
                .unwrap()
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
        assert_eq!(v, 7);
        a.create_user("alice", PW, Role::Listener).unwrap();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        assert_eq!(a.stored_plays(alice).unwrap(), 0);
    }
}
