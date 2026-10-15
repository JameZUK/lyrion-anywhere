//! Lyrion Anywhere gateway: the only internet-facing component.
//!
//! It authenticates devices, tunnels Slimproto over WebSocket with policy enforcement, and
//! relays audio streams. Nothing reaches LMS unless a handler here explicitly allows it.
//!
//! # `/v1/player`
//!
//! - client → gateway, first message (text), either a bare device token (new session) or
//!   `{"token":"…","resume":{"session":"…","secret":"…","received":n}}` to resume a session
//!   after a
//!   dropped connection. Then binary messages carrying the client → server Slimproto byte
//!   stream (any chunking). A new session must begin with HELO; a resumed one must not.
//!   The only text message allowed afterwards is a stream acknowledgement,
//!   `{"ack":{"stream":n,"bytes":b}}`, which clients send at least every 1 MiB received;
//!   the gateway stays at most [`Config::replay_window`] ahead of it.
//! - gateway → client: a text greeting
//!   `{"session":"…","secret":"…","mac":"02:…","name":"…","resumed":bool}` (the player
//!   identity to use in HELO and the resume secret), then one binary message per
//!   server → client Slimproto frame, length prefix included, exactly as LMS sent it.
//!
//! If the socket drops, the session (and LMS's connection) survives for
//! [`Config::resume_grace`]; frames from LMS are queued and delivered on resume. A new
//! session for the same device ends any previous one.
//!
//! # `/v1/stream/<session>/<n>[?offset=k]`
//!
//! Audio for the n-th `strm s` frame the client received (counting from 1), with the device
//! token as a bearer credential. The gateway replays only the request LMS itself issued,
//! after checking it is this player's stream. `offset` resumes a broken download.

pub mod accounts;
pub mod admin;
pub mod alarms;
pub mod analysis;
pub mod art;
mod auth;
/// Demo-only seeding (tools/demo); never in release builds.
#[cfg(debug_assertions)]
pub mod demo;
pub mod diagnostics;
pub mod download;
pub mod events;
pub mod filters;
mod flac_strip;
pub mod history;
pub mod home;
pub mod info;
pub mod limits;
pub mod live;
pub mod lms_health;
pub mod lyrics;
pub mod menu_actions;
pub mod plays;
pub mod queue_edit;
pub mod queue_save;
pub mod ratings;
mod relay;
mod rpc;
pub mod rpc_policy;
mod session;
pub mod stations;
pub mod stats;
mod status_fix;
pub mod totp;
pub mod transcode;
mod tunnel;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tracing::warn;

use accounts::Accounts;
use session::Session;

