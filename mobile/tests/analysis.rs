//! Track analysis from the phone, end to end: the real gateway (in-process, analysis on)
//! in front of a fake LMS serving generated FLAC files.
//!
//! Covered: fetching an analysis (ready, pending then ready, unavailable, refused ids,
//! signed out); and R128 levelling in the player: the gain from the analysis of the track
//! LMS starts, the fallback when there is none, and the signal path naming the source.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_core::proto::{Strm, encode_server_frame, format_mac};
use anywhere_mobile::{
    Analysis, AnalysisOutcome, ApiError, DspSettings, Levelling, LocalPlayer, LocalPlayerConfig,
    LocalPlayerObserver, LocalPlayerStatus, OutputKind, SignalPathInfo,
};
use common::flac;
use common::*;
use tokio::io::AsyncWriteExt;

/// A stereo 997 Hz tone at `db` dBFS, 16-bit.
fn tone(rate: u32, secs: f64, db: f64) -> Vec<i32> {
    let amp = 10f64.powf(db / 20.0) * 32767.0;
    (0..(secs * f64::from(rate)) as usize)
        .flat_map(|i| {
            let x = (amp * (2.0 * std::f64::consts::PI * 997.0 * i as f64 / f64::from(rate)).sin())
                .round() as i32;
            [x, x]
        })
        .collect()
}

