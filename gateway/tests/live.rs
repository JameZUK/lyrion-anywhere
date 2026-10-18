//! Lower-data live streams (docs/plans/streaming-quality.md) against a fake LMS.
//!
//! The plumbing is tested with a stand-in encoder (a shell script that prefixes `OggS` to
//! its input), so results are exact:
//! - which streams are re-encoded, and the `strm` the player is sent;
//! - resume, LMS's byte counts, failures and the slot limit;
//! - the encoder is stopped when its stream is abandoned.
//!
//! One test runs the real `ffmpeg` and decodes its output with the players' own decoder.
//!
//! Security: the level can only be one of three words, a malformed control message ends
//! the session, and an unknown level at sign-in is taken as `original` (never a refusal).

mod common;
use common::flac::*;
use common::*;

use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use anywhere_core::proto::{ClientMsg, Stat};
use anywhere_gateway::transcode::Ffmpeg;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    scratch_dir("anywhere-live", tag)
}

/// A stand-in encoder: records its arguments and process id, then runs `body` (by default
/// `OggS` followed by its input).
fn fake_encoder(dir: &Path, body: Option<&str>) -> PathBuf {
    let path = dir.join("ffmpeg");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" '--' >> '{runs}'\necho $$ >> '{pids}'\n{body}\n",
        runs = dir.join("runs").display(),
        pids = dir.join("pids").display(),
        body = body.unwrap_or("printf 'OggS'; exec cat"),
    );
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn runs(dir: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(dir.join("runs"))
        .unwrap_or_default()
        .split("--\n")
        .filter(|r| !r.is_empty())
        .map(|r| r.lines().map(str::to_string).collect())
        .collect()
}

