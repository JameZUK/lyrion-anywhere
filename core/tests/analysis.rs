//! Track analysis: conformance with signals whose loudness is known analytically (EBU Tech
//! 3341 and 3342 cases, synthesised from their published descriptions), at several sample
//! rates, plus robustness and encoding tests.

use anywhere_core::analysis::{
    Analyser, AnalysisConfig, AnalysisError, TrackAnalysis, album_loudness, analyse_encoded,
};
use std::f64::consts::PI;

const FULL: bool = !cfg!(debug_assertions);

/// Stereo 997 Hz sine segments: (seconds, level dBFS peak per channel).
fn tone(rate: u32, segments: &[(f64, f64)]) -> Vec<f32> {
    let mut v = Vec::new();
    let mut i = 0u64;
    for &(secs, db) in segments {
        let a = 10f64.powf(db / 20.0);
        for _ in 0..(secs * f64::from(rate)) as u64 {
            let x = (a * (2.0 * PI * 997.0 * i as f64 / f64::from(rate)).sin()) as f32;
            v.push(x);
            v.push(x);
            i += 1;
        }
    }
    v
}

fn analyse(rate: u32, channels: usize, x: &[f32]) -> TrackAnalysis {
    let mut a = Analyser::new(rate, channels, AnalysisConfig::default()).unwrap();
    for c in x.chunks(4096 * channels) {
        a.push(c).unwrap();
    }
    a.finish()
}

fn rates() -> &'static [u32] {
    if FULL {
        &[44_100, 48_000, 96_000, 192_000]
    } else {
        &[44_100, 48_000, 192_000]
    }
}

#[test]
fn tech3341_steady_tones() {
    for &rate in rates() {
        for level in [-23.0, -33.0, -18.0] {
            let a = analyse(
                rate,
                2,
                &tone(rate, &[(if FULL { 20.0 } else { 5.0 }, level)]),
            );
            let i = a.integrated_lufs.unwrap();
            assert!((i - level).abs() <= 0.1, "{rate} Hz, {level}: {i}");
            let m = a.max_momentary_lufs.unwrap();
            assert!((m - level).abs() <= 0.1, "{rate} Hz momentary {m}");
            // A sine's sample peak and true peak are its amplitude.
            assert!((a.sample_peak_dbfs.unwrap() - level).abs() < 0.05);
            assert!((a.true_peak_dbtp.unwrap() - level).abs() < 0.1);
        }
    }
}

#[test]
fn tech3341_gated_sequences() {
    let rate = 48_000;
    // Case 3: quieter parts removed by the relative gate.
    let a = analyse(
        rate,
        2,
        &tone(rate, &[(10.0, -36.0), (60.0, -23.0), (10.0, -36.0)]),
    );
    assert!((a.integrated_lufs.unwrap() + 23.0).abs() <= 0.1, "{a:?}");
    // Case 4: below the absolute gate as well.
    let a = analyse(
        rate,
        2,
        &tone(
            rate,
            &[
                (10.0, -72.0),
                (10.0, -36.0),
                (60.0, -23.0),
                (10.0, -36.0),
                (10.0, -72.0),
            ],
        ),
    );
    assert!((a.integrated_lufs.unwrap() + 23.0).abs() <= 0.1);
    // Two levels 10 dB apart, both above the relative gate: the power mean,
    // 10·log10((10^-2 + 10^-3) / 2) = -22.60 LUFS.
    let a = analyse(rate, 2, &tone(rate, &[(20.0, -20.0), (20.0, -30.0)]));
    assert!(
        (a.integrated_lufs.unwrap() + 22.60).abs() <= 0.1,
        "{:?}",
        a.integrated_lufs
    );
}

#[test]
fn tech3342_loudness_range() {
    let cases: [(&[(f64, f64)], f64); 4] = [
        (&[(20.0, -20.0), (20.0, -30.0)], 10.0),
        (&[(20.0, -20.0), (20.0, -15.0)], 5.0),
        (&[(20.0, -40.0), (20.0, -20.0)], 20.0),
        (
            &[
                (20.0, -50.0),
                (20.0, -35.0),
                (20.0, -20.0),
                (20.0, -35.0),
                (20.0, -50.0),
            ],
            15.0,
        ),
    ];
    for &rate in if FULL {
        &[44_100u32, 48_000][..]
    } else {
        &[48_000u32][..]
    } {
        for (segs, want) in cases {
            let a = analyse(rate, 2, &tone(rate, segs));
            let lra = a.loudness_range_lu.unwrap();
            assert!(
                (lra - want).abs() <= 1.0,
                "{rate} Hz {segs:?}: LRA {lra}, want {want}"
            );
        }
    }
}

