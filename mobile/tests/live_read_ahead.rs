//! Opt-in live check of read-ahead (docs/ARCHITECTURE.md §5.10) against a real LMS: with a
//! spool folder and the setting on, the engine has the whole of the current track and then
//! the whole of the next one on the phone long before the current one ends.
//!
//! **Running it:** `ANYWHERE_LIVE=1 cargo test -p anywhere-mobile --test live_read_ahead --
//! --nocapture` with `dev/local.env` (gitignored) giving `LMS_HOST`, `LMS_JSON_PORT` and
//! `TEST_PLAYER_MAC`, and optionally `LMS_SLIM_PORT` (3483) and `LMS_CLI_PORT` (9090).
//! `ANYWHERE_LIVE_ENV` names another env file. Without `ANYWHERE_LIVE=1` the test prints
//! "skipped: no live environment" and passes without touching anything, so CI runs it as a
//! no-op; `ANYWHERE_LIVE=1` without a usable env file fails, and `ANYWHERE_REQUIRE_LIVE=1`
//! turns a skip into a failure.
//!
//! **Set-up:** an in-process gateway in front of the real LMS and the engine behind the test
//! [`Proxy`] as the dedicated test player (its device is given `TEST_PLAYER_MAC`), at volume
//! 0, with a fresh spool folder and read-ahead on, at the *Original* level (so the library's
//! own files pass through untouched: no transcoding). Two tracks of [`MIN_TRACK_S`] or more
//! are loaded on the test player and it plays.
//!
//! **What is measured** from the engine's status (`cushion_ms`: the decoded audio plus
//! every byte not yet decoded, spooled or inside the core, at the byte rate of the track
//! being decoded), every half second from the start of play:
//! 1. when the rest of track 1 is on the phone (`elapsed + cushion` reaches the track's
//!    length, within [`RATE_SLACK`] for the byte-rate estimate);
//! 2. when the next track is being fetched (the cushion exceeds the rest of track 1 by
//!    [`NEXT_FRACTION`] of track 2), which can only happen once LMS has sent it on the
//!    engine's early `STMd` (after [`anywhere_core::player::DEFAULT_READY_AFTER_MS`]
//!    heard);
//! 3. when the whole of track 2 is on the phone too: its spool file holds about the audio
//!    bytes LMS reports for it, or the cushion counts them (track 2's bytes at track 1's
//!    rate, within [`RATE_SLACK`]).
//!
//! Each must happen within its bound ([`NEXT_WITHIN_S`], [`FULL_WITHIN_S`]), and the
//! figure at the end is the rest of track 1 plus track 2 at track 1's rate, within the
//! slack (#30: it left out the next track once the core held it); the samples are printed
//! on failure or with `--nocapture`.
//!
//! Tracks whose file fits the core's input whole
//! ([`anywhere_core::player::INPUT_HIGH_WATER`]) are preferred: the usual MP3 or Opus
//! track, whose next track the core holds in its queued slot rather than the spool, which
//! is where #30 hid. Without two such tracks, any two long enough.
//!
//! **Clean-up**, also on failure: stop, clear the test player's queue, put its name back,
//! stop the engine and delete the spool folder. No other player is touched: the test
//! refuses to run if the test player is in a sync group.

mod common;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anywhere_core::player::INPUT_HIGH_WATER;
use anywhere_core::proto::{format_mac, parse_mac};
use anywhere_gateway::accounts::{Accounts, Role};
use anywhere_gateway::{Config, Gateway, router};
use anywhere_mobile::{
    LocalPlayback, LocalPlayer, LocalPlayerConfig, LocalPlayerObserver, LocalPlayerStatus,
    OutputKind, PlayerConnection, Session, SignalPathInfo,
};
use common::{MemStore, PW, Proxy};
use futures_util::FutureExt;
use serde_json::{Value, json};

