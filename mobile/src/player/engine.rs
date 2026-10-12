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
//! - a broken download resumes at the byte offset the core has, and is acknowledged at
//!   least every 256 KiB so the gateway's replay window always covers it;
//! - an expired access token is renewed and the socket reconnects; a refusal or a
//!   takeover by another connection stops for good.
//!
//! Only this thread touches the core player, the pipeline and the output; the real-time
//! output callback only reads the ring.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, connect_async_tls_with_config};

use anywhere_core::player::{Action, OutputState, Player, PlayerConfig};
use anywhere_core::proto::{ClientMsg, format_mac};

use super::audio::{AudioPath, AudioSource, OutputShared, dsp_transitions};
use super::clock::mono_ns;
use super::output::AudioOutput;
use super::pipeline::{DspSettings, Levelling};
use super::protocol::{self, CloseAction, Greeting};
use super::ring::CHANNELS;
use super::transitions::{TrackFacts, TransitionMode};
use super::{
    LocalPlayback, LocalPlayerObserver, LocalPlayerStatus, PlayerConnection, SignalPathInfo,
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
/// gateway's 60 s grace. Only what arrives counts: the player's own STATs going out say
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
    StreamHeaders {
        id: u64,
        content_type: String,
    },
    StreamData {
        id: u64,
        bytes: Vec<u8>,
    },
    StreamEnd {
        id: u64,
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
}

/// State shared with the app-facing [`super::LocalPlayer`].
pub(crate) struct Shared {
    pub status: Mutex<LocalPlayerStatus>,
    pub signal: Mutex<Option<SignalPathInfo>>,
    pub observer: Arc<dyn LocalPlayerObserver>,
    /// The ring, the hold, the route and the negotiator.
    pub out: Arc<OutputShared>,
}

struct StreamTask {
    id: u64,
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
    session: Option<(String, String)>,
    received: u64,
    ws_gen: u64,
    ws_out: Option<mpsc::UnboundedSender<Out>>,
    ws_task: Option<tokio::task::AbortHandle>,
    open: bool,
    attempt: u32,
    disconnected_at: Option<Instant>,
    reconnect_at: Option<(Instant, bool)>,
    renewals_in_a_row: u32,
    outbox: VecDeque<Vec<u8>>,
    connection: PlayerConnection,
    // Player.
    player: Option<Player>,
    mac: Option<[u8; 6]>,
    name: String,
    stream: Option<StreamTask>,
    stream_ids: u64,
    last_ack: (u64, u64),
    last_flush: u64,
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
    last_status_key: Option<(PlayerConnection, LocalPlayback, u32, bool)>,
    last_signal: Option<SignalPathInfo>,
    stopped: bool,
}

impl Engine {
    pub(crate) fn new(
        core: Arc<Core>,
        shared: Arc<Shared>,
        output: Box<dyn AudioOutput>,
        dsp: DspSettings,
        max_rate: u32,
        cmds: mpsc::UnboundedReceiver<Cmd>,
    ) -> Result<Engine, ApiError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let audio = AudioPath::new(shared.out.clone(), output, dsp)?;
        Ok(Engine {
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
            session: None,
            received: 0,
            ws_gen: 0,
            ws_out: None,
            ws_task: None,
            open: false,
            attempt: 0,
            disconnected_at: None,
            reconnect_at: None,
            renewals_in_a_row: 0,
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
        self.connect(false);
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
                    // The app let go of the player, or asked to stop.
                    Some(Cmd::Stop) | None => self.shutdown("stopped").await,
                },
                _ = tick.tick() => self.on_tick(),
            }
        }
    }

    /// Leave LMS cleanly: `BYE!` (the player disappears from LMS at once), then close.
    async fn shutdown(&mut self, reason: &str) {
        if self.open
            && let Some(out) = &self.ws_out
        {
            let bye = ClientMsg::Bye.encode();
            let _ = out.send(Out::Binary(bye));
            // Give the socket task a moment to deliver it.
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        self.close_stream();
        if let Some(t) = self.ws_task.take() {
            t.abort();
        }
        self.close_output();
        self.shared.out.ring.set_hold(true);
        self.connection = PlayerConnection::Stopped {
            reason: reason.into(),
        };
        self.player = None;
        self.publish_status(true);
        self.publish_signal(true);
        self.stopped = true;
    }

    /// Close the output for good (stopping): Android's normal mixing is restored.
    fn close_output(&mut self) {
        let src = self.player.as_mut().map(|p| p as &mut dyn AudioSource);
        self.audio.close_output(src);
    }

    // --- Connection ---------------------------------------------------------------------

    fn connect(&mut self, resume: bool) {
        if let Some(t) = self.ws_task.take() {
            t.abort();
        }
        self.ws_gen += 1;
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        self.ws_out = Some(out_tx);
        self.open = false;
        self.connection = if self.player.is_some() {
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
        let task = tokio::spawn(socket_task(
            self.ws_gen,
            self.core.clone(),
            self.server.clone(),
            resume,
            self.tx.clone(),
            out_rx,
        ));
        self.ws_task = Some(task.abort_handle());
        self.publish_status(true);
    }

    fn on_greeting(&mut self, g: Greeting) {
        self.open = true;
        self.attempt = 0;
        self.renewals_in_a_row = 0;
        self.disconnected_at = None;
        self.connection = PlayerConnection::Connected;
        let fresh = !g.resumed || self.player.is_none();
        self.session = Some((g.session.clone(), g.secret.clone()));
        if fresh {
            // A new session: LMS sees a new connection, so start from a clean player.
            self.close_stream();
            self.outbox.clear();
            self.received = 0;
            self.drop_queued_audio();
            // Track numbers start again with the new player.
            self.loudness.clear();
            self.facts.clear();
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
            self.player = Some(p);
            self.mac = Some(g.mac);
            self.name = g.name;
        }
        let queued: Vec<_> = self.outbox.drain(..).collect();
        for b in queued {
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
        let was_open = self.open;
        self.open = false;
        self.ws_out = None;
        self.ws_task = None;
        if was_open || self.disconnected_at.is_none() {
            self.disconnected_at.get_or_insert_with(Instant::now);
        }
        let action = protocol::close_action(code, reason);
        let resume = match action {
            CloseAction::Stop => {
                self.close_stream();
                self.close_output();
                self.audio.pipeline.unconfigure();
                self.shared.out.ring.set_hold(true);
                self.player = None;
                self.connection = PlayerConnection::Stopped {
                    reason: if reason.is_empty() {
                        "refused".into()
                    } else {
                        protocol::clean_name(reason)
                    },
                };
                self.stopped_by_server();
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
        let within_grace = self
            .disconnected_at
            .is_some_and(|t| t.elapsed() < protocol::RESUME_FOR);
        let resume = resume && self.session.is_some() && within_grace;
        if !resume {
            self.session = None;
        }
        let delay = protocol::backoff(self.attempt, protocol::random01());
        self.attempt = self.attempt.saturating_add(1);
        self.reconnect_at = Some((Instant::now() + delay, resume));
        self.connection = PlayerConnection::Reconnecting {
            delay_ms: delay.as_millis() as u64,
        };
        self.publish_status(true);
    }

    fn stopped_by_server(&mut self) {
        self.stopped = true;
        self.publish_status(true);
        self.publish_signal(true);
    }

    fn send_ws(&mut self, msg: Out) {
        if let (true, Some(out)) = (self.open, &self.ws_out) {
            let _ = out.send(msg);
        } else if let Out::Binary(b) = msg {
            // Held until the session resumes; acknowledgements are only useful live.
            self.outbox.push_back(b);
            while self.outbox.len() > protocol::OUTBOX_LIMIT {
                self.outbox.pop_front();
            }
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
                    // A protocol error: drop the socket and resume.
                    if let Some(t) = self.ws_task.take() {
                        t.abort();
                    }
                    self.on_closed(None, "");
                }
                self.run_actions();
                self.apply_corrections();
            }
            Ev::WsClosed { conn, code, reason } if conn == self.ws_gen => {
                self.on_closed(code, &reason)
            }
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
                let now = self.now_ms();
                let Some(p) = &mut self.player else { return };
                p.on_stream_data(&bytes, now);
                let want = p.wants_stream_data();
                let open = p.open_stream();
                if let Some(s) = &self.stream {
                    s.want.store(want, Ordering::Release);
                }
                if let Some((n, got)) = open
                    && (n != self.last_ack.0 || got - self.last_ack.1 >= protocol::ACK_EVERY)
                    && self.open
                {
                    self.last_ack = (n, got);
                    self.send_ws(Out::Text(protocol::ack_message(n, got)));
                }
            }
            Ev::StreamEnd { id } if self.is_current(id) => {
                self.stream = None;
                let now = self.now_ms();
                if let Some(p) = &mut self.player {
                    p.on_stream_end(now);
                }
                self.run_actions();
            }
            Ev::StreamFailed { id } if self.is_current(id) => {
                self.stream = None;
                let now = self.now_ms();
                if let Some(p) = &mut self.player {
                    p.on_stream_error(now);
                }
                self.run_actions();
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
        if !levelling && !transitions.enabled() {
            return;
        }
        let smart = transitions.mode == TransitionMode::Smart;
        let (Some(mac), Ok(seq)) = (self.mac, u32::try_from(stream_number.saturating_sub(1)))
        else {
            return;
        };
        let fresh = self
            .player
            .as_ref()
            .is_none_or(|p| p.playing_track().is_none());
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
            let track = core
                .rpc(&player, protocol::stream_status_command())
                .await
                .ok()
                .and_then(|st| protocol::stream_track(&st, fresh));
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
                Action::OpenStream { stream_number, .. } => {
                    self.open_stream(stream_number);
                    self.look_up_loudness(stream_number);
                }
                Action::CloseStream => self.close_stream(),
            }
        }
    }

    fn open_stream(&mut self, n: u64) {
        self.close_stream();
        let Some((session, _)) = self.session.clone() else {
            return;
        };
        self.stream_ids += 1;
        let id = self.stream_ids;
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
        }));
        self.last_ack = (n, 0);
        self.stream = Some(StreamTask {
            id,
            want,
            task: task.abort_handle(),
        });
    }

    fn close_stream(&mut self) {
        if let Some(s) = self.stream.take() {
            s.task.abort();
        }
    }

    // --- Audio --------------------------------------------------------------------------

    /// Drop audio queued between the core and the speaker (a flush, or a new session).
    fn drop_queued_audio(&mut self) {
        self.audio.drop_queued_audio();
        self.timed_start = false;
    }

    fn on_tick(&mut self) {
        if let Some((at, resume)) = self.reconnect_at
            && Instant::now() >= at
        {
            self.reconnect_at = None;
            self.connect(resume);
        }
        let now_ns = mono_ns();
        let now = self.ms_at(now_ns);
        let ring = self.shared.out.ring.clone();

        // Frames that have become audible since the last tick, exact as of now.
        let played = self.audio.played_at(now_ns);
        let Some(p) = &mut self.player else {
            self.publish_status(false);
            return;
        };
        p.presented(played, now);
        p.tick(now);
        if p.flush_count() != self.last_flush {
            self.last_flush = p.flush_count();
            self.drop_queued_audio();
        }
        self.run_actions();
        self.apply_corrections();

        let src = self.player.as_mut().map(|p| p as &mut dyn AudioSource);
        self.audio.poll_output(src);
        let gate = self.gate.is_some_and(|(_, until)| Instant::now() < until);
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
        if let Some(s) = &self.stream {
            s.want.store(p.wants_stream_data(), Ordering::Release);
        }
        self.run_actions();
        self.publish_status(false);
        self.publish_signal(false);
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
        let (playback, elapsed_ms, core_ms, error) = match &self.player {
            Some(p) => {
                let s = p.status();
                let playback = match s.output {
                    OutputState::Stopped => LocalPlayback::Stopped,
                    OutputState::Waiting | OutputState::Loaded => LocalPlayback::Buffering,
                    OutputState::Playing => LocalPlayback::Playing,
                    OutputState::Paused { .. } => LocalPlayback::Paused,
                };
                (playback, s.elapsed_ms, s.buffered_ms, s.last_error)
            }
            None => (LocalPlayback::Stopped, 0, 0, None),
        };
        let previous_error = self
            .shared
            .status
            .lock()
            .ok()
            .and_then(|s| s.last_error.clone());
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
        }
    }

    fn publish_status(&mut self, force: bool) {
        let st = self.current_status();
        let key = (st.connection.clone(), st.playback, st.output_rate, st.held);
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
            self.shared.observer.on_status(st);
        }
    }

    fn publish_signal(&mut self, force: bool) {
        let force = force || self.audio.take_signal_dirty();
        let path = self.player.as_ref().and_then(|p| self.audio.signal_path(p));
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

/// A content type from the gateway, made safe to put in a header line for LMS.
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
async fn socket_task(
    conn: u64,
    core: Arc<Core>,
    server: Arc<Server>,
    resume: Option<(String, String, u64)>,
    tx: mpsc::UnboundedSender<Ev>,
    mut out_rx: mpsc::UnboundedReceiver<Out>,
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
    let first = match &resume {
        Some((session, secret, received)) => {
            protocol::resume_message(&token, session, secret, *received)
        }
        None => token.clone(),
    };
    if ws.send(Message::Text(first.into())).await.is_err() {
        let _ = tx.send(closed(None, ""));
        return;
    }
    let mut greeted = false;
    // Regression: the time limit was set afresh on every pass of the loop, so the player's
    // own STATs (one a second while playing) kept a dead socket looking alive until the
    // gateway's grace had run out, and the player had to start a new session.
    let idle = tokio::time::sleep(SOCKET_IDLE);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            _ = &mut idle => {
                let _ = tx.send(closed(None, ""));
                return;
            }
            msg = ws.next() => {
                idle.as_mut().reset(tokio::time::Instant::now() + SOCKET_IDLE);
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
                    Message::Close(frame) => {
                        let (code, reason) = frame
                            .map(|f| (Some(u16::from(f.code)), f.reason.to_string()))
                            .unwrap_or((None, String::new()));
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
                    // message before the greeting, text after it) is ignored.
                    _ => {}
                }
            }
            out = out_rx.recv() => {
                let Some(out) = out else { return };
                let msg = match out {
                    Out::Binary(b) => Message::Binary(b.into()),
                    Out::Text(t) => Message::Text(t.into()),
                };
                if !matches!(tokio::time::timeout(SOCKET_SEND_TIMEOUT, ws.send(msg)).await, Ok(Ok(()))) {
                    let _ = tx.send(closed(None, ""));
                    return;
                }
            }
        }
    }
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
}

