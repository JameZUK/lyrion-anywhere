//! Probing streams whose headers are large (embedded artwork) or never end.
//!
//! Regression: a failed probe doubled the bytes it waited for, which could exceed the
//! player's input limit, so the host stopped reading and the track stalled for ever.

use anywhere_core::decode::PROBE_MAX_BYTES;
use anywhere_core::player::{Action, Player, PlayerConfig};
use anywhere_core::proto::{ClientFrameReader, Strm, encode_server_frame};

fn cfg() -> PlayerConfig {
    PlayerConfig {
        mac: [2, 0, 0, 0, 0, 7],
        uuid: [0; 16],
        name: "Probe".into(),
        codecs: vec!["flc".into()],
        max_sample_rate: 192_000,
        firmware: "test".into(),
    }
}

fn strm_flac() -> Vec<u8> {
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
        request: b"GET /stream.mp3?player=02:00:00:00:00:07 HTTP/1.0\r\n\r\n".to_vec(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for b in data {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn block_header(last: bool, kind: u8, len: usize) -> Vec<u8> {
    assert!(len < 1 << 24);
    let mut v = vec![(u8::from(last) << 7) | kind];
    v.extend_from_slice(&(len as u32).to_be_bytes()[1..]);
    v
}

const BLOCK: u64 = 4096;

/// A FLAC file (44.1 kHz, 16-bit stereo) with a PICTURE block of
/// `picture` bytes before the audio.
fn flac_with_picture(picture: usize, frames: u64) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    // STREAMINFO
    v.extend(block_header(false, 0, 34));
    v.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    v.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    v.extend_from_slice(&[0; 6]); // frame sizes unknown
    let packed: u64 = (44_100u64 << 44) | (1 << 41) | (15 << 36) | (frames * BLOCK);
    v.extend_from_slice(&packed.to_be_bytes());
    v.extend_from_slice(&[0; 16]); // MD5 not computed
    // PICTURE
    let mime = b"image/jpeg";
    let mut pic = Vec::with_capacity(picture + 64);
    pic.extend_from_slice(&3u32.to_be_bytes()); // front cover
    pic.extend_from_slice(&(mime.len() as u32).to_be_bytes());
    pic.extend_from_slice(mime);
    pic.extend_from_slice(&0u32.to_be_bytes()); // no description
    for x in [1u32, 1, 24, 0] {
        pic.extend_from_slice(&x.to_be_bytes());
    }
    pic.extend_from_slice(&(picture as u32).to_be_bytes());
    pic.extend((0..picture).map(|i| (i % 251) as u8));
    v.extend(block_header(true, 6, pic.len()));
    v.extend(pic);
    // Audio frames, verbatim so the file is several MiB beyond the artwork.
    for n in 0..frames {
        assert!(n < 0x800, "frame numbers up to two UTF-8 bytes");
        let mut f = vec![0xFF, 0xF8, 0xC9, 0x18];
        if n < 0x80 {
            f.push(n as u8);
        } else {
            f.extend_from_slice(&[0xC0 | (n >> 6) as u8, 0x80 | (n & 0x3F) as u8]);
        }
        f.push(crc8(&f));
        for ch in 0..2i32 {
            f.push(0x02); // verbatim subframe
            for i in 0..BLOCK as i32 {
                let s = ((i * 37 + ch * 1000) % 2000 - 1000) as i16;
                f.extend_from_slice(&s.to_be_bytes());
            }
        }
        let c = crc16(&f);
        f.extend_from_slice(&c.to_be_bytes());
        v.extend(f);
    }
    v
}

fn stats(actions: &[Action]) -> Vec<String> {
    let mut r = ClientFrameReader::default();
    for a in actions {
        if let Action::Send(b) = a {
            r.push(b);
        }
    }
    let mut out = vec![];
    while let Some((op, p)) = r.next_frame().unwrap() {
        if &op == b"STAT" {
            out.push(String::from_utf8_lossy(&p[0..4]).into_owned());
        }
    }
    out
}

/// Feed `next_chunk` to the player the way a host does (only while it wants data), pulling
/// audio as it is decoded. Returns (frames played, STAT events) after `max_steps`.
fn run(mut next_chunk: impl FnMut() -> Option<Vec<u8>>, max_steps: usize) -> (u64, Vec<String>) {
    let mut p = Player::new(cfg());
    p.connected(0);
    p.on_control_data(&strm_flac(), 1).unwrap();
    let mut events = stats(&p.take_actions());
    let mut played = 0u64;
    let mut buf = vec![0f32; 8192];
    let mut ended = false;
    for step in 0..max_steps {
        let now = 2 + step as u64;
        if p.wants_stream_data() && !ended {
            match next_chunk() {
                Some(c) => p.on_stream_data(&c, now),
                None => {
                    ended = true;
                    p.on_stream_end(now);
                }
            }
        }
        p.tick(now);
        loop {
            let n = p.pull_audio(&mut buf);
            if n == 0 {
                break;
            }
            p.played(n as u64);
            played += n as u64;
        }
        events.extend(stats(&p.take_actions()));
        if events.iter().any(|e| e == "STMu" || e == "STMn") {
            break;
        }
    }
    (played, events)
}

#[test]
fn flac_with_six_mib_of_artwork_plays() {
    // ~16 MB in all: the stream doesn't end before the player's 8 MiB input limit.
    let frames = 600;
    let file = flac_with_picture(6 * 1024 * 1024, frames);
    assert!(file.len() > 13 * 1024 * 1024);
    // 96 KiB reads make a probe fail at exactly 6 MiB buffered, where doubling the wait
    // would have crossed the player's 8 MiB input limit.
    let mut chunks = file.chunks(96 * 1024).map(|c| c.to_vec());
    let (played, events) = run(|| chunks.next(), 10_000);
    assert!(
        !events.contains(&"STMn".to_string()),
        "track failed: {events:?}"
    );
    assert!(
        events.contains(&"STMd".to_string()),
        "never decoded: {events:?}"
    );
    assert_eq!(played, frames * BLOCK, "every frame decodes and plays");
}

#[test]
fn headers_that_never_end_fail_the_track_instead_of_stalling() {
    // A FLAC marker followed by an endless chain of near-16 MiB metadata blocks: the probe
    // can never complete, and the stream never ends.
    let mut pending: Vec<u8> = b"fLaC".to_vec();
    let mut fed = 0usize;
    let next = || {
        if pending.is_empty() {
            pending = block_header(false, 1, (1 << 24) - 1);
            pending.resize((1 << 24) + 3, 0);
        }
        let n = pending.len().min(256 * 1024);
        fed += n;
        Some(pending.drain(..n).collect())
    };
    let (_, events) = run(next, 10_000);
    assert!(
        events.contains(&"STMn".to_string()),
        "an unprobeable stream must fail cleanly, got {events:?}"
    );
    assert!(fed <= PROBE_MAX_BYTES + 512 * 1024, "read {fed} bytes");
}
