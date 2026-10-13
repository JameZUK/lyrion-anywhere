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
    let pull_ints = negotiator.is_some();
    let h = Harness::start().await;
    let proxy = if with_proxy {
        Some(Proxy::start(h.addr).await)
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
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 192_000,
            dsp,
        },
    );
    player.set_output_negotiator(negotiator);
    let obs = Arc::new(Obs::default());
    player.start(obs.clone()).unwrap();
    let lms = h.lms.slim_conn().await;
    wait_for("HELO", || {
        h.lms.slim_events().first().map(String::as_str) == Some("HELO")
    })
    .await;
    let helo = &h.lms.slim_payloads(b"HELO")[0];
    let mac: [u8; 6] = helo[2..8].try_into().unwrap();

    // Pull like an audio device, a little faster than real time.
    let pulled = Arc::new(Pulled::default());
    let stop_pull = Arc::new(AtomicBool::new(false));
    let (p, out, stop) = (player.clone(), pulled.clone(), stop_pull.clone());
    tokio::spawn(async move {
        while !stop.load(Ordering::SeqCst) {
            if pull_ints {
                let b = p.read_pcm_int(2048);
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
            let b = p.read_pcm(2048);
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