#[test]
fn true_peak_of_inter_sample_peaks() {
    // fs/4 sine at 45°: samples at -3.01 dB of the amplitude, true peak at the amplitude.
    for &rate in rates() {
        let x: Vec<f32> = (0..rate as usize * 2)
            .flat_map(|i| {
                let v = (0.5 * (PI / 2.0 * i as f64 + PI / 4.0).sin()) as f32;
                [v, v]
            })
            .collect();
        let a = analyse(rate, 2, &x);
        let sp = a.sample_peak_dbfs.unwrap();
        let tp = a.true_peak_dbtp.unwrap();
        assert!((sp + 9.03).abs() < 0.02, "{sp}");
        assert!((tp + 6.02).abs() < 0.2, "{rate}: {tp}");
    }
}

#[test]
fn mono_and_surround_weighting() {
    let rate = 48_000;
    let st = tone(rate, &[(5.0, -23.0)]);
    let mono: Vec<f32> = st.chunks(2).map(|f| f[0]).collect();
    let a = analyse(rate, 1, &mono);
    assert!(
        (a.integrated_lufs.unwrap() + 26.01).abs() < 0.1,
        "mono is one channel"
    );
    // Six channels: a signal in the LFE alone doesn't count; in a surround it counts 1.41×.
    let six = |ch: usize| -> Vec<f32> {
        mono.iter()
            .flat_map(|s| (0..6).map(move |c| if c == ch { *s } else { 0.0 }))
            .collect()
    };
    assert!(analyse(rate, 6, &six(3)).integrated_lufs.is_none());
    let front = analyse(rate, 6, &six(0)).integrated_lufs.unwrap();
    let surround = analyse(rate, 6, &six(4)).integrated_lufs.unwrap();
    assert!((surround - front - 10.0 * 1.41f64.log10()).abs() < 0.02);
}

#[test]
fn silence_and_fades() {
    let rate = 44_100;
    // 2 s silence, 4 s fade in, 20 s of tone, 6 s fade out, 3 s silence.
    let mut x: Vec<f32> = vec![0.0; 2 * rate as usize * 2];
    let body = 10f64.powf(-12.0 / 20.0);
    let n_in = 4 * rate as usize;
    let n_body = 20 * rate as usize;
    let n_out = 6 * rate as usize;
    for i in 0..n_in + n_body + n_out {
        // Fades are linear in dB, between -40 dB and the body level (so they stay above
        // the -60 dBFS silence threshold).
        let g_db = if i < n_in {
            -40.0 * (1.0 - i as f64 / n_in as f64)
        } else if i < n_in + n_body {
            0.0
        } else {
            -40.0 * ((i - n_in - n_body) as f64 / n_out as f64)
        };
        let v = (body
            * 10f64.powf(g_db / 20.0)
            * (2.0 * PI * 440.0 * i as f64 / f64::from(rate)).sin()) as f32;
        x.push(v);
        x.push(v);
    }
    x.extend(std::iter::repeat_n(0.0f32, 3 * rate as usize * 2));
    let a = analyse(rate, 2, &x);
    assert!(
        (a.leading_silence_s - 2.0).abs() < 0.05,
        "{}",
        a.leading_silence_s
    );
    assert!(
        (a.trailing_silence_s - 3.0).abs() < 0.05,
        "{}",
        a.trailing_silence_s
    );
    assert!(a.fade_in.fade, "{:?}", a.fade_in);
    assert!((a.fade_in.length_s - 4.0).abs() < 1.0, "{:?}", a.fade_in);
    assert!(a.fade_out.fade, "{:?}", a.fade_out);
    assert!((a.fade_out.length_s - 6.0).abs() < 1.5, "{:?}", a.fade_out);
    assert!(a.fade_out.slope_db_per_s < -3.0);

    // A plain tone has no fades and no silence.
    let a = analyse(rate, 2, &tone(rate, &[(12.0, -20.0)]));
    assert!(!a.fade_in.fade && !a.fade_out.fade);
    assert_eq!(a.leading_silence_s, 0.0);
    assert_eq!(a.trailing_silence_s, 0.0);
}

