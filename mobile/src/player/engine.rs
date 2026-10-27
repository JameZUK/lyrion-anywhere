//! The player engine: one thread with its own single-threaded tokio runtime, driving the
//! core's sans-IO Slimproto player.
//!
//! ```text
//!   /v1/player socket ─┐                         ┌─► ring ─► output (AAudio callback)
//!   /v1/stream fetch ──┼─► events ─► core Player ─┤
//!   app commands ──────┘     ▲   (decode, STAT)  └─► pipeline (DSP, resample)
//!                            └── 20 ms tick: played frames, decode, fill, status
//! ```
//!
//! Behaviour follows the web player (`web/app/static/player/player-worker.js`):
//! - a dropped socket **resumes** the same gateway session (LMS never sees the player
//!   leave), re-sending only what the gateway lacks; after the grace period, or when the
//!   gateway says the session is gone, it starts afresh with a new HELO (same MAC);
//! - a broken download resumes at the byte offset already received, and is acknowledged
//!   at least every 256 KiB so the gateway's replay window always covers it; it keeps
//!   trying for as long as the gateway holds the session (the granted resume grace);
//! - each stream is spooled to disk as fast as the network allows (`super::spool`) and
//!   fed to the decoder as it wants it, so a gap in the signal is covered by what is
//!   spooled; in read-ahead mode the next track is fetched long before the current one
//!   ends (docs/plans/wave3-design.md, "Rolling cache", Phase A);
//! - streams received whole are kept in the replay cache (`super::cache`, Phase B) when
//!   the account may download; a stream whose first bytes match a kept one, checked by its
//!   hash, is played from the phone instead of the rest of the download;
//! - a FLAC stream at the Original level whose audio is a downloaded original's (the same
//!   STREAMINFO, the same bytes so far, the file checked whole) plays from the download
//!   after its first bytes (`super::downloads`, Phase C);
//! - the resume grace is negotiated (asked for when `/v1/me` offers it), and the app says
//!   when the phone has no network: reconnecting then waits for it, and starts at once
//!   when it returns;
//! - after a stall, a player in a sync group skips what the others played meanwhile, or
//!   has LMS restart the group in step, where the others are, when too little is buffered
//!   to do that; an underrun report (`STMo`) held through the gap isn't sent to LMS then,
//!   since LMS would pause the whole group for it;
//! - a gap longer than the grace, while playing, ends in a new session that restarts the
//!   track heard last where it was, once LMS confirms that track is still there and nobody
//!   paused or stopped it (`protocol::restart_commands`);
//! - an expired access token is renewed and the socket reconnects; a refusal or a
//!   takeover by another connection stops for good;
//! - while the link is down the phone's own controls work on what it holds (`super::outage`):
//!   play and pause at once; a skip, a jump or a seek plays its own copies of the queue (a
//!   "takeover", which leaves LMS's session: a new one starts when the server is back, and
//!   LMS is then told where the phone is, if its queue still has that track there).
//!
//! Only this thread touches the core player, the pipeline and the output; the real-time
//! output callback only reads the ring.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::{Connector, connect_async_tls_with_config};

use anywhere_core::player::{
    Action, OutputState, Player, PlayerConfig, SYNC_STALL_SKIP_MS, StallResume,
};
use anywhere_core::proto::{ClientMsg, format_mac};

use super::audio::{AudioPath, AudioSource, OutputShared, dsp_transitions};
use super::cache::{self, Digest, Pin, SharedCache, StreamHash};
use super::clock::mono_ns;
use super::downloads::{self, Downloads, FlacHead, Found};
use super::local::{LocalTrack, Opener};
use super::outage::{self, LocalCmd, OfflineControls, Snapshot, Sources, Takeover};
use super::output::AudioOutput;
use super::pipeline::{DspSettings, Levelling};
use super::protocol::{self, CloseAction, Greeting};
use super::ring::CHANNELS;
use super::spool::{FEED_CHUNK, Spool, SpoolDir, SpoolReadError};
use super::transitions::{TrackFacts, TransitionMode};
use super::{
    LocalPlayback, LocalPlayerObserver, LocalPlayerStatus, PlayerConnection, SignalPathInfo,
    StreamQuality,
};
use crate::error::ApiError;
use crate::server::Server;
use crate::session::Core;
use crate::transport::{self, Http};
use anywhere_core::dsp::levelling::Measurement;

pub(crate) const TICK: Duration = Duration::from_millis(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// LMS asks for the player's status every 5 s; a socket that has brought nothing for this
/// long has died without closing (a network change): resume on a new one, well within the
/// gateway's 60 s grace. The gateway also pings every 5 s (answered by the WebSocket layer),
/// so a socket whose LMS is busy and silent isn't taken for dead. Only what arrives counts: the player's own STATs going out say
/// nothing about whether anything is getting through.
const SOCKET_IDLE: Duration = Duration::from_secs(25);
/// Longest wait to hand one message to a socket (a dead one stops taking them).
const SOCKET_SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Between two pieces of an audio stream (the gateway gives up on LMS after 60 s).
const STREAM_READ_TIMEOUT: Duration = Duration::from_secs(75);
/// Status notifications at most this often (state changes are sent at once).
pub(crate) const STATUS_EVERY: Duration = Duration::from_millis(500);

/// Codecs this build decodes from a stream, as Slimproto capability tokens. Not
/// advertised, so LMS converts them (losslessly where the source is lossless):
/// - `pcm`, `aif`: LMS may send raw headerless PCM, which the decoder can't probe;
/// - `alc`, `aac`: in MP4 files whose index (`moov`) comes after the audio, which is
///   common, a stream can't be decoded without seeking (only ADTS AAC and "fast start"
///   files can), so such tracks would fail.
pub fn codecs() -> Vec<String> {
    let mut c = vec!["flc", "ogg", "mp3"];
    if cfg!(feature = "opus") {
        c.insert(1, "ops");
    }
    c.into_iter().map(String::from).collect()
}

/// Commands from the app.
pub(crate) enum Cmd {
    Stop,
    SetDsp(DspSettings),
    /// The output device or the bit-perfect setting changed: negotiate the output again.
    Renegotiate,
    /// The stream level for the streams that follow. `applied` is told (or dropped) once
    /// the gateway has echoed the level, or at once when nothing is sent to it.
    SetQuality(StreamQuality, Option<std::sync::mpsc::Sender<()>>),
    /// The phone's media volume as an attenuation (dB) for loudness compensation, or
    /// `None` when it isn't known.
    SetLoudnessReference(Option<f64>),
    /// Whether the phone has a network (Android's `ConnectivityManager`).
    Network(bool),
    /// Read ahead: report a track ready once it is fully received (needs the spool).
    SetReadAhead(bool),
    /// How much of a track must be heard before read-ahead fetches the next one (ms).
    SetReadyAfter(u64),
    /// The replay cache's size (bytes; 0: off).
    SetCacheBytes(u64),
    /// LMS's queue as the app last saw it (for the controls while the server is away).
    Queue(Snapshot),
    /// A control used on the phone while the server can't be reached.
    Local(LocalCmd),
}

/// Most bytes handed to the decoder per call of the feeder (a tick is 20 ms: 50 MB/s).
const FEED_PER_CALL: usize = 1024 * 1024;
/// How long the first connection waits for `/v1/me` (to know whether to ask for a longer
/// resume grace) before going ahead without it.
const FEATURES_WAIT: Duration = Duration::from_secs(2);
/// A connection that has lived this long after its greeting counts as working: the
/// reconnect backoff starts again from the beginning. Sooner (a server that greets and
/// drops the connection at once), each reconnect waits longer.
const STABLE_AFTER: Duration = Duration::from_secs(5);
/// The network coming back reconnects at once only this long after the last connection
/// was opened (Android reports several changes a second during a handover).
const NETWORK_SETTLE: Duration = Duration::from_millis(1500);
/// A live socket outlasts a loss of the network this short: Android reports the default
/// network lost and back within a moment whenever it picks it again (Wi-Fi scoring, a
/// fallback to mobile data, a validation blip), and dropping a working socket for that
/// cost a reconnect about once a minute (live, 2026-10-15). A socket that really has died
/// meanwhile is caught by [`SOCKET_IDLE`] and [`SOCKET_SEND_TIMEOUT`].
const NETWORK_LOSS_SETTLE: Duration = Duration::from_secs(2);
/// Longest the close handshake may take when the engine closes a socket on purpose.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// Network losses are counted over this long ([`LocalPlayerStatus::network_flaps`]).
const FLAP_WINDOW: Duration = Duration::from_secs(60);
/// No more than this many are remembered.
const MAX_FLAPS: usize = 1_000;
/// In a sync group, how long to wait after a stall for enough audio to skip the stall
/// before deciding (see [`stall_step`]).
const REJOIN_WAIT: Duration = Duration::from_secs(2);
/// Audio wanted beyond the stall before skipping it, so the output doesn't run dry again
/// at once.
const REJOIN_MARGIN_MS: u64 = 250;
/// After a new session replaced one lost while playing, how long to let LMS settle (it
/// may restart the player itself) before asking it to restart the track heard last.
const RESTART_SETTLE: Duration = Duration::from_millis(1000);
/// A track that ended with LMS still in reach, and nothing sent to follow it, counts as
/// just ended when the link goes down this soon after: LMS may have been about to send the
/// next one. Later, LMS chose to stop there, and the phone mustn't start playing by itself.
const ENDED_GRACE: Duration = Duration::from_secs(15);
/// Queue positions remembered (by track sequence number).
const POSITIONS_KEEP: usize = 8;
/// A stream that LMS said began within this much of its track's start is the whole track
/// (LMS's `time`, asked a moment after the stream starts, less what was heard since).
const WHOLE_FROM_MS: u64 = 1_000;
/// While the server can't be reached, what is on the phone is looked at again this often.
const SOURCES_EVERY: Duration = Duration::from_secs(10);

/// What to do about a stall (an underrun) now that audio may be available again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StallStep {
    /// Nothing to do: not stalled, or no audio yet.
    Nothing,
    /// Hold the output back a little longer: in a group, waiting for enough audio to skip
    /// the stall.
    Wait,
    /// Decide now ([`Player::resume_after_stall`]).
    Resume,
}

/// The policy for resuming after a stall (A6). Alone, carry on as soon as there is audio.
/// In a sync group, wait (up to [`REJOIN_WAIT`]) until the audio decoded covers the stall
/// and a margin, so it can be skipped and the player is back in step; past that wait,
/// decide anyway (the core then says whether to skip or to have LMS restart the player).
pub(crate) fn stall_step(
    stalled_ms: Option<u64>,
    buffered_ms: u64,
    in_group: bool,
    waited: Duration,
) -> StallStep {
    let Some(stalled) = stalled_ms else {
        return StallStep::Nothing;
    };
    if buffered_ms == 0 {
        return StallStep::Nothing;
    }
    if !in_group || stalled <= SYNC_STALL_SKIP_MS {
        return StallStep::Resume;
    }
    if buffered_ms >= stalled + REJOIN_MARGIN_MS || waited >= REJOIN_WAIT {
        StallStep::Resume
    } else {
        StallStep::Wait
    }
}

/// An estimate of how much listening is held ahead, in ms: the audio decoded, plus the
/// compressed bytes not yet decoded (`undecoded_bytes`: spooled, or inside the core for
/// this track and the queued next one) at the byte rate of the track being decoded
/// (`bytes_per_ms`, see [`Player::decoded_rate`]). Without a rate yet, only what is
/// decoded.
pub(crate) fn cushion_ms(
    decoded_ahead_ms: u64,
    undecoded_bytes: u64,
    bytes_per_ms: Option<f64>,
) -> u64 {
    let Some(per_ms) = bytes_per_ms.filter(|r| r.is_finite() && *r > 0.0) else {
        return decoded_ahead_ms;
    };
    let ahead = undecoded_bytes as f64 / per_ms;
    decoded_ahead_ms.saturating_add(ahead.min(1e12) as u64)
}

/// What the engine reacts to.
enum Ev {
    WsGreeting {
        conn: u64,
        greeting: Greeting,
    },
    WsFrame {
        conn: u64,
        bytes: Vec<u8>,
    },
    WsClosed {
        conn: u64,
        code: Option<u16>,
        reason: String,
    },
    /// The gateway's stream message: what LMS served and what the gateway sent for a stream.
    StreamInfo {
        conn: u64,
        message: protocol::StreamMessage,
    },
    /// The gateway has applied a change of stream level (its echo of `{"quality"}`).
    QualityEcho {
        conn: u64,
        level: StreamQuality,
    },
    StreamHeaders {
        id: u64,
        content_type: String,
    },
    /// What the gateway offers (`/v1/me` features; `None`: it couldn't be asked).
    Features(Option<protocol::Features>),
    /// Whether this player is in a sync group (from LMS's status).
    SyncGroup {
        player_gen: u64,
        grouped: bool,
    },
    /// Where track `seq` sits in LMS's queue (for restarting it after a lost session).
    Position {
        player_gen: u64,
        seq: u32,
        fresh: bool,
        pos: protocol::StreamPosition,
    },
    StreamData {
        id: u64,
        bytes: Vec<u8>,
    },
    StreamEnd {
        id: u64,
        /// The stream's fingerprint and hash, for the replay cache (when it is in use).
        digest: Option<StreamDigest>,
    },
    /// The rest of stream `id` comes from the replay cache.
    FromCache {
        id: u64,
    },
    /// The rest of stream `id` comes from a downloaded original.
    FromDownloads {
        id: u64,
    },
    /// Whether the account may download (`/v1/account`; `None`: it couldn't be asked).
    Downloads {
        player_gen: u64,
        allowed: Option<bool>,
    },
    StreamFailed {
        id: u64,
    },
    /// The R128 measurement for track `seq` (`None`: no analysis, use tags).
    Loudness {
        player_gen: u64,
        seq: u32,
        measurement: Option<Measurement>,
    },
    /// What transitions need to know about track `seq` (sent once LMS has said which track
    /// it is, and again when its analysis arrives).
    Facts {
        player_gen: u64,
        seq: u32,
        facts: TrackFacts,
    },
}

/// How long a fresh start waits for its track's analysis before playing with tag-based
/// levelling instead (only when analysis levelling is on).
const LOUDNESS_WAIT: Duration = Duration::from_millis(1500);
/// How long a lookup keeps asking while the gateway is still analysing: a gapless next
/// track has until the current one ends.
const LOUDNESS_BUDGET_FRESH: Duration = Duration::from_secs(10);
const LOUDNESS_BUDGET_NEXT: Duration = Duration::from_secs(90);
/// Measurements remembered (by track sequence number).
const LOUDNESS_KEEP: usize = 8;

/// Messages towards the gateway.
enum Out {
    Binary(Vec<u8>),
    Text(String),
    /// Close the socket cleanly (a close frame, then the gateway's answer) and end.
    Close,
}

/// State shared with the app-facing [`super::LocalPlayer`].
pub(crate) struct Shared {
    pub status: Mutex<LocalPlayerStatus>,
    pub signal: Mutex<Option<SignalPathInfo>>,
    pub observer: Arc<dyn LocalPlayerObserver>,
    /// The ring, the hold, the route and the negotiator.
    pub out: Arc<OutputShared>,
    /// Tracks whose stream is spooled whole now (for the queue's "on this device" marks).
    pub spooled: Mutex<Vec<i64>>,
}

struct StreamTask {
    id: u64,
    /// The stream number (its spool's).
    n: u64,
    want: Arc<AtomicBool>,
    task: tokio::task::AbortHandle,
}

