//! Test harness: the real gateway (in-process) in front of a minimal fake LMS, plus an
//! in-memory secure store, an event collector and a TCP proxy that can cut connections.
#![allow(dead_code)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_gateway::accounts::{Accounts, Role};
use anywhere_gateway::{Config, Gateway, router};
use anywhere_mobile::{
    EventListener, EventsState, SecureStore, Server, ServerEvent, Session, SessionObserver,
    StoreError, StoredCredentials,
};
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::IntoResponse;
use axum::routing::any;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::OwnedWriteHalf;

pub const PW: &str = "correct horse battery";

/// Home players the fake LMS knows (not granted to anyone in these tests).
pub const HOME_PLAYER: &str = "aa:bb:cc:dd:ee:01";

/// Canned LMS JSON-RPC results.
fn fake_result(cmd: &str) -> Value {
    match cmd {
        "albums" => json!({"count": 2, "albums_loop": [
            {"id": 7, "album": "First Album", "artist": "Artist", "artist_id": 3, "year": 1999, "artwork_track_id": "abcdef12"},
            {"id": 8, "album": "Second Album", "url": "file:///srv/music/x"}
        ]}),
        "titles" => json!({"count": 2, "titles_loop": [
            {"id": 1, "title": "One", "tracknum": "1", "duration": "61.5", "coverid": "abcdef12"},
            {"id": 2, "title": "Two", "tracknum": "2", "url": "file:///srv/music/x/2.flac"}
        ]}),
        "artists" => json!({"count": 1, "artists_loop": [{"id": 3, "artist": "Artist"}]}),
        "genres" => json!({"count": 1, "genres_loop": [{"id": 4, "genre": "Jazz"}]}),
        "search" => json!({"count": 3,
            "contributors_loop": [{"contributor": "Artist", "contributor_id": 3}],
            "albums_loop": [{"album": "First Album", "album_id": 7}],
            "tracks_loop": [{"track": "One", "track_id": 1}]}),
        "players" => json!({"count": 1, "players_loop": [
            {"playerid": HOME_PLAYER, "name": "Home One", "ip": "10.0.0.31:4000", "connected": 1}
        ]}),
        "status" => json!({"player_name": "Phone", "mode": "play", "time": 12.5, "duration": 61.5,
            "mixer volume": 40, "playlist_tracks": 2, "playlist_cur_index": "0",
            "playlist_loop": [{"id": 1, "title": "One", "coverid": "abcdef12", "playlist index": 0}]}),
        _ => json!({}),
    }
}

pub struct FakeLms {
    pub http_addr: SocketAddr,
    pub cli_addr: SocketAddr,
    pub slim_addr: SocketAddr,
    pub rpc: Arc<Mutex<Vec<Value>>>,
    pub http_paths: Arc<Mutex<Vec<String>>>,
    cli_clients: Arc<tokio::sync::Mutex<Vec<OwnedWriteHalf>>>,
}

/// Size of the "huge" cover served for `/music/99999999/…`.
pub const HUGE_COVER: usize = 3 * 1024 * 1024;

