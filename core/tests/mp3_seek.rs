//! Seeking in an MP3: LMS restarts the stream at a byte offset in the middle of the file,
//! so the new stream has no ID3 tag, no Xing header, and starts part-way through a frame.
//! The decoder must find the next frame and play from there, not stall or fail the track.
//!
//! Fixtures: an 8 s 440 Hz tone at -18 dBFS (generated with ffmpeg's sine source, 44.1 kHz
//! stereo), as 192 kbps CBR and as VBR.

use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use anywhere_core::player::{Action, OutputState, Player, PlayerConfig};
use anywhere_core::proto::{ClientFrameReader, Strm, encode_server_frame};

const CBR: &[u8] = include_bytes!("fixtures/sine-440hz-192k-cbr.mp3");
const VBR: &[u8] = include_bytes!("fixtures/sine-440hz-vbr.mp3");
/// Samples per channel in an MPEG-1 Layer III frame.
const MP3_FRAME: usize = 1152;
const RATE: usize = 44_100;
const SECONDS: usize = 8;

/// Decode `bytes` as a stream that ends, returning (rate, stereo frames, final status).
fn decode(bytes: &[u8], chunk: usize) -> (u32, usize, DecodeStatus) {
    let input = InputBuffer::new();
    let mut dec = StreamDecoder::new(input.clone(), b'm');
    let (mut rate, mut frames) = (0u32, 0usize);
    let mut sink = |r: u32, s: &[f32]| {
        rate = r;
        frames += s.len() / 2;
    };
    for c in bytes.chunks(chunk) {
        input.push(c);
        let st = dec.work(64, &mut sink);
        assert!(
            !matches!(st, DecodeStatus::Failed(_)),
            "failed mid-stream: {st:?}"
        );
    }
    input.set_eof();
    loop {
        let st = dec.work(64, &mut sink);
        if st != DecodeStatus::Progress {
            return (rate, frames, st);
        }
    }
}

/// Stereo frames a whole file decodes to (the reference for the cut streams).
fn whole(file: &[u8]) -> usize {
    let (rate, frames, st) = decode(file, 16 * 1024);
    assert_eq!(st, DecodeStatus::Finished);
    assert_eq!(rate, RATE as u32);
    assert!(
        frames.abs_diff(RATE * SECONDS) < 3 * MP3_FRAME,
        "whole file: {frames}"
    );
    frames
}

/// From any byte offset, the stream decodes to the end, losing at most a few frames at the
/// cut (the partial frame, and one the decoder may need to prime its bit reservoir).
fn check_cuts(file: &[u8], name: &str) {
    let total = whole(file);
    let bytes_per_frame = file.len() as f64 / (total as f64 / MP3_FRAME as f64);
    // Odd offsets, none frame-aligned, across the file.
    for offset in [
        1usize,
        417,
        5_003,
        12_345,
        33_333,
        file.len() / 2 + 7,
        file.len() * 3 / 4 + 101,
    ] {
        let cut = &file[offset..];
        let (rate, frames, st) = decode(cut, 7_919);
        assert_eq!(st, DecodeStatus::Finished, "{name} at {offset}: {st:?}");
        assert_eq!(rate, RATE as u32, "{name} at {offset}");
        let expected = ((cut.len() as f64 / bytes_per_frame) * MP3_FRAME as f64) as usize;
        assert!(
            frames + 4 * MP3_FRAME >= expected,
            "{name} at {offset}: {frames} frames decoded, about {expected} expected"
        );
        // A cut stream has no Xing/Info header, so the encoder delay and padding aren't
        // trimmed: up to about two frames more than the whole file, never more.
        assert!(
            frames <= total + 2 * MP3_FRAME,
            "{name} at {offset}: {frames} frames, more than the whole file ({total})"
        );
    }
}

#[test]
fn cbr_mp3_decodes_from_any_byte_offset() {
    check_cuts(CBR, "CBR");
}

#[test]
fn vbr_mp3_decodes_from_any_byte_offset() {
    check_cuts(VBR, "VBR");
}

