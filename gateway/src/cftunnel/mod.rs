//! The built-in Cloudflare Tunnel (docs/plans/cloudflare-tunnel.md, "Build contract").
//!
//! The gateway can install the official `cloudflared` (a pinned release, checked against the
//! SHA-256 compiled into this build), register it with a tunnel token or create a tunnel
//! through Cloudflare's API, and supervise it: `cloudflared` dials out to Cloudflare, which
//! then serves `https://<hostname>` from its edge with no open port, certificate or proxy at
//! home. A temporary *quick* tunnel (a random `trycloudflare.com` address, no account) can
//! be tried for an hour.
//!
//! Nothing is downloaded or started until an admin endpoint asks. While a tunnel runs, the
//! gateway's settings follow it ([`Overrides`]): `https://<hostname>` is an accepted origin
//! and the client address is read from `CF-Connecting-IP` on requests from the tunnel's
//! source (loopback, or a non-loopback listen address trusted as a proxy for the duration).
//! They revert when it stops.
//!
//! Data, all under `<data>/tunnel/` (0700): `cloudflared` (0700), `token` (0600, the only
//! secret kept), `state.json` (hostname, installed version, whether the registered tunnel
//! runs at start-up) and `pid` (the running child, so a stale one left by a killed gateway is
//! stopped at the next start, once `/proc/<pid>/exe` shows it is our binary). Nothing about
//! the temporary tunnel is stored (its child's `pid` only while it runs).

pub mod api;
pub mod download;
pub mod pins;
pub mod scrub;
pub mod token;

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use download::UrlPolicy;
use scrub::scrub;

/// Lines of `cloudflared` output kept.
pub const LOG_LINES: usize = 200;
/// Lines of it in the status.
pub const STATUS_LOG_LINES: usize = 50;
/// Longest `cloudflared --version` may take.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
/// `cloudflared`'s environment's `PATH`.
const CHILD_PATH: &str = "/usr/bin:/bin";

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Monotonic time for the tunnel's uptime, the backoff reset and the temporary tunnel's hour.
/// Injected so a test can move it on.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> Duration;
}

/// The real clock.
#[derive(Debug)]
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// A clock a test moves on by hand (it also runs as real time does).
#[derive(Debug)]
pub struct ManualClock {
    base: Instant,
    offset_ms: AtomicU64,
}

impl ManualClock {
    pub fn new() -> Arc<ManualClock> {
        Arc::new(ManualClock {
            base: Instant::now(),
            offset_ms: AtomicU64::new(0),
        })
    }

    pub fn advance(&self, by: Duration) {
        self.offset_ms
            .fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        self.base.elapsed() + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// How the tunnel is set up. [`Settings::default`] has it off, as `--no-tunnel` does (the
/// gateway's command line turns it on under `<data-dir>/tunnel`).
#[derive(Debug, Clone)]
pub struct Settings {
    /// Off: every POST answers 409 and the state is `unsupported`.
    pub enabled: bool,
    /// `<data>/tunnel`.
    pub dir: Option<PathBuf>,
    /// A system-installed `cloudflared` (absolute path) used instead of a download.
    pub system_binary: Option<PathBuf>,
    /// The gateway's public listener: the ingress target.
    pub listen: SocketAddr,
    /// `std::env::consts::OS` and `ARCH` (tests may pretend).
    pub os: String,
    pub arch: String,
    /// The release installed, and its asset's SHA-256 (`None`: from [`pins`]).
    pub pinned_version: String,
    pub pinned_sha256: Option<String>,
    /// Where release assets are (`<base>/<version>/<asset>`).
    pub asset_base: String,
    /// Cloudflare's API.
    pub api_base: String,
    /// Tests only: also allow plain HTTP to exactly this origin (the fake asset server and
    /// API). Never set from the command line.
    pub test_origin: Option<String>,
    pub max_download: u64,
    pub download_timeout: Duration,
    /// How often the readiness endpoint is polled.
    pub poll_interval: Duration,
    /// Restart backoff after an exit: `backoff_base` doubling up to `backoff_max`, back to
    /// the start after `backoff_reset` up.
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    pub backoff_reset: Duration,
    /// How long the temporary tunnel runs.
    pub quick_lifetime: Duration,
    /// How long a stopping `cloudflared` has after SIGTERM before it is killed.
    pub stop_grace: Duration,
    pub clock: Arc<dyn Clock>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            dir: None,
            system_binary: None,
            listen: SocketAddr::from((Ipv4Addr::LOCALHOST, 9443)),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            pinned_version: pins::CLOUDFLARED_VERSION.into(),
            pinned_sha256: None,
            asset_base: download::RELEASES.into(),
            api_base: api::API_BASE.into(),
            test_origin: None,
            max_download: download::MAX_BYTES,
            download_timeout: download::TIME_LIMIT,
            poll_interval: Duration::from_secs(5),
            backoff_base: Duration::from_secs(2),
            backoff_max: Duration::from_secs(60),
            backoff_reset: Duration::from_secs(600),
            quick_lifetime: Duration::from_secs(3600),
            stop_grace: Duration::from_secs(5),
            clock: Arc::new(SystemClock::default()),
        }
    }
}

/// The restart delay after `attempt` exits in a row (0-based): 2, 4, 8 … 60 s.
pub fn backoff(attempt: u32, base: Duration, max: Duration) -> Duration {
    base.checked_mul(1u32 << attempt.min(16))
        .unwrap_or(max)
        .min(max)
}

/// The address `cloudflared` reaches the gateway on: the listen address, with an
/// unspecified one (`0.0.0.0`, `::`) as loopback.
pub fn ingress_target(listen: SocketAddr) -> SocketAddr {
    let ip = match listen.ip() {
        IpAddr::V4(v) if v.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v) if v.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, listen.port())
}

// ---------------------------------------------------------------------------
// The settings that follow the tunnel
// ---------------------------------------------------------------------------

/// The gateway's settings while a tunnel runs: an extra accepted origin, and the tunnel's
/// source trusted for `CF-Connecting-IP`. Empty (the default) when none runs. Shared between
/// [`crate::Config`] (which the request checks read) and the [`Tunnel`].
#[derive(Clone, Default)]
pub struct Overrides(Arc<RwLock<Option<Active>>>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Active {
    /// `https://<hostname>`.
    origin: String,
    /// A non-loopback listen address: where the tunnel's requests come from.
    proxy: Option<IpAddr>,
}

impl std::fmt::Debug for Overrides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Overrides").field(&*self.read()).finish()
    }
}

impl Overrides {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Option<Active>> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set(&self, hostname: &str, listen: SocketAddr) {
        let ip = crate::canonical(ingress_target(listen).ip());
        let active = Active {
            origin: format!("https://{hostname}"),
            proxy: (!ip.is_loopback()).then_some(ip),
        };
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(active);
    }

    pub(crate) fn clear(&self) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Is a tunnel running (so the overrides apply)?
    pub fn active(&self) -> bool {
        self.read().is_some()
    }

    /// The running tunnel's `https://<hostname>`.
    pub fn origin(&self) -> Option<String> {
        self.read().as_ref().map(|a| a.origin.clone())
    }

    /// Is this exactly the running tunnel's origin?
    pub fn origin_allowed(&self, origin: &str) -> bool {
        self.read().as_ref().is_some_and(|a| a.origin == origin)
    }

    /// Is `ip` the running tunnel's non-loopback source (trusted as a proxy meanwhile)?
    pub fn trusted_proxy(&self, ip: IpAddr) -> bool {
        let ip = crate::canonical(ip);
        self.read().as_ref().is_some_and(|a| a.proxy == Some(ip))
    }

    /// Does a request from `ip` come from the running tunnel (loopback or its source)?
    pub fn from_tunnel(&self, ip: IpAddr) -> bool {
        let ip = crate::canonical(ip);
        self.read()
            .as_ref()
            .is_some_and(|a| ip.is_loopback() || a.proxy == Some(ip))
    }
}

