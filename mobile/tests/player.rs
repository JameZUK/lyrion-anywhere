//! This phone as an LMS player, end to end: the real gateway (in-process) in front of a
//! fake LMS that speaks Slimproto and serves `/stream.mp3`, and the engine with a pull
//! output standing in for the audio device.
//!
//! Covered: gapless, bit-perfect playback of FLAC to the end of the playlist with the
//! right STAT events; a source-rate change reopening the output; resuming after the
//! connection drops mid-stream (LMS never sees the player leave, and no audio is lost or
//! repeated); token expiry; refusals (unsupported formats, streams the gateway's policy
//! refuses, server redirects); hostile frames from LMS; stopping and signing out.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_core::proto::{Strm, encode_server_frame, format_mac};
use anywhere_mobile::{
    DspSettings, LocalPlayback, LocalPlayer, LocalPlayerConfig, LocalPlayerObserver,
    LocalPlayerStatus, MixerOffer, NotBitPerfect, OfferBlocker, OutputKind, OutputNegotiator,
    OutputOffers, PcmEncoding, PlayerConnection, Server, Session, SignalPathInfo, SignalVerdict,
};
use common::flac;
use common::*;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;

#[derive(Default)]
struct Obs {
    statuses: Mutex<Vec<LocalPlayerStatus>>,
    paths: Mutex<Vec<Option<SignalPathInfo>>>,
}

impl LocalPlayerObserver for Obs {
    fn on_status(&self, status: LocalPlayerStatus) {
        self.statuses.lock().unwrap().push(status);
    }
    fn on_signal_path(&self, path: Option<SignalPathInfo>) {
        self.paths.lock().unwrap().push(path);
    }
}

impl Obs {
    fn saw(&self, f: impl Fn(&LocalPlayerStatus) -> bool) -> bool {
        self.statuses.lock().unwrap().iter().any(f)
    }
    fn last_path(&self) -> Option<SignalPathInfo> {
        self.paths
            .lock()
            .unwrap()
            .iter()
            .rev()
            .flatten()
            .next()
            .cloned()
    }
}

/// Audio pulled from the player, as (sample rate, samples) runs: float, or integers in
/// the output's format (as a DAC receives them) when pulling integers.
#[derive(Default)]
struct Pulled {
    runs: Mutex<Vec<(u32, Vec<f32>)>>,
    int_runs: Mutex<Vec<(u32, PcmEncoding, Vec<i32>)>>,
}

impl Pulled {
    fn frames(&self) -> usize {
        let f: usize = self
            .runs
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.len() / 2)
            .sum();
        let i: usize = self
            .int_runs
            .lock()
            .unwrap()
            .iter()
            .map(|(_, _, s)| s.len() / 2)
            .sum();
        f + i
    }
    fn all(&self) -> Vec<f32> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, s)| s.clone())
            .collect()
    }
}

struct Rig {
    h: Harness,
    proxy: Option<Proxy>,
    session: Arc<Session>,
    player: Arc<LocalPlayer>,
    obs: Arc<Obs>,
    lms: OwnedWriteHalf,
    mac: [u8; 6],
    pulled: Arc<Pulled>,
    stop_pull: Arc<AtomicBool>,
    /// While set, nothing is pulled (the output is held up).
    pause_pull: Arc<AtomicBool>,
    /// The cache folder, if the player spools.
    cache: Option<std::path::PathBuf>,
}

/// How a rig is set up (see [`rig_opts`]).
#[derive(Default)]
struct RigOpts {
    proxy: bool,
    /// The proxy hides the gateway's `resume_grace` feature (an older gateway).
    old_gateway: bool,
    negotiator: Option<Arc<dyn OutputNegotiator>>,
    dsp: DspSettings,
    /// Spool streams to a fresh cache folder (read-ahead on).
    spool: bool,
    /// The gateway's `max_resume_grace`.
    max_grace_s: Option<u64>,
    /// Pull at this many times real time (`None`: as fast as the engine fills).
    pace: Option<f64>,
    /// Read-ahead reports a track ready after this much is heard.
    ready_after_ms: Option<u64>,
    /// Start with pulling paused.
    paused: bool,
}

async fn rig(with_proxy: bool) -> Rig {
    rig_with(with_proxy, None, DspSettings::default()).await
}

/// A rig whose player negotiates its output with `negotiator` (a stand-in for Android's
/// `AudioManager`) and is pulled as integers when one is given.
async fn rig_with(
    with_proxy: bool,
    negotiator: Option<Arc<dyn OutputNegotiator>>,
    dsp: DspSettings,
) -> Rig {
    rig_opts(RigOpts {
        proxy: with_proxy,
        negotiator,
        dsp,
        ..RigOpts::default()
    })
    .await
}

/// A fresh, empty cache folder for spools.
fn temp_cache() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "anywhere-player-cache-{}-{}",
        std::process::id(),
        anywhere_gateway::random_id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Every spool file under `root` (any depth).
fn spool_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "spool") {
                out.push(p);
            }
        }
    }
    out
}

async fn rig_opts(o: RigOpts) -> Rig {
    let pull_ints = o.negotiator.is_some();
    let h = match o.max_grace_s {
        Some(s) => Harness::start_with(|c| c.max_resume_grace = Duration::from_secs(s)).await,
        None => Harness::start().await,
    };
    let proxy = if o.proxy || o.old_gateway {
        let rewrite = o.old_gateway.then(|| {
            (
                b"\"resume_grace\":true".to_vec(),
                b"\"resume_grace\":null".to_vec(),
            )
        });
        Some(Proxy::start_rewriting(h.addr, rewrite).await)
    } else {
        None
    };
    let server: Arc<Server> = match &proxy {
        Some(p) => p.server(),
        None => h.server(),
    };
    let session = Session::new(server, MemStore::new(), "Test phone".into()).unwrap();
    session
        .login("alice".into(), PW.into(), None)
        .await
        .unwrap();
    let cache = o.spool.then(temp_cache);
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 192_000,
            cache_dir: cache.as_ref().map(|c| c.to_string_lossy().into_owned()),
            read_ahead: true,
            dsp: o.dsp,
        },
    );
    player.set_output_negotiator(o.negotiator);
    if let Some(ms) = o.ready_after_ms {
        player.debug_set_ready_after_ms(ms);
    }
    let obs = Arc::new(Obs::default());
    player.start(obs.clone()).unwrap();
    let lms = h.lms.slim_conn().await;
    wait_for("HELO", || {
        h.lms.slim_events().first().map(String::as_str) == Some("HELO")
    })
    .await;
    let helo = &h.lms.slim_payloads(b"HELO")[0];
    let mac: [u8; 6] = helo[2..8].try_into().unwrap();

    // Pull like an audio device: a little faster than real time, or paced.
    let pulled = Arc::new(Pulled::default());
    let stop_pull = Arc::new(AtomicBool::new(false));
    let pause_pull = Arc::new(AtomicBool::new(o.paused));
    let (p, out, stop, pause) = (
        player.clone(),
        pulled.clone(),
        stop_pull.clone(),
        pause_pull.clone(),
    );
    let pace = o.pace;
    tokio::spawn(async move {
        // Paced: the device frames consumed so far, and since when.
        let mut started: Option<(std::time::Instant, u32)> = None;
        let mut consumed = 0u64;
        while !stop.load(Ordering::SeqCst) {
            if pause.load(Ordering::SeqCst) {
                started = None;
                tokio::time::sleep(Duration::from_millis(4)).await;
                continue;
            }
            let mut want = 2048u32;
            if let Some(speed) = pace {
                let rate = p.status().output_rate;
                match started {
                    Some((t, r)) if r == rate && rate > 0 => {
                        let due = (t.elapsed().as_secs_f64() * f64::from(rate) * speed) as u64;
                        want = due.saturating_sub(consumed).min(2048) as u32;
                    }
                    _ => {
                        started = Some((std::time::Instant::now(), rate));
                        consumed = 0;
                        want = 0;
                    }
                }
                if want == 0 {
                    tokio::time::sleep(Duration::from_millis(4)).await;
                    continue;
                }
            }
            if pull_ints {
                let b = p.read_pcm_int(want);
                let n = b.audio_samples as usize;
                if n > 0 {
                    let mut runs = out.int_runs.lock().unwrap();
                    match runs.last_mut() {
                        Some((r, e, s)) if *r == b.sample_rate && *e == b.encoding => {
                            s.extend_from_slice(&b.samples[..n])
                        }
                        _ => runs.push((b.sample_rate, b.encoding, b.samples[..n].to_vec())),
                    }
                }
                tokio::time::sleep(Duration::from_millis(4)).await;
                continue;
            }
            let b = p.read_pcm(want);
            if b.sample_rate > 0 {
                consumed += b.samples.len() as u64 / 2;
            }
            let n = b.audio_samples as usize;
            if n > 0 {
                let mut runs = out.runs.lock().unwrap();
                match runs.last_mut() {
                    Some((r, s)) if *r == b.sample_rate => s.extend_from_slice(&b.samples[..n]),
                    _ => runs.push((b.sample_rate, b.samples[..n].to_vec())),
                }
            }
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    });
    Rig {
        h,
        proxy,
        session,
        player,
        obs,
        lms,
        mac,
        pulled,
        stop_pull,
        pause_pull,
        cache,
    }
}

