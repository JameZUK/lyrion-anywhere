//! Protocol 2 on the player socket (docs/PROTOCOL.md, *Versions*, "The stream message",
//! close 4426), against a scripted gateway that speaks the documented JSON:
//! - the gateway's stream message reaches the signal-path snapshot of the stream it names,
//!   by `seq` (the count of `strm s` frames): across a track change, a resume (the message
//!   re-sent after the greeting) and a seek, never attached to the wrong stream, and junk
//!   text is ignored;
//! - every sign-in form carries `"protocol": 2`;
//! - close 4426 stops the player for good with `UpdateApp`, never retried;
//! - a gateway whose `/v1/me` gives no protocol, or one below 2, gets no player socket
//!   (`UpdatePlugin`);
//! - a change of level waits for the gateway's echo (at most `QUALITY_ECHO_WAIT`), and
//!   junk is not the echo.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_core::proto::{Strm, encode_server_frame};
use anywhere_mobile::{
    LocalPlayer, LocalPlayerConfig, LocalPlayerObserver, LocalPlayerStatus, OutputKind,
    PlayerConnection, Server, Session, SignalPathInfo, StreamQuality, StreamSent,
};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use common::flac;
use common::{MemStore, PW, wait_for};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const SESSION: &str = "0123456789abcdef0123456789abcdef";
const SECRET: &str = "fedcba9876543210fedcba9876543210";

/// What the scripted gateway sends down a player socket.
enum Down {
    Msg(Message),
    /// Drop the connection with no close frame (a lost network).
    Cut,
}