impl FakeLms {
    pub async fn start() -> FakeLms {
        let rpc: Arc<Mutex<Vec<Value>>> = Default::default();
        let http_paths: Arc<Mutex<Vec<String>>> = Default::default();
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_addr = http.local_addr().unwrap();
        let (r, paths) = (rpc.clone(), http_paths.clone());
        let app = Router::new().fallback(any(move |req: Request| {
            let (r, paths) = (r.clone(), paths.clone());
            async move {
                let path = req.uri().path().to_string();
                paths.lock().unwrap().push(path.clone());
                if path == "/jsonrpc.js" {
                    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                        .await
                        .unwrap_or_default();
                    let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                    let params = v["params"].clone();
                    let cmd = params[1][0].as_str().unwrap_or("").to_string();
                    r.lock().unwrap().push(params);
                    let reply =
                        json!({"id": 1, "method": "slim.request", "result": fake_result(&cmd)});
                    return ([("content-type", "application/json")], reply.to_string())
                        .into_response();
                }
                if path.starts_with("/music/99999999/") {
                    return ([("content-type", "image/jpeg")], vec![0xffu8; HUGE_COVER])
                        .into_response();
                }
                if path.starts_with("/music/66666666/") {
                    return ([("content-type", "text/html")], "<script>alert(1)</script>")
                        .into_response();
                }
                if path.starts_with("/music/") || path.starts_with("/html/images/") {
                    return (
                        [("content-type", "image/jpeg")],
                        Body::from(b"IMAGEDATA".to_vec()),
                    )
                        .into_response();
                }
                ([("content-type", "text/plain")], "LMS internal page").into_response()
            }
        }));
        tokio::spawn(async move { axum::serve(http, app).await.unwrap() });

        let cli = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cli_addr = cli.local_addr().unwrap();
        let cli_clients: Arc<tokio::sync::Mutex<Vec<OwnedWriteHalf>>> = Default::default();
        let cc = cli_clients.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = cli.accept().await {
                let (mut rd, wr) = sock.into_split();
                cc.lock().await.push(wr);
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = rd.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                    }
                });
            }
        });

        // Slimproto isn't used by these tests; something must listen there all the same.
        let slim = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slim_addr = slim.local_addr().unwrap();
        tokio::spawn(async move { while slim.accept().await.is_ok() {} });

        FakeLms {
            http_addr,
            cli_addr,
            slim_addr,
            rpc,
            http_paths,
            cli_clients,
        }
    }

    /// Send a notification line to every CLI listener (the gateway's event feed).
    pub async fn cli_send(&self, line: &str) {
        for w in self.cli_clients.lock().await.iter_mut() {
            let _ = w.write_all(format!("{line}\n").as_bytes()).await;
        }
    }

    pub async fn cli_listeners(&self) -> usize {
        self.cli_clients.lock().await.len()
    }

    pub fn rpc_calls(&self) -> Vec<Value> {
        self.rpc.lock().unwrap().clone()
    }

    pub fn http_paths(&self) -> Vec<String> {
        self.http_paths.lock().unwrap().clone()
    }
}

pub struct Harness {
    pub lms: FakeLms,
    pub addr: SocketAddr,
    pub accounts: Arc<Accounts>,
}

impl Harness {
    pub async fn start() -> Harness {
        let lms = FakeLms::start().await;
        let cfg = Config {
            lms_slimproto: lms.slim_addr.to_string(),
            lms_http: lms.http_addr.to_string(),
            lms_cli: lms.cli_addr.to_string(),
            lms_ips: vec![Ipv4Addr::LOCALHOST],
            auth_timeout: Duration::from_millis(500),
            ..Config::default()
        };
        let accounts = Arc::new(Accounts::in_memory().unwrap());
        accounts.create_user("alice", PW, Role::Listener).unwrap();
        accounts.create_user("bob", PW, Role::Listener).unwrap();
        let gw = Gateway::new(cfg, accounts.clone());
        let app = router(gw);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap()
        });
        Harness {
            lms,
            addr,
            accounts,
        }
    }

    pub fn server(&self) -> Arc<Server> {
        Server::new(format!("http://{}", self.addr)).unwrap()
    }

    pub fn session(&self, store: Arc<MemStore>) -> Arc<Session> {
        Session::new(self.server(), store, "Test phone".into()).unwrap()
    }

    /// A session for alice, signed in with her password.
    pub async fn alice(&self) -> (Arc<Session>, Arc<MemStore>) {
        let store = MemStore::new();
        let s = self.session(store.clone());
        s.login("alice".into(), PW.into(), None).await.unwrap();
        (s, store)
    }
}

/// An in-memory [`SecureStore`] that counts writes.
#[derive(Default)]
pub struct MemStore {
    pub creds: Mutex<Option<StoredCredentials>>,
    pub saves: AtomicUsize,
    pub clears: AtomicUsize,
}

