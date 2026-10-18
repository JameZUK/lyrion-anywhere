//! TLS, end to end: the app's own HTTP client, its event socket and its player socket all
//! go through `transport::tls_config`, and each refuses a certificate the device doesn't
//! trust.
//!
//! The real gateway runs behind a local TLS proxy whose certificate can be swapped between
//! connections. The certificates (`tests/fixtures/tls`, test-only, valid until 2126) are:
//! - `trusted.pem`: for `127.0.0.1`, issued by the test CA `ca.pem`;
//! - `self-signed.pem`: for `127.0.0.1`, issued by nobody the device trusts;
//! - `wrong-name.pem`: issued by the test CA, but for another name.
//!
//! The device's trust store is the platform verifier's. On Linux (where the tests run) that
//! is the system store, which `SSL_CERT_FILE` replaces: here it holds the test CA alone, as
//! a phone with that CA installed would. Nothing here can trust a certificate any other way
//! (the app has no "trust anyway" switch), so acceptance proves the configured path is the
//! one in use, and each refusal that nothing is sent before the certificate is checked.
//!
//! Everything runs in one test, which sets `SSL_CERT_FILE` before any client exists (the
//! environment is read once per process for the shared client).

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_mobile::{
    ApiError, DspSettings, Events, LocalPlayer, LocalPlayerConfig, LocalPlayerObserver,
    LocalPlayerStatus, OutputKind, Server, Session, SignalPathInfo,
};
use common::*;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tls/");

fn server_config(name: &str) -> Arc<ServerConfig> {
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(format!("{FIXTURES}{name}.pem"))
            .unwrap()
            .map(Result::unwrap)
            .collect();
    let key = PrivateKeyDer::from_pem_file(format!("{FIXTURES}{name}.key")).unwrap();
    let mut cfg =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(cfg)
}

/// A TLS proxy in front of the gateway. Each connection gets the certificate set at the
/// time; a handshake the client refuses never reaches the gateway.
struct TlsProxy {
    addr: SocketAddr,
    cert: Arc<Mutex<Arc<ServerConfig>>>,
    /// Handshakes completed, and refused.
    accepted: Arc<AtomicUsize>,
    refused: Arc<AtomicUsize>,
    cut: tokio::sync::broadcast::Sender<()>,
}

impl TlsProxy {
    async fn start(target: SocketAddr, cert: &str) -> TlsProxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cert = Arc::new(Mutex::new(server_config(cert)));
        let (accepted, refused) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (cut, _) = tokio::sync::broadcast::channel::<()>(4);
        let (c, a, r, k) = (cert.clone(), accepted.clone(), refused.clone(), cut.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let cfg = c.lock().unwrap().clone();
                let (a, r) = (a.clone(), r.clone());
                let mut stop = k.subscribe();
                tokio::spawn(async move {
                    let tls = tokio_rustls::TlsAcceptor::from(cfg).accept(tcp).await;
                    let Ok(mut tls) = tls else {
                        r.fetch_add(1, Ordering::SeqCst);
                        return;
                    };
                    a.fetch_add(1, Ordering::SeqCst);
                    let Ok(mut up) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut tls, &mut up) => {}
                        _ = stop.recv() => {}
                    }
                });
            }
        });
        TlsProxy {
            addr,
            cert,
            accepted,
            refused,
            cut,
        }
    }

    /// Present `name`'s certificate from the next connection on, and drop every open one.
    fn present(&self, name: &str) {
        *self.cert.lock().unwrap() = server_config(name);
        let _ = self.cut.send(());
    }

    fn server(&self) -> Arc<Server> {
        Server::new(format!("https://127.0.0.1:{}", self.addr.port())).unwrap()
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.accepted.load(Ordering::SeqCst),
            self.refused.load(Ordering::SeqCst),
        )
    }
}

struct QuietPlayer;

impl LocalPlayerObserver for QuietPlayer {
    fn on_status(&self, _: LocalPlayerStatus) {}
    fn on_signal_path(&self, _: Option<SignalPathInfo>) {}
}

fn is_certificate_refusal(e: &ApiError) -> bool {
    matches!(e, ApiError::Network { detail } if detail.to_lowercase().contains("certificate"))
}

