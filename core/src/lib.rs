//! Lyrion Anywhere player core: Slimproto protocol, stream buffering, decoding, the audio
//! engine's signal processing ([`dsp`]) and track analysis ([`analysis`]).
//!
//! The core performs no I/O. A host (native CLI, WebAssembly, mobile app) feeds it control
//! and stream bytes, executes the [`player::Action`]s it emits, and pulls PCM for output.

pub mod analysis;
#[cfg(feature = "player")]
pub mod decode;
pub mod dsp;
pub mod lyrics;
pub mod mux;
pub mod names;
#[cfg(feature = "opus")]
pub mod opus;
#[cfg(feature = "player")]
pub mod player;
pub mod privnet;
pub mod proto;
pub mod quality;
#[cfg(test)]
mod test_timing;
pub mod visual;

/// The lowest sample rate the core plays, processes or analyses (the lowest MP3 rate).
pub const MIN_SAMPLE_RATE: u32 = 8_000;
/// The highest sample rate the core plays, processes or analyses.
pub const MAX_SAMPLE_RATE: u32 = 768_000;

/// Whether the core handles audio at `rate` Hz: [`MIN_SAMPLE_RATE`] to [`MAX_SAMPLE_RATE`].
/// A rate a stream declares outside that is a broken or hostile header.
pub const fn sample_rate_supported(rate: u32) -> bool {
    MIN_SAMPLE_RATE <= rate && rate <= MAX_SAMPLE_RATE
}