impl Rig {
    async fn send(&mut self, frame: Vec<u8>) {
        self.lms.write_all(&frame).await.unwrap();
    }

    fn count(&self, event: &str) -> usize {
        self.h
            .lms
            .slim_events()
            .iter()
            .filter(|e| *e == event)
            .count()
    }

    async fn wait_event(&self, event: &str, n: usize, secs: u64) {
        for _ in 0..secs * 100 {
            if self.count(event) >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "{event} ×{n} never arrived; LMS saw {:?}",
            self.h.lms.slim_events()
        );
    }

    fn stop_pulling(&self) {
        self.stop_pull.store(true, Ordering::SeqCst);
    }
}

fn strm_frame(command: u8, format: u8, request: &str) -> Vec<u8> {
    let s = Strm {
        command,
        autostart: b'1',
        format,
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
        request: request.as_bytes().to_vec(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

fn strm_s(format: u8, mac: &[u8; 6]) -> Vec<u8> {
    let req = format!(
        "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
        format_mac(mac)
    );
    strm_frame(b's', format, &req)
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

fn assert_bits_equal(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length differs");
    if let Some(i) = got
        .iter()
        .zip(want)
        .position(|(a, b)| a.to_bits() != b.to_bits())
    {
        panic!(
            "{what}: first difference at sample {i}: {} vs {}",
            got[i], want[i]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_flac_gaplessly_and_bit_perfectly_to_the_end() {
    let mut r = rig(false).await;
    assert_eq!(
        format_mac(&r.mac),
        r.session.current().unwrap().device.player_id,
        "HELO uses the MAC the gateway assigned"
    );
    let helo = String::from_utf8_lossy(&r.h.lms.slim_payloads(b"HELO")[0][36..]).into_owned();
    assert!(
        helo.contains("flc") && helo.contains("MaxSampleRate=192000"),
        "{helo}"
    );
    assert!(!helo.contains(",pcm"), "raw PCM is not advertised");

    let t1 = flac::encode(44_100, 16, &flac::noise(44_100 + 123, 16, 1));
    let t2 = flac::encode(44_100, 16, &flac::noise(30_011, 16, 2));
    let mut want = flac::decode(&t1).1;
    want.extend(flac::decode(&t2).1);
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(t1));
        s.push_back(StreamPlan::whole(t2));
    }
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &r.mac.clone())).await;
    // As LMS does: once the player has decoded the track, send the next one.
    r.wait_event("STMd", 1, 20).await;
    let mac = r.mac;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();

    let runs = r.pulled.runs.lock().unwrap().clone();
    assert!(runs.iter().all(|(rate, _)| *rate == 44_100));
    assert_bits_equal(&r.pulled.all(), &want, "two tracks, gapless");

    let ev = r.h.lms.slim_events();
    assert_eq!(r.count("STMc"), 2, "{ev:?}");
    assert_eq!(r.count("STMs"), 2, "one track start each: {ev:?}");
    assert_eq!(r.count("STMd"), 2, "{ev:?}");
    assert_eq!(r.count("STMu"), 1, "{ev:?}");
    assert_eq!(r.count("RESP"), 2);
    assert_eq!(r.count("HELO"), 1);
    // The second track started only after the first was decoded, and played out last.
    let pos = |e: &str| ev.iter().position(|x| x == e).unwrap();
    assert!(pos("STMd") < ev.iter().rposition(|x| x == "STMc").unwrap());
    assert_eq!(
        ev.iter().rposition(|x| x.starts_with("STM") && x != "STMt"),
        ev.iter().rposition(|x| x == "STMu")
    );

    let path = r.obs.last_path().expect("a signal path");
    assert_eq!(path.source_codec, "FLAC");
    assert_eq!((path.source_rate, path.source_bits), (44_100, Some(16)));
    assert_eq!(path.verdict, SignalVerdict::BitPerfect);
    assert!(path.engine_bit_perfect && !path.dsp_active && !path.resampled);
    assert!(!path.bit_perfect, "not claimed through the Android mixer");
    assert!(r.obs.saw(|s| s.playback == LocalPlayback::Playing));
    assert!(r.obs.saw(|s| s.connection == PlayerConnection::Connected));
    r.player.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rate_change_reopens_the_output_at_the_new_rate() {
    let mut r = rig(false).await;
    let t1 = flac::encode(44_100, 16, &flac::noise(22_050, 16, 3));
    let t2 = flac::encode(96_000, 24, &flac::noise(48_000, 24, 4));
    let (want1, want2) = (flac::decode(&t1).1, flac::decode(&t2).1);
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(t1));
        s.push_back(StreamPlan::whole(t2));
    }
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMd", 1, 20).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();

    let runs = r.pulled.runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 2, "one run per rate");
    assert_eq!(runs[0].0, 44_100);
    assert_eq!(runs[1].0, 96_000);
    assert_bits_equal(&runs[0].1, &want1, "44.1 kHz track");
    assert_bits_equal(&runs[1].1, &want2, "96 kHz track");
    let path = r.obs.last_path().unwrap();
    assert_eq!(
        (path.source_rate, path.source_bits, path.output_rate),
        (96_000, Some(24), 96_000)
    );
    assert!(!path.resampled);
    assert!(r.obs.saw(|s| s.output_rate == 96_000));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_connection_resumes_mid_stream_without_losing_audio() {
    let mut r = rig(true).await;
    let track = flac::encode(48_000, 24, &flac::noise(48_000 * 4, 24, 5));
    let want = flac::decode(&track).1;
    let gate = Arc::new(tokio::sync::Notify::new());
    r.h.lms.streams.lock().unwrap().push_back(StreamPlan {
        body: track,
        gate_at: Some((400 * 1024, gate.clone())),
    });
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    // Some audio is playing; the stream is held part-way.
    let pulled = r.pulled.clone();
    wait_for("audio", move || pulled.frames() > 10_000).await;
    let reconnects_before = r.obs.statuses.lock().unwrap().len();

    // The phone loses its network.
    r.proxy.as_ref().unwrap().cut_all();
    let obs = r.obs.clone();
    wait_for("reconnecting", move || {
        obs.saw(|s| matches!(s.connection, PlayerConnection::Reconnecting { .. }))
    })
    .await;
    let obs = r.obs.clone();
    wait_for("connected again", move || {
        obs.statuses.lock().unwrap()[reconnects_before..]
            .iter()
            .skip_while(|s| s.connection == PlayerConnection::Connected)
            .any(|s| s.connection == PlayerConnection::Connected)
    })
    .await;
    gate.notify_one();
    r.wait_event("STMu", 1, 40).await;
    r.stop_pulling();

    assert_bits_equal(&r.pulled.all(), &want, "audio across the reconnect");
    assert_eq!(
        r.h.lms.slim_accepted.load(Ordering::SeqCst),
        1,
        "LMS never saw the player leave"
    );
    assert_eq!(r.count("HELO"), 1, "resumed, not a new session");
    assert_eq!(
        r.h.lms.stream_requests(),
        1,
        "the gateway resumed its upstream"
    );
    assert!(r.proxy.as_ref().unwrap().accepted.load(Ordering::SeqCst) >= 4);
}

/// Regression: a connection that died without closing (a network change) was only noticed
/// once the gateway's grace had run out, because the player's own STATs kept resetting the
/// silence limit; it then had to start a new session, which makes LMS restart or stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_that_goes_silent_is_resumed_within_the_grace() {
    let mut r = rig(true).await;
    let track = flac::encode(48_000, 16, &flac::noise(48_000 * 2, 16, 3));
    r.h.lms.streams.lock().unwrap().push_back(StreamPlan {
        body: track,
        gate_at: None,
    });
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let pulled = r.pulled.clone();
    wait_for("audio", move || pulled.frames() > 10_000).await;
    let before = r.obs.statuses.lock().unwrap().len();
    let accepted = r.proxy.as_ref().unwrap().accepted.load(Ordering::SeqCst);

    r.proxy.as_ref().unwrap().freeze_all();
    let started = std::time::Instant::now();
    loop {
        let reconnected = r.obs.statuses.lock().unwrap()[before..]
            .iter()
            .skip_while(|s| s.connection == PlayerConnection::Connected)
            .any(|s| s.connection == PlayerConnection::Connected);
        if reconnected {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(45),
            "a silent connection must be given up and resumed well within the grace"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(r.proxy.as_ref().unwrap().accepted.load(Ordering::SeqCst) > accepted);
    assert_eq!(r.count("HELO"), 1, "resumed, not a new session");
    assert_eq!(
        r.h.lms.slim_accepted.load(Ordering::SeqCst),
        1,
        "LMS never saw the player leave"
    );
    r.stop_pulling();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_token_is_renewed_and_the_session_resumed() {
    let r = rig(true).await;
    r.session.debug_invalidate_access_token();
    r.proxy.as_ref().unwrap().cut_all();
    let obs = r.obs.clone();
    wait_for("reconnecting", move || {
        obs.saw(|s| matches!(s.connection, PlayerConnection::Reconnecting { .. }))
    })
    .await;
    let obs = r.obs.clone();
    wait_for("connected again", move || {
        let st = obs.statuses.lock().unwrap();
        let first_drop = st
            .iter()
            .position(|s| matches!(s.connection, PlayerConnection::Reconnecting { .. }))
            .unwrap();
        st[first_drop..]
            .iter()
            .any(|s| s.connection == PlayerConnection::Connected)
    })
    .await;
    assert_eq!(r.h.lms.slim_accepted.load(Ordering::SeqCst), 1);
    assert_eq!(r.count("HELO"), 1);
    assert!(r.player.is_running());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_formats_and_refused_streams_fail_cleanly() {
    let mut r = rig(false).await;
    // WMA: refused by the player itself, nothing fetched.
    let mac = r.mac;
    r.send(strm_s(b'w', &mac)).await;
    r.wait_event("STMn", 1, 5).await;
    assert_eq!(r.count("STMc"), 0);

    // A stream for another player: the gateway's policy refuses it (404), so the track
    // fails at once (DSCO) instead of being retried, and LMS is never asked.
    let other = format!(
        "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
        format_mac(&[2, 0x4c, 0x41, 9, 9, 9])
    );
    r.send(strm_frame(b's', b'f', &other)).await;
    r.wait_event("DSCO", 1, 10).await;
    let dsco = r.h.lms.slim_payloads(b"DSCO");
    assert_eq!(dsco[0], vec![1], "reported as a failed stream");
    assert_eq!(r.h.lms.stream_requests(), 0);

    // A redirect to another server never reaches the player (and it stays connected).
    let mut serv = vec![10, 0, 0, 10];
    serv.extend_from_slice(b"secret");
    r.send(encode_server_frame(*b"serv", &serv)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(r.player.status().connection, PlayerConnection::Connected);
    assert_eq!(r.h.lms.slim_accepted.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.count("DSCO"), 1, "no retry storm");
}

/// Small deterministic PRNG for the hostile-frame barrage.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_frames_from_lms_never_crash_the_player() {
    let mut r = rig(false).await;
    let mut rng = Rng(0xdead_beef_1234_5678);
    let ops: [&[u8; 4]; 8] = [
        b"strm", b"audg", b"aude", b"setd", b"vers", b"vfdc", b"grfe", b"zzzz",
    ];
    for _ in 0..400 {
        let op = ops[(rng.next() % ops.len() as u64) as usize];
        let len = (rng.next() % 64) as usize;
        let mut payload: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if op == b"strm" && !payload.is_empty() {
            // Any command but "start" (which would need a stream plan).
            payload[0] = b"pqtfau"[(rng.next() % 6) as usize];
        }
        r.send(encode_server_frame(*op, &payload)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(r.player.is_running(), "the engine survived");

    // And it still plays.
    let track = flac::encode(44_100, 16, &flac::noise(20_000, 16, 6));
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(track));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_frame(b'q', b'f', "")).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    assert!(r.player.is_running());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_says_goodbye_to_lms() {
    let r = rig(false).await;
    r.player.stop();
    wait_for("BYE!", || {
        r.h.lms.slim_events().contains(&"BYE!".to_string())
    })
    .await;
    let obs = r.obs.clone();
    wait_for("stopped", move || {
        obs.saw(|s| matches!(&s.connection, PlayerConnection::Stopped { reason } if reason == "stopped"))
    })
    .await;
    assert!(!r.player.is_running());
    // It can start again: the same player identity comes back.
    let obs = Arc::new(Obs::default());
    r.player.start(obs.clone()).unwrap();
    wait_for("second HELO", || r.count("HELO") == 2).await;
    let helos = r.h.lms.slim_payloads(b"HELO");
    assert_eq!(helos[0][2..8], helos[1][2..8]);
    r.player.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_out_ends_the_player() {
    let r = rig(false).await;
    r.session.logout().await.unwrap();
    let obs = r.obs.clone();
    wait_for("stopped", move || {
        obs.saw(|s| matches!(s.connection, PlayerConnection::Stopped { .. }))
    })
    .await;
    for _ in 0..100 {
        if !r.player.is_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!r.player.is_running(), "a refusal is final");
    // And it can't be started while signed out.
    assert!(r.player.start(Arc::new(Obs::default())).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hold_silences_at_once_and_ducking_scales() {
    let mut r = rig(false).await;
    let track = flac::encode(44_100, 16, &flac::noise(44_100 * 3, 16, 8));
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(track));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let pulled = r.pulled.clone();
    wait_for("audio", move || pulled.frames() > 4_000).await;
    r.player.set_hold(true);
    tokio::time::sleep(Duration::from_millis(60)).await;
    let before = r.pulled.frames();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(r.pulled.frames(), before, "held: nothing more played");
    assert!(r.player.status().held);
    // The app paused LMS meanwhile; LMS stays paused for a while: still silent.
    r.send(strm_frame(b'p', b'f', "")).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(r.pulled.frames(), before);
    // Resumed from LMS (by any controller): the hold releases itself.
    r.send(strm_frame(b'u', b'f', "")).await;
    let pulled = r.pulled.clone();
    wait_for("audio again", move || pulled.frames() > before + 4_000).await;
    r.player.set_duck(true);
    let path = {
        tokio::time::sleep(Duration::from_millis(100)).await;
        r.player.signal_path().unwrap()
    };
    assert!(!path.engine_bit_perfect, "ducking is processing");
    r.stop_pulling();
}

/// The sleep timer's fade (Wave 1): the engine lowers its own output before the timer's
/// end, keeps it down past the end (LMS stops the player then), and cancelling restores
/// full volume at once. Re-arming replaces the fade.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sleep_fade_lowers_the_output_and_cancelling_restores_it() {
    let mut r = rig(false).await;
    let track = flac::encode(44_100, 16, &flac::noise(44_100 * 40, 16, 8));
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(track));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let pulled = r.pulled.clone();
    wait_for("audio", move || pulled.frames() > 8_000).await;
    let peak_from = |from: usize| -> f32 {
        r.pulled.all()[from.min(r.pulled.all().len())..]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    };
    let full = peak_from(0);
    assert!(full > 0.1, "the noise is loud enough to measure ({full})");

    // A timer far off: nothing changes yet.
    r.player.clone().set_sleep_fade(Some(600.0), 30);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mark = r.pulled.all().len();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        peak_from(mark) > full * 0.5,
        "full volume well before the fade"
    );

    // Re-armed to end almost now: faded to silence.
    r.player.clone().set_sleep_fade(Some(0.05), 30);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mark = r.pulled.all().len();
    let pulled = r.pulled.clone();
    wait_for("more audio", move || pulled.all().len() > mark + 8_000).await;
    assert!(
        peak_from(mark) < full * 0.01,
        "faded out ({} vs {full})",
        peak_from(mark)
    );
    assert!(
        !r.player.signal_path().unwrap().engine_bit_perfect,
        "a fade is processing"
    );

    // Cancelled (or the timer turned off elsewhere): full volume again at once.
    r.player.clone().set_sleep_fade(None, 30);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mark = r.pulled.all().len();
    let pulled = r.pulled.clone();
    wait_for("audio after cancelling", move || {
        pulled.all().len() > mark + 8_000
    })
    .await;
    assert!(
        peak_from(mark) > full * 0.5,
        "restored ({} vs {full})",
        peak_from(mark)
    );

    // Hostile values: no fade rather than silence, and never a panic.
    for (ends, fade) in [
        (Some(f64::NAN), 30),
        (Some(-5.0), 30),
        (Some(f64::INFINITY), 30),
        (Some(1e12), 30),
        (Some(0.05), 0),
    ] {
        r.player.clone().set_sleep_fade(ends, fade);
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let mark = r.pulled.all().len();
    let pulled = r.pulled.clone();
    wait_for("audio after junk", move || {
        pulled.all().len() > mark + 8_000
    })
    .await;
    assert!(peak_from(mark) > full * 0.5, "junk leaves the output alone");
    r.stop_pulling();
}

/// A stand-in for Android's `AudioManager` with a USB DAC attached: it offers what a
/// typical DAC lists and records every mixer attribute the engine applies.
struct FakeDac {
    offers: Vec<MixerOffer>,
    accept: bool,
    applied: Mutex<Vec<Option<MixerOffer>>>,
    /// Unplugged: the output is no longer a USB DAC.
    unplugged: AtomicBool,
}

impl FakeDac {
    fn typical(accept: bool) -> Arc<FakeDac> {
        let mut offers = Vec::new();
        for rate in [44_100, 48_000, 96_000, 192_000] {
            for encoding in [PcmEncoding::I16, PcmEncoding::I24Packed, PcmEncoding::I32] {
                offers.push(MixerOffer {
                    encoding,
                    sample_rate: rate,
                    channels: 2,
                    bit_perfect: true,
                });
            }
        }
        Arc::new(FakeDac {
            offers,
            accept,
            applied: Mutex::new(Vec::new()),
            unplugged: AtomicBool::new(false),
        })
    }

    fn applied(&self) -> Vec<Option<MixerOffer>> {
        self.applied.lock().unwrap().clone()
    }
}

impl OutputNegotiator for FakeDac {
    fn offers(&self) -> OutputOffers {
        if self.unplugged.load(Ordering::SeqCst) {
            return OutputOffers {
                device: String::new(),
                blocker: Some(OfferBlocker::NoUsbDac),
                offers: vec![],
            };
        }
        OutputOffers {
            device: "Example USB DAC".into(),
            blocker: None,
            offers: self.offers.clone(),
        }
    }
    fn apply(&self, offer: Option<MixerOffer>) -> bool {
        self.applied.lock().unwrap().push(offer);
        self.accept
    }
}

/// Integers as the decoder delivers them, shifted into a `container`-bit DAC sample.
fn shifted(samples: &[i32], bits: u32, container: u32) -> Vec<i32> {
    samples.iter().map(|s| s << (container - bits)).collect()
}

/// A USB DAC with bit-perfect mixer attributes: each track's mixer attributes are applied
/// (16-bit at 44.1 kHz, then 24-bit at 96 kHz), the output opens in that integer format,
/// the DAC receives the file's very integers, LMS's volume doesn't touch them, the signal
/// path says bit-perfect end to end, and stopping restores Android's mixer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bit_perfect_dac_receives_the_files_integers() {
    let dac = FakeDac::typical(true);
    let mut r = rig_with(false, Some(dac.clone()), DspSettings::default()).await;
    let n1 = flac::noise(22_050, 16, 21);
    let n2 = flac::noise(48_000, 24, 22);
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(flac::encode(44_100, 16, &n1)));
        s.push_back(StreamPlan::whole(flac::encode(96_000, 24, &n2)));
    }
    let mac = r.mac;
    // LMS's volume at 25 %: ignored on a bit-perfect route.
    let mut audg = vec![0u8; 18];
    audg[8] = 1;
    audg[9] = 255;
    audg[10..14].copy_from_slice(&0x4000u32.to_be_bytes());
    audg[14..18].copy_from_slice(&0x4000u32.to_be_bytes());
    r.send(encode_server_frame(*b"audg", &audg)).await;
    r.send(strm_s(b'f', &mac)).await;
    let obs = r.obs.clone();
    wait_for("a bit-perfect path", move || {
        obs.last_path().is_some_and(|p| p.bit_perfect)
    })
    .await;
    let p = r.obs.last_path().unwrap();
    assert!(p.direct && p.fixed_volume && p.not_bit_perfect.is_empty());
    assert_eq!(p.output_device, "Example USB DAC");
    assert_eq!((p.output_bits, p.output_rate), (Some(16), 44_100));
    assert_eq!(p.output_format, "16-bit integer");
    assert!(!p.stages.iter().any(|s| s.name.starts_with("Volume")));
    r.wait_event("STMd", 1, 20).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();

    let runs = r.pulled.int_runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 2, "one run per format");
    assert_eq!((runs[0].0, runs[0].1), (44_100, PcmEncoding::I16));
    assert_eq!((runs[1].0, runs[1].1), (96_000, PcmEncoding::I24Packed));
    assert_eq!(runs[0].2, n1, "16-bit track, integer for integer");
    assert_eq!(runs[1].2, n2, "24-bit track, integer for integer");
    let p = r.obs.last_path().unwrap();
    assert!(p.bit_perfect);
    assert_eq!((p.output_bits, p.output_rate), (Some(24), 96_000));
    // The mixer attributes followed the tracks.
    let applied = dac.applied();
    assert_eq!(applied[0].unwrap().encoding, PcmEncoding::I16);
    assert_eq!(applied.last().unwrap().unwrap().sample_rate, 96_000);
    // Stopping hands the DAC back to Android's mixer.
    r.player.stop();
    let d = dac.clone();
    wait_for("mixer attributes cleared", move || {
        d.applied().last() == Some(&None)
    })
    .await;
}

/// Unplugging the DAC mid-track (the app renegotiates): the output goes back to float
/// through Android's mixer, the mixer attributes are cleared, LMS's volume applies again
/// and the panel says why. Removing the negotiator clears through the one that set them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unplugging_the_dac_renegotiates_mid_track() {
    let dac = FakeDac::typical(true);
    let mut r = rig_with(false, Some(dac.clone()), DspSettings::default()).await;
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(flac::encode(
            48_000,
            24,
            &flac::noise(48_000 * 3, 24, 25),
        )));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let obs = r.obs.clone();
    wait_for("bit-perfect", move || {
        obs.last_path().is_some_and(|p| p.bit_perfect)
    })
    .await;
    dac.unplugged.store(true, Ordering::SeqCst);
    r.player.renegotiate_output();
    let obs = r.obs.clone();
    wait_for("back to the mixer", move || {
        obs.last_path().is_some_and(|p| !p.direct)
    })
    .await;
    let p = r.obs.last_path().unwrap();
    assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::NoUsbDac]);
    assert!(!p.fixed_volume && p.engine_bit_perfect);
    assert_eq!(dac.applied().last(), Some(&None), "attributes cleared");
    // Plugged back in: bit-perfect again.
    dac.unplugged.store(false, Ordering::SeqCst);
    r.player.renegotiate_output();
    let obs = r.obs.clone();
    wait_for("bit-perfect again", move || {
        obs.last_path().is_some_and(|p| p.bit_perfect)
    })
    .await;
    // The app drops its negotiator (the service went away): cleared through the old one.
    r.player.set_output_negotiator(None);
    let d = dac.clone();
    wait_for("cleared by the old negotiator", move || {
        d.applied().last() == Some(&None)
    })
    .await;
    let obs = r.obs.clone();
    wait_for("float", move || obs.last_path().is_some_and(|p| !p.direct)).await;
    r.stop_pulling();
    r.player.stop();
}