fn rms_db(x: &[f32]) -> f64 {
    let p = x.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * p.log10()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetches_a_tracks_analysis() {
    let h = Harness::start().await;
    h.lms.add_track(
        1,
        "flc",
        flac::encode(44_100, 16, &tone(44_100, 3.0, -20.0)),
    );
    let (session, _) = h.alice().await;
    let a = Analysis::new(session.clone());
    let AnalysisOutcome::Ready { analysis } = a.track(1).await.unwrap() else {
        panic!("not ready");
    };
    assert!((analysis.duration_s - 3.0).abs() < 0.01);
    assert_eq!(analysis.sample_rate, 44_100);
    assert!((analysis.integrated_lufs.unwrap() + 20.0).abs() < 0.3);
    assert!((analysis.true_peak_dbtp.unwrap() + 20.0).abs() < 0.3);
    assert_eq!(analysis.peak.len(), analysis.rms.len());
    assert!(!analysis.peak.is_empty());

    // Not in the library, or not a library file: unavailable (the plain seek bar).
    assert!(matches!(
        a.track(4242).await.unwrap(),
        AnalysisOutcome::Unavailable { .. }
    ));
    // Ids the gateway would refuse are refused here first.
    for bad in [0, -5, 1_000_000_000_000] {
        assert!(matches!(
            a.track(bad).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    // Signed out: no access.
    session.logout().await.unwrap();
    assert!(matches!(a.track(1).await, Err(ApiError::Unauthorised)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_analysis_says_when_to_ask_again() {
    let h = Harness::start_with(|c| c.analysis_wait = Duration::ZERO).await;
    h.lms.add_track(
        1,
        "flc",
        flac::encode(44_100, 16, &tone(44_100, 20.0, -14.0)),
    );
    let (session, _) = h.alice().await;
    let a = Analysis::new(session);
    match a.track(1).await.unwrap() {
        AnalysisOutcome::Pending { retry_after_ms } => assert_eq!(retry_after_ms, 3000),
        other => panic!("expected pending, got {other:?}"),
    }
    let mut ready = false;
    for _ in 0..200 {
        if let AnalysisOutcome::Ready { analysis } = a.track(1).await.unwrap() {
            assert!((analysis.integrated_lufs.unwrap() + 14.0).abs() < 0.3);
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analysis_off_on_the_server_is_unavailable_not_an_error() {
    let h = Harness::start_with(|c| c.analysis_cache_dir = None).await;
    h.lms.add_track(
        1,
        "flc",
        flac::encode(44_100, 16, &tone(44_100, 1.0, -14.0)),
    );
    let (session, _) = h.alice().await;
    match Analysis::new(session).track(1).await.unwrap() {
        AnalysisOutcome::Unavailable { reason } => {
            assert_eq!(reason, "analysis is not available on this server")
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Levelling in the player
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Obs {
    paths: Mutex<Vec<Option<SignalPathInfo>>>,
}

impl LocalPlayerObserver for Obs {
    fn on_status(&self, _status: LocalPlayerStatus) {}
    fn on_signal_path(&self, path: Option<SignalPathInfo>) {
        self.paths.lock().unwrap().push(path);
    }
}

fn strm_s(mac: &[u8; 6]) -> Vec<u8> {
    let req = format!(
        "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
        format_mac(mac)
    );
    let s = Strm {
        command: b's',
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
        request: req.into_bytes(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

fn audg_full() -> Vec<u8> {
    let mut p = vec![0u8; 18];
    p[8] = 1;
    p[9] = 255;
    p[10..14].copy_from_slice(&0x10000u32.to_be_bytes());
    p[14..18].copy_from_slice(&0x10000u32.to_be_bytes());
    encode_server_frame(*b"audg", &p)
}

/// Play `file` (also served as library track 1 when `in_library`) with analysis levelling
/// on; returns the PCM played and the last signal path.
async fn play_levelled(file: Vec<u8>, in_library: bool) -> (Vec<f32>, Option<SignalPathInfo>) {
    let h = Harness::start().await;
    if in_library {
        h.lms.add_track(1, "flc", file.clone());
    }
    h.lms
        .streams
        .lock()
        .unwrap()
        .push_back(StreamPlan::whole(file));
    let (session, _) = h.alice().await;
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 192_000,
            dsp: DspSettings {
                enabled: true,
                levelling: Levelling::Track,
                target_lufs: -18.0,
                limiter: false,
                analysis_levelling: true,
                ..DspSettings::default()
            },
        },
    );
    let obs = Arc::new(Obs::default());
    player.start(obs.clone()).unwrap();
    let mut lms = h.lms.slim_conn().await;
    wait_for("HELO", || {
        h.lms.slim_events().first().map(String::as_str) == Some("HELO")
    })
    .await;
    let helo = &h.lms.slim_payloads(b"HELO")[0];
    let mac: [u8; 6] = helo[2..8].try_into().unwrap();

    let pulled: Arc<Mutex<Vec<f32>>> = Default::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (p, out, st) = (player.clone(), pulled.clone(), stop.clone());
    tokio::spawn(async move {
        while !st.load(Ordering::SeqCst) {
            let b = p.read_pcm(2048);
            let n = b.audio_samples as usize;
            out.lock().unwrap().extend_from_slice(&b.samples[..n]);
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    });
    lms.write_all(&audg_full()).await.unwrap();
    lms.write_all(&strm_s(&mac)).await.unwrap();
    for _ in 0..3000 {
        if h.lms.slim_events().iter().any(|e| e == "STMu") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        h.lms.slim_events().iter().any(|e| e == "STMu"),
        "{:?}",
        h.lms.slim_events()
    );
    stop.store(true, Ordering::SeqCst);
    player.stop();
    let path = obs
        .paths
        .lock()
        .unwrap()
        .iter()
        .rev()
        .flatten()
        .next()
        .cloned();
    let pcm = pulled.lock().unwrap().clone();
    (pcm, path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn levels_to_the_target_from_the_tracks_analysis() {
    // A loud track (-10 dBFS sine: about -10 LUFS) is turned down 8 dB to reach -18 LUFS.
    let pcm = tone(44_100, 3.0, -10.0);
    let file = flac::encode(44_100, 16, &pcm);
    let source = flac::decode(&file).1;
    let (played, path) = play_levelled(file, true).await;
    assert_eq!(played.len(), source.len(), "every frame played once");
    let gain = rms_db(&played) - rms_db(&source);
    assert!((gain + 8.0).abs() < 0.3, "gain {gain:.2} dB");
    // Applied from the first sample (decided before the track started).
    let head = 4410 * 2;
    let g_head = rms_db(&played[..head]) - rms_db(&source[..head]);
    assert!((g_head + 8.0).abs() < 0.3, "head gain {g_head:.2} dB");
    let path = path.expect("a signal path");
    assert!(path.dsp_active);
    assert!(
        path.stages
            .iter()
            .any(|s| s.name.starts_with("Levelling") && s.name.ends_with("(R128 analysis)")),
        "{:?}",
        path.stages
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_an_analysis_the_track_plays_unlevelled_after_a_short_wait() {
    // Not a library file the gateway can analyse, and no ReplayGain tags: no levelling.
    let pcm = tone(44_100, 2.0, -10.0);
    let file = flac::encode(44_100, 16, &pcm);
    let source = flac::decode(&file).1;
    let started = std::time::Instant::now();
    let (played, path) = play_levelled(file, false).await;
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(played.len(), source.len());
    assert!((rms_db(&played) - rms_db(&source)).abs() < 0.01);
    let path = path.expect("a signal path");
    assert!(
        !path.stages.iter().any(|s| s.name.contains("R128")),
        "{:?}",
        path.stages
    );
}