fn pids(dir: &Path) -> Vec<u32> {
    std::fs::read_to_string(dir.join("pids"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

struct Live {
    h: Harness,
    dir: PathBuf,
}

async fn live(body: Option<&str>, jobs: usize) -> Live {
    let dir = scratch("enc");
    let path = fake_encoder(&dir, body);
    let h = harness_with(|c| {
        c.ffmpeg = Some(Ffmpeg {
            path,
            mp3: true,
            opus: true,
        });
        c.live_transcode_jobs = jobs;
    })
    .await;
    Live { h, dir }
}

/// The `strm` payload inside a server frame sent to the player.
fn strm_payload(m: &Message) -> Vec<u8> {
    match m {
        Message::Binary(b) => {
            assert_eq!(&b[2..6], b"strm", "expected strm, got {:?}", &b[2..6]);
            b[6..].to_vec()
        }
        other => panic!("expected a frame, got {other:?}"),
    }
}

/// Sign in with `auth` (a JSON message or a bare token), say HELO, and have LMS send a
/// `strm s` in `format`. Returns the socket, the session id, LMS's end and the `strm` the
/// player received.
/// Which of the harness's two devices.
#[derive(Clone, Copy)]
enum Who {
    A,
    B,
}
use Who::{A, B};

async fn announce(
    h: &mut Harness,
    auth: String,
    who: Who,
    format: u8,
) -> (Ws, String, OwnedWriteHalf, Vec<u8>) {
    let mac = match who {
        A => h.mac_a,
        B => h.mac_b,
    };
    let mut ws = h.dial().await;
    ws.send(Message::Text(auth.into())).await.unwrap();
    let g = loop {
        if let Message::Text(t) = next(&mut ws).await {
            break serde_json::from_str::<serde_json::Value>(&t).unwrap();
        }
    };
    let sid = g["session"].as_str().unwrap().to_string();
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(mac).into())).await.unwrap();
    lms.write_all(&strm_format(
        b's',
        format,
        Ipv4Addr::UNSPECIFIED,
        &stream_req(mac),
    ))
    .await
    .unwrap();
    let strm = strm_payload(&next(&mut ws).await);
    (ws, sid, lms, strm)
}

fn auth(token: &str, quality: Option<&str>) -> String {
    match quality {
        Some(q) => serde_json::json!({ "token": token, "quality": q }).to_string(),
        None => serde_json::json!({ "token": token }).to_string(),
    }
}

/// LMS sends another `strm s` (a seek or the next track) and the player receives it.
async fn restream(ws: &mut Ws, lms: &mut OwnedWriteHalf, mac: [u8; 6], format: u8) -> Vec<u8> {
    lms.write_all(&strm_format(
        b's',
        format,
        Ipv4Addr::UNSPECIFIED,
        &stream_req(mac),
    ))
    .await
    .unwrap();
    strm_payload(&next(ws).await)
}

async fn fetch_all(h: &Harness, sid: &str, n: u64, tok: &str) -> (Vec<u8>, String) {
    let mut resp = h.get_stream(sid, n, Some(tok)).await;
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
    let mut got = Vec::new();
    while let Some(c) = resp.chunk().await.unwrap() {
        got.extend_from_slice(&c);
    }
    (got, ct)
}

fn ogg_of(file: &[u8]) -> Vec<u8> {
    let mut v = b"OggS".to_vec();
    v.extend_from_slice(file);
    v
}

#[tokio::test]
async fn high_quality_flac_is_re_encoded_and_the_player_is_told_opus() {
    let Live { mut h, dir } = live(None, 4).await;
    let file = library_flac(1000, 20);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (_ws, sid, _lms, strm) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    assert_eq!(
        &strm[..7],
        b"s1u????",
        "format Ogg Opus, PCM parameters unknown"
    );
    let (got, ct) = fetch_all(&h, &sid, 1, &tok).await;
    assert_eq!(
        got,
        ogg_of(&file),
        "the encoder's output, from the whole FLAC stream"
    );
    assert_eq!(ct, "audio/ogg");
    let r = runs(&dir);
    assert_eq!(r.len(), 1);
    let pos = r[0].iter().position(|a| a == "-b:a").unwrap();
    assert_eq!(r[0][pos + 1], "160k");
    assert!(r[0].contains(&"pipe:0".to_string()) && r[0].contains(&"libopus".to_string()));
    assert_eq!(
        h.lms
            .http_paths()
            .iter()
            .filter(|p| p.starts_with("/stream"))
            .count(),
        1
    );
}

#[tokio::test]
async fn data_saver_is_96k_and_set_by_a_control_message() {
    let Live { mut h, dir } = live(None, 4).await;
    let file = library_flac(0, 10);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    // Signed in without a level: the first stream is sent as it is.
    let (mut ws, sid, mut lms, strm) = announce(&mut h, auth(&tok, None), A, b'f').await;
    assert_eq!(strm[2], b'f');
    // Mobile data: the next stream is re-encoded.
    ws.send(Message::Text(r#"{"quality":"low"}"#.into()))
        .await
        .unwrap();
    let strm = restream(&mut ws, &mut lms, h.mac_a, b'f').await;
    assert_eq!(strm[2], b'u');
    let (got, _) = fetch_all(&h, &sid, 2, &tok).await;
    assert_eq!(got, ogg_of(&file));
    let r = runs(&dir);
    assert_eq!(r.len(), 1);
    assert_eq!(
        r[0][r[0].iter().position(|a| a == "-b:a").unwrap() + 1],
        "96k"
    );
    // Back on Wi-Fi.
    ws.send(Message::Text(r#"{"quality":"original"}"#.into()))
        .await
        .unwrap();
    let strm = restream(&mut ws, &mut lms, h.mac_a, b'f').await;
    assert_eq!(strm[2], b'f');
    let (got, _) = fetch_all(&h, &sid, 3, &tok).await;
    assert!(got.starts_with(b"fLaC"));
    assert_eq!(runs(&dir).len(), 1, "not re-encoded");
}

#[tokio::test]
async fn only_flac_is_re_encoded() {
    let Live { mut h, dir } = live(None, 4).await;
    let file = library_flac(0, 5);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (mut ws, sid, mut lms, strm) = announce(&mut h, auth(&tok, Some("low")), A, b'm').await;
    assert_eq!(strm[2], b'm');
    let (got, _) = fetch_all(&h, &sid, 1, &tok).await;
    assert_eq!(got, file, "passed through");
    for (i, f) in b"oaupl".iter().copied().enumerate() {
        let s = restream(&mut ws, &mut lms, h.mac_a, f).await;
        assert_eq!(s[2], f, "{}", f as char);
        let (got, _) = fetch_all(&h, &sid, i as u64 + 2, &tok).await;
        assert_eq!(got, file);
    }
    assert!(runs(&dir).is_empty());
}

#[tokio::test]
async fn a_resumed_stream_continues_the_encoded_stream() {
    let Live { mut h, dir: _dir } = live(None, 4).await;
    let file = library_flac(0, 120);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (_ws, sid, _lms, _) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    let whole = ogg_of(&file);
    let mut got = Vec::new();
    for stop in [3usize, 70_000, 200_000] {
        let mut resp = reqwest::Client::new()
            .get(h.url(&format!("/v1/stream/{sid}/1?offset={}", got.len())))
            .bearer_auth(&tok)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        while got.len() < stop {
            got.extend_from_slice(&resp.chunk().await.unwrap().unwrap());
        }
        drop(resp);
    }
    let rest = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1?offset={}", got.len())))
        .bearer_auth(&tok)
        .send()
        .await
        .unwrap();
    got.extend_from_slice(&rest.bytes().await.unwrap());
    assert_eq!(got, whole, "identical to an unbroken fetch");
    assert_eq!(
        h.lms
            .http_paths()
            .iter()
            .filter(|p| p.starts_with("/stream"))
            .count(),
        1,
        "LMS asked once"
    );
    let beyond = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1?offset={}", whole.len() + 1)))
        .bearer_auth(&tok)
        .send()
        .await
        .unwrap();
    assert_eq!(beyond.status(), 416);
}

#[tokio::test]
async fn lms_hears_the_bytes_it_sent() {
    let Live { mut h, dir: _dir } = live(None, 4).await;
    let file = library_flac(0, 30);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (mut ws, sid, _lms, _) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    let (got, _) = fetch_all(&h, &sid, 1, &tok).await;
    let stat = |bytes: u64| {
        ClientMsg::Stat(Stat {
            event: *b"STMt",
            bytes_received: bytes,
            ..Default::default()
        })
        .encode()
    };
    let total = got.len() as u64;
    for b in [0, total / 2, total, u64::MAX] {
        ws.send(Message::Binary(stat(b).into())).await.unwrap();
    }
    wait_for(|| h.lms.stats().len() == 4).await;
    let seen: Vec<u64> = h.lms.stats().iter().map(|s| s.bytes_received).collect();
    let f = file.len() as u64;
    assert_eq!(seen[0], 0);
    assert_eq!(seen[1], (total / 2) * f / total, "in proportion");
    assert_eq!(
        seen[2], f,
        "all of it, once the player has the whole stream"
    );
    assert_eq!(seen[3], f, "never more than LMS sent");
}

#[tokio::test]
async fn a_failed_encoder_ends_the_stream_with_an_error() {
    let Live { mut h, dir: _dir } = live(Some("printf 'OggS'; cat > /dev/null; exit 1"), 4).await;
    let file = library_flac(0, 10);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (_ws, sid, _lms, _) = announce(&mut h, auth(&tok, Some("low")), A, b'f').await;
    let mut resp = h.get_stream(&sid, 1, Some(&tok)).await;
    assert_eq!(resp.status(), 200);
    let mut got = Vec::new();
    let ended = loop {
        match tokio::time::timeout(Duration::from_secs(20), resp.chunk())
            .await
            .expect("never hangs")
        {
            Ok(Some(c)) => got.extend_from_slice(&c),
            Ok(None) => break "clean",
            Err(_) => break "error",
        }
    };
    assert_eq!(got, b"OggS");
    assert_eq!(ended, "error", "never mistaken for the end of the track");
}

#[tokio::test]
async fn an_encoder_that_cant_start_is_a_bad_gateway() {
    let dir = scratch("missing");
    let h = harness_with(|c| {
        c.ffmpeg = Some(Ffmpeg {
            path: dir.join("no-such-ffmpeg"),
            mp3: true,
            opus: true,
        });
    })
    .await;
    let mut h = h;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 5)));
    let tok = h.tok_a.clone();
    let (_ws, sid, _lms, strm) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    assert_eq!(strm[2], b'u');
    let resp = h.get_stream(&sid, 1, Some(&tok)).await;
    assert_eq!(resp.status(), 502);
    // A retry never runs an encoder outside the slot limit.
    let resp = h.get_stream(&sid, 1, Some(&tok)).await;
    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn beyond_the_slot_limit_streams_are_sent_as_they_are() {
    let Live { mut h, dir } = live(None, 1).await;
    let file = library_flac(0, 10);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let (ta, tb) = (h.tok_a.clone(), h.tok_b.clone());
    let (mut wa, _sa, mut la, strm_a) = announce(&mut h, auth(&ta, Some("high")), A, b'f').await;
    assert_eq!(strm_a[2], b'u');
    let (_wb, sb, _lb, strm_b) = announce(&mut h, auth(&tb, Some("high")), B, b'f').await;
    assert_eq!(strm_b[2], b'f', "no free slot: the original");
    let (got, _) = fetch_all(&h, &sb, 1, &tb).await;
    assert!(got.starts_with(b"fLaC"));
    // A's next stream gives its slot back first, so it can have it again.
    let again = restream(&mut wa, &mut la, h.mac_a, b'f').await;
    assert_eq!(again[2], b'u');
    assert!(runs(&dir).is_empty(), "no encoder started without a fetch");
}

#[tokio::test]
async fn no_encoder_means_no_feature_and_no_rewrite() {
    for tweak in [0usize, 1] {
        let mut h = harness_with(|c| {
            if tweak == 1 {
                // ffmpeg without Opus, or live encoding switched off.
                c.ffmpeg = Some(Ffmpeg {
                    path: "/usr/bin/ffmpeg".into(),
                    mp3: true,
                    opus: false,
                });
            } else {
                c.live_transcode_jobs = 0;
            }
        })
        .await;
        let me: serde_json::Value = reqwest::Client::new()
            .get(h.url("/v1/me"))
            .bearer_auth(&h.tok_a)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(me["features"]["stream_quality"], false);
        *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 5)));
        let tok = h.tok_a.clone();
        let (_ws, _sid, _lms, strm) = announce(&mut h, auth(&tok, Some("low")), A, b'f').await;
        assert_eq!(strm[2], b'f');
    }
    let Live { h, dir: _dir } = live(None, 4).await;
    let me: serde_json::Value = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["features"]["stream_quality"], true);
    assert_eq!(me["features"]["queue_edit"], true);
    assert_eq!(me["features"]["resume_grace"], true);
}

