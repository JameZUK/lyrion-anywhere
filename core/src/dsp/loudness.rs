//! Loudness-compensated volume: gentle bass and treble shelves that grow as the player's
//! volume is turned down.
//!
//! # Why
//!
//! The ear's sensitivity to low and high frequencies falls faster than its sensitivity to
//! the midrange as the level drops (the equal-loudness contours of ISO 226). Music turned
//! down therefore sounds thin. Between the 80- and 40-phon contours, 100 Hz needs roughly
//! 10 dB more than 1 kHz to sound as loud, and the top octave a few dB more.
//!
//! # The curve
//!
//! The player's volume is a linear amplitude factor (LMS's `audg`, as the engine applies
//! it). Its attenuation `att = −20·log10(volume)` dB drives one *amount*:
//!
//! ```text
//! amount = clamp(att / FULL_AT_DB, 0, 1)       (FULL_AT_DB = 40 dB)
//! bass   = amount × MAX_BASS_DB                 (+10 dB low shelf at 100 Hz)
//! treble = amount × MAX_TREBLE_DB               (+4 dB high shelf at 10 kHz)
//! ```
//!
//! So full volume is exactly 0 dB (the stage is bypassed), every 4 dB of attenuation adds
//! 1 dB of bass, and the boost stops growing at −40 dB. The boost is always well below the
//! attenuation that causes it (a quarter of it at most), so the shelves never lift the
//! signal above where it would have been at full volume; the chain's true-peak limiter
//! (always present with this stage) is the backstop.
//!
//! Non-finite volumes never produce a non-finite boost: NaN and +∞ mean no compensation,
//! zero or below (muted) the most.
//!
//! # Smoothness
//!
//! A volume change moves the target amount at once, but the amount the filters use
//! follows it at most [`RAMP_PER_S`] per second, with the shelves redesigned every
//! [`SUB_BLOCK_FRAMES`] frames. Each step is a few hundredths of a dB, so volume changes
//! never cause zipper noise.

use super::biquad::{Biquad, Coeffs, FilterKind, FilterSpec};
use super::signal_path::{Stage, fmt_db};

/// Low shelf corner, Hz.
pub const BASS_HZ: f64 = 100.0;
/// High shelf corner, Hz.
pub const TREBLE_HZ: f64 = 10_000.0;
/// Most bass boost, dB.
pub const MAX_BASS_DB: f64 = 10.0;
/// Most treble boost, dB.
pub const MAX_TREBLE_DB: f64 = 4.0;
/// Attenuation at which the compensation is complete, dB.
pub const FULL_AT_DB: f64 = 40.0;
/// Shelf slope (cookbook Q form; 1/√2 is the steepest without overshoot).
pub const SHELF_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// How fast the applied amount may move, in full ranges per second.
pub const RAMP_PER_S: f64 = 2.0;
/// Frames between coefficient updates while ramping.
pub const SUB_BLOCK_FRAMES: usize = 32;

/// The compensation for one volume, dB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compensation {
    pub bass_db: f64,
    pub treble_db: f64,
}

impl Compensation {
    pub const NONE: Compensation = Compensation {
        bass_db: 0.0,
        treble_db: 0.0,
    };

    fn of_amount(amount: f64) -> Compensation {
        Compensation {
            bass_db: amount * MAX_BASS_DB,
            treble_db: amount * MAX_TREBLE_DB,
        }
    }
}

/// How much of the full compensation applies at `volume` (linear), in 0..=1.
pub fn amount(volume: f64) -> f64 {
    if volume.is_nan() || volume >= 1.0 {
        return 0.0;
    }
    if volume <= 0.0 {
        return 1.0;
    }
    let att = -20.0 * volume.log10();
    (att / FULL_AT_DB).clamp(0.0, 1.0)
}

/// The bass and treble boost at `volume` (linear amplitude, 1.0 = full).
pub fn compensation(volume: f64) -> Compensation {
    Compensation::of_amount(amount(volume))
}

