//! `anywhere-gateway`: serve the gateway, or administer accounts.

use std::io::BufRead;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use anywhere_gateway::accounts::{Accounts, Role};
use anywhere_gateway::limits::{self, ServeLimits};
use anywhere_gateway::{Config, Gateway, admin, random_id, router, transcode};

#[derive(Parser)]
#[command(version, about = "Lyrion Anywhere gateway")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

// Parsed once at start-up; the size difference between commands doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Cmd {
    /// Run the gateway.
    Serve {
        /// Public listener. Keep this on loopback (or a LAN address) behind a TLS proxy.
        #[arg(long, default_value = "127.0.0.1:9443")]
        listen: SocketAddr,
        /// Admin API listener (loopback only; never put it behind the proxy).
        #[arg(long, default_value = "127.0.0.1:9445")]
        admin_listen: SocketAddr,
        /// Directory for the account database and the admin secret.
        #[arg(long)]
        data_dir: PathBuf,
        /// LMS host name or address.
        #[arg(long, default_value = "127.0.0.1")]
        lms: String,
        #[arg(long, default_value_t = 3483)]
        lms_slimproto_port: u16,
        #[arg(long, default_value_t = 9000)]
        lms_http_port: u16,
        /// LMS command-line interface port (for notifications).
        #[arg(long, default_value_t = 9090)]
        lms_cli_port: u16,
        /// Extra Origin accepted for WebSocket upgrades and auth requests (repeatable).
        #[arg(long = "allowed-origin")]
        allowed_origins: Vec<String>,
        /// Reverse proxy address allowed to set X-Forwarded-* (repeatable).
        #[arg(long = "trusted-proxy")]
        trusted_proxies: Vec<IpAddr>,
        /// Header a CDN sets to the client's address (e.g. CF-Connecting-IP), honoured only
        /// on requests from a trusted proxy. Only use it if the proxy accepts connections
        /// from the CDN alone.
        #[arg(long, value_parser = parse_client_ip_header)]
        client_ip_header: Option<String>,
        /// Web app directory to serve at /.
        #[arg(long)]
        static_dir: Option<PathBuf>,
        /// Append logs to this file instead of standard output (used by the LMS plugin).
        #[arg(long)]
        log_file: Option<PathBuf>,
        /// ffmpeg for transcoded downloads (default: `ffmpeg` on PATH, if any).
        #[arg(long, conflicts_with = "no_transcode")]
        ffmpeg: Option<PathBuf>,
        /// Offer downloads in their original format only, even if ffmpeg is installed.
        #[arg(long)]
        no_transcode: bool,
        /// Disk space for transcoded downloads, in MiB (the least recently used go first).
        #[arg(long, default_value_t = 2048, value_parser = clap::value_parser!(u64).range(16..=1_048_576))]
        download_cache_mb: u64,
        /// Send FLAC streams with their embedded artwork and padding, as LMS sends them
        /// (by default the gateway drops them so playback starts sooner).
        #[arg(long)]
        keep_flac_pictures: bool,
        /// Live streams re-encoded to Opus at once for players on mobile data (0 turns this
        /// off; beyond it, streams are sent as they are).
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u64).range(0..=64))]
        live_transcode_jobs: u64,
        /// Turn off track analysis (waveforms and loudness): `/v1/analysis` answers 404.
        #[arg(long)]
        no_analysis: bool,
        /// Disk space for track analyses, in MiB (about 2 KiB per track; the least recently
        /// used go first).
        #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u64).range(1..=65_536))]
        analysis_cache_mb: u64,
        /// Open client connections at once (beyond this, new ones wait).
        #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(u64).range(16..=65_536))]
        max_connections: u64,
        /// The longest a player session (and its LMS connection) is held for a client that
        /// lost its connection, in seconds, when the client asks for a longer grace (players
        /// that don't ask get 60 s).
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(60..=3600))]
        max_resume_grace: u64,
        /// A system-installed cloudflared (absolute path) for the built-in Cloudflare Tunnel,
        /// instead of the pinned release the gateway downloads on request.
        #[arg(long, conflicts_with = "no_tunnel")]
        cloudflared: Option<PathBuf>,
        /// Turn off the built-in Cloudflare Tunnel: its admin routes answer 409 and its state
        /// is "unsupported". (Otherwise it is available under <data-dir>/tunnel, but nothing is
        /// downloaded or run until the admin asks.)
        #[arg(long)]
        no_tunnel: bool,
    },
    /// Administer accounts directly (before the LMS plugin is set up).
    Admin {
        #[arg(long)]
        data_dir: PathBuf,
        #[command(subcommand)]
        cmd: AdminCmd,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    /// Create a user. The password is read from the first line of standard input.
    CreateUser {
        username: String,
        #[arg(long, default_value = "listener")]
        role: String,
    },
    /// Set a user's password (read from standard input).
    SetPassword { username: String },
    /// List users.
    ListUsers,
    /// List active devices.
    ListDevices,
    /// Create a one-time pairing code for a user's new app.
    Pair { username: String },
    /// Remove a user's two-step verification (e.g. a lost phone).
    ResetTotp { username: String },
    /// Demo only (tools/demo; debug builds only): seed invented listening history, a JSON
    /// list of plays read from standard input, into a demo data directory.
    #[cfg(debug_assertions)]
    #[command(hide = true)]
    DemoSeedHistory { username: String },
}