#[test]
fn degenerate_inputs() {
    // Zero-length track.
    let a = Analyser::new(44_100, 2, AnalysisConfig::default())
        .unwrap()
        .finish();
    assert_eq!(a.frames, 0);
    assert!(a.integrated_lufs.is_none() && a.true_peak_dbtp.is_none());
    assert!(a.waveform.peak.is_empty());
    // Digital silence.
    let a = analyse(48_000, 2, &vec![0.0; 48_000 * 2 * 3]);
    assert!(a.integrated_lufs.is_none() && a.loudness_range_lu.is_none());
    assert_eq!(a.leading_silence_s, 3.0);
    assert!(a.waveform.peak.iter().all(|v| *v == 0));
    // NaN, infinities and subnormals.
    let mut x = tone(48_000, &[(3.0, -20.0)]);
    for i in (0..x.len()).step_by(1000) {
        x[i] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e-45][i / 1000 % 4];
    }
    let a = analyse(48_000, 2, &x);
    assert!(a.invalid_samples > 0);
    assert!((a.integrated_lufs.unwrap() + 20.0).abs() < 0.2);
    let sub = vec![1e-310f64; 48_000 * 2];
    let mut an = Analyser::new(48_000, 2, AnalysisConfig::default()).unwrap();
    an.push(&sub).unwrap();
    assert!(an.finish().integrated_lufs.is_none());
}

#[test]
fn bad_parameters_and_rate_changes_are_refused() {
    assert_eq!(
        Analyser::new(0, 2, AnalysisConfig::default()).err(),
        Some(AnalysisError::UnsupportedRate(0))
    );
    assert!(Analyser::new(10_000_000, 2, AnalysisConfig::default()).is_err());
    assert!(Analyser::new(48_000, 0, AnalysisConfig::default()).is_err());
    assert!(Analyser::new(48_000, 99, AnalysisConfig::default()).is_err());
    let mut a = Analyser::new(48_000, 2, AnalysisConfig::default()).unwrap();
    assert_eq!(a.push(&[0.0f32; 3]), Err(AnalysisError::PartialFrame));
    a.push_with_rate(48_000, &[0.1f32; 4]).unwrap();
    assert_eq!(
        a.push_with_rate(44_100, &[0.1f32; 4]),
        Err(AnalysisError::RateChanged {
            expected: 48_000,
            got: 44_100
        })
    );
    // Hostile config values are clamped rather than trusted.
    let cfg = AnalysisConfig {
        buckets: usize::MAX,
        silence_threshold_dbfs: f64::NAN,
        min_silence_s: f64::INFINITY,
        fade_window_s: -5.0,
    };
    let mut a = Analyser::new(48_000, 1, cfg).unwrap();
    a.push(&[0.5f32; 48_000]).unwrap();
    let r = a.finish();
    assert_eq!(r.waveform.peak.len(), 10_000);
}

#[test]
fn very_long_tracks_use_constant_memory() {
    // Simulate a long track at a low rate: an hour in release builds, ten minutes in debug.
    let rate = 8_000;
    let secs = if FULL { 3600 } else { 600 };
    let mut a = Analyser::new(rate, 1, AnalysisConfig::default()).unwrap();
    let block: Vec<f32> = (0..rate as usize)
        .map(|i| (0.1 * (2.0 * PI * 997.0 * i as f64 / f64::from(rate)).sin()) as f32)
        .collect();
    for _ in 0..secs {
        a.push(&block).unwrap();
    }
    let r = a.finish();
    assert_eq!(r.frames, rate as u64 * secs);
    assert_eq!(r.waveform.peak.len(), 1000);
    // The same as ten seconds of the same signal.
    let short = analyse(rate, 1, &block.repeat(10));
    assert!((r.integrated_lufs.unwrap() - short.integrated_lufs.unwrap()).abs() < 0.02);
    assert!(r.waveform.peak.iter().all(|p| *p == r.waveform.peak[0]));
}