#[tokio::test]
async fn hostile_control_messages_end_the_session() {
    let bad = [
        r#"{"quality":"ultra"}"#,
        r#"{"quality":"HIGH"}"#,
        r#"{"quality":1}"#,
        r#"{"quality":null}"#,
        r#"{"quality":"low","ack":{"stream":1,"bytes":1}}"#,
        r#"{"quality":"low","x":1}"#,
        r#"{}"#,
    ];
    for msg in bad {
        let mut h = harness().await;
        let tok = h.tok_a.clone();
        let mut ws = h.dial().await;
        ws.send(Message::Text(auth(&tok, None).into()))
            .await
            .unwrap();
        next(&mut ws).await;
        let _lms = h.lms.accept().await;
        ws.send(Message::Text(msg.into())).await.unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
    }
    // Too long.
    let mut h = harness().await;
    let tok = h.tok_a.clone();
    let mut ws = h.dial().await;
    ws.send(Message::Text(auth(&tok, None).into()))
        .await
        .unwrap();
    next(&mut ws).await;
    let _lms = h.lms.accept().await;
    let long = format!(r#"{{"quality":"{}"}}"#, "l".repeat(300));
    ws.send(Message::Text(long.into())).await.unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
}

#[tokio::test]
async fn an_unknown_level_at_sign_in_is_original_never_a_refusal() {
    let Live { mut h, dir } = live(None, 4).await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 5)));
    let tok = h.tok_a.clone();
    for q in ["ultra", "", "HIGH"] {
        let (_ws, _sid, _lms, strm) = announce(&mut h, auth(&tok, Some(q)), A, b'f').await;
        assert_eq!(strm[2], b'f', "{q:?}");
    }
    // A level that isn't a string is a malformed sign-in, as before.
    let mut ws = h.dial().await;
    ws.send(Message::Text(
        serde_json::json!({"token": tok, "quality": 5})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    assert!(runs(&dir).is_empty());
}

#[tokio::test]
async fn the_encoder_is_stopped_when_its_stream_is_abandoned() {
    // Never finishes on its own: waits for more input after the stream.
    let Live { mut h, dir } = live(Some("printf 'OggS'; exec sleep 600"), 4).await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 5)));
    let tok = h.tok_a.clone();
    let (mut ws, sid, mut lms, _) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    let mut resp = h.get_stream(&sid, 1, Some(&tok)).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(&resp.chunk().await.unwrap().unwrap()[..], b"OggS");
    wait_for(|| !pids(&dir).is_empty()).await;
    let pid = pids(&dir)[0];
    let alive = |pid: u32| Path::new(&format!("/proc/{pid}")).exists();
    assert!(alive(pid));
    // The next track: the old stream is abandoned and its client goes away.
    drop(resp);
    let _ = restream(&mut ws, &mut lms, h.mac_a, b'm').await;
    wait_for(|| !alive(pid)).await;
}