fn main() {
    let cli = Cli::parse();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let log_file = match &cli.cmd {
        Cmd::Serve { log_file, .. } => log_file.clone(),
        _ => None,
    };
    match log_file {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .init();
        }
        None => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
    match cli.cmd {
        Cmd::Admin { data_dir, cmd } => admin_command(&data_dir, cmd),
        Cmd::Serve {
            listen,
            admin_listen,
            data_dir,
            lms,
            lms_slimproto_port,
            lms_http_port,
            lms_cli_port,
            allowed_origins,
            trusted_proxies,
            client_ip_header,
            static_dir,
            log_file: _,
            ffmpeg,
            no_transcode,
            download_cache_mb,
            keep_flac_pictures,
            live_transcode_jobs,
            no_analysis,
            analysis_cache_mb,
            max_connections,
            max_resume_grace,
            cloudflared,
            no_tunnel,
        } => {
            if cloudflared.as_ref().is_some_and(|p| !p.is_absolute()) {
                fail("--cloudflared must be an absolute path");
            }
            if !admin_listen.ip().is_loopback() {
                fail("--admin-listen must be a loopback address");
            }
            let accounts = Arc::new(open_accounts(&data_dir));
            let admin_secret = admin_secret(&data_dir);
            let lms_ips: Vec<Ipv4Addr> = (lms.as_str(), lms_slimproto_port)
                .to_socket_addrs()
                .unwrap_or_else(|e| fail(format!("resolve {lms}: {e}")))
                .filter_map(|a| match a.ip() {
                    IpAddr::V4(v4) => Some(v4),
                    _ => None,
                })
                .collect();
            let cfg = Config {
                lms_slimproto: format!("{lms}:{lms_slimproto_port}"),
                lms_http: format!("{lms}:{lms_http_port}"),
                lms_cli: format!("{lms}:{lms_cli_port}"),
                lms_ips,
                allowed_origins,
                trusted_proxies: trusted_proxies.clone(),
                client_ip_header: client_ip_header
                    .as_deref()
                    .and_then(anywhere_gateway::valid_client_ip_header),
                static_dir,
                ffmpeg: transcoder(ffmpeg.as_deref(), no_transcode),
                download_cache_dir: Some(data_dir.join("download-cache")),
                download_cache_bytes: download_cache_mb * 1024 * 1024,
                analysis_cache_dir: if no_analysis {
                    tracing::info!("track analysis turned off");
                    None
                } else {
                    Some(data_dir.join("analysis-cache"))
                },
                analysis_cache_bytes: analysis_cache_mb * 1024 * 1024,
                strip_flac_pictures: !keep_flac_pictures,
                live_transcode_jobs: live_transcode_jobs as usize,
                max_resume_grace: std::time::Duration::from_secs(max_resume_grace),
                // Plays are signed for the plugin with the secret it already reads.
                plays_secret: Some(admin_secret.clone()),
                tunnel: anywhere_gateway::cftunnel::Settings {
                    enabled: !no_tunnel,
                    dir: Some(data_dir.join("tunnel")),
                    system_binary: cloudflared,
                    listen,
                    ..Default::default()
                },
                ..Config::default()
            };
            // A tunnel to a non-loopback listen address connects from that address: it
            // isn't held to the per-address connection cap (it is this host).
            let mut serve_trusted = trusted_proxies.clone();
            let tunnel_source = anywhere_gateway::cftunnel::ingress_target(listen).ip();
            if !no_tunnel && !tunnel_source.is_loopback() {
                serve_trusted.push(tunnel_source);
            }
            let body_timeout = cfg.body_timeout;
            let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| fail(e));
            rt.block_on(async move {
                let gw = Gateway::new(cfg, accounts.clone());
                let public = tokio::net::TcpListener::bind(listen)
                    .await
                    .unwrap_or_else(|e| fail(format!("bind {listen}: {e}")));
                let adminl = tokio::net::TcpListener::bind(admin_listen)
                    .await
                    .unwrap_or_else(|e| fail(format!("bind {admin_listen}: {e}")));
                if accounts.user_count().unwrap_or(0) == 0 {
                    tracing::warn!(
                        "no users yet: create one with `anywhere-gateway admin --data-dir … create-user NAME --role admin`"
                    );
                }
                tracing::info!(%listen, %admin_listen, "gateway listening");
                let admin_app = admin::router(gw.clone(), admin_secret).layer(
                    axum::middleware::from_fn_with_state(body_timeout, limits::body_deadline),
                );
                let tunnel = gw.tunnel().clone();
                tunnel.startup().await;
                let public_app = router(gw);
                let a = tokio::spawn(limits::serve(
                    adminl,
                    admin_app,
                    ServeLimits {
                        max_connections: 64,
                        ..ServeLimits::default()
                    },
                    std::future::pending(),
                ));
                limits::serve(
                    public,
                    public_app,
                    ServeLimits {
                        max_connections: max_connections as usize,
                        trusted_proxies: serve_trusted,
                        ..ServeLimits::default()
                    },
                    shutdown_signal(),
                )
                .await;
                a.abort();
                // Stop cloudflared on the way out (the registered tunnel comes back with the
                // gateway if it was running).
                tunnel.shutdown().await;
            });
        }
    }
}

