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
//!   - per client address, a hard lock. A success clears only the failures of the user
//!     who signed in, so an attacker with an account of their own can't reset the lock;
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
//!   towards the same lockouts as wrong passwords. So does the right password without a
//!   code: it costs a password check like any other attempt.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};

use anywhere_core::proto::format_mac;

pub const ACCESS_TTL: Duration = Duration::from_secs(15 * 60);
/// Default sign-in lifetime in days: unused this long, a device must sign in again. Every
/// use renews it (refresh tokens rotate, each with a fresh lifetime). Admins can change it
/// (the LMS plugin's settings page shows the same default).
pub const DEFAULT_SESSION_DAYS: u32 = 90;
/// [`DEFAULT_SESSION_DAYS`] as a duration.
pub const REFRESH_TTL: Duration = Duration::from_secs(DEFAULT_SESSION_DAYS as u64 * 24 * 3600);
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
/// Pairing codes a signed-in user may create for their own devices per hour
/// (`POST /v1/devices/pair`). Admins are not limited.
pub const SELF_PAIRING_PER_HOUR: i64 = 5;
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
/// Audit entries are kept this long…
pub const AUDIT_RETENTION: Duration = Duration::from_secs(365 * 24 * 3600);
/// …and at most this many (the newest). Failed sign-ins are audited, so without a bound
/// anyone could grow the database.
pub const AUDIT_MAX_ROWS: i64 = 100_000;
/// The audit log is pruned once per this many entries written.
const AUDIT_PRUNE_EVERY: i64 = 1000;
/// Revoked devices are kept this long (for the audit trail), then deleted.
pub const REVOKED_DEVICE_RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);
/// Siblings one refresh token may have (lost responses and concurrent refreshes).
const MAX_REFRESH_RETRIES: i64 = 8;
/// Renewals one device may make per [`REFRESH_WINDOW`]. A client renews every few minutes
/// at most (more on a burst of page loads); a loop of renewals is refused.
pub const REFRESHES_PER_WINDOW: u32 = 30;
pub const REFRESH_WINDOW: Duration = Duration::from_secs(60);
/// Live access tokens kept per device (older ones still unexpired are dropped).
pub const MAX_ACCESS_PER_DEVICE: usize = 8;
/// Used refresh tokens kept per device for reuse detection (newest first).
const KEEP_USED_REFRESH: i64 = 32;
/// Unused refresh tokens kept per device when it signs in again (newest first): the
/// earlier ones belong to sign-ins the client has replaced.
const KEEP_UNUSED_REFRESH: i64 = 4;
/// Revoked devices' players waiting to be forgotten by LMS (beyond this, oldest dropped).
const FORGET_QUEUE_CAP: usize = 1000;
/// A device name that has nothing usable left after cleaning.
pub const DEFAULT_DEVICE_NAME: &str = "Browser";
pub const MAX_DEVICE_NAME: usize = 64;
/// A device's last address and time are written at most this often while it is in use
/// (sign-in and token renewal always write).
pub const SEEN_WRITE_INTERVAL: Duration = Duration::from_secs(60);
/// Devices whose last write time is remembered in memory (beyond this, the stalest go).
const SEEN_CAP: usize = 4096;
/// Longest client platform description kept, e.g. "Safari on iPadOS" or "Pixel 8".
pub const MAX_CLIENT_PLATFORM: usize = 48;
/// Longest User-Agent kept (for display only).
pub const MAX_USER_AGENT: usize = 200;

/// What kind of client a device is, as it reports itself at sign-in and renewal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// The web app installed to the home screen or as an app (standalone display mode).
    Pwa,
    /// The web app in an ordinary browser tab.
    Browser,
    /// The Android app.
    Android,
}

impl ClientKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ClientKind::Pwa => "pwa",
            ClientKind::Browser => "browser",
            ClientKind::Android => "android",
        }
    }

    pub fn parse(s: &str) -> Option<ClientKind> {
        match s {
            "pwa" => Some(ClientKind::Pwa),
            "browser" => Some(ClientKind::Browser),
            "android" => Some(ClientKind::Android),
            _ => None,
        }
    }
}

/// What a client said about itself (each part optional: older clients send nothing).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    pub kind: Option<ClientKind>,
    /// Cleaned again by [`clean_platform`] when stored.
    pub platform: Option<String>,
    /// Cleaned again by [`clean_user_agent`] when stored.
    pub user_agent: Option<String>,
}

/// A client's platform description: cleaned like a device name, at most
/// [`MAX_CLIENT_PLATFORM`] characters. `None` if nothing usable is left.
pub fn clean_platform(raw: &str) -> Option<String> {
    let s: String = clean_name(raw)?.chars().take(MAX_CLIENT_PLATFORM).collect();
    let s = s.trim_end().to_string();
    (!s.is_empty()).then_some(s)
}

/// A User-Agent for display: printable ASCII only, without characters that mean anything
/// in markup (`< > " ' ` \ &`), runs of spaces collapsed, at most [`MAX_USER_AGENT`]
/// characters. `None` if nothing is left.
pub fn clean_user_agent(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut space = false;
    for c in raw.chars() {
        if c == ' ' || c == '\t' {
            space = !out.is_empty();
            continue;
        }
        if !c.is_ascii_graphic() || "<>\"'`\\&".contains(c) {
            continue;
        }
        let need = if space { 2 } else { 1 };
        if out.len() + need > MAX_USER_AGENT {
            break;
        }
        if space {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

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
    /// The client address it was last seen from (as resolved behind trusted proxies).
    pub last_addr: Option<String>,
    /// `pwa`, `browser` or `android`, as the client last reported.
    pub client_kind: Option<String>,
    /// E.g. "Safari on iPadOS" or "Pixel 8".
    pub client_platform: Option<String>,
    pub user_agent: Option<String>,
}

/// One of a user's own devices, as they see it (no addresses or MACs).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OwnDevice {
    pub id: i64,
    pub name: String,
    /// `pwa`, `browser` or `android`, as the client last reported.
    pub client_kind: Option<String>,
    /// E.g. "Safari on iPadOS" or "Pixel 8".
    pub client_platform: Option<String>,
    /// When it first signed in (Unix seconds).
    pub created_at: i64,
    /// When it was last used (Unix seconds), to the minute.
    pub last_seen: Option<i64>,
}

/// A signed-in user's request to change their own password
/// ([`Accounts::change_own_password`]).
pub struct OwnPasswordChange<'a> {
    pub user: i64,
    pub username: &'a str,
    /// The device asking (it stays signed in).
    pub device: i64,
    pub current: &'a str,
    pub new: &'a str,
    /// The two-step code, needed when two-step is on.
    pub code: Option<&'a str>,
    /// Also sign out the user's other devices.
    pub sign_out_others: bool,
}

