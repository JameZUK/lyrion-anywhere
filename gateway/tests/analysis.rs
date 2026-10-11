//! Track analysis (`/v1/analysis/track/…`) against a fake LMS serving real (generated)
//! audio, hostile audio, and downloads that stall.

mod common;
use common::*;

use std::path::PathBuf;

use anywhere_core::analysis::TrackAnalysis;
use anywhere_gateway::analysis::BINARY_TYPE;
use reqwest::StatusCode as S;
use serde_json::Value;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "anywhere-an-{tag}-{}",
        anywhere_gateway::random_id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 16-bit stereo WAV of a 997 Hz tone at `db` dBFS, `secs` long.
fn wav_tone(rate: u32, secs: f64, db: f64) -> Vec<u8> {
    let frames = (secs * f64::from(rate)) as usize;
    let amp = 10f64.powf(db / 20.0);
    let data_len = frames * 4;
    let mut v = Vec::with_capacity(44 + data_len);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 4).to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        let x = amp * (2.0 * std::f64::consts::PI * 997.0 * i as f64 / f64::from(rate)).sin();
        let s = ((x * 32767.0).round() as i16).to_le_bytes();
        v.extend_from_slice(&s);
        v.extend_from_slice(&s);
    }
    v
}

struct An {
    h: Harness,
    dir: PathBuf,
}

async fn with_analysis(tweak: impl FnOnce(&mut Config)) -> An {
    let dir = scratch("cache");
    let d = dir.clone();
    let h = harness_with(move |c| {
        c.analysis_cache_dir = Some(d);
        tweak(c);
    })
    .await;
    // 10: a 3 s tone at -20 dBFS; 11: a corrupt FLAC; 12: an unsupported type.
    h.lms
        .add_track(10, "wav", wav_tone(44_100, 3.0, -20.0), 3.0);
    h.lms
        .add_track(11, "flc", b"fLaC\0\0\0\x22garbage".repeat(500), 3.0);
    h.lms.add_track(12, "wma", vec![0u8; 1000], 3.0);
    An { h, dir }
}

impl Harness {
    async fn analysis(
        &self,
        path: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut req = reqwest::Client::new().get(self.url(path));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.send().await.unwrap()
    }

    async fn track_analysis(&self, id: &str) -> reqwest::Response {
        self.analysis(&format!("/v1/analysis/track/{id}"), Some(&self.tok_a), &[])
            .await
    }
}

fn downloads_of(h: &Harness, id: u64) -> usize {
    let p = format!("/music/{id}/download");
    h.lms.downloads().iter().filter(|(d, _)| *d == p).count()
}

#[tokio::test]
async fn analyses_a_track_caches_it_and_answers_in_both_encodings() {
    let a = with_analysis(|_| {}).await;
    let h = &a.h;
    let r = h.track_analysis("10").await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["content-type"], "application/json");
    assert_eq!(r.headers()["cache-control"], "private, no-cache");
    let etag = r.headers()["etag"].to_str().unwrap().to_string();
    let body = r.text().await.unwrap();
    assert!(!body.contains("/srv/music"), "no paths: {body}");
    let an = TrackAnalysis::from_json(&body).unwrap();
    assert_eq!(an.sample_rate, 44_100);
    assert_eq!(an.frames, 3 * 44_100);
    assert!((an.integrated_lufs.unwrap() + 20.0).abs() < 0.2, "{an:?}");
    assert_eq!(an.waveform.peak.len(), an.waveform.rms.len());
    assert!(!an.waveform.peak.is_empty());
    assert_eq!(downloads_of(h, 10), 1);

    // Cached: answered again without fetching the file.
    let r = h.track_analysis("10").await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(downloads_of(h, 10), 1);
    assert!(h.gw.analysis().unwrap().cache_bytes() > 0);
    let files: Vec<_> = std::fs::read_dir(&a.dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].ends_with(".laan") && files[0].len() == 69);

    // Binary: by query or by Accept, the same analysis.
    for (path, headers) in [
        ("/v1/analysis/track/10?format=binary", vec![]),
        ("/v1/analysis/track/10", vec![("accept", BINARY_TYPE)]),
    ] {
        let r = h.analysis(path, Some(&h.tok_a), &headers).await;
        assert_eq!(r.status(), S::OK);
        assert_eq!(r.headers()["content-type"], BINARY_TYPE);
        let b = r.bytes().await.unwrap();
        assert!(b.len() < 2200);
        let bin = TrackAnalysis::from_bytes(&b).unwrap();
        assert_eq!(bin.waveform, an.waveform);
        assert_eq!(bin.frames, an.frames);
    }

    // Conditional requests.
    let r = h
        .analysis(
            "/v1/analysis/track/10",
            Some(&h.tok_a),
            &[("if-none-match", &etag)],
        )
        .await;
    assert_eq!(r.status(), S::NOT_MODIFIED);
    let r = h
        .analysis(
            "/v1/analysis/track/10?format=binary",
            Some(&h.tok_a),
            &[("if-none-match", &etag)],
        )
        .await;
    assert_eq!(r.status(), S::OK, "the binary form has its own tag");

    // Bob may see it too (the library is shared); still no second fetch.
    let r = h
        .analysis("/v1/analysis/track/10", Some(&h.tok_b), &[])
        .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(downloads_of(h, 10), 1);
}