/// A 24-bit track after a 16-bit one at the same rate: the output switches format at the
/// track boundary (after the first has played out), and both arrive integer for integer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_depth_at_the_same_rate_switches_the_dac_format() {
    let dac = FakeDac::typical(true);
    let mut r = rig_with(false, Some(dac.clone()), DspSettings::default()).await;
    let n1 = flac::noise(20_000, 16, 26);
    let n2 = flac::noise(20_000, 24, 27);
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(flac::encode(44_100, 16, &n1)));
        s.push_back(StreamPlan::whole(flac::encode(44_100, 24, &n2)));
    }
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMd", 1, 20).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let runs = r.pulled.int_runs.lock().unwrap().clone();
    assert_eq!(
        runs.len(),
        2,
        "{:?}",
        runs.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>()
    );
    assert_eq!(
        (runs[0].1, runs[1].1),
        (PcmEncoding::I16, PcmEncoding::I24Packed)
    );
    assert_eq!(runs[0].2, n1);
    assert_eq!(runs[1].2, n2);
    let encodings: Vec<_> = dac
        .applied()
        .into_iter()
        .flatten()
        .map(|o| o.encoding)
        .collect();
    assert_eq!(encodings, vec![PcmEncoding::I16, PcmEncoding::I24Packed]);
}

