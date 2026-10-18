//! Opt-in live check of A6 (docs/plans/wave3-design.md): the real engine rejoins a sync
//! group in step after a gap in its connection.
//!
//! **Running it:** `ANYWHERE_LIVE=1 cargo test -p anywhere-mobile --test live_sync -- --nocapture`
//! with `dev/local.env` (gitignored) giving `LMS_HOST`, `LMS_JSON_PORT` and
//! `TEST_PLAYER_MAC`, and optionally `SYNC_PLAYER_MAC` (default `02:4c:41:00:00:02`),
//! `LMS_SLIM_PORT` (3483) and `LMS_CLI_PORT` (9090). `ANYWHERE_LIVE_ENV` names another env
//! file. Both players must be locally administered `02:4c:41:…` test addresses. Without
//! `ANYWHERE_LIVE=1` the test prints "skipped: no live environment" and passes without
//! touching anything, so CI runs it as a no-op; `ANYWHERE_LIVE=1` without a usable env file
//! fails. A dedicated live run also sets `ANYWHERE_REQUIRE_LIVE=1`, which turns a skip into
//! a failure.
//!
//! **Set-up:** an in-process gateway in front of the real LMS, the engine behind the test
//! [`Proxy`] as the dedicated test player (its device is given `TEST_PLAYER_MAC`), and a
//! minimal fake Slimproto player as the second group member (`SYNC_PLAYER_MAC`). The fake
//! follows LMS's timing exactly (timed start, pauses and skips), so its elapsed time is the
//! group's position. Both are at volume 0. The engine keeps streams in memory (no spool), so
//! its cushion is what the core holds (10 s decoded and up to 8 MiB undecoded: about a
//! minute and a half of CD-quality FLAC) rather than the whole track; each gap lasts until the engine
//! has run dry, then for a set time more. The check takes about four minutes.
//!
//! **Phases:** for each, the tracks are loaded afresh and the group plays. Then the network
//! goes (as the app reports it, and the proxy down) until the engine has stalled, stays down
//! a set time more, and comes back:
//! 1. a short stall: the engine has enough audio to skip the stall and LMS does not restart
//!    the group;
//! 2. a long stall (more than the engine can decode ahead): `NotEnoughBuffered`, so the
//!    engine has LMS restart the group (`time`), and LMS does.
//!
//! In each, the offset between the engine's elapsed time and the fake's is measured before
//! the gap and after the rejoin (the median of 40 paired samples over 2 s) and must be within
//! [`MAX_OFFSET_MS`] after it. The engine's figure is its status from its last 20 ms tick, so
//! it reads up to 20 ms early (about 10 ms on average); the bound allows for that.
//!
//! What the other member hears is asserted too (see "A6 live check" in the design doc):
//! - LMS holds it paused (`strm p`, not a sync correction) for at most
//!   [`MAX_OTHERS_PAUSED_MS`] in either phase. Before 2026-10-15 the engine's `STMo`, held
//!   through the gap, reached LMS on reconnect and LMS paused the whole group to rebuffer
//!   it, for 4 to 15 s; a grouped engine now drops that stale report.
//! - The long stall's restart moves it by at most [`MAX_JUMP_MS`]: the engine restarts the
//!   group where the others are (its stall point plus the time stalled). Before, it used
//!   LMS's `time ?`, which lagged at the engine's own stall point, and the group went back
//!   about 14 s.
//!
//! **Clean-up**, also on failure: stop, clear the test player's queue, unsync both, restore
//! the test player's name, and `client forget` the second player until LMS no longer lists
//! it. No other player is touched: the test refuses to run if the test player is already in
//! a group, and the second player must be a locally administered `02:4c:41:…` address.

mod common;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anywhere_core::proto::{
    ClientMsg, DEVICE_ID_SOFTWARE, Helo, ServerFrameReader, Stat, Strm, format_mac, parse_mac,
};
use anywhere_gateway::accounts::{Accounts, Role};
use anywhere_gateway::{Config, Gateway, router};
use anywhere_mobile::{
    LocalPlayback, LocalPlayer, LocalPlayerConfig, LocalPlayerObserver, LocalPlayerStatus,
    OutputKind, PlayerConnection, Session, SignalPathInfo,
};
use common::{MemStore, PW, Proxy};
use futures_util::FutureExt;
use serde_json::{Value, json};