#[tokio::test]
async fn the_cache_survives_a_restart_and_a_damaged_entry_is_redone() {
    let a = with_analysis(|_| {}).await;
    assert_eq!(a.h.track_analysis("10").await.status(), S::OK);
    let entry = std::fs::read_dir(&a.dir)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    // Damage it: the gateway notices, drops it and analyses again.
    std::fs::write(&entry, b"LAAN-not-really").unwrap();
    let r = a.h.track_analysis("10").await;
    assert_eq!(r.status(), S::OK);
    TrackAnalysis::from_json(&r.text().await.unwrap()).unwrap();
    assert_eq!(downloads_of(&a.h, 10), 2);
    assert!(TrackAnalysis::from_bytes(&std::fs::read(&entry).unwrap()).is_ok());
    // A new gateway on the same directory keeps the entry (it is LRU-indexed at start).
    let cfg = Config {
        analysis_cache_dir: Some(a.dir.clone()),
        ..Config::default()
    };
    let gw2 = Gateway::new(cfg, a.h.accounts.clone());
    assert!(gw2.analysis().unwrap().cache_bytes() > 0);
}

#[tokio::test]
async fn missing_invalid_and_foreign_credentials_are_refused() {
    let a = with_analysis(|_| {}).await;
    let h = &a.h;
    let r = h.analysis("/v1/analysis/track/10", None, &[]).await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    let r = h
        .analysis("/v1/analysis/track/10", Some("not-a-token"), &[])
        .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    let r = h
        .analysis("/v1/analysis/track/10", Some(&"x".repeat(5000)), &[])
        .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    // A revoked device.
    let dev = h.accounts.authenticate(&h.tok_b).unwrap().device_id;
    h.accounts.revoke_device(dev).unwrap();
    let r = h
        .analysis("/v1/analysis/track/10", Some(&h.tok_b), &[])
        .await;
    assert_eq!(r.status(), S::UNAUTHORIZED);
    assert_eq!(
        downloads_of(h, 10),
        0,
        "nothing fetched for refused requests"
    );
    assert!(h.lms.rpc_calls().is_empty(), "nothing looked up either");
}

#[tokio::test]
async fn users_without_download_permission_are_refused() {
    let a = with_analysis(|_| {}).await;
    let h = &a.h;
    h.accounts.set_downloads("alice", false, None).unwrap();
    let r = h.track_analysis("10").await;
    assert_eq!(r.status(), S::FORBIDDEN);
    assert!(h.lms.rpc_calls().is_empty());
    // Bob still may.
    let r = h
        .analysis("/v1/analysis/track/10", Some(&h.tok_b), &[])
        .await;
    assert_eq!(r.status(), S::OK);
}

#[tokio::test]
async fn bad_ids_traversal_and_unknown_parameters_are_refused() {
    let a = with_analysis(|_| {}).await;
    let h = &a.h;
    for id in [
        "0",
        "-1",
        "01",
        "1e3",
        "1.0",
        "abc",
        "..",
        "%2e%2e",
        "..%2F..%2Fmusic%2F1%2Fdownload",
        "1%2F..%2F2",
        "99999999999999999999999999",
        "0x10",
        "%00",
    ] {
        let r = h.track_analysis(id).await;
        assert_eq!(r.status(), S::NOT_FOUND, "{id}");
    }
    let r = h
        .analysis(
            "/v1/analysis/track/10/../../../music/10/download",
            Some(&h.tok_a),
            &[],
        )
        .await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert!(h.lms.rpc_calls().is_empty(), "malformed ids reached LMS");
    for q in [
        "format=cbor",
        "format=json&x=1",
        "wait=100",
        "format=",
        "format=../x",
    ] {
        let r = h
            .analysis(&format!("/v1/analysis/track/10?{q}"), Some(&h.tok_a), &[])
            .await;
        assert_eq!(r.status(), S::BAD_REQUEST, "{q}");
    }
    // Unknown ids: looked up, refused, never fetched (LMS would hang).
    let r = h.track_analysis("999").await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert!(h.lms.downloads().is_empty());
}

