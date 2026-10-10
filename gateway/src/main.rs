//! `anywhere-gateway`: serve the gateway, or administer accounts.

use std::io::BufRead;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use anywhere_gateway::accounts::{Accounts, Role};
use anywhere_gateway::{Config, Gateway, admin, random_id, router};

#[derive(Parser)]
#[command(version, about = "Lyrion Anywhere gateway")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

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
        /// Web app directory to serve at /.
        #[arg(long)]
        static_dir: Option<PathBuf>,
        /// Append logs to this file instead of standard output (used by the LMS plugin).
        #[arg(long)]
        log_file: Option<PathBuf>,
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
            static_dir,
            log_file: _,
        } => {
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
                trusted_proxies,
                static_dir,
                ..Config::default()
            };
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
                let admin_app = admin::router(gw.clone(), admin_secret);
                let public_app = router(gw);
                let a = tokio::spawn(async move {
                    axum::serve(adminl, admin_app.into_make_service_with_connect_info::<SocketAddr>()).await
                });
                axum::serve(public, public_app.into_make_service_with_connect_info::<SocketAddr>())
                    .with_graceful_shutdown(async {
                        let _ = tokio::signal::ctrl_c().await;
                    })
                    .await
                    .unwrap_or_else(|e| fail(e.to_string()));
                a.abort();
            });
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
    };
    if let Err(e) = result {
        fail(format!("{e:?}"));
    }
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