/// The platform refused the mixer attributes: float through the mixer, LMS's volume
/// applies, and the panel says why it isn't bit-perfect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_mixer_falls_back_to_float_and_says_why() {
    let dac = FakeDac::typical(false);
    let mut r = rig_with(false, Some(dac.clone()), DspSettings::default()).await;
    let n = flac::noise(22_050, 16, 23);
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(flac::encode(48_000, 16, &n)));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let p = r.obs.last_path().unwrap();
    assert!(!p.bit_perfect && !p.direct && p.engine_bit_perfect);
    assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::NotApplied]);
    assert_eq!(p.output_format, "32-bit float");
    let runs = r.pulled.int_runs.lock().unwrap().clone();
    assert_eq!(runs[0].1, PcmEncoding::Float);
    // Float pulled as 24-bit integers: still the file's samples (padded).
    assert_eq!(
        runs.iter().flat_map(|r| r.2.clone()).collect::<Vec<_>>(),
        shifted(&n, 16, 24)
    );
}

/// DSP on a bit-perfect DAC: the route stays direct (no Android resampling), the processed
/// audio is dithered onto the DAC's grid, and the panel says the DSP is why it isn't
/// bit-perfect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dsp_on_a_dac_is_direct_but_not_bit_perfect() {
    let dac = FakeDac::typical(true);
    let dsp = DspSettings {
        enabled: true,
        preamp_db: -6.0,
        limiter: false,
        ..DspSettings::default()
    };
    let mut r = rig_with(false, Some(dac.clone()), dsp).await;
    let n = flac::noise(22_050, 16, 24);
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(flac::encode(44_100, 16, &n)));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let p = r.obs.last_path().unwrap();
    assert!(p.direct && !p.bit_perfect);
    assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::DspActive]);
    assert!(p.stages.iter().any(|s| s.name.starts_with("Preamp")));
    assert!(
        p.stages
            .iter()
            .any(|s| s.name.starts_with("Dither to 16-bit"))
    );
    let runs = r.pulled.int_runs.lock().unwrap().clone();
    let got: Vec<i32> = runs.iter().flat_map(|r| r.2.clone()).collect();
    assert_eq!(got.len(), n.len());
    // About half the level, within the dither's ±1 LSB.
    let g = 10f64.powf(-6.0 / 20.0);
    for (o, i) in got.iter().zip(&n) {
        assert!(
            (f64::from(*o) - f64::from(*i) * g).abs() <= 1.5,
            "{o} vs {i}"
        );
    }
    // Turning DSP off again takes effect for the next audio: settings apply live.
    r.player.set_dsp(DspSettings::default());
}