#[derive(Debug, Clone)]
pub struct Config {
    /// LMS Slimproto and HTTP listeners (host:port).
    pub lms_slimproto: String,
    pub lms_http: String,
    /// LMS command-line interface (host:port), for notifications.
    pub lms_cli: String,
    /// Addresses LMS may name in `strm` server_ip (0.0.0.0 is always allowed).
    pub lms_ips: Vec<Ipv4Addr>,
    /// Extra `Origin` values (e.g. `https://music.example.com`) accepted on WebSocket
    /// upgrade. Same-origin and absent Origin (native clients) are always accepted.
    pub allowed_origins: Vec<String>,
    /// Reverse proxies allowed to set X-Forwarded-*. Loopback is always trusted.
    pub trusted_proxies: Vec<IpAddr>,
    /// A header carrying the client's address set by a CDN in front of the proxy (e.g.
    /// `CF-Connecting-IP`). Honoured only on requests from a trusted proxy; otherwise the
    /// client address comes from `X-Forwarded-For` (see [`client_ip`]).
    pub client_ip_header: Option<axum::http::HeaderName>,
    pub auth_timeout: Duration,
    pub dial_timeout: Duration,
    /// How long a session (and its LMS connection) survives without a client socket, unless
    /// the client asked for another grace when it signed in.
    pub resume_grace: Duration,
    /// The longest resume grace a client may ask for (`"grace"` in the player socket's
    /// sign-in message). Each detached session holds one LMS connection, still one per
    /// device.
    pub max_resume_grace: Duration,
    /// A connected client that has sent nothing for this long (it answers LMS's status
    /// polls every 5 s) is reported to LMS by the gateway until it speaks again, as while
    /// detached: LMS drops a player it hasn't heard from for about 15 s, which would end the
    /// session before the client noticed its socket had died.
    pub client_silence: Duration,
    /// Bytes of LMS → client frames queued while no socket is attached; beyond this the
    /// session ends.
    pub detached_queue_limit: usize,
    /// Audio bytes kept per stream so a download can resume from a slightly earlier offset.
    pub replay_window: usize,
    /// Drop embedded artwork (PICTURE) and PADDING from the start of FLAC streams, so
    /// players can start sooner (see `flac_strip`). On by default.
    pub strip_flac_pictures: bool,
    /// Web app directory served at `/`, if any.
    pub static_dir: Option<PathBuf>,
    /// Ping interval on quiet WebSockets, so proxies with idle timeouts (Cloudflare drops
    /// after 100 s) keep them open.
    pub ws_keepalive: Duration,
    /// Keepalive interval on player sockets: a ping, and for clients that ask at sign-in a
    /// `{"keepalive":true}` text message (browsers don't show pings to scripts). It keeps a
    /// client from giving up on a socket that is quiet only because LMS is blocked.
    pub player_keepalive: Duration,
    /// An LMS request unanswered this long means LMS is blocked (see [`lms_health`]).
    pub lms_stall_after: Duration,
    /// The system ffmpeg for transcoded downloads (`None`: originals only).
    pub ffmpeg: Option<transcode::Ffmpeg>,
    /// Where transcodes are cached (`None`: no transcoding).
    pub download_cache_dir: Option<PathBuf>,
    /// Most bytes the transcode cache may hold.
    pub download_cache_bytes: u64,
    /// ffmpeg jobs at once.
    pub transcode_jobs: usize,
    /// Live streams re-encoded to Opus at once, for clients on mobile data (0: never;
    /// docs/plans/streaming-quality.md).
    pub live_transcode_jobs: usize,
    /// Longest a single transcode may run.
    pub transcode_timeout: Duration,
    /// Downloads one user may run at once.
    pub downloads_per_user: usize,
    /// Time limit for looking a track up before a download.
    pub download_lookup_timeout: Duration,
    /// Time limit for LMS to start answering a download request.
    pub download_header_timeout: Duration,
    /// Time limit for one Music & Artist Information lookup (biography, review, photo),
    /// which may go online.
    pub info_lookup_timeout: Duration,
    /// Longest LMS may pause mid-download.
    pub download_read_timeout: Duration,
    /// Event sockets one device may hold open; a newer one closes the oldest.
    pub events_per_device: usize,
    /// Event sockets one user may hold open across all their devices.
    pub events_per_user: usize,
    /// Artwork requests one user may have in flight at once.
    pub art_per_user: usize,
    /// Artwork requests in flight at once, for everyone.
    pub art_total: usize,
    /// How long an artwork request waits for a free slot before getting 429.
    pub art_queue_wait: Duration,
    /// Longest a request body (JSON, at most a few KiB) may take to arrive.
    pub body_timeout: Duration,
    /// Where track analyses are cached (`None`: analysis off). See [`analysis`].
    pub analysis_cache_dir: Option<PathBuf>,
    /// Most bytes the analysis cache may hold (about 2 KiB per track).
    pub analysis_cache_bytes: u64,
    /// Analyses running at once.
    pub analysis_jobs: usize,
    /// Analyses queued or running, for everyone.
    pub analysis_queue: usize,
    /// Analyses one user may have queued or running.
    pub analysis_per_user: usize,
    /// Largest file analysed.
    pub analysis_max_bytes: u64,
    /// Longest track analysed.
    pub analysis_max_duration: Duration,
    /// Longest one analysis (fetch and decode) may take.
    pub analysis_timeout: Duration,
    /// How long a request waits for a running analysis before getting 202.
    pub analysis_wait: Duration,
    /// How long a failed analysis is remembered (and not tried again).
    pub analysis_retry_after: Duration,
    /// The key that signs `anywhere play_record` for the plugin (the admin secret, which
    /// the plugin reads from the data directory). `None`: play sync is unavailable.
    pub plays_secret: Option<String>,
    /// Plays one user may have recorded an hour (`POST /v1/plays`).
    pub plays_per_hour: usize,
    /// Uploads of plays one user may make a minute.
    pub plays_requests_per_minute: usize,
    /// Longest one upload of plays may spend talking to LMS; the rest are answered `retry`.
    pub plays_budget: Duration,
    /// After a player starts a track, how long to wait before asking LMS which track it is
    /// (for listening history; LMS needs a moment to move on to it).
    pub history_lookup_delay: Duration,
    /// Time limit for looking one artist's name up for the listening stats.
    pub stats_lookup_timeout: Duration,
    /// Fixed answers for host names, consulted before DNS when the gateway checks where a
    /// URL would lead (the image proxy, URLs in free text): like a hosts file. Empty in
    /// production; tests use it to give a public-looking name a local address.
    pub host_answers: std::collections::HashMap<String, Vec<IpAddr>>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            lms_slimproto: "127.0.0.1:3483".into(),
            lms_http: "127.0.0.1:9000".into(),
            lms_cli: "127.0.0.1:9090".into(),
            lms_ips: vec![],
            allowed_origins: vec![],
            trusted_proxies: vec![],
            client_ip_header: None,
            auth_timeout: Duration::from_secs(5),
            dial_timeout: Duration::from_secs(5),
            resume_grace: Duration::from_secs(60),
            max_resume_grace: Duration::from_secs(600),
            client_silence: Duration::from_secs(8),
            detached_queue_limit: 1024 * 1024,
            replay_window: 4 * 1024 * 1024,
            strip_flac_pictures: true,
            static_dir: None,
            ws_keepalive: Duration::from_secs(30),
            player_keepalive: Duration::from_secs(5),
            lms_stall_after: Duration::from_secs(4),
            ffmpeg: None,
            download_cache_dir: None,
            download_cache_bytes: 2048 * 1024 * 1024,
            transcode_jobs: 1,
            live_transcode_jobs: 4,
            transcode_timeout: Duration::from_secs(600),
            downloads_per_user: 3,
            download_lookup_timeout: Duration::from_secs(10),
            download_header_timeout: Duration::from_secs(20),
            info_lookup_timeout: info::LOOKUP_TIMEOUT,
            download_read_timeout: Duration::from_secs(30),
            events_per_device: 4,
            events_per_user: 16,
            art_per_user: 8,
            art_total: 32,
            art_queue_wait: Duration::from_secs(10),
            body_timeout: Duration::from_secs(15),
            analysis_cache_dir: None,
            analysis_cache_bytes: 64 * 1024 * 1024,
            analysis_jobs: 1,
            analysis_queue: 16,
            analysis_per_user: 2,
            analysis_max_bytes: 1024 * 1024 * 1024,
            analysis_max_duration: Duration::from_secs(2 * 3600),
            analysis_timeout: Duration::from_secs(300),
            analysis_wait: Duration::from_secs(10),
            analysis_retry_after: Duration::from_secs(600),
            plays_secret: None,
            plays_per_hour: 1000,
            plays_requests_per_minute: 30,
            plays_budget: Duration::from_secs(30),
            history_lookup_delay: Duration::from_millis(1500),
            stats_lookup_timeout: stats::LOOKUP_TIMEOUT,
            host_answers: Default::default(),
        }
    }
}