pub(crate) struct Engine {
    core: Arc<Core>,
    server: Arc<Server>,
    http: Http,
    shared: Arc<Shared>,
    max_rate: u32,
    tx: mpsc::UnboundedSender<Ev>,
    rx: mpsc::UnboundedReceiver<Ev>,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    /// The player's clock origin on `CLOCK_MONOTONIC`, the clock the output's timestamps
    /// use: jiffies are milliseconds since then.
    start_ns: i64,
    /// A timed start is armed in the ring, which releases its hold at the exact frame.
    timed_start: bool,
    // Connection.
    /// The stream level wanted, and whether the gateway can change it mid-session (`None`
    /// until known).
    quality: StreamQuality,
    quality_feature: Option<bool>,
    /// A caller waiting for the gateway to echo the level sent (one at a time: a newer
    /// change drops the older waiter, which then returns).
    quality_waiter: Option<(StreamQuality, std::sync::mpsc::Sender<()>)>,
    /// The gateway takes a resume grace at sign-in (`None` until known).
    grace_feature: Option<bool>,
    /// The first connection waits for the features until then.
    connect_at: Option<Instant>,
    /// The resume grace the gateway granted (seconds; `None`: it didn't say).
    grace: Option<u64>,
    /// The phone has a network (as the app last said; assumed until told otherwise).
    network: bool,
    /// Tells stream fetches when the network comes back.
    net_tx: tokio::sync::watch::Sender<bool>,
    /// Reconnecting waits for the network; `Some(resume)` once it is wanted.
    waiting_for_network: Option<bool>,
    session: Option<(String, String)>,
    received: u64,
    ws_gen: u64,
    ws_out: Option<mpsc::UnboundedSender<Out>>,
    ws_task: Option<tokio::task::AbortHandle>,
    open: bool,
    attempt: u32,
    /// When the open connection was greeted, until it has lived [`STABLE_AFTER`].
    greeted_at: Option<Instant>,
    /// When a connection was last opened.
    last_connect: Option<Instant>,
    /// The app said the network went while the socket was live, then: the socket is kept
    /// until [`NETWORK_LOSS_SETTLE`] has passed without the network coming back.
    network_lost_pending: Option<Instant>,
    /// When the app said the network went, over the last [`FLAP_WINDOW`].
    flaps: VecDeque<Instant>,
    disconnected_at: Option<Instant>,
    reconnect_at: Option<(Instant, bool)>,
    renewals_in_a_row: u32,
    /// Protocol errors close together, and when the last was: each backs off further, so
    /// a server that keeps sending a bad frame isn't reconnected to in a tight loop.
    bad_frames: (u32, Option<Instant>),
    /// Frames for LMS held while the link is down (see [`protocol::release_outbox`]).
    outbox: VecDeque<protocol::Held>,
    connection: PlayerConnection,
    // Player.
    player: Option<Player>,
    mac: Option<[u8; 6]>,
    name: String,
    stream: Option<StreamTask>,
    stream_ids: u64,
    last_ack: (u64, u64),
    last_flush: u64,
    last_trim: u64,
    // Spooling and read-ahead.
    /// Where spools go (`None`: no cache folder, so streams are held in memory).
    spool_dir: Option<SpoolDir>,
    /// One per stream the decoder still takes, oldest (being fed) first; the newest is
    /// being fetched.
    spools: VecDeque<Spool>,
    /// The read-ahead setting (in force only with a spool folder).
    read_ahead: bool,
    /// How much must be heard before read-ahead reports a track ready (`None`: the core's
    /// default).
    ready_after_ms: Option<u64>,
    /// The byte rate of the track decoded last (bytes per ms of audio, from the core once
    /// a second of a track is decoded), kept across the hand-over to the next track: what
    /// the cushion turns undecoded bytes into time with.
    byte_rate: Option<f64>,
    // The replay cache (Phase B).
    /// This user's cache on this server (`None`: no cache folder).
    cache: Option<SharedCache>,
    /// Its size (0: off).
    cache_bytes: u64,
    /// The account may download (`None` until known): the cache is used only when it may.
    downloads_allowed: Option<bool>,
    /// Streams (by number) whose rest came from the cache, newest last.
    from_cache: VecDeque<u64>,
    // Downloaded originals as a source (Phase C).
    /// This user's downloads on this server (`None`: no downloads folder).
    downloads: Option<Arc<Downloads>>,
    /// Streams (by number) whose rest came from a download, newest last.
    from_downloads: VecDeque<u64>,
    // Sync groups (A6).
    in_sync_group: bool,
    /// A stall in a group waiting for enough audio, since then.
    stall_wait: Option<Instant>,
    rejoin: Option<tokio::task::AbortHandle>,
    // Restarting after a gap longer than the grace (the owner's decision 3).
    /// Queue positions of recent tracks: (seq, position, ms into the track where its
    /// stream began).
    positions: VecDeque<(u32, protocol::StreamPosition, u64)>,
    /// While the link to LMS is down: where to restart if the session is lost.
    gap: Option<protocol::GapWatch>,
    restart: Option<tokio::task::AbortHandle>,
    // Offline controls (`super::outage`).
    /// LMS's queue as the app last saw it.
    snapshot: Snapshot,
    /// Playing the phone's own copies of the queue, with no server.
    takeover: Option<Takeover>,
    /// The user played, paused, skipped or sought on the phone while the link was down: on
    /// reconnecting, the phone's state is the truth (see `protocol::restart_commands`).
    acted: bool,
    /// Stopped at the end of a track: the next one needs the server.
    waiting_for_server: bool,
    /// When the track heard last played out (and `STMu` went) with the link up and nothing
    /// to follow it yet.
    ended_online_at: Option<Instant>,
    /// Where each track is on the phone, whole, and when that was looked at.
    sources: Option<(Instant, Sources)>,
    /// Spools kept whole for a takeover (their files go when they do).
    held_spools: Vec<(Spool, i64, LocalTrack)>,
    /// The `strm` format of recent streams (by number).
    stream_formats: VecDeque<(u64, u8)>,
    /// The gateway's stream messages for recent streams of this session (by number).
    stream_book: protocol::StreamBook,
    /// What the controls can do now (while the link is down).
    controls: Option<OfflineControls>,
    // Audio: pipeline, ring, output and its negotiated route.
    audio: AudioPath,
    underruns_seen: u64,
    was_playing: bool,
    // Analysis levelling.
    loudness: VecDeque<(u32, Option<Measurement>)>,
    /// Track facts for transitions (by track sequence number).
    facts: VecDeque<(u32, TrackFacts)>,
    /// A fresh start waiting (until the instant) for track `seq`'s analysis.
    gate: Option<(u32, Instant)>,
    lookup: Option<tokio::task::AbortHandle>,
    /// Bumped with every new core player (track numbers start again).
    player_gen: u64,
    // Reporting.
    last_status_at: Option<Instant>,
    last_status_key: Option<(
        PlayerConnection,
        LocalPlayback,
        u32,
        bool,
        Option<ControlsKey>,
    )>,
    last_signal: Option<SignalPathInfo>,
    stopped: bool,
}

