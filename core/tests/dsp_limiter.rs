//! The true-peak limiter's guarantee: whatever goes in, the output's true peak (measured
//! with the BS.1770-4 Annex 2 method, `dsp::truepeak`) never exceeds the ceiling.
//!
//! Signals are chosen to be adversarial: full-scale square waves (every sample at the rail,
//! large inter-sample overshoot), fs/4 sines at 45° (samples 3 dB below the true peak),
//! isolated and alternating impulses, sudden bursts after silence, steps, and random noise
//! at extreme gains, at several sample rates, ceilings and attack times.
//!
//! Debug builds run a reduced matrix; CI also runs this file in release for the full one.

/// The full matrix only in optimised builds (it is slow unoptimised).
const FULL: bool = !cfg!(debug_assertions);

use anywhere_core::dsp::limiter::{LimiterConfig, TruePeakLimiter};
use anywhere_core::dsp::truepeak::true_peak;
use anywhere_core::dsp::{db_to_gain, gain_to_db};
use std::f64::consts::PI;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// Run `x` (interleaved) through a limiter, flushing the tail; returns the output.
fn limit(x: &[f64], rate: u32, channels: usize, cfg: LimiterConfig) -> Vec<f64> {
    let mut l = TruePeakLimiter::new(rate, channels, cfg);
    let mut out = x.to_vec();
    for chunk in out.chunks_mut(997 * channels) {
        l.process(chunk);
    }
    let mut tail = vec![0.0; l.latency_frames() * channels];
    l.process(&mut tail);
    out.extend_from_slice(&tail);
    // Drop the leading latency so output lines up with input.
    out.drain(..l.latency_frames() * channels);
    out
}

fn signals(rate: u32, rng: &mut Rng) -> Vec<(String, Vec<f64>)> {
    let n = rate as usize / if FULL { 2 } else { 16 };
    let fs = f64::from(rate);
    let mut v: Vec<(String, Vec<f64>)> = Vec::new();
    for period in [2usize, 3, 4, 6, 8, 44, 441] {
        let sq: Vec<f64> = (0..n)
            .flat_map(|i| {
                let s = if (i / (period / 2).max(1)) % 2 == 0 {
                    1.0
                } else {
                    -1.0
                };
                [s, -s]
            })
            .collect();
        v.push((format!("square period {period}"), sq));
    }
    for phase in [PI / 4.0, 0.0, 0.3] {
        let s: Vec<f64> = (0..n)
            .flat_map(|i| {
                let x = (2.0 * PI * 0.25 * i as f64 + phase).sin();
                [x, x]
            })
            .collect();
        v.push((format!("fs/4 sine phase {phase:.2}"), s));
    }
    // Near-Nyquist sine: large true-peak excess on some phases.
    let s: Vec<f64> = (0..n)
        .flat_map(|i| {
            let x = (2.0 * PI * 0.45 * i as f64 + 0.7).sin();
            [x, -x]
        })
        .collect();
    v.push(("0.45 fs sine".into(), s));
    // Isolated impulses and impulse pairs.
    let mut imp = vec![0.0; n * 2];
    for k in (100..n - 100).step_by(rate as usize / 50) {
        imp[k * 2] = 1.0;
        imp[k * 2 + 3] = -1.0;
        imp[(k + 1) * 2] = -1.0;
    }
    v.push(("impulses".into(), imp));
    // Burst after silence, then a step.
    let mut burst = vec![0.0; n * 2];
    for i in n / 3..n / 2 {
        burst[i * 2] = rng.unit();
        burst[i * 2 + 1] = rng.unit();
    }
    for i in n * 2 / 3..n {
        burst[i * 2] = 0.99;
        burst[i * 2 + 1] = -0.99;
    }
    v.push(("burst and step".into(), burst));
    // Random noise with random sign flips at full scale.
    let noise: Vec<f64> = (0..n * 2)
        .map(|_| if rng.next() & 1 == 0 { 1.0 } else { -1.0 } * (0.8 + 0.2 * rng.unit().abs()))
        .collect();
    v.push(("rail noise".into(), noise));
    // A sweep.
    let sweep: Vec<f64> = (0..n)
        .flat_map(|i| {
            let t = i as f64 / fs;
            let x = (2.0 * PI * (20.0 * t + 0.5 * (fs * 0.45 - 20.0) * t * t * 2.0)).sin();
            [x, x * 0.5]
        })
        .collect();
    v.push(("sweep".into(), sweep));
    v
}