#[test]
fn encodings_are_compact_and_round_trip() {
    let a = analyse(
        44_100,
        2,
        &tone(44_100, &[(1.0, -60.0), (8.0, -14.0), (1.0, -40.0)]),
    );
    let json = a.to_json();
    assert!(json.len() < 4096, "JSON is {} bytes", json.len());
    assert_eq!(TrackAnalysis::from_json(&json).unwrap(), a);
    let bin = a.to_bytes();
    assert!(bin.len() < 2200, "binary is {} bytes", bin.len());
    let back = TrackAnalysis::from_bytes(&bin).unwrap();
    assert_eq!(back.waveform, a.waveform);
    assert_eq!(back.frames, a.frames);
    assert!((back.integrated_lufs.unwrap() - a.integrated_lufs.unwrap()).abs() < 1e-4);
    assert_eq!(back.to_bytes(), bin);
}

/// Small deterministic PRNG for the decoder robustness tests.
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
fn decoders_survive_hostile_input() {
    let a = analyse(44_100, 2, &tone(44_100, &[(3.0, -14.0)]));
    let bin = a.to_bytes();
    let json = a.to_json();
    let mut r = Rng(99);
    let iters = if FULL { 50_000 } else { 5_000 };
    for _ in 0..iters {
        // Truncations, bit flips and random bytes: never a panic.
        let mut b = bin.clone();
        match r.next() % 3 {
            0 => b.truncate((r.next() % bin.len() as u64) as usize),
            1 => {
                for _ in 0..1 + r.next() % 8 {
                    let i = (r.next() % b.len() as u64) as usize;
                    b[i] ^= 1 << (r.next() % 8);
                }
            }
            _ => b = (0..r.next() % 300).map(|_| r.next() as u8).collect(),
        }
        let _ = TrackAnalysis::from_bytes(&b);
        let mut j = json.clone().into_bytes();
        let i = (r.next() % j.len() as u64) as usize;
        j[i] = b"{}[]\":,0aZ=+/\\"[(r.next() % 14) as usize];
        if let Ok(s) = std::str::from_utf8(&j) {
            let _ = TrackAnalysis::from_json(s);
        }
    }
    // Specific refusals.
    assert!(TrackAnalysis::from_bytes(b"").is_err());
    assert!(TrackAnalysis::from_bytes(b"LAAN\x02\x00").is_err());
    let mut long = bin.clone();
    long.push(0);
    assert!(TrackAnalysis::from_bytes(&long).is_err(), "trailing bytes");
    let huge = vec![b'A'; 1 << 20];
    assert!(TrackAnalysis::from_bytes(&huge).is_err());
    assert!(TrackAnalysis::from_json(&format!("\"{}\"", "A".repeat(1 << 20))).is_err());
    let bad = json.replacen("\"version\":1", "\"version\":9", 1);
    assert!(TrackAnalysis::from_json(&bad).is_err());
}

#[test]
fn album_loudness_from_tracks() {
    let rate = 44_100;
    let t1 = analyse(rate, 2, &tone(rate, &[(5.0, -20.0)]));
    let t2 = analyse(rate, 2, &tone(rate, &[(5.0, -26.0)]));
    let (l, p) = album_loudness(&[t1.clone(), t2]).unwrap();
    // Power mean of -20 and -26 over equal durations.
    let want = 10.0 * ((10f64.powf(-2.0) + 10f64.powf(-2.6)) / 2.0).log10();
    assert!((l - want).abs() < 0.05, "{l} vs {want}");
    assert_eq!(p, t1.true_peak_dbtp);
    assert!(album_loudness(&[]).is_none());
}

#[test]
fn analyses_an_encoded_file() {
    // A 16-bit stereo WAV of a -20 dBFS tone, decoded by the core's own decoder.
    let rate = 48_000u32;
    let pcm = tone(rate, &[(4.0, -20.0)]);
    let mut wav = Vec::new();
    let data_len = pcm.len() * 2;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in &pcm {
        wav.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes());
    }
    let a = analyse_encoded(&wav, b'p', AnalysisConfig::default()).unwrap();
    assert_eq!(a.sample_rate, rate);
    assert_eq!(a.frames, u64::from(rate) * 4);
    assert!((a.integrated_lufs.unwrap() + 20.0).abs() < 0.1);
    assert!(analyse_encoded(&[0x42; 1000], b'f', AnalysisConfig::default()).is_err());
}
