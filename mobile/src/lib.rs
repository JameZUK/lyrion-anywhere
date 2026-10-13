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
//! - [`MusicLibrary`], [`Players`], [`Menus`]: browsing, remote control and LMS menus (apps,
//!   radio, plugins, "More…"), using only commands the gateway's allow-list accepts (the
//!   same shapes as the web app) and menu actions LMS offered. `Players` also moves
//!   playback between players (`transfer`, docs/PROTOCOL.md › Transfer), edits the queue
//!   and joins sync groups;
//! - [`Art`]: artwork bytes, fetched with the bearer token and size-capped;
//! - [`Analysis`]: track analyses (waveform, loudness) from the gateway;
//! - [`Discovery`]: "Start radio" and "More like this", from AudioMuse-AI's menus or LMS's
//!   Don't Stop The Music, whichever the server has;
//! - [`auto`]: the browse tree for cars (Android Auto now, CarPlay later);
//! - [`Me`]: the user's listening history, customisable Home, stations and pinned menus;
//!   [`StationRunner`] keeps a station going (docs/plans/wave1-design.md §6–8);
//! - [`Events`]: the live event stream with reconnects and token renewal;
//! - [`sleep`]: the sleep timer's rules (end of track/album/queue, re-arming, fade-out);
//! - [`LocalPlayer`]: this phone as an LMS player (Slimproto through the gateway, the
//!   core's decoders, optional DSP, a lock-free ring and AAudio output);
//! - offline ([`offline`]): [`OfflineLibrary`] (downloads on the device, per user and
//!   server), [`Downloader`] (resumable downloads), [`sync_plays`] (plays heard offline, to
//!   `POST /v1/plays`) and [`plan_smart`] (smart downloads);
//! - [`OfflinePlayer`]: downloads played from the app's own queue through the same audio
//!   path as [`LocalPlayer`] (DSP, levelling, bit-perfect output, signal path, gapless).
//!
//! Tokens and passwords are never logged or put in error messages.

uniffi::setup_scaffolding!();

pub mod analysis;
pub mod art;
pub mod auto;
pub mod commands;
pub mod crash;
pub mod discovery;
pub mod edits;
pub mod error;
pub mod events;
pub mod library;
pub mod lyrics;
pub mod me;
pub mod menus;
pub mod model;
pub mod offline;
pub mod player;
pub mod players;
pub mod routine;
pub mod server;
pub mod session;
pub mod sleep;
mod tls;
pub mod transfer;
mod transport;

#[cfg(target_os = "android")]
mod android;

pub use analysis::{Analysis, AnalysisOutcome, TrackAnalysisInfo};
pub use art::{Art, ArtImage};
pub use auto::{BrowseNode, PageWindow};
pub use discovery::{Discovery, DiscoveryOptions, DstmProvider, DstmState, RadioPlan, RadioSeed};
pub use error::ApiError;
pub use events::{EventListener, EventSubscription, Events, EventsState, ServerEvent};
pub use library::{FavouriteLibraryItems, MusicLibrary};
pub use lyrics::{LyricLine, TrackLyrics};
pub use me::{
    FilterAlbums, FilterRule, HistoryEntry, HistoryPage, HomeLayout, HomeShelf, Me, MenuStep,
    PinnedMenu, RunnerUpdate, ShelfKind, SmartFilter, Station, StationBatch, StationRunner,
    StationSpec,
};
pub use menus::{
    AppEntry, MenuAction, MenuItem, MenuItemKind, MenuOutcome, MenuPage, MenuParam, Menus,
    PluginEntry, Suitability, app_menu, info_menu,
};
pub use model::*;
pub use offline::{
    DownloadFailure, DownloadFormat, DownloadItem, DownloadObserver, DownloadOrigin,
    DownloadPolicy, DownloadRequest, DownloadState, Downloader, DownloadsSnapshot, FormatChoice,
    NetworkKind, OfflineAlbum, OfflineLibrary, OfflineTrack, PlayRecord, PlayVerdict,
    PlaysSyncOutcome, SmartPlan, plan_smart, remove_offline_data, sync_plays,
};
pub use player::{
    CrossfeedSetting, DitherSetting, DspSettings, EqProfileSummary, Levelling, LocalPlayback,
    LocalPlayer, LocalPlayerConfig, LocalPlayerObserver, LocalPlayerStatus, LocalRepeat,
    MixerOffer, NotBitPerfect, OfferBlocker, OfflinePlayer, OfflinePlayerObserver,
    OfflinePlayerStatus, OutputKind, OutputNegotiator, OutputOffers, OutputRoute, PcmBlock,
    PcmEncoding, PcmIntBlock, PlayerConnection, SignalPathInfo, SignalStage, SignalVerdict,
    StreamQuality, TransitionMode, VISUAL_FRAME_BYTES, VISUAL_FRAME_FLOATS, check_eq_profile,
    stream_quality_for,
};
pub use players::Players;
pub use routine::{
    AlarmChange, AlarmInfo, AlarmListing, AlarmSound, QueuedRating, RatingStyle,
    RatingsSyncOutcome, Routine, TrackRating, next_alarm_in, sync_ratings,
};
pub use server::{PairingInvite, Server, parse_pairing_invite};
pub use session::{SecureStore, Session, SessionObserver, StoreError, StoredCredentials};
pub use sleep::SleepEnd;
pub use transfer::{
    TransferMethod, TransferOutcome, TransferProgress, TransferRefusal, TransferStage,
};

/// The version of this library, for the app's "About" screen.
#[uniffi::export]
pub fn library_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