/// Audio starts arriving well before the end of the stream: the decoder doesn't wait for the
/// whole download (only for its fixed read-ahead margin).
#[test]
fn mid_file_mp3_produces_audio_before_the_stream_ends() {
    let cut = &CBR[20_001..];
    let input = InputBuffer::new();
    let mut dec = StreamDecoder::new(input.clone(), b'm');
    let mut frames = 0usize;
    let mut fed = 0usize;
    for c in cut.chunks(4096) {
        input.push(c);
        fed += c.len();
        dec.work(64, &mut |_, s| frames += s.len() / 2);
        if frames > 0 {
            break;
        }
    }
    assert!(frames > 0, "no audio before the end of the stream");
    assert!(
        fed <= anywhere_core::decode::DECODE_MARGIN + 64 * 1024,
        "first audio only after {fed} bytes"
    );
}

fn cfg() -> PlayerConfig {
    PlayerConfig {
        mac: [2, 0, 0, 0, 0, 9],
        uuid: [0; 16],
        name: "Seek".into(),
        codecs: vec!["mp3".into()],
        max_sample_rate: 192_000,
        firmware: "test".into(),
    }
}

fn strm(command: u8, kib: u8) -> Vec<u8> {
    let s = Strm {
        command,
        autostart: b'1',
        format: b'm',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: kib,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        replay_gain: 0,
        server_port: 9000,
        server_ip: 0,
        request: b"GET /stream.mp3?player=02:00:00:00:00:09 HTTP/1.0\r\n\r\n".to_vec(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
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

/// The whole seek as LMS does it: flush, then a new stream starting mid-file. The player
/// plays again (no STMn, no stall), and with quick start it does so before LMS's 255 KiB
/// threshold has arrived.
#[test]
fn seek_into_an_mp3_plays_from_the_new_offset() {
    for quick in [None, Some(500)] {
        let mut p = Player::new(cfg());
        p.set_quick_start(quick);
        p.connected(0);
        p.on_control_data(&strm(b's', 1), 1).unwrap();
        p.on_stream_data(CBR, 2);
        for t in 3..60 {
            p.tick(t);
        }
        assert_eq!(p.status().output, OutputState::Playing);
        p.take_actions();

        // Seek: flush, then a stream from an unaligned offset, arriving fast. (The tone is
        // repeated to make a stream longer than LMS's threshold and the decoder's margin.)
        p.on_control_data(&strm(b'q', 0), 100).unwrap();
        p.on_control_data(&strm(b's', 255), 100).unwrap();
        p.on_stream_headers(b"HTTP/1.0 200 OK\r\n\r\n", 100);
        let long: Vec<u8> = [&CBR[70_001..], &CBR[20..], &CBR[20..]].concat();
        let mut now = 101;
        let mut fed = 0;
        for c in long.chunks(16 * 1024) {
            fed += c.len();
            p.on_stream_data(c, now);
            for _ in 0..4 {
                p.tick(now);
                now += 1;
            }
            if p.status().output == OutputState::Playing {
                break;
            }
        }
        let events = stats(&p.take_actions());
        assert!(
            !events.contains(&"STMn".to_string()),
            "track failed: {events:?}"
        );
        assert_eq!(
            p.status().output,
            OutputState::Playing,
            "quick start {quick:?}, {fed} bytes"
        );
        if quick.is_some() {
            assert!(fed < 255 * 1024, "quick start: playing after {fed} bytes");
        }
        // Within a few frames: the first after a mid-file sync can be silent (the bit
        // reservoir it refers to was cut off), the tone follows at once.
        let mut buf = vec![0f32; 3 * MP3_FRAME * 2];
        let n = p.pull_audio(&mut buf);
        assert!(n > 2 * MP3_FRAME, "audio after the seek");
        assert!(
            buf[..n * 2].iter().any(|s| s.abs() > 0.05),
            "the tone, not silence, within three frames"
        );
    }
}