pub struct Gateway {
    cfg: Config,
    accounts: Arc<Accounts>,
    http: reqwest::Client,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// App and radio commands LMS lists in its menus (the only plugin commands allowed).
    apps: Mutex<std::collections::HashSet<String>>,
    apps_refreshed: Mutex<Option<std::time::Instant>>,
    /// Menu actions LMS offered, per device (see [`menu_actions`]).
    menu_actions: Mutex<HashMap<i64, menu_actions::DeviceActions>>,
    diagnostics_throttle: diagnostics::Throttle,
    /// Image URLs LMS has handed out (the only ones its image proxy may fetch for clients).
    known_images: Mutex<art::KnownImages>,
    events: Arc<events::Hub>,
    /// HTTP client for downloads from LMS (with a per-read time limit).
    dl_http: reqwest::Client,
    /// ffmpeg and the transcode cache, when transcoding is available.
    transcoder: Option<transcode::Transcoder>,
    /// Downloads running, per user.
    active_downloads: Mutex<HashMap<i64, usize>>,
    download_audit: Mutex<download::AuditSeen>,
    download_salt: std::sync::OnceLock<String>,
    /// Artwork requests in flight, per user and overall.
    art_limit: Arc<limits::Limiter>,
    /// Track analysis, when it is on.
    analysis: Option<analysis::Analysis>,
    /// Limits on uploads of offline plays.
    plays: plays::PlayLimits,
    /// Renames of a device by its own user ([`RENAMES_PER_WINDOW`] per [`RENAME_WINDOW`]).
    renames: plays::Window,
    /// "Your devices" requests per asking device ([`DEVICE_REQUESTS_PER_WINDOW`] per
    /// [`DEVICE_REQUEST_WINDOW`]).
    device_requests: plays::Window,
    /// Requests to `/v1/me/…` per user a minute.
    me_requests: plays::Window,
    /// Users with an offline ratings batch in flight (one at a time each).
    rating_batches: Mutex<std::collections::HashSet<i64>>,
    /// Station batches per user a minute.
    station_batches: plays::Window,
    /// Tracks each station served lately (never repeated soon).
    station_served: stations::Served,
    /// Smart filter runs per user a minute, runs in flight, and recent results.
    filter_runs: plays::Window,
    filter_slots: Arc<limits::Limiter>,
    filter_results: filters::Results,
    /// Pruning of listening history.
    history_upkeep: history::Upkeep,
    /// Listening stats answers and artist names.
    stats: stats::StatsCache,
    /// Lyrics answers and the plugin lookup limit.
    lyrics: lyrics::LyricsCache,
    /// Artist and album information answers and the lookup limit.
    info: info::InfoCache,
    /// Free slots for live Opus encodes.
    live_slots: Arc<tokio::sync::Semaphore>,
    /// Whether LMS is answering (see [`lms_health`]).
    lms_health: lms_health::LmsHealth,
}

/// Renames a device may make of itself per [`RENAME_WINDOW`] (`PATCH /v1/devices/self`).
pub const RENAMES_PER_WINDOW: usize = 10;
pub const RENAME_WINDOW: Duration = Duration::from_secs(60);
/// Requests a user may make of "Your devices" (`/v1/devices`: listing and signing out) per
/// [`DEVICE_REQUEST_WINDOW`], across all their devices.
pub const DEVICE_REQUESTS_PER_WINDOW: usize = 30;
pub const DEVICE_REQUEST_WINDOW: Duration = Duration::from_secs(60);

pub type AppState = Arc<Gateway>;

