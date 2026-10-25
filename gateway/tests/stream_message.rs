//! The stream message (protocol 2, docs/plans/signal-path-redesign.md): for every stream the
//! gateway relays, one text message on the player socket saying what LMS served and what the
//! gateway sent. It is never a frame (not counted in `received`) and comes again after a
//! resume greeting.
//!
//! Ordering: the gateway reads a FLAC stream's first 42 bytes and queues the message on the
//! player socket before it answers the stream request, so the message is on its way before
//! any of the body is. The tests check it is there to be read once the response headers
//! are in, before a byte of the body is read (two connections give no stronger order).

mod common;
use common::flac::*;
use common::*;

use serde_json::{Value, json};

/// A 24-bit, 96 kHz stereo FLAC file (the test writer's file with its STREAMINFO changed).
fn flac_24_96(frames: usize) -> Vec<u8> {
    let mut f = library_flac(0, frames);
    let packed: u64 = (96_000u64 << 44) | (1 << 41) | (23 << 36) | (frames * BLOCK) as u64;
    f[18..26].copy_from_slice(&packed.to_be_bytes());
    f
}

/// A FLAC stream without its header, as LMS sends after a seek: frames straight away.
fn headerless(len: usize) -> Vec<u8> {
    let mut v = vec![0xff, 0xf8, 0x69, 0x08, 0x00, 0x00];
    let mut x = 7u32;
    while v.len() < len {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        v.push((x >> 16) as u8);
    }
    v
}

/// A `strm s` with every field the message reads: format, PCM size, rate and channels
/// codes, and the server's ReplayGain (16.16, 0 for none).
fn strm_s(mac: [u8; 6], format: u8, pcm: [u8; 3], gain: u32) -> Vec<u8> {
    let mut p = vec![0u8; 24];
    p[0] = b's';
    p[1] = b'1';
    p[2] = format;
    p[3..6].copy_from_slice(&pcm);
    p[6] = b'1';
    p[14..18].copy_from_slice(&gain.to_be_bytes());
    p[18..20].copy_from_slice(&9000u16.to_be_bytes());
    p.extend_from_slice(stream_req(mac).as_bytes());
    encode_server_frame(*b"strm", &p)
}

const UNKNOWN: [u8; 3] = *b"???";

struct Player {
    ws: Ws,
    sid: String,
    secret: String,
    lms: OwnedWriteHalf,
    /// Binary messages (frames) received.
    frames: u64,
}

async fn greeting(ws: &mut Ws) -> Value {
    match next(ws).await {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("expected greeting, got {other:?}"),
    }
}

async fn open(h: &mut Harness) -> Player {
    let mut ws = h.dial().await;
    ws.send(Message::Text(sign_in_form(&h.tok_a).into()))
        .await
        .unwrap();
    let g = greeting(&mut ws).await;
    let lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    Player {
        ws,
        sid: g["session"].as_str().unwrap().to_string(),
        secret: g["secret"].as_str().unwrap().to_string(),
        lms,
        frames: 0,
    }
}

impl Player {
    /// LMS sends `frame`; the player receives it (and nothing else before it).
    async fn lms_sends(&mut self, frame: &[u8]) {
        self.lms.write_all(frame).await.unwrap();
        match next(&mut self.ws).await {
            Message::Binary(b) => assert_eq!(&b[..], frame),
            other => panic!("expected a frame, got {other:?}"),
        }
        self.frames += 1;
    }
}

/// Fetch stream `n`: the stream message must be ready on the socket once the response
/// headers are in, before any body is read. Returns the message and the body.
async fn fetch(h: &Harness, p: &mut Player, n: u64) -> (Value, Vec<u8>) {
    let mut resp = h.get_stream(&p.sid, n, Some(&h.tok_a)).await;
    assert_eq!(resp.status(), 200);
    let m = match next(&mut p.ws).await {
        m @ Message::Text(_) => stream_message(&m).unwrap_or_else(|| panic!("{m:?}")),
        other => panic!("expected the stream message first, got {other:?}"),
    };
    let mut body = Vec::new();
    while let Some(c) = resp.chunk().await.unwrap() {
        body.extend_from_slice(&c);
    }
    (m, body)
}

/// Nothing more on the socket for a moment.
async fn quiet(ws: &mut Ws) {
    if let Ok(m) = tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
        panic!("expected nothing, got {m:?}");
    }
}

#[tokio::test]
async fn a_passed_through_flac_says_its_streaminfo() {
    let mut h = harness().await;
    let file = flac_24_96(20);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(file.clone()));
    let mut p = open(&mut h).await;
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    let (m, body) = fetch(&h, &mut p, 1).await;
    assert_eq!(
        m,
        json!({"seq": 1,
            "served": {"codec": "flac", "rate": 96000, "bits": 24, "channels": 2},
            "sent": {"mode": "passthrough"}})
    );
    assert!(m["served"].get("gain_db").is_none(), "no server gain");
    // The header that was read went on: the client has the stream from its start.
    assert_eq!(&body[..42], &file[..42]);
    // One message per stream.
    quiet(&mut p.ws).await;
}

