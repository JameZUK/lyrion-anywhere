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
    LocalPlayerStatus, OutputKind, PlayerConnection, Server, Session, SignalPathInfo,
    SignalVerdict,
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

/// Audio pulled from the player, as (sample rate, samples) runs.
#[derive(Default)]
struct Pulled {
    runs: Mutex<Vec<(u32, Vec<f32>)>>,
}

impl Pulled {
    fn frames(&self) -> usize {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.len() / 2)
            .sum()
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
            dsp: DspSettings::default(),
        },
    );
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