#[tokio::test]
async fn streams_and_unsupported_formats_are_refused_without_fetching() {
    let a = with_analysis(|_| {}).await;
    let h = &a.h;
    let r = h.track_analysis("2").await; // a radio stream
    assert_eq!(r.status(), S::NOT_FOUND);
    let r = h.track_analysis("12").await; // WMA
    assert_eq!(r.status(), S::UNSUPPORTED_MEDIA_TYPE);
    let r = h.track_analysis("5").await; // an unknown type
    assert_eq!(r.status(), S::UNSUPPORTED_MEDIA_TYPE);
    assert!(h.lms.downloads().is_empty());
}

#[tokio::test]
async fn oversize_and_overlong_tracks_are_refused() {
    let a = with_analysis(|c| {
        c.analysis_max_bytes = 100_000;
        c.analysis_max_duration = Duration::from_secs(60);
    })
    .await;
    let h = &a.h;
    // LMS says it's too big: refused before fetching.
    let r = h.track_analysis("10").await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    assert_eq!(downloads_of(h, 10), 0);
    // LMS says it's too long.
    h.lms
        .add_track(20, "wav", wav_tone(8_000, 0.5, -20.0), 4000.0);
    let r = h.track_analysis("20").await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    assert_eq!(downloads_of(h, 20), 0);
    // LMS understates the size: caught by the download's Content-Length.
    let mut item_lies = wav_tone(44_100, 1.0, -20.0);
    let real = item_lies.len();
    h.lms
        .add_track(21, "wav", std::mem::take(&mut item_lies), 1.0);
    h.lms.extra.lock().unwrap().get_mut(&21).unwrap().0["filesize"] = 1000.into();
    assert!(real > 100_000);
    let r = h.track_analysis("21").await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    // And LMS understates the duration: caught while decoding.
    let a2 = with_analysis(|c| c.analysis_max_duration = Duration::from_secs(2)).await;
    a2.h.lms
        .add_track(22, "wav", wav_tone(8_000, 4.0, -20.0), 1.0);
    let r = a2.h.track_analysis("22").await;
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "track too long to analyse"
    );
    // Remembered: not fetched again.
    assert_eq!(
        a2.h.track_analysis("22").await.status(),
        S::PAYLOAD_TOO_LARGE
    );
    assert_eq!(downloads_of(&a2.h, 22), 1);
}

#[tokio::test]
async fn corrupt_audio_is_a_clean_error_remembered_for_a_while() {
    let a = with_analysis(|_| {}).await;
    let h = &a.h;
    for id in ["11", "1"] {
        let r = h.track_analysis(id).await;
        assert_eq!(r.status(), S::UNPROCESSABLE_ENTITY, "{id}");
        assert_eq!(
            r.json::<Value>().await.unwrap()["error"],
            "the audio could not be analysed"
        );
        // Asked again: answered from memory, the file isn't fetched and decoded again.
        let r = h.track_analysis(id).await;
        assert_eq!(r.status(), S::UNPROCESSABLE_ENTITY);
        assert_eq!(downloads_of(h, id.parse().unwrap()), 1);
    }
    // Truncated, garbage and hostile files: an answer every time, and the gateway lives on.
    let base = wav_tone(8_000, 1.0, -20.0);
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for i in 0..24u64 {
        let mut b = base.clone();
        match i % 4 {
            0 => b.truncate((next() % 60) as usize),
            1 => {
                for _ in 0..8 {
                    let k = (next() % 44) as usize;
                    b[k] = next() as u8;
                }
            }
            2 => b = (0..next() % 3000).map(|_| next() as u8).collect(),
            _ => b.truncate(44 + (next() % 1000) as usize),
        }
        let kind = ["wav", "flc", "mp3", "ogg"][(i % 4) as usize];
        h.lms.add_track(100 + i, kind, b, 1.0);
        let r = h.track_analysis(&(100 + i).to_string()).await;
        assert!(
            matches!(r.status(), S::OK | S::UNPROCESSABLE_ENTITY),
            "{i}: {}",
            r.status()
        );
    }
    assert_eq!(h.track_analysis("10").await.status(), S::OK);
}