/// The largest offset accepted between the engine and the group after a rejoin.
const MAX_OFFSET_MS: i64 = 100;
/// The longest LMS may hold the other member paused around a rejoin (it should not pause it
/// at all: this allows for a stray pause unrelated to the gap).
const MAX_OTHERS_PAUSED_MS: u64 = 1_000;
/// The furthest a restart may move the other member, either way: the engine's estimate of
/// where the group is, less the round trips to LMS and LMS's own restart.
const MAX_JUMP_MS: i64 = 1_500;
/// Tracks loaded must be at least this long, so each phase stays within the first.
const MIN_TRACK_S: f64 = 240.0;

// --- Configuration ----------------------------------------------------------------------

#[derive(Clone)]
struct LiveEnv {
    host: String,
    json_port: u16,
    slim_port: u16,
    cli_port: u16,
    test_mac: String,
    sync_mac: String,
}

/// The live configuration, or `None` (with "skipped" printed) when the check isn't asked
/// for. A dedicated live run sets `ANYWHERE_REQUIRE_LIVE=1`, which makes a skip a failure,
/// so a run meant to check against LMS can't pass without doing so.
fn live_env() -> Option<LiveEnv> {
    let path = std::env::var("ANYWHERE_LIVE_ENV")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../dev/local.env").to_string());
    let asked = std::env::var("ANYWHERE_LIVE").as_deref() == Ok("1");
    let text = if asked {
        std::fs::read_to_string(&path).ok()
    } else {
        None
    };
    let required = std::env::var("ANYWHERE_REQUIRE_LIVE").as_deref() == Ok("1");
    match live_config(asked, required, &path, text.as_deref()) {
        Ok(Some(env)) => Some(env),
        Ok(None) => {
            eprintln!(
                "live_sync: skipped: no live environment (set ANYWHERE_LIVE=1 and provide \
                 dev/local.env to check the engine against a real LMS)"
            );
            None
        }
        Err(e) => panic!("live_sync: {e}"),
    }
}

/// The decision [`live_env`] makes: `Ok(None)` to skip, `Err` to fail. `asked` is
/// `ANYWHERE_LIVE=1`, `required` is `ANYWHERE_REQUIRE_LIVE=1`, and `text` is the env file
/// at `path` (`None` if there is none, or the check wasn't asked for).
fn live_config(
    asked: bool,
    required: bool,
    path: &str,
    text: Option<&str>,
) -> Result<Option<LiveEnv>, String> {
    if !asked {
        return if required {
            Err(
                "ANYWHERE_REQUIRE_LIVE=1 but the live check wasn't asked for \
                 (ANYWHERE_LIVE=1): it would have been skipped"
                    .into(),
            )
        } else {
            Ok(None)
        };
    }
    // Asked for and unable to run is a failure, never a quiet pass.
    let text = text.ok_or_else(|| format!("ANYWHERE_LIVE=1 but there is no env file at {path}"))?;
    let get = |k: &str| {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .find(|(key, _)| key.trim() == k)
            .map(|(_, v)| v.trim().to_string())
    };
    let need = |k: &str| get(k).ok_or_else(|| format!("{path} has no {k}"));
    let port = |k: &str, default: u16| match get(k) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("bad {k}")),
    };
    let mac = |v: String, k: &str| {
        parse_mac(&v)
            .map(|m| format_mac(&m))
            .ok_or_else(|| format!("{k} is not a MAC address"))
    };
    let env = LiveEnv {
        host: need("LMS_HOST")?,
        json_port: port("LMS_JSON_PORT", 9000)?,
        slim_port: port("LMS_SLIM_PORT", 3483)?,
        cli_port: port("LMS_CLI_PORT", 9090)?,
        test_mac: mac(need("TEST_PLAYER_MAC")?, "TEST_PLAYER_MAC")?,
        sync_mac: mac(
            get("SYNC_PLAYER_MAC").unwrap_or_else(|| "02:4c:41:00:00:02".into()),
            "SYNC_PLAYER_MAC",
        )?,
    };
    // Both are test players: the engine's device takes the first's address and its queue is
    // cleared, and the second is forgotten afterwards. Never let either be a real one.
    for (k, m) in [
        ("TEST_PLAYER_MAC", &env.test_mac),
        ("SYNC_PLAYER_MAC", &env.sync_mac),
    ] {
        if !m.starts_with("02:4c:41:") {
            return Err(format!(
                "{k} must be a test address (02:4c:41:…): the check takes it over and \
                 clears or removes it"
            ));
        }
    }
    if env.sync_mac == env.test_mac {
        return Err("the two test players must differ".into());
    }
    Ok(Some(env))
}

