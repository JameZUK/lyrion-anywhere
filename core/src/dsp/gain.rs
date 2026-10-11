//! A plain gain stage (preamp, levelling). Exactly 0 dB is a true bypass: the buffer is
//! not touched, so it is bit-transparent.

use super::db_to_gain;

/// Most gain accepted, in either direction. Anything beyond is a configuration mistake.
pub const MAX_GAIN_DB: f64 = 60.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Gain {
    db: f64,
    linear: f64,
}

impl Default for Gain {
    fn default() -> Self {
        Gain {
            db: 0.0,
            linear: 1.0,
        }
    }
}

impl Gain {
    /// A gain of `db` decibels, clamped to ±[`MAX_GAIN_DB`]; NaN means 0 dB.
    pub fn new(db: f64) -> Self {
        let db = if db.is_nan() {
            0.0
        } else {
            db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
        };
        Gain {
            db,
            linear: db_to_gain(db),
        }
    }

    pub fn db(&self) -> f64 {
        self.db
    }

    pub fn is_bypassed(&self) -> bool {
        self.db == 0.0
    }

    pub fn process(&self, buf: &mut [f64]) {
        if self.is_bypassed() {
            return;
        }
        for s in buf {
            *s *= self.linear;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_untouched() {
        let mut b = [0.1, -0.3, f64::MIN_POSITIVE, 0.999_999_9];
        let orig = b;
        Gain::new(0.0).process(&mut b);
        assert_eq!(b.map(f64::to_bits), orig.map(f64::to_bits));
        assert!(Gain::new(f64::NAN).is_bypassed());
    }

    #[test]
    fn applies_and_clamps() {
        let mut b = [0.5];
        Gain::new(-6.020_599_913_279_624).process(&mut b);
        assert!((b[0] - 0.25).abs() < 1e-12);
        assert_eq!(Gain::new(1e9).db(), MAX_GAIN_DB);
        assert_eq!(Gain::new(f64::NEG_INFINITY).db(), -MAX_GAIN_DB);
    }
}