/// Regression: a stream superseded by the next `strm s` kept its LMS connection, encoder
/// and encode slot until it timed out, as long as its client kept the response open.
#[tokio::test]
async fn a_superseded_stream_is_cancelled_even_if_its_client_stays() {
    let Live { mut h, dir } = live(Some("printf 'OggS'; exec sleep 600"), 1).await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 5)));
    let tok = h.tok_a.clone();
    let (mut ws, sid, mut lms, _) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    let mut resp = h.get_stream(&sid, 1, Some(&tok)).await;
    assert_eq!(&resp.chunk().await.unwrap().unwrap()[..], b"OggS");
    wait_for(|| !pids(&dir).is_empty()).await;
    let pid = pids(&dir)[0];
    let alive = |pid: u32| Path::new(&format!("/proc/{pid}")).exists();
    // The next track, with the old response still held open.
    let _ = restream(&mut ws, &mut lms, h.mac_a, b'm').await;
    wait_for(|| !alive(pid)).await;
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(Some(_)) = resp.chunk().await {}
    })
    .await;
    assert!(end.is_ok(), "the old response ends");
    // Its encode slot (the only one) is free again.
    let strm = restream(&mut ws, &mut lms, h.mac_a, b'f').await;
    assert_eq!(strm[2], b'u', "the next FLAC stream is re-encoded");
}