#[tokio::test]
async fn a_stalled_fetch_times_out_and_is_remembered() {
    let a = with_analysis(|c| {
        c.analysis_timeout = Duration::from_millis(400);
        c.analysis_wait = Duration::from_secs(5);
    })
    .await;
    let h = &a.h;
    let started = std::time::Instant::now();
    let r = h.track_analysis("8").await; // sends 1000 bytes, then stalls
    assert_eq!(r.status(), S::GATEWAY_TIMEOUT);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(h.track_analysis("8").await.status(), S::GATEWAY_TIMEOUT);
    assert_eq!(downloads_of(h, 8), 1);
    // The service is free again.
    assert_eq!(h.track_analysis("10").await.status(), S::OK);
}

#[tokio::test]
async fn slow_analyses_answer_202_then_the_result() {
    let a = with_analysis(|c| c.analysis_wait = Duration::from_millis(50)).await;
    let h = &a.h;
    // A minute of audio takes longer than 50 ms to fetch and analyse in a debug build.
    h.lms
        .add_track(30, "wav", wav_tone(44_100, 60.0, -20.0), 60.0);
    let r = h.track_analysis("30").await;
    assert_eq!(r.status(), S::ACCEPTED);
    assert_eq!(r.headers()["retry-after"], "3");
    assert_eq!(r.json::<Value>().await.unwrap()["status"], "pending");
    // Polling joins the same job: one fetch.
    let mut status = S::ACCEPTED;
    for _ in 0..600 {
        status = h.track_analysis("30").await.status();
        if status != S::ACCEPTED {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(status, S::OK);
    assert_eq!(downloads_of(h, 30), 1);
}

#[tokio::test]
async fn per_user_and_global_limits_give_429() {
    let a = with_analysis(|c| {
        c.analysis_per_user = 1;
        c.analysis_queue = 2;
        c.analysis_wait = Duration::from_millis(50);
        c.analysis_timeout = Duration::from_secs(30);
    })
    .await;
    let h = &a.h;
    // Alice's one job: a download that stalls.
    assert_eq!(h.track_analysis("8").await.status(), S::ACCEPTED);
    // Another track while it runs: over her limit.
    let r = h.track_analysis("10").await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert_eq!(r.headers()["retry-after"], "5");
    // Asking for the same track again joins the job: not refused.
    assert_eq!(h.track_analysis("8").await.status(), S::ACCEPTED);
    // Bob can start one (the queue holds two); it waits behind Alice's.
    let r = h
        .analysis("/v1/analysis/track/4", Some(&h.tok_b), &[])
        .await;
    assert_eq!(r.status(), S::ACCEPTED);
    // ...but the queue is now full for everyone.
    let accounts = &h.accounts;
    accounts.create_user("carol", PW, Role::Listener).unwrap();
    let c = accounts
        .login("carol", PW, None, "device-c", None, "test")
        .unwrap();
    let r = h
        .analysis("/v1/analysis/track/10", Some(&c.access), &[])
        .await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    // Cached results are never limited.
    let b = with_analysis(|c| c.analysis_per_user = 1).await;
    assert_eq!(b.h.track_analysis("10").await.status(), S::OK);
    for _ in 0..5 {
        assert_eq!(b.h.track_analysis("10").await.status(), S::OK);
    }
}

#[tokio::test]
async fn analysis_can_be_turned_off() {
    let h = harness().await; // no cache directory: off
    let r = h
        .analysis("/v1/analysis/track/1", Some(&h.tok_a), &[])
        .await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "analysis is not available on this server"
    );
    // Still 401 without credentials (nothing is revealed).
    assert_eq!(
        h.analysis("/v1/analysis/track/1", None, &[]).await.status(),
        S::UNAUTHORIZED
    );
    let me: Value = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["features"]["analysis"], false);
    let a = with_analysis(|_| {}).await;
    let me: Value = reqwest::Client::new()
        .get(a.h.url("/v1/me"))
        .bearer_auth(&a.h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["features"]["analysis"], true);
}