/// Fetch stream `n` of the session, resuming at the byte offset already delivered after a
/// network failure or a 5xx, with backoff; anything else fails the track.
async fn stream_task(job: StreamJob) {
    let StreamJob {
        id,
        n,
        session,
        core,
        server,
        http,
        tx,
        want,
    } = job;
    let mut got = 0u64;
    let mut headers_seen = false;
    let mut tries = 0u32;
    let mut renewed = false;
    loop {
        let token = match core.access_token().await {
            Ok(t) => t,
            Err(ApiError::Unauthorised) => {
                let _ = tx.send(Ev::StreamFailed { id });
                return;
            }
            Err(_) => String::new(), // fails as a network error below and is retried
        };
        let url = server.stream_url(&session, n, got);
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
                loop {
                    // Back-pressure: stop reading while the core's input is full.
                    while !want.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    match tokio::time::timeout(STREAM_READ_TIMEOUT, body.frame()).await {
                        Ok(Some(Ok(frame))) => {
                            if let Ok(data) = frame.into_data()
                                && !data.is_empty()
                            {
                                got += data.len() as u64;
                                tries = 0;
                                if tx
                                    .send(Ev::StreamData {
                                        id,
                                        bytes: data.to_vec(),
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        Ok(None) => {
                            let _ = tx.send(Ev::StreamEnd { id });
                            return;
                        }
                        // Broken mid-download, or stalled: resume from `got`.
                        _ => break,
                    }
                }
                0
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
        if !protocol::stream_should_retry(status) || tries >= protocol::STREAM_RETRIES {
            let _ = tx.send(Ev::StreamFailed { id });
            return;
        }
        tokio::time::sleep(protocol::backoff(tries, protocol::random01())).await;
        tries += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
