//! Lyrion Anywhere player core: Slimproto protocol, stream buffering and decoding.
//!
//! The core performs no I/O. A host (native CLI, WebAssembly, mobile app) feeds it control
//! and stream bytes, executes the [`player::Action`]s it emits, and pulls PCM for output.

pub mod decode;
pub mod player;
pub mod proto;