/// SIGINT (Ctrl-C) or SIGTERM (what the LMS plugin and service managers send).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        tracing::info!("shutting down");
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The ffmpeg to transcode downloads with, if any (spec D10).
fn transcoder(explicit: Option<&Path>, disabled: bool) -> Option<transcode::Ffmpeg> {
    if disabled {
        tracing::info!("transcoding turned off: downloads in their original format only");
        return None;
    }
    let path = match explicit {
        Some(p) if !p.is_absolute() => {
            tracing::warn!(
                "--ffmpeg must be an absolute path; downloads in their original format only"
            );
            return None;
        }
        Some(p) => p.to_path_buf(),
        None => match transcode::find_on_path() {
            Some(p) => p,
            None => {
                tracing::info!("ffmpeg not found: downloads in their original format only");
                return None;
            }
        },
    };
    match transcode::probe(&path, std::time::Duration::from_secs(10)) {
        Some(f) => {
            tracing::info!(path = %f.path.display(), mp3 = f.mp3, opus = f.opus, "transcoding downloads with ffmpeg");
            Some(f)
        }
        None => {
            tracing::warn!(path = %path.display(), "ffmpeg unusable (not executable, or without libmp3lame/libopus): downloads in their original format only");
            None
        }
    }
}

fn open_accounts(dir: &Path) -> Accounts {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(format!("{}: {e}", dir.display())));
    restrict_dir(dir);
    let path = dir.join("accounts.db");
    let accounts =
        Accounts::open(&path).unwrap_or_else(|e| fail(format!("account database: {e:?}")));
    restrict_file(&path);
    accounts
}

