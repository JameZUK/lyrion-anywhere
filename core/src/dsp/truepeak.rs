//! True-peak detection by 4× oversampling, following the method of ITU-R BS.1770-4
//! Annex 2: upsample by four with a low-pass interpolating filter, then take the largest
//! absolute value. (The Annex's attenuation step exists for integer arithmetic and is
//! unnecessary in `f64`.)
//!
//! The interpolator is our own design, not the Annex's example coefficients: a polyphase
//! windowed-sinc filter with [`TAPS_PER_PHASE`] taps per phase, cutoff at the original
//! Nyquist frequency and a Kaiser window, each phase normalised to unity gain at DC. Phase
//! 0 is the original sample itself (the sinc is zero at every other integer), so the true
//! peak can never read below the sample peak.
//!
//! Sampling a sine at 4× its rate under-reads its peak by at most `1 - cos(π f / 4 fs)`,
//! about 0.17 dB at fs/4 and 0.55 dB at 0.45 fs, which is the same order as the Annex's
//! example filter. The limiter uses this same detector, so its ceiling is guaranteed as
//! measured by this meter.

/// Oversampling factor.
pub const OVERSAMPLE: usize = 4;
/// Samples either side of an interpolated point that contribute to it.
pub const HALF_TAPS: usize = 8;
/// Taps per polyphase branch.
pub const TAPS_PER_PHASE: usize = 2 * HALF_TAPS;
/// Kaiser window shape parameter.
const KAISER_BETA: f64 = 6.0;

/// Polyphase coefficients for phases 1..OVERSAMPLE (phase 0 is the sample itself).
#[derive(Debug, Clone)]
pub struct Interpolator {
    phases: [[f64; TAPS_PER_PHASE]; OVERSAMPLE - 1],
}

impl Default for Interpolator {
    fn default() -> Self {
        Self::new()
    }
}

/// Zeroth-order modified Bessel function of the first kind (power series).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let p = std::f64::consts::PI * x;
        p.sin() / p
    }
}

impl Interpolator {
    pub fn new() -> Self {
        let mut phases = [[0.0; TAPS_PER_PHASE]; OVERSAMPLE - 1];
        let half = HALF_TAPS as f64;
        let norm = bessel_i0(KAISER_BETA);
        for (p, taps) in phases.iter_mut().enumerate() {
            let frac = (p + 1) as f64 / OVERSAMPLE as f64;
            for (j, c) in taps.iter_mut().enumerate() {
                // Time of the input sample relative to the interpolated point.
                let d = (j as f64 - half + 1.0) - frac;
                let r = d / half;
                let w = if r.abs() >= 1.0 {
                    0.0
                } else {
                    bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / norm
                };
                *c = sinc(d) * w;
            }
            let sum: f64 = taps.iter().sum();
            for c in taps.iter_mut() {
                *c /= sum;
            }
        }
        Interpolator { phases }
    }

    /// The largest |value| in the interval starting at the sample `window[HALF_TAPS - 1]`,
    /// including that sample, given `window` = the [`TAPS_PER_PHASE`] samples around it.
    #[inline]
    pub fn interval_peak(&self, window: &[f64]) -> f64 {
        let w = &window[..TAPS_PER_PHASE];
        let mut peak = w[HALF_TAPS - 1].abs();
        for taps in &self.phases {
            let mut acc = 0.0;
            for k in 0..TAPS_PER_PHASE {
                acc += taps[k] * w[k];
            }
            peak = peak.max(acc.abs());
        }
        peak
    }

    /// Interpolated value at `phase` (0..OVERSAMPLE) of the interval described above.
    pub fn value(&self, window: &[f64], phase: usize) -> f64 {
        if phase == 0 {
            return window[HALF_TAPS - 1];
        }
        self.phases[phase - 1]
            .iter()
            .zip(window)
            .map(|(c, x)| c * x)
            .sum()
    }