// ---------------------------------------------------------------------------
// The real ffmpeg
// ---------------------------------------------------------------------------

/// The system ffmpeg with Opus, if installed. CI sets `ANYWHERE_REQUIRE_FFMPEG=1`, so the
/// test below can't be skipped there.
fn system_ffmpeg() -> Option<Ffmpeg> {
    let path = [
        "/usr/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/opt/homebrew/bin/ffmpeg",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())?;
    let f = anywhere_gateway::transcode::probe(&path, Duration::from_secs(10))?;
    f.opus.then_some(f)
}

/// Decode with the players' own decoder; returns interleaved stereo samples.
fn decode(bytes: &[u8], format: u8) -> Vec<f32> {
    let input = InputBuffer::new();
    input.push(bytes);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, format);
    let mut pcm = Vec::new();
    loop {
        match dec.work(64, &mut |_, s| pcm.extend_from_slice(s)) {
            DecodeStatus::Finished => return pcm,
            DecodeStatus::Failed(e) => panic!("decode failed: {e}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn real_ffmpeg_makes_opus_the_players_can_decode() {
    let Some(ff) = system_ffmpeg() else {
        assert!(
            std::env::var("ANYWHERE_REQUIRE_FFMPEG").is_err(),
            "ffmpeg with libopus is required here"
        );
        eprintln!("skipping: no ffmpeg with libopus");
        return;
    };
    let mut h = harness_with(|c| c.ffmpeg = Some(ff)).await;
    let frames = 400; // about 10.4 s at 44.1 kHz
    let file = library_flac(0, frames);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (mut ws, sid, mut lms, strm) = announce(&mut h, auth(&tok, Some("low")), A, b'f').await;
    assert_eq!(strm[2], b'u');
    let (got, ct) = fetch_all(&h, &sid, 1, &tok).await;
    assert_eq!(ct, "audio/ogg");
    assert!(got.starts_with(b"OggS"));
    // The test signal is noise, Opus's worst case, yet it is a fraction of the FLAC.
    assert!(
        got.len() * 4 < file.len(),
        "{} vs {}",
        got.len(),
        file.len()
    );
    let pcm = decode(&got, b'u');
    let secs = (frames * BLOCK) as f64 / 44_100.0;
    let decoded = pcm.len() as f64 / 2.0 / 48_000.0;
    assert!(
        (decoded - secs).abs() < 0.1,
        "decoded {decoded:.3} s of {secs:.3} s"
    );

    // A seek: LMS sends the header and then audio from a frame in the middle; the encoder
    // copes.
    let audio = audio_offset(&file);
    let frame = frames / 2;
    let start = audio
        + file[audio..]
            .windows(4)
            .enumerate()
            .filter(|(_, w)| *w == [0xFF, 0xF8, 0x39, 0x18])
            .nth(frame)
            .unwrap()
            .0;
    let mut seeked = file[..audio].to_vec();
    seeked.extend_from_slice(&file[start..]);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(seeked));
    assert_eq!(restream(&mut ws, &mut lms, h.mac_a, b'f').await[2], b'u');
    let (got, _) = fetch_all(&h, &sid, 2, &tok).await;
    let pcm = decode(&got, b'u');
    let want = ((frames - frame) * BLOCK) as f64 / 44_100.0;
    let decoded = pcm.len() as f64 / 2.0 / 48_000.0;
    assert!(
        (decoded - want).abs() < 0.1,
        "decoded {decoded:.3} s of {want:.3} s after a seek"
    );
}

/// Rolling cache S3: the replay cache matches a stream by its first bytes, so two encodes
/// of the same track at the same level must be the same bytes (no random Ogg serial, no
/// version strings).
#[tokio::test]
async fn real_ffmpeg_encodes_the_same_track_to_the_same_bytes() {
    let Some(ff) = system_ffmpeg() else {
        assert!(
            std::env::var("ANYWHERE_REQUIRE_FFMPEG").is_err(),
            "ffmpeg with libopus is required here"
        );
        eprintln!("skipping: no ffmpeg with libopus");
        return;
    };
    let mut h = harness_with(|c| c.ffmpeg = Some(ff)).await;
    let file = library_flac(0, 200);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let tok = h.tok_a.clone();
    let (mut ws, sid, mut lms, strm) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    assert_eq!(strm[2], b'u');
    let (first, _) = fetch_all(&h, &sid, 1, &tok).await;
    assert_eq!(restream(&mut ws, &mut lms, h.mac_a, b'f').await[2], b'u');
    let (second, _) = fetch_all(&h, &sid, 2, &tok).await;
    assert!(first.len() > 1000);
    assert!(first == second, "two encodes of one track differ");
    // A different track is different bytes (the check above isn't vacuous).
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 150)));
    restream(&mut ws, &mut lms, h.mac_a, b'f').await;
    let (other, _) = fetch_all(&h, &sid, 3, &tok).await;
    assert!(other != first);
}