/// LMS's volume at 25 % (0x4000 in 16.16).
fn audg_quarter() -> Vec<u8> {
    let mut p = vec![0u8; 18];
    p[8] = 1;
    p[9] = 255;
    p[10..14].copy_from_slice(&0x4000u32.to_be_bytes());
    p[14..18].copy_from_slice(&0x4000u32.to_be_bytes());
    encode_server_frame(*b"audg", &p)
}

fn loudness_dsp() -> DspSettings {
    DspSettings {
        enabled: true,
        loudness: true,
        ..DspSettings::default()
    }
}

/// Loudness-compensated volume on Android's mixer: LMS's volume at 25 % (−12 dB) brings
/// +3 dB of bass and +1.2 dB of treble, the panel shows the stage, and the limiter keeps
/// the result under its ceiling.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loudness_follows_lms_volume_on_the_mixer() {
    let mut r = rig_with(false, None, loudness_dsp()).await;
    let n = flac::noise(44_100, 16, 31);
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(flac::encode(44_100, 16, &n)));
    let mac = r.mac;
    r.send(audg_quarter()).await;
    r.send(strm_s(b'f', &mac)).await;
    let obs = r.obs.clone();
    wait_for("the loudness stage", move || {
        obs.last_path().is_some_and(|p| {
            p.stages
                .iter()
                .any(|s| s.name == "Loudness +3.0 dB bass, +1.2 dB treble")
        })
    })
    .await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let p = r.obs.last_path().unwrap();
    let stage = p
        .stages
        .iter()
        .find(|s| s.name.starts_with("Loudness"))
        .unwrap();
    assert!(!stage.bit_transparent);
    assert!(p.dsp_active && !p.engine_bit_perfect);
    assert!(p.not_bit_perfect.contains(&NotBitPerfect::DspActive));
    let ceiling = 10f32.powf(-1.0 / 20.0) + 1e-6;
    let all = r.pulled.all();
    assert!(!all.is_empty());
    assert!(all.iter().all(|x| x.is_finite() && x.abs() <= ceiling));
    r.player.stop();
}

/// On a bit-perfect DAC the DAC's volume rules: loudness is bypassed, the file's integers
/// reach the DAC untouched, and the panel says loudness was bypassed (and still calls the
/// path bit-perfect).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loudness_is_bypassed_on_a_bit_perfect_dac() {
    let dac = FakeDac::typical(true);
    let mut r = rig_with(false, Some(dac.clone()), loudness_dsp()).await;
    let n = flac::noise(22_050, 16, 32);
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(flac::encode(44_100, 16, &n)));
    let mac = r.mac;
    r.send(audg_quarter()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let p = r.obs.last_path().unwrap();
    assert!(p.direct && p.bit_perfect, "{:?}", p.not_bit_perfect);
    let stage = p
        .stages
        .iter()
        .find(|s| s.name.starts_with("Loudness"))
        .expect("the panel names the bypass");
    assert_eq!(stage.name, "Loudness bypassed (bit-perfect output)");
    assert!(stage.bit_transparent);
    let runs = r.pulled.int_runs.lock().unwrap().clone();
    let got: Vec<i32> = runs.iter().flat_map(|run| run.2.clone()).collect();
    assert_eq!(got, n, "integer for integer");
    r.player.stop();
}

/// Opus is advertised only by builds that decode it, and an Ogg Opus stream plays through
/// the whole player (gateway, decoder, pipeline, output).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opus_is_advertised_and_plays_end_to_end() {
    let mut r = rig(false).await;
    let helo = String::from_utf8_lossy(&r.h.lms.slim_payloads(b"HELO")[0][36..]).into_owned();
    assert_eq!(
        helo.contains("ops"),
        cfg!(feature = "opus"),
        "advertised exactly when built in: {helo}"
    );
    if !cfg!(feature = "opus") {
        return;
    }
    let fixture = include_bytes!("../../core/tests/fixtures/sine-997hz-stereo.opus").to_vec();
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(fixture));
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'u', &mac)).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let runs = r.pulled.runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, 48_000, "Opus decodes at 48 kHz");
    // One second, sample-accurate (pre-skip and end trimming applied).
    assert_eq!(runs[0].1.len() / 2, 48_000);
    let peak = runs[0].1.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!((0.08..0.12).contains(&peak), "a −20 dBFS sine: {peak}");
    let p = r.obs.last_path().unwrap();
    assert_eq!(p.source_codec, "Opus");
    assert_eq!(p.verdict, SignalVerdict::LossySource);
    assert!(p.not_bit_perfect.contains(&NotBitPerfect::LossySource));
}

/// While this phone is the player, loudness compensation follows the phone's media volume
/// as well as LMS's level: 25 % in LMS (−12 dB) and the phone 28 dB down is the full 40 dB,
/// and the panel says what it follows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loudness_follows_the_phones_volume_with_lms_level() {
    let mut r = rig_with(false, None, loudness_dsp()).await;
    r.player.set_loudness_reference_db(Some(28.0));
    let n = flac::noise(44_100, 16, 33);
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(flac::encode(44_100, 16, &n)));
    let mac = r.mac;
    r.send(audg_quarter()).await;
    r.send(strm_s(b'f', &mac)).await;
    let obs = r.obs.clone();
    wait_for("loudness following the phone", move || {
        obs.last_path().is_some_and(|p| {
            p.stages.iter().any(|s| {
                s.name == "Loudness +10.0 dB bass, +4.0 dB treble (following the phone's volume)"
            })
        })
    })
    .await;
    // A headset that keeps its own volume: LMS's level alone again.
    r.player.set_loudness_reference_db(None);
    let obs = r.obs.clone();
    wait_for("loudness on LMS's level", move || {
        obs.last_path().is_some_and(|p| {
            p.stages
                .iter()
                .any(|s| s.name == "Loudness +3.0 dB bass, +1.2 dB treble")
        })
    })
    .await;
    assert_eq!(r.player.loudness_reference_db(), None);
    r.player.stop();
}

