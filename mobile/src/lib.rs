//! Lyrion Anywhere's native host layer, shared by the Android app (and later iOS).
//!
//! `core/` stays sans-IO; this crate does the I/O: HTTPS through rustls with the platform's
//! certificate verifier, the `/v1/events` WebSocket, sign-in and token rotation. It is
//! exposed to Kotlin/Swift through UniFFI as a small, coarse API:
//!
//! - [`Server`]: a validated gateway address;
//! - [`Session`]: sign-in (password, two-step, pairing), token refresh, sign-out. The
//!   refresh token is persisted only through the app's [`SecureStore`] (Keystore-backed on
//!   Android); access tokens live in memory only;
//! - [`MusicLibrary`], [`Players`]: browsing and remote control, using only commands the
//!   gateway's allow-list accepts (the same shapes as the web app);
//! - [`Art`]: artwork bytes, fetched with the bearer token and size-capped;
//! - [`Analysis`]: track analyses (waveform, loudness) from the gateway;
//! - [`Events`]: the live event stream with reconnects and token renewal;
//! - [`LocalPlayer`]: this phone as an LMS player (Slimproto through the gateway, the
//!   core's decoders, optional DSP, a lock-free ring and AAudio output).
//!
//! Tokens and passwords are never logged or put in error messages.

uniffi::setup_scaffolding!();

pub mod analysis;
pub mod art;
pub mod commands;
pub mod error;
pub mod events;
pub mod library;
pub mod model;
pub mod player;
pub mod players;
pub mod server;
pub mod session;
mod transport;

#[cfg(target_os = "android")]
mod android;

pub use analysis::{Analysis, AnalysisOutcome, TrackAnalysisInfo};
pub use art::{Art, ArtImage};
pub use error::ApiError;
pub use events::{EventListener, EventSubscription, Events, EventsState, ServerEvent};
pub use library::MusicLibrary;
pub use model::*;
pub use player::{
    DspSettings, Levelling, LocalPlayback, LocalPlayer, LocalPlayerConfig, LocalPlayerObserver,
    LocalPlayerStatus, OutputKind, OutputRoute, PcmBlock, PlayerConnection, SignalPathInfo,
    SignalStage, SignalVerdict,
};
pub use players::Players;
pub use server::{PairingInvite, Server, parse_pairing_invite};
pub use session::{SecureStore, Session, SessionObserver, StoreError, StoredCredentials};

/// The version of this library, for the app's "About" screen.
#[uniffi::export]
pub fn library_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
