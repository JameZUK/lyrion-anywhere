//! Gated loudness (ITU-R BS.1770-4 §2.8, EBU R128) and loudness range (EBU Tech 3342), in
//! constant memory.
//!
//! K-weighted, channel-weighted power is summed into 100 ms sub-blocks. A gating block is
//! four sub-blocks (400 ms, 75 % overlap, one new block every 100 ms); a short-term block
//! is thirty (3 s, also every 100 ms).
//!
//! Gating needs the whole track's blocks, but only through their loudness distribution,
//! so blocks go into a histogram of 0.01 LU bins from -70 to +10 LUFS. Each bin keeps the
//! count and the *exact* summed power of its blocks, so the integrated loudness is exact
//! except for blocks within 0.01 LU of the relative gate (whose bin is gated as a whole).
//! The short-term histogram, used for the loudness range, works the same way.

/// Absolute gate, LUFS.
pub const ABSOLUTE_GATE: f64 = -70.0;
/// Relative gate for integrated loudness, LU.
pub const RELATIVE_GATE: f64 = -10.0;
/// Relative gate for loudness range, LU.
pub const LRA_RELATIVE_GATE: f64 = -20.0;

const BIN_LU: f64 = 0.01;
const HIST_MIN: f64 = ABSOLUTE_GATE;
const HIST_MAX: f64 = 10.0;
const BINS: usize = ((HIST_MAX - HIST_MIN) / BIN_LU) as usize;

/// Loudness of a mean-square power.
#[inline]
pub fn power_to_lufs(p: f64) -> f64 {
    -0.691 + 10.0 * p.log10()
}

/// Mean-square power of a loudness.
#[inline]
pub fn lufs_to_power(l: f64) -> f64 {
    10f64.powf((l + 0.691) / 10.0)
}

/// Histogram of block loudness above the absolute gate.
#[derive(Debug, Clone)]
pub struct GatingHistogram {
    count: Vec<u64>,
    power: Vec<f64>,
    total: u64,
}

impl Default for GatingHistogram {
    fn default() -> Self {
        GatingHistogram {
            count: vec![0; BINS],
            power: vec![0.0; BINS],
            total: 0,
        }
    }
}

impl GatingHistogram {
    /// Add a block of mean-square power `p` (ignored if below the absolute gate).
    pub fn add(&mut self, p: f64) {
        let l = power_to_lufs(p);
        // Also refuses NaN.
        if l.is_nan() || l <= ABSOLUTE_GATE {
            return;
        }
        let i = (((l - HIST_MIN) / BIN_LU) as usize).min(BINS - 1);
        self.count[i] += 1;
        self.power[i] += p;
        self.total += 1;
    }

    pub fn blocks(&self) -> u64 {
        self.total
    }

    /// Lower edge of bin `i`, LUFS.
    fn bin_lufs(i: usize) -> f64 {
        HIST_MIN + i as f64 * BIN_LU
    }

    /// First bin whose blocks pass a relative gate of `gate` LU below the mean of all
    /// (absolute-gated) blocks.
    fn relative_start(&self, gate: f64) -> Option<usize> {
        if self.total == 0 {
            return None;
        }
        let mean = self.power.iter().sum::<f64>() / self.total as f64;
        let threshold = power_to_lufs(mean) + gate;
        // A bin is in if its centre is above the threshold.
        let start = ((threshold - HIST_MIN) / BIN_LU - 0.5).ceil().max(0.0) as usize;
        Some(start.min(BINS))
    }

    /// Integrated (gated) loudness, LUFS: `None` when no block passes the gates.
    pub fn integrated(&self) -> Option<f64> {
        let start = self.relative_start(RELATIVE_GATE)?;
        let n: u64 = self.count[start..].iter().sum();
        if n == 0 {
            return None;
        }
        let p: f64 = self.power[start..].iter().sum();
        Some(power_to_lufs(p / n as f64))
    }

    /// Loudness range per EBU Tech 3342 (this histogram must hold short-term blocks):
    /// the spread between the 10th and 95th percentiles after a -20 LU relative gate.
    pub fn loudness_range(&self) -> Option<f64> {
        let start = self.relative_start(LRA_RELATIVE_GATE)?;
        let n: u64 = self.count[start..].iter().sum();
        if n == 0 {
            return None;
        }
        let percentile = |q: f64| {
            // Nearest-rank on the sorted values.
            let rank = ((q * (n - 1) as f64).round() as u64).min(n - 1);
            let mut seen = 0;
            for i in start..BINS {
                seen += self.count[i];
                if seen > rank {
                    return Self::bin_lufs(i) + BIN_LU / 2.0;
                }
            }
            Self::bin_lufs(BINS - 1)
        };
        Some((percentile(0.95) - percentile(0.10)).max(0.0))
    }

    /// Merge another histogram (e.g. to measure an album's tracks together).
    pub fn merge(&mut self, other: &GatingHistogram) {
        for i in 0..BINS {
            self.count[i] += other.count[i];
            self.power[i] += other.power[i];
        }
        self.total += other.total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_level_integrates_to_itself() {
        let mut h = GatingHistogram::default();
        for _ in 0..1000 {
            h.add(lufs_to_power(-23.0));
        }
        assert!((h.integrated().unwrap() + 23.0).abs() < 1e-9);
        assert_eq!(h.loudness_range().unwrap(), 0.0);
    }

    #[test]
    fn gates() {
        let mut h = GatingHistogram::default();
        // Silence and very quiet blocks are gated out.
        h.add(0.0);
        h.add(lufs_to_power(-80.0));
        assert_eq!(h.blocks(), 0);
        assert!(h.integrated().is_none());
        // A loud section and a much quieter one: the quiet one is relatively gated.
        for _ in 0..100 {
            h.add(lufs_to_power(-20.0));
            h.add(lufs_to_power(-40.0));
        }
        assert!((h.integrated().unwrap() + 20.0).abs() < 1e-9);
        h.add(f64::NAN);
        assert_eq!(h.blocks(), 200);
    }

    #[test]
    fn range_of_two_levels() {
        let mut h = GatingHistogram::default();
        for _ in 0..500 {
            h.add(lufs_to_power(-20.0));
            h.add(lufs_to_power(-30.0));
        }
        assert!((h.loudness_range().unwrap() - 10.0).abs() < 0.02);
    }
}