// --- Rolling cache, Phase A: spool, read-ahead, gaps, grace -----------------------------

/// The samples pulled so far must be exactly the start of `want`.
fn assert_prefix(got: &[f32], want: &[f32], what: &str) {
    assert!(
        got.len() <= want.len(),
        "{what}: more audio than there is ({} > {})",
        got.len(),
        want.len()
    );
    assert_bits_equal(got, &want[..got.len()], what);
}

/// Wait for the player to be connected again after a drop (statuses from `from` on).
async fn wait_reconnected(r: &Rig, from: usize, secs: u64) {
    for _ in 0..secs * 100 {
        let again = r.obs.statuses.lock().unwrap()[from..]
            .iter()
            .skip_while(|s| s.connection == PlayerConnection::Connected)
            .any(|s| s.connection == PlayerConnection::Connected);
        if again {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never connected again");
}

/// Read-ahead: the track is reported ready (`STMd`) as soon as it is in and a little of it
/// has been heard, so LMS sends the next one at once, and it is fully fetched long before
/// the current track ends. Both play gaplessly and bit-perfectly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_ahead_fetches_the_next_track_long_before_the_current_one_ends() {
    let mut r = rig_opts(RigOpts {
        spool: true,
        ready_after_ms: Some(1000),
        pace: Some(4.0),
        ..RigOpts::default()
    })
    .await;
    let t1 = flac::encode(44_100, 16, &flac::noise(44_100 * 20, 16, 21));
    let t2 = flac::encode(44_100, 16, &flac::noise(44_100 * 5, 16, 22));
    let mut want = flac::decode(&t1).1;
    let t1_frames = want.len() / 2;
    want.extend(flac::decode(&t2).1);
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(t1));
        s.push_back(StreamPlan::whole(t2));
    }
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMd", 1, 20).await;
    let at_stmd = r.pulled.frames();
    assert!(
        at_stmd < t1_frames / 4,
        "STMd early: after {at_stmd} of {t1_frames} frames"
    );
    r.send(strm_s(b'f', &mac)).await;
    // Track 2 fully received (its DSCO) while track 1 still has most of its way to go.
    r.wait_event("DSCO", 2, 20).await;
    let at_fetched = r.pulled.frames();
    assert!(
        at_fetched < t1_frames / 2,
        "track 2 fetched after {at_fetched} of {t1_frames} frames of track 1"
    );
    let st = r.player.status();
    assert!(st.read_ahead);
    assert_eq!(st.resume_window_ms, 595_000, "asked for 600 s, granted");
    assert!(
        st.cushion_ms > 5_000,
        "the rest of track 1 and all of track 2 are on the phone: {} ms",
        st.cushion_ms
    );
    assert!(r.player.cushion_ms() > 0);
    r.wait_event("STMu", 1, 40).await;
    r.stop_pulling();
    assert_bits_equal(&r.pulled.all(), &want, "two tracks read ahead, gapless");
    let ev = r.h.lms.slim_events();
    assert_eq!(r.count("STMd"), 2, "{ev:?}");
    assert_eq!(r.count("STMs"), 2, "{ev:?}");
    assert_eq!(r.count("STMu"), 1, "{ev:?}");
    assert_eq!(r.count("HELO"), 1);
    // Every spool is gone once its track has been fed.
    assert!(spool_files(r.cache.as_ref().unwrap()).is_empty());
    r.player.stop();
}

/// The phone loses its network for 90 s, five seconds before a track boundary, with a
/// grace of 120 s: the music carries on from the spool, bit for bit, across the boundary;
/// the session resumes (no new HELO, LMS never sees the player leave), and LMS hears about
/// track 2's start (`STMs`) once the phone is back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_90_second_gap_across_a_boundary_is_covered_and_the_session_resumes() {
    let mut r = rig_opts(RigOpts {
        proxy: true,
        spool: true,
        max_grace_s: Some(120),
        ready_after_ms: Some(1000),
        pace: Some(1.0),
        ..RigOpts::default()
    })
    .await;
    let rate = 22_050usize;
    let t1 = flac::encode(rate as u32, 16, &flac::noise(rate * 14, 16, 31));
    let t2 = flac::encode(rate as u32, 16, &flac::noise(rate * 105, 16, 32));
    assert!(
        t2.len() > anywhere_core::player::INPUT_HIGH_WATER,
        "more than the core holds: the spool covers the gap"
    );
    let mut want = flac::decode(&t1).1;
    let t1_frames = want.len() / 2;
    want.extend(flac::decode(&t2).1);
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(t1));
        s.push_back(StreamPlan::whole(t2));
    }
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMd", 1, 20).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("DSCO", 2, 20).await;
    assert_eq!(r.player.status().resume_window_ms, 115_000, "120 s granted");
    // Five seconds before the boundary, the network goes.
    let pulled = r.pulled.clone();
    wait_for("track 1 nearly over", move || {
        pulled.frames() >= t1_frames - rate * 5
    })
    .await;
    let proxy = r.proxy.as_ref().unwrap();
    let statuses = r.obs.statuses.lock().unwrap().len();
    let underruns = r.player.status().underruns;
    let stms_before = r.count("STMs");
    assert_eq!(stms_before, 1);
    proxy.set_down(true);
    let gap = std::time::Instant::now();
    while gap.elapsed() < Duration::from_secs(90) {
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(r.player.is_running());
        assert_eq!(r.count("STMs"), 1, "nothing reaches LMS in the gap");
    }
    let heard_in_gap = r.pulled.frames();
    assert!(
        heard_in_gap > t1_frames + rate * 80,
        "the music carried on into track 2: {heard_in_gap} frames"
    );
    assert_eq!(
        r.player.status().underruns,
        underruns,
        "not a moment of silence"
    );
    proxy.set_down(false);
    wait_reconnected(&r, statuses, 20).await;
    r.wait_event("STMs", 2, 10).await;
    r.stop_pulling();
    assert_prefix(&r.pulled.all(), &want, "audio across the gap");
    assert_eq!(r.count("HELO"), 1, "resumed, not a new session");
    assert_eq!(
        r.h.lms.slim_accepted.load(Ordering::SeqCst),
        1,
        "LMS never saw the player leave"
    );
    assert_eq!(r.h.lms.stream_requests(), 2, "nothing fetched twice");
    r.player.stop();
}

/// A gap longer than what is buffered (the stream itself was slow, so little is on the
/// phone): the audio runs out (`STMo`), and when the network is back the stream resumes
/// where it was and playback recovers (`STMr`), with no audio lost or repeated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gap_longer_than_the_buffer_reports_stmo_then_recovers_with_stmr() {
    let mut r = rig_opts(RigOpts {
        proxy: true,
        spool: true,
        max_grace_s: Some(120),
        pace: Some(1.0),
        ..RigOpts::default()
    })
    .await;
    let track = flac::encode(44_100, 16, &flac::noise(44_100 * 6, 16, 41));
    let want = flac::decode(&track).1;
    let gate = Arc::new(tokio::sync::Notify::new());
    r.h.lms.streams.lock().unwrap().push_back(StreamPlan {
        body: track,
        gate_at: Some((256 * 1024, gate.clone())),
    });
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let pulled = r.pulled.clone();
    wait_for("audio", move || pulled.frames() > 10_000).await;
    let statuses = r.obs.statuses.lock().unwrap().len();
    r.proxy.as_ref().unwrap().set_down(true);
    // The 1.5 s that arrived play out, then the output runs dry.
    for _ in 0..1000 {
        if r.player.status().underruns > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(r.player.status().underruns > 0, "the audio ran out");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(r.count("STMo"), 0, "held until the session resumes");
    r.proxy.as_ref().unwrap().set_down(false);
    wait_reconnected(&r, statuses, 20).await;
    r.wait_event("STMo", 1, 10).await;
    gate.notify_one();
    r.wait_event("STMr", 1, 30).await;
    r.wait_event("STMu", 1, 30).await;
    r.stop_pulling();
    let ev = r.h.lms.slim_events();
    let pos = |e: &str| ev.iter().position(|x| x == e).unwrap();
    assert!(pos("STMo") < pos("STMr"), "{ev:?}");
    assert_eq!(
        r.count("STMs"),
        1,
        "never a second STMs for one track: {ev:?}"
    );
    assert_eq!(r.count("HELO"), 1);
    assert_bits_equal(&r.pulled.all(), &want, "nothing lost or repeated");
}