impl Engine {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        core: Arc<Core>,
        shared: Arc<Shared>,
        output: Box<dyn AudioOutput>,
        dsp: DspSettings,
        max_rate: u32,
        quality: StreamQuality,
        spool_dir: Option<SpoolDir>,
        read_ahead: bool,
        cache: Option<(SharedCache, u64)>,
        downloads: Option<Arc<Downloads>>,
        cmds: mpsc::UnboundedReceiver<Cmd>,
    ) -> Result<Engine, ApiError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let audio = AudioPath::new(shared.out.clone(), output, dsp)?;
        let (net_tx, _) = tokio::sync::watch::channel(true);
        Ok(Engine {
            grace_feature: None,
            connect_at: None,
            grace: None,
            network: true,
            net_tx,
            waiting_for_network: None,
            last_trim: 0,
            spool_dir,
            spools: VecDeque::new(),
            read_ahead,
            ready_after_ms: None,
            cache_bytes: cache.as_ref().map_or(0, |(_, b)| *b),
            cache: cache.map(|(c, _)| c),
            downloads_allowed: None,
            from_cache: VecDeque::new(),
            downloads,
            from_downloads: VecDeque::new(),
            byte_rate: None,
            in_sync_group: false,
            stall_wait: None,
            rejoin: None,
            positions: VecDeque::new(),
            gap: None,
            restart: None,
            snapshot: Snapshot::default(),
            takeover: None,
            acted: false,
            waiting_for_server: false,
            ended_online_at: None,
            sources: None,
            held_spools: Vec::new(),
            stream_formats: VecDeque::new(),
            stream_book: protocol::StreamBook::default(),
            controls: None,
            server: core.server().clone(),
            core,
            http: Http::private()?,
            shared,
            max_rate,
            tx,
            rx,
            cmds,
            start_ns: mono_ns(),
            timed_start: false,
            quality,
            quality_feature: None,
            quality_waiter: None,
            session: None,
            received: 0,
            ws_gen: 0,
            ws_out: None,
            ws_task: None,
            open: false,
            attempt: 0,
            greeted_at: None,
            last_connect: None,
            network_lost_pending: None,
            flaps: VecDeque::new(),
            disconnected_at: None,
            reconnect_at: None,
            renewals_in_a_row: 0,
            bad_frames: (0, None),
            outbox: VecDeque::new(),
            connection: PlayerConnection::Connecting,
            player: None,
            mac: None,
            name: String::new(),
            stream: None,
            stream_ids: 0,
            last_ack: (0, 0),
            last_flush: 0,
            audio,
            underruns_seen: 0,
            was_playing: false,
            loudness: VecDeque::new(),
            facts: VecDeque::new(),
            gate: None,
            lookup: None,
            player_gen: 0,
            last_status_at: None,
            last_status_key: None,
            last_signal: None,
            stopped: false,
        })
    }

    fn now_ms(&self) -> u64 {
        self.ms_at(mono_ns())
    }

    /// Host time (jiffies, ms) at monotonic time `ns`.
    fn ms_at(&self, ns: i64) -> u64 {
        (ns.saturating_sub(self.start_ns) / 1_000_000).max(0) as u64
    }

    /// Hand LMS's sync corrections to the output, where they apply at once and exactly.
    fn apply_corrections(&mut self) {
        let Some(p) = &mut self.player else { return };
        let corrections = p.take_corrections();
        if corrections.is_empty() {
            return;
        }
        let now_ns = mono_ns();
        let start_ns = self.start_ns;
        for c in corrections {
            let to_ns = |ms: u64| start_ns.saturating_add((ms as i64).saturating_mul(1_000_000));
            if self.audio.apply_correction(c, now_ns, to_ns) {
                self.timed_start = true;
            }
        }
    }

    pub(crate) async fn run(mut self) {
        self.learn_features();
        self.learn_downloads();
        // The first connection waits (briefly) to know whether to ask for a longer grace.
        self.connect_at = Some(Instant::now() + FEATURES_WAIT);
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        while !self.stopped {
            tokio::select! {
                Some(ev) = self.rx.recv() => self.handle(ev),
                cmd = self.cmds.recv() => match cmd {
                    Some(Cmd::SetDsp(d)) => {
                        self.audio.pipeline.set_dsp(d);
                        self.publish_signal(true);
                    }
                    Some(Cmd::Renegotiate) => self.audio.renegotiate = true,
                    Some(Cmd::SetQuality(q, applied)) => self.set_quality(q, applied),
                    Some(Cmd::SetLoudnessReference(db)) => {
                        self.audio.pipeline.set_loudness_reference_db(db);
                        self.publish_signal(true);
                    }
                    Some(Cmd::Network(up)) => self.set_network(up),
                    Some(Cmd::SetReadAhead(on)) => {
                        self.read_ahead = on;
                        self.apply_read_ahead();
                    }
                    Some(Cmd::SetReadyAfter(ms)) => {
                        self.ready_after_ms = Some(ms);
                        self.apply_read_ahead();
                    }
                    Some(Cmd::SetCacheBytes(b)) => {
                        self.cache_bytes = b;
                        if let Some(c) = &self.cache {
                            c.lock().unwrap().set_cap(b);
                        }
                        self.sources = None;
                    }
                    Some(Cmd::Queue(q)) => {
                        self.snapshot = q;
                        self.publish_status(true);
                    }
                    Some(Cmd::Local(c)) => self.local_command(c),
                    // The app let go of the player, or asked to stop.
                    Some(Cmd::Stop) | None => self.shutdown("stopped").await,
                },
                _ = tick.tick() => self.on_tick(),
            }
        }
    }

    /// Leave LMS cleanly: `BYE!` (the player disappears from LMS at once), then close.
    async fn shutdown(&mut self, reason: &str) {
        // First, so nothing moves the group while the player leaves.
        self.cancel_restarts();
        self.network_lost_pending = None;
        if self.open
            && let Some(out) = self.ws_out.take()
        {
            let bye = ClientMsg::Bye.encode();
            let _ = out.send(Out::Binary(bye));
            // Then a close frame, so the gateway sees the socket closed, not reset.
            let _ = out.send(Out::Close);
            // Give the socket task a moment to deliver both.
            if let Some(t) = &self.ws_task {
                let until = Instant::now() + CLOSE_WAIT + Duration::from_millis(250);
                while !t.is_finished() && Instant::now() < until {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        self.close_stream();
        if let Some(t) = self.ws_task.take() {
            t.abort();
        }
        self.gap = None;
        self.close_output();
        self.shared.out.ring.set_hold(true);
        self.connection = PlayerConnection::Stopped {
            reason: reason.into(),
        };
        self.player = None;
        self.end_takeover();
        self.drop_spools();
        self.publish_status(true);
        self.publish_signal(true);
        self.stopped = true;
    }

    /// Close the output for good (stopping): Android's normal mixing is restored.
    fn close_output(&mut self) {
        let src = match self.takeover.as_mut() {
            Some(t) => Some(&mut t.source as &mut dyn AudioSource),
            None => self.player.as_mut().map(|p| p as &mut dyn AudioSource),
        };
        self.audio.close_output(src);
    }

    // --- Connection ---------------------------------------------------------------------

    /// Ask the gateway (once) what it offers: whether a session may change its stream
    /// level, and whether the sign-in takes a resume grace.
    fn learn_features(&self) {
        let core = self.core.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let features = core
                .get_json("v1/me")
                .await
                .ok()
                .map(|v| protocol::parse_features(&v));
            let _ = tx.send(Ev::Features(features));
        });
    }

    /// Ask whether the account may download: the replay cache needs it (the owner's
    /// decision 1). Asked when the player starts and with every new session.
    fn learn_downloads(&self) {
        if self.cache.is_none() {
            return;
        }
        let (core, tx, player_gen) = (self.core.clone(), self.tx.clone(), self.player_gen);
        tokio::spawn(async move {
            let allowed = core
                .get_json("v1/account")
                .await
                .ok()
                .map(|v| v["downloads"]["enabled"].as_bool() == Some(true));
            let _ = tx.send(Ev::Downloads {
                player_gen,
                allowed,
            });
        });
    }

    /// The replay cache, when it is in use: a cache folder, a size, the download
    /// permission, and spooling to disk.
    fn active_cache(&self) -> Option<&SharedCache> {
        self.cache.as_ref().filter(|_| {
            self.cache_bytes > 0 && self.downloads_allowed == Some(true) && self.spool_dir.is_some()
        })
    }

    /// Read-ahead is in force with the setting on and somewhere to spool.
    fn apply_read_ahead(&mut self) {
        let on = self.read_ahead && self.spool_dir.is_some();
        let after = self.ready_after_ms;
        if let Some(p) = &mut self.player {
            p.set_ready_when_received(on);
            if let Some(ms) = after {
                p.set_ready_after_ms(ms);
            }
        }
    }

    /// The core's view of the link to LMS: the socket is up and the phone has a network.
    fn update_link(&mut self) {
        let up = self.open && self.network;
        let now = self.now_ms();
        if let Some(p) = &mut self.player {
            p.set_link_up(up, now);
        }
    }

    /// The app says whether the phone has a network. A live socket outlasts a loss shorter
    /// than [`NETWORK_LOSS_SETTLE`] untouched (see [`Engine::settle_network`]); otherwise
    /// see [`Engine::apply_network`].
    fn set_network(&mut self, up: bool) {
        let said_up = self.network && self.network_lost_pending.is_none();
        if up == said_up {
            return;
        }
        if !up {
            self.count_flap();
        }
        if up && self.network_lost_pending.take().is_some() {
            // Back within the settle time: the socket was never touched.
            self.publish_status(true);
            return;
        }
        if !up && self.open && !self.stopped {
            self.network_lost_pending = Some(Instant::now());
            self.publish_status(true);
            return;
        }
        self.apply_network(up);
    }

    /// A loss of the network that has outlasted [`NETWORK_LOSS_SETTLE`] is applied.
    fn settle_network(&mut self) {
        if self
            .network_lost_pending
            .is_some_and(|t| t.elapsed() >= NETWORK_LOSS_SETTLE)
        {
            self.network_lost_pending = None;
            self.apply_network(false);
        }
    }

    fn count_flap(&mut self) {
        let now = Instant::now();
        while self
            .flaps
            .front()
            .is_some_and(|t| now.duration_since(*t) >= FLAP_WINDOW)
            || self.flaps.len() >= MAX_FLAPS
        {
            self.flaps.pop_front();
        }
        self.flaps.push_back(now);
    }

    /// Network losses the app reported in the last minute.
    fn flaps_per_min(&self) -> u32 {
        self.flaps
            .iter()
            .filter(|t| t.elapsed() < FLAP_WINDOW)
            .count() as u32
    }

    /// The network's state, as it now stands. Without one, the socket is closed (its grace
    /// starts) and no reconnect is tried until the network is back; then the backoff starts
    /// again and one is tried at once, or [`NETWORK_SETTLE`] after the last if that was
    /// only just now (a flapping network). So a "gone, back" while reconnecting (the app's
    /// "reconnect now") retries at once.
    fn apply_network(&mut self, up: bool) {
        if up == self.network {
            return;
        }
        self.network = up;
        let _ = self.net_tx.send(up);
        if self.stopped {
            return;
        }
        if !up {
            if self.open {
                self.close_socket(true);
                self.on_closed(None, "");
            } else if let Some(t) = self.ws_task.take() {
                // Still connecting: nothing has been said on it yet.
                t.abort();
                self.on_closed(None, "");
            } else if let Some((_, resume)) = self.reconnect_at.take() {
                self.waiting_for_network = Some(resume);
                self.connection = PlayerConnection::Reconnecting { delay_ms: 0 };
            }
        } else {
            let now = Instant::now();
            self.attempt = 0;
            // At once, unless a connection was opened only just now.
            let at = self
                .last_connect
                .map_or(now, |t| (t + NETWORK_SETTLE).max(now));
            let resume = self
                .waiting_for_network
                .take()
                .or(self.reconnect_at.map(|(_, r)| r));
            if let Some(resume) = resume {
                self.reconnect_at = Some((at, resume));
                if matches!(self.connection, PlayerConnection::Reconnecting { .. }) {
                    self.connection = PlayerConnection::Reconnecting {
                        delay_ms: at.saturating_duration_since(now).as_millis() as u64,
                    };
                }
            }
        }
        self.update_link();
        self.publish_status(true);
    }

    /// Still within the window for resuming the session after its socket dropped.
    fn within_grace(&self) -> bool {
        let window = protocol::resume_for(self.grace);
        self.disconnected_at.is_none_or(|t| t.elapsed() < window)
    }

    /// A new level for the streams that follow. `applied` hears once the gateway has echoed
    /// it (so a seek that follows gets a stream at the new level); dropped at once when
    /// nothing is sent (the same level, no live socket, or a gateway that can't change it
    /// mid-session: the level is then in the next sign-in form).
    fn set_quality(&mut self, q: StreamQuality, applied: Option<std::sync::mpsc::Sender<()>>) {
        if q == self.quality {
            // Still on its way: this caller waits for the same echo.
            if let (Some(tx), Some((level, _))) = (applied, &self.quality_waiter)
                && *level == q
            {
                self.quality_waiter = Some((q, tx));
            }
            return;
        }
        self.quality = q;
        // A newer change: an older waiter is dropped (and returns).
        self.quality_waiter = None;
        if self.send_quality()
            && let Some(tx) = applied
        {
            self.quality_waiter = Some((q, tx));
        }
    }

    /// Tell the gateway the level now, if it can take it mid-session (true when sent);
    /// otherwise it is asked for when the player next connects.
    fn send_quality(&mut self) -> bool {
        if self.open && self.quality_feature == Some(true) {
            self.send_ws(Out::Text(protocol::quality_message(self.quality)));
            return true;
        }
        false
    }

    /// The gateway's echo of a level: the caller waiting for it returns. An echo of another
    /// level (an older change's) is not the one awaited.
    fn on_quality_echo(&mut self, level: StreamQuality) {
        if let Some((want, _)) = &self.quality_waiter
            && *want == level
            && let Some((_, tx)) = self.quality_waiter.take()
        {
            let _ = tx.send(());
        }
    }

    /// Close the live socket on purpose: a close frame first, so the gateway sees a clean
    /// close rather than a reset (dropping a socket with unread data in its buffer makes
    /// the kernel reset the connection). Its task finishes by itself, within
    /// [`CLOSE_WAIT`], reporting nothing more; with `ignore_rest`, what it delivered before
    /// is ignored too.
    fn close_socket(&mut self, ignore_rest: bool) {
        if ignore_rest {
            self.ws_gen += 1;
        }
        if let Some(out) = self.ws_out.take() {
            let _ = out.send(Out::Close);
        }
        if let Some(t) = self.ws_task.take() {
            // In case it is stuck (a send that never completes).
            tokio::spawn(async move {
                tokio::time::sleep(CLOSE_WAIT * 2).await;
                t.abort();
            });
        }
    }

    fn connect(&mut self, resume: bool) {
        if let Some(t) = self.ws_task.take() {
            t.abort();
        }
        self.ws_gen += 1;
        self.last_connect = Some(Instant::now());
        self.greeted_at = None;
        // The new sign-in form carries the level: nothing to wait for.
        self.quality_waiter = None;
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        self.ws_out = Some(out_tx);
        self.open = false;
        self.connection = if self.player.is_some() || self.takeover.is_some() {
            PlayerConnection::Reconnecting { delay_ms: 0 }
        } else {
            PlayerConnection::Connecting
        };
        let resume = if resume {
            self.session
                .as_ref()
                .map(|(s, k)| (s.clone(), k.clone(), self.received))
        } else {
            None
        };
        self.connect_at = None;
        // Ask for a longer grace only where it is understood.
        let grace = (self.grace_feature == Some(true)).then_some(protocol::GRACE_ASK);
        let task = tokio::spawn(socket_task(
            self.ws_gen,
            self.core.clone(),
            self.server.clone(),
            resume,
            self.quality,
            grace,
            self.tx.clone(),
            out_rx,
            SOCKET_IDLE,
        ));
        self.ws_task = Some(task.abort_handle());
        self.publish_status(true);
    }

    fn on_greeting(&mut self, g: Greeting) {
        self.open = true;
        // The backoff starts again only once this connection has lived a while (see
        // `on_tick`): a server that greets and drops at once is not retried every 0.5 s.
        self.greeted_at = Some(Instant::now());
        self.renewals_in_a_row = 0;
        self.disconnected_at = None;
        self.connection = PlayerConnection::Connected;
        let fresh = !g.resumed || self.player.is_none();
        // The user acted on the phone while the server was away: the phone's state is the
        // truth (where it is, paused or not), if LMS's queue still agrees.
        let acted = std::mem::take(&mut self.acted);
        let local_point = if acted { self.local_point() } else { None };
        let core_paused = self.core_paused();
        // A session lost while playing: where to restart (a resumed one needs nothing).
        let gap_point = self
            .gap
            .take()
            .filter(|w| fresh && self.player.is_some() && !w.blocked)
            .and_then(|w| w.point);
        let restart_at = if acted {
            local_point.filter(|_| fresh)
        } else {
            gap_point
        };
        self.waiting_for_server = false;
        self.end_takeover();
        self.session = Some((g.session.clone(), g.secret.clone()));
        // A resumed session without a grant keeps what it was granted before.
        if g.grace.is_some() || fresh {
            self.grace = g.grace;
        }
        if fresh {
            // A new session: LMS sees a new connection, so start from a clean player.
            self.close_stream();
            self.drop_spools();
            self.outbox.clear();
            self.received = 0;
            self.byte_rate = None;
            self.from_cache.clear();
            self.from_downloads.clear();
            // Stream numbers start again with the session.
            self.stream_book.clear();
            self.in_sync_group = false;
            self.stall_wait = None;
            self.cancel_restarts();
            self.drop_queued_audio();
            // Track numbers start again with the new player.
            self.loudness.clear();
            self.facts.clear();
            self.positions.clear();
            self.gate = None;
            self.player_gen += 1;
            if let Some(t) = self.lookup.take() {
                t.abort();
            }
            let mut p = Player::new(PlayerConfig {
                mac: g.mac,
                uuid: [0; 16],
                name: g.name.clone(),
                codecs: codecs(),
                max_sample_rate: self.max_rate,
                firmware: concat!("anywhere-android-", env!("CARGO_PKG_VERSION")).into(),
            });
            p.connected(self.now_ms());
            // Timed starts, pauses and skips for sync are applied at the output.
            p.set_output_corrections(true);
            // A bit-perfect DAC keeps its fixed volume across sessions.
            p.set_fixed_volume(self.audio.route_state.direct);
            self.audio.forget_plan();
            self.last_flush = p.flush_count();
            self.last_trim = p.trim_count();
            self.player = Some(p);
            self.apply_read_ahead();
            // The permission may have changed since the last session.
            self.learn_downloads();
            self.mac = Some(g.mac);
            self.name = g.name;
            if let Some(point) = restart_at {
                self.restart_where_left(point);
            }
        } else {
            // Back after a gap: is the player (still) in a sync group?
            self.look_up_sync_group();
            // LMS never lost the player; it is told only whether the phone is paused.
            if let Some(cmds) = protocol::resumed_commands(acted, core_paused) {
                self.tell_lms(cmds);
            }
        }
        self.update_link();
        // What waited through the gap goes to LMS now, except, in a sync group, a stale
        // mid-track `STMo`: LMS would pause the whole group to rebuffer, while this player
        // handles the stall itself (see `protocol::release_outbox` for the reasoning).
        // Group membership is as last known: a fresh session has emptied the outbox, and a
        // resumed one asks again just above, but that answer is still on its way.
        let released = protocol::release_outbox(&mut self.outbox, self.in_sync_group);
        if released.stall_unreported
            && let Some(p) = &mut self.player
        {
            p.withdraw_underrun_report();
        }
        for b in released.frames {
            self.send_ws(Out::Binary(b));
        }
        if let Some((n, bytes)) = self.player.as_ref().and_then(Player::open_stream) {
            self.last_ack = (n, bytes);
            self.send_ws(Out::Text(protocol::ack_message(n, bytes)));
        }
        self.run_actions();
        self.publish_status(true);
    }

    fn on_closed(&mut self, code: Option<u16>, reason: &str) {
        if self.network_lost_pending.take().is_some() {
            // The network went and then the socket did: the loss was real.
            self.network = false;
            let _ = self.net_tx.send(false);
        }
        let was_open = self.open;
        self.open = false;
        self.ws_out = None;
        self.ws_task = None;
        // No echo can come now; the level goes in the next sign-in form.
        self.quality_waiter = None;
        self.update_link();
        if was_open || self.disconnected_at.is_none() {
            self.disconnected_at.get_or_insert_with(Instant::now);
        }
        self.watch_gap();
        let action = protocol::close_action(code, reason);
        let resume = match action {
            CloseAction::Stop | CloseAction::UpdateApp => {
                // Refused, revoked, signed out, replaced or too old for the gateway (4426):
                // nothing of it is kept, and the same form is never sent again.
                if code == Some(1008)
                    && let Some(c) = &self.cache
                {
                    // Refused or revoked: nothing of this account's streams stays.
                    c.lock().unwrap().clear();
                }
                self.stop_for_good(if action == CloseAction::UpdateApp {
                    // The app says what to do; the gateway's reason is never shown.
                    PlayerConnection::UpdateApp
                } else {
                    PlayerConnection::Stopped {
                        reason: if reason.is_empty() {
                            "refused".into()
                        } else {
                            protocol::clean_name(reason)
                        },
                    }
                });
                return;
            }
            CloseAction::Reauthenticate => {
                // The socket task has renewed the token. One immediate retry; a server that
                // keeps refusing gets the normal backoff.
                self.renewals_in_a_row += 1;
                if self.renewals_in_a_row == 1 {
                    self.reconnect_at = Some((Instant::now(), self.session.is_some()));
                    return;
                }
                true
            }
            CloseAction::NewSession => {
                self.session = None;
                false
            }
            CloseAction::Resume => true,
        };
        let resume = resume && self.session.is_some() && self.within_grace();
        if !resume {
            self.session = None;
        }
        if !self.network {
            // No network: nothing to try until the app says it is back.
            self.reconnect_at = None;
            self.waiting_for_network = Some(resume);
            self.connection = PlayerConnection::Reconnecting { delay_ms: 0 };
            self.publish_status(true);
            return;
        }
        let delay = protocol::backoff(self.attempt, protocol::random01());
        self.attempt = self.attempt.saturating_add(1);
        self.reconnect_at = Some((Instant::now() + delay, resume));
        self.connection = PlayerConnection::Reconnecting {
            delay_ms: delay.as_millis() as u64,
        };
        self.publish_status(true);
    }

    /// End the player for good, as `connection` says: nothing of the session is kept, no
    /// connection is tried again, and a socket still open is closed.
    fn stop_for_good(&mut self, connection: PlayerConnection) {
        self.close_socket(true);
        self.open = false;
        self.connect_at = None;
        self.reconnect_at = None;
        self.session = None;
        self.quality_waiter = None;
        self.gap = None;
        self.cancel_restarts();
        self.close_stream();
        self.close_output();
        self.audio.pipeline.unconfigure();
        self.shared.out.ring.set_hold(true);
        self.player = None;
        self.end_takeover();
        self.acted = false;
        self.drop_spools();
        // Nor are its downloads read again (the app deletes them on revocation).
        self.downloads = None;
        self.connection = connection;
        self.stopped_by_server();
    }

    fn stopped_by_server(&mut self) {
        self.stopped = true;
        self.publish_status(true);
        self.publish_signal(true);
    }

    fn send_ws(&mut self, msg: Out) {
        if let (true, Some(out)) = (self.open, &self.ws_out) {
            let _ = out.send(msg);
        } else if let Out::Binary(frame) = msg {
            // Held until the session resumes; acknowledgements are only useful live, and so
            // are heartbeats (`push_outbox` never holds an `STMt`). An
            // `STMo` sent while the track is still arriving is an underrun mid-track (the
            // core's `output_underrun`), as opposed to the one sent in place of `STMu` once
            // everything received has played out: only the first may be dropped on a
            // grouped rejoin. Frames are dispatched as soon as the core makes them, so its
            // state now is its state then.
            let gap_underrun = protocol::is_stat_event(&frame, b"STMo")
                && self.player.as_ref().is_some_and(|p| !p.audio_exhausted());
            protocol::push_outbox(
                &mut self.outbox,
                protocol::Held {
                    frame,
                    gap_underrun,
                },
                protocol::OUTBOX_LIMIT,
            );
        }
    }

    // --- Events -------------------------------------------------------------------------

    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::WsGreeting { conn, greeting } if conn == self.ws_gen => self.on_greeting(greeting),
            Ev::WsFrame { conn, bytes } if conn == self.ws_gen => {
                self.received += 1;
                let now = self.now_ms();
                let Some(p) = &mut self.player else { return };
                if p.on_control_data(&bytes, now).is_err() {
                    // A protocol error: close the socket and resume, backing off more for
                    // each one close to the last (up to the usual most).
                    // What it already delivered is still heeded, as before: the rest of a
                    // run of bad frames is used up at once, not one per reconnect.
                    self.close_socket(false);
                    let n = bad_frame_streak(self.bad_frames, Instant::now());
                    self.bad_frames = (n, Some(Instant::now()));
                    self.attempt = self.attempt.max(n - 1);
                    self.on_closed(None, "");
                }
                self.run_actions();
                // A flush or a partial flush takes effect at once, before more of the
                // dropped audio can be heard.
                self.follow_flushes();
                self.apply_corrections();
                self.feed();
            }
            Ev::WsClosed { conn, code, reason } if conn == self.ws_gen => {
                self.on_closed(code, &reason)
            }
            Ev::StreamInfo { conn, message } if conn == self.ws_gen => {
                // It may arrive just after its stream started, or again after a resume:
                // kept by stream number, it reaches the snapshot once that stream plays
                // (the next publish picks it up). Published at once only when it is for the
                // stream heard now and changed what is kept.
                let seq = message.seq;
                let heard = self
                    .player
                    .as_ref()
                    .and_then(Player::playing_track)
                    .is_some_and(|t| u64::from(t) + 1 == seq);
                if self.stream_book.insert(message) && heard && self.takeover.is_none() {
                    self.publish_signal(true);
                }
            }
            Ev::QualityEcho { conn, level } if conn == self.ws_gen => self.on_quality_echo(level),
            Ev::Features(Some(f)) if f.protocol.is_none_or(|p| p < protocol::PROTOCOL_VERSION) => {
                // A gateway too old for this engine's protocol (the app should not have
                // started the player: this is the second line). Nothing is played from it.
                log::warn!(
                    "player: the gateway speaks protocol {:?}; {} needed: not connecting",
                    f.protocol,
                    protocol::PROTOCOL_VERSION
                );
                self.stop_for_good(PlayerConnection::UpdatePlugin);
            }
            Ev::Features(f) => {
                let f = f.unwrap_or_default();
                self.quality_feature = Some(f.stream_quality);
                self.grace_feature = Some(f.resume_grace);
                self.send_quality();
                if self.ws_gen == 0 && self.connect_at.is_some() && !self.stopped {
                    if self.network {
                        self.connect(false);
                    } else {
                        self.connect_at = None;
                        self.waiting_for_network = Some(false);
                    }
                }
            }
            Ev::SyncGroup {
                player_gen,
                grouped,
            } if player_gen == self.player_gen => self.in_sync_group = grouped,
            Ev::Position {
                player_gen,
                seq,
                fresh,
                pos,
            } if player_gen == self.player_gen => self.note_position(seq, fresh, pos),
            Ev::StreamHeaders { id, content_type } if self.is_current(id) => {
                let now = self.now_ms();
                let ct = clean_content_type(&content_type);
                if let Some(p) = &mut self.player {
                    p.on_stream_headers(
                        format!("HTTP/1.0 200 OK\r\nContent-Type: {ct}\r\n\r\n").as_bytes(),
                        now,
                    );
                }
                self.run_actions();
            }
            Ev::StreamData { id, bytes } if self.is_current(id) => {
                let Some(n) = self.stream.as_ref().map(|s| s.n) else {
                    return;
                };
                let others: u64 = self
                    .spools
                    .iter()
                    .filter(|s| s.n != n)
                    .map(Spool::disk_bytes)
                    .sum();
                let Some(spool) = self.spools.iter_mut().find(|s| s.n == n) else {
                    // The track was dropped (the core no longer takes it).
                    self.close_stream();
                    return;
                };
                spool.write(&bytes, others);
                let got = spool.received;
                if let Some(p) = &mut self.player
                    && p.open_stream().is_some_and(|(open, _)| open == n)
                {
                    p.stream_bytes_received(got);
                }
                // Acknowledged as received from the network (not what the cache supplies).
                if (n != self.last_ack.0 || got - self.last_ack.1 >= protocol::ACK_EVERY)
                    && self.open
                    && !self.from_cache.contains(&n)
                    && !self.from_downloads.contains(&n)
                {
                    self.last_ack = (n, got);
                    self.send_ws(Out::Text(protocol::ack_message(n, got)));
                }
                self.feed();
            }
            Ev::StreamEnd { id, digest } if self.is_current(id) => {
                let n = self.stream.take().map(|s| s.n);
                let now = self.now_ms();
                if let Some(spool) = self.spools.iter_mut().find(|s| Some(s.n) == n) {
                    spool.complete = true;
                }
                if let (Some(n), Some(d)) = (n, digest) {
                    self.keep_in_cache(n, d);
                }
                // The whole stream is in (`DSCO`; in read-ahead mode `STMd` may go now).
                if let Some(p) = &mut self.player
                    && p.open_stream().map(|(open, _)| open) == n
                {
                    p.stream_received(now);
                }
                self.run_actions();
                self.feed();
            }
            Ev::FromCache { id } if self.is_current(id) => {
                if let Some(n) = self.stream.as_ref().map(|s| s.n) {
                    self.from_cache.retain(|x| *x != n);
                    self.from_cache.push_back(n);
                    while self.from_cache.len() > POSITIONS_KEEP {
                        self.from_cache.pop_front();
                    }
                }
                self.publish_signal(true);
            }
            Ev::FromDownloads { id } if self.is_current(id) => {
                if let Some(n) = self.stream.as_ref().map(|s| s.n) {
                    self.from_downloads.retain(|x| *x != n);
                    self.from_downloads.push_back(n);
                    while self.from_downloads.len() > POSITIONS_KEEP {
                        self.from_downloads.pop_front();
                    }
                }
                self.publish_signal(true);
            }
            Ev::Downloads {
                player_gen,
                allowed,
            } if player_gen == self.player_gen => {
                if allowed.is_some() {
                    self.downloads_allowed = allowed;
                }
                if allowed == Some(false)
                    && let Some(c) = &self.cache
                {
                    // No download permission: no copies are kept on the phone.
                    c.lock().unwrap().clear();
                }
            }
            Ev::StreamFailed { id } if self.is_current(id) => {
                let n = self.stream.take().map(|s| s.n);
                // What was received is still fed; the track ends there.
                if let Some(spool) = self.spools.iter_mut().find(|s| Some(s.n) == n) {
                    spool.failed = true;
                }
                self.feed();
            }
            Ev::Loudness {
                player_gen,
                seq,
                measurement,
            } if player_gen == self.player_gen => {
                self.loudness.retain(|(s, _)| *s != seq);
                self.loudness.push_back((seq, measurement));
                while self.loudness.len() > LOUDNESS_KEEP {
                    self.loudness.pop_front();
                }
                if self.gate.is_some_and(|(s, _)| s == seq) {
                    self.gate = None;
                }
            }
            Ev::Facts {
                player_gen,
                seq,
                facts,
            } if player_gen == self.player_gen => {
                self.facts.retain(|(s, _)| *s != seq);
                self.facts.push_back((seq, facts));
                while self.facts.len() > LOUDNESS_KEEP {
                    self.facts.pop_front();
                }
            }
            // Stale: from an older socket or a superseded stream.
            _ => {}
        }
    }

    /// Look up the track LMS just started streaming (stream `n` carries track
    /// `seq = n - 1`), for what needs it:
    /// - **analysis levelling:** its R128 loudness. A fresh start waits briefly for it (see
    ///   [`LOUDNESS_WAIT`]); a gapless next track has until the current one ends;
    /// - **transitions:** its album and length (from LMS), and for smart transitions the
    ///   silence and fades at its ends (from its analysis). Never waited for: a boundary
    ///   whose facts are late is played gapless.
    fn look_up_loudness(&mut self, stream_number: u64) {
        let dsp = self.audio.pipeline.dsp();
        let levelling = dsp.enabled && dsp.analysis_levelling && dsp.levelling != Levelling::Off;
        let transitions = dsp_transitions(dsp);
        let smart = transitions.mode == TransitionMode::Smart;
        let (Some(mac), Ok(seq)) = (self.mac, u32::try_from(stream_number.saturating_sub(1)))
        else {
            return;
        };
        let fresh = self
            .player
            .as_ref()
            .is_none_or(|p| p.playing_track().is_none());
        if !levelling && !transitions.enabled() {
            // Only whether the player is in a sync group (for rejoining after a stall), and
            // where the track is in the queue (for restarting it after a lost session).
            self.look_up_status(Some((seq, fresh)));
            return;
        }
        if fresh {
            if levelling {
                self.gate = Some((seq, Instant::now() + LOUDNESS_WAIT));
            }
            if let Some(t) = self.lookup.take() {
                t.abort();
            }
        }
        let (core, tx, player_gen) = (self.core.clone(), self.tx.clone(), self.player_gen);
        let budget = if fresh {
            LOUDNESS_BUDGET_FRESH
        } else {
            LOUDNESS_BUDGET_NEXT
        };
        let task = tokio::spawn(async move {
            let player = format_mac(&mac);
            let status = core
                .rpc(&player, protocol::stream_status_command())
                .await
                .ok();
            if let Some(st) = &status {
                let _ = tx.send(Ev::SyncGroup {
                    player_gen,
                    grouped: protocol::in_sync_group(st),
                });
                if let Some(pos) = protocol::stream_position(st, fresh) {
                    let _ = tx.send(Ev::Position {
                        player_gen,
                        seq,
                        fresh,
                        pos,
                    });
                }
            }
            let track = status.and_then(|st| protocol::stream_track(&st, fresh));
            let mut facts = TrackFacts {
                album: track
                    .as_ref()
                    .and_then(|t| t.album_id)
                    .map(|a| a.to_string()),
                duration_s: track.as_ref().and_then(|t| t.duration_s),
                edges: None,
            };
            if transitions.enabled() {
                // What LMS said is enough for the album rule and a fixed crossfade.
                let _ = tx.send(Ev::Facts {
                    player_gen,
                    seq,
                    facts: facts.clone(),
                });
            }
            let analysis = match &track {
                Some(t) if levelling || smart => {
                    crate::analysis::fetch_waiting(&core, t.id, budget)
                        .await
                        .ok()
                        .flatten()
                }
                _ => None,
            };
            if levelling {
                let _ = tx.send(Ev::Loudness {
                    player_gen,
                    seq,
                    measurement: analysis.as_ref().and_then(|a| a.measurement()),
                });
            }
            if smart && let Some(a) = &analysis {
                facts.edges = Some(a.edges());
                if facts.duration_s.is_none() && a.duration_s.is_finite() && a.duration_s > 0.0 {
                    facts.duration_s = Some(a.duration_s);
                }
                let _ = tx.send(Ev::Facts {
                    player_gen,
                    seq,
                    facts,
                });
            }
        });
        self.lookup = Some(task.abort_handle());
    }

    /// Ask LMS whether this player is in a sync group (the same status query as for a new
    /// track's details).
    fn look_up_sync_group(&self) {
        self.look_up_status(None);
    }

    /// LMS's status for this player: whether it is in a sync group, and, for a stream just
    /// started (`stream`: its track's sequence number and whether it is a fresh start),
    /// where that track is in the queue.
    fn look_up_status(&self, stream: Option<(u32, bool)>) {
        let Some(mac) = self.mac else { return };
        let (core, tx, player_gen) = (self.core.clone(), self.tx.clone(), self.player_gen);
        tokio::spawn(async move {
            if let Ok(st) = core
                .rpc(&format_mac(&mac), protocol::stream_status_command())
                .await
            {
                let _ = tx.send(Ev::SyncGroup {
                    player_gen,
                    grouped: protocol::in_sync_group(&st),
                });
                if let Some((seq, fresh)) = stream
                    && let Some(pos) = protocol::stream_position(&st, fresh)
                {
                    let _ = tx.send(Ev::Position {
                        player_gen,
                        seq,
                        fresh,
                        pos,
                    });
                }
            }
        });
    }

    /// Remember where track `seq` is in the queue. A fresh stream may begin part-way into
    /// its track (a seek): LMS's `time` then, less what has been heard of the stream since,
    /// is where it began.
    fn note_position(&mut self, seq: u32, fresh: bool, pos: protocol::StreamPosition) {
        let heard = self
            .player
            .as_ref()
            .filter(|p| p.playing_track() == Some(seq))
            .map_or(0, |p| p.status().elapsed_ms);
        let began = if fresh {
            pos.lms_time_ms.unwrap_or(0).saturating_sub(heard)
        } else {
            0
        };
        self.positions.retain(|(s, _, _)| *s != seq);
        self.positions.push_back((seq, pos, began));
        while self.positions.len() > POSITIONS_KEEP {
            self.positions.pop_front();
        }
    }

    /// Where playback is now, as a queue position and time into the track (when known).
    fn heard_point(&self) -> Option<protocol::RestartPoint> {
        let p = self.player.as_ref()?;
        let seq = p.playing_track()?;
        let (_, pos, began) = self.positions.iter().find(|(s, _, _)| *s == seq)?;
        Some(protocol::RestartPoint {
            index: pos.index,
            track_id: pos.track_id,
            elapsed_ms: began.saturating_add(p.status().elapsed_ms),
            paused: false,
            local: false,
        })
    }

    /// While the link to LMS is down, keep note of where to restart should the session be
    /// lost (see [`protocol::watch_gap`]).
    fn watch_gap(&mut self) {
        let Some(p) = &self.player else {
            self.gap = None;
            return;
        };
        let playing = p.status().output == OutputState::Playing;
        let held = self.shared.out.hold.load(Ordering::Acquire);
        let heard = self.heard_point();
        self.gap = Some(protocol::watch_gap(self.gap, playing, held, heard));
    }

    /// A new session replaced one lost while this player was playing: once LMS has settled,
    /// restart the track heard last where it was, if LMS still has it there and isn't
    /// already playing (or paused). See [`protocol::restart_commands`].
    fn restart_where_left(&mut self, point: protocol::RestartPoint) {
        let Some(mac) = self.mac else { return };
        let core = self.core.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(RESTART_SETTLE).await;
            let player = format_mac(&mac);
            let Ok(st) = core
                .rpc(&player, protocol::restart_status_command(point.index))
                .await
            else {
                return;
            };
            let Some(cmds) = protocol::restart_commands(&point, &st) else {
                return;
            };
            for c in cmds {
                if core.rpc(&player, c).await.is_err() {
                    return;
                }
            }
        });
        self.restart = Some(task.abort_handle());
    }

    /// Drop any restart or sync-group rejoin under way: they ask LMS to move this player
    /// (and a rejoin, its whole group), which must not happen once the player has stopped,
    /// been stopped by the server, or left LMS for the phone's own copies.
    fn cancel_restarts(&mut self) {
        if let Some(t) = self.restart.take() {
            t.abort();
        }
        if let Some(t) = self.rejoin.take() {
            t.abort();
        }
    }

    /// A stalled player in a sync group had too little audio to skip the stall (it has been
    /// stalled `stalled_ms`): ask LMS to restart the group where the others are now. LMS
    /// then flushes and restarts every member at that point, in step (the others hear a
    /// short gap). Minimal by design: LMS never pulls a member back into line by itself.
    ///
    /// Where the others are is where this player stopped plus the time it has been stalled
    /// (they played on meanwhile), checked against LMS's current track first
    /// ([`protocol::rejoin_command`]: nothing is sent if the track changed, LMS isn't
    /// playing, or the point is past the track's end). Not LMS's `time ?`: with this player
    /// the group's master, that lags by however long LMS last heard from it, and the whole
    /// group went back 14 s after a 14 s stall in the live check. Only when the track's
    /// queue position was never learnt (a radio stream, or LMS didn't answer) is LMS's
    /// own `time ?` used, as before.
    fn rejoin_group(&mut self, stalled_ms: u64) {
        let Some(mac) = self.mac else { return };
        if self.rejoin.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        // Nothing has been heard since the stall began, so what was heard last is where it
        // stopped.
        let point = self.heard_point().map(|h| protocol::RejoinPoint {
            index: h.index,
            track_id: h.track_id,
            stalled_at_ms: h.elapsed_ms,
            stalled_ms,
        });
        let decided = Instant::now();
        let core = self.core.clone();
        let task = tokio::spawn(async move {
            let player = format_mac(&mac);
            let cmd = if let Some(point) = point {
                let Ok(st) = core.rpc(&player, protocol::stream_status_command()).await else {
                    return;
                };
                // The others played on while LMS was asked.
                let waited = u64::try_from(decided.elapsed().as_millis()).unwrap_or(u64::MAX);
                let point = protocol::RejoinPoint {
                    stalled_ms: point.stalled_ms.saturating_add(waited),
                    ..point
                };
                protocol::rejoin_command(&point, &st)
            } else {
                let Ok(answer) = core
                    .rpc(
                        &player,
                        vec![serde_json::json!("time"), serde_json::json!("?")],
                    )
                    .await
                else {
                    return;
                };
                protocol::group_time(&answer).map(|t| {
                    vec![
                        serde_json::json!("time"),
                        serde_json::json!(format!("{t:.3}")),
                    ]
                })
            };
            if let Some(cmd) = cmd {
                let _ = core.rpc(&player, cmd).await;
            }
        });
        self.rejoin = Some(task.abort_handle());
    }

    /// A fresh start is waiting for its track's loudness (nothing else is audible).
    fn gating(&mut self) -> bool {
        match self.gate {
            Some((_, until)) if Instant::now() < until => true,
            Some(_) => {
                self.gate = None;
                false
            }
            None => false,
        }
    }

    fn is_current(&self, id: u64) -> bool {
        self.stream.as_ref().is_some_and(|s| s.id == id)
    }

    fn run_actions(&mut self) {
        let Some(p) = &mut self.player else { return };
        let actions = p.take_actions();
        for a in actions {
            match a {
                Action::Send(b) => self.send_ws(Out::Binary(b)),
                // The gateway fetches exactly what LMS asked for; we fetch it from the
                // gateway by number.
                Action::OpenStream {
                    stream_number,
                    format,
                    ..
                } => {
                    self.open_stream(stream_number, format);
                    self.look_up_loudness(stream_number);
                }
                Action::CloseStream => self.close_stream(),
            }
        }
        self.reconcile_spools();
    }

    fn open_stream(&mut self, n: u64, format: u8) {
        self.close_stream();
        let Some((session, _)) = self.session.clone() else {
            return;
        };
        self.stream_ids += 1;
        let id = self.stream_ids;
        self.stream_formats.retain(|(s, _)| *s != n);
        self.stream_formats.push_back((n, format));
        while self.stream_formats.len() > POSITIONS_KEEP {
            self.stream_formats.pop_front();
        }
        // A spool for it, on disk where possible (an older one for this number is gone).
        self.spools.retain(|s| s.n != n);
        self.spools
            .push_back(Spool::create(n, self.spool_dir.as_ref(), id));
        let want = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(stream_task(StreamJob {
            id,
            n,
            session,
            core: self.core.clone(),
            server: self.server.clone(),
            http: self.http.clone(),
            tx: self.tx.clone(),
            want: want.clone(),
            budget: protocol::stream_retry_budget(self.grace),
            net: self.net_tx.subscribe(),
            // Every stream is fingerprinted while a cache may be used (whether to keep it
            // is decided when it ends); it is looked up only while the cache is in use.
            cache: self
                .cache
                .as_ref()
                .filter(|_| self.cache_bytes > 0 && self.spool_dir.is_some())
                .map(|c| CacheJob {
                    cache: c.clone(),
                    level: cache::level_byte(self.quality),
                    format,
                    look_up: self.active_cache().is_some(),
                }),
            // Only a FLAC stream at the Original level can be a downloaded original's.
            downloads: self
                .downloads
                .clone()
                .filter(|_| self.quality == StreamQuality::Original && format == b'f'),
        }));
        self.last_ack = (n, 0);
        self.stream = Some(StreamTask {
            id,
            n,
            want,
            task: task.abort_handle(),
        });
    }

    fn close_stream(&mut self) {
        if let Some(s) = self.stream.take() {
            s.task.abort();
        }
    }

    // --- Spools -------------------------------------------------------------------------

    /// Stream `n` ended cleanly: move its spool into the replay cache, if the whole stream
    /// is in the file and the cache is in use. Its file is then read from the cache.
    fn keep_in_cache(&mut self, n: u64, d: StreamDigest) {
        let Some(c) = self.active_cache().cloned() else {
            return;
        };
        let (track, whole) = self
            .positions
            .iter()
            .find(|(seq, _, _)| u64::from(*seq) + 1 == n)
            .map_or((None, false), |(_, p, began)| {
                (Some(p.track_id), *began < WHOLE_FROM_MS)
            });
        let Some(spool) = self.spools.iter_mut().find(|s| s.n == n) else {
            return;
        };
        let Some(from) = spool.whole_file().map(std::path::Path::to_path_buf) else {
            return;
        };
        if spool.received != d.len {
            return;
        }
        let kept = c.lock().unwrap().adopt(
            &from,
            cache::NewEntry {
                fp: d.fp,
                level: d.level,
                format: d.format,
                len: d.len,
                sha256: d.sha256,
                track,
                whole,
            },
        );
        self.sources = None;
        if let Some(to) = kept {
            spool.kept_by_cache(to, Pin::adopted(&c, d.fp));
        }
    }

    /// Keep a spool only for the streams the core still takes (a flush, a partial flush or
    /// a replaced stream drops the others, and their files go).
    fn reconcile_spools(&mut self) {
        let live = self
            .player
            .as_ref()
            .map(Player::fed_streams)
            .unwrap_or_default();
        self.spools.retain(|s| live.contains(&s.n));
        if let Some(s) = &self.stream
            && !self.spools.iter().any(|sp| sp.n == s.n)
        {
            self.close_stream();
        }
    }

    /// Delete every spool (stopping, signing out, refused, a new session).
    fn drop_spools(&mut self) {
        self.spools.clear();
        if let Some(d) = &self.spool_dir {
            d.clear();
        }
    }

    /// Hand the decoder what it wants from the spools, oldest first: the rest of a stream,
    /// then its end (or its failure) once it has all been fed. Also sets the fetch's
    /// back-pressure (only a spool kept in memory holds the network back).
    fn feed(&mut self) {
        let now = self.now_ms();
        let Some(p) = &mut self.player else { return };
        let mut budget = FEED_PER_CALL;
        while let Some(front) = self.spools.front_mut() {
            // Only ever the stream the core is feeding (they are kept in step).
            if p.fed_streams().first() != Some(&front.n) {
                break;
            }
            if !front.caught_up() {
                if budget == 0 || !p.wants_stream_data() {
                    break;
                }
                match front.read(FEED_CHUNK.min(budget)) {
                    Ok(bytes) => {
                        budget -= bytes.len();
                        p.on_stream_data(&bytes, now);
                    }
                    Err(SpoolReadError::Broken) => {
                        // The file can't give back what was written: the track ends with
                        // what was fed (never anything else).
                        let n = front.n;
                        self.spools.pop_front();
                        if self.stream.as_ref().is_some_and(|s| s.n == n)
                            && let Some(s) = self.stream.take()
                        {
                            s.task.abort();
                        }
                        p.on_stream_error(now);
                    }
                }
                continue;
            }
            if front.complete {
                p.on_stream_end(now);
                self.spools.pop_front();
                continue;
            }
            if front.failed {
                p.on_stream_error(now);
                self.spools.pop_front();
                continue;
            }
            break; // waiting for the network
        }
        if let Some(s) = &self.stream {
            let want = self
                .spools
                .iter()
                .find(|sp| sp.n == s.n)
                .is_none_or(Spool::wants_network);
            s.want.store(want, Ordering::Release);
        }
        self.run_actions();
    }

    /// Follow the core's flushes: a full one drops all audio queued for the output; a
    /// partial one (`strm f`) drops only what is held of tracks after the audible one.
    fn follow_flushes(&mut self) {
        let Some(p) = &self.player else { return };
        let (flushes, trims, trim) = (p.flush_count(), p.trim_count(), p.trim());
        if flushes != self.last_flush {
            self.last_flush = flushes;
            self.last_trim = trims;
            self.drop_queued_audio();
            self.stall_wait = None;
        } else if trims != self.last_trim {
            self.last_trim = trims;
            if let Some(t) = trim {
                self.audio.trim_after(t.keep_track);
            }
        }
        self.reconcile_spools();
    }

    /// Spooled ahead of the decoder, in bytes (for the cushion).
    fn unfed_bytes(&self) -> u64 {
        self.spools.iter().map(Spool::unfed).sum()
    }

    // --- Audio --------------------------------------------------------------------------

    /// Drop audio queued between the core and the speaker (a flush, or a new session).
    fn drop_queued_audio(&mut self) {
        self.audio.drop_queued_audio();
        self.timed_start = false;
    }

    fn on_tick(&mut self) {
        self.settle_network();
        if self.open && self.greeted_at.is_some_and(|t| t.elapsed() >= STABLE_AFTER) {
            self.greeted_at = None;
            self.attempt = 0;
        }
        if self.ws_gen == 0 && self.connect_at.is_some_and(|t| Instant::now() >= t) && !self.stopped
        {
            // `/v1/me` didn't answer in time: connect without asking for a longer grace.
            if self.network {
                self.connect(false);
            } else {
                self.connect_at = None;
                self.waiting_for_network = Some(false);
            }
        }
        if let Some((at, resume)) = self.reconnect_at
            && Instant::now() >= at
            && self.network
        {
            self.reconnect_at = None;
            // The window may have passed while waiting (for the backoff, or the network).
            let resume = resume && self.session.is_some() && self.within_grace();
            if !resume {
                self.session = None;
            }
            self.connect(resume);
        }
        let now_ns = mono_ns();
        let now = self.ms_at(now_ns);
        let ring = self.shared.out.ring.clone();
        if self.takeover.is_some() {
            self.tick_takeover();
            return;
        }

        // Frames that have become audible since the last tick, exact as of now.
        let played = self.audio.played_at(now_ns);
        if self.player.is_some() && !self.open {
            self.watch_gap();
        }
        let Some(p) = &mut self.player else {
            self.publish_status(false);
            return;
        };
        p.presented(played, now);
        p.tick(now);
        // The cushion's byte rate: the track being decoded, once a second of it is; across
        // the hand-over to the next track, the last one's.
        if let Some(rate) = p.decoded_rate() {
            self.byte_rate = Some(rate);
        }
        self.run_actions();
        self.follow_flushes();
        self.apply_corrections();
        self.feed();
        let stall_hold = self.after_stall(now);

        let src = self.player.as_mut().map(|p| p as &mut dyn AudioSource);
        self.audio.poll_output(src);
        let gate = stall_hold || self.gate.is_some_and(|(_, until)| Instant::now() < until);
        if let Some(p) = self.player.as_mut() {
            let loudness = &self.loudness;
            let facts = &self.facts;
            self.audio.fill(
                p,
                gate,
                &|seq| {
                    loudness
                        .iter()
                        .find(|(s, _)| *s == seq)
                        .and_then(|(_, m)| *m)
                },
                &|seq| {
                    facts
                        .iter()
                        .find(|(s, _)| *s == seq)
                        .map(|(_, f)| f.clone())
                },
            );
        }
        if let Some(e) = self.audio.take_error()
            && let Ok(mut st) = self.shared.status.lock()
        {
            st.last_error = Some(e);
        }
        let gating = self.gating();

        let Some(p) = &mut self.player else { return };
        let state = p.status().output;
        let playing = state == OutputState::Playing;
        // A local hold lasts until LMS next starts or resumes playback (from this app or
        // any other controller): the hold covers the moment before LMS's own pause lands.
        if playing && !self.was_playing {
            self.shared.out.hold.store(false, Ordering::Release);
        }
        self.was_playing = playing;
        let user_hold = self.shared.out.hold.load(Ordering::Acquire);
        // A timed start armed in the ring releases the hold itself, at the exact frame:
        // leave it alone until the core's start time has come.
        let timed = self.timed_start && matches!(state, OutputState::Paused { until: Some(_) });
        if self.timed_start && (!timed || user_hold) {
            self.timed_start = false;
            ring.cancel_start();
        }
        if !timed || user_hold {
            ring.set_hold(user_hold || !playing);
        }
        // While a fresh start waits for its loudness, the silence is not an underrun.
        let exhausted = p.audio_exhausted() && self.audio.idle();
        ring.set_expect_data(playing && !user_hold && !exhausted && !gating);
        let (underruns, _) = ring.underruns();
        if underruns > self.underruns_seen {
            self.underruns_seen = underruns;
            p.output_underrun();
        }
        let rate = self.audio.pipeline.out_rate().unwrap_or(48_000) as u64;
        let cap_bytes = ring.capacity() as u64 * 4;
        p.set_host_buffer(ring.len() as u64 * 4, cap_bytes.min(rate * 8 * 2));
        self.run_actions();
        self.after_last_track();
        self.publish_status(false);
        self.publish_signal(false);
    }

    /// After a stall (an underrun), once audio is back: carry on, or in a sync group get
    /// back in step with the others (A6). Returns true while the output should wait a
    /// little longer (a grouped player gathering enough audio to skip the stall).
    fn after_stall(&mut self, now: u64) -> bool {
        let Some(p) = &mut self.player else {
            return false;
        };
        let stalled = p.stall_ms(now);
        if stalled.is_none() {
            self.stall_wait = None;
            return false;
        }
        let buffered = p.status().buffered_ms;
        let waited = self.stall_wait.map_or(Duration::ZERO, |t| t.elapsed());
        match stall_step(stalled, buffered, self.in_sync_group, waited) {
            StallStep::Nothing => false,
            StallStep::Wait => {
                self.stall_wait.get_or_insert_with(Instant::now);
                true
            }
            StallStep::Resume => {
                self.stall_wait = None;
                if let StallResume::NotEnoughBuffered { stalled_ms, .. } =
                    p.resume_after_stall(self.in_sync_group, now)
                {
                    self.rejoin_group(stalled_ms);
                }
                false
            }
        }
    }

    // --- Offline controls (`super::outage`) ---------------------------------------------

    /// The link to LMS is down (no socket, or no network) with something to play: the
    /// phone's own controls apply.
    fn offline(&self) -> bool {
        (self.player.is_some() || self.takeover.is_some()) && !(self.open && self.network)
    }

    /// Look again at what is on the phone, whole (at most every [`SOURCES_EVERY`] unless
    /// `force`).
    fn refresh_sources(&mut self, force: bool) {
        if !force
            && self
                .sources
                .as_ref()
                .is_some_and(|(at, _)| at.elapsed() < SOURCES_EVERY)
        {
            return;
        }
        let mut s = Sources::default();
        if let Some(d) = &self.downloads {
            s.downloads = d.by_track();
        }
        // The cache only while it may be used (never with the download permission gone).
        if let Some(c) = self
            .cache
            .as_ref()
            .filter(|_| self.cache_bytes > 0 && self.downloads_allowed != Some(false))
        {
            let c = c.lock().unwrap();
            s.cache = c
                .whole_tracks(cache::level_byte(self.quality))
                .into_iter()
                .map(|(id, e)| {
                    let path = c.path_of(&e.fp);
                    (id, (e, path))
                })
                .collect();
        }
        s.spools = self.whole_spools();
        self.sources = Some((Instant::now(), s));
    }

    /// Streams spooled whole whose track LMS named from its start, by track: those held for
    /// a takeover, and the player's own.
    fn whole_spools(&self) -> HashMap<i64, LocalTrack> {
        let mut out: HashMap<i64, LocalTrack> = self
            .held_spools
            .iter()
            .map(|(_, id, t)| (*id, t.clone()))
            .collect();
        for sp in &self.spools {
            let Some(path) = sp.whole_file() else {
                continue;
            };
            let known = self
                .positions
                .iter()
                .find(|(seq, _, _)| u64::from(*seq) + 1 == sp.n)
                .filter(|(_, _, began)| *began < WHOLE_FROM_MS)
                .map(|(_, p, _)| p.track_id);
            let ext = self
                .stream_formats
                .iter()
                .find(|(n, _)| *n == sp.n)
                .and_then(|(_, f)| anywhere_core::decode::format_extension(*f));
            if let (Some(id), Some(ext)) = (known, ext) {
                out.insert(
                    id,
                    LocalTrack {
                        key: format!("spool:{}", sp.n),
                        track_id: id,
                        path: path.to_path_buf(),
                        ext: ext.into(),
                        duration_secs: 0.0,
                        album: None,
                    },
                );
            }
        }
        out
    }

    /// Whether queue position `i` is on the phone, whole (as last looked at).
    fn available(&self, i: usize) -> bool {
        let Some((_, s)) = &self.sources else {
            return false;
        };
        self.snapshot
            .id(i)
            .is_some_and(|id| s.availability(id) != outage::LocalAvailability::None)
    }

    /// Where playback is, for the controls (`None`: online, or nothing to play).
    fn offline_moment(&self) -> Option<outage::Moment> {
        if !self.offline() {
            return None;
        }
        if let Some(t) = &self.takeover {
            let cur = t.current();
            return Some(outage::Moment {
                cur,
                track_id: cur.and_then(|i| self.snapshot.id(i)),
                elapsed_ms: t.elapsed_ms(),
                playing: t.playing() && !t.waiting,
                loaded: true,
                local: true,
                waiting: t.waiting,
            });
        }
        let p = self.player.as_ref()?;
        let st = p.status();
        let heard = self.heard_point();
        // Where LMS said the track heard is; the app's queue decides if it moved since.
        let cur = match heard {
            Some(h) => self
                .snapshot
                .position_of(h.track_id, Some(h.index as usize)),
            None => self.snapshot.current(),
        };
        let held = self.shared.out.hold.load(Ordering::Acquire);
        Some(outage::Moment {
            cur,
            track_id: heard.map(|h| h.track_id),
            elapsed_ms: heard.map_or(st.elapsed_ms, |h| h.elapsed_ms),
            playing: st.output == OutputState::Playing && !held && !self.waiting_for_server,
            loaded: matches!(st.output, OutputState::Playing | OutputState::Paused { .. })
                && p.playing_track().is_some(),
            local: false,
            waiting: self.waiting_for_server,
        })
    }

    /// Work out what the controls can do now (none while online).
    fn update_controls(&mut self) {
        if !self.offline() {
            self.controls = None;
            return;
        }
        self.refresh_sources(false);
        self.controls = self
            .offline_moment()
            .map(|m| outage::controls(&self.snapshot, m, &|i| self.available(i)));
    }

    /// The player LMS was driving is paused (by LMS, or on the phone), or held.
    fn core_paused(&self) -> bool {
        let held = self.shared.out.hold.load(Ordering::Acquire);
        self.player
            .as_ref()
            .is_some_and(|p| held || matches!(p.status().output, OutputState::Paused { .. }))
    }

    /// Where the phone is after the user acted on it, for LMS.
    fn local_point(&self) -> Option<protocol::RestartPoint> {
        if let Some(t) = &self.takeover {
            return t.point(&self.snapshot);
        }
        let paused = self.core_paused() || self.waiting_for_server;
        self.heard_point().map(|p| protocol::RestartPoint {
            paused,
            local: true,
            ..p
        })
    }

    /// A control used on the phone while the server can't be reached. Only what the
    /// controls allow now is done (the app greys out the rest).
    fn local_command(&mut self, c: LocalCmd) {
        if !self.offline() {
            return;
        }
        self.refresh_sources(true);
        let Some(m) = self.offline_moment() else {
            return;
        };
        let gate = outage::controls(&self.snapshot, m, &|i| self.available(i));
        let cur = gate.index.map(|i| i as usize);
        let done = match c {
            LocalCmd::Play if gate.play_pause => self.local_play(),
            LocalCmd::Pause if gate.play_pause => self.local_pause(),
            LocalCmd::Next if gate.next => cur
                .and_then(|c| self.snapshot.next_of(c))
                .is_some_and(|to| self.local_go(to, 0, true)),
            LocalCmd::Previous if gate.previous => cur.is_some_and(
                |c| match outage::previous_step(&self.snapshot, c, m.elapsed_ms) {
                    outage::Back::Restart => self.local_go(c, 0, true),
                    outage::Back::To(to) => self.local_go(to, 0, true),
                },
            ),
            LocalCmd::Jump(i) if self.available(i as usize) => self.local_go(i as usize, 0, true),
            LocalCmd::Seek(ms) if gate.seek => {
                cur.is_some_and(|c| self.local_go(c, ms, m.playing || m.waiting))
            }
            _ => false,
        };
        if done {
            self.acted = true;
            self.waiting_for_server = false;
        }
        self.publish_status(true);
    }

    fn local_play(&mut self) -> bool {
        // An explicit play lifts a hold (headphones unplugged, focus lost).
        self.shared.out.hold.store(false, Ordering::Release);
        if let Some(t) = self.takeover.as_mut() {
            t.source.play();
            return true;
        }
        if self
            .player
            .as_ref()
            .is_some_and(|p| matches!(p.status().output, OutputState::Paused { .. }))
        {
            self.tell_core(b'u');
        }
        true
    }

    fn local_pause(&mut self) -> bool {
        if let Some(t) = self.takeover.as_mut() {
            t.source.pause();
            self.shared.out.ring.set_hold(true);
            return true;
        }
        if self
            .player
            .as_ref()
            .is_some_and(|p| p.status().output == OutputState::Playing)
        {
            self.tell_core(b'p');
            self.shared.out.ring.set_hold(true);
        }
        true
    }

    /// Pause (`p`) or unpause (`u`) the player LMS was driving, as LMS would. What it would
    /// tell LMS is dropped: on reconnecting, LMS is told the phone's state instead.
    fn tell_core(&mut self, command: u8) {
        self.run_actions();
        let now = self.now_ms();
        let (Some(p), Some(frame)) = (self.player.as_mut(), outage::local_strm(command)) else {
            return;
        };
        let _ = p.on_control_data(&frame, now);
        let _ = p.take_actions();
    }

    /// Play queue position `index`, `ms` in, from the phone's copies (`play`: or paused
    /// there). False if it isn't on the phone.
    fn local_go(&mut self, index: usize, ms: u64, play: bool) -> bool {
        if play {
            self.shared.out.hold.store(false, Ordering::Release);
        }
        if self.takeover.is_none() {
            return self.begin_takeover(index, ms, play);
        }
        let (Some((_, sources)), Some(t)) = (&self.sources, self.takeover.as_mut()) else {
            return false;
        };
        t.play_from(&self.snapshot, sources, index, ms, play)
    }

    /// Leave LMS's player for the phone's own copies of the queue, from `index`. Its
    /// session goes too: a new one starts when the server is back, and LMS is then told
    /// where the phone is.
    fn begin_takeover(&mut self, index: usize, ms: u64, play: bool) -> bool {
        let opener = self.opener();
        let seed = u64::from(getrandom::u32().unwrap_or(0x9e37)) << 32 | self.now_ms();
        let Some((_, sources)) = &self.sources else {
            return false;
        };
        let Some(t) = Takeover::start(&self.snapshot, sources, index, ms, play, opener, seed)
        else {
            return false;
        };
        // Streams spooled whole stay on the phone while the takeover may play them.
        let spooled = self.whole_spools();
        for sp in std::mem::take(&mut self.spools) {
            let key = format!("spool:{}", sp.n);
            if let Some((id, t)) = spooled.iter().find(|(_, t)| t.key == key) {
                self.held_spools.push((sp, *id, t.clone()));
            }
        }
        self.close_stream();
        self.player = None;
        self.drop_queued_audio();
        self.audio.forget_plan();
        self.from_cache.clear();
        self.from_downloads.clear();
        self.session = None;
        self.outbox.clear();
        self.gap = None;
        self.cancel_restarts();
        // A reconnect under way would resume LMS's session: start afresh instead.
        if let Some(task) = self.ws_task.take() {
            task.abort();
            self.ws_out = None;
            if self.network {
                let delay = protocol::backoff(self.attempt, protocol::random01());
                self.reconnect_at = Some((Instant::now() + delay, false));
            } else {
                self.waiting_for_network = Some(false);
            }
        }
        if let Some((at, _)) = self.reconnect_at {
            self.reconnect_at = Some((at, false));
        }
        if let Some(w) = self.waiting_for_network.as_mut() {
            *w = false;
        }
        self.takeover = Some(t);
        true
    }

    /// Stop playing the phone's own copies (the server is back, or the player stops).
    fn end_takeover(&mut self) {
        if let Some(mut t) = self.takeover.take() {
            t.source.stop();
            self.audio.drop_queued_audio();
            self.audio.forget_plan();
            self.shared.out.ring.set_hold(true);
        }
        self.held_spools.clear();
        self.sources = None;
    }

    /// Opens a track for the takeover: a plain file only (never a symlink), and a replay
    /// cache entry only once its whole file has been checked against its hash (once per
    /// entry; a mismatch deletes it).
    fn opener(&self) -> Opener {
        let cache = self.cache.clone();
        let checked: Mutex<HashSet<cache::Digest>> = Mutex::new(HashSet::new());
        Box::new(move |t: &LocalTrack| {
            if let Some(hex) = t.key.strip_prefix(outage::CACHE_KEY) {
                let gone = || "the copy on this phone has gone".to_string();
                let fp = cache::Digest::parse(hex).ok_or_else(gone)?;
                let c = cache.as_ref().ok_or_else(gone)?;
                if !checked.lock().unwrap().contains(&fp) {
                    let entry = c.lock().unwrap().lookup(&fp).ok_or_else(gone)?;
                    if cache::verify(&t.path, &entry, &[]).is_err() {
                        c.lock().unwrap().remove(&fp);
                        return Err("the copy on this phone doesn't match".into());
                    }
                    checked.lock().unwrap().insert(fp);
                }
            }
            let f = cache::open_plain(&t.path)
                .map_err(|e| format!("not on this phone ({})", e.kind()))?;
            anywhere_core::decode::FileDecoder::open(Box::new(f), Some(&t.ext))
        })
    }

    /// One tick of the takeover: as the offline player's, through the same audio path.
    fn tick_takeover(&mut self) {
        let ring = self.shared.out.ring.clone();
        let Some(t) = self.takeover.as_mut() else {
            return;
        };
        let played = self.audio.played();
        if played > 0 {
            t.source.played(played);
        }
        t.source.decode_ahead();
        if t.source.flush_count() != t.last_flush {
            t.last_flush = t.source.flush_count();
            self.audio.drop_queued_audio();
            self.audio.forget_plan();
        }
        self.audio
            .poll_output(Some(&mut t.source as &mut dyn AudioSource));
        let facts = t.source.fact_seqs();
        self.audio.fill(&mut t.source, false, &|_| None, &|seq| {
            facts
                .iter()
                .find(|(s, _)| *s == seq)
                .and_then(|(_, f)| f.clone())
        });
        if let Some(e) = self.audio.take_error()
            && let Ok(mut st) = self.shared.status.lock()
        {
            st.last_error = Some(e);
        }
        let playing = t.playing();
        if playing && !t.was_playing {
            self.shared.out.hold.store(false, Ordering::Release);
        }
        t.was_playing = playing;
        // The end of what the phone holds in a row: stop, and wait for the server.
        if t.check_end(ring.is_empty() && self.audio.idle()) {
            self.audio.drop_queued_audio();
            self.audio.forget_plan();
        }
        // Plays heard with no server aren't recorded here: LMS records what it plays.
        let _ = t.source.take_events();
        let user_hold = self.shared.out.hold.load(Ordering::Acquire);
        let playing = t.playing();
        ring.set_hold(user_hold || !playing);
        ring.set_expect_data(
            playing && !user_hold && !(t.source.audio_exhausted() && self.audio.idle()),
        );
        self.publish_status(false);
        self.publish_signal(false);
    }

    /// With the link down, the track LMS sent has played out and nothing follows it: carry
    /// on with the next entry if it is on the phone, or stop there and wait for the server.
    ///
    /// The track may also have ended just before the link went down, with `STMu` sent and
    /// the next track not yet sent: that counts too, within [`ENDED_GRACE`].
    fn after_last_track(&mut self) {
        if self.takeover.is_some() || self.waiting_for_server {
            return;
        }
        let Some(p) = &self.player else {
            self.ended_online_at = None;
            return;
        };
        let output = p.status().output;
        // Played out: a stop from LMS (`strm q`, a flush) forgets the track instead.
        let drained = p.playing_track().is_some()
            && p.audio_exhausted()
            && self.audio.idle()
            && self.shared.out.ring.is_empty()
            && self.spools.is_empty()
            && self.stream.is_none();
        if !self.offline() {
            self.ended_online_at = (drained && output == OutputState::Stopped)
                .then(|| self.ended_online_at.unwrap_or_else(Instant::now));
            return;
        }
        let just_ended = output == OutputState::Stopped
            && self
                .ended_online_at
                .is_some_and(|t| t.elapsed() <= ENDED_GRACE);
        if !drained || !(output == OutputState::Playing || just_ended) {
            return;
        }
        self.ended_online_at = None;
        self.refresh_sources(true);
        let cur = self.offline_moment().and_then(|m| m.cur);
        match cur.and_then(|c| self.snapshot.after_end(c)) {
            Some(next) if self.available(next) && self.begin_takeover(next, 0, true) => {
                self.acted = true;
            }
            _ => self.waiting_for_server = true,
        }
        self.publish_status(true);
    }

    /// Send LMS these commands for this player (best effort).
    fn tell_lms(&self, cmds: Vec<Vec<serde_json::Value>>) {
        let Some(mac) = self.mac else { return };
        let core = self.core.clone();
        tokio::spawn(async move {
            let player = format_mac(&mac);
            for c in cmds {
                if core.rpc(&player, c).await.is_err() {
                    return;
                }
            }
        });
    }

    // --- Reporting ----------------------------------------------------------------------

    fn current_status(&self) -> LocalPlayerStatus {
        let ring = &self.shared.out.ring;
        let (underruns, underrun_frames) = ring.underruns();
        let rate = self.audio.pipeline.out_rate().unwrap_or(0);
        let ring_ms = if rate > 0 {
            (ring.len() / CHANNELS) as u64 * 1000 / u64::from(rate)
        } else {
            0
        };
        let (playback, elapsed_ms, core_ms, error, input_bytes) = match &self.player {
            _ if self.takeover.is_some() => {
                let t = self.takeover.as_ref().expect("a takeover");
                let playback = match t.source.transport() {
                    _ if t.waiting => LocalPlayback::Paused,
                    super::local::Transport::Playing if t.source.playing_track().is_none() => {
                        LocalPlayback::Buffering
                    }
                    super::local::Transport::Playing => LocalPlayback::Playing,
                    super::local::Transport::Paused => LocalPlayback::Paused,
                    super::local::Transport::Stopped => LocalPlayback::Stopped,
                };
                (playback, t.elapsed_ms(), t.source.buffered_ms(), None, 0)
            }
            Some(p) => {
                let s = p.status();
                let playback = match s.output {
                    OutputState::Stopped => LocalPlayback::Stopped,
                    OutputState::Waiting | OutputState::Loaded => LocalPlayback::Buffering,
                    OutputState::Playing => LocalPlayback::Playing,
                    OutputState::Paused { .. } => LocalPlayback::Paused,
                };
                (
                    playback,
                    s.elapsed_ms,
                    s.buffered_ms,
                    s.last_error,
                    s.input_bytes as u64,
                )
            }
            None => (LocalPlayback::Stopped, 0, 0, None, 0),
        };
        let cushion = cushion_ms(
            core_ms + ring_ms,
            self.unfed_bytes() + input_bytes,
            self.byte_rate,
        );
        let previous_error = self
            .shared
            .status
            .lock()
            .ok()
            .and_then(|s| s.last_error.clone());
        let clock = self.audio.output_clock();
        LocalPlayerStatus {
            connection: self.connection.clone(),
            playback,
            player_id: self.mac.map(|m| format_mac(&m)).unwrap_or_default(),
            name: self.name.clone(),
            elapsed_ms,
            buffered_ms: core_ms + ring_ms,
            output_rate: rate,
            underruns,
            underrun_frames,
            held: self.shared.out.hold.load(Ordering::Acquire),
            last_error: error.map(|e| protocol::clean_name(&e)).or(previous_error),
            cushion_ms: cushion,
            resume_window_ms: protocol::resume_for(self.grace).as_millis() as u64,
            read_ahead: self.read_ahead && self.spool_dir.is_some(),
            from_cache: self.hearing_from_cache(),
            from_downloads: self.hearing_from_downloads(),
            offline_controls: self.controls.clone(),
            output_clock: clock.clock,
            output_latency_ms: clock.latency_ms,
            output_clock_errors: clock.errors_per_min,
            network_flaps: self.flaps_per_min(),
        }
    }

    /// The track heard now came from the replay cache (or, with no server, a kept stream).
    fn hearing_from_cache(&self) -> bool {
        match &self.takeover {
            Some(t) => t
                .current_track()
                .is_some_and(|c| !c.key.starts_with(outage::DOWNLOAD_KEY)),
            None => self.hearing_from(&self.from_cache),
        }
    }

    /// The track heard now came from a download.
    fn hearing_from_downloads(&self) -> bool {
        match &self.takeover {
            Some(t) => t
                .current_track()
                .is_some_and(|c| c.key.starts_with(outage::DOWNLOAD_KEY)),
            None => self.hearing_from(&self.from_downloads),
        }
    }

    /// The track heard now is one of `streams` (by number).
    fn hearing_from(&self, streams: &VecDeque<u64>) -> bool {
        self.player
            .as_ref()
            .and_then(Player::playing_track)
            .is_some_and(|seq| streams.contains(&(u64::from(seq) + 1)))
    }

    fn publish_status(&mut self, force: bool) {
        self.update_controls();
        let st = self.current_status();
        let key = (
            st.connection.clone(),
            st.playback,
            st.output_rate,
            st.held,
            st.offline_controls.as_ref().map(controls_key),
        );
        let due = self
            .last_status_at
            .is_none_or(|t| t.elapsed() >= STATUS_EVERY);
        let changed = self.last_status_key.as_ref() != Some(&key);
        if let Ok(mut s) = self.shared.status.lock() {
            *s = st.clone();
        }
        if force || changed || due {
            self.last_status_at = Some(Instant::now());
            self.last_status_key = Some(key);
            if let Ok(mut s) = self.shared.spooled.lock() {
                *s = self.whole_spools().into_keys().collect();
            }
            self.shared.observer.on_status(st);
        }
    }

    fn publish_signal(&mut self, force: bool) {
        let force = force || self.audio.take_signal_dirty();
        let mut path = match &self.takeover {
            Some(t) => self.audio.signal_path(&t.source),
            None => self.player.as_ref().and_then(|p| self.audio.signal_path(p)),
        };
        // The gateway's stream message for the stream heard: track `seq` is the core's
        // (`seq` + 1)-th `strm s`, the number the gateway counts too.
        let heard = match &self.takeover {
            Some(_) => None,
            None => self
                .player
                .as_ref()
                .and_then(Player::playing_track)
                .and_then(|seq| self.stream_book.get(u64::from(seq) + 1)),
        };
        if let Some(p) = &mut path {
            p.from_cache = self.hearing_from_cache();
            p.from_downloads = self.hearing_from_downloads();
            p.served = heard.map(|m| (&m.served).into());
            p.sent = heard.map(|m| (&m.sent).into());
        }
        if !force && path == self.last_signal {
            return;
        }
        self.last_signal = path.clone();
        if let Ok(mut s) = self.shared.signal.lock() {
            *s = path.clone();
        }
        self.shared.observer.on_signal_path(path);
    }
}

