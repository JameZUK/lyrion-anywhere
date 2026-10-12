//! FLAC streams through the relay lose their embedded artwork and padding: the audio is
//! untouched, resuming with `?offset=` still works, LMS still hears its own byte counts,
//! and other streams pass through byte for byte. Also measures what it saves before the
//! first audio, on a simulated slow link.

mod common;
use common::*;

use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use anywhere_core::proto::{ClientMsg, Stat};

// ---------------------------------------------------------------------------
// A small FLAC writer (16-bit stereo, verbatim subframes)
// ---------------------------------------------------------------------------

const BLOCK: usize = 1152;

fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut crc, b| {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |mut crc, b| {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
        crc
    })
}

fn block(last: bool, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![(u8::from(last) << 7) | kind];
    v.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    v.extend_from_slice(body);
    v
}

/// A FLAC file laid out as library files usually are: STREAMINFO, SEEKTABLE,
/// VORBIS_COMMENT, PICTURE (`picture` bytes of image), PADDING, then `frames` frames.
fn library_flac(picture: usize, frames: usize) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    let mut si = Vec::new();
    si.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    si.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    si.extend_from_slice(&[0; 6]);
    let packed: u64 = (44_100u64 << 44) | (1 << 41) | (15 << 36) | (frames * BLOCK) as u64;
    si.extend_from_slice(&packed.to_be_bytes());
    si.extend_from_slice(&[0; 16]);
    v.extend(block(false, 0, &si));
    v.extend(block(false, 3, &[0; 36]));
    let mut vc = 4u32.to_le_bytes().to_vec();
    vc.extend_from_slice(b"test");
    vc.extend_from_slice(&1u32.to_le_bytes());
    vc.extend_from_slice(&10u32.to_le_bytes());
    vc.extend_from_slice(b"TITLE=Test");
    v.extend(block(false, 4, &vc));
    let mut pic = 3u32.to_be_bytes().to_vec();
    pic.extend_from_slice(&10u32.to_be_bytes());
    pic.extend_from_slice(b"image/jpeg");
    pic.extend_from_slice(&0u32.to_be_bytes());
    for x in [600u32, 600, 24, 0, picture as u32] {
        pic.extend_from_slice(&x.to_be_bytes());
    }
    pic.extend((0..picture).map(|i| (i * 31 % 253) as u8));
    v.extend(block(false, 6, &pic));
    v.extend(block(true, 1, &[0; 8192]));
    let mut seed = 0x1234_5678_9abc_def1u64;
    for n in 0..frames {
        let mut f = vec![0xFF, 0xF8, 0x39, 0x18];
        if n < 0x80 {
            f.push(n as u8);
        } else {
            f.extend_from_slice(&[0xC0 | (n >> 6) as u8, 0x80 | (n & 0x3F) as u8]);
        }
        f.push(crc8(&f));
        for _channel in 0..2 {
            f.push(0x02); // a verbatim subframe
            for _ in 0..BLOCK {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                f.extend_from_slice(&((seed >> 40) as i16 / 8).to_be_bytes());
            }
        }
        let c = crc16(&f);
        f.extend_from_slice(&c.to_be_bytes());
        v.extend(f);
    }
    v
}

/// Where the first audio frame starts.
fn audio_offset(flac: &[u8]) -> usize {
    let mut pos = 4;
    loop {
        let len = u32::from_be_bytes([0, flac[pos + 1], flac[pos + 2], flac[pos + 3]]) as usize;
        let last = flac[pos] & 0x80 != 0;
        pos += 4 + len;
        if last {
            return pos;
        }
    }
}

/// Decode with the players' own stream decoder.
fn decode(bytes: &[u8]) -> Vec<f32> {
    let input = InputBuffer::new();
    input.push(bytes);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, b'f');
    let mut pcm = Vec::new();
    loop {
        match dec.work(64, &mut |_, s| pcm.extend_from_slice(s)) {
            DecodeStatus::Finished => return pcm,
            DecodeStatus::Failed(e) => panic!("decode failed: {e}"),
            _ => {}
        }
    }
}