#[tokio::test]
async fn real_ffmpeg_ends_a_corrupt_stream_with_an_error() {
    let Some(ff) = system_ffmpeg() else {
        assert!(std::env::var("ANYWHERE_REQUIRE_FFMPEG").is_err());
        return;
    };
    let mut h = harness_with(|c| c.ffmpeg = Some(ff)).await;
    // Not FLAC at all: ffmpeg refuses it, and the stream ends with an error.
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(b"MZ\x90\0not audio at all".repeat(5000)));
    let tok = h.tok_a.clone();
    let (mut ws, sid, mut lms, _) = announce(&mut h, auth(&tok, Some("high")), A, b'f').await;
    assert_eq!(ends(&h, &sid, 1, &tok).await, "error");
    // FLAC whose frames are garbage: whatever ffmpeg makes of it, nothing hangs or
    // panics, and the next stream on the session still works.
    let mut junk = library_flac(0, 50);
    let audio = audio_offset(&junk);
    for (i, b) in junk[audio..].iter_mut().enumerate() {
        *b = (i * 7919 % 251) as u8;
    }
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(junk));
    restream(&mut ws, &mut lms, h.mac_a, b'f').await;
    ends(&h, &sid, 2, &tok).await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(library_flac(0, 50)));
    restream(&mut ws, &mut lms, h.mac_a, b'f').await;
    let (got, _) = fetch_all(&h, &sid, 3, &tok).await;
    assert!(!decode(&got, b'u').is_empty());
}

/// Read stream `n` to its end: "clean" or "error". Fails if it hangs.
async fn ends(h: &Harness, sid: &str, n: u64, tok: &str) -> &'static str {
    let mut resp = h.get_stream(sid, n, Some(tok)).await;
    assert_eq!(resp.status(), 200);
    loop {
        match tokio::time::timeout(Duration::from_secs(30), resp.chunk())
            .await
            .expect("never hangs")
        {
            Ok(Some(_)) => {}
            Ok(None) => return "clean",
            Err(_) => return "error",
        }
    }
}