fn shelves(amount: f64, sample_rate: u32) -> (Coeffs, Coeffs) {
    if amount <= 0.0 {
        return (Coeffs::IDENTITY, Coeffs::IDENTITY);
    }
    let c = Compensation::of_amount(amount);
    let rate = f64::from(sample_rate);
    let spec = |kind, freq_hz, gain_db| FilterSpec {
        kind,
        freq_hz,
        gain_db,
        q: SHELF_Q,
    };
    (
        Coeffs::design(&spec(FilterKind::LowShelf, BASS_HZ, c.bass_db), rate),
        Coeffs::design(&spec(FilterKind::HighShelf, TREBLE_HZ, c.treble_db), rate),
    )
}

/// The loudness stage: two shelves whose gain follows the player's volume.
#[derive(Debug, Clone)]
pub struct Loudness {
    sample_rate: u32,
    channels: usize,
    /// The amount the volume asks for.
    target: f64,
    /// The amount the filters use now (ramps towards `target`).
    current: f64,
    bass: Biquad,
    treble: Biquad,
    /// The filters hold no state and the amount is zero: the buffer is left untouched.
    idle: bool,
    /// Frames processed with identity coefficients since the amount reached zero.
    quiet_frames: usize,
}

impl Loudness {
    /// A stage at full volume (bypassed until the volume drops).
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Loudness {
            sample_rate,
            channels,
            target: 0.0,
            current: 0.0,
            bass: Biquad::new(Coeffs::IDENTITY, channels),
            treble: Biquad::new(Coeffs::IDENTITY, channels),
            idle: true,
            quiet_frames: 0,
        }
    }

    /// The player's volume now (linear). The filters follow smoothly.
    pub fn set_volume(&mut self, volume: f64) {
        self.target = amount(volume);
        if self.target > 0.0 {
            self.idle = false;
        }
    }

    /// Jump straight to the target (no ramp): only where there is no audio to smooth, e.g.
    /// before the first block of a stream.
    pub fn settle(&mut self) {
        self.current = self.target;
        let (b, t) = shelves(self.current, self.sample_rate);
        self.bass.coeffs = b;
        self.treble.coeffs = t;
    }

    /// The compensation the volume asks for (what the stage settles on).
    pub fn target(&self) -> Compensation {
        Compensation::of_amount(self.target)
    }

    /// The compensation applied to the most recent sample.
    pub fn current(&self) -> Compensation {
        Compensation::of_amount(self.current)
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        self.reset();
        let (b, t) = shelves(self.current, sample_rate);
        self.bass.coeffs = b;
        self.treble.coeffs = t;
    }

    /// Clear filter state (after a seek or flush). The amount is kept.
    pub fn reset(&mut self) {
        self.bass.reset();
        self.treble.reset();
        self.idle = self.current == 0.0 && self.target == 0.0;
        self.quiet_frames = 0;
    }

    /// Leaves every sample untouched (full volume, nothing left in the filters).
    pub fn is_bypassed(&self) -> bool {
        self.idle
    }

    /// Filter interleaved frames in place. Allocates nothing.
    pub fn process(&mut self, buf: &mut [f64]) {
        if self.idle {
            return;
        }
        let step = RAMP_PER_S * SUB_BLOCK_FRAMES as f64 / f64::from(self.sample_rate.max(1));
        for chunk in buf.chunks_mut(SUB_BLOCK_FRAMES * self.channels) {
            if self.current != self.target {
                let d = self.target - self.current;
                self.current = if d.abs() <= step {
                    self.target
                } else {
                    self.current + step.copysign(d)
                };
                let (b, t) = shelves(self.current, self.sample_rate);
                self.bass.coeffs = b;
                self.treble.coeffs = t;
            }
            self.bass.process(chunk);
            self.treble.process(chunk);
            self.quiet_frames = if self.current == 0.0 {
                self.quiet_frames
                    .saturating_add(chunk.len() / self.channels)
            } else {
                0
            };
        }
        if self.target == 0.0 && self.quiet_frames >= 2 {
            // Identity coefficients for two frames have emptied the filters' state (it is
            // two samples deep), so the output already equals the input: from now on, a
            // true bypass.
            self.bass.reset();
            self.treble.reset();
            self.idle = true;
        }
    }

    /// The stage for the signal-path panel, from the target (so it doesn't flicker while
    /// ramping).
    pub fn stage(&self) -> Stage {
        let c = self.target();
        if self.target == 0.0 {
            Stage::transparent("Loudness (none at full volume)")
        } else {
            Stage::processing(format!(
                "Loudness {} bass, {} treble",
                fmt_db(c.bass_db),
                fmt_db(c.treble_db)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const ODD: [f64; 12] = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        -0.0,
        0.0,
        f64::MIN_POSITIVE,
        1e-300,
        1.0 + f64::EPSILON,
        2.0,
        f64::MAX,
        -f64::MAX,
    ];

    #[test]
    fn full_volume_is_exactly_zero() {
        assert_eq!(compensation(1.0), Compensation::NONE);
        assert_eq!(compensation(1.5), Compensation::NONE);
        assert_eq!(compensation(f64::INFINITY), Compensation::NONE);
        assert_eq!(compensation(f64::NAN), Compensation::NONE, "NaN: no boost");
    }

    #[test]
    fn the_curve_is_monotonic_in_volume() {
        let mut last = compensation(0.0);
        for i in 1..=10_000 {
            let v = f64::from(i) / 10_000.0;
            let c = compensation(v);
            assert!(
                c.bass_db <= last.bass_db && c.treble_db <= last.treble_db,
                "at {v}"
            );
            last = c;
        }
        assert_eq!(last, Compensation::NONE);
    }

    #[test]
    fn the_curve_is_bounded_and_finite_on_any_input() {
        for v in ODD {
            let c = compensation(v);
            assert!(c.bass_db.is_finite() && c.treble_db.is_finite(), "{v}");
            assert!((0.0..=MAX_BASS_DB).contains(&c.bass_db), "{v}");
            assert!((0.0..=MAX_TREBLE_DB).contains(&c.treble_db), "{v}");
        }
        // Muted and below: the most, never more.
        assert_eq!(compensation(0.0).bass_db, MAX_BASS_DB);
        assert_eq!(compensation(-3.0).treble_db, MAX_TREBLE_DB);
        assert_eq!(compensation(1e-9).bass_db, MAX_BASS_DB);
    }

    #[test]
    fn the_curve_follows_the_attenuation() {
        // −20 dB: half the compensation.
        let c = compensation(0.1);
        assert!((c.bass_db - 5.0).abs() < 1e-9 && (c.treble_db - 2.0).abs() < 1e-9);
        // The boost is always at most a quarter of the attenuation.
        for i in 1..1000 {
            let v = f64::from(i) / 1000.0;
            let att = -20.0 * v.log10();
            assert!(compensation(v).bass_db <= att / 4.0 + 1e-12);
        }
    }

    #[test]
    fn shelves_hit_their_targets() {
        for rate in [44_100, 48_000, 96_000, 192_000] {
            let (b, t) = shelves(1.0, rate);
            let r = f64::from(rate);
            assert!((b.response_db(20.0, r) - MAX_BASS_DB).abs() < 0.3);
            assert!(b.response_db(1000.0, r).abs() < 0.2);
            assert!(t.response_db(1000.0, r).abs() < 0.1);
            assert!((t.response_db(r * 0.45, r) - MAX_TREBLE_DB).abs() < 0.6);
            // Never above the shelf's gain anywhere (no overshoot in frequency).
            for f in [10.0, 50.0, 100.0, 500.0, 5000.0, 15_000.0] {
                assert!(b.response_db(f, r) <= MAX_BASS_DB + 1e-9);
                assert!(t.response_db(f, r) <= MAX_TREBLE_DB + 1e-9);
            }
        }
        assert_eq!(shelves(0.0, 48_000), (Coeffs::IDENTITY, Coeffs::IDENTITY));
    }

    #[test]
    fn full_volume_leaves_samples_untouched() {
        let mut l = Loudness::new(44_100, 2);
        l.set_volume(1.0);
        assert!(l.is_bypassed());
        let mut b = [0.25, -0.5, f64::MIN_POSITIVE, 1.0];
        let o = b;
        l.process(&mut b);
        assert_eq!(b, o);
        assert!(l.stage().bit_transparent);
    }

    #[test]
    fn turning_up_to_full_returns_to_a_true_bypass() {
        let mut l = Loudness::new(48_000, 2);
        l.set_volume(0.05);
        let mut buf = vec![0.1; 48_000];
        l.process(&mut buf);
        assert!(!l.is_bypassed());
        assert!(!l.stage().bit_transparent);
        l.set_volume(1.0);
        l.process(&mut buf);
        assert!(l.is_bypassed(), "ramped back down and emptied");
        let o = buf.clone();
        l.process(&mut buf);
        assert_eq!(buf, o);
    }

    #[test]
    fn volume_steps_are_smooth() {
        // DC through a low shelf comes out at the shelf's gain, so the output level is the
        // applied bass boost. A jump in volume must not jump the output.
        let rate = 44_100;
        let mut l = Loudness::new(rate, 1);
        let mut out = Vec::new();
        for v in [1.0, 0.01, 0.5, 0.001, 1.0] {
            l.set_volume(v);
            let mut block = vec![0.1; rate as usize / 2];
            l.process(&mut block);
            out.extend_from_slice(&block);
        }
        let worst = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max);
        // A 0.1 DC level boosted by up to 10 dB moves by much less than 1e-3 per sample.
        assert!(worst < 1e-3, "worst step {worst}");
        // And it does get there: −40 dB settles on the full boost.
        let mut l = Loudness::new(rate, 1);
        l.set_volume(0.01);
        let mut block = vec![0.1; rate as usize];
        l.process(&mut block);
        assert_eq!(l.current(), l.target());
        let db = 20.0 * (block[block.len() - 1] / 0.1).log10();
        assert!((db - MAX_BASS_DB).abs() < 0.05, "{db}");
    }

    #[test]
    fn coefficient_steps_are_small() {
        let rate = 44_100;
        let mut l = Loudness::new(rate, 2);
        l.set_volume(0.0);
        let mut last = l.current().bass_db;
        let mut block = vec![0.0; SUB_BLOCK_FRAMES * 2];
        while l.current() != l.target() {
            l.process(&mut block);
            let now = l.current().bass_db;
            assert!((now - last).abs() <= 0.02, "{last} → {now}");
            last = now;
        }
    }

    #[test]
    fn processing_stays_finite_on_any_volume_and_rate() {
        for v in ODD {
            for rate in [8_000, 44_100, 384_000] {
                let mut l = Loudness::new(rate, 2);
                l.set_volume(v);
                let mut buf: Vec<f64> = (0..8192)
                    .map(|i| (2.0 * PI * 60.0 * f64::from(i) / f64::from(rate)).sin())
                    .collect();
                l.process(&mut buf);
                assert!(buf.iter().all(|x| x.is_finite()), "{v} at {rate}");
            }
        }
    }

    #[test]
    fn rate_changes_and_resets_keep_working() {
        let mut l = Loudness::new(44_100, 2);
        l.set_volume(0.1);
        l.settle();
        assert_eq!(l.current(), l.target());
        l.set_sample_rate(96_000);
        l.reset();
        let mut b = vec![0.5; 4096];
        l.process(&mut b);
        assert!(b.iter().all(|x| x.is_finite()));
        assert!(!l.is_bypassed());
    }
}
