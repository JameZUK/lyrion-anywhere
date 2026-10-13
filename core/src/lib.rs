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
#[cfg(feature = "opus")]
pub mod opus;
#[cfg(feature = "player")]
pub mod player;
pub mod proto;
pub mod visual;