/// What of the offline controls makes a status worth sending at once when it changes.
type ControlsKey = (Option<u32>, bool, bool, bool, bool, bool, bool, bool);

fn controls_key(c: &OfflineControls) -> ControlsKey {
    (
        c.index,
        c.playing,
        c.play_pause,
        c.next,
        c.previous,
        c.seek,
        c.local,
        c.waiting_for_server,
    )
}

/// A content type from the gateway, made safe to put in a header line for LMS.
/// Protocol errors this close together count as one streak.
const BAD_FRAME_WINDOW: Duration = Duration::from_secs(300);

/// The streak a protocol error at `now` makes, after `(streak, last)`.
fn bad_frame_streak((n, last): (u32, Option<Instant>), now: Instant) -> u32 {
    match last {
        Some(t) if now.saturating_duration_since(t) < BAD_FRAME_WINDOW => n.saturating_add(1),
        _ => 1,
    }
}

fn clean_content_type(ct: &str) -> String {
    let ok =
        !ct.is_empty() && ct.len() <= 100 && ct.bytes().all(|b| b.is_ascii_graphic() || b == b' ');
    if ok {
        ct.to_string()
    } else {
        "application/octet-stream".into()
    }
}

// --- Socket task -------------------------------------------------------------------------

/// One player-socket connection: authenticate (or resume), deliver the greeting, then
/// forward frames both ways until it closes.
#[allow(clippy::too_many_arguments)]
async fn socket_task(
    conn: u64,
    core: Arc<Core>,
    server: Arc<Server>,
    resume: Option<(String, String, u64)>,
    quality: StreamQuality,
    grace: Option<u64>,
    tx: mpsc::UnboundedSender<Ev>,
    mut out_rx: mpsc::UnboundedReceiver<Out>,
    // How long nothing may arrive (see [`SOCKET_IDLE`]); shorter in tests.
    socket_idle: Duration,
) {
    let closed = |code: Option<u16>, reason: &str| Ev::WsClosed {
        conn,
        code,
        reason: reason.into(),
    };
    let token = match core.access_token().await {
        Ok(t) => t,
        // Signed out: final.
        Err(ApiError::Unauthorised) => {
            let _ = tx.send(closed(Some(1008), "signed out"));
            return;
        }
        Err(_) => {
            let _ = tx.send(closed(None, ""));
            return;
        }
    };
    let Ok(request) = server.player_url().as_str().into_client_request() else {
        let _ = tx.send(closed(Some(1008), "bad server address"));
        return;
    };
    let connector = if server.is_secure() {
        match transport::tls_config(&[b"http/1.1"]) {
            Ok(cfg) => Connector::Rustls(Arc::new(cfg)),
            Err(_) => {
                let _ = tx.send(closed(Some(1008), "TLS is not available"));
                return;
            }
        }
    } else {
        Connector::Plain
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(protocol::MAX_FRAME_MESSAGE))
        .max_frame_size(Some(protocol::MAX_FRAME_MESSAGE));
    let connecting = connect_async_tls_with_config(request, Some(config), true, Some(connector));
    let mut ws = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok((ws, _))) => ws,
        _ => {
            let _ = tx.send(closed(None, ""));
            return;
        }
    };
    let first = protocol::sign_in_message_with_grace(
        &token,
        resume
            .as_ref()
            .map(|(s, k, n)| (s.as_str(), k.as_str(), *n)),
        quality,
        grace,
    );
    if ws.send(Message::Text(first.into())).await.is_err() {
        let _ = tx.send(closed(None, ""));
        return;
    }
    let mut greeted = false;
    let mut junk_logged = false;
    // Regression: the time limit was set afresh on every pass of the loop, so the player's
    // own STATs (one a second while playing) kept a dead socket looking alive until the
    // gateway's grace had run out, and the player had to start a new session.
    let idle = tokio::time::sleep(socket_idle);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            _ = &mut idle => {
                let _ = tx.send(closed(None, ""));
                return;
            }
            msg = ws.next() => {
                // Anything counts, the gateway's WebSocket Pings (every 5 s) too: they keep
                // the socket alive while LMS is busy and sends nothing.
                idle.as_mut().reset(tokio::time::Instant::now() + socket_idle);
                let msg = match msg {
                    Some(Ok(m)) => m,
                    // Silence, end of stream or a broken socket.
                    _ => {
                        let _ = tx.send(closed(None, ""));
                        return;
                    }
                };
                match msg {
                    Message::Text(t) if !greeted => match protocol::parse_greeting(t.as_str()) {
                        Some(g) => {
                            greeted = true;
                            let _ = tx.send(Ev::WsGreeting { conn, greeting: g });
                        }
                        None => {
                            let _ = ws.close(None).await;
                            let _ = tx.send(closed(None, ""));
                            return;
                        }
                    },
                    Message::Binary(b) if greeted => {
                        let _ = tx.send(Ev::WsFrame { conn, bytes: b.to_vec() });
                    }
                    // Text after the greeting: the stream message (protocol 2), the echo of
                    // a level change, or the text keepalive. Anything else is ignored (and
                    // said once per connection); it is never counted as a frame.
                    Message::Text(t) => match protocol::parse_stream_message(t.as_str()) {
                        Some(message) => {
                            let _ = tx.send(Ev::StreamInfo { conn, message });
                        }
                        None if t.as_str() == r#"{"keepalive":true}"# => {}
                        None => {
                            if let Some(level) = protocol::parse_quality_echo(t.as_str()) {
                                let _ = tx.send(Ev::QualityEcho { conn, level });
                            } else if !junk_logged {
                                junk_logged = true;
                                log::warn!(
                                    "player socket: ignored a text message the protocol doesn't have ({} bytes)",
                                    t.len()
                                );
                            }
                        }
                    },
                    Message::Close(frame) => {
                        let (code, reason) = frame
                            .map(|f| (Some(u16::from(f.code)), f.reason.to_string()))
                            .unwrap_or((None, String::new()));
                        // Answer it (the answer is queued while reading): the close handshake
                        // completes, rather than the socket being dropped with it unsent.
                        let _ = tokio::time::timeout(CLOSE_WAIT, ws.flush()).await;
                        if protocol::close_action(code, &reason) == CloseAction::Reauthenticate
                            && let Err(ApiError::Unauthorised) = core.renew(Some(&token)).await
                        {
                            // The sign-in itself has ended.
                            let _ = tx.send(closed(Some(1008), "signed out"));
                            return;
                        }
                        let _ = tx.send(closed(code, &reason));
                        return;
                    }
                    // Pings are answered by the WebSocket layer; anything else (a binary
                    // message before the greeting) is ignored.
                    _ => {}
                }
            }
            out = out_rx.recv() => {
                let Some(out) = out else { return };
                let msg = match out {
                    Out::Binary(b) => Message::Binary(b.into()),
                    Out::Text(t) => Message::Text(t.into()),
                    Out::Close => {
                        // Leave cleanly: a close frame, then (briefly) the gateway's answer.
                        // The engine has moved on: nothing more is reported.
                        let frame = CloseFrame {
                            code: CloseCode::Normal,
                            reason: "".into(),
                        };
                        let _ = tokio::time::timeout(CLOSE_WAIT, async {
                            if ws.close(Some(frame)).await.is_ok() {
                                while let Some(Ok(_)) = ws.next().await {}
                            }
                        })
                        .await;
                        return;
                    }
                };
                if !send_within(&mut ws, msg, SOCKET_SEND_TIMEOUT).await {
                    let _ = tx.send(closed(None, ""));
                    return;
                }
            }
        }
    }
}