#[test]
fn output_true_peak_never_exceeds_ceiling() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    let mut worst = f64::NEG_INFINITY;
    let rates: &[u32] = if FULL {
        &[44_100, 48_000, 96_000, 192_000]
    } else {
        &[44_100, 192_000]
    };
    for &rate in rates {
        for (name, x) in signals(rate, &mut rng) {
            for boost_db in [0.0, 6.0, 20.0] {
                for (ceiling, attack) in [(-1.0, 2.0), (0.0, 0.5), (-0.1, 5.0), (-6.0, 1.0)] {
                    let cfg = LimiterConfig {
                        ceiling_dbtp: ceiling,
                        attack_ms: attack,
                        release_ms: 80.0,
                    };
                    let g = db_to_gain(boost_db);
                    let input: Vec<f64> = x.iter().map(|s| s * g).collect();
                    let out = limit(&input, rate, 2, cfg);
                    assert_eq!(out.len(), input.len());
                    let tp = true_peak(&out, 2);
                    let over = gain_to_db(tp) - ceiling;
                    worst = worst.max(over);
                    assert!(
                        tp <= db_to_gain(ceiling),
                        "{name} @ {rate} Hz, +{boost_db} dB, ceiling {ceiling}: {:.4} dBTP",
                        gain_to_db(tp)
                    );
                }
            }
        }
    }
    println!("worst true peak relative to ceiling: {worst:.4} dB");
}

#[test]
fn random_signals_never_exceed_ceiling() {
    let mut rng = Rng(42);
    for round in 0..if FULL { 40 } else { 8 } {
        let rate = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000][round % 6];
        let channels = 1 + round % 3;
        let frames = 2_000 + (rng.next() % 6_000) as usize;
        let gain = db_to_gain((rng.next() % 30) as f64);
        // Piecewise signals: random segments of silence, noise, rails and sines.
        let mut x = Vec::with_capacity(frames * channels);
        while x.len() < frames * channels {
            let seg = 1 + (rng.next() % 300) as usize;
            let kind = rng.next() % 4;
            let f = rng.unit().abs() * 0.5;
            for i in 0..seg * channels {
                x.push(
                    match kind {
                        0 => 0.0,
                        1 => rng.unit(),
                        2 => {
                            if (i / channels) % 2 == 0 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        _ => (2.0 * PI * f * (i / channels) as f64).sin(),
                    } * gain,
                );
            }
        }
        x.truncate(frames * channels);
        let cfg = LimiterConfig {
            ceiling_dbtp: -((rng.next() % 30) as f64) / 10.0,
            attack_ms: 0.5 + (rng.next() % 50) as f64 / 10.0,
            release_ms: 10.0 + (rng.next() % 500) as f64,
        };
        let out = limit(&x, rate, channels, cfg);
        let tp = true_peak(&out, channels);
        assert!(
            tp <= db_to_gain(cfg.ceiling_dbtp),
            "round {round}: {:.4} dBTP over {:?}",
            gain_to_db(tp),
            cfg
        );
    }
}

#[test]
fn quiet_material_is_untouched_and_loud_material_is_only_reduced() {
    // A -10 dBFS sine is passed bit-exactly (delayed); a +6 dB one is reduced, and the
    // gain never exceeds unity.
    let rate = 48_000;
    let x: Vec<f64> = (0..rate as usize)
        .map(|i| db_to_gain(-10.0) * (2.0 * PI * 1000.0 * i as f64 / f64::from(rate)).sin())
        .collect();
    let out = limit(&x, rate, 1, LimiterConfig::default());
    assert_eq!(out, x);

    let loud: Vec<f64> = x.iter().map(|s| s * db_to_gain(16.0)).collect();
    let out = limit(&loud, rate, 1, LimiterConfig::default());
    for (o, i) in out.iter().zip(&loud) {
        assert!(o.abs() <= i.abs() + 1e-12);
    }
    // In the steady state the sine sits just under the ceiling (the limiter isn't
    // over-reducing).
    let mid = &out[rate as usize / 2..];
    let peak = mid.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(gain_to_db(peak) > -1.5, "{}", gain_to_db(peak));
}

#[test]
fn hostile_input_is_survived() {
    let mut l = TruePeakLimiter::new(44_100, 2, LimiterConfig::default());
    let mut x = vec![
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e300,
        -1e300,
        f64::MIN_POSITIVE,
        5e-324,
        0.5,
    ];
    x.extend(std::iter::repeat_n(0.0, 4000));
    l.process(&mut x);
    assert!(x.iter().all(|v| v.is_finite()));
    assert!(true_peak(&x, 2) <= db_to_gain(-1.0));
}
