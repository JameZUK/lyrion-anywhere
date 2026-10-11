//! Timed checks that the DSP chain and track analysis run far faster than real time.
//! Thresholds are asserted only in optimised builds (`cargo test --release`); debug builds
//! just report. Run with `--nocapture` to see the figures.

use anywhere_core::analysis::{Analyser, AnalysisConfig};
use anywhere_core::dsp::chain::{Chain, ChainConfig};
use anywhere_core::dsp::crossfeed::CrossfeedPreset;
use anywhere_core::dsp::eq::parse_autoeq;
use anywhere_core::dsp::limiter::LimiterConfig;
use std::time::Instant;

const OPTIMISED: bool = !cfg!(debug_assertions);
const RATE: u32 = 44_100;

/// Music-like test material: a few tones plus noise, with a slow level swing.
fn material(secs: usize) -> Vec<f32> {
    let mut seed = 1u64;
    let n = RATE as usize * secs;
    let mut v = Vec::with_capacity(n * 2);
    for i in 0..n {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let noise = (seed >> 40) as f64 / (1u64 << 24) as f64 - 0.5;
        let t = i as f64 / f64::from(RATE);
        let swing = 0.5 + 0.4 * (t * 0.3).sin();
        let x = swing * (0.3 * (t * 2.0 * std::f64::consts::PI * 220.0).sin() + 0.2 * noise);
        v.push(x as f32);
        v.push((x * 0.9) as f32);
    }
    v
}

#[test]
fn analysis_is_far_faster_than_real_time() {
    let secs = if OPTIMISED { 120 } else { 10 };
    let x = material(secs);
    let start = Instant::now();
    let mut a = Analyser::new(RATE, 2, AnalysisConfig::default()).unwrap();
    for c in x.chunks(8192) {
        a.push(c).unwrap();
    }
    let r = a.finish();
    let elapsed = start.elapsed().as_secs_f64();
    let factor = secs as f64 / elapsed;
    println!("analysis: {secs} s of 44.1 kHz stereo in {elapsed:.3} s = {factor:.0}× real time");
    assert!(r.integrated_lufs.is_some());
    if OPTIMISED {
        assert!(factor >= 200.0, "analysis only {factor:.0}× real time");
    }
}

#[test]
fn full_chain_is_far_faster_than_real_time() {
    let secs = if OPTIMISED { 60 } else { 5 };
    let mut x: Vec<f64> = material(secs).iter().map(|s| f64::from(*s)).collect();
    let eq: String = (1..=10)
        .map(|i| {
            format!(
                "Filter {i}: ON PK Fc {} Hz Gain 2 dB Q 1.4\n",
                30 * (1 << i)
            )
        })
        .collect();
    let cfg = ChainConfig {
        preamp_db: -3.0,
        levelling_db: 6.0,
        eq: Some(parse_autoeq(&eq).unwrap()),
        crossfeed: Some(CrossfeedPreset::Natural.config()),
        limiter: Some(LimiterConfig::default()),
        ..Default::default()
    };
    let mut chain = Chain::new(cfg, RATE, 2).unwrap();
    let mut out = vec![0i32; 4096];
    let start = Instant::now();
    for c in x.chunks_mut(4096) {
        chain.process(c);
        chain.quantise(c, Some(16), 16, &mut out[..c.len()]);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let factor = secs as f64 / elapsed;
    println!(
        "chain (gain, 10-band EQ, crossfeed, true-peak limiter, dither): {secs} s in {elapsed:.3} s = {factor:.0}× real time"
    );
    if OPTIMISED {
        assert!(factor >= 50.0, "chain only {factor:.0}× real time");
    }
}