/// The admin API secret: created on first start, readable only by this user.
fn admin_secret(dir: &Path) -> String {
    let path = dir.join("admin.secret");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let s = s.trim().to_string();
        if s.len() >= 32 {
            return s;
        }
    }
    let secret = format!("{}{}", random_id(), random_id());
    write_private(&path, &secret);
    tracing::info!(path = %path.display(), "created admin secret");
    secret
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}
#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) {}

/// Owner-only access to the database (and its WAL/SHM side files).
#[cfg(unix)]
fn restrict_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
}
#[cfg(not(unix))]
fn restrict_file(_path: &Path) {}

fn write_private(path: &Path, contents: &str) {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    f.write_all(contents.as_bytes()).unwrap_or_else(|e| fail(e));
}

fn read_password() -> String {
    eprint!("password (from stdin): ");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .unwrap_or_else(|e| fail(e));
    eprintln!();
    line.trim_end_matches(['\r', '\n']).to_string()
}

fn admin_command(dir: &Path, cmd: AdminCmd) {
    let accounts = open_accounts(dir);
    let result = match cmd {
        AdminCmd::CreateUser { username, role } => {
            let role = Role::parse(&role).unwrap_or_else(|| fail("role must be listener or admin"));
            accounts
                .create_user(&username, &read_password(), role)
                .map(|_| println!("created {username}"))
        }
        AdminCmd::SetPassword { username } => accounts
            .set_password(&username, &read_password())
            .map(|_| println!("password set")),
        AdminCmd::ListUsers => accounts.list_users().map(|users| {
            for u in users {
                println!(
                    "{}\t{}{}",
                    u.username,
                    u.role,
                    if u.disabled { "\tdisabled" } else { "" }
                );
            }
        }),
        AdminCmd::ListDevices => accounts.list_devices().map(|devices| {
            for d in devices {
                println!("{}\t{}\t{}\t{}", d.id, d.username, d.mac, d.name);
            }
        }),
        AdminCmd::ResetTotp { username } => accounts
            .totp_reset(&username)
            .map(|_| println!("two-step verification removed for {username}")),
        AdminCmd::Pair { username } => accounts.create_pairing_code(&username).map(|code| {
            println!("pairing code for {username}: {code} (valid 10 minutes, single use)");
        }),
        #[cfg(debug_assertions)]
        AdminCmd::DemoSeedHistory { username } => {
            use std::io::Read;
            let mut input = Vec::new();
            let _ = std::io::stdin()
                .lock()
                .take(anywhere_gateway::demo::MAX_INPUT as u64 + 1)
                .read_to_end(&mut input)
                .unwrap_or_else(|e| fail(e));
            let now = anywhere_gateway::history::unix_now();
            match anywhere_gateway::demo::seed(&accounts, dir, &username, &input, now) {
                Ok(n) => {
                    println!("seeded {n} plays for {username}");
                    Ok(())
                }
                Err(e) => fail(format!("refused: {e:?}")),
            }
        }
    };
    if let Err(e) = result {
        fail(format!("{e:?}"));
    }
}

/// `--client-ip-header`: a plain header name that can't stand in for the headers the gateway
/// already interprets itself.
fn parse_client_ip_header(s: &str) -> Result<String, String> {
    anywhere_gateway::valid_client_ip_header(s)
        .map(|_| s.to_string())
        .ok_or_else(|| "must be a header name such as CF-Connecting-IP".to_string())
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
