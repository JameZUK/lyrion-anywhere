//! Look-ahead brickwall limiter on true peak.
//!
//! # Method
//!
//! The gain is linked across channels and computed in five streaming steps:
//!
//! 1. **Detect.** A [`TruePeakDetector`] gives the largest 4×-oversampled |value| of each
//!    inter-sample interval (BS.1770-4 Annex 2 method).
//! 2. **Requirement.** Each interpolated point depends on the [`TAPS_PER_PHASE`] samples
//!    around it. The gain each *sample* may have is `ceiling / max(points it contributes
//!    to)`, a sliding maximum over `2·HALF_TAPS` intervals.
//! 3. **Hold.** A forward-looking minimum over `W + 2·HALF_TAPS` samples, where `W` is the
//!    attack (look-ahead) length.
//! 4. **Release.** Instant drops, exponential recovery towards the held value.
//! 5. **Attack.** A moving average over `W` samples turns each drop into a straight ramp
//!    that reaches its target exactly on time.
//!
//! Because every gain in a point's support is at most `ceiling / |that point|`, and the
//! hold makes the gain flat across a point's support around a peak, the output's true
//! peak stays at or below the ceiling. Where the gain still varies across a support (on a
//! ramp), the interpolated output can differ slightly from the gain-weighted input, so the
//! limiter aims [`SAFETY_MARGIN_DB`] below the ceiling. `core/tests/dsp_limiter.rs` checks
//! the guarantee with adversarial signals (full-scale square waves, fs/4 sines at 45°,
//! impulses, bursts and random noise at extreme gains) measured by the same meter.
//!
//! Latency is [`TruePeakLimiter::latency_frames`]; the host flushes the tail by processing
//! that many frames of silence.

use super::truepeak::{HALF_TAPS, TAPS_PER_PHASE, TruePeakDetector};
use super::{db_to_gain, gain_to_db};

/// The limiter aims this far below the configured ceiling (see the module docs).
pub const SAFETY_MARGIN_DB: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LimiterConfig {
    /// Ceiling, dBTP (-20..=0).
    pub ceiling_dbtp: f64,
    /// Look-ahead and attack time, ms (0.5..=20).
    pub attack_ms: f64,
    /// Release time constant, ms (10..=2000).
    pub release_ms: f64,
}

impl Default for LimiterConfig {
    fn default() -> Self {
        LimiterConfig {
            ceiling_dbtp: -1.0,
            attack_ms: 2.0,
            release_ms: 150.0,
        }
    }
}

impl LimiterConfig {
    /// The same configuration with every value forced into its supported range.
    pub fn sanitised(&self) -> Self {
        let f = |x: f64, d: f64, lo: f64, hi: f64| if x.is_finite() { x.clamp(lo, hi) } else { d };
        LimiterConfig {
            ceiling_dbtp: f(self.ceiling_dbtp, -1.0, -20.0, 0.0),
            attack_ms: f(self.attack_ms, 2.0, 0.5, 20.0),
            release_ms: f(self.release_ms, 150.0, 10.0, 2000.0),
        }
    }
}

/// Sliding-window extreme (minimum, or maximum via negation) over the last `window`
/// values, with a fixed-capacity monotonic queue: O(1) amortised, no allocation.
#[derive(Debug, Clone)]
struct SlidingMin {
    window: u64,
    idx: Vec<u64>,
    val: Vec<f64>,
    head: usize,
    len: usize,
    count: u64,
}

impl SlidingMin {
    fn new(window: usize) -> Self {
        let cap = window + 1;
        SlidingMin {
            window: window as u64,
            idx: vec![0; cap],
            val: vec![0.0; cap],
            head: 0,
            len: 0,
            count: 0,
        }
    }

    fn cap(&self) -> usize {
        self.idx.len()
    }