#[tokio::test]
async fn the_server_gain_is_in_db_when_lms_set_one() {
    let mut h = harness().await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(flac_24_96(5)));
    let mut p = open(&mut h).await;
    // 0.6918 of unity (16.16) is -3.2 dB.
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 45_338)).await;
    let (m, _) = fetch(&h, &mut p, 1).await;
    assert_eq!(m["served"]["gain_db"], json!(-3.2));
    assert_eq!(m["served"]["rate"], 96000);
    // Unity and above.
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 2 * 65_536))
        .await;
    let (m, _) = fetch(&h, &mut p, 2).await;
    assert_eq!(m["served"]["gain_db"], json!(6.0));
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    let (m, _) = fetch(&h, &mut p, 3).await;
    assert!(m["served"].get("gain_db").is_none(), "{m}");
}

#[tokio::test]
async fn a_pcm_stream_takes_its_format_from_the_strm() {
    let mut h = harness().await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(vec![0u8; 4096]));
    let mut p = open(&mut h).await;
    p.lms_sends(&strm_s(h.mac_a, b'p', *b"292", 0)).await;
    let (m, body) = fetch(&h, &mut p, 1).await;
    assert_eq!(
        m,
        json!({"seq": 1,
            "served": {"codec": "pcm", "rate": 96000, "bits": 24, "channels": 2},
            "sent": {"mode": "passthrough"}})
    );
    assert_eq!(body, vec![0u8; 4096]);
    // 16-bit 44.1 kHz mono; unknown codes are null.
    p.lms_sends(&strm_s(h.mac_a, b'p', *b"131", 0)).await;
    let (m, _) = fetch(&h, &mut p, 2).await;
    assert_eq!(
        m["served"],
        json!({"codec": "pcm", "rate": 44100, "bits": 16, "channels": 1})
    );
    p.lms_sends(&strm_s(h.mac_a, b'p', *b"?Z9", 0)).await;
    let (m, _) = fetch(&h, &mut p, 3).await;
    assert_eq!(
        m["served"],
        json!({"codec": "pcm", "rate": null, "bits": null, "channels": null})
    );
}

/// An MP3 is sent as it is, its format unknown to the gateway; its bytes are not read.
#[tokio::test]
async fn an_mp3_is_sent_as_it_is() {
    let mut h = harness().await;
    let mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x00\xff\xfb\x90\x64".repeat(100);
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(mp3.clone()));
    let mut p = open(&mut h).await;
    p.lms_sends(&strm_s(h.mac_a, b'm', *b"131", 45_338)).await;
    let (m, body) = fetch(&h, &mut p, 1).await;
    assert_eq!(
        m,
        json!({"seq": 1,
            "served": {"codec": "mp3", "rate": null, "bits": null, "channels": null,
                "gain_db": -3.2},
            "sent": {"mode": "passthrough"}})
    );
    assert_eq!(body, mp3);
    // The other codes.
    for (n, (format, codec)) in [
        (b'o', "ogg"),
        (b'a', "aac"),
        (b'l', "alac"),
        (b'u', "unknown"),
        (b'?', "unknown"),
    ]
    .into_iter()
    .enumerate()
    {
        p.lms_sends(&strm_s(h.mac_a, format, UNKNOWN, 0)).await;
        let (m, _) = fetch(&h, &mut p, n as u64 + 2).await;
        assert_eq!(m["served"]["codec"], codec);
        assert_eq!(m["sent"], json!({"mode": "passthrough"}));
    }
}

/// A FLAC stream without a header (a seek, 0.6.3) leaves the format null and reaches the
/// client byte for byte, never held up.
#[tokio::test]
async fn a_headerless_flac_is_relayed_untouched() {
    let mut h = harness().await;
    for len in [10, 41, 42, 43, 70_000] {
        let body = headerless(len);
        *h.lms.stream_body.lock().unwrap() = Some(Arc::new(body.clone()));
        let mut p = open(&mut h).await;
        p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
        let (m, got) = fetch(&h, &mut p, 1).await;
        assert_eq!(
            m,
            json!({"seq": 1,
                "served": {"codec": "flac", "rate": null, "bits": null, "channels": null},
                "sent": {"mode": "passthrough"}}),
            "{len}"
        );
        assert_eq!(got, body, "{len} bytes relayed exactly");
    }
    // Shorter than a header, or empty: the same.
    for body in [Vec::new(), b"fLaC".to_vec()] {
        *h.lms.stream_body.lock().unwrap() = Some(Arc::new(body.clone()));
        let mut p = open(&mut h).await;
        p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
        let (m, got) = fetch(&h, &mut p, 1).await;
        assert_eq!(m["served"]["rate"], Value::Null);
        assert_eq!(got, body);
    }
}

