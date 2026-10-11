//! K-weighting (ITU-R BS.1770-4 §2.1) at any sample rate.
//!
//! The Recommendation publishes the two K-weighting biquads (a high-shelf "pre-filter"
//! modelling the head, and the "RLB" high-pass) only at 48 kHz. To use them at another
//! rate, each 48 kHz biquad is mapped back through the bilinear transform to its
//! continuous-time prototype and re-discretised at the new rate, pre-warped at the
//! filter's natural frequency so that frequency lands in the same place. At 48 kHz the
//! result is the published filter exactly.
//!
//! In terms of `s' = (1 - z⁻¹)/(1 + z⁻¹)`, a biquad `b0 + b1 z⁻¹ + b2 z⁻²` becomes
//! `n2 s'² + n1 s' + n0` with `n0 = (b0+b1+b2)/4`, `n1 = (b0-b2)/2`, `n2 = (b0-b1+b2)/4`.
//! Re-targeting from rate `f0` to `f` while keeping frequency `fp` fixed substitutes
//! `s' → λ s'` with `λ = tan(π fp / f0) / tan(π fp / f)`.

use crate::dsp::biquad::Coeffs;
use std::f64::consts::PI;

/// The BS.1770-4 pre-filter at 48 kHz: (b0, b1, b2, a1, a2).
pub const PRE_FILTER_48K: [f64; 5] = [
    1.535_124_859_586_97,
    -2.691_696_189_406_38,
    1.198_392_810_852_85,
    -1.690_659_293_182_41,
    0.732_480_774_215_85,
];

/// The BS.1770-4 RLB high-pass at 48 kHz: (b0, b1, b2, a1, a2).
pub const RLB_FILTER_48K: [f64; 5] = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];

/// Re-target a biquad designed at `from` Hz to `to` Hz (see the module docs).
pub fn retarget(c: [f64; 5], from: f64, to: f64) -> Coeffs {
    let [b0, b1, b2, a1, a2] = c;
    let s_poly =
        |p0: f64, p1: f64, p2: f64| [(p0 + p1 + p2) / 4.0, (p0 - p2) / 2.0, (p0 - p1 + p2) / 4.0];
    let n = s_poly(b0, b1, b2);
    let d = s_poly(1.0, a1, a2);
    // Natural frequency of the poles, in the s' (tan) domain, back to Hz at `from`.
    let w = (d[0] / d[2]).abs().sqrt();
    let fp = from / PI * w.atan();
    let lambda = (PI * fp / from).tan() / (PI * fp / to).tan();
    let scale = |p: [f64; 3]| [p[0], p[1] * lambda, p[2] * lambda * lambda];
    let (n, d) = (scale(n), scale(d));
    let z = |p: [f64; 3]| [p[2] + p[1] + p[0], 2.0 * (p[0] - p[2]), p[2] - p[1] + p[0]];
    let (bn, an) = (z(n), z(d));
    Coeffs {
        b0: bn[0] / an[0],
        b1: bn[1] / an[0],
        b2: bn[2] / an[0],
        a1: an[1] / an[0],
        a2: an[2] / an[0],
    }
}

/// Both K-weighting stages at `sample_rate`.
pub fn k_weighting(sample_rate: u32) -> [Coeffs; 2] {
    let fs = f64::from(sample_rate);
    [
        retarget(PRE_FILTER_48K, 48_000.0, fs),
        retarget(RLB_FILTER_48K, 48_000.0, fs),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_at_48k() {
        let [pre, rlb] = k_weighting(48_000);
        let c = PRE_FILTER_48K;
        for (a, b) in [pre.b0, pre.b1, pre.b2, pre.a1, pre.a2].iter().zip(c) {
            assert!((a - b).abs() < 1e-12);
        }
        let c = RLB_FILTER_48K;
        for (a, b) in [rlb.b0, rlb.b1, rlb.b2, rlb.a1, rlb.a2].iter().zip(c) {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn response_is_rate_independent() {
        let k = |rate: u32, f: f64| {
            let [a, b] = k_weighting(rate);
            a.response_db(f, f64::from(rate)) + b.response_db(f, f64::from(rate))
        };
        for rate in [32_000, 44_100, 88_200, 96_000, 176_400, 192_000, 384_000] {
            for f in [
                20.0, 50.0, 100.0, 500.0, 997.0, 1500.0, 3000.0, 6000.0, 10_000.0,
            ] {
                let d = (k(rate, f) - k(48_000, f)).abs();
                let tol = if f > 6000.0 { 0.1 } else { 0.03 };
                assert!(d < tol, "{rate} Hz at {f} Hz: {d} dB");
            }
        }
        // The familiar shape: +0.69 dB at 997 Hz (hence BS.1770's -0.691 constant), a
        // +4 dB high shelf, and the low cut.
        assert!(
            (k(48_000, 997.0) - 0.691).abs() < 0.01,
            "{}",
            k(48_000, 997.0)
        );
        assert!((k(48_000, 10_000.0) - 4.0).abs() < 0.2);
        assert!(k(48_000, 20.0) < -10.0);
    }
}
