//! Lyrion Anywhere player core: Slimproto protocol, stream buffering and decoding.
//!
//! The core performs no I/O. A host (native CLI, WebAssembly, mobile app) feeds it control
//! and stream bytes, executes the [`player::Action`]s it emits, and pulls PCM for output.

#[cfg(feature = "player")]
pub mod decode;
pub mod mux;
#[cfg(feature = "player")]
pub mod player;
pub mod proto;