    /// Push a value; returns the minimum of the last `window` values pushed.
    #[inline]
    fn push(&mut self, v: f64) -> f64 {
        let i = self.count;
        self.count += 1;
        // Drop larger values from the back: they can never be the minimum again.
        while self.len > 0 {
            let back = (self.head + self.len - 1) % self.cap();
            if self.val[back] >= v {
                self.len -= 1;
            } else {
                break;
            }
        }
        let slot = (self.head + self.len) % self.cap();
        self.idx[slot] = i;
        self.val[slot] = v;
        self.len += 1;
        // Expire the front.
        while self.idx[self.head] + self.window <= i {
            self.head = (self.head + 1) % self.cap();
            self.len -= 1;
        }
        self.val[self.head]
    }

    fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.count = 0;
    }
}

#[derive(Debug, Clone)]
pub struct TruePeakLimiter {
    config: LimiterConfig,
    channels: usize,
    ceiling: f64,
    det: TruePeakDetector,
    /// Sliding maximum of interval peaks (stored negated).
    tp_max: SlidingMin,
    /// Forward-looking hold of the required gain.
    hold: SlidingMin,
    env: f64,
    release_coef: f64,
    attack: usize,
    ring: Vec<f64>,
    ring_pos: usize,
    ring_sum: f64,
    ring_since_resum: usize,
    /// Sliding maximum of the attack ring (stored negated).
    ring_max: SlidingMin,
    ring_min: SlidingMin,
    delay: Vec<f64>,
    delay_pos: usize,
    latency: usize,
    min_gain: f64,
    last_gain: f64,
}

impl TruePeakLimiter {
    pub fn new(sample_rate: u32, channels: usize, config: LimiterConfig) -> Self {
        let config = config.sanitised();
        let channels = channels.max(1);
        let rate = f64::from(sample_rate.clamp(8_000, 768_000));
        let attack = ((config.attack_ms * 1e-3 * rate).round() as usize).max(1);
        let hold_len = attack + 2 * HALF_TAPS;
        let latency = 2 * HALF_TAPS + hold_len - 2;
        let release_frames = config.release_ms * 1e-3 * rate;
        let mut l = TruePeakLimiter {
            config,
            channels,
            ceiling: db_to_gain(config.ceiling_dbtp - SAFETY_MARGIN_DB),
            det: TruePeakDetector::new(channels),
            tp_max: SlidingMin::new(TAPS_PER_PHASE),
            hold: SlidingMin::new(hold_len),
            env: 1.0,
            release_coef: 1.0 - (-1.0 / release_frames).exp(),
            attack,
            ring: vec![1.0; attack],
            ring_pos: 0,
            ring_sum: attack as f64,
            ring_since_resum: 0,
            ring_max: SlidingMin::new(attack),
            ring_min: SlidingMin::new(attack),
            delay: vec![0.0; latency * channels],
            delay_pos: 0,
            latency,
            min_gain: 1.0,
            last_gain: 1.0,
        };
        l.reset();
        l
    }

