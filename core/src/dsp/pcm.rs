//! Conversion between integer or `f32` PCM and the engine's `f64` samples.
//!
//! Integer samples are right-aligned in an `i32` (a 16-bit sample lies in -32768..=32767)
//! and scale to ±1.0 by 2^-(bits-1). Both directions are exact for every value an integer
//! of up to 32 bits can hold (see the [module docs](super) for why), so integer → `f64` →
//! integer is the identity.
//!
//! A depth is a [`Depth`], which only [`Depth::new`] makes and which refuses anything but
//! [`MIN_BITS`] to [`MAX_BITS`]. No conversion can be handed a depth it can't do, so a
//! host learns of a bad one where it chooses it, as an error, never as a panic in an audio
//! thread.

use std::fmt;

/// Lowest and highest supported integer bit depths.
pub const MIN_BITS: u32 = 8;
pub const MAX_BITS: u32 = 32;

/// An integer sample depth the conversions support: [`MIN_BITS`] to [`MAX_BITS`] bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Depth(u32);

/// A bit depth the conversions don't support: outside [`MIN_BITS`] to [`MAX_BITS`], or, for
/// [`int_to_f32`], deeper than an `f32` holds exactly ([`F32_EXACT_BITS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedDepth(pub u32);

impl fmt::Display for UnsupportedDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported bit depth {}", self.0)
    }
}

impl std::error::Error for UnsupportedDepth {}

impl Depth {
    pub const D16: Depth = Depth(16);
    pub const D24: Depth = Depth(24);
    pub const D32: Depth = Depth(32);