    /// The sum of the absolute coefficients of the worst phase: an upper bound on how far
    /// an interpolated value can exceed the window's sample peak.
    pub fn max_abs_gain(&self) -> f64 {
        self.phases
            .iter()
            .map(|t| t.iter().map(|c| c.abs()).sum::<f64>())
            .fold(1.0, f64::max)
    }
}

/// Streaming true-peak detector for interleaved frames of a fixed channel count.
///
/// Each pushed frame completes the interval [`HALF_TAPS`] frames earlier, so results lag
/// the input by `HALF_TAPS` frames. History starts as silence.
#[derive(Debug, Clone)]
pub struct TruePeakDetector {
    interp: Interpolator,
    channels: usize,
    /// Per channel, a ring of TAPS_PER_PHASE samples stored twice so any window is contiguous.
    hist: Vec<f64>,
    pos: usize,
    /// Sliding maximum of |sample| (over channels) across the window (stored negated).
    window_peak: super::window::SlidingMin,
    /// Upper bound on |interpolated| / |window peak|.
    bound: f64,
}

impl TruePeakDetector {
    pub fn new(channels: usize) -> Self {
        let channels = channels.max(1);
        let interp = Interpolator::new();
        TruePeakDetector {
            bound: interp.max_abs_gain(),
            interp,
            channels,
            hist: vec![0.0; channels * 2 * TAPS_PER_PHASE],
            pos: 0,
            window_peak: super::window::SlidingMin::new(TAPS_PER_PHASE),
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn reset(&mut self) {
        self.hist.fill(0.0);
        self.pos = 0;
        self.window_peak.reset();
    }

    /// Push one frame; returns the largest |value| over all channels in the interval that
    /// this frame completed. Non-finite samples count as silence.
    #[inline]
    pub fn push_frame(&mut self, frame: &[f64]) -> f64 {
        self.push_frame_above(frame, -1.0)
    }

    /// Like [`push_frame`](Self::push_frame), but when the interval provably can't exceed
    /// `floor` it skips the interpolation and returns an upper bound that is ≤ `floor`.
    /// Callers that only care about values above a threshold (the limiter's ceiling, a
    /// meter's running maximum) save most of the work this way.
    #[inline]
    pub fn push_frame_above(&mut self, frame: &[f64], floor: f64) -> f64 {
        let n = TAPS_PER_PHASE;
        let mut frame_peak: f64 = 0.0;
        for c in 0..self.channels {
            let x = super::sanitise(frame.get(c).copied().unwrap_or(0.0));
            let base = c * 2 * n;
            self.hist[base + self.pos] = x;
            self.hist[base + self.pos + n] = x;
            frame_peak = frame_peak.max(x.abs());
        }
        let start_pos = self.pos + 1;
        self.pos = (self.pos + 1) % n;
        // No interpolated value can exceed the window's sample peak times Σ|h|.
        let bound = -self.window_peak.push(-frame_peak) * self.bound;
        if bound <= floor {
            return bound;
        }
        let mut peak: f64 = 0.0;
        for c in 0..self.channels {
            // The window runs from the oldest sample to the one just written.
            let start = c * 2 * n + start_pos;
            peak = peak.max(self.interp.interval_peak(&self.hist[start..start + n]));
        }
        peak
    }
}

/// Measures the true peak of a whole stream.
#[derive(Debug, Clone)]
pub struct TruePeakMeter {
    det: TruePeakDetector,
    peak: f64,
    zero: Vec<f64>,
}

impl TruePeakMeter {
    pub fn new(channels: usize) -> Self {
        let det = TruePeakDetector::new(channels);
        let zero = vec![0.0; det.channels()];
        TruePeakMeter {
            det,
            peak: 0.0,
            zero,
        }
    }

    /// Feed interleaved samples (a trailing partial frame is ignored).
    pub fn push(&mut self, samples: &[f64]) {
        for f in samples.chunks_exact(self.det.channels) {
            self.peak = self.peak.max(self.det.push_frame_above(f, self.peak));
        }
    }

    /// Flush the filter (as if followed by silence) and return the linear true peak.
    pub fn finish(&mut self) -> f64 {
        for _ in 0..HALF_TAPS {
            let p = self.det.push_frame(&self.zero);
            self.peak = self.peak.max(p);
        }
        self.det.reset();
        self.peak
    }
}

/// The true peak (linear) of interleaved `samples`, assuming silence either side.
pub fn true_peak(samples: &[f64], channels: usize) -> f64 {
    let mut m = TruePeakMeter::new(channels);
    m.push(samples);
    m.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// A sine with 50 ms raised-cosine fades, so the abrupt start and end of a test
    /// signal don't add their own (genuine) inter-sample overshoot.
    fn sine(freq: f64, rate: f64, phase: f64, amp: f64, n: usize) -> Vec<f64> {
        let fade = (rate * 0.05) as usize;
        (0..n)
            .map(|i| {
                let edge = i.min(n - 1 - i);
                let w = if edge < fade {
                    0.5 - 0.5 * (PI * edge as f64 / fade as f64).cos()
                } else {
                    1.0
                };
                w * amp * (2.0 * PI * freq * i as f64 / rate + phase).sin()
            })
            .collect()
    }

    #[test]
    fn phase_zero_is_the_sample() {
        let ip = Interpolator::new();
        let w: Vec<f64> = (0..TAPS_PER_PHASE).map(|i| i as f64 * 0.01).collect();
        assert_eq!(ip.value(&w, 0), w[HALF_TAPS - 1]);
        // DC passes at unity gain on every phase.
        let dc = [0.5; TAPS_PER_PHASE];
        for p in 0..OVERSAMPLE {
            assert!((ip.value(&dc, p) - 0.5).abs() < 1e-12);
        }
    }

    #[test]
    fn quarter_rate_sine_at_45_degrees_reads_true_peak() {
        // Samples sit at ±0.7071 of the amplitude; the true peak is the amplitude.
        let x = sine(12_000.0, 48_000.0, PI / 4.0, 1.0, 48_000);
        let sample_peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!((sample_peak - 0.5f64.sqrt()).abs() < 1e-9);
        let tp = true_peak(&x, 1);
        assert!((tp - 1.0).abs() < 0.03, "tp {tp}");
    }

    #[test]
    fn sines_read_within_tolerance() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            for f in [100.0, 997.0, 5_000.0, 10_000.0, 15_000.0, 19_000.0] {
                for ph in [0.0, 0.3, 1.1] {
                    let x = sine(f, rate, ph, 0.5, rate as usize / 4);
                    let tp = super::super::gain_to_db(true_peak(&x, 1) / 0.5);
                    assert!(tp > -0.6 && tp < 0.15, "{rate} {f} {ph}: {tp} dB");
                }
            }
        }
    }