/// A gap longer than the grace: the session is gone, so the player starts a new one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gap_longer_than_the_grace_starts_a_new_session() {
    let r = rig_opts(RigOpts {
        proxy: true,
        max_grace_s: Some(8),
        ..RigOpts::default()
    })
    .await;
    wait_for("grant known", || {
        r.player.status().resume_window_ms == 3_000
    })
    .await;
    let statuses = r.obs.statuses.lock().unwrap().len();
    r.proxy.as_ref().unwrap().set_down(true);
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(r.count("HELO"), 1);
    r.proxy.as_ref().unwrap().set_down(false);
    wait_reconnected(&r, statuses, 20).await;
    wait_for("a new session's HELO", || r.count("HELO") == 2).await;
    assert_eq!(r.h.lms.slim_accepted.load(Ordering::SeqCst), 2);
    let helos = r.h.lms.slim_payloads(b"HELO");
    assert_eq!(helos[0][2..8], helos[1][2..8], "the same player");
    r.player.stop();
}

/// An older gateway (no `resume_grace` feature): the player doesn't ask, and resumes for
/// 55 s, inside the 60 s such a gateway holds a session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_an_older_gateway_the_resume_window_is_55_seconds() {
    let r = rig_opts(RigOpts {
        old_gateway: true,
        ..RigOpts::default()
    })
    .await;
    wait_for("connected", || {
        r.player.status().connection == PlayerConnection::Connected
    })
    .await;
    assert_eq!(r.player.status().resume_window_ms, 55_000);
    r.player.stop();
}

/// Regression (`strm f`): LMS drops the next track, already received and partly decoded,
/// while the current one is still playing. Only the current track is heard, to its end;
/// not a sample of the dropped track, though some of it had reached the output's queue.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strm_f_drops_the_next_tracks_audio_and_keeps_the_current_one() {
    let mut r = rig_opts(RigOpts {
        spool: true,
        paused: true,
        ..RigOpts::default()
    })
    .await;
    // Track 1 is shorter than the output's queue (0.4 s), so it holds the start of track 2.
    let t1 = flac::encode(44_100, 16, &flac::noise(44_100 / 4, 16, 51));
    let t2 = flac::encode(44_100, 16, &flac::noise(44_100 * 2, 16, 52));
    let want = flac::decode(&t1).1;
    {
        let mut s = r.h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(t1));
        s.push_back(StreamPlan::whole(t2));
    }
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMd", 1, 20).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("DSCO", 2, 20).await;
    // Let the engine fill its queue with the end of track 1 and the start of track 2.
    tokio::time::sleep(Duration::from_millis(500)).await;
    // A little of track 1 is heard, so it is the audible track.
    let first = r.player.read_pcm(1024);
    assert_eq!(first.audio_samples, 2048);
    r.pulled
        .runs
        .lock()
        .unwrap()
        .push((first.sample_rate, first.samples.clone()));
    r.wait_event("STMs", 1, 10).await;
    r.send(strm_frame(b'f', b'0', "")).await;
    r.wait_event("STMf", 1, 10).await;
    r.pause_pull.store(false, Ordering::SeqCst);
    r.wait_event("STMu", 1, 20).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    r.stop_pulling();
    assert_bits_equal(&r.pulled.all(), &want, "track 1 alone, to its end");
    assert_eq!(r.count("STMs"), 1, "track 2 never started");
    r.player.stop();
}

/// Signing out stops the player and deletes its spools; nothing in them names the user,
/// server or track. Leftovers are cleared by `remove_stream_spools`, and refused outside
/// an absolute folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_out_clears_the_spools() {
    let mut r = rig_opts(RigOpts {
        spool: true,
        ..RigOpts::default()
    })
    .await;
    let track = flac::encode(44_100, 16, &flac::noise(44_100 * 4, 16, 61));
    let gate = Arc::new(tokio::sync::Notify::new());
    r.h.lms.streams.lock().unwrap().push_back(StreamPlan {
        body: track,
        gate_at: Some((64 * 1024, gate.clone())),
    });
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let cache = r.cache.clone().unwrap();
    let c = cache.clone();
    wait_for("a spool on disk", move || !spool_files(&c).is_empty()).await;
    for f in spool_files(&cache) {
        let rel = f
            .strip_prefix(&cache)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let parts: Vec<&str> = rel.split('/').collect();
        assert_eq!(parts.len(), 3, "{rel}");
        assert!(parts[0].len() == 32 && parts[0].bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(parts[1], "spool");
        assert!(
            parts[2]
                .trim_end_matches(".spool")
                .bytes()
                .all(|b| b.is_ascii_digit())
        );
        assert!(!rel.contains("alice"));
    }
    r.session.logout().await.unwrap();
    let obs = r.obs.clone();
    wait_for("stopped", move || {
        obs.saw(|s| matches!(s.connection, PlayerConnection::Stopped { .. }))
    })
    .await;
    let c = cache.clone();
    wait_for("spools deleted", move || spool_files(&c).is_empty()).await;
    gate.notify_one();

    // A leftover (a crash) is removed by the app's clean-up call.
    let dir = cache.join("0123456789abcdef0123456789abcdef").join("spool");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("7.spool"), b"x").unwrap();
    anywhere_mobile::remove_stream_spools(cache.to_string_lossy().into_owned()).unwrap();
    assert!(spool_files(&cache).is_empty());
    assert!(anywhere_mobile::remove_stream_spools("relative/cache".into()).is_err());
    assert!(anywhere_mobile::remove_stream_spools("/tmp/../etc".into()).is_err());
}

/// With no network, the player stops trying to reconnect (each try costs battery), and
/// tries at once when the network is back, resuming the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_network_pauses_reconnecting_and_its_return_reconnects_at_once() {
    let r = rig_opts(RigOpts {
        proxy: true,
        max_grace_s: Some(120),
        ..RigOpts::default()
    })
    .await;
    wait_for("connected", || {
        r.player.status().connection == PlayerConnection::Connected
    })
    .await;
    let statuses = r.obs.statuses.lock().unwrap().len();
    let proxy = r.proxy.as_ref().unwrap();
    proxy.set_down(true);
    r.player.set_network_available(false);
    wait_for("dropped", || {
        matches!(
            r.player.status().connection,
            PlayerConnection::Reconnecting { .. }
        )
    })
    .await;
    let tries = proxy.accepted.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        proxy.accepted.load(Ordering::SeqCst),
        tries,
        "no attempts without a network"
    );
    proxy.set_down(false);
    let back = std::time::Instant::now();
    r.player.set_network_available(true);
    wait_reconnected(&r, statuses, 5).await;
    assert!(
        back.elapsed() < Duration::from_secs(2),
        "reconnected at once, not after a backoff"
    );
    assert_eq!(r.count("HELO"), 1, "resumed");
    r.player.stop();
}

/// Robustness: the network coming and going at random, with LMS starting, flushing and
/// trimming streams meanwhile, never brings the player down.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn random_cuts_and_resumes_never_panic() {
    let mut r = rig_opts(RigOpts {
        proxy: true,
        spool: true,
        max_grace_s: Some(30),
        ready_after_ms: Some(200),
        pace: Some(2.0),
        ..RigOpts::default()
    })
    .await;
    let mut rng = Rng(0x5eed_cafe_f00d_1234);
    let mac = r.mac;
    r.send(audg_full()).await;
    for i in 0..60 {
        match rng.next() % 9 {
            0 => r.proxy.as_ref().unwrap().cut_all(),
            1 => r.proxy.as_ref().unwrap().set_down(true),
            2 => r.proxy.as_ref().unwrap().set_down(false),
            3 => r.player.set_network_available(rng.next().is_multiple_of(2)),
            4 | 5 => {
                let secs = 1 + (rng.next() % 3) as usize;
                let body = flac::encode(44_100, 16, &flac::noise(44_100 * secs, 16, i));
                let gate_at = rng.next().is_multiple_of(3).then(|| {
                    let g = Arc::new(tokio::sync::Notify::new());
                    g.notify_one();
                    ((rng.next() % body.len() as u64) as usize, g)
                });
                r.h.lms
                    .streams
                    .lock()
                    .unwrap()
                    .push_back(StreamPlan { body, gate_at });
                let _ = r.lms.write_all(&strm_s(b'f', &mac)).await;
            }
            6 => {
                let _ = r.lms.write_all(&strm_frame(b'f', b'0', "")).await;
            }
            7 => {
                let _ = r.lms.write_all(&strm_frame(b'q', b'0', "")).await;
            }
            _ => {
                let _ = r.lms.write_all(&strm_frame(b't', b'0', "")).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(rng.next() % 300)).await;
        assert!(r.player.is_running(), "step {i}");
    }
    r.proxy.as_ref().unwrap().set_down(false);
    r.player.set_network_available(true);
    wait_for("connected in the end", || {
        r.player.status().connection == PlayerConnection::Connected
    })
    .await;
    assert!(r.player.is_running());
    r.player.stop();
    let c = r.cache.clone().unwrap();
    wait_for("spools deleted on stop", move || spool_files(&c).is_empty()).await;
}