/// Tracks shorter than this are not used (the check needs time to show the next track
/// arriving well before the end).
const MIN_TRACK_S: f64 = 240.0;
/// The cushion is bytes at the byte rate of the track so far, and a FLAC or VBR track's
/// rate varies, so "the rest of track 1" is this fraction of its length or more; likewise
/// track 2's spool against LMS's average bitrate over its length, and the figure at the
/// end against both tracks.
const RATE_SLACK: f64 = 0.85;
/// The figure at the end may exceed what LMS's averages give by this much: early in a VBR
/// track the rate so far can sit well under the track's average (seen: 8% on an MP3).
const RATE_DRIFT: f64 = 1.35;
/// The next track counts as being fetched once the cushion exceeds the rest of the current
/// one by this fraction of track 2 (at track 1's rate): a jump no drift of the rate gives.
const NEXT_FRACTION: f64 = 0.5;
/// The next track must be arriving within this long of play starting: 20 s heard (the
/// default) plus the fetch of the rest of track 1 and the first of track 2.
const NEXT_WITHIN_S: u64 = 90;
/// Both tracks must be on the phone whole within this long of play starting.
const FULL_WITHIN_S: u64 = 150;

// --- Configuration ----------------------------------------------------------------------

#[derive(Clone)]
struct LiveEnv {
    host: String,
    json_port: u16,
    slim_port: u16,
    cli_port: u16,
    test_mac: String,
}

/// The live configuration, or `None` (with "skipped" printed) when the check isn't asked
/// for.
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
                "live_read_ahead: skipped: no live environment (set ANYWHERE_LIVE=1 and \
                 provide dev/local.env to check read-ahead against a real LMS)"
            );
            None
        }
        Err(e) => panic!("live_read_ahead: {e}"),
    }
}

/// The decision [`live_env`] makes: `Ok(None)` to skip, `Err` to fail.
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
    let test_mac = parse_mac(&need("TEST_PLAYER_MAC")?)
        .map(|m| format_mac(&m))
        .ok_or("TEST_PLAYER_MAC is not a MAC address")?;
    // The engine's device takes the test player's address and its queue is cleared: never a
    // real player.
    if !test_mac.starts_with("02:4c:41:") {
        return Err(
            "TEST_PLAYER_MAC must be a test address (02:4c:41:…): the check takes it over \
             and clears it"
                .into(),
        );
    }
    Ok(Some(LiveEnv {
        host: need("LMS_HOST")?,
        json_port: port("LMS_JSON_PORT", 9000)?,
        slim_port: port("LMS_SLIM_PORT", 3483)?,
        cli_port: port("LMS_CLI_PORT", 9090)?,
        test_mac,
    }))
}

/// Not live: how the live check decides to run, skip or fail (this runs everywhere).
#[test]
fn the_live_check_runs_only_as_asked_and_only_on_a_test_player() {
    let path = "dev/local.env";
    assert!(matches!(live_config(false, false, path, None), Ok(None)));
    assert!(live_config(false, true, path, None).is_err());
    assert!(live_config(true, false, path, None).is_err());
    let good = "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=02:4c:41:00:00:01\n";
    let env = live_config(true, false, path, Some(good)).unwrap().unwrap();
    assert_eq!(env.test_mac, "02:4c:41:00:00:01");
    assert_eq!(
        (env.json_port, env.slim_port, env.cli_port),
        (9000, 3483, 9090)
    );
    let real = "LMS_HOST=lms.example.lan\nTEST_PLAYER_MAC=aa:bb:cc:dd:ee:01\n";
    assert!(live_config(true, false, path, Some(real)).is_err());
    assert!(live_config(true, false, path, Some("LMS_HOST=x\n")).is_err());
}

// --- LMS -----------------------------------------------------------------------------

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

// --- The engine --------------------------------------------------------------------------

struct NoObs;
impl LocalPlayerObserver for NoObs {
    fn on_status(&self, _: LocalPlayerStatus) {}
    fn on_signal_path(&self, _: Option<SignalPathInfo>) {}
}

struct Engine {
    _proxy: Proxy,
    player: Arc<LocalPlayer>,
    stop_pull: Arc<AtomicBool>,
    _session: Arc<Session>,
    spool_root: std::path::PathBuf,
}

