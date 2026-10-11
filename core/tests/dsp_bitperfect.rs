//! Bit-perfect harness: with every stage bypassed, integer PCM converted into the engine's
//! `f64` format, run through the chain and converted back must come out identical, at
//! every common sample rate and depth, for long pseudo-random sequences that include the
//! full-scale extremes.

use anywhere_core::dsp::chain::{Chain, ChainConfig};
use anywhere_core::dsp::eq::parse_autoeq;
use anywhere_core::dsp::pcm;
use anywhere_core::dsp::signal_path::{OutputInfo, SignalPath, SourceInfo, Verdict};

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

/// `frames` stereo frames of `bits`-deep samples: random values, with runs of the extremes
/// (most negative, most positive, ±1 LSB, zero) mixed in.
fn sequence(bits: u32, frames: usize, seed: u64) -> Vec<i32> {
    let mut r = Rng(seed);
    let lo = -(1i64 << (bits - 1));
    let hi = (1i64 << (bits - 1)) - 1;
    let specials = [lo, hi, 0, 1, -1, lo + 1, hi - 1];
    (0..frames * 2)
        .map(|i| {
            let v = if i % 97 < 7 {
                specials[i % 97]
            } else {
                let span = (hi - lo + 1) as u64;
                lo + (r.next() % span) as i64
            };
            v as i32
        })
        .collect()
}

/// Run integer PCM through a chain in uneven blocks, back to integers at `out_bits`.
fn through_chain(chain: &mut Chain, input: &[i32], in_bits: u32, out_bits: u32) -> Vec<i32> {
    let mut out = Vec::with_capacity(input.len());
    let mut f = vec![0.0f64; 4096 * 2];
    let mut q = vec![0i32; 4096 * 2];
    let mut pos = 0;
    let mut step = 1;
    while pos < input.len() {
        // Block sizes vary from 1 frame to 4096 frames.
        step = (step * 7 + 3) % 4096 + 1;
        let n = (step * 2).min(input.len() - pos);
        pcm::to_f64(&input[pos..pos + n], in_bits, &mut f[..n]);
        chain.process(&mut f[..n]);
        chain.quantise(&f[..n], Some(in_bits), out_bits, &mut q[..n]);
        out.extend_from_slice(&q[..n]);
        pos += n;
    }
    out
}

#[test]
fn bypassed_chain_is_bit_perfect_at_every_rate_and_depth() {
    for (ri, rate) in RATES.into_iter().enumerate() {
        for bits in [16, 24, 32] {
            // Two seconds of stereo per case.
            let input = sequence(
                bits,
                rate as usize * 2,
                0x5eed + ri as u64 * 31 + u64::from(bits),
            );
            let mut chain = Chain::new(ChainConfig::default(), rate, 2).unwrap();
            assert!(chain.is_bit_transparent());
            let out = through_chain(&mut chain, &input, bits, bits);
            assert!(out == input, "{bits}-bit at {rate} Hz is not bit-perfect");
        }
    }
}

#[test]
fn neutral_settings_stay_bit_perfect() {
    // A flat EQ, 0 dB levelling after a change back, and preamp + levelling cancelling to
    // 0 dB are all true bypasses.
    let flat = parse_autoeq("Preamp: 0 dB\nFilter 1: ON PK Fc 1000 Hz Gain 0 dB Q 1\n").unwrap();
    let cfg = ChainConfig {
        eq: Some(flat),
        preamp_db: -3.0,
        levelling_db: 3.0,
        ..Default::default()
    };
    let mut chain = Chain::new(cfg, 96_000, 2).unwrap();
    assert!(chain.is_bit_transparent());
    chain.set_levelling_db(-2.0);
    assert!(!chain.is_bit_transparent());
    chain.set_levelling_db(3.0);
    let input = sequence(24, 96_000, 7);
    assert!(through_chain(&mut chain, &input, 24, 24) == input);
}

#[test]
fn widening_and_float_output_are_exact() {
    let input = sequence(16, 48_000, 9);
    let mut chain = Chain::new(ChainConfig::default(), 48_000, 2).unwrap();
    // 16-bit into a 24-bit output: the same values, shifted left by 8, never dithered.
    let wide = through_chain(&mut chain, &input, 16, 24);
    assert!(wide.iter().zip(&input).all(|(w, i)| *w == i << 8));
    // 16- and 24-bit through f32 output and back: exact (f32 has a 24-bit significand).
    for bits in [16, 24] {
        let input = sequence(bits, 48_000, 11);
        let mut f = vec![0.0; input.len()];
        let mut f32s = vec![0.0f32; input.len()];
        let mut back64 = vec![0.0; input.len()];
        let mut back = vec![0; input.len()];
        pcm::to_f64(&input, bits, &mut f);
        chain.process(&mut f);
        pcm::f64_to_f32(&f, &mut f32s);
        pcm::f32_to_f64(&f32s, &mut back64);
        pcm::from_f64(&back64, bits, &mut back);
        assert!(back == input, "{bits}-bit via f32");
    }
}

#[test]
fn processing_marks_the_path_and_dithers() {
    let cfg = ChainConfig {
        levelling_db: -3.2,
        ..Default::default()
    };
    let mut chain = Chain::new(cfg, 96_000, 2).unwrap();
    let src = SourceInfo {
        codec: "FLAC".into(),
        lossless: true,
        sample_rate: 96_000,
        bits: Some(24),
        channels: 2,
    };
    let out = OutputInfo {
        device: "USB DAC".into(),
        sample_rate: 96_000,
        bits: Some(24),
    };
    let p = SignalPath::new(src.clone(), chain.stages(), out.clone());
    assert_eq!(p.verdict, Verdict::LosslessProcessed);
    assert!(p.summary().contains("Levelling −3.2 dB"), "{}", p.summary());

    // Processed output is dithered: the same input twice gives different LSBs.
    let input = vec![1000i32; 2048];
    let a = through_chain(&mut chain, &input, 24, 24);
    assert!(
        a.iter().any(|v| *v != a[0]),
        "dither expected on processed output"
    );

    let bypass = Chain::new(ChainConfig::default(), 96_000, 2).unwrap();
    let p = SignalPath::new(src, bypass.stages(), out);
    assert_eq!(p.verdict, Verdict::BitPerfect);
}