/// Not live: how the live check decides to run, skip or fail (this runs everywhere).
#[test]
fn the_live_check_runs_only_as_asked_and_only_on_test_players() {
    let good = "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=02:4c:41:00:00:01\n";
    let path = "dev/local.env";
    // Not asked: skipped, unless a live run requires it.
    assert!(matches!(live_config(false, false, path, None), Ok(None)));
    assert!(live_config(false, true, path, None).is_err());
    // Asked without an env file: a failure, not a pass.
    assert!(live_config(true, false, path, None).is_err());
    let env = live_config(true, true, path, Some(good)).unwrap().unwrap();
    assert_eq!(env.test_mac, "02:4c:41:00:00:01");
    assert_eq!(env.sync_mac, "02:4c:41:00:00:02");
    assert_eq!(
        (env.json_port, env.slim_port, env.cli_port),
        (9000, 3483, 9090)
    );
    for bad in [
        // A real player's address as either test player.
        "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=00:04:20:12:34:56\n",
        "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=02:4c:41:00:00:01\nSYNC_PLAYER_MAC=00:04:20:12:34:56\n",
        // The same player twice, a missing key, a bad port, a bad address.
        "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=02:4c:41:00:00:02\n",
        "TEST_PLAYER_MAC=02:4c:41:00:00:01\n",
        "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=02:4c:41:00:00:01\nLMS_JSON_PORT=x\n",
        "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=nope\n",
    ] {
        assert!(live_config(true, false, path, Some(bad)).is_err(), "{bad}");
    }
}

// --- LMS JSON-RPC -----------------------------------------------------------------------

/// One `slim.request` to LMS (blocking).
fn rpc(env: &LiveEnv, player: &str, cmd: &[&str]) -> Result<Value, String> {
    let body = json!({"id": 1, "method": "slim.request", "params": [player, cmd]}).to_string();
    let addr = (env.host.as_str(), env.json_port);
    let mut s = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|e| e.to_string())?
        .find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).ok())
        .ok_or("cannot reach LMS")?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let req = format!(
        "POST /jsonrpc.js HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        env.host,
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    s.read_to_end(&mut out).map_err(|e| e.to_string())?;
    let at = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| format!("no HTTP body in {:?}", String::from_utf8_lossy(&out)))?;
    let v: Value = serde_json::from_slice(&out[at + 4..]).map_err(|e| e.to_string())?;
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

async fn rpc_async(env: &LiveEnv, player: &str, cmd: &[&str]) -> Value {
    let (env, player) = (env.clone(), player.to_string());
    let cmd: Vec<String> = cmd.iter().map(|c| c.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let cmd: Vec<&str> = cmd.iter().map(String::as_str).collect();
        rpc(&env, &player, &cmd)
    })
    .await
    .unwrap()
    .unwrap_or_else(|e| panic!("LMS request failed: {e}"))
}

struct Listed {
    id: String,
    name: String,
    connected: bool,
}

/// Players LMS lists.
fn players(env: &LiveEnv) -> Vec<Listed> {
    rpc(env, "", &["players", "0", "200"])
        .ok()
        .and_then(|r| r.get("players_loop").cloned())
        .and_then(|l| l.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| {
            Some(Listed {
                id: p.get("playerid")?.as_str()?.to_lowercase(),
                name: p.get("name")?.as_str()?.to_string(),
                connected: p.get("connected").and_then(Value::as_i64) == Some(1),
            })
        })
        .collect()
}

// --- A minimal fake Slimproto player --------------------------------------------------

/// What the fake player is doing. Times are its jiffies (ms since it started).
#[derive(Default)]
struct FakeState {
    /// Streams LMS has started (`strm s`).
    streams: u32,
    /// The stream's audio started (timed start reached).
    started: bool,
    /// Playing since this jiffy (`None`: not playing).
    since: Option<u64>,
    /// Elapsed time banked before `since` (pauses, skips).
    base_ms: i64,
    loaded: bool,
    received: u64,
    /// LMS's sync corrections (`strm p`/`a` with an interval).
    corrections: Vec<String>,
    /// Elapsed just before each `strm q` (the position the group left).
    stopped_at: Vec<i64>,
    /// Paused by LMS (`strm p` without an interval) since this jiffy.
    paused_since: Option<u64>,
    /// Time spent paused by LMS so far.
    paused_ms: u64,
    /// What was received and sent (but heartbeats), with the jiffies: for a failure report.
    log: Vec<String>,
}

struct Fake {
    mac: [u8; 6],
    t0: Instant,
    w: Mutex<TcpStream>,
    st: Mutex<FakeState>,
    closed: AtomicBool,
    host: String,
}