impl MemStore {
    pub fn new() -> Arc<MemStore> {
        Arc::new(MemStore::default())
    }

    pub fn with(creds: StoredCredentials) -> Arc<MemStore> {
        let s = MemStore::default();
        *s.creds.lock().unwrap() = Some(creds);
        Arc::new(s)
    }

    pub fn get(&self) -> Option<StoredCredentials> {
        self.creds.lock().unwrap().clone()
    }

    pub fn saves(&self) -> usize {
        self.saves.load(Ordering::SeqCst)
    }
}

impl SecureStore for MemStore {
    fn load(&self) -> Result<Option<StoredCredentials>, StoreError> {
        Ok(self.get())
    }
    fn save(&self, credentials: StoredCredentials) -> Result<(), StoreError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        *self.creds.lock().unwrap() = Some(credentials);
        Ok(())
    }
    fn clear(&self) -> Result<(), StoreError> {
        self.clears.fetch_add(1, Ordering::SeqCst);
        *self.creds.lock().unwrap() = None;
        Ok(())
    }
}

/// Records sign-outs.
#[derive(Default)]
pub struct Observer {
    pub signed_out: Mutex<Vec<bool>>,
}

impl SessionObserver for Observer {
    fn on_signed_out(&self, revoked: bool) {
        self.signed_out.lock().unwrap().push(revoked);
    }
}

/// Collects events and states.
#[derive(Default)]
pub struct Collector {
    pub events: Mutex<Vec<ServerEvent>>,
    pub states: Mutex<Vec<EventsState>>,
}

impl EventListener for Collector {
    fn on_event(&self, event: ServerEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn on_state(&self, state: EventsState) {
        self.states.lock().unwrap().push(state);
    }
}

impl Collector {
    pub fn events(&self) -> Vec<ServerEvent> {
        self.events.lock().unwrap().clone()
    }
    pub fn states(&self) -> Vec<EventsState> {
        self.states.lock().unwrap().clone()
    }
    pub fn live_count(&self) -> usize {
        self.states()
            .iter()
            .filter(|s| **s == EventsState::Live)
            .count()
    }
}

/// Poll `cond` until it holds (10 s at most).
pub async fn wait_for(what: &str, cond: impl Fn() -> bool) {
    for _ in 0..1000 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A TCP proxy in front of the gateway whose connections can all be cut at once.
pub struct Proxy {
    pub addr: SocketAddr,
    cut: tokio::sync::broadcast::Sender<()>,
    pub accepted: Arc<AtomicUsize>,
}

impl Proxy {
    pub async fn start(target: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (cut, _) = tokio::sync::broadcast::channel::<()>(4);
        let accepted = Arc::new(AtomicUsize::new(0));
        let (c, a) = (cut.clone(), accepted.clone());
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                a.fetch_add(1, Ordering::SeqCst);
                let mut stop = c.subscribe();
                tokio::spawn(async move {
                    let Ok(mut upstream) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {}
                        _ = stop.recv() => {}
                    }
                });
            }
        });
        Proxy {
            addr,
            cut,
            accepted,
        }
    }

    /// Drop every open connection (as a phone losing its network would).
    pub fn cut_all(&self) {
        let _ = self.cut.send(());
    }

    pub fn server(&self) -> Arc<Server> {
        Server::new(format!("http://{}", self.addr)).unwrap()
    }
}

/// Decode base32 (RFC 4648, no padding), for TOTP secrets.
pub fn base32_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u64, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("bad base32"),
        } as u64;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// The TOTP code `steps_ahead` steps from now.
pub fn totp_code(secret_b32: &str, steps_ahead: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let step = now / anywhere_gateway::totp::STEP_SECS + steps_ahead;
    format!(
        "{:06}",
        anywhere_gateway::totp::code_at(&base32_decode(secret_b32), step)
    )
}