/// The gateway (in-process, in front of the real LMS), the proxy, and the engine signed in
/// as a device whose player MAC is the test player's, spooling to a fresh folder with
/// read-ahead on.
async fn start_engine(env: &LiveEnv, device_name: &str) -> Engine {
    let dir = std::env::temp_dir().join(format!(
        "anywhere-live-read-ahead-{}",
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
    let spool_root = dir.join("stream-cache");
    std::fs::create_dir_all(&spool_root).unwrap();
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 96_000,
            dsp: Default::default(),
            // As the Android app runs: spooling to disk, reading the next track ahead.
            cache_dir: Some(spool_root.to_string_lossy().into_owned()),
            read_ahead: true,
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
        _proxy: proxy,
        player,
        stop_pull,
        _session: session,
        spool_root,
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

/// A sample of the engine's status, `at` ms after play started.
struct Sample {
    at: u64,
    elapsed_ms: u64,
    cushion_ms: u64,
    buffered_ms: u64,
    read_ahead: bool,
    /// The spool files on disk, (stream number, bytes), oldest stream first.
    spools: Vec<(u64, u64)>,
}

impl std::fmt::Display for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "t={:>5.1}s elapsed={:>5.1}s cushion={:>6.1}s decoded={:>4.1}s read_ahead={} spools={:?}",
            self.at as f64 / 1000.0,
            self.elapsed_ms as f64 / 1000.0,
            self.cushion_ms as f64 / 1000.0,
            self.buffered_ms as f64 / 1000.0,
            self.read_ahead,
            self.spools
        )
    }
}

/// Spool files under `root` (any depth): (stream number from the file's name, bytes),
/// oldest stream first.
fn spool_files(root: &std::path::Path) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "spool") {
                let n = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(u64::MAX);
                let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                out.push((n, len));
            }
        }
    }
    out.sort_unstable();
    out
}

/// What LMS says of a track: its length (ms), about how many bytes of audio it holds (its
/// average bitrate over its length; the file's size when the bitrate isn't given) and the
/// file's size (0 when LMS doesn't say).
struct Track {
    id: String,
    duration_ms: u64,
    audio_bytes: u64,
    file_bytes: u64,
}

fn track_of(t: &Value) -> Option<Track> {
    let d = t.get("duration").and_then(Value::as_f64)?;
    let id = t
        .get("id")
        .map(|i| i.to_string().trim_matches('"').to_string())?;
    // A number, or a string of digits.
    let filesize = match t.get("filesize") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    };
    // "1,017kbps" or a number (bps).
    let bps = match t.get("bitrate") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let digits: String = s.chars().filter(char::is_ascii_digit).collect();
            digits.parse::<f64>().ok().map(|k| k * 1000.0)
        }
        _ => None,
    };
    let audio_bytes = match bps {
        Some(b) if b > 0.0 => (b / 8.0 * d) as u64,
        _ => filesize,
    };
    Some(Track {
        id,
        duration_ms: (d * 1000.0) as u64,
        audio_bytes,
        file_bytes: filesize,
    })
}

struct Outcome {
    /// Seconds after play started when the rest of track 1 was on the phone.
    track1_whole_s: Option<f64>,
    /// … when track 2 was arriving (the cushion exceeded the rest of track 1).
    next_fetched_s: Option<f64>,
    /// … when track 2 was on the phone whole too.
    track2_whole_s: Option<f64>,
    samples: Vec<Sample>,
    /// Track 1's length, and track 2 at track 1's byte rate (ms): what the cushion should
    /// show at the end, less what has been heard of track 1.
    d1_ms: u64,
    expect2_ms: f64,
}