impl Fake {
    fn start(env: &LiveEnv) -> Arc<Fake> {
        let sock = TcpStream::connect((env.host.as_str(), env.slim_port)).expect("Slimproto");
        sock.set_nodelay(true).ok();
        let fake = Arc::new(Fake {
            mac: parse_mac(&env.sync_mac).unwrap(),
            t0: Instant::now(),
            w: Mutex::new(sock.try_clone().unwrap()),
            st: Mutex::new(FakeState::default()),
            closed: AtomicBool::new(false),
            host: env.host.clone(),
        });
        fake.send(ClientMsg::Helo(Helo {
            device_id: DEVICE_ID_SOFTWARE,
            revision: 0,
            mac: fake.mac,
            uuid: [0; 16],
            wlan_channels: 0x4000,
            bytes_received: 0,
            lang: *b"en",
            capabilities: "Model=squeezelite,ModelName=SqueezeLite,AccuratePlayPoints=1,\
                           MaxSampleRate=48000,flc,mp3,ogg,pcm"
                .into(),
        }));
        let f = fake.clone();
        std::thread::spawn(move || f.read_loop(sock));
        let f = fake.clone();
        std::thread::spawn(move || {
            // Heartbeats while playing, as squeezelite sends them.
            while !f.closed.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
                if f.st.lock().unwrap().since.is_some() {
                    f.stat(*b"STMt", 0);
                }
            }
        });
        fake
    }

    fn now(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64
    }

    fn elapsed_at(st: &FakeState, now: u64) -> i64 {
        match st.since {
            Some(t) if now >= t => st.base_ms + (now - t) as i64,
            _ => st.base_ms,
        }
    }

    /// Note a step of the test in the log, with what the engine shows.
    fn note(&self, what: &str, engine: &LocalPlayerStatus) {
        let now = self.now();
        self.st.lock().unwrap().log.push(format!(
            "{now} == {what} (engine: {:?}, {} ms, buffered {} ms)",
            engine.playback, engine.elapsed_ms, engine.buffered_ms
        ));
    }

    fn elapsed(&self) -> i64 {
        Self::elapsed_at(&self.st.lock().unwrap(), self.now())
    }

    fn send(&self, m: ClientMsg) {
        let _ = self.w.lock().unwrap().write_all(&m.encode());
    }

    fn stat(&self, event: [u8; 4], server_ts: u32) {
        let now = self.now();
        let (elapsed, received) = {
            let mut st = self.st.lock().unwrap();
            let elapsed = Self::elapsed_at(&st, now).max(0) as u32;
            if &event != b"STMt" {
                st.log.push(format!(
                    "{now} -> {} at {elapsed} ms",
                    String::from_utf8_lossy(&event)
                ));
            }
            (elapsed, st.received)
        };
        // Its buffers are as full as a real player's that holds the whole track, so LMS's
        // rebuffering (after another member's underrun) finds it ready at once.
        let fullness = received.min(1 << 21) as u32;
        self.send(ClientMsg::Stat(Stat {
            event,
            stream_buffer_size: 1 << 21,
            stream_buffer_fullness: fullness,
            bytes_received: received,
            jiffies: now as u32,
            output_buffer_size: 1 << 21,
            output_buffer_fullness: fullness,
            elapsed_seconds: elapsed / 1000,
            elapsed_milliseconds: elapsed,
            server_timestamp: server_ts,
            ..Stat::default()
        }));
    }

    fn read_loop(self: Arc<Self>, mut sock: TcpStream) {
        let mut r = ServerFrameReader::default();
        let mut buf = [0u8; 8192];
        loop {
            let n = match sock.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            r.push(&buf[..n]);
            while let Ok(Some((op, p))) = r.next_frame() {
                if &op == b"strm"
                    && let Ok(s) = Strm::parse(&p)
                {
                    self.on_strm(s);
                }
            }
        }
    }

    fn on_strm(self: &Arc<Self>, s: Strm) {
        let now = self.now();
        if s.command != b't' {
            self.st.lock().unwrap().log.push(format!(
                "{now} <- strm {} autostart {} interval {}",
                char::from(s.command),
                char::from(s.autostart),
                s.replay_gain
            ));
        }
        match s.command {
            b't' => self.stat(*b"STMt", s.replay_gain),
            b's' => {
                let n = {
                    let mut st = self.st.lock().unwrap();
                    st.streams += 1;
                    st.started = false;
                    st.since = None;
                    st.base_ms = 0;
                    st.loaded = false;
                    st.received = 0;
                    st.streams
                };
                let f = self.clone();
                std::thread::spawn(move || f.fetch(n, s));
            }
            b'u' => {
                let first = {
                    let mut st = self.st.lock().unwrap();
                    let at = if s.replay_gain == 0 {
                        now
                    } else {
                        u64::from(s.replay_gain)
                    };
                    st.since = Some(at);
                    if let Some(p) = st.paused_since.take() {
                        st.paused_ms += at.saturating_sub(p);
                    }
                    !std::mem::replace(&mut st.started, true)
                };
                if first {
                    // `STMs` when the audio actually starts.
                    let wait = u64::from(s.replay_gain).saturating_sub(now);
                    let f = self.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(wait.min(30_000)));
                        f.stat(*b"STMs", 0);
                    });
                } else {
                    self.stat(*b"STMr", 0);
                }
            }
            b'p' => {
                let mut st = self.st.lock().unwrap();
                if s.replay_gain > 0 {
                    // Pause for an interval: a sync correction.
                    if let Some(t) = st.since.as_mut() {
                        *t += u64::from(s.replay_gain);
                    }
                    st.corrections.push(format!("pause {} ms", s.replay_gain));
                } else {
                    st.base_ms = Self::elapsed_at(&st, now);
                    st.since = None;
                    st.paused_since.get_or_insert(now);
                    drop(st);
                    self.stat(*b"STMp", 0);
                }
            }
            b'a' => {
                let mut st = self.st.lock().unwrap();
                st.base_ms += i64::from(s.replay_gain);
                st.corrections.push(format!("skip {} ms", s.replay_gain));
            }
            b'q' => {
                {
                    let mut st = self.st.lock().unwrap();
                    let at = Self::elapsed_at(&st, now);
                    if st.started {
                        st.stopped_at.push(at);
                    }
                    if let Some(p) = st.paused_since.take() {
                        st.paused_ms += now.saturating_sub(p);
                    }
                    st.since = None;
                    st.started = false;
                }
                self.stat(*b"STMf", 0);
            }
            b'f' => self.stat(*b"STMf", 0),
            _ => {}
        }
    }

    /// Fetch stream `n` as a player would, reporting connect, headers and loaded; the audio
    /// itself is counted and dropped.
    fn fetch(self: Arc<Self>, n: u32, s: Strm) {
        let current = |f: &Fake| f.st.lock().unwrap().streams == n;
        let ip = if s.server_ip == 0 {
            None
        } else {
            Some(Ipv4Addr::from(s.server_ip))
        };
        let conn = match ip {
            Some(ip) => TcpStream::connect(SocketAddr::from((ip, s.server_port))),
            None => TcpStream::connect((self.host.as_str(), s.server_port)),
        };
        let Ok(mut t) = conn else {
            self.send(ClientMsg::Dsco(3));
            return;
        };
        if t.write_all(&s.request).is_err() {
            return;
        }
        self.stat(*b"STMc", 0);
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match t.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => break,
            }
        }
        if !current(&self) {
            return;
        }
        self.send(ClientMsg::Resp(head));
        self.stat(*b"STMh", 0);
        let threshold = u64::from(s.threshold.max(1)) * 1024;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let got = t.read(&mut buf).unwrap_or(0);
            if !current(&self) || self.closed.load(Ordering::SeqCst) {
                return;
            }
            let load = {
                let mut st = self.st.lock().unwrap();
                st.received += got as u64;
                let due = !st.loaded && (st.received >= threshold || got == 0);
                if due {
                    st.loaded = true;
                }
                due
            };
            if load {
                if s.autostart == b'1' || s.autostart == b'3' {
                    {
                        let mut st = self.st.lock().unwrap();
                        st.started = true;
                        st.since = Some(self.now());
                    }
                    self.stat(*b"STMs", 0);
                } else {
                    self.stat(*b"STMl", 0);
                }
            }
            if got == 0 {
                break;
            }
        }
        // The decoder's `STMd` is never sent, so LMS doesn't stream the next track: each
        // phase stays within the first.
        self.send(ClientMsg::Dsco(0));
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.w.lock().unwrap().shutdown(std::net::Shutdown::Both);
    }
}