impl Gateway {
    pub fn new(cfg: Config, accounts: Arc<Accounts>) -> AppState {
        let http = reqwest::Client::builder()
            .connect_timeout(cfg.dial_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("http client");
        let dl_http = reqwest::Client::builder()
            .connect_timeout(cfg.dial_timeout)
            .read_timeout(cfg.download_read_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("http client");
        let transcoder = match (&cfg.ffmpeg, &cfg.download_cache_dir) {
            (Some(ffmpeg), Some(dir)) => {
                match transcode::Cache::open(dir, cfg.download_cache_bytes) {
                    Ok(cache) => Some(transcode::Transcoder::new(
                        ffmpeg.clone(),
                        cache,
                        cfg.transcode_jobs,
                        cfg.transcode_timeout,
                    )),
                    Err(e) => {
                        warn!(error = %e, dir = %dir.display(), "transcode cache unavailable; originals only");
                        None
                    }
                }
            }
            _ => None,
        };
        let gw = Arc::new(Gateway {
            accounts,
            http,
            sessions: Mutex::new(HashMap::new()),
            apps: Mutex::new(Default::default()),
            apps_refreshed: Mutex::new(None),
            menu_actions: Mutex::new(HashMap::new()),
            diagnostics_throttle: diagnostics::Throttle::default(),
            known_images: Mutex::new(Default::default()),
            events: Arc::new(events::Hub::default()),
            dl_http,
            transcoder,
            active_downloads: Mutex::new(HashMap::new()),
            download_audit: Mutex::new(HashMap::new()),
            download_salt: std::sync::OnceLock::new(),
            art_limit: limits::Limiter::new(cfg.art_per_user, cfg.art_total),
            analysis: analysis::Analysis::new(&cfg),
            plays: plays::PlayLimits::new(cfg.plays_requests_per_minute, cfg.plays_per_hour),
            renames: plays::Window::new(RENAMES_PER_WINDOW, RENAME_WINDOW),
            device_requests: plays::Window::new(DEVICE_REQUESTS_PER_WINDOW, DEVICE_REQUEST_WINDOW),
            me_requests: plays::Window::new(
                history::ME_REQUESTS_PER_MINUTE,
                Duration::from_secs(60),
            ),
            rating_batches: Default::default(),
            station_batches: plays::Window::new(
                stations::BATCHES_PER_MINUTE,
                Duration::from_secs(60),
            ),
            station_served: Default::default(),
            filter_runs: plays::Window::new(filters::RUNS_PER_MINUTE, Duration::from_secs(60)),
            filter_slots: limits::Limiter::new(filters::RUNS_PER_USER, filters::RUNS_TOTAL),
            filter_results: Default::default(),
            history_upkeep: Default::default(),
            stats: Default::default(),
            lyrics: Default::default(),
            info: Default::default(),
            live_slots: Arc::new(tokio::sync::Semaphore::new(cfg.live_transcode_jobs)),
            lms_health: lms_health::LmsHealth::new(cfg.lms_stall_after),
            cfg,
        });
        // The watchdog needs a runtime; a gateway built outside one (some unit tests) has none.
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(lms_health::watch(Arc::downgrade(&gw)));
        }
        gw
    }

    pub fn lms_health(&self) -> &lms_health::LmsHealth {
        &self.lms_health
    }

    /// The `ffmpeg` that can make Opus, if live streams may be re-encoded at all.
    pub(crate) fn live_ffmpeg(&self) -> Option<&std::path::Path> {
        self.cfg
            .ffmpeg
            .as_ref()
            .filter(|f| f.opus && self.cfg.live_transcode_jobs > 0)
            .map(|f| f.path.as_path())
    }

    /// A slot for one live encode, if one is free.
    pub(crate) fn live_slot(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.live_slots.clone().try_acquire_owned().ok()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    fn remove_session(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }

    pub fn accounts(&self) -> &Arc<Accounts> {
        &self.accounts
    }

    /// The track analysis service, if it is on.
    pub fn analysis(&self) -> Option<&analysis::Analysis> {
        self.analysis.as_ref()
    }

    fn apps(&self) -> std::collections::HashSet<String> {
        self.apps.lock().unwrap().clone()
    }

    fn add_apps(&self, names: Vec<String>) {
        self.apps.lock().unwrap().extend(names);
    }

    /// End any live session for these devices (after revocation), then have LMS forget the
    /// players of every device revoked so far, so revoked devices don't pile up as players.
    pub async fn end_sessions_for(self: &Arc<Self>, device_ids: &[i64]) {
        let doomed: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| device_ids.contains(&s.device.device_id))
            .cloned()
            .collect();
        for s in doomed {
            s.end(self, session::EndReason::Revoked).await;
        }
        let macs = self.accounts.take_players_to_forget();
        if !macs.is_empty() {
            let gw = self.clone();
            tokio::spawn(async move { gw.forget_players(macs).await });
        }
    }

    /// Rename a device (`owner`: only if it is this user's; `None` for an admin). If it has
    /// a live session, its LMS player is renamed too, with LMS's own `name` command (LMS
    /// then tells the player, as it does when someone renames a player in LMS). Otherwise
    /// the new name reaches LMS when the device next connects (its HELO name reply).
    pub async fn rename_device(
        self: &Arc<Self>,
        device_id: i64,
        owner: Option<i64>,
        name: &str,
    ) -> Result<String, accounts::AuthError> {
        let (name, changed) = self
            .accounts
            .rename_device_if_changed(device_id, owner, name)?;
        if !changed {
            return Ok(name);
        }
        let live: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.device.device_id == device_id && !s.is_ended())
            .cloned()
            .collect();
        for s in live {
            s.set_name(&name);
            let cmd = ["name".to_string(), name.clone()];
            if let Err(e) = self.lms_command(&s.device.mac_string(), &cmd).await {
                warn!(mac = %s.device.mac_string(), error = %e, "could not rename the LMS player");
            }
        }
        Ok(name)
    }