    pub fn config(&self) -> LimiterConfig {
        self.config
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Frames of delay between input and output.
    pub fn latency_frames(&self) -> usize {
        self.latency
    }

    /// The gain applied to the most recent output frame, dB (≤ 0).
    pub fn current_gain_db(&self) -> f64 {
        gain_to_db(self.last_gain)
    }

    /// The deepest gain reduction so far, dB (≤ 0).
    pub fn max_reduction_db(&self) -> f64 {
        gain_to_db(self.min_gain)
    }

    /// Clear all state (e.g. after a seek), as if preceded by silence.
    pub fn reset(&mut self) {
        self.det.reset();
        self.tp_max.reset();
        self.hold.reset();
        // Prime the windows with "no reduction" so the start isn't treated specially.
        for _ in 0..TAPS_PER_PHASE {
            self.tp_max.push(-0.0);
        }
        for _ in 0..self.hold.window {
            self.hold.push(1.0);
        }
        self.env = 1.0;
        self.ring.fill(1.0);
        self.ring_sum = self.attack as f64;
        self.ring_pos = 0;
        self.ring_since_resum = 0;
        self.ring_max.reset();
        self.ring_min.reset();
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.min_gain = 1.0;
        self.last_gain = 1.0;
    }

    /// Limit interleaved frames in place. Output is delayed by [`Self::latency_frames`].
    /// Non-finite input samples are treated as silence.
    pub fn process(&mut self, buf: &mut [f64]) {
        let ch = self.channels;
        for frame in buf.chunks_exact_mut(ch) {
            for s in frame.iter_mut() {
                *s = super::sanitise(*s);
            }
            // 1. Interval peak, 2. sliding maximum over the support → requirement.
            let p = self.det.push_frame(frame);
            let peak = -self.tp_max.push(-p);
            let req = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            // 3. Hold.
            let m = self.hold.push(req);
            // 4. Release.
            self.env = if m <= self.env {
                m
            } else {
                self.env + (m - self.env) * self.release_coef
            };
            // 5. Attack: moving average.
            self.ring_sum += self.env - self.ring[self.ring_pos];
            self.ring[self.ring_pos] = self.env;
            self.ring_pos = (self.ring_pos + 1) % self.attack;
            self.ring_since_resum += 1;
            if self.ring_since_resum >= self.attack.max(64) {
                // Re-sum exactly from time to time so rounding can't drift.
                self.ring_sum = self.ring.iter().sum();
                self.ring_since_resum = 0;
            }
            // Every value in the ring is at most the requirement, so the mean is too. A
            // running sum can round outside the ring's range (badly so across a huge
            // dynamic range), so the mean is clamped to the ring's exact extremes.
            let ring_max = -self.ring_max.push(-self.env);
            let ring_min = self.ring_min.push(self.env);
            let g = (self.ring_sum / self.attack as f64)
                .clamp(ring_min, ring_max)
                .min(1.0);
            self.last_gain = g;
            if g < self.min_gain {
                self.min_gain = g;
            }

            // Delay line: swap the new frame in, the delayed frame out.
            let base = self.delay_pos * ch;
            for (c, s) in frame.iter_mut().enumerate() {
                let delayed = self.delay[base + c];
                self.delay[base + c] = *s;
                *s = if g == 1.0 { delayed } else { delayed * g };
            }
            self.delay_pos = (self.delay_pos + 1) % self.latency;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_min_matches_brute_force() {
        let mut s = SlidingMin::new(5);
        let vals = [
            5.0, 3.0, 4.0, 9.0, 1.0, 7.0, 8.0, 6.0, 6.0, 2.0, 9.0, 9.0, 9.0, 9.0, 9.0,
        ];
        for (i, v) in vals.iter().enumerate() {
            let got = s.push(*v);
            let lo = i.saturating_sub(4);
            let want = vals[lo..=i].iter().cloned().fold(f64::INFINITY, f64::min);
            assert_eq!(got, want, "at {i}");
        }
    }

    #[test]
    fn quiet_signal_passes_unchanged_after_latency() {
        let mut l = TruePeakLimiter::new(48_000, 2, LimiterConfig::default());
        let lat = l.latency_frames();
        let n = 2000;
        let input: Vec<f64> = (0..n * 2)
            .map(|i| ((i * 7919) % 1000) as f64 / 5000.0 - 0.1)
            .collect();
        let mut buf = input.clone();
        l.process(&mut buf);
        assert!(buf[..lat * 2].iter().all(|x| *x == 0.0));
        assert_eq!(&buf[lat * 2..], &input[..(n - lat) * 2]);
        assert_eq!(l.max_reduction_db(), 0.0);
    }

    #[test]
    fn config_is_sanitised() {
        let c = LimiterConfig {
            ceiling_dbtp: 6.0,
            attack_ms: f64::NAN,
            release_ms: -1.0,
        }
        .sanitised();
        assert_eq!(c.ceiling_dbtp, 0.0);
        assert_eq!(c.attack_ms, 2.0);
        assert_eq!(c.release_ms, 10.0);
    }
}