// --- The engine ---------------------------------------------------------------------------

struct NoObs;
impl LocalPlayerObserver for NoObs {
    fn on_status(&self, _: LocalPlayerStatus) {}
    fn on_signal_path(&self, _: Option<SignalPathInfo>) {}
}

struct Engine {
    proxy: Proxy,
    player: Arc<LocalPlayer>,
    stop_pull: Arc<AtomicBool>,
    _session: Arc<Session>,
}

/// The gateway (in-process, in front of the real LMS), the proxy, and the engine signed in
/// as a device whose player MAC is the test player's.
async fn start_engine(env: &LiveEnv, device_name: &str) -> Engine {
    let dir = std::env::temp_dir().join(format!(
        "anywhere-live-sync-{}",
        anywhere_gateway::random_id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("accounts.db");
    let accounts = Arc::new(Accounts::open(&db).unwrap());
    accounts.create_user("alice", PW, Role::Listener).unwrap();
    // Every device this gateway creates gets the test player's MAC (one device here).
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(&format!(
            "CREATE TRIGGER live_test_mac AFTER INSERT ON devices BEGIN \
             UPDATE devices SET mac = '{}' WHERE id = NEW.id; END;",
            env.test_mac
        ))
        .unwrap();

    let lms_ips: Vec<Ipv4Addr> = tokio::net::lookup_host((env.host.as_str(), env.slim_port))
        .await
        .expect("resolve LMS_HOST")
        .filter_map(|a| match a {
            SocketAddr::V4(v4) => Some(*v4.ip()),
            SocketAddr::V6(_) => None,
        })
        .collect();
    let cfg = Config {
        lms_slimproto: format!("{}:{}", env.host, env.slim_port),
        lms_http: format!("{}:{}", env.host, env.json_port),
        lms_cli: format!("{}:{}", env.host, env.cli_port),
        lms_ips,
        ..Config::default()
    };
    let app = router(Gateway::new(cfg, accounts));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });

    let proxy = Proxy::start(addr).await;
    let session = Session::new(proxy.server(), MemStore::new(), device_name.into()).unwrap();
    session
        .login("alice".into(), PW.into(), None)
        .await
        .unwrap();
    assert_eq!(
        session.current().unwrap().device.player_id,
        env.test_mac,
        "the device was given the test player's MAC"
    );
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 96_000,
            dsp: Default::default(),
            // In memory: the cushion is what the core holds (about a minute of CD-quality
            // FLAC), not the whole track, so a gap of a minute or so is a stall.
            cache_dir: None,
            read_ahead: false,
            stream_cache_bytes: 0,
            downloads_dir: None,
        },
    );
    player.start(Arc::new(NoObs)).unwrap();

    // Pull in real time, as an audio device would.
    let stop_pull = Arc::new(AtomicBool::new(false));
    let (p, stop) = (player.clone(), stop_pull.clone());
    tokio::spawn(async move {
        let mut started: Option<(Instant, u32)> = None;
        let mut consumed = 0u64;
        while !stop.load(Ordering::SeqCst) {
            let rate = p.status().output_rate;
            let want = match started {
                Some((t, r)) if r == rate && rate > 0 => {
                    let due = (t.elapsed().as_secs_f64() * f64::from(rate)) as u64;
                    due.saturating_sub(consumed).min(4096) as u32
                }
                _ => {
                    started = Some((Instant::now(), rate));
                    consumed = 0;
                    0
                }
            };
            if want > 0 {
                let b = p.read_pcm(want);
                if b.sample_rate > 0 {
                    consumed += b.samples.len() as u64 / 2;
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    Engine {
        proxy,
        player,
        stop_pull,
        _session: session,
    }
}

// --- The check ------------------------------------------------------------------------------

async fn wait_until(what: &str, secs: u64, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out after {secs} s waiting for {what}");
}

/// The engine's elapsed time less the fake's (ms): the median of 40 paired samples over 2 s.
async fn offset(e: &Engine, fake: &Fake) -> i64 {
    let mut d = Vec::new();
    for _ in 0..40 {
        let f = fake.elapsed();
        let s = e.player.status();
        d.push(s.elapsed_ms as i64 - f);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    d.sort_unstable();
    d[d.len() / 2]
}

/// What one phase measured.
struct Phase {
    /// The engine's elapsed less the other member's, before the gap and after the rejoin.
    before_ms: i64,
    after_ms: i64,
    /// From the engine's first underrun to it playing again.
    stall_ms: u64,
    /// LMS restarted the group (a new stream to the other member).
    restarted: bool,
    /// When it did: how far the restart point was from where the others were.
    jump_ms: Option<i64>,
    /// How long LMS held the other member paused (its rebuffering of the group).
    others_paused_ms: u64,
    /// Sync corrections LMS sent the other member after the gap.
    corrections: Vec<String>,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "offset {} ms before the gap; stalled {} ms; offset {} ms after the rejoin; \
             LMS restarted the group: {}",
            self.before_ms, self.stall_ms, self.after_ms, self.restarted
        )?;
        if let Some(j) = self.jump_ms {
            write!(f, " (moving the others {j} ms)")?;
        }
        write!(f, "; the others paused {} ms", self.others_paused_ms)?;
        write!(
            f,
            "; corrections to the other member: {:?}",
            self.corrections
        )
    }
}

/// Load the tracks, play, take the network away until the engine has been stalled for
/// `stall`, bring it back and measure.
async fn phase(env: &LiveEnv, e: &Engine, fake: &Fake, tracks: &str, stall: Duration) -> Phase {
    let tm = env.test_mac.as_str();
    let streams0 = fake.st.lock().unwrap().streams;
    rpc_async(env, tm, &["playlistcontrol", "cmd:load", tracks]).await;
    wait_until("both players to start", 30, || {
        let st = fake.st.lock().unwrap();
        let s = e.player.status();
        st.streams > streams0
            && st.started
            && s.playback == LocalPlayback::Playing
            && s.elapsed_ms > 500
    })
    .await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    let before_ms = offset(e, fake).await;
    fake.st.lock().unwrap().corrections.clear();
    let (streams_before, paused_before) = {
        let st = fake.st.lock().unwrap();
        (st.streams, st.paused_ms)
    };

    let underruns = e.player.status().underruns;
    // As the app does: the network is gone (the engine stops trying), then back (it
    // reconnects at once).
    e.player.set_network_available(false);
    e.proxy.set_down(true);
    fake.note("network down", &e.player.status());
    wait_until("the engine to stall", 180, || {
        e.player.status().underruns > underruns
    })
    .await;
    fake.note("the engine stalled", &e.player.status());
    let stalled_at = Instant::now();
    tokio::time::sleep(stall).await;
    fake.note("network back", &e.player.status());
    e.proxy.set_down(false);
    e.player.set_network_available(true);
    let at_restore = e.player.status().elapsed_ms;
    wait_until("the engine to play again", 60, || {
        let s = e.player.status();
        s.connection == PlayerConnection::Connected
            && s.playback == LocalPlayback::Playing
            && (s.elapsed_ms > at_restore + 300 || s.elapsed_ms + 5_000 < at_restore)
    })
    .await;
    fake.note("the engine plays again", &e.player.status());
    let stall_ms = stalled_at.elapsed().as_millis() as u64;
    // Let LMS's own small corrections land, then measure.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let after_ms = offset(e, fake).await;
    fake.note("measured", &e.player.status());
    let (others_paused_ms, restarted, jump_ms, corrections) = {
        let st = fake.st.lock().unwrap();
        let restarted = st.streams > streams_before;
        (
            st.paused_ms - paused_before,
            restarted,
            restarted.then(|| st.stopped_at.last().copied()).flatten(),
            st.corrections.clone(),
        )
    };
    let lms_time_s = rpc_async(env, tm, &["time", "?"])
        .await
        .get("_time")
        .and_then(|t| t.as_f64().or_else(|| t.as_str()?.parse().ok()))
        .unwrap_or(-1.0);
    let jump_ms = jump_ms.map(|left| (lms_time_s * 1000.0) as i64 - fake.elapsed() - left);
    Phase {
        before_ms,
        after_ms,
        stall_ms,
        restarted,
        jump_ms,
        others_paused_ms,
        corrections,
    }
}

async fn run(env: &LiveEnv, e: &Engine, fake: &Fake) {
    let (tm, sm) = (env.test_mac.as_str(), env.sync_mac.as_str());
    wait_until("LMS to see both players", 20, || {
        let p = players(env);
        [tm, sm]
            .iter()
            .all(|m| p.iter().any(|l| l.id == *m && l.connected))
    })
    .await;
    for m in [tm, sm] {
        rpc_async(env, m, &["mixer", "volume", "0"]).await;
    }
    rpc_async(env, tm, &["sync", sm]).await;

    let titles = rpc_async(env, "", &["titles", "0", "300", "tags:d"]).await;
    let ids: Vec<String> = titles
        .get("titles_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("duration").and_then(Value::as_f64).unwrap_or(0.0) >= MIN_TRACK_S)
        .filter_map(|t| {
            t.get("id")
                .map(|i| i.to_string().trim_matches('"').to_string())
        })
        .take(3)
        .collect();
    assert!(
        ids.len() == 3,
        "the library needs three tracks of {MIN_TRACK_S} s or more"
    );
    let tracks = format!("track_id:{}", ids.join(","));

    let short = phase(env, e, fake, &tracks, Duration::from_secs(3)).await;
    eprintln!("live_sync: short stall: {short}");
    let long = phase(env, e, fake, &tracks, Duration::from_secs(14)).await;
    eprintln!("live_sync: long stall: {long}");

    assert!(
        !short.restarted,
        "a short stall is skipped in place, without LMS restarting the group: {short}"
    );
    assert!(
        short.after_ms.abs() <= MAX_OFFSET_MS,
        "after a short stall the engine is within {MAX_OFFSET_MS} ms of the group: {short}"
    );
    assert!(
        long.restarted,
        "a stall longer than the audio decoded has LMS restart the group (NotEnoughBuffered): \
         {long}"
    );
    assert!(
        long.after_ms.abs() <= MAX_OFFSET_MS,
        "after LMS restarted the group the engine is within {MAX_OFFSET_MS} ms of it: {long}"
    );
    for (what, p) in [("short", &short), ("long", &long)] {
        assert!(
            p.others_paused_ms <= MAX_OTHERS_PAUSED_MS,
            "after a {what} stall LMS doesn't pause the others to rebuffer (at most \
             {MAX_OTHERS_PAUSED_MS} ms): {p}"
        );
    }
    let jump = long
        .jump_ms
        .unwrap_or_else(|| panic!("the restart point was measured: {long}"));
    assert!(
        jump.abs() <= MAX_JUMP_MS,
        "the restart leaves the others where they were (within {MAX_JUMP_MS} ms), not back at \
         the engine's stall point: {long}"
    );
}

/// Undo everything, on success or failure. Touches only the two test players.
async fn clean_up(env: &LiveEnv, e: Option<&Engine>, fake: Option<&Fake>, name: Option<String>) {
    let env2 = env.clone();
    let (tm, sm) = (env.test_mac.clone(), env.sync_mac.clone());
    tokio::task::spawn_blocking(move || {
        let _ = rpc(&env2, &tm, &["stop"]);
        let _ = rpc(&env2, &tm, &["playlist", "clear"]);
        let _ = rpc(&env2, &sm, &["sync", "-"]);
        let _ = rpc(&env2, &tm, &["sync", "-"]);
        if let Some(n) = &name {
            let _ = rpc(&env2, &tm, &["name", n]);
        }
    })
    .await
    .unwrap();
    if let Some(e) = e {
        e.stop_pull.store(true, Ordering::SeqCst);
        e.player.stop();
    }
    if let Some(f) = fake {
        f.close();
    }
    let env2 = env.clone();
    let gone = tokio::task::spawn_blocking(move || {
        for _ in 0..15 {
            std::thread::sleep(Duration::from_secs(1));
            let _ = rpc(&env2, &env2.sync_mac, &["client", "forget"]);
            if !players(&env2).iter().any(|p| p.id == env2.sync_mac) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap();
    assert!(gone, "the second test player was removed from LMS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_rejoins_a_sync_group_in_step_after_a_gap() {
    let Some(env) = live_env() else { return };
    let tm = env.test_mac.clone();
    // The test player's name, to put back (the engine answers LMS's name query with its
    // device's name). LMS answers player commands only while the player is connected.
    let name = players(&env)
        .into_iter()
        .find(|p| p.id == tm)
        .map(|p| p.name);

    // The device is named as the test player already is, so answering LMS's name query
    // changes nothing.
    let device_name = name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Anywhere test player".into());
    let e = start_engine(&env, &device_name).await;
    let connected = AssertUnwindSafe(wait_until("LMS to see the engine", 20, || {
        players(&env).iter().any(|p| p.id == tm && p.connected)
    }))
    .catch_unwind()
    .await;
    // Never touch a group with other players in it.
    let status = rpc_async(&env, &tm, &["status", "-", "1"]).await;
    let grouped = ["sync_master", "sync_slaves"].iter().any(|k| {
        status
            .get(*k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    });
    if connected.is_err() || grouped {
        e.stop_pull.store(true, Ordering::SeqCst);
        e.player.stop();
        assert!(
            !grouped,
            "the test player is already in a sync group: left alone"
        );
        panic!("LMS never saw the engine connect as the test player");
    }

    let fake = Fake::start(&env);
    let outcome = AssertUnwindSafe(run(&env, &e, &fake)).catch_unwind().await;
    if outcome.is_err() {
        eprintln!("live_sync: the engine: {:?}", e.player.status());
        let lms = rpc_async(&env, &tm, &["status", "-", "1"]).await;
        eprintln!(
            "live_sync: LMS: mode {} time {} sync {} {}",
            lms["mode"], lms["time"], lms["sync_master"], lms["sync_slaves"]
        );
    }
    // Shown on failure, or with `--nocapture`.
    eprintln!("live_sync: the other member's log (jiffies in ms):");
    for l in &fake.st.lock().unwrap().log {
        eprintln!("  {l}");
    }
    clean_up(&env, Some(&e), Some(&fake), name).await;
    if let Err(p) = outcome {
        std::panic::resume_unwind(p);
    }
}