/// Hand `msg` to the socket within `limit`: false if that failed, or if the socket stopped
/// taking messages (a dead connection whose send buffer has filled).
async fn send_within<S>(ws: &mut S, msg: Message, limit: Duration) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    matches!(tokio::time::timeout(limit, ws.send(msg)).await, Ok(Ok(())))
}

// --- Stream task -------------------------------------------------------------------------

struct StreamJob {
    id: u64,
    n: u64,
    session: String,
    core: Arc<Core>,
    server: Arc<Server>,
    http: Http,
    tx: mpsc::UnboundedSender<Ev>,
    want: Arc<AtomicBool>,
    /// How long it may keep failing before the track is given up (the session's grace).
    budget: Duration,
    /// Whether the phone has a network.
    net: tokio::sync::watch::Receiver<bool>,
    /// The replay cache, when it is in use.
    cache: Option<CacheJob>,
    /// Downloaded originals, for a FLAC stream at the Original level.
    downloads: Option<Arc<Downloads>>,
}

/// What a stream task needs of the replay cache.
struct CacheJob {
    cache: SharedCache,
    /// The stream level ([`cache::level_byte`]) and the `strm` format byte.
    level: u8,
    format: u8,
    /// Look the stream up (the cache is in use now); otherwise only fingerprint it.
    look_up: bool,
}

