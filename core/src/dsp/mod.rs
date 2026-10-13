//! The audio engine's signal processing: pure, deterministic and free of I/O.
//!
//! Everything here works on **interleaved `f64` frames** with any channel count (crossfeed
//! needs exactly two). The host converts decoded PCM in with [`pcm`], runs a [`Chain`], and
//! converts out to the output's format, again with [`pcm`].
//!
//! # Internal format and bit-perfect playback
//!
//! Samples are `f64` scaled to ±1.0. Converting an integer sample of up to 32 bits to `f64`
//! and back is **exact**: an `f64` has a 53-bit significand, so every `i32` is representable,
//! and the scale factor is a power of two (2^-(bits-1)), which only changes the exponent.
//! [`pcm::to_f64`] followed by [`pcm::from_f64`] therefore returns the identical integers,
//! which `core/tests/dsp_bitperfect.rs` proves at every common sample rate and depth.
//!
//! A bypassed stage leaves the buffer untouched (it does not multiply by 1.0 or add 0.0),
//! so a [`Chain`] with every stage bypassed is bit-transparent by construction, and the
//! output quantiser never dithers when the bit depth is preserved.
//!
//! # Stages, in processing order
//!
//! 1. [`gain::Gain`]: preamp plus loudness levelling ([`levelling`]).
//! 2. [`eq::ParametricEq`]: RBJ-cookbook biquads, with an AutoEq profile parser.
//! 3. [`loudness::Loudness`]: loudness-compensated volume, bass and treble shelves that
//!    grow as the player's volume drops (equal-loudness contours, ISO 226).
//! 4. [`crossfeed::Crossfeed`]: Bauer-style headphone crossfeed.
//! 5. [`limiter::TruePeakLimiter`]: look-ahead brickwall limiter on oversampled true peak.
//! 6. Output quantisation with [`dither::Dither`], only when the bit depth is reduced.
//!
//! The limiter runs after EQ, loudness and crossfeed so it catches every gain stage. Resampling
//! ([`resample`], behind the `resample` feature) and transitions ([`transition`]) sit
//! outside the chain, because they change the number of frames.
//!
//! [`signal_path::SignalPath`] describes the result for the app's signal-path panel.
//!
//! # Real-time behaviour
//!
//! All buffers are allocated when a stage is built or its sample rate changes. Processing
//! a block allocates nothing.

pub mod biquad;
pub mod chain;
pub mod crossfeed;
pub mod dither;
pub mod eq;
pub mod gain;
pub mod levelling;
pub mod limiter;
pub mod loudness;
pub mod pcm;
#[cfg(feature = "resample")]
pub mod resample;
pub mod signal_path;
pub mod transition;
pub mod truepeak;
mod window;

pub use chain::{Chain, ChainConfig};

/// Decibels to a linear amplitude factor.
#[inline]
pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// A linear amplitude factor to decibels (`-inf` for zero).
#[inline]
pub fn gain_to_db(gain: f64) -> f64 {
    20.0 * gain.abs().log10()
}

/// Replace NaN and infinities with silence, so one bad sample can't poison filter state.
#[inline]
pub(crate) fn sanitise(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

/// Flush values so small they are inaudible (below -600 dBFS) to zero. Recursive filters
/// otherwise decay into subnormal numbers, which are very slow on some CPUs.
#[inline]
pub(crate) fn flush_denormal(x: f64) -> f64 {
    if x.abs() < 1e-30 { 0.0 } else { x }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_round_trip() {
        for db in [-60.0, -6.0, -1.0, 0.0, 3.0, 12.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-12);
        }
        assert_eq!(db_to_gain(0.0), 1.0);
        assert!(gain_to_db(0.0).is_infinite());
    }

    #[test]
    fn sanitise_and_flush() {
        assert_eq!(sanitise(f64::NAN), 0.0);
        assert_eq!(sanitise(f64::INFINITY), 0.0);
        assert_eq!(sanitise(0.5), 0.5);
        assert_eq!(flush_denormal(1e-310), 0.0);
        assert_eq!(flush_denormal(1e-3), 1e-3);
    }
}