    /// `bits` deep, if the conversions support it.
    pub const fn new(bits: u32) -> Result<Depth, UnsupportedDepth> {
        if MIN_BITS <= bits && bits <= MAX_BITS {
            Ok(Depth(bits))
        } else {
            Err(UnsupportedDepth(bits))
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The scale from a sample of this depth to ±1.0: 2^(bits-1), built from the exponent
    /// so it is exact for every depth.
    #[inline]
    fn scale(self) -> f64 {
        f64::from_bits((1023 + u64::from(self.0) - 1) << 52)
    }
}

impl TryFrom<u32> for Depth {
    type Error = UnsupportedDepth;

    fn try_from(bits: u32) -> Result<Depth, UnsupportedDepth> {
        Depth::new(bits)
    }
}

/// Integers of `depth` to `f64`. `dst` must be at least as long as `src`.
pub fn to_f64(src: &[i32], depth: Depth, dst: &mut [f64]) {
    let inv = 1.0 / depth.scale();
    for (d, s) in dst.iter_mut().zip(src) {
        *d = f64::from(*s) * inv;
    }
}

/// The range of a signed integer of `depth`.
#[inline]
pub fn int_range(depth: Depth) -> (f64, f64) {
    let s = depth.scale();
    (-s, s - 1.0)
}

/// `f64` to integers of `depth`, rounding to nearest and clamping to the integer range
/// (NaN becomes 0). No dither: use [`super::dither::Dither`] for that. Exact for any value
/// that came from [`to_f64`] at the same depth.
pub fn from_f64(src: &[f64], depth: Depth, dst: &mut [i32]) {
    let s = depth.scale();
    let (lo, hi) = int_range(depth);
    for (d, x) in dst.iter_mut().zip(src) {
        *d = quantise_one(*x * s, lo, hi);
    }
}

/// Round an already scaled value to the integer grid and clamp it.
#[inline]
pub(crate) fn quantise_one(v: f64, lo: f64, hi: f64) -> i32 {
    if v.is_nan() {
        return 0;
    }
    v.round().clamp(lo, hi) as i32
}

/// The scale factor from ±1.0 to integers of `depth`.
#[inline]
pub(crate) fn int_scale(depth: Depth) -> f64 {
    depth.scale()
}

/// Significant bits an `f32` carries exactly: integer PCM up to this deep survives
/// integer → `f32` → integer unchanged (24-bit mantissa, power-of-two scale). The engine's
/// ring and the core's decoded PCM are `f32`, so this is the deepest source an integer
/// output can deliver bit-perfect.
pub const F32_EXACT_BITS: u32 = 24;

/// One `f32` sample (±1.0 full scale) to an integer of `depth`, right-aligned, rounding to
/// nearest and clamping (NaN becomes 0). The identity on the integer grid: a sample that
/// came from an integer of at most `min(depth, F32_EXACT_BITS)` bits converts back exactly
/// (shifted left when `depth` is deeper, which is lossless padding). Real-time safe.
#[inline]
pub fn f32_to_int(x: f32, depth: Depth) -> i32 {
    let (lo, hi) = int_range(depth);
    quantise_one(f64::from(x) * depth.scale(), lo, hi)
}

/// Integers of `depth` to `f32`, exactly. A depth over [`F32_EXACT_BITS`] is refused (an
/// `f32` can't hold it exactly) and `dst` is left as it was.
pub fn int_to_f32(src: &[i32], depth: Depth, dst: &mut [f32]) -> Result<(), UnsupportedDepth> {
    if depth.bits() > F32_EXACT_BITS {
        return Err(UnsupportedDepth(depth.bits()));
    }
    let inv = 1.0 / depth.scale();
    for (d, s) in dst.iter_mut().zip(src) {
        *d = (f64::from(*s) * inv) as f32;
    }
    Ok(())
}

/// Is `x` exactly on the grid of `depth` (so [`f32_to_int`] is lossless)?
#[inline]
pub fn f32_on_grid(x: f32, depth: Depth) -> bool {
    let v = f64::from(x) * depth.scale();
    let (lo, hi) = int_range(depth);
    v.fract() == 0.0 && (lo..=hi).contains(&v)
}

/// `f32` to `f64` (exact).
pub fn f32_to_f64(src: &[f32], dst: &mut [f64]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = f64::from(*s);
    }
}

/// `f64` to `f32` (rounded to nearest; exact for values that came from `f32`).
pub fn f64_to_f32(src: &[f64], dst: &mut [f32]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = *s as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(bits: u32) -> Depth {
        Depth::new(bits).unwrap()
    }

    #[test]
    fn scale_is_exact_power_of_two() {
        assert_eq!(depth(16).scale(), 32768.0);
        assert_eq!(depth(24).scale(), 8_388_608.0);
        assert_eq!(depth(32).scale(), 2_147_483_648.0);
        assert_eq!(depth(8).scale(), 128.0);
    }

    #[test]
    fn extremes_round_trip() {
        for bits in [8, 16, 20, 24, 32] {
            let d = depth(bits);
            let (lo, hi) = int_range(d);
            let v = [lo as i32, hi as i32, 0, 1, -1, lo as i32 + 1, hi as i32 - 1];
            let mut f = [0.0; 7];
            let mut back = [0; 7];
            to_f64(&v, d, &mut f);
            assert!(f.iter().all(|x| (-1.0..1.0).contains(x)));
            from_f64(&f, d, &mut back);
            assert_eq!(v, back, "{bits}-bit");
        }
    }

    #[test]
    fn clamps_and_handles_nan() {
        let mut out = [0; 5];
        from_f64(
            &[2.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
            Depth::D16,
            &mut out,
        );
        assert_eq!(out, [32767, -32768, 0, 32767, -32768]);
        from_f64(&[1.0, -1.0, 0.0, 0.0, 0.0], Depth::D32, &mut out);
        assert_eq!(out[..2], [i32::MAX, i32::MIN]);
    }

    /// Stability (open since the 15 October review): an unsupported depth was an assertion,
    /// a panic in whichever thread converted audio (on Android, the audio thread). It is
    /// refused, as an error, where the depth is chosen.
    #[test]
    fn unsupported_depths_are_errors_not_panics() {
        for bits in [0, 1, 7, 33, 64, u32::MAX] {
            assert_eq!(Depth::new(bits), Err(UnsupportedDepth(bits)));
            assert_eq!(Depth::try_from(bits), Err(UnsupportedDepth(bits)));
        }
        assert_eq!(UnsupportedDepth(33).to_string(), "unsupported bit depth 33");
        for bits in MIN_BITS..=MAX_BITS {
            assert_eq!(Depth::new(bits).map(Depth::bits), Ok(bits));
        }
        assert_eq!(
            (Depth::D16.bits(), Depth::D24.bits(), Depth::D32.bits()),
            (16, 24, 32)
        );
        // An f32 can't hold more than 24 bits exactly: refused, and nothing written.
        let mut f = [7.0f32; 2];
        assert_eq!(
            int_to_f32(&[1, 2], Depth::D32, &mut f),
            Err(UnsupportedDepth(32))
        );
        assert_eq!(f, [7.0, 7.0]);
        assert_eq!(int_to_f32(&[1, -2], Depth::D24, &mut f), Ok(()));
        assert_eq!(f, [1.0 / 8_388_608.0, -2.0 / 8_388_608.0]);
    }
}