    #[test]
    fn stereo_takes_the_louder_channel_and_ignores_nan() {
        let mut x = vec![0.0; 200];
        x[101] = -0.8;
        x[50] = f64::NAN;
        assert!(true_peak(&x, 2) >= 0.8);
        assert_eq!(true_peak(&[], 2), 0.0);
    }

    #[test]
    fn bounded_push_agrees_with_exact() {
        let x: Vec<f64> = (0..20_000)
            .map(|i| {
                ((i * 7919 % 1013) as f64 / 1013.0 - 0.5) * if i % 3000 < 50 { 1.8 } else { 0.4 }
            })
            .collect();
        let mut exact = TruePeakDetector::new(2);
        let mut fast = TruePeakDetector::new(2);
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for f in x.as_chunks::<2>().0 {
            let e = exact.push_frame(f);
            let q = fast.push_frame_above(f, b);
            assert!(q >= e || q <= b, "bound violated");
            a = a.max(e);
            b = b.max(q);
        }
        assert_eq!(a, b);
    }

    #[test]
    fn bound_holds() {
        let ip = Interpolator::new();
        // Σ|h| of a 16-tap half-sample phase is about 2.04.
        assert!(ip.max_abs_gain() > 1.0 && ip.max_abs_gain() < 2.5);
    }
}