    /// Best effort: `client forget` each player in LMS. Failures are only logged.
    async fn forget_players(&self, macs: Vec<String>) {
        // Let LMS notice the closed Slimproto connection first.
        tokio::time::sleep(FORGET_DELAY).await;
        for mac in macs {
            let cmd = ["client".to_string(), "forget".to_string()];
            match self.lms_command(&mac, &cmd).await {
                Ok(_) => tracing::info!(%mac, "removed a revoked device's LMS player"),
                Err(e) => {
                    warn!(%mac, error = %e, "could not remove a revoked device's LMS player")
                }
            }
        }
    }
}

/// Pause between ending a revoked device's session and asking LMS to forget its player.
const FORGET_DELAY: Duration = Duration::from_millis(200);

pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route("/v1/auth/login", post(auth::login))
        .route("/v1/auth/pair", post(auth::pair))
        .route("/v1/auth/refresh", post(auth::refresh))
        .route("/v1/auth/logout", post(auth::logout))
        .route("/v1/me", get(auth::me))
        .route("/v1/account", get(auth::account))
        .route("/v1/account/totp/setup", post(auth::totp_setup))
        .route("/v1/account/totp/confirm", post(auth::totp_confirm))
        .route("/v1/account/totp/disable", post(auth::totp_disable))
        .route("/v1/account/password", post(auth::change_password))
        .route("/v1/devices", get(auth::own_devices))
        .route("/v1/devices/self", axum::routing::patch(auth::rename_self))
        .route("/v1/devices/pair", post(auth::create_pairing_code))
        .route("/v1/devices/sign-out-others", post(auth::sign_out_others))
        .route(
            "/v1/devices/{id}",
            axum::routing::delete(auth::sign_out_device),
        )
        .route("/v1/rpc", post(rpc::rpc))
        .route("/v1/diagnostics", post(diagnostics::report))
        .route("/v1/art/{*path}", get(art::art))
        .route("/v1/download/track/{id}/info", get(download::info))
        .route("/v1/download/track/{id}", get(download::track))
        .route("/v1/analysis/track/{id}", get(analysis::track))
        .route("/v1/lyrics/track/{id}", get(lyrics::track))
        .route("/v1/info/artist/{id}", get(info::artist))
        .route("/v1/info/album/{id}", get(info::album))
        .route(
            "/v1/players/{player}/alarms",
            get(alarms::list).post(alarms::add),
        )
        .route(
            "/v1/players/{player}/alarms/enabled",
            axum::routing::put(alarms::set_all),
        )
        .route(
            "/v1/players/{player}/alarms/{id}",
            axum::routing::patch(alarms::update).delete(alarms::delete),
        )
        .route("/v1/players/{player}/queue/edit", post(queue_edit::edit))
        .route("/v1/players/{player}/queue/save", post(queue_save::save))
        .route("/v1/plays", post(plays::record))
        .route(
            "/v1/me/history",
            get(history::get)
                .delete(history::delete)
                .patch(history::patch),
        )
        .route("/v1/me/shelves/{name}", get(history::shelf))
        .route("/v1/me/stats", get(stats::get))
        .route(
            "/v1/me/ratings",
            get(ratings::get)
                .post(ratings::batch)
                .delete(ratings::delete_all),
        )
        .route(
            "/v1/me/ratings/style",
            get(ratings::get_style).put(ratings::put_style),
        )
        .route("/v1/me/ratings/{track}", axum::routing::put(ratings::put))
        .route(
            "/v1/me/home",
            get(home::get).put(home::put).delete(home::reset),
        )
        .route(
            "/v1/me/stations",
            get(stations::list_stations).post(stations::create_station),
        )
        .route(
            "/v1/me/stations/{id}",
            axum::routing::delete(stations::delete_station),
        )
        .route("/v1/me/stations/{id}/next", post(stations::next))
        .route(
            "/v1/me/menus",
            get(stations::list_menus).post(stations::create_menu),
        )
        .route(
            "/v1/me/menus/{id}",
            axum::routing::delete(stations::delete_menu),
        )
        .route("/v1/me/menus/{id}/open", post(stations::open_menu))
        .route(
            "/v1/me/filters",
            get(filters::list_filters).post(filters::create),
        )
        .route(
            "/v1/me/filters/{id}",
            axum::routing::put(filters::update).delete(filters::delete),
        )
        .route("/v1/me/filters/{id}/albums", get(filters::albums))
        .route("/v1/events", get(events::events_upgrade))
        .route("/v1/player", get(tunnel::player_upgrade))
        .route("/v1/stream/{session}/{n}", get(relay::stream))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            note_device_use,
        ));
    app = match &state.cfg.static_dir {
        Some(dir) => app.fallback_service(tower_http::services::ServeDir::new(dir)),
        None => app.fallback(|| async { StatusCode::NOT_FOUND }),
    };
    // Request bodies must arrive within the time limit (innermost: after the TLS check).
    let app = app.layer(middleware::from_fn_with_state(
        state.cfg.body_timeout,
        limits::body_deadline,
    ));
    // The outermost layer runs last on the way out: security headers go on every response,
    // the "TLS required" refusal included.
    app.layer(middleware::from_fn_with_state(
        state.clone(),
        require_secure_transport,
    ))
    .layer(middleware::from_fn(security_headers))
    .with_state(state)
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Browser features the app never uses are switched off for it and anything it might embed.
/// Fullscreen and autoplay stay available to the app's own origin.
const PERMISSIONS_POLICY: &str = "accelerometer=(), ambient-light-sensor=(), autoplay=(self), \
     bluetooth=(), browsing-topics=(), camera=(), display-capture=(), encrypted-media=(), \
     fullscreen=(self), geolocation=(), gyroscope=(), hid=(), idle-detection=(), \
     magnetometer=(), microphone=(), midi=(), payment=(), picture-in-picture=(), \
     publickey-credentials-get=(), screen-wake-lock=(self), serial=(), usb=(), \
     xr-spatial-tracking=()";

