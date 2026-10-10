//! `anywhere-gateway`: serve the gateway, or mint a device token.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use anywhere_gateway::devices::{Devices, new_token};
use anywhere_gateway::{Config, Gateway, router};

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
        /// Address to listen on. Keep this on loopback behind a reverse proxy.
        #[arg(long, default_value = "127.0.0.1:9443")]
        listen: SocketAddr,
        /// LMS host name or address.
        #[arg(long, default_value = "127.0.0.1")]
        lms: String,
        #[arg(long, default_value_t = 3483)]
        lms_slimproto_port: u16,
        #[arg(long, default_value_t = 9000)]
        lms_http_port: u16,
        /// JSON file of devices: [{"name","mac","token_sha256"}].
        #[arg(long)]
        devices: PathBuf,
        /// Extra Origin accepted for WebSocket upgrades (repeatable).
        #[arg(long = "allowed-origin")]
        allowed_origins: Vec<String>,
        /// Reverse proxy address allowed to set X-Forwarded-* (repeatable).
        #[arg(long = "trusted-proxy")]
        trusted_proxies: Vec<IpAddr>,
        /// Web app directory to serve at /.
        #[arg(long)]
        static_dir: Option<PathBuf>,
    },
    /// Generate a device token. Prints the token (give it to the device) and the devices.json
    /// entry (store it). The token itself is never stored.
    NewToken {
        #[arg(long)]
        name: String,
        /// Locally administered MAC, e.g. 02:4c:41:00:00:01.
        #[arg(long)]
        mac: String,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    match Cli::parse().cmd {
        Cmd::NewToken { name, mac } => {
            let (token, hash) = new_token();
            println!("token: {token}");
            println!(
                "entry: {}",
                serde_json::json!({ "name": name, "mac": mac, "token_sha256": hash })
            );
        }
        Cmd::Serve {
            listen,
            lms,
            lms_slimproto_port,
            lms_http_port,
            devices,
            allowed_origins,
            trusted_proxies,
            static_dir,
        } => {
            let json = std::fs::read_to_string(&devices)
                .unwrap_or_else(|e| fail(format!("{}: {e}", devices.display())));
            let devices = Devices::from_json(&json).unwrap_or_else(|e| fail(e));
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
                lms_ips,
                allowed_origins,
                trusted_proxies,
                static_dir,
                ..Config::default()
            };
            let app = router(Gateway::new(cfg, devices));
            let listener = tokio::net::TcpListener::bind(listen)
                .await
                .unwrap_or_else(|e| fail(format!("bind {listen}: {e}")));
            tracing::info!(%listen, "gateway listening");
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .unwrap_or_else(|e| fail(e.to_string()));
        }
    }
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