/// What [`Accounts::change_own_password`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordChanged {
    /// Devices signed out with it (their live sessions are the caller's to end).
    pub revoked: Vec<i64>,
    /// The user's other devices, still signed in.
    pub others: Vec<OwnDevice>,
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
    /// Hard keys: how many of `failures` each username's attempts made, so a success
    /// clears only that user's own (at most [`FAILED_CLIENTS_CAP`] users; failures beyond
    /// that are nobody's and stay until the window ends).
    by_user: HashMap<String, u32>,
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
            by_user: HashMap::new(),
        }
    }

    fn roll_window(&mut self, now: Instant) {
        if now.duration_since(self.window_start) > FAILURE_WINDOW {
            self.failures = 0;
            self.by_user.clear();
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
    /// The username the attempt was for, if it names one that can exist: its failures on
    /// the hard keys are its own, cleared by its own success (see [`Attempt::by`]).
    who: Option<String>,
    done: bool,
}

impl Attempt<'_> {
    fn all_keys(&self) -> impl Iterator<Item = &String> {
        self.keys.iter().chain(self.soft.as_ref().map(|(k, _)| k))
    }

    /// Attribute the attempt to `username`.
    fn by(mut self, username: &str) -> Self {
        self.who = Some(username.to_string());
        self
    }

    fn release(&self, limits: &mut HashMap<String, Lock>) {
        for k in self.all_keys() {
            if let Some(l) = limits.get_mut(k) {
                l.in_flight = l.in_flight.saturating_sub(1);
            }
        }
    }

    fn failed(self) {
        self.count_failure(true);
    }

    /// The right password without the two-step code it needs: counted against the hard
    /// keys like a failure, so whoever knows the password can't have it checked without
    /// limit (each check a full hash, holding a permit). Not against the soft key: the
    /// usual two-step sign-in asks without a code first, and while someone else's wrong
    /// guesses throttle the username, that must not make the owner's address one that
    /// failed. The owner's success then clears it.
    fn needs_code(self) {
        self.count_failure(false);
    }

    fn count_failure(mut self, soft_too: bool) {
        self.done = true;
        let mut limits = self.accounts.limits.lock().unwrap();
        let now = Instant::now();
        self.release(&mut limits);
        for k in &self.keys {
            record_failure_in(&mut limits, k, self.who.as_deref(), now);
        }
        if !soft_too {
            return;
        }
        if let Some((k, client)) = &self.soft {
            record_failure_in(&mut limits, k, None, now);
            if let Some(l) = limits.get_mut(k)
                && l.failed_clients.len() < FAILED_CLIENTS_CAP
            {
                l.failed_clients.insert(client.clone());
            }
        }
    }

    /// Clears the user's own failures from the hard keys. An address's failures for other
    /// usernames (or for none) still count: an attacker with an account of their own must
    /// not be able to reset the address's lock by signing in to it. A soft key stays
    /// locked if it is (the attacker's failures still count), but this client is no longer
    /// one that failed.
    fn succeeded(mut self) {
        self.done = true;
        let mut limits = self.accounts.limits.lock().unwrap();
        self.release(&mut limits);
        for k in &self.keys {
            if let Some(l) = limits.get_mut(k) {
                let own = self
                    .who
                    .as_ref()
                    .and_then(|w| l.by_user.remove(w))
                    .unwrap_or(0);
                l.failures = l.failures.saturating_sub(own);
            }
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

/// Count a failure against `key`, made by `who` (a username) if known.
fn record_failure_in(
    limits: &mut HashMap<String, Lock>,
    key: &str,
    who: Option<&str>,
    now: Instant,
) {
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
        l.by_user.clear();
    } else if let Some(w) = who
        && (l.by_user.len() < FAILED_CLIENTS_CAP || l.by_user.contains_key(w))
    {
        *l.by_user.entry(w.to_string()).or_insert(0) += 1;
    }
}

/// Failures allowed per key within the window before a lockout.
const MAX_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const FIRST_LOCKOUT: Duration = Duration::from_secs(15 * 60);
const MAX_LOCKOUT: Duration = Duration::from_secs(24 * 3600);

pub struct Accounts {
    /// The read-only connection that request-path reads use (see [`Self::reader`]), on the
    /// same file as `db`. Under WAL a reader never waits for the writer (nor a write stalled
    /// on the disk) and sees every committed write. `None` for an in-memory database, which
    /// a second connection could not share: reads then use `db`. Declared before `db` so it
    /// closes first, leaving the writer to checkpoint and tidy the WAL away.
    reader: Option<Mutex<Connection>>,
    /// The one connection that writes (and reads that must see the same transaction).
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
    /// When each device's last address was written (to rate-limit the writes).
    seen: Mutex<HashMap<i64, Instant>>,
    /// Renewals per device in the current window: (window start, count).
    refreshes: Mutex<HashMap<i64, (Instant, u32)>>,
    /// Tests only: how long each device-bookkeeping and renewal write stalls (ms), as on a
    /// disk that has stopped answering. 0 in production.
    write_stall_ms: AtomicU64,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A short, one-way label for a refresh token in the log (8 hex digits of its hash's hash):
/// enough to follow a token family across lines, useless for presenting.
pub(crate) fn fingerprint(token: &str) -> String {
    fingerprint_of_hash(&sha256(token))
}

fn fingerprint_of_hash(h: &[u8]) -> String {
    let d = Sha256::digest(h);
    d[..4].iter().map(|b| format!("{b:02x}")).collect()
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

fn prune_audit_on(db: &Connection, max_rows: i64, oldest: i64) -> rusqlite::Result<usize> {
    let old = db.execute("DELETE FROM audit WHERE at < ?1", [oldest])?;
    let surplus = db.execute(
        "DELETE FROM audit WHERE id <= (SELECT COALESCE(MAX(id), 0) FROM audit) - ?1",
        [max_rows.max(0)],
    )?;
    Ok(old + surplus)
}

/// The read connection for the database at `path`: opened read-only, and `query_only` as
/// well, so a write through it is refused whatever SQL asks. The busy timeout is rusqlite's
/// (5 s, as on the writer): under WAL a reader waits only while the database is recovered.
fn open_reader(path: &Path) -> Result<Connection, AuthError> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA query_only = 1;",
    )?;
    Ok(db)
}

/// Pairing codes: 10 characters from an unambiguous alphabet (~50 bits), easy to type.
const PAIRING_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

impl Accounts {
    /// Open (creating and migrating as needed) the database at `path`, with a second,
    /// read-only connection for reads.
    pub fn open(path: &Path) -> Result<Accounts, AuthError> {
        let mut a = Self::with_connection(Connection::open(path)?)?;
        // Opened only now, so it sees the schema `migrate` has brought up to date.
        a.reader = Some(Mutex::new(open_reader(path)?));
        Ok(a)
    }

    /// A private in-memory database (tests, the demo). It has the one connection: reads go
    /// through the writer, so they see what it wrote.
    pub fn in_memory() -> Result<Accounts, AuthError> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    /// Whether reads have a connection of their own (a database on disk).
    pub fn has_read_connection(&self) -> bool {
        self.reader.is_some()
    }

    /// The connection for reads: the read-only one, or the writer for an in-memory database.
    /// Never call it with the writer's lock held (in memory, that is the same lock).
    fn reader(&self) -> MutexGuard<'_, Connection> {
        self.reader.as_ref().unwrap_or(&self.db).lock().unwrap()
    }

    /// Run `f` on the write connection (for the per-user stores in [`crate::history`],
    /// [`crate::home`], [`crate::ratings`], [`crate::filters`] and [`crate::stations`],
    /// which keep their own SQL). Anything that writes, or reads to decide a write, uses this.
    pub(crate) fn with_db<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, AuthError> {
        Ok(f(&self.db.lock().unwrap())?)
    }

    /// Run `f`, which only reads, on the read connection: it doesn't wait for a write in
    /// progress, and sees every write committed before it started. A write attempted here
    /// fails (the connection is read-only).
    pub(crate) fn read_db<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, AuthError> {
        Ok(f(&self.reader())?)
    }

    /// The database's schema version (for tests).
    pub fn schema_version(&self) -> i64 {
        self.db
            .lock()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0)
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
               id INTEGER PRIMARY KEY AUTOINCREMENT,
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
            reader: None,
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
            seen: Mutex::new(HashMap::new()),
            refreshes: Mutex::new(HashMap::new()),
            write_stall_ms: AtomicU64::new(0),
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
        // Disabled first, so no sign-in or renewal can add a device (and a live token)
        // between revoking the devices and deleting the user.
        self.db.lock().unwrap().execute(
            "UPDATE users SET disabled = 1 WHERE username = ?1",
            [username],
        )?;
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
        let db = self.reader();
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
            .reader()
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
            // Nobody's failure: no success clears it.
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
        let attempt = attempt.by(username);
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
            // Counted against the address (see `Attempt::needs_code`) and audited.
            SecondFactor::Missing => {
                attempt.needs_code();
                self.audit("login.totp_required", Some(username), None, client);
                return Err(AuthError::TotpRequired);
            }
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
        let user: Option<i64> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM users WHERE username = ?1 AND disabled = 0",
                [username],
                |r| r.get(0),
            )
            .optional()?;
        let user = user.ok_or(AuthError::BadInput("no such enabled user"))?;
        let code = self.insert_pairing_code(user)?;
        // The code itself is never written anywhere but its hash.
        self.audit("pairing.created", Some(username), None, "admin");
        Ok(code)
    }

    /// A signed-in user creates a one-time pairing code for another device of their own
    /// (`POST /v1/devices/pair`). The code signs in as this user only.
    ///
    /// - Needs the account password, so a stolen access token (15 minutes) alone can't be
    ///   turned into a lasting sign-in on the thief's device. Wrong passwords count towards
    ///   the per-user lock that guards other account changes ([`Self::totp_begin`]).
    /// - At most [`SELF_PAIRING_PER_HOUR`] an hour per user, and none while the user is at
    ///   the [`NEW_DEVICES_PER_HOUR`] limit (the code could not be redeemed anyway). Both
    ///   are checked before the password, so a refused request costs no password check.
    pub fn create_own_pairing_code(
        &self,
        user: i64,
        username: &str,
        device: i64,
        password: &str,
    ) -> Result<String, AuthError> {
        let now = now_secs();
        {
            let db = self.db.lock().unwrap();
            db.execute("DELETE FROM pairing_requests WHERE at <= ?1", [now - 3600])?;
            let (made, oldest): (i64, Option<i64>) = db.query_row(
                "SELECT COUNT(*), MIN(at) FROM pairing_requests WHERE user_id = ?1",
                [user],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let (devices, oldest_device): (i64, Option<i64>) = db.query_row(
                "SELECT COUNT(*), MIN(created_at) FROM devices
                 WHERE user_id = ?1 AND created_at > ?2",
                params![user, now - 3600],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let limited_since = if made >= SELF_PAIRING_PER_HOUR {
                Some(oldest)
            } else if devices >= NEW_DEVICES_PER_HOUR {
                Some(oldest_device)
            } else {
                None
            };
            if let Some(since) = limited_since {
                drop(db);
                let wait = since.map_or(3600, |t| (t + 3600 - now).clamp(1, 3600));
                self.audit("pairing.rate_limited", Some(username), Some(device), "");
                return Err(AuthError::Locked(Duration::from_secs(wait as u64)));
            }
        }
        match self.confirm_password(user, username, device, password) {
            Ok(attempt) => drop(attempt),
            Err(e) => {
                if e == AuthError::Invalid {
                    self.audit("pairing.self_failed", Some(username), Some(device), "");
                }
                return Err(e);
            }
        }
        self.db.lock().unwrap().execute(
            "INSERT INTO pairing_requests (user_id, at) VALUES (?1, ?2)",
            params![user, now],
        )?;
        let code = self.insert_pairing_code(user)?;
        self.audit("pairing.created", Some(username), Some(device), "self");
        Ok(code)
    }

    /// Store a new code (only its hash) for a user, first dropping expired ones.
    fn insert_pairing_code(&self, user: i64) -> Result<String, AuthError> {
        let raw = random_bytes::<10>();
        let code: String = raw
            .iter()
            .map(|b| PAIRING_ALPHABET[(*b as usize) % 32] as char)
            .collect();
        let now = now_secs();
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM pairing_codes WHERE expires_at <= ?1", [now])?;
        db.execute(
            "INSERT INTO pairing_codes (code_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![
                sha256(&code).to_vec(),
                user,
                now + PAIRING_TTL.as_secs() as i64
            ],
        )?;
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
        let presented = fingerprint_of_hash(&h);
        type Row = (i64, i64, i64, Option<i64>, Option<Vec<u8>>);
        let mut db = self.db.lock().unwrap();
        self.stall_write();
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
            tracing::info!(target: "anywhere_gateway::refresh", token = %presented, outcome = "unknown", "refresh outcome");
            return Err(AuthError::Invalid);
        };
        // Refused before anything changes, so the client can try the same token later.
        self.count_refresh(device)?;
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
                prune_refresh_on(&tx, device, now)?;
                tx.commit()?;
                tracing::info!(
                    target: "anywhere_gateway::refresh",
                    device,
                    token = %presented,
                    issued = %fingerprint(&minted.refresh),
                    replacements = children + 1,
                    since_use_s = used_at.map_or(-1, |at| now - at),
                    outcome = "retried",
                    "refresh outcome"
                );
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
            tracing::warn!(
                target: "anywhere_gateway::refresh",
                device,
                token = %presented,
                parent = %parent.as_deref().map(fingerprint_of_hash).unwrap_or_default(),
                replacements = children,
                replacements_used = children_used,
                since_use_s = used_at.map_or(-1, |at| now - at),
                outcome = "reused",
                "refresh outcome"
            );
            drop(tx);
            drop(db);
            self.revoke_device(device)?;
            self.audit("refresh.reused", None, Some(device), "device revoked");
            return Err(AuthError::Reused(device));
        }
        if expires <= now {
            tracing::info!(target: "anywhere_gateway::refresh", device, token = %presented, outcome = "expired", "refresh outcome");
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
        prune_refresh_on(&tx, device, now)?;
        tx.commit()?;
        tracing::info!(
            target: "anywhere_gateway::refresh",
            device,
            token = %presented,
            issued = %fingerprint(&minted.refresh),
            parent = %parent.as_deref().map(fingerprint_of_hash).unwrap_or_default(),
            superseded = superseded.len(),
            outcome = "rotated",
            "refresh outcome"
        );
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
        session_days_on(&self.reader())
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
        self.reader()
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
            .reader()
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
                // Signed out by itself: the audit names it as the device that acted.
                self.revoke(p.device_id, Some((p.user_id, &p.username, p.device_id)))?;
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
        Ok(self.reader().query_row(
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

    /// Check a signed-in user's own password (for account changes) from one of their
    /// devices, rate-limited per user and device with a lock of its own: sign-in failures
    /// (which anyone can cause) don't block it, and nor do wrong guesses made with another
    /// of the user's devices (keyed per user, a stolen access token could lock the owner
    /// out of account changes everywhere for up to a day).
    fn confirm_password(
        &self,
        user: i64,
        username: &str,
        device: i64,
        password: &str,
    ) -> Result<Attempt<'_>, AuthError> {
        let attempt = self
            .begin_attempt(vec![confirm_key(user, device)], None)?
            .by(username);
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
        device: i64,
        password: &str,
    ) -> Result<TotpSetup, AuthError> {
        match self.confirm_password(user, username, device, password) {
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
        device: i64,
        password: &str,
        code: &str,
    ) -> Result<(), AuthError> {
        let attempt = match self.confirm_password(user, username, device, password) {
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
    // Changing one's own password
    // -----------------------------------------------------------------------

    /// A signed-in user changes their own password (`POST /v1/account/password`), from
    /// `device`, at `client` (its address, as for [`Self::login`]).
    ///
    /// - The new password must pass the policy and differ from the current one; both are
    ///   checked first, so a refused request costs no password check and no attempt.
    /// - Needs the current password, and a two-step code when that is on. Wrong ones count
    ///   as failed sign-ins: against this user's account-change lock (shared with two-step
    ///   setup and pairing codes), the client's address (a hard lock that also stops its
    ///   sign-ins) and the username's sign-in lock (soft, as at sign-in). So a stolen access
    ///   token buys no more guesses here than at the sign-in form.
    /// - Outstanding pairing codes the user made are dropped (whoever saw one could still
    ///   sign in with it). The user's other devices stay signed in unless `sign_out_others`:
    ///   each holds its own tokens, not the password. The caller's own device always stays.
    pub fn change_own_password(
        &self,
        req: &OwnPasswordChange<'_>,
        client: &str,
    ) -> Result<PasswordChanged, AuthError> {
        check_password(req.new)?;
        if req.new == req.current {
            return Err(AuthError::BadInput(
                "the new password must be different from the current one",
            ));
        }
        let username = req.username;
        let client_key = format!("client:{client}");
        let attempt = self
            .begin_attempt(
                vec![confirm_key(req.user, req.device), client_key.clone()],
                Some((format!("user:{username}"), client_key)),
            )?
            .by(username);
        // Disabled (or deleted) since the token was issued: refused like a wrong token.
        let hash: String = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT password_hash FROM users WHERE id = ?1 AND disabled = 0",
                [req.user],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(AuthError::Invalid)?;
        if !self.check_password_hash(req.current, &hash)? {
            attempt.failed();
            self.audit(
                "password.change_failed",
                Some(username),
                Some(req.device),
                client,
            );
            return Err(AuthError::Invalid);
        }
        match self.second_factor(req.user, req.code)? {
            SecondFactor::Off | SecondFactor::Accepted => {}
            // The password was right; counted as at sign-in (see `Attempt::needs_code`).
            SecondFactor::Missing => {
                attempt.needs_code();
                self.audit(
                    "password.change_totp_required",
                    Some(username),
                    Some(req.device),
                    client,
                );
                return Err(AuthError::TotpRequired);
            }
            SecondFactor::Wrong => {
                attempt.failed();
                self.audit(
                    "password.change_failed",
                    Some(username),
                    Some(req.device),
                    client,
                );
                return Err(AuthError::Invalid);
            }
        }
        attempt.succeeded();
        let new_hash = hash_password(req.new)?;
        {
            let db = self.db.lock().unwrap();
            // Only over the hash that was checked: of two changes racing with the same
            // current password, one wins and the other is told the password is wrong.
            let n = db.execute(
                "UPDATE users SET password_hash = ?1 WHERE id = ?2 AND password_hash = ?3",
                params![new_hash, req.user, hash],
            )?;
            if n == 0 {
                return Err(AuthError::Invalid);
            }
            db.execute("DELETE FROM pairing_codes WHERE user_id = ?1", [req.user])?;
        }
        // The same event as an admin's reset; the detail says who. Never the password.
        self.audit(
            "user.password_changed",
            Some(username),
            Some(req.device),
            "self",
        );
        let revoked = if req.sign_out_others {
            self.revoke_other_own_devices(req.user, username, req.device)?
        } else {
            Vec::new()
        };
        let others = self
            .own_devices(req.user)?
            .into_iter()
            .filter(|d| d.id != req.device)
            .collect();
        Ok(PasswordChanged { revoked, others })
    }

    /// A user's devices that are still signed in, oldest first.
    pub fn own_devices(&self, user: i64) -> Result<Vec<OwnDevice>, AuthError> {
        let db = self.reader();
        let mut st = db.prepare(
            "SELECT id, name, client_kind, client_platform, created_at, last_seen FROM devices
             WHERE user_id = ?1 AND revoked = 0 ORDER BY id",
        )?;
        let rows = st.query_map([user], |r| {
            let kind: Option<String> = r.get(2)?;
            let platform: Option<String> = r.get(3)?;
            Ok(OwnDevice {
                id: r.get(0)?,
                name: clean_device_name(&r.get::<_, String>(1)?),
                client_kind: kind
                    .as_deref()
                    .and_then(ClientKind::parse)
                    .map(|k| k.as_str().to_string()),
                client_platform: platform.as_deref().and_then(clean_platform),
                created_at: r.get(4)?,
                // Coarse: a user needs "a few minutes ago", not the second.
                last_seen: r.get::<_, Option<i64>>(5)?.map(|t| t - t.rem_euclid(60)),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A user signs out one of their own devices (`DELETE /v1/devices/{id}`) from another,
    /// `from`. Returns false, changing nothing, unless `device_id` is one of `user`'s
    /// signed-in devices: another user's device is indistinguishable from one that doesn't
    /// exist. The device's live sessions are the caller's to end. Audited as
    /// `device.revoked`, detail `self from <from>`.
    pub fn revoke_own_device(
        &self,
        user: i64,
        username: &str,
        device_id: i64,
        from: i64,
    ) -> Result<bool, AuthError> {
        self.revoke(device_id, Some((user, username, from)))
    }

    /// A user signs out all their devices but `keep` (the one asking). Returns the devices
    /// signed out (their live sessions are the caller's to end).
    pub fn revoke_other_own_devices(
        &self,
        user: i64,
        username: &str,
        keep: i64,
    ) -> Result<Vec<i64>, AuthError> {
        let mut revoked = Vec::new();
        for d in self.own_devices(user)? {
            if d.id != keep && self.revoke(d.id, Some((user, username, keep)))? {
                revoked.push(d.id);
            }
        }
        Ok(revoked)
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
        let db = self.reader();
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
        let db = self.reader();
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
        let db = self.reader();
        let mut st = db.prepare(
            "SELECT d.id, u.username, d.name, d.mac, d.created_at, d.last_seen, d.last_addr,
                    d.client_kind, d.client_platform, d.user_agent
             FROM devices d JOIN users u ON u.id = d.user_id WHERE d.revoked = 0 ORDER BY d.id",
        )?;
        let rows = st.query_map([], |r| {
            // Cleaned again on the way out, whatever is stored.
            let addr: Option<String> = r.get(6)?;
            let kind: Option<String> = r.get(7)?;
            let platform: Option<String> = r.get(8)?;
            let ua: Option<String> = r.get(9)?;
            Ok(DeviceInfo {
                id: r.get(0)?,
                username: r.get(1)?,
                name: clean_device_name(&r.get::<_, String>(2)?),
                mac: r.get(3)?,
                created_at: r.get(4)?,
                last_seen: r.get(5)?,
                last_addr: addr
                    .and_then(|a| a.parse::<std::net::IpAddr>().ok())
                    .map(|a| a.to_string()),
                client_kind: kind
                    .as_deref()
                    .and_then(ClientKind::parse)
                    .map(|k| k.as_str().to_string()),
                client_platform: platform.as_deref().and_then(clean_platform),
                user_agent: ua.as_deref().and_then(clean_user_agent),
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

    /// Whether `user` already has a recorded play (under another key) of `track_id` that
    /// started less than `within` seconds before or after `played_at`.
    pub fn play_overlaps(
        &self,
        user: i64,
        key: &str,
        track_id: u64,
        played_at: u64,
        within: u64,
    ) -> Result<bool, AuthError> {
        let found: Option<i64> = self
            .reader()
            .query_row(
                "SELECT 1 FROM plays WHERE user_id = ?1 AND track_id = ?2 AND state = 1
                 AND key != ?3 AND played_at > ?4 - ?5 AND played_at < ?4 + ?5 LIMIT 1",
                params![user, track_id as i64, key, played_at as i64, within as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
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
        let n: i64 = self.reader().query_row(
            "SELECT count(*) FROM plays WHERE user_id = ?1",
            [user],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// A user's id, by name.
    pub fn user_id_by_name(&self, username: &str) -> Result<Option<i64>, AuthError> {
        Ok(self
            .reader()
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
        let db = self.reader();
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
        self.revoke(device_id, None)
    }

    /// Revoke a device; with `owner` (user id and name), only if it is that user's.
    /// `owner`: a user signing out one of their own devices, from another or itself (user
    /// id, name, the device acting); `None` for an admin or the system. A device already
    /// revoked (or missing) is left alone either way.
    fn revoke(&self, device_id: i64, owner: Option<(i64, &str, i64)>) -> Result<bool, AuthError> {
        let n = {
            let db = self.db.lock().unwrap();
            let n = db.execute(
                "UPDATE devices SET revoked = 1, revoked_at = ?2, last_addr = NULL, user_agent = NULL
                 WHERE id = ?1 AND revoked = 0 AND (?3 IS NULL OR user_id = ?3)",
                params![device_id, now_secs(), owner.map(|(id, _, _)| id)],
            )?;
            if n == 0 {
                // Already revoked, or not this user's, or no such device: touch nothing
                // (on the admin path as on the owner's).
                return Ok(false);
            }
            let mac: String =
                db.query_row("SELECT mac FROM devices WHERE id = ?1", [device_id], |r| {
                    r.get(0)
                })?;
            let mut q = self.forget.lock().unwrap();
            if q.len() >= FORGET_QUEUE_CAP {
                q.pop_front();
            }
            q.push_back(mac);
            drop(q);
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
        self.seen.lock().unwrap().remove(&device_id);
        match owner {
            // Signed out by its own user, from one of their devices (or itself): which one.
            Some((_, username, from)) => self.audit(
                "device.revoked",
                Some(username),
                Some(device_id),
                &format!("self from {from}"),
            ),
            None => self.audit("device.revoked", None, Some(device_id), ""),
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
        self.reader()
            .query_row(
                "SELECT 1 FROM devices d JOIN users u ON u.id = d.user_id
                 WHERE d.id = ?1 AND d.revoked = 0 AND u.disabled = 0",
                [device_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// A device was used from `addr` (an authenticated request). Its last address and time
    /// are written at most once per [`SEEN_WRITE_INTERVAL`], so busy clients cost no extra
    /// database writes. Returns whether this call wrote.
    pub fn note_seen(&self, device_id: i64, addr: std::net::IpAddr) -> bool {
        self.note_seen_at(device_id, addr, Instant::now())
    }

    fn note_seen_at(&self, device_id: i64, addr: std::net::IpAddr, now: Instant) -> bool {
        if !self.claim_seen_at(device_id, now) {
            return false;
        }
        self.write_seen(device_id, addr);
        true
    }

    /// The in-memory half of [`Self::note_seen`]: is a write of the device's last address due
    /// (and if so, it is counted as done)? Cheap, so callers on async workers check it inline
    /// and run [`Self::write_seen`] off them only when it says so.
    pub fn claim_seen(&self, device_id: i64) -> bool {
        self.claim_seen_at(device_id, Instant::now())
    }

    fn claim_seen_at(&self, device_id: i64, now: Instant) -> bool {
        {
            let mut seen = self.seen.lock().unwrap();
            if let Some(at) = seen.get(&device_id)
                && now.saturating_duration_since(*at) < SEEN_WRITE_INTERVAL
            {
                return false;
            }
            if seen.len() >= SEEN_CAP && !seen.contains_key(&device_id) {
                seen.retain(|_, at| now.saturating_duration_since(*at) < SEEN_WRITE_INTERVAL);
                if seen.len() >= SEEN_CAP
                    && let Some(k) = seen.iter().min_by_key(|(_, at)| **at).map(|(k, _)| *k)
                {
                    // Still full of busy devices: forget the stalest.
                    seen.remove(&k);
                }
            }
            seen.insert(device_id, now);
        }
        true
    }

    /// The database half of [`Self::note_seen`]: write the device's last address and time.
    pub fn write_seen(&self, device_id: i64, addr: std::net::IpAddr) {
        let db = self.db.lock().unwrap();
        self.stall_write();
        let _ = db.execute(
            "UPDATE devices SET last_seen = ?1, last_addr = ?2 WHERE id = ?3 AND revoked = 0",
            params![now_secs(), addr.to_string(), device_id],
        );
    }

    /// At sign-in and renewal: record where the device is and what it said about itself
    /// (always written; parts the client didn't send are left as they were).
    pub fn record_client(&self, device_id: i64, addr: std::net::IpAddr, info: &ClientInfo) {
        self.seen.lock().unwrap().insert(device_id, Instant::now());
        let db = self.db.lock().unwrap();
        self.stall_write();
        let _ = db.execute(
            "UPDATE devices SET last_seen = ?1, last_addr = ?2,
               client_kind = COALESCE(?3, client_kind),
               client_platform = COALESCE(?4, client_platform),
               user_agent = COALESCE(?5, user_agent)
             WHERE id = ?6 AND revoked = 0",
            params![
                now_secs(),
                addr.to_string(),
                info.kind.map(ClientKind::as_str),
                info.platform.as_deref().and_then(clean_platform),
                info.user_agent.as_deref().and_then(clean_user_agent),
                device_id
            ],
        );
    }

    /// Rename a device. `owner`: only if it belongs to this user (a listener renaming their
    /// own device); `None` for an admin. The name must be 1 to [`MAX_DEVICE_NAME`]
    /// characters and is cleaned like every device name ([`clean_name`]); it must differ
    /// from the user's other devices' names. Returns the name stored. Live access tokens
    /// carry the new name at once.
    pub fn rename_device(
        &self,
        device_id: i64,
        owner: Option<i64>,
        raw: &str,
    ) -> Result<String, AuthError> {
        self.rename_device_if_changed(device_id, owner, raw)
            .map(|(name, _)| name)
    }

    /// [`Accounts::rename_device`], also saying whether the name changed. A rename to the
    /// name the device already has writes nothing (no audit entry, nothing for LMS), so
    /// repeating one costs the server nothing.
    pub fn rename_device_if_changed(
        &self,
        device_id: i64,
        owner: Option<i64>,
        raw: &str,
    ) -> Result<(String, bool), AuthError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AuthError::BadInput("enter a name for the device"));
        }
        if trimmed.chars().count() > MAX_DEVICE_NAME {
            return Err(AuthError::BadInput(
                "device names are at most 64 characters",
            ));
        }
        let name = clean_name(trimmed).ok_or(AuthError::BadInput(
            "use letters, digits and spaces in device names",
        ))?;
        let db = self.db.lock().unwrap();
        let found: Option<(i64, String, String)> = db
            .query_row(
                "SELECT d.user_id, d.name, u.username FROM devices d JOIN users u ON u.id = d.user_id
                 WHERE d.id = ?1 AND d.revoked = 0",
                [device_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (user, current, username) = match (found, owner) {
            (Some((u, c, n)), Some(o)) if u == o => (u, c, n),
            (Some((u, c, n)), None) => (u, c, n),
            _ => return Err(AuthError::BadInput("no such device")),
        };
        if current == name {
            return Ok((name, false));
        }
        let taken: Option<i64> = db
            .query_row(
                "SELECT id FROM devices
                 WHERE user_id = ?1 AND id != ?2 AND revoked = 0 AND lower(name) = lower(?3)",
                params![user, device_id, name],
                |r| r.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(AuthError::BadInput(
                "another of this user's devices already has that name",
            ));
        }
        db.execute(
            "UPDATE devices SET name = ?1 WHERE id = ?2",
            params![name, device_id],
        )?;
        drop(db);
        for e in self.access.lock().unwrap().values_mut() {
            if e.principal.device_id == device_id {
                e.principal.device_name = name.clone();
            }
        }
        // Who renamed it: the owner themselves, or an admin through the admin API.
        let by = if owner.is_some() {
            "by the user"
        } else {
            "by an admin"
        };
        self.audit(
            "device.renamed",
            Some(&username),
            Some(device_id),
            &format!("{name} ({by})"),
        );
        Ok((name, true))
    }

    /// Make the device-bookkeeping and renewal writes (`note_seen`, `record_client`,
    /// `touch_device`, `refresh`) stall for `d` each, holding the database, as a disk that
    /// has stopped answering would (tests only).
    #[doc(hidden)]
    pub fn stall_writes_for_test(&self, d: Duration) {
        self.write_stall_ms
            .store(d.as_millis() as u64, Ordering::Relaxed);
    }

    /// See [`Self::stall_writes_for_test`]: called with the database lock held.
    fn stall_write(&self) {
        let ms = self.write_stall_ms.load(Ordering::Relaxed);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }

    /// Make every outstanding pairing code expired (tests only).
    #[doc(hidden)]
    pub fn expire_pairing_codes_for_test(&self) {
        let _ = self
            .db
            .lock()
            .unwrap()
            .execute("UPDATE pairing_codes SET expires_at = 0", []);
    }

    /// The stored last address of a device, revoked or not (tests only).
    #[doc(hidden)]
    pub fn stored_address_for_test(&self, device_id: i64) -> Option<String> {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT last_addr FROM devices WHERE id = ?1",
                [device_id],
                |r| r.get(0),
            )
            .ok()
            .flatten()
    }

    pub fn touch_device(&self, device_id: i64) {
        let db = self.db.lock().unwrap();
        self.stall_write();
        let _ = db.execute(
            "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
            params![now_secs(), device_id],
        );
    }

    fn own_device(&self, user: i64, device: i64) -> Result<Option<i64>, AuthError> {
        Ok(self
            .reader()
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

    /// Issue a fresh access + refresh pair for a device (signing in, new or again). The
    /// device's unused refresh tokens beyond the newest [`KEEP_UNUSED_REFRESH`] go, with
    /// their access tokens: they belong to sign-ins the client has replaced, and only
    /// expiry removed them, so signing in over and over grew the table without bound.
    fn issue(&self, device: i64) -> Result<Tokens, AuthError> {
        let db = self.db.lock().unwrap();
        let minted = mint(&db, device, now_secs())?;
        let stale: Vec<Vec<u8>> = {
            let mut st = db.prepare(
                "SELECT token_hash FROM refresh_tokens WHERE device_id = ?1 AND used = 0
                 ORDER BY rowid DESC LIMIT -1 OFFSET ?2",
            )?;
            st.query_map(params![device, KEEP_UNUSED_REFRESH], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for h in &stale {
            db.execute("DELETE FROM refresh_tokens WHERE token_hash = ?1", [h])?;
        }
        if !stale.is_empty() {
            self.access
                .lock()
                .unwrap()
                .retain(|_, e| !stale.iter().any(|s| s[..] == e.refresh[..]));
        }
        Ok(self.activate(minted))
    }

    /// Count a renewal by `device`; refused beyond [`REFRESHES_PER_WINDOW`].
    fn count_refresh(&self, device: i64) -> Result<(), AuthError> {
        let mut m = self.refreshes.lock().unwrap();
        let now = Instant::now();
        if m.len() >= LIMITS_CAP {
            m.retain(|_, (start, _)| now.duration_since(*start) < REFRESH_WINDOW);
        }
        let e = m.entry(device).or_insert((now, 0));
        if now.duration_since(e.0) >= REFRESH_WINDOW {
            *e = (now, 0);
        }
        if e.1 >= REFRESHES_PER_WINDOW {
            let wait = (e.0 + REFRESH_WINDOW).saturating_duration_since(now);
            return Err(AuthError::Locked(wait.max(Duration::from_secs(1))));
        }
        e.1 += 1;
        Ok(())
    }

    /// Make minted tokens live: register the access token in memory. A device keeps at
    /// most [`MAX_ACCESS_PER_DEVICE`] live access tokens (the newest).
    fn activate(&self, m: Minted) -> Tokens {
        let mut map = self.access.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, e| e.expires > now);
        let device = m.principal.device_id;
        let mut mine: Vec<([u8; 32], Instant)> = map
            .iter()
            .filter(|(_, e)| e.principal.device_id == device)
            .map(|(k, e)| (*k, e.expires))
            .collect();
        if mine.len() >= MAX_ACCESS_PER_DEVICE {
            mine.sort_by_key(|(_, exp)| *exp);
            for (k, _) in &mine[..=mine.len() - MAX_ACCESS_PER_DEVICE] {
                map.remove(k);
            }
        }
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
            who: None,
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
        let db = self.db.lock().unwrap();
        let ok = db
            .execute(
                "INSERT INTO audit (at, event, username, device_id, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![now_secs(), event, username, device, detail],
            )
            .is_ok();
        // Kept bounded: every AUDIT_PRUNE_EVERY entries, the old and the surplus go.
        if ok && db.last_insert_rowid() % AUDIT_PRUNE_EVERY == 0 {
            let oldest = now_secs() - AUDIT_RETENTION.as_secs() as i64;
            let _ = prune_audit_on(&db, AUDIT_MAX_ROWS, oldest);
            // Expired refresh tokens of devices that never renewed again.
            let _ = db.execute(
                "DELETE FROM refresh_tokens WHERE expires_at <= ?1",
                [now_secs()],
            );
        }
    }

    /// Remove audit entries older than `oldest` (Unix seconds), and all but the newest
    /// `max_rows`. Returns how many were removed.
    pub fn prune_audit(&self, max_rows: i64, oldest: i64) -> Result<usize, AuthError> {
        Ok(prune_audit_on(&self.db.lock().unwrap(), max_rows, oldest)?)
    }

    pub fn audit_log(&self, limit: u32) -> Result<Vec<AuditEntry>, AuthError> {
        let db = self.reader();
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

/// The rate-limit key for a user's account changes from one of their devices.
fn confirm_key(user: i64, device: i64) -> String {
    format!("confirm:{user}:{device}")
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
    .unwrap_or(DEFAULT_SESSION_DAYS)
}

/// Store a new refresh token for an active device (on the caller's connection or
/// transaction) and pick a new access token.
/// Drop `device`'s expired refresh tokens, and all but its newest [`KEEP_USED_REFRESH`] used
/// ones (kept so a copied token presented later is still caught as reuse).
fn prune_refresh_on(db: &Connection, device: i64, now: i64) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM refresh_tokens WHERE device_id = ?1 AND expires_at <= ?2",
        params![device, now],
    )?;
    db.execute(
        "DELETE FROM refresh_tokens WHERE device_id = ?1 AND used = 1 AND token_hash NOT IN (
           SELECT token_hash FROM refresh_tokens WHERE device_id = ?1 AND used = 1
           ORDER BY used_at DESC LIMIT ?2)",
        params![device, KEEP_USED_REFRESH],
    )?;
    Ok(())
}

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
/// Bring the schema up to date in one transaction: a migration that fails half-way leaves
/// the database exactly as it was (SQLite's schema changes and `user_version` are both
/// transactional), to be tried again on the next start.
///
/// Foreign keys are off while it runs (SQLite ignores the switch inside a transaction, so
/// it is set around it): a step that rebuilds a table drops the old one, which with them
/// on would delete every row that refers to it. Such a step checks the references itself.
fn migrate(db: &Connection) -> Result<(), AuthError> {
    let fk: i64 = db.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    db.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let r = (|| {
        let tx = db.unchecked_transaction()?;
        migrate_steps(&tx)?;
        tx.commit()?;
        Ok(())
    })();
    if fk != 0 {
        db.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    r
}

fn migrate_steps(db: &Connection) -> Result<(), AuthError> {
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
    if version < 8 {
        // Shown to admins in the device list: where each device was last seen from (the
        // client address as resolved behind trusted proxies), and what kind of client it
        // is. The address and User-Agent are cleared when the device is revoked.
        db.execute_batch(
            "ALTER TABLE devices ADD COLUMN last_addr TEXT;
             ALTER TABLE devices ADD COLUMN client_kind TEXT;
             ALTER TABLE devices ADD COLUMN client_platform TEXT;
             ALTER TABLE devices ADD COLUMN user_agent TEXT;
             PRAGMA user_version = 8;",
        )?;
    }
    if version < 9 {
        // Pairing codes users create for their own devices: when, per user, for the hourly
        // limit (rows older than an hour are dropped as new ones are made).
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS pairing_requests (
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS pairing_requests_user ON pairing_requests(user_id, at);
             PRAGMA user_version = 9;",
        )?;
    }
    if version < 10 {
        // Wave 1 (docs/plans/wave1-design.md §6–8), per user and never shared:
        // - `history`: what the user played through Lyrion Anywhere (live on their own
        //   devices' players, or offline and sent back), by stable track key, for the Home
        //   shelves and stations. Kept 13 months and at most `history::MAX_ROWS_PER_USER`
        //   rows; see `crate::history`.
        // - `user_prefs`: whether history is recorded (on unless the user turns it off),
        //   the Home layout (validated JSON, see `crate::home`), and whether the built-in
        //   stations have been added for the user.
        // - `user_pins`: stations (built-in kinds and ones pinned from menus) and pinned
        //   menus, each a validated JSON spec (see `crate::stations`). `broken` says why a
        //   pinned menu action is no longer offered.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
               id INTEGER PRIMARY KEY,
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               track_key TEXT NOT NULL,
               track_id INTEGER NOT NULL,
               album_id INTEGER,
               artist_id INTEGER,
               year INTEGER,
               played_at INTEGER NOT NULL,
               seconds REAL NOT NULL,
               source TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS history_user_played ON history(user_id, played_at);
             CREATE INDEX IF NOT EXISTS history_user_key ON history(user_id, track_key);
             CREATE INDEX IF NOT EXISTS history_played ON history(played_at);
             CREATE TABLE IF NOT EXISTS user_prefs (
               user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
               history INTEGER NOT NULL DEFAULT 1,
               home TEXT,
               stations_seeded INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS user_pins (
               id INTEGER PRIMARY KEY,
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               kind TEXT NOT NULL,
               name TEXT NOT NULL,
               spec TEXT NOT NULL,
               broken TEXT,
               created_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS user_pins_user ON user_pins(user_id, kind);
             PRAGMA user_version = 10;",
        )?;
    }
    if version < 11 {
        // Wave 2 ratings (docs/plans/wave2-design.md §2), per user and never shared: one
        // rating (0–100) per stable track key, the newest kept; and how the user rates
        // (stars or a heart). See `crate::ratings`.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_ratings (
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               track_key TEXT NOT NULL,
               track_id INTEGER NOT NULL,
               rating INTEGER NOT NULL,
               rated_at INTEGER NOT NULL,
               PRIMARY KEY (user_id, track_key));",
        )?;
        // Safe to run again (as every migration here): add the column only if it's missing.
        let has_style: bool = db.query_row(
            "SELECT count(*) FROM pragma_table_info('user_prefs') WHERE name = 'rating_style'",
            [],
            |r| r.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_style {
            db.execute_batch(
                "ALTER TABLE user_prefs ADD COLUMN rating_style TEXT NOT NULL DEFAULT 'stars';",
            )?;
        }
        db.execute_batch("PRAGMA user_version = 11;")?;
    }
    if version < 12 {
        // Wave 2 smart filters (docs/plans/wave2-design.md §5.6), per user and never
        // shared: each a name and a validated JSON spec (rules, sort, limit; see
        // `crate::filters`). Safe to run again.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_filters (
               id INTEGER PRIMARY KEY,
               user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
               name TEXT NOT NULL,
               spec TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS user_filters_user ON user_filters(user_id);
             PRAGMA user_version = 12;",
        )?;
    }
    if version < 13 {
        // Device ids are never reused (AUTOINCREMENT). Without it SQLite hands a new
        // device the id of the newest one once that is deleted (a deleted user, a pruned
        // revocation), and what is keyed by the id (learned menu actions, audit rows)
        // would pass to it. SQLite can't add AUTOINCREMENT to a table, so it is rebuilt
        // with every row and id kept; the references to it (refresh tokens, diagnostics)
        // carry over by name. Safe to run again: skipped once the table has it.
        let sql: String = db.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'devices'",
            [],
            |r| r.get(0),
        )?;
        if !sql.contains("AUTOINCREMENT") {
            let fk: i64 = db.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            if fk != 0 {
                // Dropping the old table would cascade to every token and report.
                return Err(AuthError::Storage(
                    "rebuilding devices needs foreign keys off".into(),
                ));
            }
            db.execute_batch(
                "CREATE TABLE devices_new (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                   name TEXT NOT NULL,
                   mac TEXT NOT NULL UNIQUE,
                   created_at INTEGER NOT NULL,
                   last_seen INTEGER,
                   revoked INTEGER NOT NULL DEFAULT 0,
                   revoked_at INTEGER,
                   last_addr TEXT,
                   client_kind TEXT,
                   client_platform TEXT,
                   user_agent TEXT);
                 INSERT INTO devices_new (id, user_id, name, mac, created_at, last_seen,
                   revoked, revoked_at, last_addr, client_kind, client_platform, user_agent)
                 SELECT id, user_id, name, mac, created_at, last_seen,
                   revoked, revoked_at, last_addr, client_kind, client_platform, user_agent
                 FROM devices;
                 DROP TABLE devices;
                 ALTER TABLE devices_new RENAME TO devices;",
            )?;
            for table in ["refresh_tokens", "device_diagnostics"] {
                let exists: i64 = db.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )?;
                if exists == 0 {
                    continue;
                }
                let broken: i64 = db.query_row(
                    "SELECT count(*) FROM pragma_foreign_key_check(?1)",
                    [table],
                    |r| r.get(0),
                )?;
                if broken > 0 {
                    return Err(AuthError::Storage(format!(
                        "rebuilding devices broke references in {table}"
                    )));
                }
            }
        }
        db.execute_batch("PRAGMA user_version = 13;")?;
    }
    Ok(())
}

/// The schema version [`migrate`] brings a database to.
pub const SCHEMA_VERSION: i64 = 13;

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

    #[test]
    fn token_fingerprints_are_short_stable_and_not_the_hash() {
        let f = fingerprint("example-refresh-token");
        assert_eq!(f.len(), 8);
        assert!(f.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(f, fingerprint("example-refresh-token"));
        assert_ne!(f, fingerprint("example-refresh-tokem"));
        // The stored hash is never what is logged, so the log can't be matched against the
        // database's token hashes.
        let stored: String = sha256("example-refresh-token")[..4]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_ne!(f, stored);
        assert_eq!(f, fingerprint_of_hash(&sha256("example-refresh-token")));
    }

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
        // Unset: the default, 90 days.
        assert_eq!(a.session_days(), DEFAULT_SESSION_DAYS);
        assert_eq!(DEFAULT_SESSION_DAYS, 90);
        assert!(SESSION_DAYS.contains(&DEFAULT_SESSION_DAYS));
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        assert_eq!(t.refresh_expires_in, 90 * 86_400);
        a.set_session_days(30).unwrap();
        assert_eq!(a.session_days(), 30);
        // Applies from the next renewal: the lifetime slides with use.
        let t = a.refresh(&t.refresh).unwrap();
        assert_eq!(t.refresh_expires_in, 30 * 86_400);
        for ok in [1, 365] {
            a.set_session_days(ok).unwrap();
            assert_eq!(a.session_days(), ok);
        }
        a.set_session_days(30).unwrap();
        for bad in [0, 366, u32::MAX] {
            assert!(a.set_session_days(bad).is_err());
        }
        assert_eq!(a.session_days(), 30);
        // A bad stored value falls back to the default.
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE settings SET value = 'x' WHERE key = 'session_days'",
                [],
            )
            .unwrap();
        assert_eq!(a.session_days(), DEFAULT_SESSION_DAYS);
        for stored in ["0", "366", "-5", "", "90.5"] {
            a.db.lock()
                .unwrap()
                .execute(
                    "UPDATE settings SET value = ?1 WHERE key = 'session_days'",
                    [stored],
                )
                .unwrap();
            assert_eq!(a.session_days(), DEFAULT_SESSION_DAYS, "{stored:?}");
        }
    }

    #[test]
    fn a_failed_migration_changes_nothing() {
        // At version 10 but without the table migration 11 alters: it fails half-way.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY);
             PRAGMA user_version = 10;",
        )
        .unwrap();
        assert!(migrate(&db).is_err());
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 10);
        let made: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'user_ratings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(made, 0, "the half-done step was rolled back");
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
    fn renewals_are_rate_limited_per_device() {
        // Regression: a refresh token could be renewed in a tight loop, each renewal adding
        // a live access token and a database row.
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let other = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let mut cur = t.refresh.clone();
        for _ in 0..REFRESHES_PER_WINDOW {
            cur = a.refresh(&cur).unwrap().refresh;
        }
        match a.refresh(&cur) {
            Err(AuthError::Locked(d)) => assert!(d <= REFRESH_WINDOW),
            x => panic!("expected a refusal, got {x:?}"),
        }
        // Refused without using the token up, and other devices are unaffected.
        a.refreshes.lock().unwrap().clear();
        assert!(a.refresh(&cur).is_ok());
        assert!(a.refresh(&other.refresh).is_ok());
    }

    #[test]
    fn renewals_keep_state_bounded() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let device = t.principal.device_id;
        let mut cur = t.refresh.clone();
        let mut accesses = vec![t.access.clone()];
        for _ in 0..100 {
            a.refreshes.lock().unwrap().clear();
            let n = a.refresh(&cur).unwrap();
            cur = n.refresh;
            accesses.push(n.access);
        }
        let live = a
            .access
            .lock()
            .unwrap()
            .values()
            .filter(|e| e.principal.device_id == device)
            .count();
        assert_eq!(live, MAX_ACCESS_PER_DEVICE);
        // The newest access tokens work; the oldest are gone.
        assert!(a.authenticate(accesses.last().unwrap()).is_some());
        assert!(a.authenticate(&accesses[0]).is_none());
        let rows: i64 =
            a.db.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM refresh_tokens WHERE device_id = ?1",
                    [device],
                    |r| r.get(0),
                )
                .unwrap();
        assert!(rows <= KEEP_USED_REFRESH + 1, "{rows} rows");
        // The chain still renews.
        assert!(a.refresh(&cur).is_ok());
    }

    #[test]
    fn expired_refresh_rows_are_pruned_on_renewal() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let device = t.principal.device_id;
        a.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO refresh_tokens (token_hash, device_id, expires_at) VALUES (x'01', ?1, 1)",
                [device],
            )
            .unwrap();
        a.refresh(&t.refresh).unwrap();
        let old: i64 =
            a.db.lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM refresh_tokens WHERE token_hash = x'01'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(old, 0);
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

    /// A signed-in user's own device: (user id, device id).
    fn signed_in(a: &Accounts, username: &str) -> (i64, i64) {
        let t = a.login(username, PW, None, "Laptop", None, "c").unwrap();
        (t.principal.user_id, t.principal.device_id)
    }

    fn pairing_code_count(a: &Accounts) -> i64 {
        a.db.lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM pairing_codes", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn users_create_pairing_codes_for_themselves_only() {
        let a = store();
        a.create_user("bob", PW, Role::Listener).unwrap();
        let (alice, laptop) = signed_in(&a, "alice");
        let code = a
            .create_own_pairing_code(alice, "alice", laptop, PW)
            .unwrap();
        assert_eq!(code.len(), 10);
        assert!(code.bytes().all(|b| PAIRING_ALPHABET.contains(&b)));
        // The code signs in as alice, on a new device, and only once.
        let t = a.pair(&code, "Phone", "c").unwrap();
        assert_eq!(t.principal.username, "alice");
        assert_ne!(t.principal.device_id, laptop);
        assert_eq!(a.pair(&code, "Phone", "c"), Err(AuthError::Invalid));
        // Expired codes are refused like any other.
        let code = a
            .create_own_pairing_code(alice, "alice", laptop, PW)
            .unwrap();
        a.db.lock()
            .unwrap()
            .execute("UPDATE pairing_codes SET expires_at = 0", [])
            .unwrap();
        assert_eq!(a.pair(&code, "Phone", "c"), Err(AuthError::Invalid));
        // Audited with who made it and from which device, never with the code.
        let log = a.audit_log(100).unwrap();
        assert!(log.iter().any(|e| e.event == "pairing.created"
            && e.detail == "self"
            && e.username.as_deref() == Some("alice")
            && e.device_id == Some(laptop)));
        assert!(!log.iter().any(|e| e.detail.contains(&code)));
        // Expired codes are cleared out as new ones are made.
        let expired = a
            .create_own_pairing_code(alice, "alice", laptop, PW)
            .unwrap();
        a.db.lock()
            .unwrap()
            .execute("UPDATE pairing_codes SET expires_at = 0", [])
            .unwrap();
        a.create_pairing_code("bob").unwrap();
        assert_eq!(pairing_code_count(&a), 1);
        assert_eq!(a.pair(&expired, "x", "c"), Err(AuthError::Invalid));
    }

    /// Regression guard: an access token alone (15 minutes) must not become a lasting
    /// sign-in on another device.
    #[test]
    fn own_pairing_codes_need_the_password_and_wrong_ones_lock() {
        let a = store();
        let (alice, laptop) = signed_in(&a, "alice");
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                a.create_own_pairing_code(alice, "alice", laptop, "wrong password!"),
                Err(AuthError::Invalid)
            );
        }
        assert!(matches!(
            a.create_own_pairing_code(alice, "alice", laptop, PW),
            Err(AuthError::Locked(_))
        ));
        assert_eq!(pairing_code_count(&a), 0, "no code was made");
        assert!(
            a.audit_log(100)
                .unwrap()
                .iter()
                .any(|e| e.event == "pairing.self_failed")
        );
        // The lock is the account-change one: signing in still works.
        assert!(a.login("alice", PW, None, "x", None, "c2").is_ok());
    }

    #[test]
    fn own_pairing_codes_are_rate_limited_per_user() {
        let a = store();
        a.create_user("bob", PW, Role::Listener).unwrap();
        let (alice, laptop) = signed_in(&a, "alice");
        for _ in 0..SELF_PAIRING_PER_HOUR {
            a.create_own_pairing_code(alice, "alice", laptop, PW)
                .unwrap();
        }
        let checks = a.password_checks();
        match a.create_own_pairing_code(alice, "alice", laptop, PW) {
            Err(AuthError::Locked(wait)) => {
                assert!(wait > Duration::ZERO && wait <= Duration::from_secs(3600))
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(a.password_checks(), checks, "refused before the password");
        assert!(
            a.audit_log(100)
                .unwrap()
                .iter()
                .any(|e| e.event == "pairing.rate_limited")
        );
        // Bob has his own allowance; admins aren't limited.
        let (bob, phone) = signed_in(&a, "bob");
        assert!(a.create_own_pairing_code(bob, "bob", phone, PW).is_ok());
        assert!(a.create_pairing_code("alice").is_ok());
        // An hour later, alice may again.
        a.db.lock()
            .unwrap()
            .execute("UPDATE pairing_requests SET at = at - 3601", [])
            .unwrap();
        assert!(
            a.create_own_pairing_code(alice, "alice", laptop, PW)
                .is_ok()
        );
    }

    #[test]
    fn no_own_pairing_codes_while_at_the_new_device_limit() {
        let a = store();
        let (alice, laptop) = signed_in(&a, "alice");
        for i in 1..NEW_DEVICES_PER_HOUR {
            a.login("alice", PW, None, &format!("d{i}"), None, "c")
                .unwrap();
        }
        assert!(matches!(
            a.create_own_pairing_code(alice, "alice", laptop, PW),
            Err(AuthError::Locked(_))
        ));
        // A code made earlier still meets the same limit when it is redeemed.
        a.db.lock()
            .unwrap()
            .execute("UPDATE devices SET created_at = created_at - 3601", [])
            .unwrap();
        let code = a
            .create_own_pairing_code(alice, "alice", laptop, PW)
            .unwrap();
        for i in 0..NEW_DEVICES_PER_HOUR {
            a.login("alice", PW, None, &format!("e{i}"), None, "c")
                .unwrap();
        }
        assert!(matches!(a.pair(&code, "x", "c"), Err(AuthError::Locked(_))));
    }

    #[test]
    fn own_pairing_codes_die_with_the_user() {
        let a = store();
        let (alice, laptop) = signed_in(&a, "alice");
        let code = a
            .create_own_pairing_code(alice, "alice", laptop, PW)
            .unwrap();
        a.set_disabled("alice", true).unwrap();
        assert_eq!(a.pair(&code, "x", "c"), Err(AuthError::Invalid));
        a.set_disabled("alice", false).unwrap();
        let (alice, laptop) = signed_in(&a, "alice");
        let code = a
            .create_own_pairing_code(alice, "alice", laptop, PW)
            .unwrap();
        a.delete_user("alice").unwrap();
        assert_eq!(a.pair(&code, "x", "c"), Err(AuthError::Invalid));
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
        let setup = a.totp_begin(id, username, 1, PW).unwrap();
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
        a.totp_begin(id, "alice", 1, PW).unwrap();
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
            matches!(
                a.totp_begin(id, "alice", 1, PW),
                Err(AuthError::BadInput(_))
            ),
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
            a.totp_begin(id, "alice", 1, "wrong password!"),
            Err(AuthError::Invalid)
        );
        assert_eq!(a.totp_begin(id, "alice", 1, ""), Err(AuthError::Invalid));
        // Nothing pending: confirming has nothing to confirm.
        assert!(matches!(
            a.totp_confirm(id, "alice", "123456"),
            Err(AuthError::BadInput(_))
        ));
        // Wrong passwords here count towards a lockout of their own.
        for _ in 0..MAX_FAILURES - 2 {
            let _ = a.totp_begin(id, "alice", 1, "wrong password!");
        }
        assert!(matches!(
            a.totp_begin(id, "alice", 1, PW),
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
            a.totp_disable(id, "alice", 1, "wrong password!", &code),
            Err(AuthError::Invalid)
        );
        assert_eq!(
            a.totp_disable(id, "alice", 1, PW, "000000"),
            Err(AuthError::Invalid)
        );
        assert!(a.totp_enabled(id).unwrap());
        a.totp_disable(id, "alice", 1, PW, &code).unwrap();
        assert!(!a.totp_enabled(id).unwrap());
        assert!(a.login("alice", PW, None, "x", None, "c").is_ok());
        assert!(matches!(
            a.totp_disable(id, "alice", 1, PW, &code),
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
        assert_eq!(version, SCHEMA_VERSION);
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

    /// Version 10 (Wave 1: history, Home and stations) upgrades a version-9 database in
    /// place: existing users and devices stay, the new per-user tables are there and
    /// empty, and they go with the user.
    #[test]
    fn migration_to_11_adds_ratings_and_keeps_prefs() {
        let path = std::env::temp_dir().join(format!("anywhere-mig11-{}.db", crate::random_id()));
        {
            let a = Accounts::open(&path).unwrap();
            a.create_user("alice", PW, Role::Listener).unwrap();
            let alice = user_id(&a, "alice");
            // As a version-10 gateway left it, with a saved preference.
            a.with_db(|db| {
                db.execute_batch(
                    "DROP TABLE user_ratings; ALTER TABLE user_prefs DROP COLUMN rating_style;
                     PRAGMA user_version = 10;",
                )?;
                db.execute(
                    "INSERT INTO user_prefs (user_id, history) VALUES (?1, 0)",
                    [alice],
                )?;
                Ok(())
            })
            .unwrap();
            assert_eq!(a.schema_version(), 10);
        }
        let a = Accounts::open(&path).unwrap();
        assert_eq!(a.schema_version(), SCHEMA_VERSION);
        let alice = user_id(&a, "alice");
        assert!(
            !crate::history::enabled(&a, alice).unwrap(),
            "the saved preference survives"
        );
        assert_eq!(crate::ratings::style(&a, alice).unwrap(), "stars");
        assert_eq!(crate::ratings::count(&a, alice).unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    /// Version 12 (smart filters) upgrades a version-11 database in place, is safe to run
    /// again, and filters go with their user.
    #[test]
    fn migration_to_12_adds_filters() {
        let path = std::env::temp_dir().join(format!("anywhere-mig12-{}.db", crate::random_id()));
        {
            let a = Accounts::open(&path).unwrap();
            a.create_user("alice", PW, Role::Listener).unwrap();
            a.with_db(|db| db.execute_batch("DROP TABLE user_filters; PRAGMA user_version = 11;"))
                .unwrap();
            assert_eq!(a.schema_version(), 11);
        }
        let a = Accounts::open(&path).unwrap();
        assert_eq!(a.schema_version(), SCHEMA_VERSION);
        let alice = user_id(&a, "alice");
        assert!(crate::filters::list(&a, alice).unwrap().is_empty());
        // Running the step again (as after a half-finished upgrade) changes nothing.
        a.with_db(|db| {
            db.execute_batch("PRAGMA user_version = 11;")?;
            migrate(db).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(a.schema_version(), SCHEMA_VERSION);
        let d = crate::filters::parse_draft(
            &serde_json::json!({"name": "F", "rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": 5}),
            None,
        )
        .unwrap();
        crate::filters::add(&a, alice, &d).unwrap();
        a.delete_user("alice").unwrap();
        let left: i64 = a
            .with_db(|db| db.query_row("SELECT count(*) FROM user_filters", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(left, 0, "filters go with the user");
        drop(a);
        for ext in ["db", "db-wal", "db-shm"] {
            let _ = std::fs::remove_file(path.with_extension(ext));
        }
    }

    #[test]
    fn migration_to_10_adds_the_per_user_tables() {
        let path = std::env::temp_dir().join(format!("anywhere-mig10-{}.db", crate::random_id()));
        {
            let a = Accounts::open(&path).unwrap();
            a.create_user("alice", PW, Role::Listener).unwrap();
            // Wind the schema back to version 9, as an older gateway left it.
            a.with_db(|db| {
                db.execute_batch(
                    "DROP TABLE history; DROP TABLE user_prefs; DROP TABLE user_pins;
                     PRAGMA user_version = 9;",
                )
            })
            .unwrap();
            assert_eq!(a.schema_version(), 9);
        }
        let a = Accounts::open(&path).unwrap();
        assert_eq!(a.schema_version(), SCHEMA_VERSION);
        let alice = user_id(&a, "alice");
        let count = |table: &str| -> i64 {
            a.with_db(|db| db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
                .unwrap()
        };
        assert_eq!(count("history"), 0);
        assert_eq!(count("user_prefs"), 0);
        assert_eq!(count("user_pins"), 0);
        // The indexes the history queries rely on exist.
        let indexes: Vec<String> = a
            .with_db(|db| {
                let mut st = db.prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'history'",
                )?;
                let rows = st.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .unwrap();
        for want in ["history_user_played", "history_user_key", "history_played"] {
            assert!(
                indexes.iter().any(|i| i == want),
                "{want} missing: {indexes:?}"
            );
        }
        a.with_db(|db| {
            db.execute(
                "INSERT INTO history (user_id, track_key, track_id, played_at, seconds, source)
                 VALUES (?1, ?2, 1, 1, 200, 'live')",
                params![alice, "a".repeat(64)],
            )?;
            db.execute(
                "INSERT INTO user_pins (user_id, kind, name, spec, created_at)
                 VALUES (?1, 'station', 'S', '{}', 1)",
                [alice],
            )
        })
        .unwrap();
        a.delete_user("alice").unwrap();
        assert_eq!(count("history"), 0, "history goes with the user");
        assert_eq!(count("user_pins"), 0, "stations go with the user");
        drop(a);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
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
                record_failure_in(&mut limits, &format!("client:flood{i}"), None, now);
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
        assert_eq!(v, SCHEMA_VERSION);
        a.create_user("alice", PW, Role::Listener).unwrap();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        assert_eq!(a.stored_plays(alice).unwrap(), 0);
    }

    fn device_row(a: &Accounts, id: i64) -> DeviceInfo {
        a.list_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap()
    }

    #[test]
    fn the_device_columns_are_added_to_an_existing_database() {
        // A version 7 database: as before this migration.
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        {
            let db = a.db.lock().unwrap();
            db.execute_batch(
                "ALTER TABLE devices DROP COLUMN last_addr;
                 ALTER TABLE devices DROP COLUMN client_kind;
                 ALTER TABLE devices DROP COLUMN client_platform;
                 ALTER TABLE devices DROP COLUMN user_agent;
                 PRAGMA user_version = 7;",
            )
            .unwrap();
            migrate(&db).unwrap();
            let v: i64 = db
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(v, SCHEMA_VERSION);
            // Migrating again is a no-op.
            migrate(&db).unwrap();
        }
        // Existing devices keep working, with nothing recorded yet.
        let d = device_row(&a, t.principal.device_id);
        assert_eq!(d.name, "Phone");
        assert_eq!(
            (d.last_addr, d.client_kind, d.client_platform, d.user_agent),
            (None, None, None, None)
        );
        let t = a.refresh(&t.refresh).unwrap();
        a.record_client(
            t.principal.device_id,
            "198.51.100.1".parse().unwrap(),
            &ClientInfo {
                kind: Some(ClientKind::Android),
                platform: Some("Pixel 8".into()),
                user_agent: None,
            },
        );
        let d = device_row(&a, t.principal.device_id);
        assert_eq!(d.last_addr.as_deref(), Some("198.51.100.1"));
        assert_eq!(d.client_kind.as_deref(), Some("android"));
        assert_eq!(d.client_platform.as_deref(), Some("Pixel 8"));
    }

    #[test]
    fn device_addresses_are_written_at_most_once_a_minute() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let id = t.principal.device_id;
        let ip1: std::net::IpAddr = "198.51.100.1".parse().unwrap();
        let ip2: std::net::IpAddr = "2001:db8::2".parse().unwrap();
        let start = Instant::now();
        assert!(a.note_seen_at(id, ip1, start));
        assert_eq!(
            device_row(&a, id).last_addr.as_deref(),
            Some("198.51.100.1")
        );
        // Within the interval nothing is written, even from another address.
        assert!(!a.note_seen_at(id, ip2, start + Duration::from_secs(1)));
        assert!(!a.note_seen_at(id, ip2, start + Duration::from_secs(59)));
        assert_eq!(
            device_row(&a, id).last_addr.as_deref(),
            Some("198.51.100.1")
        );
        // After it, the next use writes.
        assert!(a.note_seen_at(id, ip2, start + SEEN_WRITE_INTERVAL));
        assert_eq!(device_row(&a, id).last_addr.as_deref(), Some("2001:db8::2"));
        // Sign-in and renewal always write.
        a.record_client(id, ip1, &ClientInfo::default());
        assert_eq!(
            device_row(&a, id).last_addr.as_deref(),
            Some("198.51.100.1")
        );
    }

    #[test]
    fn the_seen_table_is_bounded() {
        let a = store();
        let ip: std::net::IpAddr = "198.51.100.1".parse().unwrap();
        let now = Instant::now();
        // Device ids that don't exist write nothing to the database but are remembered.
        for id in 0..(SEEN_CAP as i64 + 100) {
            a.note_seen_at(10_000 + id, ip, now);
        }
        assert!(a.seen.lock().unwrap().len() <= SEEN_CAP);
    }

    #[test]
    fn revoked_devices_lose_their_address_and_user_agent() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let id = t.principal.device_id;
        a.record_client(
            id,
            "198.51.100.1".parse().unwrap(),
            &ClientInfo {
                kind: Some(ClientKind::Browser),
                platform: Some("Firefox on Linux".into()),
                user_agent: Some("Mozilla/5.0 (X11; Linux x86_64)".into()),
            },
        );
        assert!(a.revoke_device(id).unwrap());
        assert_eq!(a.stored_address_for_test(id), None);
        let ua: Option<String> =
            a.db.lock()
                .unwrap()
                .query_row("SELECT user_agent FROM devices WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
        assert_eq!(ua, None);
        // A revoked device is never written again.
        a.note_seen_at(
            id,
            "198.51.100.2".parse().unwrap(),
            Instant::now() + SEEN_WRITE_INTERVAL * 2,
        );
        assert_eq!(a.stored_address_for_test(id), None);
    }

    #[test]
    fn client_info_is_kept_until_replaced_and_cleaned() {
        let a = store();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let id = t.principal.device_id;
        let ip: std::net::IpAddr = "198.51.100.1".parse().unwrap();
        a.record_client(
            id,
            ip,
            &ClientInfo {
                kind: Some(ClientKind::Pwa),
                platform: Some("<i>Safari</i> on\u{202e} iPadOS".into()),
                user_agent: Some("Mozilla/5.0 <script>\"x\"</script>\u{7}\u{e9}".into()),
            },
        );
        let d = device_row(&a, id);
        assert_eq!(d.client_kind.as_deref(), Some("pwa"));
        assert_eq!(d.client_platform.as_deref(), Some("iSafarii on iPadOS"));
        assert_eq!(d.user_agent.as_deref(), Some("Mozilla/5.0 scriptx/script"));
        // Nothing sent: kept.
        a.record_client(id, ip, &ClientInfo::default());
        assert_eq!(device_row(&a, id).client_kind.as_deref(), Some("pwa"));
        // Whatever is in the database is cleaned on the way out.
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE devices SET client_kind = 'root', client_platform = '<x>',
                   user_agent = '<>', last_addr = 'not an address', name = '<b>\u{202e}' WHERE id = ?1",
                [id],
            )
            .unwrap();
        let d = device_row(&a, id);
        assert_eq!(d.client_kind, None);
        assert_eq!(d.client_platform.as_deref(), Some("x"));
        assert_eq!(d.user_agent, None);
        assert_eq!(d.last_addr, None);
        assert_eq!(d.name, "b");
    }

    #[test]
    fn client_kinds_parse_strictly() {
        for k in [ClientKind::Pwa, ClientKind::Browser, ClientKind::Android] {
            assert_eq!(ClientKind::parse(k.as_str()), Some(k));
        }
        for bad in ["", "PWA", "ios", " pwa", "pwa ", "android\0"] {
            assert_eq!(ClientKind::parse(bad), None, "{bad:?}");
        }
    }

    /// Small deterministic PRNG (xorshift64*), so failures are reproducible.
    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed >> 12;
        *seed ^= *seed << 25;
        *seed ^= *seed >> 27;
        seed.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    #[test]
    fn platform_and_user_agent_cleaning_never_panics_and_stays_bounded() {
        let pool: Vec<char> =
            "aZ09 .,-_'’()&<>\"`\\/;:=\t\n\r\u{0}\u{7}\u{7f}\u{202e}\u{200b}\u{300}é漢🎵\u{feff}"
                .chars()
                .collect();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            let len = (rng(&mut seed) % 300) as usize;
            let s: String = (0..len)
                .map(|_| pool[(rng(&mut seed) % pool.len() as u64) as usize])
                .collect();
            if let Some(p) = clean_platform(&s) {
                assert!(p.chars().count() <= MAX_CLIENT_PLATFORM);
                assert!(!p.is_empty() && p.trim() == p, "{p:?}");
                assert!(
                    p.chars()
                        .all(|c| !c.is_control() && !"<>\"`\\/;:=".contains(c) && c != '\u{202e}'),
                    "{p:?}"
                );
            }
            if let Some(u) = clean_user_agent(&s) {
                assert!(u.len() <= MAX_USER_AGENT);
                assert!(
                    u.chars()
                        .all(|c| (c.is_ascii_graphic() || c == ' ') && !"<>\"'`\\&".contains(c)),
                    "{u:?}"
                );
                assert!(!u.starts_with(' ') && !u.ends_with(' ') && !u.contains("  "));
            }
        }
        assert_eq!(
            clean_user_agent(&"a".repeat(1000)).unwrap().len(),
            MAX_USER_AGENT
        );
        assert_eq!(clean_user_agent("   "), None);
        assert_eq!(clean_platform("<>"), None);
    }

    #[test]
    fn renaming_devices() {
        let a = store();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let admin = a.user_id_by_name("admin").unwrap().unwrap();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let other = a.login("alice", PW, None, "Tablet", None, "c").unwrap();
        let theirs = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        let id = t.principal.device_id;

        // Success: trimmed, cleaned, and live access tokens carry the new name.
        assert_eq!(
            a.rename_device(id, Some(alice), "  Kitchen \u{202e} iPad ")
                .unwrap(),
            "Kitchen iPad"
        );
        assert_eq!(
            a.authenticate(&t.access).unwrap().device_name,
            "Kitchen iPad"
        );
        assert_eq!(device_row(&a, id).name, "Kitchen iPad");
        // And new tokens too.
        let t = a.refresh(&t.refresh).unwrap();
        assert_eq!(t.principal.device_name, "Kitchen iPad");

        // Empty, blank, too long, nothing usable.
        for bad in ["", "   ", "<>/", "\u{202e}\u{200b}", "\u{0}\u{7}"] {
            assert!(
                matches!(
                    a.rename_device(id, Some(alice), bad),
                    Err(AuthError::BadInput(_))
                ),
                "{bad:?}"
            );
        }
        assert!(a.rename_device(id, Some(alice), &"x".repeat(65)).is_err());
        assert!(a.rename_device(id, Some(alice), &"é".repeat(64)).is_ok());

        // Unique per user (ignoring case), but another user may use it.
        assert!(a.rename_device(id, Some(alice), "tablet").is_err());
        assert!(
            a.rename_device(theirs.principal.device_id, Some(admin), "Tablet")
                .is_ok()
        );
        // A device may keep its own name in another case.
        assert!(
            a.rename_device(other.principal.device_id, Some(alice), "TABLET")
                .is_ok()
        );

        // A listener can't rename another user's device (indistinguishable from none).
        assert_eq!(
            a.rename_device(theirs.principal.device_id, Some(alice), "Mine now"),
            Err(AuthError::BadInput("no such device"))
        );
        assert_eq!(device_row(&a, theirs.principal.device_id).name, "Tablet");
        // An admin (no owner) can rename anyone's.
        assert_eq!(
            a.rename_device(theirs.principal.device_id, None, "Office")
                .unwrap(),
            "Office"
        );
        // Renaming to the current name writes nothing (no audit entry).
        let renames = |a: &Accounts| {
            a.audit_log(1000)
                .unwrap()
                .iter()
                .filter(|e| e.event == "device.renamed")
                .count()
        };
        let before = renames(&a);
        assert_eq!(
            a.rename_device_if_changed(theirs.principal.device_id, None, " Office "),
            Ok(("Office".to_string(), false))
        );
        assert_eq!(renames(&a), before);
        // The audit entry names the device's owner and who renamed it.
        a.rename_device(id, Some(alice), "Den").unwrap();
        a.rename_device(theirs.principal.device_id, None, "Study")
            .unwrap();
        let log = a.audit_log(2).unwrap();
        assert_eq!(log[0].username.as_deref(), Some("admin"));
        assert_eq!(log[0].detail, "Study (by an admin)");
        assert_eq!(log[1].username.as_deref(), Some("alice"));
        assert_eq!(log[1].detail, "Den (by the user)");

        // Unknown and revoked devices.
        assert!(a.rename_device(999_999, None, "X").is_err());
        a.revoke_device(other.principal.device_id).unwrap();
        assert_eq!(
            a.rename_device(other.principal.device_id, None, "X"),
            Err(AuthError::BadInput("no such device"))
        );
    }

    #[test]
    fn the_audit_log_is_bounded_by_age_and_size() {
        let a = store();
        let now = now_secs();
        {
            let db = a.db.lock().unwrap();
            for i in 0..10 {
                db.execute(
                    "INSERT INTO audit (at, event, username, device_id, detail)
                     VALUES (?1, 'old', NULL, NULL, '')",
                    [now - 400 * 24 * 3600 + i],
                )
                .unwrap();
            }
        }
        for _ in 0..20 {
            a.audit("login.failed", None, None, "c");
        }
        // Older than the retention go; then only the newest N stay.
        a.prune_audit(15, now - AUDIT_RETENTION.as_secs() as i64)
            .unwrap();
        let log = a.audit_log(1000).unwrap();
        assert_eq!(log.len(), 15);
        assert!(log.iter().all(|e| e.event != "old"));
    }

    #[test]
    fn writing_the_audit_log_prunes_it() {
        let a = store();
        let now = now_secs();
        a.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO audit (at, event, username, device_id, detail)
                 VALUES (?1, 'ancient', NULL, NULL, '')",
                [now - 2 * AUDIT_RETENTION.as_secs() as i64],
            )
            .unwrap();
        // A flood of failed sign-ins (each audited) triggers the pruning by itself.
        for _ in 0..AUDIT_PRUNE_EVERY {
            a.audit("login.failed", None, None, "c");
        }
        assert!(
            a.audit_log(1000)
                .unwrap()
                .iter()
                .all(|e| e.event != "ancient")
        );
    }

    #[test]
    fn an_abandoned_play_claim_is_taken_over() {
        let a = store();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let dev = t.principal.device_id;
        let now = now_secs() as u64;
        let stale = Duration::from_secs(120);
        assert_eq!(
            a.claim_play(alice, dev, "key-00001", 1, now, stale)
                .unwrap(),
            PlayClaim::Claimed
        );
        // Still in flight: refused.
        assert_eq!(
            a.claim_play(alice, dev, "key-00001", 1, now, stale)
                .unwrap(),
            PlayClaim::InFlight
        );
        // A crash mid-forward left it pending for longer than `stale`: taken over.
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE plays SET created_at = created_at - 121 WHERE key = 'key-00001'",
                [],
            )
            .unwrap();
        assert_eq!(
            a.claim_play(alice, dev, "key-00001", 1, now, stale)
                .unwrap(),
            PlayClaim::Claimed
        );
        a.finish_play(alice, "key-00001").unwrap();
        assert_eq!(
            a.claim_play(alice, dev, "key-00001", 1, now, stale)
                .unwrap(),
            PlayClaim::Recorded
        );
    }

    #[test]
    fn overlapping_plays_are_found_per_user_and_track() {
        let a = store();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let dev = t.principal.device_id;
        let stale = Duration::from_secs(120);
        let at = now_secs() as u64 - 3600;
        a.claim_play(alice, dev, "key-00001", 1, at, stale).unwrap();
        // Only recorded plays count.
        assert!(!a.play_overlaps(alice, "key-00002", 1, at, 121).unwrap());
        a.finish_play(alice, "key-00001").unwrap();
        assert!(
            a.play_overlaps(alice, "key-00002", 1, at + 120, 121)
                .unwrap()
        );
        assert!(
            a.play_overlaps(alice, "key-00002", 1, at - 120, 121)
                .unwrap()
        );
        assert!(
            !a.play_overlaps(alice, "key-00002", 1, at + 121, 121)
                .unwrap()
        );
        assert!(
            !a.play_overlaps(alice, "key-00002", 2, at, 121).unwrap(),
            "another track"
        );
        assert!(
            !a.play_overlaps(alice, "key-00001", 1, at, 121).unwrap(),
            "its own key"
        );
        let bob = a.user_id_by_name("admin").unwrap().unwrap();
        assert!(
            !a.play_overlaps(bob, "key-00002", 1, at, 121).unwrap(),
            "another user"
        );
    }

    // -----------------------------------------------------------------------
    // Signing out one's own devices
    // -----------------------------------------------------------------------

    #[test]
    fn a_user_signs_out_only_their_own_devices() {
        let a = store();
        let mine = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let phone = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let theirs = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        let alice = mine.principal.user_id;
        let admin_device = theirs.principal.device_id;

        // Someone else's device: refused, and nothing of theirs is touched (its refresh
        // token still renews, its access token still works).
        assert!(
            !a.revoke_own_device(alice, "alice", admin_device, 1)
                .unwrap()
        );
        assert!(a.authenticate(&theirs.access).is_some());
        assert!(a.device_active(admin_device));
        let renewed = a.refresh(&theirs.refresh).unwrap();
        // Nor one that doesn't exist.
        assert!(!a.revoke_own_device(alice, "alice", 999_999, 1).unwrap());
        assert!(!a.revoke_own_device(alice, "alice", -1, 1).unwrap());

        // Their own: signed out, tokens dead.
        assert!(
            a.revoke_own_device(
                alice,
                "alice",
                phone.principal.device_id,
                mine.principal.device_id
            )
            .unwrap()
        );
        assert!(a.authenticate(&phone.access).is_none());
        assert!(a.refresh(&phone.refresh).is_err());
        // Only once.
        assert!(
            !a.revoke_own_device(
                alice,
                "alice",
                phone.principal.device_id,
                mine.principal.device_id
            )
            .unwrap()
        );
        assert!(a.authenticate(&mine.access).is_some());
        assert!(a.authenticate(&renewed.access).is_some());

        let e = a
            .audit_log(100)
            .unwrap()
            .into_iter()
            .find(|e| e.event == "device.revoked")
            .unwrap();
        assert_eq!(e.username.as_deref(), Some("alice"));
        assert_eq!(e.device_id, Some(phone.principal.device_id));
        assert_eq!(e.detail, format!("self from {}", mine.principal.device_id));
    }

    #[test]
    fn signing_out_other_devices_keeps_the_callers() {
        let a = store();
        let keep = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let x = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let y = a.login("alice", PW, None, "Tablet", None, "c").unwrap();
        let other = a.login("admin", PW, None, "Desk", None, "c").unwrap();
        let (user, me) = (keep.principal.user_id, keep.principal.device_id);
        let mut out = a.revoke_other_own_devices(user, "alice", me).unwrap();
        out.sort();
        assert_eq!(out, vec![x.principal.device_id, y.principal.device_id]);
        assert!(a.authenticate(&keep.access).is_some());
        assert!(a.authenticate(&x.access).is_none() && a.authenticate(&y.access).is_none());
        assert!(a.authenticate(&other.access).is_some());
        let left = a.own_devices(user).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, me);
        // Nothing left to do.
        assert!(
            a.revoke_other_own_devices(user, "alice", me)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn own_devices_say_when_and_only_to_the_minute() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        a.db.lock()
            .unwrap()
            .execute(
                "UPDATE devices SET last_seen = 1000059, created_at = 999 WHERE id = ?1",
                [t.principal.device_id],
            )
            .unwrap();
        let d = &a.own_devices(t.principal.user_id).unwrap()[0];
        assert_eq!(d.created_at, 999);
        assert_eq!(d.last_seen, Some(1000020));
    }

    // -----------------------------------------------------------------------
    // Authentication lows from the October review
    // -----------------------------------------------------------------------

    /// Regression: a sign-in from an address removed its whole record, so an attacker
    /// holding any valid account could reset the address's cross-username lock every four
    /// guesses. A success now clears only that user's own failures.
    #[test]
    fn a_success_clears_only_the_users_own_failures_from_the_address() {
        let a = store();
        a.create_user("mallory", PW, Role::Listener).unwrap();
        for i in 0..MAX_FAILURES - 1 {
            assert_eq!(
                a.login(
                    &format!("guess{i}"),
                    "whatever!!",
                    None,
                    "x",
                    None,
                    "attacker"
                ),
                Err(AuthError::Invalid)
            );
        }
        // The attacker's own account signs in from the same address...
        assert!(a.login("mallory", PW, None, "x", None, "attacker").is_ok());
        // ...and one more guess still locks the address, for every username.
        assert_eq!(
            a.login("guess-last", "whatever!!", None, "x", None, "attacker"),
            Err(AuthError::Invalid)
        );
        assert!(matches!(
            a.login("mallory", PW, None, "x", None, "attacker"),
            Err(AuthError::Locked(_))
        ));

        // A user's own mistakes are still forgiven by their own success.
        for _ in 0..MAX_FAILURES - 1 {
            let _ = a.login("alice", "wrong password!", None, "x", None, "home");
        }
        assert!(a.login("alice", PW, None, "x", None, "home").is_ok());
        for _ in 0..MAX_FAILURES - 1 {
            let _ = a.login("alice", "wrong password!", None, "x", None, "home");
        }
        assert!(a.login("alice", PW, None, "x", None, "home").is_ok());
    }

    /// Regression: the right password without a code (two-step on) was neither counted nor
    /// audited, so whoever knew the password could have it checked without limit, holding
    /// the password-check permits.
    #[test]
    fn a_missing_code_counts_as_a_failure_and_is_audited() {
        let a = store();
        enrol(&a, "alice");
        let before = a.password_checks();
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                a.login("alice", PW, None, "x", None, "prober"),
                Err(AuthError::TotpRequired)
            );
        }
        assert!(matches!(
            a.login("alice", PW, None, "x", None, "prober"),
            Err(AuthError::Locked(_))
        ));
        assert_eq!(a.password_checks() - before, u64::from(MAX_FAILURES));
        let audited = a
            .audit_log(100)
            .unwrap()
            .into_iter()
            .filter(|e| e.event == "login.totp_required" && e.detail == "prober")
            .count();
        assert_eq!(audited, MAX_FAILURES as usize);

        // The usual two-step sign-in (asked for the code, then giving it) still works, and
        // leaves nothing counted against the address.
        assert_eq!(
            a.login("alice", PW, None, "x", None, "phone"),
            Err(AuthError::TotpRequired)
        );
        let code = current_code(&a, "alice", 1);
        assert!(
            a.login("alice", PW, Some(&code), "x", None, "phone")
                .is_ok()
        );
        assert!(
            a.limits
                .lock()
                .unwrap()
                .get("client:phone")
                .is_none_or(|l| l.failures == 0)
        );
    }

    /// Counting the missing code must not let someone else's wrong guesses (which throttle
    /// the username) lock the owner out of the usual two-step sign-in from a clean address.
    #[test]
    fn a_throttled_two_step_user_still_signs_in_from_elsewhere() {
        let a = store();
        enrol(&a, "alice");
        for i in 0..MAX_FAILURES {
            let _ = a.login(
                "alice",
                "wrong password!",
                None,
                "x",
                None,
                &format!("bot{i}"),
            );
        }
        assert_eq!(
            a.login("alice", PW, None, "x", None, "home"),
            Err(AuthError::TotpRequired)
        );
        let code = current_code(&a, "alice", 1);
        let t = a.login("alice", PW, Some(&code), "x", None, "home");
        assert!(t.is_ok(), "{t:?}");
    }

    /// The same for a password change without its code.
    #[test]
    fn a_password_change_without_its_code_counts_and_is_audited() {
        let a = store();
        enrol(&a, "alice");
        let code = current_code(&a, "alice", 1);
        let t = a
            .login("alice", PW, Some(&code), "Laptop", None, "c")
            .unwrap();
        let req = OwnPasswordChange {
            user: t.principal.user_id,
            username: "alice",
            device: t.principal.device_id,
            current: PW,
            new: "a much better passphrase",
            code: None,
            sign_out_others: false,
        };
        for _ in 0..MAX_FAILURES {
            assert_eq!(
                a.change_own_password(&req, "c").map(|_| ()),
                Err(AuthError::TotpRequired)
            );
        }
        assert!(matches!(
            a.change_own_password(&req, "c"),
            Err(AuthError::Locked(_))
        ));
        let audited = a
            .audit_log(100)
            .unwrap()
            .into_iter()
            .filter(|e| e.event == "password.change_totp_required")
            .count();
        assert_eq!(audited, MAX_FAILURES as usize);
    }

    /// Regression: account changes were locked per user, so anyone holding one of the
    /// user's access tokens (15 minutes) could lock the owner out of password change,
    /// two-step changes and pairing for up to a day. The lock is now per user and device.
    #[test]
    fn one_device_cannot_lock_the_users_other_devices_out_of_account_changes() {
        let a = store();
        let (alice, laptop) = signed_in(&a, "alice");
        let (_, phone) = signed_in(&a, "alice");
        let wrong = OwnPasswordChange {
            user: alice,
            username: "alice",
            device: laptop,
            current: "wrong password!",
            new: "a much better passphrase",
            code: None,
            sign_out_others: false,
        };
        for _ in 0..3 * MAX_FAILURES {
            let _ = a.change_own_password(&wrong, "198.51.100.66");
        }
        // The device that failed is locked out of account changes...
        assert!(matches!(
            a.create_own_pairing_code(alice, "alice", laptop, PW),
            Err(AuthError::Locked(_))
        ));
        // ...the owner's other device is not.
        assert!(a.create_own_pairing_code(alice, "alice", phone, PW).is_ok());
    }

    fn refresh_rows(a: &Accounts, device: i64) -> i64 {
        a.db.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM refresh_tokens WHERE device_id = ?1",
                [device],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Regression: each sign-in reusing a device added an unused refresh row that only
    /// expiry removed, so the table grew without bound.
    #[test]
    fn signing_in_again_on_a_device_keeps_its_refresh_rows_bounded() {
        let a = store();
        let first = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let d = first.principal.device_id;
        let mut last = first;
        for _ in 0..20 {
            last = a.login("alice", PW, None, "Laptop", Some(d), "c").unwrap();
            assert_eq!(last.principal.device_id, d);
        }
        let rows = refresh_rows(&a, d);
        assert!(rows <= KEEP_UNUSED_REFRESH, "{rows} rows");
        // The newest sign-in's token still renews.
        assert!(a.refresh(&last.refresh).is_ok());
    }

    /// Regression: device ids were reused once the newest device was deleted, so a new
    /// device could inherit what was keyed by the old one's id (learned menu actions,
    /// audit rows).
    #[test]
    fn device_ids_are_never_reused() {
        let a = store();
        a.create_user("bob", PW, Role::Listener).unwrap();
        let bob = a.login("bob", PW, None, "Phone", None, "c").unwrap();
        let gone = bob.principal.device_id;
        a.delete_user("bob").unwrap();
        let next = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        assert!(next.principal.device_id > gone);
    }

    fn devices_autoincrement(a: &Accounts) -> bool {
        a.with_db(|db| {
            db.query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'devices'",
                [],
                |r| r.get::<_, String>(0),
            )
        })
        .unwrap()
        .contains("AUTOINCREMENT")
    }

    /// Version 13 rebuilds the devices table so its ids are never reused, keeping every
    /// device, its refresh tokens and its diagnostics; it is safe to run again.
    #[test]
    fn migration_to_13_keeps_devices_and_stops_id_reuse() {
        let path = std::env::temp_dir().join(format!("anywhere-mig13-{}.db", crate::random_id()));
        let (laptop, newest, refresh) = {
            let a = Accounts::open(&path).unwrap();
            a.create_user("alice", PW, Role::Listener).unwrap();
            let laptop = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
            let newest = a.login("alice", PW, None, "Phone", None, "c").unwrap();
            a.save_diagnostics(laptop.principal.device_id, "{}")
                .unwrap();
            // As a version-12 gateway left it: no AUTOINCREMENT on devices.
            a.with_db(|db| {
                db.execute_batch(
                    "PRAGMA foreign_keys = OFF;
                     CREATE TABLE devices_v12 (
                       id INTEGER PRIMARY KEY,
                       user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                       name TEXT NOT NULL,
                       mac TEXT NOT NULL UNIQUE,
                       created_at INTEGER NOT NULL,
                       last_seen INTEGER,
                       revoked INTEGER NOT NULL DEFAULT 0,
                       revoked_at INTEGER,
                       last_addr TEXT,
                       client_kind TEXT,
                       client_platform TEXT,
                       user_agent TEXT);
                     INSERT INTO devices_v12 SELECT * FROM devices;
                     DROP TABLE devices;
                     ALTER TABLE devices_v12 RENAME TO devices;
                     PRAGMA foreign_keys = ON;
                     PRAGMA user_version = 12;",
                )
            })
            .unwrap();
            assert_eq!(a.schema_version(), 12);
            assert!(!devices_autoincrement(&a));
            (
                laptop.principal.device_id,
                newest.principal.device_id,
                laptop.refresh,
            )
        };
        let a = Accounts::open(&path).unwrap();
        assert_eq!(a.schema_version(), SCHEMA_VERSION);
        assert!(devices_autoincrement(&a));
        assert_eq!(device_row(&a, laptop).name, "Laptop");
        assert!(a.diagnostics(laptop).unwrap().is_some(), "diagnostics kept");
        let renewed = a.refresh(&refresh);
        assert!(renewed.is_ok(), "refresh tokens kept: {renewed:?}");
        // Running the step again (as after a half-finished upgrade) changes nothing.
        a.with_db(|db| {
            db.execute_batch("PRAGMA user_version = 12;")?;
            migrate(db).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(a.schema_version(), SCHEMA_VERSION);
        assert!(a.refresh(&renewed.unwrap().refresh).is_ok());
        // The newest device deleted, the next one gets a new id.
        a.with_db(|db| db.execute("DELETE FROM devices WHERE id = ?1", [newest]))
            .unwrap();
        let next = a.login("alice", PW, None, "Tablet", None, "c").unwrap();
        assert!(next.principal.device_id > newest);
        drop(a);
        for ext in ["db", "db-wal", "db-shm"] {
            let _ = std::fs::remove_file(path.with_extension(ext));
        }
    }

    /// The device that signed itself out is named, as when a user signs out another device.
    #[test]
    fn signing_out_records_the_device_that_did_it() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        a.logout(&t.access).unwrap();
        let e = a
            .audit_log(100)
            .unwrap()
            .into_iter()
            .find(|e| e.event == "device.revoked")
            .unwrap();
        assert_eq!(e.username.as_deref(), Some("alice"));
        assert_eq!(e.device_id, Some(t.principal.device_id));
        assert_eq!(e.detail, format!("self from {}", t.principal.device_id));
    }

    /// Regression: revoking a device that was already revoked (or never existed) still
    /// deleted rows for its id on the admin path; only the owner path returned early.
    #[test]
    fn revoking_a_revoked_device_again_touches_nothing() {
        let a = store();
        let t = a.login("alice", PW, None, "Laptop", None, "c").unwrap();
        let d = t.principal.device_id;
        assert!(a.revoke_device(d).unwrap());
        // Something recorded for the id since (a report racing the revocation).
        a.save_diagnostics(d, "{}").unwrap();
        assert!(!a.revoke_device(d).unwrap());
        assert!(a.diagnostics(d).unwrap().is_some());
        let revocations = a
            .audit_log(100)
            .unwrap()
            .into_iter()
            .filter(|e| e.event == "device.revoked")
            .count();
        assert_eq!(revocations, 1);
    }

    // -----------------------------------------------------------------------
    // The read connection (review M7, follow-up)
    // -----------------------------------------------------------------------

    /// A database file in the temp folder, removed (with its WAL files) when dropped.
    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new(tag: &str) -> TempDb {
            TempDb(std::env::temp_dir().join(format!("anywhere-{tag}-{}.db", crate::random_id())))
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            for ext in ["", "-wal", "-shm"] {
                let mut p = self.0.clone().into_os_string();
                p.push(ext);
                let _ = std::fs::remove_file(p);
            }
        }
    }

    fn on_disk(f: &TempDb) -> Accounts {
        let a = Accounts::open(&f.0).unwrap();
        a.create_user("alice", PW, Role::Listener).unwrap();
        a
    }

    #[test]
    fn the_read_connection_sees_each_write_once_it_commits() {
        let f = TempDb::new("reader");
        let a = on_disk(&f);
        assert!(a.has_read_connection());
        // A user, a device and a grant, each read back at once through the reader.
        a.create_user("bob", PW, Role::Listener).unwrap();
        assert!(a.list_users().unwrap().iter().any(|u| u.username == "bob"));
        assert_eq!(a.user_id_by_name("bob").unwrap(), Some(user_id(&a, "bob")));
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let p = t.principal.clone();
        assert!(a.device_active(p.device_id));
        assert!(a.player_scope(&p).unwrap().allows(&p.mac_string()));
        a.grant_player("alice", "00:04:20:00:00:01").unwrap();
        assert!(a.player_scope(&p).unwrap().allows("00:04:20:00:00:01"));
        a.set_session_days(30).unwrap();
        assert_eq!(a.session_days(), 30);
        // Security: a revocation is seen by the very next permission check.
        assert!(a.revoke_device(p.device_id).unwrap());
        assert!(!a.device_active(p.device_id));
        assert!(!a.player_scope(&p).unwrap().allows(&p.mac_string()));
        assert!(a.own_devices(p.user_id).unwrap().is_empty());
    }

    /// Under WAL the reader neither waits for a write in progress nor sees it before it
    /// commits.
    #[test]
    fn reads_do_not_wait_for_a_write_in_progress() {
        let f = TempDb::new("reader-wait");
        let a = on_disk(&f);
        let count = |a: &Accounts| {
            a.read_db(|db| db.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0)))
                .unwrap()
        };
        let (started_tx, started) = std::sync::mpsc::channel();
        let (commit, commit_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|s| {
            let a = &a;
            let writer = s.spawn(move || {
                a.with_db(|db| {
                    db.execute_batch(
                        "BEGIN IMMEDIATE;
                         INSERT INTO users (username, password_hash, role, created_at)
                         VALUES ('carol', 'x', 'listener', 0);",
                    )?;
                    started_tx.send(()).unwrap();
                    // Holds the write lock (and the writer's mutex) until told to commit.
                    commit_rx.recv().unwrap();
                    db.execute_batch("COMMIT")
                })
                .unwrap()
            });
            started.recv().unwrap();
            let t = Instant::now();
            assert_eq!(count(a), 1, "an uncommitted write is not visible");
            assert!(a.user_id_by_name("carol").unwrap().is_none());
            assert!(
                t.elapsed() < Duration::from_millis(500),
                "{:?}",
                t.elapsed()
            );
            commit.send(()).unwrap();
            writer.join().unwrap();
        });
        assert_eq!(count(&a), 2, "and visible once committed");
    }

    /// Security: the read connection can't be used to write, even by SQL that switches its
    /// `query_only` guard off (it is opened read-only as well).
    #[test]
    fn a_write_through_the_read_connection_is_refused() {
        let f = TempDb::new("reader-ro");
        let a = on_disk(&f);
        let write = "INSERT INTO settings (key, value) VALUES ('stray', '1')";
        assert!(a.read_db(|db| db.execute(write, [])).is_err());
        assert!(
            a.read_db(|db| db.execute("UPDATE users SET role = 'admin'", []))
                .is_err()
        );
        assert!(
            a.read_db(|db| {
                db.execute_batch("PRAGMA query_only = 0")?;
                db.execute(write, [])
            })
            .is_err()
        );
        assert!(
            a.read_db(|db| db.execute_batch("DROP TABLE audit"))
                .is_err()
        );
        // Nothing changed, and the writer still works.
        let stray: Option<String> = a
            .with_db(|db| {
                db.query_row("SELECT value FROM settings WHERE key = 'stray'", [], |r| {
                    r.get(0)
                })
                .optional()
            })
            .unwrap();
        assert_eq!(stray, None);
        assert_eq!(a.list_users().unwrap()[0].role, "listener");
        assert!(a.with_db(|db| db.execute(write, [])).is_ok());
        assert_eq!(
            a.read_db(|db| db.query_row(
                "SELECT value FROM settings WHERE key = 'stray'",
                [],
                |r| r.get::<_, String>(0)
            ))
            .unwrap(),
            "1"
        );
    }

    /// An in-memory database has no second connection (a second one would be a different,
    /// empty database): reads go through the writer and see what it wrote.
    #[test]
    fn in_memory_databases_read_what_they_wrote() {
        let a = store();
        assert!(!a.has_read_connection());
        let t = a.login("alice", PW, None, "Phone", None, "c").unwrap();
        let p = t.principal.clone();
        assert!(a.player_scope(&p).unwrap().allows(&p.mac_string()));
        assert_eq!(a.own_devices(p.user_id).unwrap().len(), 1);
        a.set_session_days(7).unwrap();
        assert_eq!(a.session_days(), 7);
        assert!(a.revoke_device(p.device_id).unwrap());
        assert!(!a.player_scope(&p).unwrap().allows(&p.mac_string()));
        // On this one connection a write is not refused: read_db is a promise, not a guard,
        // and on disk the guard is the read-only connection.
        assert!(
            a.read_db(|db| db.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0)))
                .unwrap()
                >= 1
        );
        // Each in-memory store is its own database.
        let other = Accounts::in_memory().unwrap();
        assert!(other.list_users().unwrap().is_empty());
    }

    /// The reader opens after the migrations, so it sees the current schema of a database
    /// an older gateway left.
    #[test]
    fn the_read_connection_sees_the_migrated_schema() {
        let f = TempDb::new("reader-schema");
        {
            let a = on_disk(&f);
            a.with_db(|db| db.execute_batch("DROP TABLE user_filters; PRAGMA user_version = 11;"))
                .unwrap();
        }
        let a = Accounts::open(&f.0).unwrap();
        let alice = user_id(&a, "alice");
        assert_eq!(crate::filters::list(&a, alice).unwrap().len(), 0);
        assert_eq!(
            a.read_db(|db| db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))
                .unwrap(),
            SCHEMA_VERSION
        );
    }
}