/// A finished stream's fingerprint and hash, for the replay cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamDigest {
    pub fp: Digest,
    pub sha256: Digest,
    pub len: u64,
    pub level: u8,
    pub format: u8,
}

/// With something in the cache it could be, a stream's first request asks for only this
/// much (`?limit=`): enough for the fingerprint after a usual header. On a hit nothing more
/// is downloaded; on a miss the rest follows with `?offset=` (one more request).
const PREFIX_BYTES: u64 = cache::FINGERPRINT_BYTES as u64 + 32 * 1024;

/// Most first bytes held while waiting to know a stream's fingerprint window.
const MAX_HEAD: usize = cache::MAX_HEADER + cache::FINGERPRINT_BYTES + 512 * 1024;

/// Follows a stream as it arrives: its whole hash, and its fingerprint once enough of its
/// first bytes are in ([`cache::window`]).
pub(crate) struct Fingerprinter {
    level: u8,
    format: u8,
    /// Every byte so far, until the fingerprint is known (or never will be).
    head: Option<Vec<u8>>,
    fp: Option<Digest>,
    hash: StreamHash,
    len: u64,
}

impl Fingerprinter {
    pub(crate) fn new(level: u8, format: u8) -> Fingerprinter {
        Fingerprinter {
            level,
            format,
            head: Some(Vec::new()),
            fp: None,
            hash: StreamHash::new(),
            len: 0,
        }
    }