async fn security_headers(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    // Web app files: content-hashed ones never change; everything else (the page, the player
    // worker, worklet and WebAssembly, which keep their names) must be revalidated, or
    // browsers keep running an old player after an update.
    if !path.starts_with("/v1/") && !h.contains_key(header::CACHE_CONTROL) {
        let policy = if path.starts_with("/_app/immutable/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(policy));
    }
    // API responses carry personal data (accounts, libraries, players): never stored by
    // browsers or shared caches. Artwork keeps its own policy; downloads set theirs.
    if path.starts_with("/v1/")
        && !path.starts_with("/v1/art/")
        && !h.contains_key(header::CACHE_CONTROL)
    {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    h.insert(
        "permissions-policy",
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    // A handler may set a stricter policy of its own (e.g. artwork).
    if h.contains_key("content-security-policy") {
        return resp;
    }
    // Scripts and style sheets come only from this origin. Inline `style` attributes are
    // allowed (the web app's framework uses them); they hold no selectors, so they can't be
    // used to read page content the way injected style sheets can.
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; \
             connect-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; \
             style-src 'self'; \
             style-src-attr 'unsafe-inline'; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    resp
}

/// API requests with a valid access token: note where the device was seen from (written at
/// most once a minute per device; see [`Accounts::note_seen`]). Invalid tokens are left for
/// the handler to refuse.
async fn note_device_use(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let device = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| gw.accounts.authenticate(t))
        .map(|p| p.device_id);
    if let Some(device) = device {
        let addr = client_addr(&gw.cfg, peer, req.headers());
        gw.accounts.note_seen(device, addr);
    }
    next.run(req).await
}

/// Refuse remote requests that didn't arrive over TLS: either via a trusted reverse proxy
/// reporting `X-Forwarded-Proto: https`, or plain HTTP from loopback with no proxy headers
/// (local development).
async fn require_secure_transport(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let ip = canonical(peer.ip());
    let trusted = ip.is_loopback() || gw.cfg.trusted_proxies.iter().any(|p| canonical(*p) == ip);
    let proto = req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok());
    let proxied = req.headers().contains_key("x-forwarded-for") || proto.is_some();
    let ok = match proto {
        Some(p) => trusted && p.eq_ignore_ascii_case("https"),
        None => ip.is_loopback() && !proxied,
    };
    if !ok {
        warn!(peer = %ip, path = %req.uri().path(), "refused insecure request");
        return (StatusCode::FORBIDDEN, "TLS required\n").into_response();
    }
    next.run(req).await
}

/// The client's address for rate limiting and audit, as a key (see [`client_key`]): IPv6
/// clients are keyed by their /64, since one subscriber usually holds a whole /64 and could
/// otherwise rotate addresses to dodge lockouts.
fn client_ip(cfg: &Config, peer: SocketAddr, headers: &HeaderMap) -> String {
    client_key(client_addr(cfg, peer, headers))
}

/// Loopback and the configured reverse proxies.
fn is_trusted_proxy(cfg: &Config, ip: IpAddr) -> bool {
    let ip = canonical(ip);
    ip.is_loopback() || cfg.trusted_proxies.iter().any(|p| canonical(*p) == ip)
}

/// The client's address. A request straight from an untrusted peer is that peer. From a
/// trusted proxy:
/// - the configured CDN header (`--client-ip-header`, e.g. `CF-Connecting-IP`), if present
///   and a single valid address;
/// - otherwise `X-Forwarded-For`, read from the right: each proxy appends the address it
///   received the request from, so the rightmost entry that isn't itself a trusted proxy
///   is the first address no trusted party vouches beyond. Everything to its left was
///   written by the client and is ignored (a client can't pick its own lockout key).
pub(crate) fn client_addr(cfg: &Config, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let peer_ip = canonical(peer.ip());
    if !is_trusted_proxy(cfg, peer_ip) {
        return peer_ip;
    }
    if let Some(name) = &cfg.client_ip_header {
        let mut values = headers.get_all(name).iter();
        if let (Some(v), None) = (values.next(), values.next())
            && let Some(ip) = v.to_str().ok().and_then(parse_forwarded)
        {
            return ip;
        }
    }
    let mut chosen = peer_ip;
    let values: Vec<&HeaderValue> = headers.get_all("x-forwarded-for").iter().collect();
    for value in values.into_iter().rev() {
        let Ok(value) = value.to_str() else {
            return chosen;
        };
        for entry in value.rsplit(',') {
            let Some(ip) = parse_forwarded(entry) else {
                return chosen;
            };
            chosen = ip;
            if !is_trusted_proxy(cfg, ip) {
                return chosen;
            }
        }
    }
    chosen
}

/// A name for `--client-ip-header`: 1–64 of A-Z a-z 0-9 `-`, and not a header with another
/// meaning to the gateway or browsers.
pub fn valid_client_ip_header(s: &str) -> Option<axum::http::HeaderName> {
    const RESERVED: [&str; 8] = [
        "host",
        "origin",
        "cookie",
        "authorization",
        "x-forwarded-for",
        "x-forwarded-proto",
        "x-anywhere-client",
        "forwarded",
    ];
    let ok = !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !RESERVED.iter().any(|r| r.eq_ignore_ascii_case(s));
    ok.then(|| axum::http::HeaderName::from_bytes(s.to_ascii_lowercase().as_bytes()).ok())
        .flatten()
}

/// One forwarded address:`198.51.100.7`, `2001:db8::1`, `[2001:db8::1]`, or either with a
/// port.
fn parse_forwarded(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    if let Ok(ip) = s.parse::<IpAddr>() {
        return Some(canonical(ip));
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
        && let Ok(ip) = inner.parse::<IpAddr>()
    {
        return Some(canonical(ip));
    }
    s.parse::<SocketAddr>().ok().map(|a| canonical(a.ip()))
}

/// An address as a rate-limit key: IPv4 as is, IPv6 as its /64 prefix.
fn client_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let prefix = std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0);
            format!("{prefix}/64")
        }
    }
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// Same-origin over HTTPS, an explicitly allowed origin, or no Origin header (non-browser
/// clients). A plain-HTTP same origin is accepted only on a plain-HTTP request, which
/// [`require_secure_transport`] admits only from loopback without proxy headers (local
/// development): through a proxy, the page must have been loaded over HTTPS.
fn origin_allowed(cfg: &Config, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if cfg.allowed_origins.iter().any(|o| o == origin) {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let plain_http_request = !headers.contains_key("x-forwarded-proto");
    !host.is_empty()
        && (origin == format!("https://{host}")
            || (plain_http_request && origin == format!("http://{host}")))
}

/// A random 128-bit identifier (hex).
pub fn random_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS random source");
    hex::encode(b)
}