/// A6: a player in a sync group that stalled for longer than it has audio to skip asks
/// LMS to restart the group where it is, rather than playing on out of step.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_grouped_player_too_far_behind_after_a_gap_asks_lms_to_restart_the_group() {
    let mut r = rig_opts(RigOpts {
        proxy: true,
        spool: true,
        max_grace_s: Some(120),
        pace: Some(1.0),
        ..RigOpts::default()
    })
    .await;
    let me = format_mac(&r.mac);
    r.h.lms
        .syncs
        .members
        .lock()
        .unwrap()
        .insert(me.clone(), HOME_PLAYER.to_string());
    let track = flac::encode(44_100, 16, &flac::noise(44_100 * 3, 16, 71));
    let gate = Arc::new(tokio::sync::Notify::new());
    r.h.lms.streams.lock().unwrap().push_back(StreamPlan {
        body: track,
        gate_at: Some((256 * 1024, gate.clone())),
    });
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    let pulled = r.pulled.clone();
    wait_for("audio", move || pulled.frames() > 10_000).await;
    let statuses = r.obs.statuses.lock().unwrap().len();
    r.proxy.as_ref().unwrap().set_down(true);
    for _ in 0..1000 {
        if r.player.status().underruns > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Stalled for far longer than the rest of the track lasts.
    tokio::time::sleep(Duration::from_secs(4)).await;
    r.proxy.as_ref().unwrap().set_down(false);
    wait_reconnected(&r, statuses, 20).await;
    gate.notify_one();
    let lms = &r.h.lms;
    let asked = || {
        lms.rpc_calls()
            .iter()
            .any(|p| p[0] == me.as_str() && p[1] == serde_json::json!(["time", "?"]))
    };
    for _ in 0..1500 {
        if asked() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(asked(), "LMS was asked where the group is");
    r.player.stop();
}

// --- Restarting after a gap longer than the grace (the owner's decision 3) ---------------

/// What happens around a gap longer than the grace, in [`gap_beyond_the_grace`].
#[derive(Clone, Copy, PartialEq)]
enum BeyondGrace {
    /// Playing throughout; LMS shows the track stopped where it was.
    Playing,
    /// The queue changed meanwhile: another track at that position.
    QueueChanged,
    /// The user stopped playback (LMS's `strm q`) before the link went.
    StoppedFirst,
    /// The user held the output (paused on the phone) during the gap.
    HeldInGap,
}

/// A track plays (it sits at queue position 2), then the network goes for longer than the
/// 8 s grace. Returns the rig, how much had been heard when the network went and when it
/// came back (seconds), and the playback commands the player then sent LMS by itself.
async fn gap_beyond_the_grace(case: BeyondGrace) -> (Rig, f64, f64, Vec<serde_json::Value>) {
    let mut r = rig_opts(RigOpts {
        proxy: true,
        spool: true,
        max_grace_s: Some(8),
        pace: Some(1.0),
        ..RigOpts::default()
    })
    .await;
    let rate = 22_050usize;
    let at = |cur: &str, id: u64, mode: &str| {
        serde_json::json!({"mode": mode, "time": 0, "playlist_cur_index": cur,
            "playlist_loop": [{"playlist index": 2, "id": id, "duration": 30}]})
    };
    *r.h.lms.status.lock().unwrap() = Some(at("2", 1, "play"));
    wait_for("grant known", || {
        r.player.status().resume_window_ms == 3_000
    })
    .await;
    let track = flac::encode(rate as u32, 16, &flac::noise(rate * 30, 16, 81));
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(track));
    let mac = r.mac;
    let me = format_mac(&mac);
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMs", 1, 20).await;
    let pulled = r.pulled.clone();
    wait_for("3 s heard", move || pulled.frames() >= rate * 3).await;
    // The player has asked LMS where the track is.
    let lms = &r.h.lms;
    wait_for("the track's position", || {
        lms.rpc_calls()
            .iter()
            .any(|p| p[0] == me.as_str() && p[1][0] == "status" && p[1][1] == "-")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    if case == BeyondGrace::StoppedFirst {
        r.send(strm_frame(b'q', b'0', "")).await;
        r.wait_event("STMf", 1, 10).await;
    }
    // Meanwhile LMS loses the player (the session expires) and shows it stopped.
    *r.h.lms.status.lock().unwrap() = Some(match case {
        BeyondGrace::QueueChanged => at("2", 9, "stop"),
        _ => at("2", 1, "stop"),
    });
    let heard_at_cut = r.pulled.frames() as f64 / rate as f64;
    let proxy = r.proxy.as_ref().unwrap();
    proxy.set_down(true);
    if case == BeyondGrace::HeldInGap {
        tokio::time::sleep(Duration::from_secs(2)).await;
        r.player.set_hold(true);
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(r.count("HELO"), 1);
    proxy.set_down(false);
    wait_for("a new session's HELO", || r.count("HELO") == 2).await;
    let heard_at_return = r.pulled.frames() as f64 / rate as f64;
    // The restart comes a second after the new session; give it ample time.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let sent: Vec<serde_json::Value> =
        r.h.lms
            .rpc_calls()
            .into_iter()
            .filter(|p| p[0] == me.as_str() && (p[1][0] == "playlist" || p[1][0] == "time"))
            .map(|p| p[1].clone())
            .collect();
    (r, heard_at_cut, heard_at_return, sent)
}

/// A gap longer than the grace while playing: on the new session the player restarts the
/// same track (by its queue position, checked with LMS first) at the point last heard, and
/// LMS's new stream plays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gap_longer_than_the_grace_restarts_the_same_track_where_it_was() {
    let (mut r, at_cut, at_return, sent) = gap_beyond_the_grace(BeyondGrace::Playing).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0], serde_json::json!(["playlist", "index", "2"]));
    assert_eq!(sent[1][0], "time");
    let t: f64 = sent[1][1].as_str().unwrap().parse().unwrap();
    assert!(
        t > at_cut + 5.0 && t < at_return + 1.0,
        "restarted at {t} s: heard {at_cut} s at the cut, {at_return} s at the return"
    );
    let me = format_mac(&r.mac);
    assert!(
        r.h.lms
            .rpc_calls()
            .iter()
            .any(|p| p[0] == me.as_str()
                && p[1] == serde_json::json!(["status", "2", "1", "tags:x"])),
        "the position was checked with LMS first"
    );
    // LMS streams it again on the new connection, and it plays.
    let track = flac::encode(22_050, 16, &flac::noise(22_050 * 5, 16, 82));
    r.h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(track));
    r.lms = r.h.lms.slim_conn().await;
    let mac = r.mac;
    r.send(audg_full()).await;
    r.send(strm_s(b'f', &mac)).await;
    r.wait_event("STMs", 2, 20).await;
    r.player.stop();
}

/// Negative: the queue changed during the gap (another track at that position), so the
/// player leaves LMS alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gap_longer_than_the_grace_does_not_restart_a_changed_queue() {
    let (r, _, _, sent) = gap_beyond_the_grace(BeyondGrace::QueueChanged).await;
    assert!(sent.is_empty(), "{sent:?}");
    r.player.stop();
}

/// Negative: the user had stopped playback before the gap, so nothing is restarted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gap_longer_than_the_grace_after_a_stop_restarts_nothing() {
    let (r, _, _, sent) = gap_beyond_the_grace(BeyondGrace::StoppedFirst).await;
    assert!(sent.is_empty(), "{sent:?}");
    r.player.stop();
}

/// Negative: the user paused on the phone during the gap, so nothing is restarted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gap_longer_than_the_grace_held_by_the_user_restarts_nothing() {
    let (r, _, _, sent) = gap_beyond_the_grace(BeyondGrace::HeldInGap).await;
    assert!(sent.is_empty(), "{sent:?}");
    r.player.stop();
}
