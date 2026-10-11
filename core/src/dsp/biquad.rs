//! Second-order IIR sections (biquads) in transposed direct form II, `f64` throughout.
//!
//! Designs follow Robert Bristow-Johnson's *Audio EQ Cookbook* formulas (peaking, low and
//! high shelf, low and high pass, notch). Filters are described by [`FilterSpec`] and
//! recomputed whenever the sample rate changes, so a profile means the same thing at any
//! rate.

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    Notch,
}

/// One filter of an equaliser profile.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilterSpec {
    pub kind: FilterKind,
    /// Centre or corner frequency, Hz.
    pub freq_hz: f64,
    /// Gain, dB (peaking and shelves only).
    pub gain_db: f64,
    /// Quality factor (for shelves, the cookbook's Q form of the slope).
    pub q: f64,
}

/// Normalised biquad coefficients (a0 = 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalised(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Coeffs {
        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// The cookbook design of `spec` at `sample_rate`. A filter at or above Nyquist, or
    /// with non-finite parameters, is the identity (it can't act on this signal).
    pub fn design(spec: &FilterSpec, sample_rate: f64) -> Coeffs {
        let FilterSpec {
            kind,
            freq_hz,
            gain_db,
            q,
        } = *spec;
        if !(freq_hz.is_finite() && gain_db.is_finite() && q.is_finite())
            || freq_hz <= 0.0
            || q <= 0.0
            || sample_rate <= 0.0
            || freq_hz >= sample_rate * 0.5
        {
            return Coeffs::IDENTITY;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq_hz / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        match kind {
            FilterKind::Peaking => Coeffs::normalised(
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterKind::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                Coeffs::normalised(
                    a * ((a + 1.0) - (a - 1.0) * cos + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - k),
                    (a + 1.0) + (a - 1.0) * cos + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - k,
                )
            }
            FilterKind::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                Coeffs::normalised(
                    a * ((a + 1.0) + (a - 1.0) * cos + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - k),
                    (a + 1.0) - (a - 1.0) * cos + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - k,
                )
            }
            FilterKind::LowPass => Coeffs::normalised(
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::HighPass => Coeffs::normalised(
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::Notch => {
                Coeffs::normalised(1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
        }
    }

    /// Magnitude response at `freq_hz`, dB.
    pub fn response_db(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq_hz / sample_rate;
        // H(e^jw) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = self.b1 * s1 + self.b2 * s2;
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = self.a1 * s1 + self.a2 * s2;
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
    }
}

/// A biquad with independent state for each of `channels` interleaved channels.
#[derive(Debug, Clone)]
pub struct Biquad {
    pub coeffs: Coeffs,
    /// Per channel: (s1, s2).
    state: Vec<[f64; 2]>,
}

impl Biquad {
    pub fn new(coeffs: Coeffs, channels: usize) -> Self {
        Biquad {
            coeffs,
            state: vec![[0.0; 2]; channels.max(1)],
        }
    }

    pub fn channels(&self) -> usize {
        self.state.len()
    }

    pub fn reset(&mut self) {
        self.state.fill([0.0; 2]);
    }

    /// One sample of channel `ch` (transposed direct form II).
    #[inline]
    pub fn tick(&mut self, ch: usize, x: f64) -> f64 {
        let c = &self.coeffs;
        let s = &mut self.state[ch];
        let y = c.b0 * x + s[0];
        s[0] = c.b1 * x - c.a1 * y + s[1];
        s[1] = c.b2 * x - c.a2 * y;
        y
    }

    /// Filter interleaved samples in place.
    pub fn process(&mut self, buf: &mut [f64]) {
        let n = self.state.len();
        for frame in buf.chunks_exact_mut(n) {
            for (ch, s) in frame.iter_mut().enumerate() {
                *s = self.tick(ch, *s);
            }
        }
        for s in &mut self.state {
            s[0] = super::flush_denormal(s[0]);
            s[1] = super::flush_denormal(s[1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: FilterKind, f: f64, g: f64, q: f64) -> FilterSpec {
        FilterSpec {
            kind,
            freq_hz: f,
            gain_db: g,
            q,
        }
    }

    #[test]
    fn responses_hit_their_targets() {
        for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let pk = Coeffs::design(&spec(FilterKind::Peaking, 1000.0, 6.0, 1.0), rate);
            assert!((pk.response_db(1000.0, rate) - 6.0).abs() < 1e-9);
            assert!(pk.response_db(20.0, rate).abs() < 0.05);
            let ls = Coeffs::design(&spec(FilterKind::LowShelf, 200.0, -4.0, 0.707), rate);
            assert!((ls.response_db(10.0, rate) + 4.0).abs() < 0.05);
            assert!((ls.response_db(200.0, rate) + 2.0).abs() < 1e-6);
            assert!(ls.response_db(10_000.0, rate).abs() < 0.05);
            let hs = Coeffs::design(&spec(FilterKind::HighShelf, 5000.0, 3.0, 0.707), rate);
            assert!((hs.response_db(5000.0, rate) - 1.5).abs() < 1e-6);
            assert!(hs.response_db(50.0, rate).abs() < 0.01);
            let lp = Coeffs::design(&spec(FilterKind::LowPass, 1000.0, 0.0, 0.707), rate);
            assert!((lp.response_db(1000.0, rate) + 3.0103).abs() < 0.01);
            assert!(lp.response_db(10.0, rate).abs() < 0.01);
            let hp = Coeffs::design(&spec(FilterKind::HighPass, 1000.0, 0.0, 0.707), rate);
            assert!((hp.response_db(1000.0, rate) + 3.0103).abs() < 0.01);
            let no = Coeffs::design(&spec(FilterKind::Notch, 1000.0, 0.0, 5.0), rate);
            assert!(no.response_db(1000.0, rate) < -100.0);
        }
    }

    #[test]
    fn degenerate_specs_are_identity() {
        let bad = [
            spec(FilterKind::Peaking, 30_000.0, 6.0, 1.0),
            spec(FilterKind::Peaking, f64::NAN, 6.0, 1.0),
            spec(FilterKind::Peaking, 1000.0, f64::INFINITY, 1.0),
            spec(FilterKind::LowPass, 1000.0, 0.0, 0.0),
            spec(FilterKind::LowPass, -5.0, 0.0, 1.0),
        ];
        for s in bad {
            assert_eq!(Coeffs::design(&s, 44_100.0), Coeffs::IDENTITY);
        }
    }

    #[test]
    fn filtering_matches_the_response() {
        let rate = 48_000.0;
        let c = Coeffs::design(&spec(FilterKind::Peaking, 3000.0, -9.0, 2.0), rate);
        let mut bq = Biquad::new(c, 1);
        let mut x: Vec<f64> = (0..48_000)
            .map(|i| (2.0 * PI * 3000.0 * i as f64 / rate).sin())
            .collect();
        bq.process(&mut x);
        let tail = &x[24_000..];
        let peak = tail.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!((20.0 * peak.log10() + 9.0).abs() < 0.01);
    }
}