/// The header the tunnel's requests carry the client's address in.
pub const CLIENT_IP_HEADER: &str = "cf-connecting-ip";

// ---------------------------------------------------------------------------
// The state machine
// ---------------------------------------------------------------------------

/// What kind of tunnel the status is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    None,
    Registered,
    Quick,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::None => "none",
            Mode::Registered => "registered",
            Mode::Quick => "quick",
        }
    }
}

/// The status's `state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Unsupported,
    NotInstalled,
    Installing,
    Installed,
    Creating,
    Starting,
    Connected,
    Reconnecting,
    Stopped,
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Unsupported => "unsupported",
            State::NotInstalled => "not_installed",
            State::Installing => "installing",
            State::Installed => "installed",
            State::Creating => "creating",
            State::Starting => "starting",
            State::Connected => "connected",
            State::Reconnecting => "reconnecting",
            State::Stopped => "stopped",
            State::Failed => "failed",
        }
    }
}

/// Work in progress that stops other actions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Op {
    #[default]
    Idle,
    Installing {
        bytes: u64,
        total: Option<u64>,
    },
    Creating(api::Step),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Started (or restarted by the admin), no edge connection yet.
    Starting,
    Connected(u32),
    /// Running (or waiting to restart after an exit) without an edge connection.
    Reconnecting,
}

/// One supervised tunnel, from start to stop.
#[derive(Debug, Clone)]
struct Run {
    id: u64,
    mode: Mode,
    phase: Phase,
    /// When the current child started (`None` between an exit and the restart).
    child_since: Option<Duration>,
    /// When the run began (the temporary tunnel's hour counts from here).
    since: Duration,
    /// The temporary tunnel's address, once known.
    quick_host: Option<String>,
}

/// Everything the status is made from. Each transition is a method; [`Machine::state`]
/// reads the state off it.
#[derive(Debug, Default)]
struct Machine {
    installed: Option<String>,
    /// The registered tunnel's hostname (its token is on disk).
    hostname: Option<String>,
    run_at_start: bool,
    /// The admin stopped the registered tunnel (in this gateway's life).
    stopped: bool,
    op: Op,
    error: Option<String>,
    run: Option<Run>,
    /// What to start once the download in progress has finished.
    after_install: Option<Mode>,
}

impl Machine {
    fn state(&self, supported: bool) -> State {
        if !supported {
            return State::Unsupported;
        }
        match self.op {
            Op::Installing { .. } => return State::Installing,
            Op::Creating(_) => return State::Creating,
            Op::Idle => {}
        }
        if let Some(run) = &self.run {
            return match run.phase {
                Phase::Starting => State::Starting,
                Phase::Connected(_) => State::Connected,
                Phase::Reconnecting => State::Reconnecting,
            };
        }
        if self.error.is_some() {
            return State::Failed;
        }
        if self.installed.is_none() {
            return State::NotInstalled;
        }
        if self.hostname.is_some() && self.stopped {
            return State::Stopped;
        }
        State::Installed
    }

    fn busy(&self) -> bool {
        self.op != Op::Idle
    }

    fn running(&self) -> Option<Mode> {
        self.run.as_ref().map(|r| r.mode)
    }

    fn run_mut(&mut self, id: u64) -> Option<&mut Run> {
        self.run.as_mut().filter(|r| r.id == id)
    }

    fn begin_install(&mut self, then: Option<Mode>) {
        self.op = Op::Installing {
            bytes: 0,
            total: None,
        };
        self.error = None;
        self.after_install = then;
    }

    fn install_progress(&mut self, bytes: u64, total: Option<u64>) {
        if matches!(self.op, Op::Installing { .. }) {
            self.op = Op::Installing { bytes, total };
        }
    }

