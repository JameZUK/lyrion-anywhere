//! Integer output for bit-perfect USB DACs: the decoded `f32` PCM converts to 16-, 24- or
//! 32-bit integers exactly, so the samples reach the DAC unchanged.
//!
//! - `f32` carries 24 significant bits, and every scale is a power of two, so any source of
//!   up to 24 bits survives decode → `f32` → integer. A deeper output container is lossless
//!   padding (the value shifted left).
//! - A 32-bit source does *not* survive `f32`; the engine must not claim bit-perfect for it
//!   (the negotiation in `mobile` refuses it), and this test pins that limit down.
//! - The real decoder is in the loop: WAV files of known integers are decoded by the core's
//!   streaming decoder (Symphonia) and converted back.

use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use anywhere_core::dsp::dither::{Dither, DitherMode};
use anywhere_core::dsp::pcm::{self, F32_EXACT_BITS, f32_on_grid, f32_to_int, int_to_f32};

const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Stereo `bits`-deep samples: random, with the extremes mixed in.
fn sequence(bits: u32, frames: usize, seed: u64) -> Vec<i32> {
    let mut r = Rng(seed);
    let lo = -(1i64 << (bits - 1));
    let hi = (1i64 << (bits - 1)) - 1;
    let specials = [lo, hi, 0, 1, -1, lo + 1, hi - 1];
    (0..frames * 2)
        .map(|i| {
            if i % 101 < 7 {
                specials[i % 101] as i32
            } else {
                (lo + (r.next() % (hi - lo + 1) as u64) as i64) as i32
            }
        })
        .collect()
}

#[test]
fn integer_sources_reach_every_container_exactly_at_every_rate() {
    for (bits, containers) in [(16u32, &[16u32, 24, 32][..]), (24, &[24, 32][..])] {
        for rate in RATES {
            let ints = sequence(bits, rate as usize / 10, u64::from(rate) * u64::from(bits));
            let mut f = vec![0f32; ints.len()];
            int_to_f32(&ints, bits, &mut f);
            for &c in containers {
                let shift = c - bits;
                for (i, x) in ints.iter().zip(&f) {
                    assert!(f32_on_grid(*x, bits));
                    let want = i64::from(*i) << shift;
                    assert_eq!(
                        i64::from(f32_to_int(*x, c)),
                        want,
                        "{bits}-bit in {c}-bit at {rate} Hz"
                    );
                }
            }
        }
    }
}

#[test]
fn thirty_two_bit_sources_do_not_survive_f32() {
    assert_eq!(F32_EXACT_BITS, 24);
    // Values with only 24 significant bits survive in a 32-bit container...
    for v in [i32::MIN, 0x7fff_ff00, -256, 0x0012_3400 << 4] {
        let x = (f64::from(v) / 2_147_483_648.0) as f32;
        assert_eq!(f32_to_int(x, 32), v, "{v:#x}");
    }
    // ...but full 32-bit values don't: why a 32-bit source is never claimed bit-perfect.
    let mut lost = 0;
    let mut r = Rng(5);
    for _ in 0..10_000 {
        let v = (r.next() >> 32) as u32 as i32 | 0x0100_0001;
        let x = (f64::from(v) / 2_147_483_648.0) as f32;
        if f32_to_int(x, 32) != v {
            lost += 1;
        }
    }
    assert!(lost > 9_000, "{lost}");
}

#[test]
fn conversion_clamps_and_survives_hostile_samples() {
    for bits in [16, 24, 32] {
        let (lo, hi) = pcm::int_range(bits);
        assert_eq!(f32_to_int(1.0, bits), hi as i32);
        assert_eq!(f32_to_int(-1.0, bits), lo as i32);
        assert_eq!(f32_to_int(f32::NAN, bits), 0);
        assert_eq!(f32_to_int(f32::INFINITY, bits), hi as i32);
        assert_eq!(f32_to_int(f32::NEG_INFINITY, bits), lo as i32);
        assert_eq!(f32_to_int(1e30, bits), hi as i32);
        assert!(!f32_on_grid(f32::NAN, bits));
        assert!(!f32_on_grid(2.0, bits));
    }
    // Off the grid: rounded to nearest.
    assert_eq!(f32_to_int(1.4 / 32_768.0, 16), 1);
    assert_eq!(f32_to_int(1.6 / 32_768.0, 16), 2);
    assert!(!f32_on_grid(0.5 / 32_768.0, 16));
}

/// Processed audio is dithered onto the 24-bit grid; carried as `f32`, it reaches a 24- or
/// 32-bit output exactly as dithered.
#[test]
fn dithered_audio_survives_the_f32_ring_exactly() {
    let mut d = Dither::new(DitherMode::TpdfShaped, 2);
    let mut r = Rng(9);
    let src: Vec<f64> = (0..20_000)
        .map(|_| (r.next() as i64) as f64 / i64::MAX as f64 * 0.9)
        .collect();
    let mut q = vec![0i32; src.len()];
    d.quantise(&src, 24, &mut q);
    let mut f = vec![0f32; q.len()];
    int_to_f32(&q, 24, &mut f);
    for (i, x) in q.iter().zip(&f) {
        assert_eq!(f32_to_int(*x, 24), *i);
        assert_eq!(f32_to_int(*x, 32), i << 8);
    }
}

/// A PCM WAV file of `bits`-deep stereo samples.
fn wav(rate: u32, bits: u32, samples: &[i32]) -> Vec<u8> {
    let bytes = (bits / 8) as usize;
    let data_len = samples.len() * bytes;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2 * bytes as u32).to_le_bytes());
    v.extend_from_slice(&(2 * bytes as u16).to_le_bytes());
    v.extend_from_slice(&(bits as u16).to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes()[..bytes]);
    }
    v
}

/// The real decoder in the loop: 16- and 24-bit files decode to `f32` that converts back
/// to the very integers in the file, at every rate, into every container at least as deep.
#[test]
fn decoded_files_convert_back_to_their_integers() {
    for bits in [16u32, 24] {
        for rate in RATES {
            let ints = sequence(bits, rate as usize / 20, u64::from(rate) + 3);
            let input = InputBuffer::new();
            input.push(&wav(rate, bits, &ints));
            input.set_eof();
            let mut dec = StreamDecoder::new(input, b'p');
            let mut got = Vec::new();
            loop {
                match dec.work(64, &mut |r, s| {
                    assert_eq!(r, rate);
                    got.extend_from_slice(s);
                }) {
                    DecodeStatus::Progress => {}
                    DecodeStatus::Finished => break,
                    other => panic!("{other:?}"),
                }
            }
            assert_eq!(dec.source_format().unwrap().bits, Some(bits));
            assert_eq!(got.len(), ints.len());
            for c in [16u32, 24, 32].into_iter().filter(|c| *c >= bits) {
                for (k, (x, i)) in got.iter().zip(&ints).enumerate() {
                    assert_eq!(
                        i64::from(f32_to_int(*x, c)),
                        i64::from(*i) << (c - bits),
                        "{bits}-bit file in {c}-bit at {rate} Hz, sample {k}"
                    );
                }
            }
        }
    }
}