/// Load the two tracks, play, and sample the engine until both tracks are on the phone or
/// the bound passes.
async fn run(env: &LiveEnv, e: &Engine) -> Outcome {
    let tm = env.test_mac.as_str();
    wait_until("LMS to see the engine", 20, || {
        players(env).iter().any(|p| p.id == tm && p.connected)
    })
    .await;
    rpc_async(env, tm, &["mixer", "volume", "0"]).await;

    let titles = rpc_async(env, "", &["titles", "0", "300", "tags:dfr"]).await;
    let long: Vec<Track> = titles
        .get("titles_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(track_of)
        .filter(|t| t.duration_ms >= (MIN_TRACK_S * 1000.0) as u64)
        .collect();
    // The reporter's case first: tracks the core takes in whole (see the module doc).
    let fits = |t: &&Track| t.file_bytes > 0 && t.file_bytes < INPUT_HIGH_WATER as u64;
    let small: Vec<&Track> = long.iter().filter(fits).take(2).collect();
    let picked: Vec<&Track> = if small.len() == 2 {
        small
    } else {
        long.iter().take(2).collect()
    };
    assert!(
        picked.len() == 2,
        "the library needs two tracks of {MIN_TRACK_S} s or more"
    );
    let (d1, d2) = (picked[0].duration_ms, picked[1].duration_ms);
    let (bytes1, bytes2) = (picked[0].audio_bytes, picked[1].audio_bytes);
    // Track 2 at track 1's average byte rate: what the cushion makes of its bytes while
    // track 1 is the track being decoded.
    let expect2_ms = if bytes1 > 0 {
        bytes2 as f64 * d1 as f64 / bytes1 as f64
    } else {
        d2 as f64
    };
    eprintln!(
        "live_read_ahead: track 1 is {:.0} s ({} bytes of audio, file {} bytes), track 2 is \
         {:.0} s ({} bytes, file {} bytes){}",
        d1 as f64 / 1000.0,
        bytes1,
        picked[0].file_bytes,
        d2 as f64 / 1000.0,
        bytes2,
        picked[1].file_bytes,
        if picked.iter().all(fits) {
            "; both fit the core's input whole"
        } else {
            ""
        }
    );
    let tracks = format!("track_id:{},{}", picked[0].id, picked[1].id);
    rpc_async(env, tm, &["playlistcontrol", "cmd:load", &tracks]).await;
    wait_until("the engine to play", 30, || {
        let s = e.player.status();
        s.playback == LocalPlayback::Playing && s.elapsed_ms > 500
    })
    .await;
    let start = Instant::now();
    let mut out = Outcome {
        track1_whole_s: None,
        next_fetched_s: None,
        track2_whole_s: None,
        samples: Vec::new(),
        d1_ms: d1,
        expect2_ms,
    };
    while start.elapsed() < Duration::from_secs(FULL_WITHIN_S) {
        let s = e.player.status();
        let at = start.elapsed().as_millis() as u64;
        let sample = Sample {
            at,
            elapsed_ms: s.elapsed_ms,
            cushion_ms: s.cushion_ms,
            buffered_ms: s.buffered_ms,
            read_ahead: s.read_ahead,
            spools: spool_files(&e.spool_root),
        };
        let secs = at as f64 / 1000.0;
        let rest1 = d1.saturating_sub(s.elapsed_ms);
        if out.track1_whole_s.is_none()
            && s.elapsed_ms + s.cushion_ms >= (d1 as f64 * RATE_SLACK) as u64
        {
            out.track1_whole_s = Some(secs);
        }
        if out.next_fetched_s.is_none()
            && s.cushion_ms as f64 >= rest1 as f64 + expect2_ms * NEXT_FRACTION
        {
            out.next_fetched_s = Some(secs);
        }
        // Track 2 is whole once its spool (the newer stream's file) holds about the
        // track's audio bytes, or once the cushion counts them: a track the core takes in
        // whole leaves its spool within a second of arriving.
        let spooled_whole = sample.spools.len() >= 2
            && sample
                .spools
                .last()
                .is_some_and(|&(_, len)| len >= (bytes2 as f64 * RATE_SLACK) as u64);
        let counted_whole = s.cushion_ms as f64 >= rest1 as f64 + expect2_ms * RATE_SLACK;
        if out.track2_whole_s.is_none() && (spooled_whole || counted_whole) {
            out.track2_whole_s = Some(secs);
        }
        out.samples.push(sample);
        if out.track2_whole_s.is_some() {
            break;
        }
        assert!(
            s.connection == PlayerConnection::Connected,
            "the engine stayed connected: {s:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    out
}

/// Undo everything, on success or failure. Touches only the test player.
async fn clean_up(env: &LiveEnv, e: Option<&Engine>, name: Option<String>) {
    let env2 = env.clone();
    let tm = env.test_mac.clone();
    tokio::task::spawn_blocking(move || {
        let _ = rpc(&env2, &tm, &["stop"]);
        let _ = rpc(&env2, &tm, &["playlist", "clear"]);
        if let Some(n) = &name {
            let _ = rpc(&env2, &tm, &["name", n]);
        }
    })
    .await
    .unwrap();
    if let Some(e) = e {
        e.stop_pull.store(true, Ordering::SeqCst);
        e.player.stop();
        let _ = std::fs::remove_dir_all(e.spool_root.parent().unwrap_or(&e.spool_root));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_ahead_puts_the_next_track_on_the_phone_long_before_the_current_one_ends() {
    let Some(env) = live_env() else { return };
    let tm = env.test_mac.clone();
    let name = players(&env)
        .into_iter()
        .find(|p| p.id == tm)
        .map(|p| p.name);
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
        clean_up(&env, Some(&e), name).await;
        assert!(
            !grouped,
            "the test player is already in a sync group: left alone"
        );
        panic!("LMS never saw the engine connect as the test player");
    }

    let outcome = AssertUnwindSafe(run(&env, &e)).catch_unwind().await;
    let lms = rpc_async(&env, &tm, &["status", "-", "1"]).await;
    eprintln!(
        "live_read_ahead: LMS at the end: mode {} time {} index {}; the engine: {:?}",
        lms["mode"],
        lms["time"],
        lms["playlist_cur_index"],
        e.player.status()
    );
    clean_up(&env, Some(&e), name).await;
    let out = match outcome {
        Ok(o) => o,
        Err(p) => std::panic::resume_unwind(p),
    };
    // Shown on failure, or with `--nocapture`.
    eprintln!("live_read_ahead: samples:");
    for s in &out.samples {
        eprintln!("  {s}");
    }
    let shown = |v: Option<f64>| v.map_or("never".to_string(), |s| format!("{s:.1} s"));
    eprintln!(
        "live_read_ahead: track 1 whole on the phone after {}; track 2 arriving after {}; \
         track 2 whole after {}",
        shown(out.track1_whole_s),
        shown(out.next_fetched_s),
        shown(out.track2_whole_s)
    );
    assert!(
        out.samples.iter().all(|s| s.read_ahead),
        "read-ahead was in force throughout"
    );
    let whole1 = out
        .track1_whole_s
        .unwrap_or_else(|| panic!("the rest of track 1 never arrived in full"));
    let next = out.next_fetched_s.unwrap_or_else(|| {
        panic!("track 2 was never fetched while track 1 played (LMS never sent it early)")
    });
    assert!(
        next <= NEXT_WITHIN_S as f64,
        "track 2 was arriving within {NEXT_WITHIN_S} s of play starting (after {next:.1} s)"
    );
    assert!(
        whole1 <= next,
        "track 1 was whole ({whole1:.1} s) before track 2 was fetched ({next:.1} s)"
    );
    let whole2 = out
        .track2_whole_s
        .unwrap_or_else(|| panic!("track 2 never arrived in full within {FULL_WITHIN_S} s"));
    assert!(whole2 <= FULL_WITHIN_S as f64);
    // The figure itself (#30): with both tracks on the phone, "buffered ahead" is the rest
    // of track 1 plus track 2 at track 1's rate, within the slack either way.
    let last = out.samples.last().expect("a sample");
    let expect = out.d1_ms.saturating_sub(last.elapsed_ms) as f64 + out.expect2_ms;
    let shown = last.cushion_ms as f64;
    assert!(
        shown >= expect * RATE_SLACK && shown <= expect * RATE_DRIFT,
        "the cushion counts the rest of track 1 and all of track 2: {:.1} s shown, about \
         {:.1} s held",
        shown / 1000.0,
        expect / 1000.0
    );
}
