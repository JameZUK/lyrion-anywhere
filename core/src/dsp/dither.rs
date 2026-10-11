//! Requantisation to integer output with TPDF dither and optional noise shaping.
//!
//! Dither is added **only when the bit depth is reduced**: when the output is integer and
//! either narrower than the source or fed by an active processing stage (whose `f64`
//! result is deeper than any integer output). It is never applied to float output, nor
//! when an integer source reaches an output of the same or greater depth untouched; that
//! path stays bit-perfect. [`needs_dither`] encodes the rule.
//!
//! TPDF (triangular) dither is the sum of two independent uniform values of ±½ LSB, which
//! makes the quantisation error's mean and variance independent of the signal. Optional
//! first-order noise shaping feeds the previous sample's total error back with a negative
//! sign, which moves the noise towards high frequencies (`1 - z⁻¹`), where hearing is less
//! sensitive. The random generator is a seeded xorshift, so output is deterministic.

use super::pcm::{int_range, int_scale, quantise_one};

/// Whether requantising to `output_bits` needs dither.
///
/// - `output_bits`: `None` for float output.
/// - `source_bits`: the source's integer depth, `None` for a float or lossy source (whose
///   decoded samples are deeper than any integer output).
/// - `processed`: some stage changed the samples.
pub fn needs_dither(output_bits: Option<u32>, source_bits: Option<u32>, processed: bool) -> bool {
    match (output_bits, source_bits) {
        (None, _) => false,
        (Some(_), _) if processed => true,
        (Some(out), Some(src)) => out < src,
        (Some(_), None) => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum DitherMode {
    /// Plain TPDF.
    #[default]
    Tpdf,
    /// TPDF with first-order noise shaping.
    TpdfShaped,
}

#[derive(Debug, Clone)]
pub struct Dither {
    mode: DitherMode,
    rng: u64,
    err: Vec<f64>,
}

impl Dither {
    pub fn new(mode: DitherMode, channels: usize) -> Self {
        Dither {
            mode,
            rng: 0x9e37_79b9_7f4a_7c15,
            err: vec![0.0; channels.max(1)],
        }
    }

    pub fn mode(&self) -> DitherMode {
        self.mode
    }

    #[inline]
    fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        // 53 random bits → [0, 1).
        (self.rng >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    pub fn reset(&mut self) {
        self.err.fill(0.0);
    }

    /// Quantise interleaved `src` to `bits`-deep integers with dither.
    pub fn quantise(&mut self, src: &[f64], bits: u32, dst: &mut [i32]) {
        let s = int_scale(bits);
        let (lo, hi) = int_range(bits);
        let ch = self.err.len();
        for (i, (d, x)) in dst.iter_mut().zip(src).enumerate() {
            let x = super::sanitise(*x) * s;
            let tpdf = self.uniform() + self.uniform() - 1.0;
            match self.mode {
                DitherMode::Tpdf => *d = quantise_one(x + tpdf, lo, hi),
                DitherMode::TpdfShaped => {
                    let c = i % ch;
                    let v = x - self.err[c];
                    let q = quantise_one(v + tpdf, lo, hi);
                    // Bound the fed-back error so clipping can't make the loop run away.
                    self.err[c] = (f64::from(q) - v).clamp(-2.0, 2.0);
                    *d = q;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule() {
        assert!(!needs_dither(None, Some(24), true), "float output");
        assert!(
            !needs_dither(Some(24), Some(16), false),
            "widened, untouched"
        );
        assert!(
            !needs_dither(Some(16), Some(16), false),
            "same depth, untouched"
        );
        assert!(needs_dither(Some(16), Some(24), false), "narrowed");
        assert!(needs_dither(Some(24), Some(24), true), "processed");
        assert!(
            needs_dither(Some(16), None, false),
            "float/lossy source to integer"
        );
    }

    #[test]
    fn tpdf_error_is_bounded_and_unbiased() {
        for mode in [DitherMode::Tpdf, DitherMode::TpdfShaped] {
            let mut d = Dither::new(mode, 1);
            let n = 100_000;
            let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.001).sin() * 0.25).collect();
            let mut q = vec![0; n];
            d.quantise(&x, 16, &mut q);
            let mut sum = 0.0;
            for (a, b) in x.iter().zip(&q) {
                let e = f64::from(*b) - a * 32768.0;
                assert!(e.abs() <= 3.0, "{mode:?} {e}");
                sum += e;
            }
            assert!((sum / n as f64).abs() < 0.02, "{mode:?} bias");
        }
    }

    #[test]
    fn dither_decorrelates_a_tiny_signal() {
        // A sine of 0.3 LSB vanishes without dither, but survives (on average) with it.
        let mut d = Dither::new(DitherMode::Tpdf, 1);
        let n = 200_000;
        let x: Vec<f64> = (0..n)
            .map(|i| 0.3 / 32768.0 * (2.0 * std::f64::consts::PI * i as f64 / 100.0).sin())
            .collect();
        let mut q = vec![0; n];
        d.quantise(&x, 16, &mut q);
        let corr: f64 = x
            .iter()
            .zip(&q)
            .map(|(a, b)| a * 32768.0 * f64::from(*b))
            .sum();
        let energy: f64 = x.iter().map(|a| (a * 32768.0).powi(2)).sum();
        assert!((corr / energy - 1.0).abs() < 0.1, "{}", corr / energy);
        let mut plain = vec![0; n];
        super::super::pcm::from_f64(&x, 16, &mut plain);
        assert!(plain.iter().all(|v| *v == 0));
    }
}