    /// The download finished: what to start next, if anything.
    fn install_finished(&mut self, result: Result<String, String>) -> Option<Mode> {
        self.op = Op::Idle;
        let then = self.after_install.take();
        match result {
            Ok(v) => {
                self.installed = Some(v);
                then
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    fn creating(&mut self, step: api::Step) {
        self.op = Op::Creating(step);
        self.error = None;
    }

    fn create_failed(&mut self, error: String) {
        self.op = Op::Idle;
        self.error = Some(error);
    }

    fn registered(&mut self, hostname: String) {
        self.op = Op::Idle;
        self.hostname = Some(hostname);
        self.run_at_start = true;
        self.stopped = false;
        self.error = None;
    }

    fn run_began(&mut self, id: u64, mode: Mode, now: Duration) {
        self.error = None;
        if mode == Mode::Registered {
            self.stopped = false;
            self.run_at_start = true;
        }
        self.run = Some(Run {
            id,
            mode,
            phase: Phase::Starting,
            child_since: None,
            since: now,
            quick_host: None,
        });
    }

    fn child_spawned(&mut self, id: u64, now: Duration) {
        if let Some(r) = self.run_mut(id) {
            r.child_since = Some(now);
        }
    }

    /// A readiness answer: `Some(n)` edge connections, or not ready.
    fn ready(&mut self, id: u64, connections: Option<u32>) {
        if let Some(r) = self.run_mut(id) {
            r.phase = match (connections, r.phase) {
                (Some(n), _) => Phase::Connected(n),
                (None, Phase::Starting) => Phase::Starting,
                (None, _) => Phase::Reconnecting,
            };
        }
    }

    fn child_exited(&mut self, id: u64) {
        if let Some(r) = self.run_mut(id) {
            r.phase = Phase::Reconnecting;
            r.child_since = None;
            r.quick_host = None;
        }
    }

    /// The temporary tunnel's address became known: `true` if it is new.
    fn quick_host(&mut self, id: u64, host: String) -> bool {
        match self.run_mut(id) {
            Some(r) if r.mode == Mode::Quick && r.quick_host.as_deref() != Some(&host) => {
                r.quick_host = Some(host);
                true
            }
            _ => false,
        }
    }

    /// The run ended (stopped, timed out, or could not start): `true` if it was this run.
    fn run_ended(&mut self, id: u64) -> bool {
        if self.run.as_ref().is_some_and(|r| r.id == id) {
            self.run = None;
            true
        } else {
            false
        }
    }

    fn spawn_failed(&mut self, id: u64, error: String) {
        if self.run_ended(id) {
            self.error = Some(error);
        }
    }

    fn admin_stopped(&mut self, was: Option<Mode>) {
        self.after_install = None;
        if was == Some(Mode::Registered) || (was.is_none() && self.hostname.is_some()) {
            self.run_at_start = false;
            self.stopped = was == Some(Mode::Registered) || self.stopped;
        }
        if self.error.is_some() && was.is_none() {
            self.error = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Parsers of cloudflared's answers and output (untrusted bytes)
// ---------------------------------------------------------------------------

/// `GET /ready`: 200 with `readyConnections` ≥ 1 means connected (that many edge
/// connections); anything else, not.
pub fn parse_ready(status: u16, body: &[u8]) -> Option<u32> {
    if status != 200 {
        return None;
    }
    let v: Value = serde_json::from_slice(body).ok()?;
    let n = v.get("readyConnections")?.as_u64()?;
    (n >= 1).then(|| n.min(1000) as u32)
}

/// A temporary tunnel's hostname: `<words>.trycloudflare.com`.
pub fn valid_quick_host(h: &str) -> bool {
    let Some(words) = h.strip_suffix(".trycloudflare.com") else {
        return false;
    };
    h.len() <= 253
        && !words.is_empty()
        && words.split('.').all(|l| {
            (1..=63).contains(&l.len())
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

/// `GET /quicktunnel`: `{"hostname": "<words>.trycloudflare.com"}`.
pub fn parse_quicktunnel(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let h = v.get("hostname")?.as_str()?.to_ascii_lowercase();
    valid_quick_host(&h).then_some(h)
}

/// The `https://<words>.trycloudflare.com` address in a line of output, if any.
pub fn quick_url_in_line(line: &str) -> Option<String> {
    let mut rest = line;
    while let Some(i) = rest.find("https://") {
        let after = &rest[i + 8..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .unwrap_or(after.len());
        let host = after[..end].to_ascii_lowercase();
        if valid_quick_host(&host) {
            return Some(host);
        }
        rest = after;
    }
    None
}

/// Is `link` (what `/proc/<pid>/exe` points to) `ours`, allowing the ` (deleted)` the
/// kernel adds once the file has been replaced or removed?
pub fn is_our_exe(link: &Path, ours: &Path) -> bool {
    if link == ours {
        return true;
    }
    link.to_str()
        .and_then(|s| s.strip_suffix(" (deleted)"))
        .is_some_and(|s| Path::new(s) == ours)
}

// ---------------------------------------------------------------------------
// The tunnel
// ---------------------------------------------------------------------------

/// Why an admin request was refused: the HTTP status (400, 409 or 500) and a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub message: String,
}

fn refuse(status: u16, message: impl Into<String>) -> Refusal {
    Refusal {
        status,
        message: message.into(),
    }
}

/// What an accepted request did: started something in the background (202), or had
/// nothing to do / finished at once (204).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    Started,
    Done,
}

const BUSY: &str = "The tunnel is busy installing cloudflared or setting up a tunnel; try again when that has finished.";
const NOT_REGISTERED: &str =
    "No tunnel is registered yet: register a tunnel token, or let the gateway create a tunnel.";
const REGISTERED_RUNNING: &str =
    "A registered tunnel is running: stop it before trying a temporary tunnel.";

struct RunHandle {
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

/// The tunnel and its supervisor (one per gateway; [`crate::Gateway::tunnel`]).
pub struct Tunnel {
    s: Settings,
    overrides: Overrides,
    /// The asset and its pinned sum, if this platform has one.
    asset: Option<(&'static str, String)>,
    m: Mutex<Machine>,
    ring: Mutex<VecDeque<String>>,
    /// Secrets masked in every line kept (the tunnel token while it runs, the API token
    /// while the exchange runs).
    secrets: Mutex<Vec<String>>,
    /// Serialises the admin actions; holds the running supervisor.
    control: tokio::sync::Mutex<Option<RunHandle>>,
    ids: AtomicU64,
    /// GitHub and Cloudflare (rustls, ring, bundled roots; HTTPS only outside tests).
    web: reqwest::Client,
    /// `cloudflared`'s loopback metrics endpoint.
    local: reqwest::Client,
}

impl Tunnel {
    pub fn new(s: Settings, overrides: Overrides) -> Arc<Tunnel> {
        let asset = download::asset_for(&s.os, &s.arch).map(|(name, sum)| {
            (
                name,
                s.pinned_sha256.clone().unwrap_or_else(|| sum.to_string()),
            )
        });
        let web = crate::http_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(s.test_origin.is_none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .no_proxy()
            .build()
            .expect("http client");
        let local = crate::http_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .no_proxy()
            .build()
            .expect("http client");
        Arc::new(Tunnel {
            s,
            overrides,
            asset,
            m: Mutex::default(),
            ring: Mutex::new(VecDeque::with_capacity(LOG_LINES)),
            secrets: Mutex::default(),
            control: tokio::sync::Mutex::new(None),
            ids: AtomicU64::new(1),
            web,
            local,
        })
    }

    /// The settings that follow the tunnel.
    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    /// Can a tunnel run here at all (turned on, a data folder, a supported platform)?
    pub fn supported(&self) -> bool {
        self.s.enabled
            && self.s.dir.is_some()
            && self.s.os == "linux"
            && (self.s.system_binary.is_some() || self.asset.is_some())
    }

    fn m(&self) -> std::sync::MutexGuard<'_, Machine> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dir(&self) -> &Path {
        self.s.dir.as_deref().unwrap_or(Path::new("/nonexistent"))
    }

    /// The `cloudflared` run: the system one, or ours.
    pub fn binary(&self) -> PathBuf {
        match &self.s.system_binary {
            Some(p) => p.clone(),
            None => self.dir().join("cloudflared"),
        }
    }

    fn token_path(&self) -> PathBuf {
        self.dir().join("token")
    }

    fn state_path(&self) -> PathBuf {
        self.dir().join("state.json")
    }

    fn pid_path(&self) -> PathBuf {
        self.dir().join("pid")
    }

    fn check_supported(&self) -> Result<(), Refusal> {
        if self.supported() {
            return Ok(());
        }
        Err(refuse(
            409,
            if self.s.enabled {
                "The built-in Cloudflare Tunnel is not available on this system (it needs Linux on x86_64, aarch64 or armhf)."
            } else {
                "The built-in Cloudflare Tunnel is turned off on this gateway (--no-tunnel)."
            },
        ))
    }

    // -- the log ---------------------------------------------------------

    /// Add a line to the log ring, scrubbed.
    fn push_log(&self, line: &str) {
        let line = {
            let secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
            let refs: Vec<&str> = secrets.iter().map(String::as_str).collect();
            scrub::scrub_with(line, &refs)
        };
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if ring.len() >= LOG_LINES {
            ring.pop_front();
        }
        ring.push_back(line);
    }

    /// A line of our own: in the ring and in the gateway's log, both scrubbed.
    fn note(&self, line: &str) {
        self.push_log(line);
        tracing::info!(target: "anywhere_gateway::cftunnel", "{}", scrub(line));
    }

    fn add_secret(&self, s: &str) {
        self.secrets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(s.to_string());
    }

    fn drop_secret(&self, s: &str) {
        self.secrets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|x| x != s);
    }

    /// Every scrubbed line kept, oldest first.
    pub fn log(&self) -> Vec<String> {
        self.ring
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    // -- status ----------------------------------------------------------

    /// `GET /admin/v1/tunnel`: every key always present.
    pub fn status(&self) -> Value {
        let supported = self.supported();
        let now = self.s.clock.now();
        let m = self.m();
        let state = m.state(supported);
        let run = m.run.as_ref().filter(|_| supported);
        let quick = run.filter(|r| r.mode == Mode::Quick);
        let mode = if !supported {
            Mode::None
        } else if quick.is_some() {
            Mode::Quick
        } else if m.hostname.is_some() {
            Mode::Registered
        } else {
            Mode::None
        };
        let hostname = match mode {
            Mode::Quick => quick.and_then(|r| r.quick_host.clone()),
            Mode::Registered => m.hostname.clone(),
            Mode::None => None,
        };
        let installed = m.installed.clone().filter(|_| supported);
        let progress = match m.op {
            Op::Installing { bytes, total } if supported => json!({"bytes": bytes, "total": total}),
            _ => Value::Null,
        };
        let step = match m.op {
            Op::Creating(step) if supported => json!(step.as_str()),
            _ => Value::Null,
        };
        let connections = match run.map(|r| r.phase) {
            Some(Phase::Connected(n)) => n,
            _ => 0,
        };
        let uptime = run
            .and_then(|r| r.child_since)
            .map(|t| now.saturating_sub(t).as_secs());
        let quick_ends_in = quick.map(|r| {
            self.s
                .quick_lifetime
                .saturating_sub(now.saturating_sub(r.since))
                .as_secs()
        });
        let error = m.error.clone().filter(|_| state == State::Failed);
        let update_available = installed
            .as_ref()
            .is_some_and(|v| self.s.system_binary.is_none() && *v != self.s.pinned_version);
        drop(m);
        let log = self.log();
        let tail: Vec<&String> = log
            .iter()
            .skip(log.len().saturating_sub(STATUS_LOG_LINES))
            .collect();
        json!({
            "supported": supported,
            "pinned_version": self.s.pinned_version,
            "installed_version": installed,
            "update_available": update_available,
            "mode": mode.as_str(),
            "state": state.as_str(),
            "hostname": hostname,
            "public_url": hostname.as_ref().map(|h| format!("https://{h}")),
            "connections": connections,
            "uptime": uptime,
            "quick_ends_in": quick_ends_in,
            "progress": progress,
            "step": step,
            "error": error,
            "log": tail,
        })
    }

    // -- files -------------------------------------------------------------

    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.dir())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(self.dir(), std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// Write a file atomically, mode 0600.
    fn write_private(&self, path: &Path, contents: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        self.ensure_dir()?;
        let tmp = self.dir().join(format!(".write-{}", crate::random_id()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let result = (|| {
            let mut f = opts.open(&tmp)?;
            f.write_all(contents)?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)?;
            if let Ok(d) = std::fs::File::open(self.dir()) {
                let _ = d.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// `state.json` from the machine as it is now.
    fn persist(&self) -> Result<(), Refusal> {
        let body = {
            let m = self.m();
            json!({
                "hostname": m.hostname,
                "installed_version": if self.s.system_binary.is_some() { None } else { m.installed.clone() },
                "run_at_start": m.run_at_start,
            })
        };
        self.write_private(&self.state_path(), body.to_string().as_bytes())
            .map_err(|e| {
                refuse(
                    500,
                    format!("The gateway could not write to its data folder ({e})."),
                )
            })
    }

    fn write_pid(&self, pid: Option<u32>) {
        if let Some(pid) = pid
            && let Err(e) = self.write_private(&self.pid_path(), pid.to_string().as_bytes())
        {
            tracing::warn!(error = %scrub(&e.to_string()), "could not record cloudflared's pid");
        }
    }

    fn remove_pid(&self) {
        let _ = std::fs::remove_file(self.pid_path());
    }

    // -- cloudflared -------------------------------------------------------

    fn command(&self) -> tokio::process::Command {
        command_for(&self.binary(), self.dir())
    }

    /// `cloudflared --version` (10 s): the version it reports.
    async fn run_version(&self, bin: &Path) -> Option<String> {
        use tokio::io::AsyncReadExt;
        let mut cmd = command_for(bin, self.dir());
        cmd.arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().ok()?;
        let mut out = child.stdout.take()?;
        let run = async {
            let mut buf = Vec::new();
            let _ = (&mut out).take(4096).read_to_end(&mut buf).await;
            let status = child.wait().await.ok()?;
            status
                .success()
                .then(|| download::parse_version_output(&String::from_utf8_lossy(&buf)))
                .flatten()
        };
        tokio::time::timeout(VERSION_TIMEOUT, run)
            .await
            .ok()
            .flatten()
    }

    /// The fixed argument list.
    pub fn args(&self, mode: Mode, metrics_port: u16) -> Vec<String> {
        let mut a: Vec<String> = [
            "tunnel",
            "--no-autoupdate",
            "--metrics",
            &format!("127.0.0.1:{metrics_port}"),
            "--loglevel",
            "info",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        match mode {
            Mode::Quick => {
                a.push("--url".into());
                a.push(format!("http://{}", ingress_target(self.s.listen)));
            }
            _ => a.push("run".into()),
        }
        a
    }

    /// Download the pinned release, check it runs, and record it.
    async fn download_and_check(&self) -> Result<String, String> {
        let Some((asset, sum)) = self.asset.clone() else {
            return Err("There is no cloudflared release for this system.".into());
        };
        self.ensure_dir()
            .map_err(|e| format!("The gateway could not write to its data folder ({e})."))?;
        let job = download::Fetch {
            url: format!(
                "{}/{}/{asset}",
                self.s.asset_base.trim_end_matches('/'),
                self.s.pinned_version
            ),
            sha256: sum,
            max_bytes: self.s.max_download,
            time_limit: self.s.download_timeout,
            policy: match &self.s.test_origin {
                Some(o) => UrlPolicy::TestOrigin(o.clone()),
                None => UrlPolicy::GitHub,
            },
        };
        self.note(&format!(
            "Downloading cloudflared {} ({asset}) from GitHub.",
            self.s.pinned_version
        ));
        let path = download::fetch(&self.web, &job, self.dir(), "cloudflared", |b, t| {
            self.m().install_progress(b, t)
        })
        .await
        .map_err(|e| e.to_string())?;
        let Some(version) = self.run_version(&path).await else {
            let _ = std::fs::remove_file(&path);
            return Err(
                "The downloaded cloudflared does not run on this system, so it was removed.".into(),
            );
        };
        self.note(&format!("Installed cloudflared {version}."));
        Ok(version)
    }

    fn spawn_install(self: &Arc<Self>) {
        let t = self.clone();
        tokio::spawn(async move {
            let r = t.download_and_check().await;
            if let Err(e) = &r {
                t.note(e);
            }
            let ok = r.is_ok();
            let then = t.m().install_finished(r);
            if ok && let Err(e) = t.persist() {
                tracing::warn!(error = %scrub(&e.message), "could not record the tunnel's state");
            }
            if let Some(mode) = then {
                let mut control = t.control.lock().await;
                // Stopped, removed or started otherwise meanwhile: nothing to do.
                let still_wanted = {
                    let m = t.m();
                    m.running().is_none()
                        && !m.busy()
                        && (mode == Mode::Quick || m.hostname.is_some())
                };
                if still_wanted {
                    t.start_locked(&mut control, mode).await;
                }
            }
        });
    }

    fn installed(&self) -> bool {
        self.m().installed.is_some() && self.binary().is_file()
    }

    /// Start a tunnel (the control lock held): install first if needed.
    async fn start_locked(self: &Arc<Self>, control: &mut Option<RunHandle>, mode: Mode) {
        if !self.installed() {
            if self.s.system_binary.is_some() {
                let msg = format!(
                    "The cloudflared at {} could not be run.",
                    self.binary().display()
                );
                self.note(&msg);
                self.m().error = Some(msg);
                return;
            }
            self.m().begin_install(Some(mode));
            self.spawn_install();
            return;
        }
        let token = if mode == Mode::Registered {
            match std::fs::read_to_string(self.token_path()) {
                Ok(t) if token::valid_tunnel_token(t.trim()) => Some(t.trim().to_string()),
                _ => {
                    let msg =
                        "The stored tunnel token could not be read: register the tunnel again.";
                    self.note(msg);
                    self.m().error = Some(msg.into());
                    return;
                }
            }
        } else {
            None
        };
        let id = self.ids.fetch_add(1, Ordering::SeqCst);
        let hostname = {
            let mut m = self.m();
            m.run_began(id, mode, self.s.clock.now());
            m.hostname.clone()
        };
        if mode == Mode::Registered {
            if let Some(h) = hostname {
                self.overrides.set(&h, self.s.listen);
            }
            if let Err(e) = self.persist() {
                tracing::warn!(error = %scrub(&e.message), "could not record the tunnel's state");
            }
        }
        let (tx, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(self.clone().supervise(id, mode, token, rx));
        // A previous run's handle (already finished) is replaced.
        *control = Some(RunHandle { stop: tx, task });
    }

    /// Stop the running tunnel, if any (the control lock held). Returns what it was.
    async fn stop_locked(&self, control: &mut Option<RunHandle>) -> Option<Mode> {
        let was = self.m().running();
        if let Some(h) = control.take() {
            let _ = h.stop.send(true);
            let mut task = h.task;
            let grace = self.s.stop_grace + Duration::from_secs(5);
            if tokio::time::timeout(grace, &mut task).await.is_err() {
                task.abort();
            }
        }
        // The supervisor clears its run on the way out; make sure.
        let id = self.m().run.as_ref().map(|r| r.id);
        if let Some(id) = id {
            self.end_run(id);
        }
        was
    }

    /// A run is over: forget it and revert the settings.
    fn end_run(&self, id: u64) {
        if self.m().run_ended(id) {
            self.overrides.clear();
        }
    }

    /// The supervisor: start `cloudflared`, watch it, restart it after an exit, stop it when
    /// told to (or after the temporary tunnel's hour).
    async fn supervise(
        self: Arc<Self>,
        id: u64,
        mode: Mode,
        token: Option<String>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        if let Some(t) = &token {
            self.add_secret(t);
        }
        let mut attempt: u32 = 0;
        loop {
            let port = match std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .and_then(|l| l.local_addr())
            {
                Ok(a) => a.port(),
                Err(e) => {
                    let msg = format!("No loopback port was free for cloudflared's metrics ({e}).");
                    self.note(&msg);
                    self.m().spawn_failed(id, msg);
                    self.overrides.clear();
                    break;
                }
            };
            let mut cmd = self.command();
            cmd.args(self.args(mode, port))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(t) = &token {
                cmd.env("TUNNEL_TOKEN", t);
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!(
                        "cloudflared could not be started ({}).",
                        scrub(&e.to_string())
                    );
                    self.note(&msg);
                    self.m().spawn_failed(id, msg);
                    self.overrides.clear();
                    break;
                }
            };
            self.write_pid(child.id());
            let started = self.s.clock.now();
            self.m().child_spawned(id, started);
            self.note(&format!(
                "Started cloudflared ({} tunnel, metrics on 127.0.0.1:{port}).",
                if mode == Mode::Quick {
                    "temporary"
                } else {
                    "registered"
                }
            ));
            if let Some(out) = child.stdout.take() {
                tokio::spawn(self.clone().read_output(id, mode, out));
            }
            if let Some(err) = child.stderr.take() {
                tokio::spawn(self.clone().read_output(id, mode, err));
            }
            let exited = loop {
                tokio::select! {
                    _ = stop.changed() => {
                        self.graceful_stop(&mut child).await;
                        break None;
                    }
                    st = child.wait() => break Some(st),
                    _ = tokio::time::sleep(self.s.poll_interval) => {
                        if self.quick_expired(id, mode) {
                            self.note("The temporary tunnel was stopped after one hour.");
                            self.graceful_stop(&mut child).await;
                            break None;
                        }
                        self.poll(id, mode, port).await;
                    }
                }
            };
            self.remove_pid();
            let Some(status) = exited else {
                break;
            };
            if self.s.clock.now().saturating_sub(started) >= self.s.backoff_reset {
                attempt = 0;
            }
            let delay = backoff(attempt, self.s.backoff_base, self.s.backoff_max);
            attempt = attempt.saturating_add(1);
            let status = match status {
                Ok(s) => s.to_string(),
                Err(e) => e.to_string(),
            };
            self.note(&format!(
                "cloudflared stopped unexpectedly ({status}); starting it again in {} s.",
                delay.as_secs().max(1)
            ));
            self.m().child_exited(id);
            if mode == Mode::Quick {
                // The next one gets a new address.
                self.overrides.clear();
            }
            if !self.wait_to_restart(id, mode, delay, &mut stop).await {
                break;
            }
        }
        if let Some(t) = &token {
            self.drop_secret(t);
        }
        self.end_run(id);
    }

    /// Sleep before a restart; `false` if told to stop (or the hour ran out) meanwhile.
    async fn wait_to_restart(
        &self,
        id: u64,
        mode: Mode,
        delay: Duration,
        stop: &mut tokio::sync::watch::Receiver<bool>,
    ) -> bool {
        let until = tokio::time::Instant::now() + delay;
        loop {
            if self.quick_expired(id, mode) {
                self.note("The temporary tunnel was stopped after one hour.");
                return false;
            }
            let now = tokio::time::Instant::now();
            if now >= until {
                return true;
            }
            let nap = (until - now).min(self.s.poll_interval);
            tokio::select! {
                _ = stop.changed() => return false,
                _ = tokio::time::sleep(nap) => {}
            }
        }
    }

    fn quick_expired(&self, id: u64, mode: Mode) -> bool {
        if mode != Mode::Quick {
            return false;
        }
        let since = self
            .m()
            .run
            .as_ref()
            .filter(|r| r.id == id)
            .map(|r| r.since);
        since.is_some_and(|s| self.s.clock.now().saturating_sub(s) >= self.s.quick_lifetime)
    }

    /// SIGTERM, then kill after the grace period.
    async fn graceful_stop(&self, child: &mut tokio::process::Child) {
        #[cfg(unix)]
        if let Some(pid) = child
            .id()
            .and_then(|p| rustix::process::Pid::from_raw(p as i32))
        {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
            if tokio::time::timeout(self.s.stop_grace, child.wait())
                .await
                .is_ok()
            {
                self.note("Stopped cloudflared.");
                return;
            }
        }
        let _ = child.kill().await;
        self.note("Stopped cloudflared (it had to be killed).");
    }

    /// One readiness poll (and, for the temporary tunnel, its address until known).
    async fn poll(&self, id: u64, mode: Mode, port: u16) {
        let base = format!("http://127.0.0.1:{port}");
        let ready = match self.local.get(format!("{base}/ready")).send().await {
            Ok(r) => {
                let status = r.status().as_u16();
                let body = read_capped(r, 64 * 1024).await;
                parse_ready(status, &body)
            }
            Err(_) => None,
        };
        self.m().ready(id, ready);
        if mode == Mode::Quick {
            let known = self
                .m()
                .run
                .as_ref()
                .filter(|r| r.id == id)
                .is_some_and(|r| r.quick_host.is_some());
            if !known
                && let Ok(r) = self.local.get(format!("{base}/quicktunnel")).send().await
                && r.status().is_success()
                && let Some(h) = parse_quicktunnel(&read_capped(r, 64 * 1024).await)
            {
                self.set_quick_host(id, h);
            }
        }
    }

    fn set_quick_host(&self, id: u64, host: String) {
        let new = self.m().quick_host(id, host.clone());
        if new {
            self.overrides.set(&host, self.s.listen);
            self.note(&format!(
                "The temporary tunnel's address is https://{host} (test only: it stops after one hour)."
            ));
        }
    }

    /// Copy the child's output into the ring, a line at a time (cut at 4 KiB).
    async fn read_output(self: Arc<Self>, id: u64, mode: Mode, out: impl AsyncRead + Unpin) {
        let mut r = BufReader::new(out);
        let mut line = Vec::new();
        while read_line_capped(&mut r, &mut line, 4096).await {
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end();
            if mode == Mode::Quick
                && let Some(h) = quick_url_in_line(text)
            {
                self.set_quick_host(id, h);
            }
            self.push_log(text);
        }
    }

    // -- start-up and shut-down -------------------------------------------

    /// At gateway start: stop a stale `cloudflared` of ours, read the stored state, and start
    /// the registered tunnel if it was running when the gateway last stopped.
    pub async fn startup(self: &Arc<Self>) {
        if !self.supported() {
            return;
        }
        if let Err(e) = self.ensure_dir() {
            tracing::warn!(error = %e, "the tunnel's data folder is unavailable");
            return;
        }
        self.clear_leftovers();
        self.stop_stale().await;
        let stored: Value = std::fs::read(self.state_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let hostname = stored
            .get("hostname")
            .and_then(Value::as_str)
            .and_then(|h| api::check_hostname(h).ok())
            .filter(|_| self.token_path().is_file());
        let run_at_start = stored.get("run_at_start").and_then(Value::as_bool) == Some(true);
        let installed = if self.binary().is_file() {
            match (
                &self.s.system_binary,
                stored.get("installed_version").and_then(Value::as_str),
            ) {
                (None, Some(v))
                    if download::parse_version_output(&format!("version {v}")).is_some() =>
                {
                    Some(v.to_string())
                }
                _ => self.run_version(&self.binary()).await,
            }
        } else {
            None
        };
        if self.s.system_binary.is_some() && installed.is_none() {
            let msg = format!(
                "The cloudflared at {} could not be run.",
                self.binary().display()
            );
            self.note(&msg);
            self.m().error = Some(msg);
        }
        {
            let mut m = self.m();
            m.installed = installed;
            m.hostname = hostname.clone();
            m.run_at_start = run_at_start && hostname.is_some();
        }
        if hostname.is_some() && run_at_start {
            let mut control = self.control.lock().await;
            self.note(
                "Starting the registered tunnel, as it was running when the gateway last stopped.",
            );
            self.start_locked(&mut control, Mode::Registered).await;
        }
    }

    /// Temporary files of an interrupted download or write.
    fn clear_leftovers(&self) {
        if let Ok(entries) = std::fs::read_dir(self.dir()) {
            for e in entries.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(".cloudflared.download-") || name.starts_with(".write-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    /// A `cloudflared` left running by a gateway that was killed: stopped only if
    /// `/proc/<pid>/exe` is our binary.
    async fn stop_stale(&self) {
        let Some(pid) = std::fs::read_to_string(self.pid_path())
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
            .filter(|p| *p > 1)
        else {
            self.remove_pid();
            return;
        };
        let ours = match self.binary().canonicalize() {
            Ok(p) => p,
            Err(_) => match self.dir().canonicalize() {
                Ok(d) if self.s.system_binary.is_none() => d.join("cloudflared"),
                _ => self.binary(),
            },
        };
        let is_ours = || {
            std::fs::read_link(format!("/proc/{pid}/exe"))
                .is_ok_and(|link| is_our_exe(&link, &ours))
        };
        if is_ours() {
            self.note("Stopping a cloudflared left running by the gateway's last run.");
            #[cfg(unix)]
            if let Some(p) = rustix::process::Pid::from_raw(pid) {
                let _ = rustix::process::kill_process(p, rustix::process::Signal::TERM);
                let deadline = tokio::time::Instant::now() + self.s.stop_grace;
                while is_ours() && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                if is_ours() {
                    let _ = rustix::process::kill_process(p, rustix::process::Signal::KILL);
                }
            }
        }
        self.remove_pid();
    }

    /// At gateway stop: stop the child. The registered tunnel's start-up setting is kept, so
    /// it comes back with the gateway.
    pub async fn shutdown(&self) {
        let mut control = self.control.lock().await;
        self.stop_locked(&mut control).await;
    }

    // -- admin actions -----------------------------------------------------

    /// `POST /admin/v1/tunnel/install`.
    pub async fn install(self: &Arc<Self>) -> Result<Accepted, Refusal> {
        self.check_supported()?;
        let _control = self.control.lock().await;
        let mut m = self.m();
        if m.busy() {
            return Err(refuse(409, BUSY));
        }
        if self.s.system_binary.is_some()
            || (m.installed.as_deref() == Some(self.s.pinned_version.as_str())
                && self.binary().is_file())
        {
            return Ok(Accepted::Done);
        }
        if m.running().is_some() {
            return Err(refuse(
                409,
                format!(
                    "The tunnel is running: use Update to move it to cloudflared {}.",
                    self.s.pinned_version
                ),
            ));
        }
        m.begin_install(None);
        drop(m);
        self.spawn_install();
        Ok(Accepted::Started)
    }

    /// `POST /admin/v1/tunnel/update`.
    pub async fn update(self: &Arc<Self>) -> Result<Accepted, Refusal> {
        self.check_supported()?;
        let mut control = self.control.lock().await;
        {
            let m = self.m();
            if m.busy() {
                return Err(refuse(409, BUSY));
            }
            if self.s.system_binary.is_some()
                || (m.installed.as_deref() == Some(self.s.pinned_version.as_str())
                    && self.binary().is_file())
            {
                return Ok(Accepted::Done);
            }
        }
        let was = self.stop_locked(&mut control).await;
        self.m().begin_install(was);
        self.spawn_install();
        Ok(Accepted::Started)
    }

    /// `POST /admin/v1/tunnel/register`.
    pub async fn register(self: &Arc<Self>, token: &str, hostname: &str) -> Result<(), Refusal> {
        self.check_supported()?;
        let token = token.trim();
        if !token::valid_tunnel_token(token) {
            return Err(refuse(
                400,
                "That is not a Cloudflare tunnel token: copy the whole token Cloudflare shows for the tunnel's connector (the long string after --token).",
            ));
        }
        let hostname = api::check_hostname(hostname).map_err(|e| refuse(400, e))?;
        let mut control = self.control.lock().await;
        if self.m().busy() {
            return Err(refuse(409, BUSY));
        }
        self.store_registration(token, &hostname)?;
        self.stop_locked(&mut control).await;
        self.note(&format!("Registered the tunnel for {hostname}."));
        self.start_locked(&mut control, Mode::Registered).await;
        Ok(())
    }

    /// Store the token and hostname (the control lock held).
    fn store_registration(&self, token: &str, hostname: &str) -> Result<(), Refusal> {
        self.write_private(&self.token_path(), token.as_bytes())
            .map_err(|e| {
                refuse(
                    500,
                    format!("The gateway could not write to its data folder ({e})."),
                )
            })?;
        self.m().registered(hostname.to_string());
        self.persist()
    }

    /// `POST /admin/v1/tunnel/create`: the exchange runs in the background.
    pub async fn create(
        self: &Arc<Self>,
        api_token: String,
        hostname: &str,
        replace_dns: bool,
    ) -> Result<Accepted, Refusal> {
        self.check_supported()?;
        if !api::valid_api_token(&api_token) {
            return Err(refuse(
                400,
                "The API token must be 20 to 128 letters, digits, '-' or '_': copy it from Cloudflare's API Tokens page.",
            ));
        }
        let hostname = api::check_hostname(hostname).map_err(|e| refuse(400, e))?;
        let _control = self.control.lock().await;
        if self.m().busy() {
            return Err(refuse(409, BUSY));
        }
        self.m().creating(api::Step::Zone);
        let t = self.clone();
        let req = api::Create {
            api_token: api::ApiToken::new(api_token),
            hostname,
            replace_dns,
            service: format!("http://{}", ingress_target(self.s.listen)),
        };
        tokio::spawn(async move { t.run_create(req).await });
        Ok(Accepted::Started)
    }

    async fn run_create(self: Arc<Self>, req: api::Create) {
        self.add_secret(req.api_token.expose());
        self.note(&format!(
            "Setting up a Cloudflare tunnel for {}.",
            req.hostname
        ));
        // Install first, so a failed download leaves nothing made at Cloudflare.
        if !self.installed() && self.s.system_binary.is_none() {
            self.m().begin_install(None);
            let r = self.download_and_check().await;
            if let Err(e) = &r {
                self.note(e);
            }
            let ok = r.is_ok();
            self.m().install_finished(r);
            if !ok {
                self.drop_secret(req.api_token.expose());
                return;
            }
            let _ = self.persist();
            self.m().creating(api::Step::Zone);
        }
        let result = api::create(&self.web, &self.s.api_base, &req, |step| {
            self.m().creating(step);
        })
        .await;
        let secret = req.api_token.expose().to_string();
        let hostname = req.hostname.clone();
        // The API token goes no further than here.
        drop(req);
        match result {
            Ok(created) => {
                let mut control = self.control.lock().await;
                match self.store_registration(&created.tunnel_token, &hostname) {
                    Ok(()) => {
                        self.note(&format!(
                            "Created the tunnel for {hostname} and stored its token."
                        ));
                        self.stop_locked(&mut control).await;
                        self.start_locked(&mut control, Mode::Registered).await;
                    }
                    Err(e) => {
                        self.note(&e.message);
                        self.m().create_failed(e.message);
                    }
                }
            }
            Err(e) => {
                self.note(&e);
                self.m().create_failed(e);
            }
        }
        self.drop_secret(&secret);
    }

    /// `POST /admin/v1/tunnel/start`.
    pub async fn start(self: &Arc<Self>) -> Result<(), Refusal> {
        self.check_supported()?;
        let mut control = self.control.lock().await;
        let running = {
            let m = self.m();
            if m.busy() {
                return Err(refuse(409, BUSY));
            }
            if m.hostname.is_none() || !self.token_path().is_file() {
                return Err(refuse(409, NOT_REGISTERED));
            }
            m.running()
        };
        match running {
            Some(Mode::Registered) => return Ok(()),
            Some(_) => {
                self.note("Stopping the temporary tunnel: the registered tunnel is starting.");
                self.stop_locked(&mut control).await;
            }
            None => {}
        }
        self.start_locked(&mut control, Mode::Registered).await;
        Ok(())
    }

    /// `POST /admin/v1/tunnel/stop`: the registered tunnel then stays stopped across
    /// restarts.
    pub async fn stop(self: &Arc<Self>) -> Result<(), Refusal> {
        self.check_supported()?;
        let mut control = self.control.lock().await;
        let was = self.stop_locked(&mut control).await;
        self.m().admin_stopped(was);
        if self.m().hostname.is_some() {
            self.persist()?;
        }
        Ok(())
    }

    /// `POST /admin/v1/tunnel/quick`.
    pub async fn quick(self: &Arc<Self>) -> Result<Accepted, Refusal> {
        self.check_supported()?;
        let mut control = self.control.lock().await;
        let running = {
            let m = self.m();
            if m.busy() {
                return Err(refuse(409, BUSY));
            }
            m.running()
        };
        match running {
            Some(Mode::Registered) => return Err(refuse(409, REGISTERED_RUNNING)),
            Some(_) => return Ok(Accepted::Started),
            None => {}
        }
        self.note("Starting a temporary tunnel (test only: it stops after one hour).");
        self.start_locked(&mut control, Mode::Quick).await;
        Ok(Accepted::Started)
    }

    /// `POST /admin/v1/tunnel/remove`: stop, and delete everything under `<data>/tunnel/`.
    pub async fn remove(self: &Arc<Self>) -> Result<(), Refusal> {
        self.check_supported()?;
        let mut control = self.control.lock().await;
        if self.m().busy() {
            return Err(refuse(409, BUSY));
        }
        self.stop_locked(&mut control).await;
        let removed = match std::fs::remove_dir_all(self.dir()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
        .and_then(|()| self.ensure_dir());
        let system = self
            .m()
            .installed
            .clone()
            .filter(|_| self.s.system_binary.is_some());
        *self.m() = Machine {
            installed: system,
            ..Machine::default()
        };
        self.overrides.clear();
        removed.map_err(|e| {
            refuse(
                500,
                format!("The gateway could not delete the tunnel's files ({e})."),
            )
        })?;
        self.note("Removed cloudflared and the tunnel token.");
        Ok(())
    }
}

/// `cloudflared` with no shell, an environment of only `HOME` (the tunnel's folder) and
/// `PATH`, stdin closed, killed if dropped. (`TUNNEL_TOKEN` is added for a registered run.)
fn command_for(bin: &Path, dir: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.env_clear()
        .env("HOME", dir)
        .env("PATH", CHILD_PATH)
        .current_dir(dir)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    cmd
}

/// Read at most `max` bytes of a body.
async fn read_capped(resp: reqwest::Response, max: usize) -> Vec<u8> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    let mut s = resp.bytes_stream();
    while let Some(Ok(chunk)) = s.next().await {
        let room = max.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Read one line (without its `\n`) into `line`, keeping at most `max` bytes of it and
/// discarding the rest. `false` at the end of the stream.
async fn read_line_capped(
    r: &mut (impl tokio::io::AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
    max: usize,
) -> bool {
    line.clear();
    let mut any = false;
    loop {
        let buf = match r.fill_buf().await {
            Ok(b) => b,
            Err(_) => return any,
        };
        if buf.is_empty() {
            return any;
        }
        any = true;
        let (chunk, done) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (&buf[..i], Some(i + 1)),
            None => (buf, None),
        };
        let room = max.saturating_sub(line.len());
        line.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = done.unwrap_or(buf.len());
        r.consume(used);
        if done.is_some() {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(m: &Machine) -> &'static str {
        m.state(true).as_str()
    }

    #[test]
    fn every_transition_of_the_state_machine() {
        let mut m = Machine::default();
        assert_eq!(m.state(false), State::Unsupported);
        assert_eq!(st(&m), "not_installed");

        // Install: progress, failure, then success.
        m.begin_install(None);
        assert_eq!(st(&m), "installing");
        m.install_progress(10, Some(100));
        assert_eq!(
            m.op,
            Op::Installing {
                bytes: 10,
                total: Some(100)
            }
        );
        assert_eq!(m.install_finished(Err("no network".into())), None);
        assert_eq!(st(&m), "failed");
        m.begin_install(Some(Mode::Quick));
        assert_eq!(st(&m), "installing", "a new attempt clears the failure");
        assert_eq!(
            m.install_finished(Ok("2026.10.1".into())),
            Some(Mode::Quick)
        );
        assert_eq!(st(&m), "installed");
        // Progress outside an install changes nothing.
        m.install_progress(5, None);
        assert_eq!(m.op, Op::Idle);

        // Create: each step, failure, success.
        m.creating(api::Step::Zone);
        assert_eq!(st(&m), "creating");
        m.creating(api::Step::Dns);
        assert_eq!(m.op, Op::Creating(api::Step::Dns));
        m.create_failed("a DNS record exists".into());
        assert_eq!(st(&m), "failed");
        m.registered("music.example.com".into());
        assert_eq!(st(&m), "installed");
        assert!(m.run_at_start);

        // A registered run: starting, connected, reconnecting, an exit, back.
        m.run_began(1, Mode::Registered, Duration::ZERO);
        assert_eq!(st(&m), "starting");
        m.child_spawned(1, Duration::from_secs(1));
        m.ready(1, None);
        assert_eq!(st(&m), "starting", "not yet connected");
        m.ready(1, Some(4));
        assert_eq!(st(&m), "connected");
        m.ready(1, None);
        assert_eq!(st(&m), "reconnecting");
        m.ready(1, Some(2));
        m.child_exited(1);
        assert_eq!(st(&m), "reconnecting");
        assert_eq!(m.run.as_ref().unwrap().child_since, None);
        // Another run's events are ignored.
        m.ready(9, Some(3));
        assert_eq!(st(&m), "reconnecting");
        assert!(!m.run_ended(9));

        // Stopped by the admin.
        m.run_ended(1);
        m.admin_stopped(Some(Mode::Registered));
        assert_eq!(st(&m), "stopped");
        assert!(!m.run_at_start);
        // Started again.
        m.run_began(2, Mode::Registered, Duration::ZERO);
        assert!(m.run_at_start && !m.stopped);
        // A child that can't be started at all.
        m.spawn_failed(2, "cloudflared could not be started.".into());
        assert_eq!(st(&m), "failed");
        assert!(m.run.is_none());
        // A late failure of an old run does not set an error.
        let mut n = Machine {
            installed: Some("x".into()),
            ..Machine::default()
        };
        n.spawn_failed(5, "late".into());
        assert_eq!(st(&n), "installed");

        // The quick tunnel's address.
        let mut q = Machine {
            installed: Some("x".into()),
            ..Machine::default()
        };
        q.run_began(3, Mode::Quick, Duration::ZERO);
        assert!(q.quick_host(3, "a-b.trycloudflare.com".into()));
        assert!(!q.quick_host(3, "a-b.trycloudflare.com".into()), "not new");
        assert!(
            !q.quick_host(4, "c.trycloudflare.com".into()),
            "another run"
        );
        q.child_exited(3);
        assert_eq!(
            q.run.as_ref().unwrap().quick_host,
            None,
            "a restart gets a new address"
        );
        q.run_ended(3);
        q.admin_stopped(Some(Mode::Quick));
        assert_eq!(st(&q), "installed");
        // A registered run never takes a quick address.
        q.run_began(6, Mode::Registered, Duration::ZERO);
        assert!(!q.quick_host(6, "x.trycloudflare.com".into()));
        // Stop clears a pending start after an install.
        let mut p = Machine::default();
        p.begin_install(Some(Mode::Registered));
        p.admin_stopped(None);
        assert_eq!(p.install_finished(Ok("v".into())), None);
    }

    #[test]
    fn backoff_doubles_to_a_minute() {
        let b = |n| backoff(n, Duration::from_secs(2), Duration::from_secs(60)).as_secs();
        let got: Vec<u64> = (0..8).map(b).collect();
        assert_eq!(got, [2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(b(u32::MAX), 60);
    }

    #[test]
    fn the_ingress_target_is_the_listen_address_with_any_as_loopback() {
        let t = |s: &str| ingress_target(s.parse().unwrap()).to_string();
        assert_eq!(t("127.0.0.1:9443"), "127.0.0.1:9443");
        assert_eq!(t("0.0.0.0:9443"), "127.0.0.1:9443");
        assert_eq!(t("[::]:9443"), "[::1]:9443");
        assert_eq!(t("10.0.0.10:9443"), "10.0.0.10:9443");
    }

    #[test]
    fn overrides_apply_only_while_set() {
        let o = Overrides::default();
        let lan: IpAddr = "10.0.0.10".parse().unwrap();
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(!o.active() && !o.origin_allowed("https://music.example.com"));
        assert!(!o.from_tunnel(lo));
        o.set("music.example.com", "127.0.0.1:9443".parse().unwrap());
        assert!(o.origin_allowed("https://music.example.com"));
        assert!(!o.origin_allowed("http://music.example.com"));
        assert!(!o.origin_allowed("https://music.example.com.evil.example"));
        assert!(o.from_tunnel(lo) && !o.from_tunnel(lan));
        assert!(
            !o.trusted_proxy(lan),
            "a loopback listener trusts no extra address"
        );
        o.set("music.example.com", "10.0.0.10:9443".parse().unwrap());
        assert!(o.trusted_proxy(lan) && o.from_tunnel(lan));
        assert!(o.trusted_proxy("::ffff:10.0.0.10".parse().unwrap()));
        assert!(!o.trusted_proxy("10.0.0.11".parse().unwrap()));
        o.clear();
        assert!(!o.active() && !o.trusted_proxy(lan) && !o.from_tunnel(lo));
    }

    #[test]
    fn readiness_answers_are_read() {
        let r = |s, b: &str| parse_ready(s, b.as_bytes());
        assert_eq!(
            r(
                200,
                r#"{"status":200,"readyConnections":4,"connectorId":"x"}"#
            ),
            Some(4)
        );
        assert_eq!(r(200, r#"{"readyConnections":0}"#), None);
        assert_eq!(r(503, r#"{"readyConnections":4}"#), None);
        assert_eq!(r(200, r#"{"readyConnections":"4"}"#), None);
        assert_eq!(r(200, r#"{"readyConnections":-1}"#), None);
        assert_eq!(r(200, r#"{"readyConnections":99999999999}"#), Some(1000));
        assert_eq!(r(200, "not json"), None);
    }

    #[test]
    fn the_quick_tunnel_address_is_read_from_the_endpoint_or_the_output() {
        assert_eq!(
            parse_quicktunnel(br#"{"hostname":"quiet-river-lamp.trycloudflare.com"}"#).as_deref(),
            Some("quiet-river-lamp.trycloudflare.com")
        );
        for bad in [
            r#"{"hostname":"evil.example"}"#,
            r#"{"hostname":"trycloudflare.com"}"#,
            r#"{"hostname":".trycloudflare.com"}"#,
            r#"{"hostname":"a b.trycloudflare.com"}"#,
            r#"{"hostname":"x.trycloudflare.com.evil.example"}"#,
            r#"{"hostname":"-x.trycloudflare.com"}"#,
            r#"{"hostname":""}"#,
            r#"{"hostname":7}"#,
            "{}",
        ] {
            assert_eq!(parse_quicktunnel(bad.as_bytes()), None, "{bad}");
        }
        assert_eq!(
            quick_url_in_line("INF |  https://quiet-river-lamp.trycloudflare.com          |")
                .as_deref(),
            Some("quiet-river-lamp.trycloudflare.com")
        );
        assert_eq!(
            quick_url_in_line(
                "see https://developers.cloudflare.com/x and https://a-b.trycloudflare.com"
            )
            .as_deref(),
            Some("a-b.trycloudflare.com")
        );
        assert_eq!(
            quick_url_in_line("INF Requesting new quick Tunnel on trycloudflare.com..."),
            None
        );
        assert_eq!(quick_url_in_line("http://a.trycloudflare.com"), None);
        assert_eq!(quick_url_in_line("https://"), None);
    }

    #[test]
    fn only_our_binary_counts_as_ours() {
        let ours = Path::new("/srv/data/tunnel/cloudflared");
        assert!(is_our_exe(Path::new("/srv/data/tunnel/cloudflared"), ours));
        assert!(is_our_exe(
            Path::new("/srv/data/tunnel/cloudflared (deleted)"),
            ours
        ));
        for other in [
            "/usr/bin/cloudflared",
            "/srv/data/tunnel/cloudflared2",
            "/srv/data/tunnel/cloudflared (deleted) (deleted)",
            "/bin/sh",
            "",
        ] {
            assert!(!is_our_exe(Path::new(other), ours), "{other}");
        }
    }

    #[test]
    fn the_argument_lists_are_fixed() {
        let t = Tunnel::new(
            Settings {
                listen: "0.0.0.0:9443".parse().unwrap(),
                ..Settings::default()
            },
            Overrides::default(),
        );
        assert_eq!(
            t.args(Mode::Registered, 20241),
            [
                "tunnel",
                "--no-autoupdate",
                "--metrics",
                "127.0.0.1:20241",
                "--loglevel",
                "info",
                "run"
            ]
        );
        assert_eq!(
            t.args(Mode::Quick, 20241),
            [
                "tunnel",
                "--no-autoupdate",
                "--metrics",
                "127.0.0.1:20241",
                "--loglevel",
                "info",
                "--url",
                "http://127.0.0.1:9443"
            ]
        );
    }

    /// A pseudo-random byte string (xorshift; no proptest in this crate).
    fn noise(seed: &mut u64, max: usize) -> Vec<u8> {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        let len = (*seed % max as u64) as usize;
        let mut y = *seed;
        (0..len)
            .map(|_| {
                y = y
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (y >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn parsers_of_cloudflared_output_never_panic() {
        let mut seed = 0x1234_5678_9abc_def1;
        let seeds: [&[u8]; 4] = [
            br#"{"readyConnections":4}"#,
            br#"{"hostname":"a.trycloudflare.com"}"#,
            b"INF https://quiet-lamp.trycloudflare.com |",
            b"cloudflared version 2026.10.1 (built x)",
        ];
        for i in 0..20_000 {
            let mut b = noise(&mut seed, 300);
            if i % 2 == 0 && !b.is_empty() {
                // Damage a well-formed input instead.
                let mut s = seeds[i % 4].to_vec();
                for (k, x) in b.iter().take(4).enumerate() {
                    let at = (*x as usize + k * 7) % s.len();
                    s[at] = b[(k + 1) % b.len()];
                }
                b = s;
            }
            let text = String::from_utf8_lossy(&b);
            let _ = parse_ready(200, &b);
            let _ = parse_quicktunnel(&b);
            let _ = quick_url_in_line(&text);
            let _ = download::parse_version_output(&text);
            let _ = token::valid_tunnel_token(&text);
            let _ = scrub::scrub(&text);
        }
    }

    #[tokio::test]
    async fn long_lines_are_cut_and_the_rest_discarded() {
        let data = format!("{}\nshort\nlast", "x".repeat(10_000));
        let mut r = BufReader::new(data.as_bytes());
        let mut line = Vec::new();
        assert!(read_line_capped(&mut r, &mut line, 4096).await);
        assert_eq!(line.len(), 4096);
        assert!(read_line_capped(&mut r, &mut line, 4096).await);
        assert_eq!(line, b"short");
        assert!(read_line_capped(&mut r, &mut line, 4096).await);
        assert_eq!(line, b"last");
        assert!(!read_line_capped(&mut r, &mut line, 4096).await);
    }
}