#[derive(Default)]
struct Fake {
    /// Every sign-in form, in order.
    forms: Mutex<Vec<Value>>,
    /// The live connection's sender.
    conn: Mutex<Option<mpsc::UnboundedSender<Down>>>,
    /// Close each connection with this code (and reason) after its sign-in form.
    close_with: Mutex<Option<(u16, &'static str)>>,
    /// Stream bodies by number.
    streams: Mutex<HashMap<u64, Vec<u8>>>,
    /// Stream numbers fetched.
    fetched: Mutex<Vec<u64>>,
    /// STAT events from the player, in order.
    stats: Mutex<Vec<String>>,
    /// `GET /v1/me` has no `protocol` (a gateway before 0.7.3).
    old_gateway: AtomicBool,
    /// `GET /v1/me` says `"protocol": 1`.
    protocol_one: AtomicBool,
    /// How the gateway answers a `{"quality"}` control message.
    echo: Mutex<Echo>,
    /// The `{"quality"}` control messages received, in order.
    quality: Mutex<Vec<String>>,
}

/// How the scripted gateway answers a change of level.
#[derive(Clone, Copy, Default, PartialEq)]
enum Echo {
    /// `{"quality":<the level>}` at once, as the gateway does once applied.
    #[default]
    Prompt,
    /// Nothing.
    Silent,
    /// Junk that is not the echo (wrong case, an extra key, another level, the stream
    /// message's shape) at once, then the real echo after `ms`.
    JunkThen { ms: u64 },
    /// Only junk.
    Junk,
}

/// What the scripted gateway sends instead of a well-formed echo of `level`.
fn junk_echoes(level: &str) -> Vec<String> {
    let other = if level == "low" { "high" } else { "low" };
    vec![
        format!(r#"{{"quality":"{}"}}"#, level.to_uppercase()),
        format!(r#"{{"quality":"{level}","extra":1}}"#),
        format!(r#"{{"quality":"{other}"}}"#),
        format!(r#"{{"Quality":"{level}"}}"#),
        format!(r#"{{"quality":["{level}"]}}"#),
        "not json".into(),
    ]
}

impl Fake {
    fn send(&self, m: Message) {
        if let Some(c) = self.conn.lock().unwrap().as_ref() {
            let _ = c.send(Down::Msg(m));
        }
    }

    fn text(&self, s: &str) {
        self.send(Message::Text(s.to_string().into()));
    }

    fn frame(&self, f: Vec<u8>) {
        self.send(Message::Binary(f.into()));
    }

    fn cut(&self) {
        if let Some(c) = self.conn.lock().unwrap().take() {
            let _ = c.send(Down::Cut);
        }
    }

    fn stats(&self, e: &str) -> usize {
        self.stats
            .lock()
            .unwrap()
            .iter()
            .filter(|s| *s == e)
            .count()
    }
}

async fn socket(fake: Arc<Fake>, mut ws: WebSocket) {
    let Some(Ok(Message::Text(form))) = ws.recv().await else {
        return;
    };
    let form: Value = serde_json::from_str(form.as_str()).unwrap_or(Value::Null);
    let resumed = form.get("resume").is_some();
    fake.forms.lock().unwrap().push(form);
    let close = *fake.close_with.lock().unwrap();
    if let Some((code, reason)) = close {
        let _ = ws
            .send(Message::Close(Some(CloseFrame {
                code,
                reason: reason.into(),
            })))
            .await;
        // Wait for the client's answer to the close.
        while let Some(Ok(_)) = ws.recv().await {}
        return;
    }
    let greeting = json!({
        "session": SESSION, "secret": SECRET, "mac": "02:4c:41:00:00:01",
        "name": "Test phone", "resumed": resumed, "grace": 60,
    });
    if ws
        .send(Message::Text(greeting.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    *fake.conn.lock().unwrap() = Some(tx);
    let mut up: Vec<u8> = Vec::new();
    loop {
        tokio::select! {
            d = rx.recv() => match d {
                Some(Down::Msg(m)) => {
                    if ws.send(m).await.is_err() {
                        return;
                    }
                }
                // Dropped with no close frame.
                Some(Down::Cut) | None => return,
            },
            m = ws.recv() => match m {
                Some(Ok(Message::Binary(b))) => {
                    // Client frames: opcode, a 32-bit length, the payload.
                    up.extend_from_slice(&b);
                    while up.len() >= 8 {
                        let len = u32::from_be_bytes(up[4..8].try_into().unwrap()) as usize;
                        if up.len() < 8 + len {
                            break;
                        }
                        if &up[..4] == b"STAT" && len >= 4 {
                            let e = String::from_utf8_lossy(&up[8..12]).into_owned();
                            fake.stats.lock().unwrap().push(e);
                        }
                        up.drain(..8 + len);
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
                    let Some(level) = v["quality"].as_str().map(str::to_string) else {
                        continue;
                    };
                    fake.quality.lock().unwrap().push(t.to_string());
                    let echo = json!({ "quality": level }).to_string();
                    let mode = *fake.echo.lock().unwrap();
                    let mut send = Vec::new();
                    if matches!(mode, Echo::Junk | Echo::JunkThen { .. }) {
                        send.extend(junk_echoes(&level));
                    }
                    if mode == Echo::Prompt {
                        send.push(echo.clone());
                    }
                    for s in send {
                        if ws.send(Message::Text(s.into())).await.is_err() {
                            return;
                        }
                    }
                    if let Echo::JunkThen { ms } = mode
                        && let Some(c) = fake.conn.lock().unwrap().clone()
                    {
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(ms)).await;
                            let _ = c.send(Down::Msg(Message::Text(echo.into())));
                        });
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

async fn start(fake: Arc<Fake>) -> Arc<Session> {
    use axum::extract::{Path, Query, Request};
    use axum::routing::{any, get};
    let f1 = fake.clone();
    let f2 = fake.clone();
    let f3 = fake.clone();
    let app = axum::Router::new()
        .route(
            "/v1/player",
            get(move |up: WebSocketUpgrade| {
                let f = f1.clone();
                async move { up.on_upgrade(move |ws| socket(f, ws)) }
            }),
        )
        .route(
            "/v1/stream/{session}/{n}",
            get(
                move |Path((_s, n)): Path<(String, u64)>, Query(q): Query<HashMap<String, u64>>| {
                    let f = f2.clone();
                    async move {
                        f.fetched.lock().unwrap().push(n);
                        // From `?offset=` (the end confirmed by an empty answer).
                        let from = q.get("offset").copied().unwrap_or(0) as usize;
                        match f.streams.lock().unwrap().get(&n).cloned() {
                            Some(body) => (
                                [("content-type", "audio/flac")],
                                body[from.min(body.len())..].to_vec(),
                            )
                                .into_response(),
                            None => axum::http::StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                },
            ),
        )
        .fallback(any(move |req: Request| {
            let old = f3.old_gateway.load(Ordering::SeqCst);
            let one = f3.protocol_one.load(Ordering::SeqCst);
            async move {
                match (req.method().as_str(), req.uri().path()) {
                    ("GET", "/v1/me") if old => json!({
                    "user": {"username": "alice", "role": "listener"},
                    "device": {"id": 1, "name": "Test phone"},
                    "features": {}})
                    .to_string()
                    .into_response(),
                    ("GET", "/v1/me") if one => json!({
                    "user": {"username": "alice", "role": "listener"},
                    "device": {"id": 1, "name": "Test phone", "mac": "02:4c:41:00:00:01"},
                    "protocol": 1,
                    "features": {"stream_quality": true, "resume_grace": true}})
                    .to_string()
                    .into_response(),
                    ("POST", "/v1/auth/login" | "/v1/auth/refresh") => json!({
                    "access_token": "access", "refresh_token": "refresh", "expires_in": 900,
                    "user": {"username": "alice", "role": "listener"},
                    "device": {"id": 1, "name": "Test phone", "mac": "02:4c:41:00:00:01"}})
                    .to_string()
                    .into_response(),
                    ("GET", "/v1/me") => json!({
                    "user": {"username": "alice", "role": "listener"},
                    "device": {"id": 1, "name": "Test phone", "mac": "02:4c:41:00:00:01"},
                    "protocol": 2,
                    "features": {"stream_quality": true, "resume_grace": true}})
                    .to_string()
                    .into_response(),
                    _ => axum::http::StatusCode::NOT_FOUND.into_response(),
                }
            }
        }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let server = Server::new(format!("http://{addr}")).unwrap();
    let s = Session::new(server, MemStore::new(), "Test phone".into()).unwrap();
    s.login("alice".into(), PW.into(), None).await.unwrap();
    s
}

#[derive(Default)]
struct Obs {
    statuses: Mutex<Vec<LocalPlayerStatus>>,
    paths: Mutex<Vec<SignalPathInfo>>,
}

impl LocalPlayerObserver for Obs {
    fn on_status(&self, status: LocalPlayerStatus) {
        self.statuses.lock().unwrap().push(status);
    }
    fn on_signal_path(&self, path: Option<SignalPathInfo>) {
        if let Some(p) = path {
            self.paths.lock().unwrap().push(p);
        }
    }
}

impl Obs {
    fn saw_path(&self, f: impl Fn(&SignalPathInfo) -> bool) -> bool {
        self.paths.lock().unwrap().iter().any(f)
    }
}

/// A player pulling at about real time (so a track plays for its length).
fn player(session: Arc<Session>, obs: Arc<Obs>) -> (Arc<LocalPlayer>, Arc<AtomicBool>) {
    let player = LocalPlayer::new(
        session,
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 192_000,
            cache_dir: None,
            read_ahead: false,
            stream_cache_bytes: 0,
            downloads_dir: None,
            dsp: Default::default(),
        },
    );
    player.start(obs).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let (p, s) = (player.clone(), stop.clone());
    tokio::spawn(async move {
        while !s.load(Ordering::SeqCst) {
            let _ = p.read_pcm(1024);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    (player, stop)
}

fn strm(command: u8) -> Vec<u8> {
    let s = Strm {
        command,
        autostart: b'1',
        format: b'f',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: 1,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        replay_gain: 0,
        server_port: 9000,
        server_ip: 0,
        request: if command == b's' {
            b"GET /stream.mp3?player=02:4c:41:00:00:01 HTTP/1.0\r\n\r\n".to_vec()
        } else {
            Vec::new()
        },
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

/// Volume 100 %: LMS sends unity gain (0x10000).
fn audg_full() -> Vec<u8> {
    let mut p = vec![0u8; 18];
    p[8] = 1; // adjust
    p[9] = 255;
    p[10..14].copy_from_slice(&0x10000u32.to_be_bytes());
    p[14..18].copy_from_slice(&0x10000u32.to_be_bytes());
    encode_server_frame(*b"audg", &p)
}

fn stream_message(seq: u64, rate: u32, bits: u32, sent: Value) -> String {
    json!({"stream": {"seq": seq,
        "served": {"codec": "flac", "rate": rate, "bits": bits, "channels": 2},
        "sent": sent}})
    .to_string()
}

fn opus(kbps: u32, level: &str) -> Value {
    json!({"mode": "opus", "kbps": kbps, "level": level})
}

/// The message is tied to its stream by `seq`: the first track's (sent just after its
/// stream started) shows only while it plays; the second's, re-sent after a resume, shows
/// for the second; a seek's new stream (the third `strm s`, counted once across the
/// resume) gets its own. Junk text and messages for other streams change nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_messages_follow_their_streams_through_a_change_a_resume_and_a_seek() {
    let fake = Arc::new(Fake::default());
    let session = start(fake.clone()).await;
    let track1 = flac::encode(44_100, 16, &flac::noise(44_100 * 2, 16, 1));
    let track2 = flac::encode(48_000, 24, &flac::noise(48_000 * 3, 24, 2));
    let track3 = flac::encode(88_200, 24, &flac::noise(88_200 * 3, 24, 3));
    {
        let mut s = fake.streams.lock().unwrap();
        s.insert(1, track1);
        s.insert(2, track2);
        s.insert(3, track3);
    }
    let obs = Arc::new(Obs::default());
    let (player, stop) = player(session, obs.clone());
    wait_for("the first connection", || {
        fake.conn.lock().unwrap().is_some()
    })
    .await;

    // Track 1. Junk first: ignored, never counted, never shown.
    fake.frame(audg_full());
    fake.frame(strm(b's'));
    wait_for("stream 1 fetched", || {
        fake.fetched.lock().unwrap().contains(&1)
    })
    .await;
    fake.text(r#"{"keepalive":true}"#);
    fake.text("not json");
    fake.text(r#"{"stream":{"seq":1,"served":{"codec":"FLAC"},"sent":{"mode":"passthrough"}}}"#);
    // A message for a stream that isn't playing.
    fake.text(&stream_message(
        9,
        192_000,
        32,
        json!({"mode": "passthrough"}),
    ));
    fake.text(&stream_message(1, 96_000, 24, opus(160, "high")));
    let o = obs.clone();
    wait_for("track 1's message in the snapshot", move || {
        o.saw_path(|p| {
            p.source_rate == 44_100
                && p.sent
                    == Some(StreamSent::Opus {
                        kbps: 160,
                        level: StreamQuality::High,
                    })
                && p.served.as_ref().is_some_and(|s| {
                    s.codec == "flac" && s.rate == Some(96_000) && s.bits == Some(24)
                })
        })
    })
    .await;
    assert!(
        !obs.saw_path(|p| p.served.as_ref().is_some_and(|s| s.rate == Some(192_000))),
        "another stream's message never shows"
    );
    // Messages for streams not heard (and the playing one's again, unchanged) publish no
    // snapshot of their own: before, each one forced a publish.
    let before = obs.paths.lock().unwrap().len();
    for n in 0..20 {
        // A stream not heard yet, its message corrected again and again (each one kept).
        let sent = if n % 2 == 0 {
            json!({"mode": "passthrough"})
        } else {
            opus(96, "low")
        };
        fake.text(&stream_message(9, 192_000, 32, sent));
        fake.text(&stream_message(1, 96_000, 24, opus(160, "high")));
    }
    // Something the engine handles after them, so they have all been read.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let published = obs.paths.lock().unwrap().len() - before;
    assert!(
        published < 10,
        "{published} snapshots for 40 messages that change nothing heard"
    );

    // Track 2, then the connection drops before its message came.
    for _ in 0..1000 {
        if fake.stats("STMd") >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        fake.stats("STMd") >= 1,
        "track 1 decoded: {:?} {:?} {:?}",
        fake.stats.lock().unwrap(),
        player.status(),
        fake.fetched.lock().unwrap()
    );
    fake.frame(strm(b's'));
    wait_for("stream 2 fetched", || {
        fake.fetched.lock().unwrap().contains(&2)
    })
    .await;
    fake.cut();
    wait_for("resumed", || fake.forms.lock().unwrap().len() >= 2).await;
    wait_for("the resumed connection", || {
        fake.conn.lock().unwrap().is_some()
    })
    .await;
    // Re-sent after the greeting, as the gateway does for the stream in progress.
    fake.text(&stream_message(
        2,
        48_000,
        24,
        json!({"mode": "passthrough"}),
    ));
    let o = obs.clone();
    wait_for("track 2's message in the snapshot", move || {
        o.saw_path(|p| {
            p.source_rate == 48_000
                && p.sent == Some(StreamSent::Passthrough)
                && p.served.as_ref().is_some_and(|s| s.rate == Some(48_000))
        })
    })
    .await;
    assert!(
        !obs.saw_path(
            |p| p.source_rate == 48_000 && matches!(p.sent, Some(StreamSent::Opus { .. }))
        ),
        "track 1's message never shows for track 2"
    );

    // A seek: `strm q`, then the third `strm s` (numbered 3: nothing counted twice
    // across the resume), with its own message.
    fake.frame(strm(b'q'));
    fake.frame(strm(b's'));
    wait_for("stream 3 fetched", || {
        fake.fetched.lock().unwrap().contains(&3)
    })
    .await;
    fake.text(&stream_message(3, 88_200, 24, opus(96, "low")));
    let o = obs.clone();
    wait_for("the seek's message in the snapshot", move || {
        o.saw_path(|p| {
            p.source_rate == 88_200
                && p.sent
                    == Some(StreamSent::Opus {
                        kbps: 96,
                        level: StreamQuality::Low,
                    })
        })
    })
    .await;

    // Every sign-in form, the resume's too, says protocol 2.
    let forms = fake.forms.lock().unwrap().clone();
    assert_eq!(forms.len(), 2, "{forms:?}");
    for f in &forms {
        assert_eq!(f["protocol"], json!(2), "{f}");
    }
    assert_eq!(forms[1]["resume"]["session"], SESSION);
    stop.store(true, Ordering::SeqCst);
    player.stop();
}

/// Close 4426: the player stops with `UpdateApp` (the app shows its own message, never
/// the reason) and never tries again with the same form.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_4426_stops_with_update_app_and_never_retries() {
    let fake = Arc::new(Fake::default());
    *fake.close_with.lock().unwrap() = Some((4426, "update the app"));
    let session = start(fake.clone()).await;
    let obs = Arc::new(Obs::default());
    let (player, stop) = player(session, obs.clone());
    let o = obs.clone();
    wait_for("UpdateApp", move || {
        o.statuses
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.connection == PlayerConnection::UpdateApp)
    })
    .await;
    assert_eq!(player.status().connection, PlayerConnection::UpdateApp);
    // Longer than the first reconnect back-off: nothing more is tried.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let forms = fake.forms.lock().unwrap().clone();
    assert_eq!(forms.len(), 1, "never retried: {forms:?}");
    assert_eq!(forms[0]["protocol"], json!(2));
    assert!(!player.is_running() || player.status().connection == PlayerConnection::UpdateApp);
    assert!(
        !obs.statuses
            .lock()
            .unwrap()
            .iter()
            .any(|s| matches!(&s.connection, PlayerConnection::Stopped { reason } if reason.contains("update"))),
        "the gateway's reason is never passed on as text"
    );
    stop.store(true, Ordering::SeqCst);
    player.stop();
}

/// `GET /v1/me`'s `protocol`: 2 from a current gateway, `None` from an older one (the app
/// then never opens the player socket).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_servers_protocol_is_read_from_v1_me() {
    let fake = Arc::new(Fake::default());
    let session = start(fake.clone()).await;
    assert_eq!(session.server_protocol().await, Ok(Some(2)));
    assert_eq!(session.me().await.unwrap().protocol, Some(2));
    fake.old_gateway.store(true, Ordering::SeqCst);
    assert_eq!(session.server_protocol().await, Ok(None));
    assert_eq!(session.me().await.unwrap().protocol, None);
}

/// The engine acts on `/v1/me`'s `protocol` itself (a second line behind the app's own
/// gate): a gateway that gives none, or one below 2, gets no player socket; the player
/// stops with `UpdatePlugin` and never tries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_gateway_gets_no_player_socket() {
    for old in [true, false] {
        let fake = Arc::new(Fake::default());
        let session = start(fake.clone()).await;
        if old {
            fake.old_gateway.store(true, Ordering::SeqCst);
        } else {
            fake.protocol_one.store(true, Ordering::SeqCst);
        }
        let obs = Arc::new(Obs::default());
        let (player, stop) = player(session, obs.clone());
        let o = obs.clone();
        wait_for("UpdatePlugin", move || {
            o.statuses
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.connection == PlayerConnection::UpdatePlugin)
        })
        .await;
        // Longer than the first connection's wait for the features, and a back-off.
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            fake.forms.lock().unwrap().is_empty(),
            "never signed in (old: {old})"
        );
        assert_eq!(player.status().connection, PlayerConnection::UpdatePlugin);
        assert!(
            !obs.statuses
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.connection == PlayerConnection::Connected),
            "never connected (old: {old})"
        );
        stop.store(true, Ordering::SeqCst);
        player.stop();
    }
}

/// A connected player on the scripted gateway, ready for a change of level.
async fn connected(echo: Echo) -> (Arc<Fake>, Arc<LocalPlayer>, Arc<AtomicBool>) {
    let fake = Arc::new(Fake::default());
    *fake.echo.lock().unwrap() = echo;
    let session = start(fake.clone()).await;
    let obs = Arc::new(Obs::default());
    let (player, stop) = player(session, obs.clone());
    let o = obs.clone();
    wait_for("connected", move || {
        o.statuses
            .lock()
            .unwrap()
            .last()
            .is_some_and(|s| s.connection == PlayerConnection::Connected)
    })
    .await;
    (fake, player, stop)
}

/// `set_stream_quality` as the app calls it (off the async threads), timed.
async fn change_level(player: &Arc<LocalPlayer>, q: StreamQuality) -> Duration {
    let p = player.clone();
    tokio::task::spawn_blocking(move || {
        let t = std::time::Instant::now();
        p.set_stream_quality(q);
        t.elapsed()
    })
    .await
    .unwrap()
}

/// A change of level waits for the gateway's echo, so the app's seek that follows gets a
/// stream at the new level: the echo arrives, and it returns promptly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_level_change_returns_once_the_gateway_echoes_it() {
    let (fake, player, stop) = connected(Echo::Prompt).await;
    let took = change_level(&player, StreamQuality::Low).await;
    assert!(took < Duration::from_millis(1000), "took {took:?}");
    assert_eq!(*fake.quality.lock().unwrap(), [r#"{"quality":"low"}"#]);
    // The same level again: nothing sent, nothing waited for.
    let took = change_level(&player, StreamQuality::Low).await;
    assert!(took < Duration::from_millis(200), "took {took:?}");
    assert_eq!(fake.quality.lock().unwrap().len(), 1);
    let took = change_level(&player, StreamQuality::Original).await;
    assert!(took < Duration::from_millis(1000), "took {took:?}");
    assert_eq!(fake.quality.lock().unwrap().len(), 2);
    stop.store(true, Ordering::SeqCst);
    player.stop();
}

/// No echo (a gateway that never answers): it returns after the wait, not later.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_level_change_with_no_echo_returns_after_the_wait() {
    let (fake, player, stop) = connected(Echo::Silent).await;
    let took = change_level(&player, StreamQuality::High).await;
    let wait = anywhere_mobile::QUALITY_ECHO_WAIT;
    assert!(took >= wait - Duration::from_millis(50), "took {took:?}");
    assert!(took < wait + Duration::from_millis(1500), "took {took:?}");
    assert_eq!(*fake.quality.lock().unwrap(), [r#"{"quality":"high"}"#]);
    assert_eq!(player.status().connection, PlayerConnection::Connected);
    stop.store(true, Ordering::SeqCst);
    player.stop();
}

/// Junk is not the echo: neither a malformed one nor another level's ends the wait; the
/// real echo after it does. Junk alone waits the whole time. The socket stays up.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junk_is_not_the_quality_echo() {
    let (fake, player, stop) = connected(Echo::JunkThen { ms: 600 }).await;
    let took = change_level(&player, StreamQuality::Low).await;
    assert!(
        took >= Duration::from_millis(550),
        "returned on junk: {took:?}"
    );
    assert!(
        took < Duration::from_millis(1800),
        "missed the echo: {took:?}"
    );
    *fake.echo.lock().unwrap() = Echo::Junk;
    let took = change_level(&player, StreamQuality::High).await;
    assert!(
        took >= anywhere_mobile::QUALITY_ECHO_WAIT - Duration::from_millis(50),
        "returned on junk: {took:?}"
    );
    assert_eq!(fake.quality.lock().unwrap().len(), 2);
    assert_eq!(fake.forms.lock().unwrap().len(), 1, "never reconnected");
    assert_eq!(player.status().connection, PlayerConnection::Connected);
    stop.store(true, Ordering::SeqCst);
    player.stop();
}