/// `seq` counts the `strm s` frames of the session: a seek and a track change each make a
/// new stream. A stream never fetched gets no message.
#[tokio::test]
async fn seq_follows_each_seek_and_track_change() {
    let mut h = harness().await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(flac_24_96(5)));
    let mut p = open(&mut h).await;
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    assert_eq!(fetch(&h, &mut p, 1).await.0["seq"], 1);
    // A seek: LMS stops the stream and starts it again from elsewhere (headerless).
    let mut q = vec![0u8; 24];
    q[0] = b'q';
    q[2] = b'f';
    p.lms_sends(&encode_server_frame(*b"strm", &q)).await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(headerless(5000)));
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    let (m, _) = fetch(&h, &mut p, 2).await;
    assert_eq!(m["seq"], 2);
    assert_eq!(m["served"]["rate"], Value::Null);
    // The next track, an MP3; one announced and skipped before it was fetched.
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    p.lms_sends(&strm_s(h.mac_a, b'm', UNKNOWN, 0)).await;
    quiet(&mut p.ws).await;
    let (m, _) = fetch(&h, &mut p, 4).await;
    assert_eq!(
        (m["seq"].clone(), m["served"]["codec"].clone()),
        (json!(4), json!("mp3"))
    );
    // An earlier stream can't be fetched any more, so it says nothing.
    assert_eq!(h.get_stream(&p.sid, 3, Some(&h.tok_a)).await.status(), 404);
    quiet(&mut p.ws).await;
}

fn resume(token: &str, p: &Player, received: u64) -> Message {
    Message::Text(
        sign_in_form_with(
            token,
            json!({"resume": {"session": p.sid, "secret": p.secret, "received": received}}),
        )
        .into(),
    )
}

/// After a resume greeting, the message for the stream in progress comes again, before
/// anything else; it was never a frame, so the client's count of frames is unchanged by it.
#[tokio::test]
async fn a_resume_gets_the_message_again_and_it_is_never_a_frame() {
    let mut h = harness().await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(flac_24_96(5)));
    let mut p = open(&mut h).await;
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    let (first, _) = fetch(&h, &mut p, 2).await;
    assert_eq!(first["seq"], 2);
    // Counting the message as a frame would claim one LMS never sent.
    let frames = p.frames;
    assert_eq!(frames, 2);
    let mut old = std::mem::replace(&mut p.ws, h.dial().await);
    old.close(None).await.ok();
    p.ws.send(resume(&h.tok_a, &p, frames + 1)).await.unwrap();
    expect_close(&mut p.ws, CloseCode::Library(4404)).await;
    // The right count: the greeting, then the stream message again, then nothing (no
    // frames were missed).
    p.ws = h.dial().await;
    p.ws.send(resume(&h.tok_a, &p, frames)).await.unwrap();
    let g = greeting(&mut p.ws).await;
    assert_eq!(g["resumed"], true);
    let again = stream_message(&next(&mut p.ws).await).expect("the stream message");
    assert_eq!(again, first);
    quiet(&mut p.ws).await;
    // Frames still flow, counted as before; a newer stream gets its own message.
    p.lms_sends(&strm_s(h.mac_a, b'm', UNKNOWN, 0)).await;
    assert_eq!(p.frames, 3);
    let (m, _) = fetch(&h, &mut p, 3).await;
    assert_eq!(m["seq"], 3);
    // Resumed again, the newest is the one sent.
    p.ws = h.dial().await;
    p.ws.send(resume(&h.tok_a, &p, p.frames)).await.unwrap();
    greeting(&mut p.ws).await;
    assert_eq!(
        stream_message(&next(&mut p.ws).await).expect("the stream message")["seq"],
        3
    );
}

/// A resumed session with no stream relayed yet: nothing after the greeting. A stream
/// fetched while the client was away is told on its return.
#[tokio::test]
async fn a_message_while_away_waits_for_the_resume() {
    let mut h = harness().await;
    *h.lms.stream_body.lock().unwrap() = Some(Arc::new(flac_24_96(5)));
    let mut p = open(&mut h).await;
    p.ws.close(None).await.ok();
    p.ws = h.dial().await;
    p.ws.send(resume(&h.tok_a, &p, 0)).await.unwrap();
    greeting(&mut p.ws).await;
    quiet(&mut p.ws).await;
    p.lms_sends(&strm_s(h.mac_a, b'f', UNKNOWN, 0)).await;
    // The socket drops; the download goes on meanwhile.
    p.ws.close(None).await.ok();
    while tokio::time::timeout(Duration::from_secs(1), p.ws.next())
        .await
        .is_ok_and(|m| m.is_some())
    {}
    let mut resp = h.get_stream(&p.sid, 1, Some(&h.tok_a)).await;
    assert_eq!(resp.status(), 200);
    while resp.chunk().await.unwrap().is_some() {}
    p.ws = h.dial().await;
    p.ws.send(resume(&h.tok_a, &p, p.frames)).await.unwrap();
    greeting(&mut p.ws).await;
    let m = stream_message(&next(&mut p.ws).await).expect("the stream message");
    assert_eq!(m["seq"], 1);
    assert_eq!(m["served"]["bits"], 24);
}