    /// Take the next bytes of the stream. True when this made the fingerprint known: the
    /// moment to look in the cache, with [`Fingerprinter::take_head`].
    pub(crate) fn push(&mut self, bytes: &[u8]) -> bool {
        self.hash.update(bytes);
        self.len += bytes.len() as u64;
        let Some(head) = &mut self.head else {
            return false;
        };
        head.extend_from_slice(bytes);
        match cache::window(head, false) {
            cache::Window::More if head.len() <= MAX_HEAD => false,
            cache::Window::At(w) => {
                self.fp = Some(cache::fingerprint(self.level, self.format, &head[..w]));
                true
            }
            // Never matched (a header too long), or too much held without knowing.
            _ => {
                self.head = None;
                false
            }
        }
    }

    /// The bytes received so far (all of them), once the fingerprint is known; held no
    /// longer after this.
    pub(crate) fn take_head(&mut self) -> Vec<u8> {
        self.head.take().unwrap_or_default()
    }

    pub(crate) fn fingerprint(&self) -> Option<Digest> {
        self.fp
    }

    /// The stream ended: its digest (a short stream's fingerprint is the whole stream).
    pub(crate) fn finish(self) -> Option<StreamDigest> {
        let fp = match (self.fp, &self.head) {
            (Some(fp), _) => fp,
            (None, Some(head)) => match cache::window(head, true) {
                cache::Window::At(w) => cache::fingerprint(self.level, self.format, &head[..w]),
                _ => return None,
            },
            (None, None) => return None,
        };
        Some(StreamDigest {
            fp,
            sha256: self.hash.finish(),
            len: self.len,
            level: self.level,
            format: self.format,
        })
    }
}

/// A checked cache entry to play the rest of a stream from.
struct Hit {
    file: std::fs::File,
    entry: cache::Entry,
    _pin: Pin,
}

/// Look in the cache for the stream whose first bytes are `received` (fingerprint `fp`).
/// A hit is used only once the whole file has been read and matches the entry's length
/// and hash and starts with `received`; a file that doesn't is deleted. `None`: carry on
/// from the network.
async fn look_in_cache(job: &CacheJob, fp: Digest, received: Vec<u8>) -> Option<Hit> {
    let (entry, path) = {
        let c = job.cache.lock().unwrap();
        let e = c.lookup(&fp)?;
        // Nothing to gain from an entry no longer than what has arrived.
        if e.len <= received.len() as u64 || e.level != job.level || e.format != job.format {
            return None;
        }
        let path = c.path_of(&fp);
        (e, path)
    };
    let pin = Pin::new(&job.cache, fp);
    let e = entry.clone();
    let checked = tokio::task::spawn_blocking(move || cache::verify(&path, &e, &received)).await;
    match checked {
        Ok(Ok(file)) => Some(Hit {
            file,
            entry,
            _pin: pin,
        }),
        _ => {
            // Corrupt or tampered with: it goes, and the network is used.
            job.cache.lock().unwrap().remove(&fp);
            drop(pin);
            None
        }
    }
}

/// How a response body ended.
enum BodyEnd {
    /// The stream ended cleanly.
    Clean,
    /// Broken mid-download, or stalled: resume from what was received.
    Broken,
    /// The engine is gone.
    Gone,
    /// The rest is in the cache.
    Hit(Hit),
    /// The rest is in a downloaded original.
    Download(Found),
}

/// Play the rest of stream `id` from a checked cache entry, at the decoder's pace, then
/// end it as the network would have. The stream's own hash is checked again at the end.
async fn play_from_cache(
    hit: Hit,
    id: u64,
    tx: &mpsc::UnboundedSender<Ev>,
    want: &AtomicBool,
    cache: &CacheJob,
    mut fpr: Fingerprinter,
) {
    use std::io::Read;
    let Hit {
        mut file,
        entry,
        _pin,
    } = hit;
    let _ = tx.send(Ev::FromCache { id });
    loop {
        while !want.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut buf = vec![0u8; cache::READ_CHUNK];
        let k = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        buf.truncate(k);
        fpr.push(&buf);
        if tx.send(Ev::StreamData { id, bytes: buf }).is_err() {
            return;
        }
        // Let the engine take each piece before the next (they share a thread).
        tokio::task::yield_now().await;
    }
    let digest = fpr.finish();
    if digest.is_some_and(|d| d.len == entry.len && d.sha256 == entry.sha256) {
        cache.cache.lock().unwrap().touch(&entry.fp);
        let _ = tx.send(Ev::StreamEnd { id, digest });
    } else {
        // The file changed under us: what was fed stands, the track ends there, and the
        // entry goes.
        cache.cache.lock().unwrap().remove(&entry.fp);
        let _ = tx.send(Ev::StreamFailed { id });
    }
}

/// The longest a stream that has received its first bytes (`?limit=`) waits for a download
/// check still running before asking the network for the rest. The check carries on
/// meanwhile and can still take over.
const DOWNLOAD_CHECK_WAIT: Duration = Duration::from_secs(3);

/// A download check running beside its stream. It reads and may decode a whole file, so
/// it never holds the stream up: what arrives meanwhile goes on to the decoder, and the
/// download takes over from wherever the stream has got to when the check is done.
struct DownloadCheck {
    answer: tokio::sync::oneshot::Receiver<Option<Found>>,
    /// Stream bytes received when it was asked (where its file is positioned).
    at: u64,
}

/// What a [`DownloadCheck`] has to say so far.
enum Checked {
    Pending,
    /// The rest of the stream, from where it has got to.
    Found(Found),
    /// No download carries this stream on: the network does.
    No,
}

impl DownloadCheck {
    /// Its answer, if it has one, positioned for `got` stream bytes received.
    fn poll(&mut self, got: u64) -> Checked {
        use tokio::sync::oneshot::error::TryRecvError;
        match self.answer.try_recv() {
            Ok(found) => self.settle(found, got),
            Err(TryRecvError::Empty) => Checked::Pending,
            Err(TryRecvError::Closed) => Checked::No,
        }
    }

    /// Wait up to `limit` for its answer.
    async fn wait(&mut self, got: u64, limit: Duration) -> Checked {
        match tokio::time::timeout(limit, &mut self.answer).await {
            Ok(Ok(found)) => self.settle(found, got),
            Ok(Err(_)) => Checked::No,
            Err(_) => Checked::Pending,
        }
    }

    /// Move a found download past what the stream brought since it was asked.
    fn settle(&self, found: Option<Found>, got: u64) -> Checked {
        use std::io::{Seek, SeekFrom};
        let Some(mut found) = found else {
            return Checked::No;
        };
        let extra = got.saturating_sub(self.at);
        if extra > found.remaining {
            return Checked::No;
        }
        if extra > 0 {
            let Ok(by) = i64::try_from(extra) else {
                return Checked::No;
            };
            if found.file.seek(SeekFrom::Current(by)).is_err() {
                return Checked::No;
            }
            found.remaining -= extra;
        }
        Checked::Found(found)
    }
}

/// Follow a FLAC stream's first bytes (`head`, while still wanted) with the next piece,
/// `data`. Once its header is in, ask the downloads (in the background) for one that
/// carries it on: a checked file, positioned where the stream carries on. Asked once;
/// `None`: not asked (yet), or never will be.
fn start_download_check(
    head: &mut Option<Vec<u8>>,
    data: &[u8],
    downloads: Option<&Arc<Downloads>>,
) -> Option<DownloadCheck> {
    let Some(d) = downloads.cloned() else {
        *head = None;
        return None;
    };
    start_check_with(head, data, move |info, len, received| {
        d.take(&info, len, &received)
    })
}

/// [`start_download_check`] with the check itself given (`take`, run on the blocking pool).
fn start_check_with(
    head: &mut Option<Vec<u8>>,
    data: &[u8],
    take: impl FnOnce(downloads::StreamInfo, usize, Vec<u8>) -> Option<Found> + Send + 'static,
) -> Option<DownloadCheck> {
    let held = head.as_mut()?;
    held.extend_from_slice(data);
    match downloads::flac_head(held) {
        FlacHead::More if held.len() <= downloads::MAX_METADATA + 2 => None,
        FlacHead::Done { info, len } if info.usable() => {
            let received = head.take()?;
            let at = received.len() as u64;
            let (tx, answer) = tokio::sync::oneshot::channel();
            tokio::task::spawn_blocking(move || {
                let _ = tx.send(take(info, len, received));
            });
            Some(DownloadCheck { answer, at })
        }
        _ => {
            *head = None;
            None
        }
    }
}

/// Play the rest of stream `id` from a checked download, at the decoder's pace, then end
/// it as the network would have. Not kept in the replay cache: the download is already on
/// the phone. A file that comes up short ends the track there.
async fn play_from_download(
    found: Found,
    id: u64,
    tx: &mpsc::UnboundedSender<Ev>,
    want: &AtomicBool,
) {
    use std::io::Read;
    let Found {
        mut file,
        remaining,
    } = found;
    let _ = tx.send(Ev::FromDownloads { id });
    let mut left = remaining;
    while left > 0 {
        while !want.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut buf = vec![0u8; cache::READ_CHUNK.min(usize::try_from(left).unwrap_or(usize::MAX))];
        let k = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        buf.truncate(k);
        left -= k as u64;
        if tx.send(Ev::StreamData { id, bytes: buf }).is_err() {
            return;
        }
        // Let the engine take each piece before the next (they share a thread).
        tokio::task::yield_now().await;
    }
    let _ = tx.send(if left == 0 {
        Ev::StreamEnd { id, digest: None }
    } else {
        Ev::StreamFailed { id }
    });
}

/// Before retrying a failed fetch: the backoff, or, with no network, until it comes back
/// (then at once). Never past `deadline`.
async fn pause_before_retry(
    net: &mut tokio::sync::watch::Receiver<bool>,
    delay: Duration,
    deadline: tokio::time::Instant,
) {
    if !*net.borrow() {
        let _ = tokio::time::timeout_at(deadline, net.wait_for(|up| *up)).await;
        return;
    }
    tokio::time::sleep_until(deadline.min(tokio::time::Instant::now() + delay)).await;
}

/// How many of the stream's first bytes are still to be asked for (`?limit=`), with `got`
/// received: `None` once there are none left (or none were asked for).
fn prefix_left(prefix_end: Option<u64>, got: u64) -> Option<u64> {
    prefix_end
        .and_then(|e| e.checked_sub(got))
        .filter(|left| *left > 0)
}

/// A stream task's watchdog: should the task panic, the engine is told the stream failed
/// (the track ends with what arrived) instead of waiting on it for ever. A task that ends
/// normally has always said how it ended.
struct StreamWatch {
    id: u64,
    tx: mpsc::UnboundedSender<Ev>,
}

impl Drop for StreamWatch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = self.tx.send(Ev::StreamFailed { id: self.id });
        }
    }
}

