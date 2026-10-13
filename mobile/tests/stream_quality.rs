//! Streaming quality (docs/plans/streaming-quality.md) through the real gateway and its
//! `ffmpeg`: this phone's player asks for Data saver, gets FLAC re-encoded to Opus and plays
//! it, then goes back to Original mid-session and gets FLAC again.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_core::proto::{Strm, encode_server_frame, format_mac};
use anywhere_mobile::player::protocol::{quality_message, sign_in_message};
use anywhere_mobile::{
    LocalPlayer, LocalPlayerConfig, LocalPlayerObserver, LocalPlayerStatus, OutputKind,
    SignalPathInfo, StreamQuality, stream_quality_for,
};
use common::flac;
use common::*;
use tokio::io::AsyncWriteExt;

#[derive(Default)]
struct Obs {
    paths: Mutex<Vec<SignalPathInfo>>,
}

impl LocalPlayerObserver for Obs {
    fn on_status(&self, _status: LocalPlayerStatus) {}
    fn on_signal_path(&self, path: Option<SignalPathInfo>) {
        if let Some(p) = path {
            self.paths.lock().unwrap().push(p);
        }
    }
}

fn strm_s(format: u8, mac: &[u8; 6]) -> Vec<u8> {
    let s = Strm {
        command: b's',
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
        request: format!(
            "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
            format_mac(mac)
        )
        .into_bytes(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

/// The system ffmpeg with Opus (CI installs it and sets `ANYWHERE_REQUIRE_FFMPEG`).
fn system_ffmpeg() -> Option<anywhere_gateway::transcode::Ffmpeg> {
    let path = [
        "/usr/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/opt/homebrew/bin/ffmpeg",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|p| p.exists())?;
    let f = anywhere_gateway::transcode::probe(&path, Duration::from_secs(10))?;
    f.opus.then_some(f)
}

#[test]
fn messages_and_the_choice_of_level() {
    // A new session at the original level: the bare token, as always.
    assert_eq!(sign_in_message("tok", None, StreamQuality::Original), "tok");
    let v: serde_json::Value =
        serde_json::from_str(&sign_in_message("t\"k", None, StreamQuality::Low)).unwrap();
    assert_eq!(v, serde_json::json!({"token": "t\"k", "quality": "low"}));
    let v: serde_json::Value = serde_json::from_str(&sign_in_message(
        "t",
        Some(("s", "x", 4)),
        StreamQuality::High,
    ))
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"token": "t", "resume": {"session": "s", "secret": "x", "received": 4}, "quality": "high"})
    );
    let v: serde_json::Value = serde_json::from_str(&sign_in_message(
        "t",
        Some(("s", "x", 4)),
        StreamQuality::Original,
    ))
    .unwrap();
    assert!(v.get("quality").is_none());
    assert_eq!(quality_message(StreamQuality::Low), r#"{"quality":"low"}"#);
    assert_eq!(
        quality_message(StreamQuality::Original),
        r#"{"quality":"original"}"#
    );

    use StreamQuality::*;
    assert_eq!(stream_quality_for(true, Original, High), High);
    assert_eq!(stream_quality_for(false, Original, High), Original);
    assert_eq!(stream_quality_for(true, Low, Low), Low);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_saver_plays_opus_then_original_plays_flac() {
    let Some(ff) = system_ffmpeg() else {
        assert!(
            std::env::var("ANYWHERE_REQUIRE_FFMPEG").is_err(),
            "ffmpeg with libopus is required here"
        );
        eprintln!("skipping: no ffmpeg with libopus");
        return;
    };
    let h = Harness::start_with(|c| c.ffmpeg = Some(ff)).await;
    let session =
        anywhere_mobile::Session::new(h.server(), MemStore::new(), "Test phone".into()).unwrap();
    session
        .login("alice".into(), PW.into(), None)
        .await
        .unwrap();
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 192_000,
            cache_dir: None,
            read_ahead: true,
            dsp: Default::default(),
        },
    );
    player.set_stream_quality(StreamQuality::Low);
    assert_eq!(player.stream_quality(), StreamQuality::Low);
    let obs = Arc::new(Obs::default());
    player.start(obs.clone()).unwrap();
    let mut lms = h.lms.slim_conn().await;
    wait_for("HELO", || {
        h.lms.slim_events().first().map(String::as_str) == Some("HELO")
    })
    .await;
    let mac: [u8; 6] = h.lms.slim_payloads(b"HELO")[0][2..8].try_into().unwrap();

    // Pull like an audio device, noting the rate of what comes out.
    let rates: Arc<Mutex<Vec<(u32, usize)>>> = Default::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (p, r, s) = (player.clone(), rates.clone(), stop.clone());
    tokio::spawn(async move {
        while !s.load(Ordering::SeqCst) {
            let b = p.read_pcm(2048);
            let n = b.audio_samples as usize;
            if n > 0 {
                let mut r = r.lock().unwrap();
                match r.last_mut() {
                    Some((rate, total)) if *rate == b.sample_rate => *total += n,
                    _ => r.push((b.sample_rate, n)),
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });

    // Track 1 at Data saver: the gateway re-encodes it; the player hears Opus at 48 kHz.
    let secs = 2;
    let t1 = flac::encode(44_100, 16, &flac::noise(44_100 * secs, 16, 1));
    let t2 = flac::encode(44_100, 16, &flac::noise(44_100, 16, 2));
    {
        let mut s = h.lms.streams.lock().unwrap();
        s.push_back(StreamPlan::whole(t1));
        s.push_back(StreamPlan::whole(t2));
    }
    lms.write_all(&strm_s(b'f', &mac)).await.unwrap();
    wait_for("Opus on the signal path", || {
        obs.paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.source_codec == "Opus")
    })
    .await;
    let opus = obs
        .paths
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|p| p.source_codec == "Opus")
        .cloned()
        .unwrap();
    assert_eq!(opus.source_rate, 48_000);
    wait_for("track 1 decoded", || {
        h.lms.slim_events().iter().any(|e| e == "STMd")
    })
    .await;

    // Back on Wi-Fi: the change reaches the gateway mid-session; the next track is FLAC.
    player.set_stream_quality(StreamQuality::Original);
    tokio::time::sleep(Duration::from_millis(300)).await;
    lms.write_all(&strm_s(b'f', &mac)).await.unwrap();
    wait_for("FLAC on the signal path", || {
        obs.paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.source_codec == "FLAC")
    })
    .await;
    wait_for("both played out", || {
        h.lms.slim_events().iter().any(|e| e == "STMu")
    })
    .await;
    stop.store(true, Ordering::SeqCst);
    let rates = rates.lock().unwrap().clone();
    assert!(
        rates.iter().any(|(r, n)| *r == 48_000 && *n > 48_000 * 2),
        "{rates:?}"
    );
    assert!(rates.iter().any(|(r, _)| *r == 44_100), "{rates:?}");
    // One session throughout: the level changed without reconnecting.
    assert_eq!(
        h.lms.slim_events().iter().filter(|e| *e == "HELO").count(),
        1
    );
    player.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changing_level_without_the_feature_keeps_the_session() {
    // No ffmpeg: `/v1/me` doesn't list stream_quality, so the player sends no control
    // message (an older gateway would end the session on one) and the change waits for the
    // next sign-in. (This gateway would accept one anyway; the point here is that nothing
    // the player does on a change disturbs the session.)
    let h = Harness::start_with(|c| c.ffmpeg = None).await;
    let session =
        anywhere_mobile::Session::new(h.server(), MemStore::new(), "Test phone".into()).unwrap();
    session
        .login("alice".into(), PW.into(), None)
        .await
        .unwrap();
    let player = LocalPlayer::new(
        session.clone(),
        LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 192_000,
            cache_dir: None,
            read_ahead: true,
            dsp: Default::default(),
        },
    );
    player.set_stream_quality(StreamQuality::High);
    player.start(Arc::new(Obs::default())).unwrap();
    let _lms = h.lms.slim_conn().await;
    wait_for("HELO", || {
        h.lms.slim_events().first().map(String::as_str) == Some("HELO")
    })
    .await;
    for q in [
        StreamQuality::Low,
        StreamQuality::Original,
        StreamQuality::High,
    ] {
        player.set_stream_quality(q);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let helos = h.lms.slim_events().iter().filter(|e| *e == "HELO").count();
    assert_eq!(helos, 1, "the session was never ended and restarted");
    player.stop();
}
