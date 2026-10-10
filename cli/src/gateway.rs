//! Gateway transport: Slimproto over `WS /v1/player`, audio over `GET /v1/stream/…`.
//!
//! This is the reference client for the gateway protocol (`gateway/src/lib.rs`).

use std::io::{ErrorKind, Read};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Once};
use std::thread;
use std::time::Duration;

use rustls::RootCertStore;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use tungstenite::client::IntoClientRequest;
use tungstenite::{Connector, Message};

use crate::{Event, StreamHandle};

pub struct GatewayConn {
    pub session: String,
    base: String,
    token: String,
    agent: ureq::Agent,
    out: Sender<Vec<u8>>,
}

struct Endpoint {
    https: bool,
    host: String,
    port: u16,
}

fn parse_base(base: &str) -> Result<Endpoint, String> {
    let (https, rest) = if let Some(r) = base.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = base.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("--gateway must start with https:// (or http:// for loopback testing)".into());
    };
    let authority = rest.trim_end_matches('/');
    if authority.contains('/') || authority.is_empty() {
        return Err("--gateway must be a base URL without a path".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (
            h.to_string(),
            p.parse().map_err(|_| "bad port in --gateway")?,
        ),
        _ => (authority.to_string(), if https { 443 } else { 80 }),
    };
    Ok(Endpoint { https, host, port })
}

/// Trust roots: the system store, plus an optional extra CA (e.g. a local test CA).
fn root_certs(ca_file: Option<&str>) -> Result<Vec<CertificateDer<'static>>, String> {
    let mut certs: Vec<CertificateDer<'static>> = rustls_native_certs::load_native_certs().certs;
    if let Some(path) = ca_file {
        let pem = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        for c in CertificateDer::pem_slice_iter(&pem) {
            certs.push(c.map_err(|e| format!("{path}: {e}"))?);
        }
    }
    if certs.is_empty() {
        return Err("no trusted CA certificates found".into());
    }
    Ok(certs)
}

fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub fn connect(
    base: &str,
    token: &str,
    ca_file: Option<&str>,
    events: Sender<Event>,
) -> Result<GatewayConn, String> {
    install_crypto_provider();
    let ep = parse_base(base)?;
    let base = base.trim_end_matches('/').to_string();
    let roots = root_certs(ca_file)?;

    // WebSocket control channel.
    let mut store = RootCertStore::empty();
    for c in &roots {
        let _ = store.add(c.clone());
    }
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    let ws_url = format!(
        "{}://{}:{}/v1/player",
        if ep.https { "wss" } else { "ws" },
        ep.host,
        ep.port
    );
    let tcp = TcpStream::connect((ep.host.as_str(), ep.port))
        .map_err(|e| format!("connect {}: {e}", ep.host))?;
    tcp.set_nodelay(true).ok();
    let tcp_ctl = tcp.try_clone().map_err(|e| e.to_string())?;
    let req = ws_url.into_client_request().map_err(|e| e.to_string())?;
    let connector = ep.https.then(|| Connector::Rustls(Arc::new(tls)));
    let (mut ws, _) = tungstenite::client_tls_with_config(req, tcp, None, connector)
        .map_err(|e| format!("websocket handshake: {e}"))?;

    ws.send(Message::Text(token.into()))
        .map_err(|e| e.to_string())?;
    let session = match ws
        .read()
        .map_err(|e| format!("gateway refused the connection: {e}"))?
    {
        Message::Text(t) => serde_json::from_str::<serde_json::Value>(t.as_str())
            .ok()
            .and_then(|v| v["session"].as_str().map(String::from))
            .ok_or("bad session message from gateway")?,
        Message::Close(f) => {
            return Err(format!(
                "gateway closed the connection: {}",
                f.map(|f| f.reason.to_string()).unwrap_or_default()
            ));
        }
        other => return Err(format!("unexpected first message: {other:?}")),
    };

    // Poll reads with a short timeout so one thread can both read and write.
    tcp_ctl
        .set_read_timeout(Some(Duration::from_millis(10)))
        .ok();
    let (out, out_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
    thread::spawn(move || {
        loop {
            while let Ok(b) = out_rx.try_recv() {
                if ws.write(Message::Binary(b.into())).is_err() {
                    let _ = events.send(Event::ControlClosed);
                    return;
                }
            }
            if ws.flush().is_err() {
                let _ = events.send(Event::ControlClosed);
                return;
            }
            match ws.read() {
                Ok(Message::Binary(b)) => {
                    if events.send(Event::Control(b.to_vec())).is_err() {
                        return;
                    }
                }
                Ok(Message::Close(f)) => {
                    if let Some(f) = f {
                        eprintln!("gateway closed: {} ({})", f.reason, u16::from(f.code));
                    }
                    let _ = events.send(Event::ControlClosed);
                    return;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => {
                    eprintln!("gateway connection error: {e}");
                    let _ = events.send(Event::ControlClosed);
                    return;
                }
            }
        }
    });

    // HTTPS agent for audio, with the same trust roots.
    let ureq_certs: Vec<ureq::tls::Certificate<'static>> = roots
        .iter()
        .map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned())
        .collect();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::new_with_certs(&ureq_certs))
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into();

    Ok(GatewayConn {
        session,
        base,
        token: token.to_string(),
        agent,
        out,
    })
}

impl GatewayConn {
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.out
            .send(bytes)
            .map_err(|_| "gateway connection closed".to_string())
    }

    /// Fetch stream `n` of this session.
    pub fn spawn_stream(&self, id: u64, n: u64, events: Sender<Event>) -> StreamHandle {
        let want = Arc::new(AtomicBool::new(true));
        let cancel = Arc::new(AtomicBool::new(false));
        let (w, c) = (want.clone(), cancel.clone());
        let url = format!("{}/v1/stream/{}/{}", self.base, self.session, n);
        let auth = format!("Bearer {}", self.token);
        let agent = self.agent.clone();
        thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let resp = agent
                    .get(&url)
                    .header("Authorization", &auth)
                    .call()
                    .map_err(|e| e.to_string())?;
                let status = resp.status();
                if status != 200 {
                    return Err(format!("gateway stream status {status}"));
                }
                let ct = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let headers = format!("HTTP/1.0 200 OK\r\nContent-Type: {ct}\r\n\r\n");
                let _ = events.send(Event::StreamHeaders(id, headers.into_bytes()));
                let mut body = resp.into_body().into_reader();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    if c.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    if !w.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    let n = body.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        let _ = events.send(Event::StreamEnd(id));
                        return Ok(());
                    }
                    if events
                        .send(Event::StreamData(id, buf[..n].to_vec()))
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            })();
            if let Err(e) = result {
                let _ = events.send(Event::StreamError(id, e));
            }
        });
        StreamHandle { want, cancel }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_parsing() {
        let e = parse_base("https://music.example.com").unwrap();
        assert!(e.https && e.host == "music.example.com" && e.port == 443);
        let e = parse_base("http://127.0.0.1:9443/").unwrap();
        assert!(!e.https && e.host == "127.0.0.1" && e.port == 9443);
        for bad in [
            "music.example.com",
            "ftp://x",
            "https://x/v1",
            "https://",
            "https://x:notaport",
        ] {
            assert!(parse_base(bad).is_err(), "{bad} accepted");
        }
    }
}
