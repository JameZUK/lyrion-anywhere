//! Conversion between integer or `f32` PCM and the engine's `f64` samples.
//!
//! Integer samples are right-aligned in an `i32` (a 16-bit sample lies in -32768..=32767)
//! and scale to ±1.0 by 2^-(bits-1). Both directions are exact for every value an integer
//! of up to 32 bits can hold (see the [module docs](super) for why), so integer → `f64` →
//! integer is the identity.

/// Lowest and highest supported integer bit depths.
pub const MIN_BITS: u32 = 8;
pub const MAX_BITS: u32 = 32;

/// The scale from a `bits`-deep integer to ±1.0, as an exact power of two.
#[inline]
fn scale(bits: u32) -> f64 {
    // 2^(bits-1), built from the exponent so it is exact for every depth.
    f64::from_bits((1023 + u64::from(bits) - 1) << 52)
}

fn check_bits(bits: u32) {
    assert!(
        (MIN_BITS..=MAX_BITS).contains(&bits),
        "unsupported bit depth {bits}"
    );
}

/// Integers of depth `bits` to `f64`. `dst` must be at least as long as `src`.
pub fn to_f64(src: &[i32], bits: u32, dst: &mut [f64]) {
    check_bits(bits);
    let inv = 1.0 / scale(bits);
    for (d, s) in dst.iter_mut().zip(src) {
        *d = f64::from(*s) * inv;
    }
}

/// The range of a `bits`-deep signed integer.
#[inline]
pub fn int_range(bits: u32) -> (f64, f64) {
    let s = scale(bits);
    (-s, s - 1.0)
}

/// `f64` to integers of depth `bits`, rounding to nearest and clamping to the integer
/// range (NaN becomes 0). No dither: use [`super::dither::Dither`] for that. Exact for any
/// value that came from [`to_f64`] at the same depth.
pub fn from_f64(src: &[f64], bits: u32, dst: &mut [i32]) {
    check_bits(bits);
    let s = scale(bits);
    let (lo, hi) = int_range(bits);
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

/// The scale factor from ±1.0 to `bits`-deep integers.
#[inline]
pub(crate) fn int_scale(bits: u32) -> f64 {
    check_bits(bits);
    scale(bits)
}

/// Significant bits an `f32` carries exactly: integer PCM up to this deep survives
/// integer → `f32` → integer unchanged (24-bit mantissa, power-of-two scale). The engine's
/// ring and the core's decoded PCM are `f32`, so this is the deepest source an integer
/// output can deliver bit-perfect.
pub const F32_EXACT_BITS: u32 = 24;

/// One `f32` sample (±1.0 full scale) to a `bits`-deep integer, right-aligned, rounding to
/// nearest and clamping (NaN becomes 0). The identity on the integer grid: a sample that
/// came from an integer of at most `min(bits, F32_EXACT_BITS)` bits converts back exactly
/// (shifted left when `bits` is deeper, which is lossless padding). Real-time safe.
#[inline]
pub fn f32_to_int(x: f32, bits: u32) -> i32 {
    let s = int_scale(bits);
    let (lo, hi) = int_range(bits);
    quantise_one(f64::from(x) * s, lo, hi)
}

/// `bits`-deep integers (`bits` ≤ [`F32_EXACT_BITS`]) to `f32`, exactly.
pub fn int_to_f32(src: &[i32], bits: u32, dst: &mut [f32]) {
    assert!(
        (MIN_BITS..=F32_EXACT_BITS).contains(&bits),
        "f32 can't hold {bits}-bit integers exactly"
    );
    let inv = 1.0 / scale(bits);
    for (d, s) in dst.iter_mut().zip(src) {
        *d = (f64::from(*s) * inv) as f32;
    }
}

/// Is `x` exactly on the `bits`-deep integer grid (so [`f32_to_int`] is lossless)?
#[inline]
pub fn f32_on_grid(x: f32, bits: u32) -> bool {
    let v = f64::from(x) * int_scale(bits);
    let (lo, hi) = int_range(bits);
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

    #[test]
    fn scale_is_exact_power_of_two() {
        assert_eq!(scale(16), 32768.0);
        assert_eq!(scale(24), 8_388_608.0);
        assert_eq!(scale(32), 2_147_483_648.0);
        assert_eq!(scale(8), 128.0);
    }

    #[test]
    fn extremes_round_trip() {
        for bits in [8, 16, 20, 24, 32] {
            let (lo, hi) = int_range(bits);
            let v = [lo as i32, hi as i32, 0, 1, -1, lo as i32 + 1, hi as i32 - 1];
            let mut f = [0.0; 7];
            let mut back = [0; 7];
            to_f64(&v, bits, &mut f);
            assert!(f.iter().all(|x| (-1.0..1.0).contains(x)));
            from_f64(&f, bits, &mut back);
            assert_eq!(v, back, "{bits}-bit");
        }
    }

    #[test]
    fn clamps_and_handles_nan() {
        let mut out = [0; 5];
        from_f64(
            &[2.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
            16,
            &mut out,
        );
        assert_eq!(out, [32767, -32768, 0, 32767, -32768]);
        from_f64(&[1.0, -1.0, 0.0, 0.0, 0.0], 32, &mut out);
        assert_eq!(out[..2], [i32::MAX, i32::MIN]);
    }

    #[test]
    #[should_panic(expected = "unsupported bit depth")]
    fn rejects_silly_depths() {
        to_f64(&[0], 33, &mut [0.0]);
    }
}