/// Constant-time string comparison for secrets.
fn secret_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_clients_are_keyed_by_their_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(client_key(a), "2001:db8:1:2::/64");
        assert_eq!(client_key(a), client_key(b));
        assert_ne!(client_key(a), client_key(c));
        let v4: IpAddr = "198.51.100.7".parse().unwrap();
        assert_eq!(client_key(v4), "198.51.100.7");
        let mapped: IpAddr = "::ffff:198.51.100.7".parse().unwrap();
        assert_eq!(client_key(canonical(mapped)), "198.51.100.7");
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn proxy_cfg() -> Config {
        Config {
            trusted_proxies: vec!["10.0.0.10".parse().unwrap(), "10.0.0.11".parse().unwrap()],
            ..Config::default()
        }
    }

    const LOOPBACK: &str = "127.0.0.1:50000";

    /// Regression: the leftmost `X-Forwarded-For` entry was used, which an appending proxy
    /// (Cloudflare Tunnel, nginx `$proxy_add_x_forwarded_for`) takes from the client, so a
    /// client could pick its own lockout key on every request.
    #[test]
    fn the_client_cannot_choose_its_address_through_an_appending_proxy() {
        let cfg = proxy_cfg();
        let peer: SocketAddr = LOOPBACK.parse().unwrap();
        for (xff, want) in [
            ("198.51.100.7", "198.51.100.7"),
            // The client sent "203.0.113.1"; the proxy appended the real address.
            ("203.0.113.1, 198.51.100.7", "198.51.100.7"),
            ("203.0.113.1,198.51.100.7", "198.51.100.7"),
            // Through two trusted proxies: skip them from the right.
            ("203.0.113.1, 198.51.100.7, 10.0.0.11", "198.51.100.7"),
            ("198.51.100.7, 10.0.0.10, 127.0.0.1", "198.51.100.7"),
            // Garbage the client wrote on the left is never reached.
            ("not-an-ip, 198.51.100.7", "198.51.100.7"),
            ("[2001:db8::1]:443", "2001:db8::/64"),
            ("198.51.100.7:1234", "198.51.100.7"),
            // Every entry is a trusted proxy: the leftmost of them.
            ("10.0.0.10, 10.0.0.11", "10.0.0.10"),
            // An unreadable entry stops the walk at the last trusted hop.
            ("198.51.100.7, garbage", "127.0.0.1"),
        ] {
            let h = headers(&[("x-forwarded-for", xff)]);
            assert_eq!(client_ip(&cfg, peer, &h), want, "{xff}");
        }
        // Several header lines read as one list, in order.
        let h = headers(&[
            ("x-forwarded-for", "203.0.113.1"),
            ("x-forwarded-for", "198.51.100.7"),
        ]);
        assert_eq!(client_ip(&cfg, peer, &h), "198.51.100.7");
        // Spoofing the leftmost entry doesn't change the key.
        let a = client_ip(
            &cfg,
            peer,
            &headers(&[("x-forwarded-for", "192.0.2.11, 198.51.100.7")]),
        );
        let b = client_ip(
            &cfg,
            peer,
            &headers(&[("x-forwarded-for", "192.0.2.22, 198.51.100.7")]),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn forwarded_headers_from_untrusted_peers_are_ignored() {
        let cfg = Config {
            client_ip_header: Some(axum::http::HeaderName::from_static("cf-connecting-ip")),
            ..proxy_cfg()
        };
        let peer: SocketAddr = "203.0.113.50:4000".parse().unwrap();
        let h = headers(&[
            ("x-forwarded-for", "198.51.100.7"),
            ("cf-connecting-ip", "198.51.100.8"),
        ]);
        assert_eq!(client_ip(&cfg, peer, &h), "203.0.113.50");
    }

    #[test]
    fn a_cdn_client_header_is_honoured_only_when_configured_and_from_a_trusted_proxy() {
        let peer: SocketAddr = LOOPBACK.parse().unwrap();
        let h = headers(&[
            ("x-forwarded-for", "198.51.100.7"),
            ("cf-connecting-ip", "198.51.100.8"),
        ]);
        // Not configured: ignored.
        assert_eq!(client_ip(&proxy_cfg(), peer, &h), "198.51.100.7");
        let cfg = Config {
            client_ip_header: Some(axum::http::HeaderName::from_static("cf-connecting-ip")),
            ..proxy_cfg()
        };
        assert_eq!(client_ip(&cfg, peer, &h), "198.51.100.8");
        let via_proxy: SocketAddr = "10.0.0.10:4000".parse().unwrap();
        assert_eq!(client_ip(&cfg, via_proxy, &h), "198.51.100.8");
        // Malformed, a list, or repeated: fall back to X-Forwarded-For.
        for bad in [
            vec![("cf-connecting-ip", "nonsense")],
            vec![("cf-connecting-ip", "198.51.100.8, 198.51.100.9")],
            vec![
                ("cf-connecting-ip", "198.51.100.8"),
                ("cf-connecting-ip", "198.51.100.9"),
            ],
        ] {
            let mut pairs = bad.clone();
            pairs.push(("x-forwarded-for", "198.51.100.7"));
            assert_eq!(
                client_ip(&cfg, peer, &headers(&pairs)),
                "198.51.100.7",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn client_ip_header_names_are_validated() {
        for good in ["CF-Connecting-IP", "True-Client-IP", "x-real-ip"] {
            assert!(valid_client_ip_header(good).is_some(), "{good}");
        }
        for bad in [
            "",
            "CF Connecting IP",
            "a:b",
            "x\r\nevil",
            "Host",
            "X-Forwarded-For",
            "cookie",
            &"a".repeat(65),
        ] {
            assert!(valid_client_ip_header(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn plain_http_origins_are_only_for_plain_http_requests() {
        let cfg = Config {
            allowed_origins: vec!["https://music.example.com".into()],
            ..Config::default()
        };
        let ok = |pairs: &[(&str, &str)]| origin_allowed(&cfg, &headers(pairs));
        assert!(ok(&[]), "no Origin: native clients");
        assert!(ok(&[
            ("host", "127.0.0.1:9443"),
            ("origin", "http://127.0.0.1:9443")
        ]));
        assert!(ok(&[
            ("host", "lms.example.lan"),
            ("origin", "https://lms.example.lan")
        ]));
        assert!(ok(&[("origin", "https://music.example.com")]));
        // Behind the TLS proxy, an http:// page of the same host is not the app.
        assert!(!ok(&[
            ("host", "music.example.com"),
            ("x-forwarded-proto", "https"),
            ("origin", "http://music.example.com"),
        ]));
        assert!(ok(&[
            ("host", "music.example.com"),
            ("x-forwarded-proto", "https"),
            ("origin", "https://music.example.com"),
        ]));
        assert!(!ok(&[
            ("host", "lms.example.lan"),
            ("origin", "https://evil.example")
        ]));
        assert!(!ok(&[("origin", "null")]));
    }
}