/// Fetch stream `n` of the session, resuming at the byte offset already delivered after a
/// network failure or a 5xx, with backoff, for as long as the gateway holds the session
/// (`budget`, from the first failure in a row); anything else fails the track.
///
/// A response that ends cleanly after bringing bytes is confirmed with one more request
/// from the offset reached ([`protocol::clean_end_action`]): a proxy can pass the gateway's
/// deliberate mid-stream end on as a clean one (issue #12). At the true end the gateway
/// answers it at once with nothing; otherwise the rest follows. A confirmation refused, or
/// failing after [`protocol::CONFIRM_RETRIES`] retries, takes the clean end as the end.
///
/// With the replay cache in use, the stream's first bytes are fingerprinted as they
/// arrive; on a checked hit the download stops and the rest comes from the cache.
async fn stream_task(job: StreamJob) {
    let _watch = StreamWatch {
        id: job.id,
        tx: job.tx.clone(),
    };
    let StreamJob {
        id,
        n,
        session,
        core,
        server,
        http,
        tx,
        want,
        budget,
        mut net,
        cache,
        downloads,
    } = job;
    let mut got = 0u64;
    let mut headers_seen = false;
    let mut tries = 0u32;
    let mut failing_since: Option<Instant> = None;
    let mut renewed = false;
    // The last response ended cleanly: this request checks that it was the end.
    let mut confirming = false;
    // Failed confirmation requests in a row.
    let mut confirm_retries = 0u32;
    let mut fpr = cache
        .as_ref()
        .map(|c| Fingerprinter::new(c.level, c.format));
    // A FLAC stream's first bytes, held until its header shows whether a download can
    // carry it on (only while some original FLAC is downloaded).
    let mut flac_head: Option<Vec<u8>> = None;
    // The check of a download for this stream, while it runs.
    let mut check: Option<DownloadCheck> = None;
    if let Some(d) = downloads.clone()
        && tokio::task::spawn_blocking(move || d.may_have_flac())
            .await
            .unwrap_or(false)
    {
        flac_head = Some(Vec::new());
    }
    // Ask for the first bytes only when the cache holds something this could be, or a
    // download might.
    let mut prefix_end = (flac_head.is_some()
        || cache
            .as_ref()
            .is_some_and(|c| c.look_up && c.cache.lock().unwrap().holds_any(c.level, c.format)))
    .then_some(PREFIX_BYTES);
    loop {
        let token = match core.access_token().await {
            Ok(t) => t,
            Err(ApiError::Unauthorised) => {
                let _ = tx.send(Ev::StreamFailed { id });
                return;
            }
            Err(_) => String::new(), // fails as a network error below and is retried
        };
        // A gateway that sent more than asked for (an older or a hostile one) leaves
        // nothing of the first bytes to ask for: the rest, from where it broke.
        let limit = prefix_left(prefix_end, got);
        if limit.is_none() {
            prefix_end = None;
        }
        let url = server.stream_url(&session, n, got, limit);
        let status = match http.get(url).bearer_auth(&token).send().await {
            Ok(resp) if resp.status().as_u16() == 200 => {
                if !headers_seen {
                    headers_seen = true;
                    let ct = resp
                        .headers()
                        .get(hyper::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let _ = tx.send(Ev::StreamHeaders {
                        id,
                        content_type: ct,
                    });
                }
                let mut body = resp.into_body();
                // Where this response started: `got - from` is what it brought.
                let from = got;
                let end = loop {
                    // Back-pressure: stop reading while the core's input is full.
                    while !want.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    match tokio::time::timeout(STREAM_READ_TIMEOUT, body.frame()).await {
                        Ok(Some(Ok(frame))) => {
                            let Ok(data) = frame.into_data() else {
                                continue;
                            };
                            if data.is_empty() {
                                continue;
                            }
                            got += data.len() as u64;
                            tries = 0;
                            failing_since = None;
                            // More audio: the clean end before was not the end.
                            confirming = false;
                            let known = fpr.as_mut().is_some_and(|f| f.push(&data));
                            if tx
                                .send(Ev::StreamData {
                                    id,
                                    bytes: data.to_vec(),
                                })
                                .is_err()
                            {
                                break BodyEnd::Gone;
                            }
                            if check.is_none() {
                                check =
                                    start_download_check(&mut flac_head, &data, downloads.as_ref());
                            }
                            if let Some(c) = &mut check {
                                match c.poll(got) {
                                    Checked::Found(found) => break BodyEnd::Download(found),
                                    Checked::No => check = None,
                                    Checked::Pending => {}
                                }
                            }
                            if known
                                && let (Some(c), Some(f)) = (&cache, &mut fpr)
                                && c.look_up
                                && let Some(fp) = f.fingerprint()
                                && let Some(hit) = look_in_cache(c, fp, f.take_head()).await
                            {
                                break BodyEnd::Hit(hit);
                            }
                        }
                        Ok(None) => break BodyEnd::Clean,
                        _ => break BodyEnd::Broken,
                    }
                };
                // A hit stops the download here; the next `strm` ends the gateway's relay.
                drop(body);
                match end {
                    BodyEnd::Gone => return,
                    BodyEnd::Clean if prefix_end == Some(got) => {
                        // The first bytes, and no hit: the rest from the network, unless a
                        // download check about to answer finds the rest on the phone.
                        prefix_end = None;
                        if let Some(c) = &mut check {
                            match c.wait(got, DOWNLOAD_CHECK_WAIT).await {
                                Checked::Found(found) => {
                                    play_from_download(found, id, &tx, &want).await;
                                    return;
                                }
                                Checked::No => check = None,
                                Checked::Pending => {}
                            }
                        }
                        continue;
                    }
                    BodyEnd::Clean
                        if protocol::clean_end_action(got - from)
                            == protocol::CleanEnd::Confirm =>
                    {
                        // Maybe not the end: ask again from here, straight away.
                        confirming = true;
                        confirm_retries = 0;
                        continue;
                    }
                    BodyEnd::Clean => {
                        let digest = fpr.and_then(Fingerprinter::finish);
                        let _ = tx.send(Ev::StreamEnd { id, digest });
                        return;
                    }
                    BodyEnd::Hit(hit) => {
                        if let (Some(c), Some(f)) = (&cache, fpr) {
                            play_from_cache(hit, id, &tx, &want, c, f).await;
                        }
                        return;
                    }
                    BodyEnd::Download(found) => {
                        play_from_download(found, id, &tx, &want).await;
                        return;
                    }
                    BodyEnd::Broken => 0,
                }
            }
            Ok(resp) if resp.status().as_u16() == 401 && !renewed => {
                // An expired token: renew once and try again straight away.
                renewed = true;
                let _ = core.renew(Some(&token)).await;
                continue;
            }
            Ok(resp) => resp.status().as_u16(),
            Err(_) => 0,
        };
        if confirming {
            if protocol::confirm_gives_up(status, confirm_retries) {
                // Can't tell: take the clean end as the track's end, as before.
                let digest = fpr.and_then(Fingerprinter::finish);
                let _ = tx.send(Ev::StreamEnd { id, digest });
                return;
            }
            confirm_retries += 1;
        }
        let since = *failing_since.get_or_insert_with(Instant::now);
        if !protocol::stream_should_retry(status)
            || !protocol::stream_retry_allowed(since.elapsed(), budget)
        {
            let _ = tx.send(Ev::StreamFailed { id });
            return;
        }
        let deadline = tokio::time::Instant::from_std(since + budget);
        let was_down = !*net.borrow();
        pause_before_retry(
            &mut net,
            protocol::backoff(tries, protocol::random01()),
            deadline,
        )
        .await;
        // Back from no network: the next try is the first of a new run.
        tries = if was_down { 0 } else { tries + 1 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_bytes_left_never_underflow() {
        assert_eq!(prefix_left(None, 0), None);
        assert_eq!(prefix_left(Some(100), 0), Some(100));
        assert_eq!(prefix_left(Some(100), 40), Some(60));
        assert_eq!(prefix_left(Some(100), 100), None);
        // More than was asked for arrived (M10): nothing left, no underflow.
        assert_eq!(prefix_left(Some(100), 101), None);
        assert_eq!(prefix_left(Some(100), u64::MAX), None);
    }

    /// Regression (review, 15 October): the check of a download standing in for a stream
    /// (up to a 16 MiB header read and a full FLAC decode) ran in the stream task, which
    /// stopped reading the stream until it was done. Asking now returns at once; the stream
    /// carries on, and once the check answers, the download takes over from wherever the
    /// stream has got to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_slow_download_check_never_holds_the_stream_up() {
        use std::io::{Read, Seek, SeekFrom};
        let pcm = downloads::tests::noise(4000, 3);
        let file = downloads::tests::flac(44_100, &pcm, &[], None);
        let path = std::env::temp_dir().join(format!(
            "anywhere-check-{}-{}",
            std::process::id(),
            protocol::random01().to_bits()
        ));
        std::fs::write(&path, &file).unwrap();
        let len = file.len() as u64;
        let slow = |path: std::path::PathBuf, delay: Duration| {
            move |_: downloads::StreamInfo, _: usize, received: Vec<u8>| {
                std::thread::sleep(delay);
                let mut f = std::fs::File::open(&path).unwrap();
                f.seek(SeekFrom::Start(received.len() as u64)).unwrap();
                Some(Found {
                    file: f,
                    remaining: len - received.len() as u64,
                })
            }
        };

        // The header and a little audio arrive in the first piece.
        let mut head = Some(Vec::new());
        let t = Instant::now();
        let mut check = start_check_with(
            &mut head,
            &file[..100],
            slow(path.clone(), Duration::from_millis(800)),
        )
        .expect("asked once the header is in");
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "asking returns at once"
        );
        assert!(head.is_none(), "asked once");
        assert!(matches!(check.poll(100), Checked::Pending));
        // More of the stream arrives while the check runs.
        let got = 700u64;
        let mut found = match check.wait(got, Duration::from_secs(10)).await {
            Checked::Found(f) => f,
            _ => panic!("the check's answer"),
        };
        assert_eq!(found.remaining, len - got);
        let mut rest = Vec::new();
        found.file.read_to_end(&mut rest).unwrap();
        assert_eq!(
            rest,
            &file[got as usize..],
            "carried on where the stream got to"
        );

        // A check still running when the wait is up leaves the stream to the network for
        // now, and can still answer later.
        let mut head = Some(Vec::new());
        let mut check = start_check_with(
            &mut head,
            &file[..100],
            slow(path.clone(), Duration::from_millis(600)),
        )
        .unwrap();
        assert!(matches!(
            check.wait(100, Duration::from_millis(50)).await,
            Checked::Pending
        ));
        assert!(matches!(
            check.wait(100, Duration::from_secs(10)).await,
            Checked::Found(_)
        ));

        // A stream already past the download's end, or no download: the network.
        let mut head = Some(Vec::new());
        let mut check =
            start_check_with(&mut head, &file[..100], slow(path.clone(), Duration::ZERO)).unwrap();
        assert!(matches!(
            check.wait(len + 1, Duration::from_secs(10)).await,
            Checked::No
        ));
        let mut head = Some(Vec::new());
        let mut check = start_check_with(&mut head, &file[..100], |_, _, _| None).unwrap();
        assert!(matches!(
            check.wait(100, Duration::from_secs(10)).await,
            Checked::No
        ));
        // Not FLAC: never asked, and the stream stops being held for it.
        let mut head = Some(Vec::new());
        assert!(start_check_with(&mut head, b"ID3\x04 an mp3", |_, _, _| None).is_none());
        assert!(head.is_none());
        let _ = std::fs::remove_file(&path);
    }

    /// A socket that stops taking messages (its connection died with the send buffer full)
    /// is given up after [`SOCKET_SEND_TIMEOUT`], well before the gateway's grace ends;
    /// one that takes them is not.
    #[tokio::test]
    async fn a_socket_that_stops_taking_messages_is_given_up() {
        use std::pin::Pin;
        use std::task::{Context, Poll};
        /// Never ready, as a socket whose peer has stopped reading.
        struct Stuck;
        impl futures_util::Sink<Message> for Stuck {
            type Error = ();
            fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
                Poll::Pending
            }
            fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), ()> {
                Ok(())
            }
            fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
                Poll::Pending
            }
            fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
                Poll::Pending
            }
        }
        let t = Instant::now();
        let limit = Duration::from_millis(100);
        assert!(!send_within(&mut Stuck, Message::Text("STAT".into()), limit).await);
        assert!(t.elapsed() >= limit && t.elapsed() < Duration::from_secs(5));
        let mut fine = futures_util::sink::drain::<Message>().sink_map_err(|_| ());
        assert!(send_within(&mut fine, Message::Text("STAT".into()), limit).await);
        let mut broken = Box::pin(
            futures_util::sink::drain::<Message>()
                .sink_map_err(|_| ())
                .with(|_: Message| async { Err::<Message, ()>(()) }),
        );
        assert!(!send_within(&mut broken, Message::Text("STAT".into()), limit).await);
        // Given up on before the socket counts as idle, and long before the gateway's 60 s
        // grace for a dropped player runs out.
        const { assert!(SOCKET_SEND_TIMEOUT.as_secs() < SOCKET_IDLE.as_secs()) };
        const { assert!(SOCKET_SEND_TIMEOUT.as_secs() <= 30) };
    }

    #[tokio::test]
    async fn a_stream_task_that_panics_still_reports_a_failure() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = tokio::spawn(async move {
            let _watch = StreamWatch { id: 7, tx };
            panic!("boom");
        });
        assert!(t.await.unwrap_err().is_panic());
        assert!(matches!(rx.recv().await, Some(Ev::StreamFailed { id: 7 })));
        // A task that ends normally says nothing more.
        let (tx, mut rx) = mpsc::unbounded_channel();
        drop(StreamWatch { id: 8, tx });
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn bad_frames_close_together_back_off_further() {
        // Regression: a server sending a bad frame after every resume was reconnected to
        // at once, for ever (the greeting reset the backoff each time).
        let t0 = Instant::now();
        assert_eq!(bad_frame_streak((0, None), t0), 1);
        let n2 = bad_frame_streak((1, Some(t0)), t0 + Duration::from_secs(2));
        assert_eq!(n2, 2);
        assert_eq!(
            bad_frame_streak((n2, Some(t0)), t0 + Duration::from_secs(4)),
            3
        );
        // Long after: a new streak.
        assert_eq!(
            bad_frame_streak(
                (7, Some(t0)),
                t0 + BAD_FRAME_WINDOW + Duration::from_secs(1)
            ),
            1
        );
        // The streak sets the backoff: it grows with each one.
        assert!(
            protocol::backoff(2, 0.5) > protocol::backoff(0, 0.5),
            "later attempts wait longer"
        );
    }

    #[test]
    fn after_a_stall_alone_carries_on_and_a_group_waits_to_skip() {
        let w0 = Duration::ZERO;
        assert_eq!(stall_step(None, 500, true, w0), StallStep::Nothing);
        assert_eq!(
            stall_step(Some(800), 0, true, w0),
            StallStep::Nothing,
            "no audio yet"
        );
        assert_eq!(
            stall_step(Some(800), 10, false, w0),
            StallStep::Resume,
            "alone"
        );
        assert_eq!(
            stall_step(Some(40), 10, true, w0),
            StallStep::Resume,
            "too short to skip"
        );
        assert_eq!(stall_step(Some(800), 500, true, w0), StallStep::Wait);
        assert_eq!(
            stall_step(Some(800), 800 + REJOIN_MARGIN_MS, true, w0),
            StallStep::Resume,
            "enough to skip the stall"
        );
        assert_eq!(
            stall_step(Some(800), 500, true, REJOIN_WAIT),
            StallStep::Resume,
            "waited long enough: the core decides (skip or restart)"
        );
    }

    #[test]
    fn the_cushion_counts_decoded_audio_and_undecoded_bytes_at_the_tracks_byte_rate() {
        // No rate yet: only what is decoded.
        assert_eq!(cushion_ms(3000, 10_000_000, None), 3000);
        // 176.4 kB/s (CD-quality PCM): 17.64 MB undecoded is 100 s.
        assert_eq!(cushion_ms(10_000, 17_640_000, Some(176.4)), 110_000);
        // Junk never panics or overflows.
        assert_eq!(cushion_ms(5, u64::MAX, Some(1e-300)), 5 + 1_000_000_000_000);
        assert_eq!(cushion_ms(5, 100, Some(f64::NAN)), 5);
        assert_eq!(cushion_ms(5, 100, Some(0.0)), 5);
        assert_eq!(cushion_ms(5, 100, Some(-1.0)), 5);
        assert_eq!(cushion_ms(5, 100, Some(f64::INFINITY)), 5);
        assert_eq!(cushion_ms(u64::MAX, u64::MAX, Some(0.5)), u64::MAX);
    }

    /// Retries back off normally; with no network they wait for it (and go at once when
    /// it returns), never past the budget's end.
    #[test]
    fn retries_wait_for_the_network_within_the_budget() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, mut rx) = tokio::sync::watch::channel(true);
            let far = tokio::time::Instant::now() + Duration::from_secs(60);
            // Network up: the backoff.
            let t = Instant::now();
            pause_before_retry(&mut rx, Duration::from_millis(150), far).await;
            assert!(t.elapsed() >= Duration::from_millis(140));
            // The backoff never runs past the budget.
            let t = Instant::now();
            let soon = tokio::time::Instant::now() + Duration::from_millis(50);
            pause_before_retry(&mut rx, Duration::from_secs(10), soon).await;
            assert!(t.elapsed() < Duration::from_secs(1));
            // No network: waits, then goes at once when it returns.
            tx.send(false).unwrap();
            let t = Instant::now();
            let back = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                tx.send(true).unwrap();
                tx
            });
            pause_before_retry(&mut rx, Duration::from_secs(10), far).await;
            let waited = t.elapsed();
            assert!(waited >= Duration::from_millis(190) && waited < Duration::from_secs(2));
            // No network until the budget ends: gives up waiting then.
            let tx = back.await.unwrap();
            tx.send(false).unwrap();
            let t = Instant::now();
            let soon = tokio::time::Instant::now() + Duration::from_millis(100);
            pause_before_retry(&mut rx, Duration::from_millis(1), soon).await;
            assert!(t.elapsed() >= Duration::from_millis(90));
            assert!(t.elapsed() < Duration::from_secs(2));
        });
    }

    /// Feed `bytes` in `chunk`-sized pieces; the fingerprint, the bytes held when it became
    /// known, and the final digest.
    fn run_fingerprinter(
        bytes: &[u8],
        chunk: usize,
        level: u8,
        format: u8,
    ) -> (Option<Digest>, usize, Option<StreamDigest>) {
        let mut f = Fingerprinter::new(level, format);
        let mut held = 0;
        for c in bytes.chunks(chunk.max(1)) {
            if f.push(c) {
                held = f.take_head().len();
            }
        }
        let fp = f.fingerprint();
        (fp, held, f.finish())
    }

    #[test]
    fn fingerprints_same_different_short_and_seeked_streams() {
        let track: Vec<u8> = (0..400_000u32).map(|i| (i * 31 % 251) as u8).collect();
        // The same stream, however it is cut up on the way: the same fingerprint and hash.
        let (a, held, da) = run_fingerprinter(&track, 16 * 1024, 1, b'u');
        let (b, _, db) = run_fingerprinter(&track, 7_777, 1, b'u');
        assert!(a.is_some());
        assert_eq!(a, b);
        assert_eq!(da, db);
        let d = da.unwrap();
        assert_eq!(d.len, track.len() as u64);
        assert_eq!((d.level, d.format), (1, b'u'));
        assert_eq!(Some(d.fp), a);
        // Known after the first 64 KiB (plus the rest of the piece that crossed it), and
        // nothing held after that.
        assert!((cache::FINGERPRINT_BYTES..cache::FINGERPRINT_BYTES + 16 * 1024).contains(&held));
        // Another stream, another level, another format: different.
        let mut other = track.clone();
        other[100] ^= 1;
        assert_ne!(run_fingerprinter(&other, 16 * 1024, 1, b'u').0, a);
        assert_ne!(run_fingerprinter(&track, 16 * 1024, 2, b'u').0, a);
        assert_ne!(run_fingerprinter(&track, 16 * 1024, 1, b'f').0, a);
        // A change past the window: the same fingerprint, a different hash (which a hit
        // must match).
        let mut tail = track.clone();
        tail[300_000] ^= 1;
        let (t, _, dt) = run_fingerprinter(&tail, 16 * 1024, 1, b'u');
        assert_eq!(t, a);
        assert_ne!(dt.unwrap().sha256, d.sha256);
        // A short stream: never known while it arrives (nothing to match), but its digest
        // covers all of it.
        let short = &track[..10_000];
        let (s, _, ds) = run_fingerprinter(short, 4096, 1, b'u');
        assert_eq!(s, None);
        let ds = ds.unwrap();
        assert_eq!(ds.len, 10_000);
        assert_eq!(ds.fp, cache::fingerprint(1, b'u', short));
        // A stream after a seek starts elsewhere in the track: different.
        assert_ne!(run_fingerprinter(&track[50_000..], 16 * 1024, 1, b'u').0, a);
        // An empty stream: a digest of nothing, never a panic.
        assert_eq!(run_fingerprinter(&[], 1, 0, b'f').2.unwrap().len, 0);
    }

    #[test]
    fn a_header_too_long_is_never_matched_but_still_hashed() {
        let mut flac = b"fLaC".to_vec();
        flac.extend([0x86, 0x20, 0, 0]); // 2 MiB of metadata
        flac.extend(vec![0u8; 3 * 1024 * 1024]);
        let (fp, _, d) = run_fingerprinter(&flac, 64 * 1024, 0, b'f');
        assert_eq!(fp, None);
        assert_eq!(d, None, "nothing to keep it under");
    }

    #[test]
    fn content_types_are_made_header_safe() {
        assert_eq!(clean_content_type("audio/flac"), "audio/flac");
        assert_eq!(
            clean_content_type("audio/x-flac; charset=x"),
            "audio/x-flac; charset=x"
        );
        for bad in [
            "",
            "audio/flac\r\nX-Evil: 1",
            "a\u{0}b",
            &"x".repeat(101),
            "é",
        ] {
            assert_eq!(
                clean_content_type(bad),
                "application/octet-stream",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn capabilities_are_well_formed_tokens() {
        let c = codecs();
        assert!(c.contains(&"flc".to_string()));
        assert!(!c.contains(&"pcm".to_string()), "raw PCM can't be probed");
        for mp4 in ["alc", "aac"] {
            assert!(!c.contains(&mp4.to_string()), "MP4 needs seeking: {mp4}");
        }
        for t in &c {
            assert!(
                t.len() <= 8
                    && t.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            );
        }
    }

    /// While LMS is busy it sends nothing for up to a minute; the gateway's WebSocket Pings
    /// must keep the player's socket alive (and be answered), and only true silence ends it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pings_keep_a_quiet_socket_alive_and_are_answered() {
        use std::sync::atomic::AtomicUsize;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let pongs = Arc::new(AtomicUsize::new(0));
        let stop_pinging = Arc::new(AtomicBool::new(false));
        let (p, stop) = (pongs.clone(), stop_pinging.clone());
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            // The sign-in, then the greeting.
            let Some(Ok(Message::Text(_))) = ws.next().await else {
                panic!("no sign-in")
            };
            let greeting = serde_json::json!({
                "mac": "02:4c:41:00:00:01", "name": "Test phone",
                "session": "0123456789abcdef0123456789abcdef",
                "secret": "fedcba9876543210fedcba9876543210",
                "resumed": false,
            });
            ws.send(Message::Text(greeting.to_string().into()))
                .await
                .unwrap();
            let mut tick = tokio::time::interval(Duration::from_millis(200));
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if !stop.load(Ordering::SeqCst)
                            && ws.send(Message::Ping(b"k".to_vec().into())).await.is_err()
                        {
                            return;
                        }
                    }
                    m = ws.next() => match m {
                        Some(Ok(Message::Pong(b))) if b.as_ref() == b"k" => {
                            p.fetch_add(1, Ordering::SeqCst);
                        }
                        Some(Ok(_)) => {}
                        _ => return,
                    },
                }
            }
        });

        let server = Server::new(format!("http://127.0.0.1:{}", addr.port())).unwrap();
        let session = crate::session::signed_in_for_test(server.clone());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_out_tx, out_rx) = mpsc::unbounded_channel();
        let idle = Duration::from_millis(800);
        tokio::spawn(socket_task(
            1,
            session.core().clone(),
            server,
            None,
            StreamQuality::Original,
            None,
            tx,
            out_rx,
            idle,
        ));
        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(
            matches!(first, Ok(Some(Ev::WsGreeting { conn: 1, .. }))),
            "greeted"
        );
        // Several idle limits of nothing but Pings: the socket stays open.
        let quiet = tokio::time::timeout(idle * 4, rx.recv()).await;
        assert!(quiet.is_err(), "pings must keep the socket alive");
        assert!(
            pongs.load(Ordering::SeqCst) >= 5,
            "every ping is answered ({} pongs)",
            pongs.load(Ordering::SeqCst)
        );
        // True silence: given up after the idle limit.
        stop_pinging.store(true, Ordering::SeqCst);
        let closed = tokio::time::timeout(idle * 4, rx.recv()).await;
        assert!(
            matches!(
                closed,
                Ok(Some(Ev::WsClosed {
                    conn: 1,
                    code: None,
                    ..
                }))
            ),
            "a silent socket is given up"
        );
    }
}
