//! Ogg Opus decoding (the `opus` feature).
//!
//! Fixture `fixtures/sine-997hz-stereo.opus`: one second of a synthetic 997 Hz stereo sine
//! at -20 dBFS peak, made by us with FFmpeg's libopus encoder (so no third-party content):
//!
//! ```text
//! ffmpeg -f lavfi -i "aevalsrc=0.1*sin(2*PI*997*t)|0.1*sin(2*PI*997*t):s=48000:d=1" \
//!   -c:a libopus -b:a 32k -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
//!   sine-997hz-stereo.opus
//! ```
#![cfg(feature = "opus")]

use anywhere_core::analysis::{AnalysisConfig, analyse_encoded};
use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder, format_extension};

const FIXTURE: &[u8] = include_bytes!("fixtures/sine-997hz-stereo.opus");

fn decode(bytes: &[u8], format: u8) -> (DecodeStatus, u32, Vec<f32>) {
    let input = InputBuffer::new();
    input.push(bytes);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, format);
    let mut rate = 0;
    let mut pcm = Vec::new();
    loop {
        let st = dec.work(16, &mut |r, s| {
            rate = r;
            pcm.extend_from_slice(s);
        });
        if st != DecodeStatus::Progress {
            return (st, rate, pcm);
        }
    }
}

#[test]
fn decodes_ogg_opus_sample_accurately() {
    assert_eq!(format_extension(b'u'), Some("opus"));
    for format in *b"uo" {
        let (st, rate, pcm) = decode(FIXTURE, format);
        assert_eq!(st, DecodeStatus::Finished);
        assert_eq!(rate, 48_000);
        // Pre-skip and end trimming give back exactly the encoded second.
        assert_eq!(pcm.len(), 48_000 * 2, "frames: {}", pcm.len() / 2);
        let peak = pcm.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.1).abs() < 0.01, "peak {peak}");
        // Aligned with the source: fit the sine's phase over the middle of the second.
        let w = 2.0 * std::f64::consts::PI * 997.0 / 48_000.0;
        let (mut s, mut c) = (0.0, 0.0);
        for i in 8_000..40_000 {
            let x = f64::from(pcm[i * 2]);
            s += x * (w * i as f64).sin();
            c += x * (w * i as f64).cos();
        }
        let lag = -c.atan2(s) / w;
        assert!(lag.abs() < 1.0, "decoded audio is {lag} samples off");
    }
}

#[test]
fn opus_loudness_matches_the_source() {
    let a = analyse_encoded(FIXTURE, b'u', AnalysisConfig::default()).unwrap();
    assert_eq!(a.sample_rate, 48_000);
    assert_eq!(a.frames, 48_000);
    // A -20 dBFS 997 Hz stereo sine is -20 LUFS; Opus at 32 kb/s keeps it within 0.2 LU.
    let i = a.integrated_lufs.unwrap();
    assert!((i + 20.0).abs() < 0.2, "{i}");
}

/// Small deterministic PRNG.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

#[test]
fn corrupt_opus_never_panics() {
    let mut r = Rng(0x0b05);
    let iters = if cfg!(debug_assertions) { 150 } else { 1500 };
    for i in 0..iters {
        let mut b = FIXTURE.to_vec();
        match i % 3 {
            // Flip bits anywhere (headers, page CRCs, packets).
            0 => {
                for _ in 0..1 + r.next() % 16 {
                    let k = (r.next() % b.len() as u64) as usize;
                    b[k] ^= 1 << (r.next() % 8);
                }
            }
            // Truncate.
            1 => b.truncate((r.next() % b.len() as u64) as usize),
            // Overwrite a run with random bytes, past the headers.
            _ => {
                let start = 200 + (r.next() % (b.len() as u64 - 400)) as usize;
                for byte in &mut b[start..start + 64] {
                    *byte = r.next() as u8;
                }
            }
        }
        let (st, _, pcm) = decode(&b, b'u');
        assert!(matches!(
            st,
            DecodeStatus::Finished | DecodeStatus::Failed(_)
        ));
        assert!(pcm.iter().all(|s| s.is_finite()));
    }
}

/// Regression: `opus-decoder` 0.1.1 overflowed building a collapse mask for bands with more
/// than 8 short blocks: a panic in debug builds, wrongly decoded transients in release
/// builds (fixed in `third_party/opus-decoder`, see its `PATCHES.md`).
///
/// Fixture `fixtures/noise-96k.opus`: the last second and a half of what the gateway's live
/// encoder (FFmpeg's libopus, 96 kb/s) made of its own synthetic test signal (pseudo-random
/// FLAC frames, `gateway/tests/live.rs`), ending where the signal stops. Cut without
/// re-encoding, so no third-party content:
///
/// ```text
/// ffmpeg -ss 9 -i whole.opus -c copy -map_metadata -1 -fflags +bitexact noise-96k.opus
/// ```
#[test]
fn a_transient_with_many_short_blocks_decodes() {
    let (st, rate, pcm) = decode(include_bytes!("fixtures/noise-96k.opus"), b'u');
    assert_eq!(st, DecodeStatus::Finished);
    assert_eq!(rate, 48_000);
    assert!(pcm.len() > 48_000 * 2, "{} samples", pcm.len());
    assert!(pcm.iter().all(|s| s.is_finite() && s.abs() <= 2.0));
}