/// Wait `secs` for `cond`, false if it never held.
async fn within(secs: u64, cond: impl Fn() -> bool) -> bool {
    for _ in 0..secs * 50 {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cond()
}

#[test]
fn only_a_trusted_certificate_is_accepted_on_every_path() {
    // SAFETY: the only test in this binary; no other thread exists yet to read the
    // environment, and nothing has built a TLS client.
    unsafe {
        std::env::set_var("SSL_CERT_FILE", format!("{FIXTURES}ca.pem"));
        std::env::remove_var("SSL_CERT_DIR");
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(run());
}

async fn run() {
    let h = Harness::start().await;

    // Untrusted issuer, then the wrong name: signing in is refused before anything is sent.
    for bad in ["self-signed", "wrong-name"] {
        let proxy = TlsProxy::start(h.addr, bad).await;
        let s = Session::new(proxy.server(), MemStore::new(), "Test phone".into()).unwrap();
        let e = s
            .login("alice".into(), PW.into(), None)
            .await
            .expect_err("an untrusted certificate is refused");
        assert!(is_certificate_refusal(&e), "{bad}: {e:?}");
        assert!(!s.has_sign_in());
        // The proxy hears of the refusal a moment after the client gives up.
        let p = &proxy;
        assert!(
            within(5, || p.counts().1 >= 1).await,
            "{bad}: the client refused the handshake"
        );
        assert_eq!(proxy.counts().0, 0, "{bad}: no handshake completed");
    }

    // Trusted: the HTTP client signs in, the event socket goes live.
    let proxy = TlsProxy::start(h.addr, "trusted").await;
    let s = Session::new(proxy.server(), MemStore::new(), "Test phone".into()).unwrap();
    s.login("alice".into(), PW.into(), None)
        .await
        .expect("the trusted certificate is accepted");
    assert!(s.me().await.is_ok());
    let c = Arc::new(Collector::default());
    let sub = Events::new(s.clone()).subscribe(c.clone()).await;
    wait_for("the event socket over TLS", || c.live_count() == 1).await;

    // The certificate changes to one not trusted: the event socket's reconnects and new
    // HTTP connections are refused (the sign-in is kept: a refusal is a network error).
    for bad in ["self-signed", "wrong-name"] {
        let (_, refused_before) = proxy.counts();
        proxy.present(bad);
        let p = &proxy;
        assert!(
            within(10, || p.counts().1 > refused_before).await,
            "{bad}: the event socket's reconnect is refused"
        );
        let e = s
            .me()
            .await
            .expect_err("an untrusted certificate is refused");
        assert!(is_certificate_refusal(&e), "{bad}: {e:?}");
        assert!(
            s.has_sign_in(),
            "{bad}: a certificate refusal is never a sign-out"
        );
        assert_eq!(c.live_count(), 1, "{bad}: never live through it");
    }

    // The player socket: refused while the certificate is untrusted, then up on a trusted
    // one (its HELO reaches LMS through the gateway).
    let player = LocalPlayer::new(
        s.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 48_000,
            dsp: DspSettings::default(),
            cache_dir: None,
            read_ahead: false,
            stream_cache_bytes: 0,
            downloads_dir: None,
        },
    );
    let (_, refused_before) = proxy.counts();
    player.start(Arc::new(QuietPlayer)).unwrap();
    let p = &proxy;
    assert!(
        within(10, || p.counts().1 > refused_before).await,
        "the player socket is refused"
    );
    let lms = &h.lms;
    assert!(
        !within(1, || lms.slim_events().iter().any(|e| e == "HELO")).await,
        "no HELO through an untrusted certificate"
    );

    proxy.present("trusted");
    // A new subscription connects at once (the old one is deep in its backoff by now).
    sub.cancel();
    let c = Arc::new(Collector::default());
    let sub = Events::new(s.clone()).subscribe(c.clone()).await;
    wait_for("the event socket live again", || c.live_count() == 1).await;
    wait_for("the player's HELO over TLS", || {
        lms.slim_events().iter().any(|e| e == "HELO")
    })
    .await;
    assert!(s.me().await.is_ok());

    player.stop();
    sub.cancel();
}