/// Feed `bytes` to the players' decoder `chunk` bytes at a time, as they would arrive;
/// returns how many had arrived when `ms` of audio had been decoded (the players' quick
/// start after a seek waits for 0.5 s).
fn bytes_before_audio(bytes: &[u8], chunk: usize, ms: u64) -> usize {
    let input = InputBuffer::new();
    let mut dec = StreamDecoder::new(input.clone(), b'f');
    let want = 44_100 * ms as usize / 1000 * 2;
    let (mut fed, mut decoded) = (0, 0);
    loop {
        match dec.work(64, &mut |_, s| decoded += s.len()) {
            DecodeStatus::Failed(e) => panic!("decode failed: {e}"),
            DecodeStatus::Finished => return fed,
            _ => {}
        }
        if decoded >= want {
            return fed;
        }
        if fed == bytes.len() {
            input.set_eof();
            continue;
        }
        let n = chunk.min(bytes.len() - fed);
        input.push(&bytes[fed..fed + n]);
        fed += n;
    }
}

// ---------------------------------------------------------------------------
// Through the gateway
// ---------------------------------------------------------------------------

/// Open a session, have LMS announce a stream in `format`, and return the stream URL.
async fn announce(h: &mut Harness, format: u8) -> (Ws, String, OwnedWriteHalf) {
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    let g = loop {
        if let Message::Text(t) = next(&mut ws).await {
            break serde_json::from_str::<serde_json::Value>(&t).unwrap();
        }
    };
    let sid = g["session"].as_str().unwrap().to_string();
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    lms.write_all(&strm_format(
        b's',
        format,
        Ipv4Addr::UNSPECIFIED,
        &stream_req(h.mac_a),
    ))
    .await
    .unwrap();
    next(&mut ws).await;
    (ws, sid, lms)
}

async fn fetch_all(h: &Harness, sid: &str) -> Vec<u8> {
    let mut resp = h.get_stream(sid, 1, Some(&h.tok_a)).await;
    assert_eq!(resp.status(), 200);
    let mut got = Vec::new();
    while let Some(c) = resp.chunk().await.unwrap() {
        got.extend_from_slice(&c);
    }
    got
}

#[tokio::test]
async fn flac_stream_loses_its_artwork_and_keeps_its_audio() {
    let mut h = harness().await;
    let file = library_flac(200_000, 40);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let (_ws, sid, _lms) = announce(&mut h, b'f').await;
    let got = fetch_all(&h, &sid).await;

    let audio = audio_offset(&file);
    assert!(
        got.len() < file.len() - 200_000,
        "artwork and padding dropped"
    );
    assert_eq!(
        &got[audio_offset(&got)..],
        &file[audio..],
        "frames byte-identical"
    );
    assert!(
        audio_offset(&got) < 200,
        "metadata now {} bytes",
        audio_offset(&got)
    );
    let (a, b) = (decode(&file), decode(&got));
    assert_eq!(a.len(), 40 * BLOCK * 2);
    assert!(
        a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
        "decoded audio bit-identical"
    );
}

#[tokio::test]
async fn a_resumed_flac_download_continues_the_stripped_stream() {
    let mut h = harness().await;
    let file = library_flac(300_000, 80);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let (_ws, sid, _lms) = announce(&mut h, b'f').await;
    let whole = {
        // What a single uninterrupted fetch would have given.
        let mut v = file[..4].to_vec();
        let audio = audio_offset(&file);
        let kept_end = audio - (4 + 8192) - (4 + 300_000 + 42);
        v.extend_from_slice(&file[4..kept_end]);
        v[4 + 38 + 40] |= 0x80; // the comment is now the last block
        v.extend_from_slice(&file[audio..]);
        v
    };

    // Break the connection twice: once just inside the metadata, once in the audio.
    let mut got = Vec::new();
    for stop in [20usize, 60_000] {
        let mut resp = reqwest::Client::new()
            .get(h.url(&format!("/v1/stream/{sid}/1?offset={}", got.len())))
            .bearer_auth(&h.tok_a)
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
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    got.extend_from_slice(&rest.bytes().await.unwrap());
    assert_eq!(got, whole, "resumed stream identical to an unbroken one");
    assert_eq!(h.lms.http_paths().len(), 1, "LMS asked only once");

    // Offsets are in the stripped stream's terms: just past its end is out of range.
    let beyond = reqwest::Client::new()
        .get(h.url(&format!("/v1/stream/{sid}/1?offset={}", whole.len() + 1)))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(beyond.status(), 416);
}

#[tokio::test]
async fn lms_hears_the_bytes_it_sent() {
    let mut h = harness().await;
    let file = library_flac(100_000, 10);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let (mut ws, sid, _lms) = announce(&mut h, b'f').await;
    let got = fetch_all(&h, &sid).await;
    let removed = (file.len() - got.len()) as u64;
    let header = audio_offset(&got) as u64;

    let stat = |bytes: u64| {
        ClientMsg::Stat(Stat {
            event: *b"STMt",
            bytes_received: bytes,
            ..Default::default()
        })
        .encode()
    };
    // (A hostile count can't overflow: it saturates.)
    for b in [0, header - 1, header, got.len() as u64, u64::MAX] {
        ws.send(Message::Binary(stat(b).into())).await.unwrap();
    }
    wait_for(|| h.lms.stats().len() == 5).await;
    let seen: Vec<u64> = h.lms.stats().iter().map(|s| s.bytes_received).collect();
    assert_eq!(
        seen,
        [0, header - 1, header + removed, file.len() as u64, u64::MAX],
        "past the metadata, LMS's own count"
    );
}

#[tokio::test]
async fn non_flac_streams_and_unusual_flac_pass_through_unchanged() {
    let file = library_flac(50_000, 10);
    let mut no_sync = file.clone();
    let at = audio_offset(&no_sync);
    no_sync[at] = 0x00; // no frame after the metadata: leave it alone
    let cases: [(&str, u8, Vec<u8>, bool); 4] = [
        ("announced as MP3", b'm', file.clone(), true),
        (
            "FLAC without a frame after the metadata",
            b'f',
            no_sync,
            true,
        ),
        ("the fake LMS's usual body", b'f', vec![], true),
        ("stripping turned off", b'f', file.clone(), false),
    ];
    for (what, format, body, strip) in cases {
        let mut h = harness_with(|c| c.strip_flac_pictures = strip).await;
        let body = if body.is_empty() {
            h.lms.audio.as_ref().clone()
        } else {
            body
        };
        *h.lms.stream_body.lock().unwrap() = Some(Arc::new(body.clone()));
        let (_ws, sid, _lms) = announce(&mut h, format).await;
        assert!(fetch_all(&h, &sid).await == body, "{what}");
    }
}

/// Before and after, on a simulated link of 80 ms round trip and 8 Mbit/s: bytes the
/// player must receive before 0.5 s of audio is decoded (when quick start begins playing),
/// for the same stream at a track's start or after a seek (LMS sends the metadata again).
#[tokio::test]
async fn measure_time_to_first_audio_on_a_slow_link() {
    const RTT_MS: f64 = 80.0;
    const BYTES_PER_MS: f64 = 8_000_000.0 / 8.0 / 1000.0;
    let mut results = Vec::new();
    for strip in [false, true] {
        let mut h = harness_with(|c| c.strip_flac_pictures = strip).await;
        let file = library_flac(480 * 1024, 200);
        *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file));
        let (_ws, sid, _lms) = announce(&mut h, b'f').await;
        let got = fetch_all(&h, &sid).await;
        let needed = bytes_before_audio(&got, 16 * 1024, 500);
        let ms = RTT_MS + needed as f64 / BYTES_PER_MS;
        println!(
            "strip={strip}: metadata {} B, {} B received before 0.5 s decoded, ~{ms:.0} ms",
            audio_offset(&got),
            needed
        );
        results.push((needed, ms));
    }
    let ((before, before_ms), (after, after_ms)) = (results[0], results[1]);
    assert!(before > 480 * 1024, "the artwork had to arrive first");
    assert!(
        before - after >= 480 * 1024,
        "{before} → {after}: the artwork no longer arrives first"
    );
    assert!(
        after_ms < before_ms / 2.0,
        "{before_ms:.0} ms → {after_ms:.0} ms"
    );
}
