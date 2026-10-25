//! A player session: the gateway's Slimproto connection to LMS for one device.
//!
//! The session outlives individual client sockets. While a socket is attached, LMS frames
//! go straight to it; while detached (the client's network dropped), frames are queued for
//! up to [`Config::resume_grace`](crate::Config::resume_grace) and delivered on resume.
//! LMS never sees the player leave.
//!
//! Client → LMS traffic is filtered: only the frames a software player needs reach LMS
//! (see [`Session::client_bytes`]).

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use anywhere_core::proto::{
    ClientFrameReader, ClientMsg, DEVICE_ID_SOFTWARE, Helo, ServerFrameReader, Stat, Strm,
    encode_server_frame, validate_stream_request,
};

use crate::accounts::Principal;
use crate::random_id;

/// Whether LMS has been told to proxy internet streams for the player (see
/// [`Session::ensure_proxied_streaming`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrefState {
    NotYet,
    Pending,
    Done,
}

/// Refused direct streams replayed through LMS, at most, in a row.
const MAX_REPLAYS: u8 = 3;
use crate::relay::Relay;
use crate::{AppState, Gateway};

/// What the attached socket's writer task should do next.
#[derive(Debug)]
pub enum Outbound {
    Frame(Vec<u8>),
    /// A text message that is not a frame (never logged for redelivery, never counted in
    /// `received`): the stream message ([`crate::stream_info`]).
    Text(String),
    Close(u16, &'static str),
}

/// A request for an attached socket to close at once with this code and reason, even if
/// its outgoing queue is full or its writer is blocked on a slow client.
pub type Kick = Option<(u16, &'static str)>;

/// Per-socket queue towards the client. Bounded: a client that can't keep up is detached
/// (it can resume) rather than letting memory grow.
pub const OUTBOUND_QUEUE: usize = 1024;

/// Longest capabilities string passed on to LMS, and the most tokens in it.
const MAX_CAPABILITIES: usize = 1024;
const MAX_CAPABILITY_TOKENS: usize = 64;
/// Longest player name passed on to LMS (characters).
const MAX_NAME: usize = 64;
/// Outstanding LMS name queries remembered (each allows one reply).
const MAX_NAME_QUERIES: u8 = 4;
/// Where a STAT payload holds the stream bytes received (big-endian `u64`).
const STAT_BYTES_RECEIVED: std::ops::Range<usize> = 15..23;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachError {
    /// The session has ended.
    Gone,
    /// The client claims frames that were never sent.
    Invalid,
    /// Frames the client missed are no longer held; it must start a new session.
    Gap,
}

/// Why a session ended (sent to an attached client as a close code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    LmsClosed,
    Replaced,
    GraceExpired,
    QueueOverflow,
    Policy,
    Revoked,
}

struct Attachment {
    generation: u64,
    tx: mpsc::Sender<Outbound>,
    kick: watch::Sender<Kick>,
}

impl Attachment {
    fn kick(&self, code: u16, reason: &'static str) {
        self.kick.send_replace(Some((code, reason)));
    }
}

/// A newly attached socket's side of the session.
pub struct Attached {
    pub generation: u64,
    pub rx: mpsc::Receiver<Outbound>,
    pub kick: watch::Receiver<Kick>,
    /// The last stream message sent in this session, to send again right after the
    /// greeting (a resumed client is never without it). Later ones follow on `rx`.
    pub stream_message: Option<String>,
}

struct State {
    attached: Option<Attachment>,
    next_generation: u64,
    /// Frames for the client, numbered from 1, kept for redelivery on resume. Frames
    /// handed to a socket may still be lost in flight, so the client says on resume how
    /// many it received and everything after that is sent again.
    log: VecDeque<(u64, Vec<u8>)>,
    log_bytes: usize,
    /// Sequence number of the last frame appended to the log.
    last_seq: u64,
    /// Highest sequence number handed to an attached socket.
    sent_upto: u64,
    seen_helo: bool,
    ended: bool,
    stream_seq: u64,
    /// The stream whose ignored acknowledgement was last logged (one line per stream).
    ack_ignored_logged: Option<u64>,
    /// `None` when the current `strm` was refused by policy.
    stream_path: Option<String>,
    /// The current `strm s` format code (`f` for FLAC).
    stream_format: u8,
    /// What the current `strm s` says LMS serves (the stream message's `served`, less a
    /// FLAC stream's format, read from its first bytes).
    stream_served: Option<crate::stream_info::Served>,
    /// The last stream message sent (or kept for a detached client), sent again after a
    /// resume greeting.
    stream_message: Option<String>,
    /// The level the client asked for (docs/plans/streaming-quality.md).
    quality: crate::live::Quality,
    /// The current stream is to be re-encoded: the level, its bit rate and the slot it
    /// holds, until its relay is made.
    live: Option<(crate::live::Quality, u32, tokio::sync::OwnedSemaphorePermit)>,
    relay: Option<Arc<Relay>>,
    /// Client → LMS framing; persists across sockets so a frame split by a drop is safe.
    from_client: ClientFrameReader,
    /// The client's most recent status report, for heartbeats while detached.
    last_stat: Option<(Stat, Instant)>,
    /// The jiffies of the last heartbeat the gateway sent LMS for the client, and when:
    /// a client `STMt` older than that, within [`STALE_GUARD`], is not forwarded.
    told: Option<(u32, Instant)>,
    /// LMS sync corrections dropped while the client was away (`pass_from_lms`), for logs.
    corrections_dropped: u64,
    /// Whether the client last reported itself playing (elapsed time advances).
    playing: bool,
    /// LMS name queries (`setd` id 0, no data) not yet answered by the client.
    name_queries: u8,
    /// Whether LMS has been told to proxy internet streams for this player (its
    /// `mp3StreamingMethod` player pref), see [`Session::ensure_proxied_streaming`].
    proxied_pref: PrefState,
    /// Whether the device's player has been given its initial LMS volume this session (see
    /// [`Session::ensure_initial_level`]).
    initial_level: PrefState,
    /// The current `strm` pointed the player at another server and was refused; the reader
    /// takes this to have LMS replay the entry through itself.
    refused_direct: bool,
    /// Replays asked for after refused direct streams, capped so a server that will not
    /// proxy cannot be asked for ever.
    replays: u8,
    /// When the attached socket last sent anything.
    last_heard: Instant,
    /// The gateway is reporting to LMS for an attached but silent client.
    speaking_for_silent: bool,
    /// What the client's player is heard playing, for the user's listening history.
    tracker: crate::history::Tracker,
    /// How long the session is held for a detached client (granted at sign-in).
    grace: Duration,
    /// Tracker steps to carry out once the state lock is released.
    steps: Vec<crate::history::Step>,
    /// A track-start lookup task is waiting: further starts don't spawn another (it looks
    /// up whichever start is the latest when it wakes).
    lookup_pending: bool,
    /// When LMS was last asked which track started, within the past minute.
    lookups: VecDeque<Instant>,
}

/// Most track-start lookups (LMS `status` queries) one session may make a minute. A
/// player starts a new track every few minutes; a client looping `STMs` frames must not
/// turn the gateway into an amplifier against LMS.
const LOOKUPS_PER_MINUTE: usize = 30;

/// How often the gateway reports status to LMS on behalf of a detached client. LMS drops
/// players it hasn't heard from for roughly 15 s.
const DETACHED_HEARTBEAT: Duration = Duration::from_secs(1);

/// How long after the gateway last spoke for the client a client heartbeat older than
/// what it told LMS is dropped. A client sends what it held through a gap the moment it
/// resumes; this only has to cover that, and must not silence for long a client whose
/// clock went back.
const STALE_GUARD: Duration = Duration::from_secs(5);

pub struct Session {
    pub id: String,
    pub device: Principal,
    /// The player name: the device's name, updated if it is renamed during the session.
    name: Mutex<String>,
    secret: String,
    lms_tx: tokio::sync::Mutex<OwnedWriteHalf>,
    state: Mutex<State>,
    /// The gateway, for listening history (weak: the gateway owns its sessions).
    gw: std::sync::Weak<Gateway>,
}

#[derive(Debug)]
pub enum ClientInput {
    /// Frames accepted and forwarded.
    Ok,
    /// The client broke protocol; the caller must end the session.
    Policy(String),
    /// Writing to LMS failed; the caller must end the session.
    LmsGone,
    /// The socket is no longer the session's current one (replaced or detached); nothing
    /// was forwarded.
    Stale,
}

/// What happened when an LMS frame was delivered.
enum Delivery {
    Ok,
    /// Too much undelivered while detached: the session must end.
    Overflow,
    /// The attached socket couldn't keep up, or had already gone, and was detached: the
    /// caller holds the session for its resume.
    Detached(u64),
}

impl Session {
    fn new(device: Principal, lms_tx: OwnedWriteHalf, gw: std::sync::Weak<Gateway>) -> Session {
        Session {
            gw,
            id: random_id(),
            name: Mutex::new(device.device_name.clone()),
            device,
            secret: random_id(),
            lms_tx: tokio::sync::Mutex::new(lms_tx),
            state: Mutex::new(State {
                attached: None,
                next_generation: 0,
                log: VecDeque::new(),
                log_bytes: 0,
                last_seq: 0,
                sent_upto: 0,
                seen_helo: false,
                proxied_pref: PrefState::NotYet,
                initial_level: PrefState::NotYet,
                refused_direct: false,
                replays: 0,
                ended: false,
                stream_seq: 0,
                ack_ignored_logged: None,
                stream_path: None,
                stream_format: 0,
                stream_served: None,
                stream_message: None,
                quality: Default::default(),
                live: None,
                relay: None,
                from_client: ClientFrameReader::default(),
                last_stat: None,
                told: None,
                corrections_dropped: 0,
                playing: false,
                name_queries: 0,
                last_heard: Instant::now(),
                speaking_for_silent: false,
                tracker: Default::default(),
                grace: crate::Config::default().resume_grace,
                steps: Vec::new(),
                lookup_pending: false,
                lookups: VecDeque::new(),
            }),
        }
    }

    /// Connect to LMS and start the session's LMS reader.
    pub async fn open(gw: &AppState, device: Principal) -> std::io::Result<Arc<Session>> {
        let lms = tokio::time::timeout(
            gw.cfg.dial_timeout,
            TcpStream::connect(&gw.cfg.lms_slimproto),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "LMS connect timeout"))??;
        let _ = lms.set_nodelay(true);
        let (rd, wr) = lms.into_split();
        let session = Arc::new(Session::new(device, wr, Arc::downgrade(gw)));
        session.set_grace(gw.cfg.resume_grace);

        // One session per device: LMS would otherwise see two players with the same MAC.
        let previous: Vec<Arc<Session>> = {
            let mut sessions = gw.sessions.lock().unwrap();
            let prev = sessions
                .values()
                .filter(|s| s.device.device_id == session.device.device_id)
                .cloned()
                .collect();
            sessions.insert(session.id.clone(), session.clone());
            prev
        };
        for p in previous {
            p.end(gw, EndReason::Replaced).await;
        }

        tokio::spawn(read_lms(gw.clone(), session.clone(), rd));
        tokio::spawn(speak_for_silent_client(gw.clone(), session.clone()));
        Ok(session)
    }

    pub fn secret_matches(&self, candidate: &str) -> bool {
        crate::secret_eq(&self.secret, candidate)
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// The player name (the device's current name).
    pub fn name(&self) -> String {
        self.name.lock().unwrap().clone()
    }

    /// The device was renamed: LMS's name queries get the new name from now on.
    pub fn set_name(&self, name: &str) {
        *self.name.lock().unwrap() = name.to_string();
    }

    /// How long the session is held for a detached client.
    pub fn grace(&self) -> Duration {
        self.state.lock().unwrap().grace
    }

    /// Set the resume grace (granted at sign-in; a resume may change it). It applies from
    /// the next time the client's socket drops.
    pub fn set_grace(&self, grace: Duration) {
        self.state.lock().unwrap().grace = grace;
    }

    pub fn is_ended(&self) -> bool {
        self.state.lock().unwrap().ended
    }

    /// Whether `generation` is the socket currently attached.
    fn is_current(st: &State, generation: u64) -> bool {
        !st.ended
            && st
                .attached
                .as_ref()
                .is_some_and(|a| a.generation == generation)
    }

    /// Attach a client socket that has received `received` frames of this session so far
    /// (0 for a new session), replacing any existing socket. Frames after `received` are
    /// queued for it straight away.
    pub fn attach(&self, received: u64) -> Result<Attached, AttachError> {
        let mut st = self.state.lock().unwrap();
        if st.ended {
            return Err(AttachError::Gone);
        }
        if received > st.last_seq {
            return Err(AttachError::Invalid);
        }
        let oldest = st.log.front().map_or(st.last_seq + 1, |(seq, _)| *seq);
        if received + 1 < oldest {
            // Frames the client never got have already been discarded.
            return Err(AttachError::Gap);
        }
        let resend: Vec<Vec<u8>> = st
            .log
            .iter()
            .filter(|(seq, _)| *seq > received)
            .map(|(_, f)| f.clone())
            .collect();
        let (tx, rx) = mpsc::channel(OUTBOUND_QUEUE + resend.len() + 8);
        if let Some(old) = st.attached.take() {
            // The old socket stops at once: its reader must not feed this session any more.
            old.kick(4000, "replaced by a newer connection");
        }
        for f in resend {
            let _ = tx.try_send(Outbound::Frame(f)); // capacity reserved above
        }
        st.sent_upto = st.last_seq;
        st.last_heard = Instant::now();
        st.speaking_for_silent = false;
        st.next_generation += 1;
        let generation = st.next_generation;
        let (kick, kick_rx) = watch::channel(None);
        st.attached = Some(Attachment {
            generation,
            tx,
            kick,
        });
        Ok(Attached {
            generation,
            rx,
            kick: kick_rx,
            stream_message: st.stream_message.clone(),
        })
    }

    /// The socket `generation` sent something (any message, even a ping): it is alive.
    pub fn heard_from(&self, generation: u64) {
        let mut st = self.state.lock().unwrap();
        if Self::is_current(&st, generation) {
            st.last_heard = Instant::now();
            if st.speaking_for_silent {
                st.speaking_for_silent = false;
                info!(mac = %self.device.mac_string(), "player socket speaking again");
            }
        }
    }

    /// Whether the gateway should report to LMS for the attached client now: it has been
    /// silent for `silence`. (A detached client is reported for by [`Self::hold_for_resume`].)
    fn attached_but_silent(&self, silence: Duration) -> bool {
        let mut st = self.state.lock().unwrap();
        let silent = !st.ended && st.attached.is_some() && st.last_heard.elapsed() >= silence;
        if silent && !st.speaking_for_silent {
            st.speaking_for_silent = true;
            info!(mac = %self.device.mac_string(), "player socket silent; reporting to LMS on its behalf");
        }
        silent
    }

    /// The client socket for `generation` went away. Starts the grace timer if it was current.
    pub fn detach(self: &Arc<Self>, gw: &AppState, generation: u64) {
        {
            let mut st = self.state.lock().unwrap();
            match &st.attached {
                Some(a) if a.generation == generation => st.attached = None,
                _ => return, // already replaced or ended
            }
        }
        info!(mac = %self.device.mac_string(), "player socket dropped; holding session for resume");
        self.hold_for_resume(gw, generation);
    }

    /// The socket for `generation` has been detached: keep LMS happy and end the session
    /// if nobody resumes it within the grace period.
    fn hold_for_resume(self: &Arc<Self>, gw: &AppState, generation: u64) {
        // Keep LMS from timing the player out while the client is away.
        let me = self.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(DETACHED_HEARTBEAT);
            every.tick().await;
            loop {
                every.tick().await;
                if !me.detached_since(generation) {
                    break;
                }
                me.heartbeat(0).await;
            }
        });
        let (gw, me) = (gw.clone(), self.clone());
        let grace = self.grace();
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let expired = {
                let st = me.state.lock().unwrap();
                !st.ended && st.attached.is_none() && st.next_generation == generation
            };
            if expired {
                info!(mac = %me.device.mac_string(), "resume grace expired");
                me.end(&gw, EndReason::GraceExpired).await;
            }
        });
    }

    /// Bytes from the client socket `generation`: frame them, enforce policy, forward to
    /// LMS.
    ///
    /// Only what a software player sends is allowed: HELO (once, first, with the software
    /// player device ID and the device's MAC; capabilities are sanitised), STAT, RESP,
    /// META, DSCO, `BYE!`, and SETD only as the answer to an LMS name query (the name is
    /// sanitised). Anything else (IR, BUTN, KNOB, BODY, UREQ, SHUT, unsolicited SETD, …)
    /// is a policy violation.
    pub async fn client_bytes(&self, generation: u64, data: &[u8]) -> ClientInput {
        let frames = {
            let mut st = self.state.lock().unwrap();
            if !Self::is_current(&st, generation) {
                return ClientInput::Stale;
            }
            st.from_client.push(data);
            let mut frames = Vec::new();
            let mut stale = 0usize;
            loop {
                let (op, payload) = match st.from_client.next_frame() {
                    Ok(Some(f)) => f,
                    Ok(None) => break,
                    Err(e) => return ClientInput::Policy(e.to_string()),
                };
                let payload = match self.check_client_frame(&mut st, &op, payload) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        stale += 1;
                        continue;
                    }
                    Err(why) => return ClientInput::Policy(why),
                };
                let mut frame = Vec::with_capacity(8 + payload.len());
                frame.extend_from_slice(&op);
                frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                frame.extend_from_slice(&payload);
                frames.push(frame);
            }
            if st.from_client.buffered() > ClientFrameReader::MAX_FRAME + 8 {
                return ClientInput::Policy("oversize frame".into());
            }
            if stale > 0 {
                debug!(
                    mac = %self.device.mac_string(),
                    dropped = stale,
                    "stale client heartbeats not forwarded (older than the gateway's own)"
                );
            }
            (frames, std::mem::take(&mut st.steps))
        };
        let (frames, steps) = frames;
        let mut lms = self.lms_tx.lock().await;
        for f in frames {
            if lms.write_all(&f).await.is_err() {
                return ClientInput::LmsGone;
            }
        }
        drop(lms);
        self.run_steps(steps);
        ClientInput::Ok
    }

    /// Carry out the listening tracker's steps: ask LMS which track started, record plays
    /// that count in the user's history, store the time heard when a play ends.
    fn run_steps(&self, steps: Vec<crate::history::Step>) {
        use crate::history::{Source, Step};
        if steps.is_empty() {
            return;
        }
        let Some(gw) = self.gw.upgrade() else { return };
        for step in steps {
            match step {
                Step::Lookup(_) => {
                    // One waiting lookup per session: starts in quick succession are
                    // coalesced, and only the latest is looked up.
                    if std::mem::replace(&mut self.state.lock().unwrap().lookup_pending, true) {
                        continue;
                    }
                    let Some(me) = gw.session(&self.id) else {
                        self.state.lock().unwrap().lookup_pending = false;
                        continue;
                    };
                    let gw = gw.clone();
                    tokio::spawn(async move {
                        let delay = gw.cfg.history_lookup_delay;
                        let mut seen = me.state.lock().unwrap().tracker.serial();
                        let generation = loop {
                            tokio::time::sleep(delay).await;
                            let mut st = me.state.lock().unwrap();
                            if st.ended {
                                st.lookup_pending = false;
                                return;
                            }
                            let now = st.tracker.serial();
                            if now != seen {
                                // Another start meanwhile: give LMS the full delay for it.
                                seen = now;
                                continue;
                            }
                            st.lookup_pending = false;
                            let t = Instant::now();
                            while st.lookups.front().is_some_and(|x| {
                                t.saturating_duration_since(*x) >= Duration::from_secs(60)
                            }) {
                                st.lookups.pop_front();
                            }
                            if st.lookups.len() >= LOOKUPS_PER_MINUTE {
                                return; // over the cap: this listen stays unidentified
                            }
                            st.lookups.push_back(t);
                            break now;
                        };
                        let track = gw.playing_track(&me.device.mac_string()).await;
                        let more = me.state.lock().unwrap().tracker.resolved(
                            generation,
                            track,
                            Instant::now(),
                        );
                        me.run_steps(more);
                    });
                }
                Step::Record {
                    serial,
                    track,
                    played_at,
                    seconds,
                } => {
                    // Off the session's task (M7): a stalled disk must not stop it relaying.
                    let Some(me) = gw.session(&self.id) else {
                        continue;
                    };
                    let gw = gw.clone();
                    let user = self.device.user_id;
                    tokio::spawn(async move {
                        let g = gw.clone();
                        let row = tokio::task::spawn_blocking(move || {
                            g.note_play(user, &track, played_at, seconds, Source::Live)
                        })
                        .await
                        .ok()
                        .flatten();
                        let more = me.state.lock().unwrap().tracker.recorded(serial, row);
                        me.run_steps(more.into_iter().collect());
                    });
                }
                Step::Update { row, seconds } => {
                    let user = self.device.user_id;
                    let gw = gw.clone();
                    tokio::spawn(async move {
                        gw.db(move |a| crate::history::update_seconds(a, user, row, seconds))
                            .await
                    });
                }
            }
        }
    }

    /// Policy for one client frame. Returns the payload to forward (possibly rewritten),
    /// or `None` for a stale heartbeat that is quietly dropped (see `State::told`).
    fn check_client_frame(
        &self,
        st: &mut State,
        op: &[u8; 4],
        payload: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, String> {
        if op == b"HELO" {
            if st.seen_helo {
                return Err("repeated HELO".into());
            }
            let h = Helo::parse(&payload).map_err(|_| "malformed HELO".to_string())?;
            if h.mac != self.device.mac {
                return Err("HELO MAC does not match device".into());
            }
            if h.device_id != DEVICE_ID_SOFTWARE {
                return Err(format!("HELO device ID {} not allowed", h.device_id));
            }
            st.seen_helo = true;
            let mut out = payload[..36].to_vec();
            out.extend_from_slice(sanitise_capabilities(&h.capabilities).as_bytes());
            return Ok(Some(out));
        }
        if !st.seen_helo {
            return Err("first frame must be HELO".into());
        }
        match op {
            b"STAT" => {
                let mut stat = Stat::parse(&payload).map_err(|_| "malformed STAT".to_string())?;
                // The gateway spoke for the client lately (it was away, or silent): LMS
                // already has a later clock for it than a heartbeat the client held
                // meanwhile. Sent now, all at once, each would make LMS re-base its clock
                // for the player and throw a sync group out (live, 2026-10-15). Events are
                // forwarded whatever their age: LMS must hear them.
                if let Some((told, at)) = st.told {
                    // Wrapping: jiffies roll over after 49 days.
                    let behind = (told.wrapping_sub(stat.jiffies) as i32) > 0;
                    if at.elapsed() >= STALE_GUARD || !behind {
                        st.told = None;
                    } else if &stat.event == b"STMt" {
                        return Ok(None);
                    }
                }
                let mut payload = payload;
                // LMS counts the bytes it sent; the client counts what it received, which
                // is less when the relay dropped FLAC artwork. Report LMS's count.
                if let Some(relay) = &st.relay {
                    let sent = relay.upstream_bytes(stat.bytes_received);
                    if sent != stat.bytes_received {
                        stat.bytes_received = sent;
                        payload[STAT_BYTES_RECEIVED].copy_from_slice(&sent.to_be_bytes());
                    }
                }
                // Not `STMu`: a client with a rolling cache can play on from it after a
                // track's end, out of LMS's reach; only a pause or a stop freezes the
                // heartbeats' elapsed time (see `synthesised_stat`).
                match &stat.event {
                    b"STMs" | b"STMr" => st.playing = true,
                    b"STMp" | b"STMf" | b"STMn" => st.playing = false,
                    _ => {}
                }
                let steps = st.tracker.on_stat(
                    stat.event,
                    stat.elapsed_milliseconds,
                    stat.jiffies,
                    Instant::now(),
                    crate::history::unix_now(),
                );
                st.steps.extend(steps);
                st.last_stat = Some((stat, Instant::now()));
                Ok(Some(payload))
            }
            b"RESP" | b"META" => Ok(Some(payload)),
            b"DSCO" if payload.len() == 1 => Ok(Some(payload)),
            b"BYE!" if payload.len() <= 1 => Ok(Some(payload)),
            b"SETD" if payload.first() == Some(&0) && st.name_queries > 0 => {
                st.name_queries -= 1;
                Ok(Some(name_reply(&payload[1..], &self.name())))
            }
            b"SETD" => Err("unsolicited SETD".into()),
            _ => Err(format!(
                "client opcode \"{}\" not allowed",
                op.escape_ascii()
            )),
        }
    }

    /// The current stream number and validated path (if allowed).
    pub fn stream(&self) -> (u64, Option<String>) {
        let st = self.state.lock().unwrap();
        (st.stream_seq, st.stream_path.clone())
    }

    /// The `strm s` format code of stream `n`, if it is still the current stream.
    pub fn stream_format(&self, n: u64) -> Option<u8> {
        let st = self.state.lock().unwrap();
        (st.stream_seq == n).then_some(st.stream_format)
    }

    /// Socket `generation` acknowledges `bytes` of stream `n`. Acks for other streams, and
    /// from sockets that are no longer current, are ignored. Never waits on network I/O.
    pub fn ack(&self, generation: u64, n: u64, bytes: u64) {
        let relay = {
            let mut st = self.state.lock().unwrap();
            if st.stream_seq != n || !Self::is_current(&st, generation) {
                // Ignored acks leave the relay waiting for ever after one window: say so,
                // once per stream (issue #12).
                if st.ack_ignored_logged != Some(n) {
                    st.ack_ignored_logged = Some(n);
                    let current_socket = Self::is_current(&st, generation);
                    info!(
                        stream = n,
                        current = st.stream_seq,
                        current_socket,
                        bytes,
                        "stream acknowledgement ignored"
                    );
                }
                return;
            }
            st.relay.clone()
        };
        if let Some(relay) = relay {
            relay.ack(bytes);
        }
    }

    /// The level of the streams that follow (from the next `strm`).
    pub fn set_quality(&self, quality: crate::live::Quality) {
        self.state.lock().unwrap().quality = quality;
    }

    /// The relay for stream `n`, created on first use. A stream chosen for re-encoding at
    /// its `strm` goes through `live_ffmpeg` ([`crate::live`]); otherwise, with
    /// `strip_flac`, a stream LMS announced as FLAC has its artwork and padding dropped
    /// ([`crate::flac_strip`]).
    pub fn relay(
        &self,
        n: u64,
        window: usize,
        strip_flac: bool,
        live_ffmpeg: Option<&std::path::Path>,
    ) -> Option<Arc<Relay>> {
        let mut st = self.state.lock().unwrap();
        if st.ended || n != st.stream_seq || st.stream_path.is_none() {
            return None;
        }
        if st.relay.is_none() {
            let relay = Relay::new(window);
            let relay = match (st.live.take(), live_ffmpeg) {
                (Some((level, bitrate, permit)), Some(ffmpeg)) => {
                    relay.with_live(ffmpeg, level, bitrate, permit)
                }
                // (The player was told Opus but gets FLAC: its core probes the content.)
                _ if strip_flac && st.stream_format == b'f' => relay.with_flac_strip(),
                _ => relay,
            };
            st.relay = Some(Arc::new(relay));
        }
        st.relay.clone()
    }

    fn detached_since(&self, generation: u64) -> bool {
        let st = self.state.lock().unwrap();
        !st.ended && st.attached.is_none() && st.next_generation == generation
    }

    fn is_detached(&self) -> bool {
        let st = self.state.lock().unwrap();
        !st.ended && st.attached.is_none()
    }

    /// A status report on behalf of the detached client: its last real STAT, with the
    /// clock (and, if it was playing, the elapsed time) advanced by wall time, so the play
    /// point LMS works out from the two stays put. The elapsed time runs on past the end
    /// of the track: the client plays on from its cache into the next one, which only it
    /// can report. Stopping it there made the player look stalled to LMS (its clock ran on,
    /// its position didn't), and in a sync group LMS then sent it, and the others, a
    /// correction every 2 s (live, 2026-10-15). How far it runs is bounded by the grace.
    fn synthesised_stat(&self, server_timestamp: u32) -> Option<Vec<u8>> {
        let mut st = self.state.lock().unwrap();
        if !st.seen_helo {
            return None;
        }
        let (mut stat, at) = st.last_stat.clone()?;
        let dt = at.elapsed().as_millis() as u64;
        stat.event = *b"STMt";
        stat.jiffies = stat.jiffies.wrapping_add(dt as u32);
        if st.playing {
            let ms = (stat.elapsed_milliseconds as u64 + dt).min(u32::MAX as u64);
            stat.elapsed_milliseconds = ms as u32;
            stat.elapsed_seconds = (ms / 1000) as u32;
        }
        stat.server_timestamp = server_timestamp;
        st.told = Some((stat.jiffies, Instant::now()));
        Some(ClientMsg::Stat(stat).encode())
    }

    async fn heartbeat(&self, server_timestamp: u32) {
        if let Some(frame) = self.synthesised_stat(server_timestamp) {
            let _ = self.lms_tx.lock().await.write_all(&frame).await;
        }
    }

    /// End the session: close LMS's connection, stop its stream, tell any attached client
    /// why.
    pub async fn end(&self, gw: &Gateway, reason: EndReason) {
        let (attached, relay, steps) = {
            let mut st = self.state.lock().unwrap();
            if st.ended {
                return;
            }
            st.ended = true;
            st.log.clear();
            st.log_bytes = 0;
            let mut steps = std::mem::take(&mut st.steps);
            steps.extend(st.tracker.finish());
            (st.attached.take(), st.relay.take(), steps)
        };
        self.run_steps(steps);
        if let Some(relay) = relay {
            relay.cancel();
        }
        gw.remove_session(&self.id);
        if let Some(a) = attached {
            let (code, why) = match reason {
                EndReason::LmsClosed => (1001, "server closed connection"),
                EndReason::Replaced => (4000, "replaced by a newer connection"),
                EndReason::GraceExpired => (1001, "session expired"),
                EndReason::QueueOverflow => (1013, "client too slow"),
                EndReason::Policy => (1008, "policy violation"),
                EndReason::Revoked => (1008, "access revoked"),
            };
            // Queued after any pending frames if there's room; the kick closes the socket
            // regardless.
            let _ = a.tx.try_send(Outbound::Close(code, why));
            a.kick(code, why);
        }
        let _ = self.lms_tx.lock().await.shutdown().await;
        info!(mac = %self.device.mac_string(), ?reason, "session ended");
    }

    /// Log one LMS frame and send it to the attached socket, if any.
    /// A frame from LMS for the client (after the checks in `read_lms`): delivered, or kept
    /// for it while it is away. `None` when it was dropped.
    ///
    /// While the client is away, LMS's sync corrections for it are dropped: a skip ahead
    /// (`strm a`), a pause for an interval (`strm p` with one) and a start at a time
    /// (`strm u` with one). They describe the moment they were sent; replayed on resume,
    /// seconds or minutes late, they would make the player jump by their sum. A pause or
    /// unpause now (interval or time 0), and everything else, is kept as before.
    fn pass_from_lms(&self, op: [u8; 4], payload: &[u8], limit: usize) -> Option<Delivery> {
        if &op == b"strm"
            && let Ok(s) = Strm::parse(payload)
            && (s.command == b'a' || (matches!(s.command, b'p' | b'u') && s.replay_gain != 0))
        {
            let mut st = self.state.lock().unwrap();
            if !st.ended && st.attached.is_none() {
                st.corrections_dropped += 1;
                debug!(
                    mac = %self.device.mac_string(),
                    command = %char::from(s.command),
                    dropped = st.corrections_dropped,
                    "sync correction for a detached client dropped"
                );
                return None;
            }
        }
        Some(self.deliver(encode_server_frame(op, payload), limit))
    }

    fn deliver(&self, frame: Vec<u8>, limit: usize) -> Delivery {
        let mut st = self.state.lock().unwrap();
        if st.ended {
            return Delivery::Ok;
        }
        st.last_seq += 1;
        let seq = st.last_seq;
        st.log_bytes += frame.len();
        st.log.push_back((seq, frame.clone()));
        let mut detached = None;
        if let Some(a) = &st.attached {
            match a.tx.try_send(Outbound::Frame(frame)) {
                Ok(()) => st.sent_upto = seq,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Client can't keep up: close the socket; it may resume and catch up.
                    warn!(mac = %self.device.mac_string(), "client too slow; detaching socket");
                    a.kick(1013, "client too slow");
                    detached = Some(a.generation);
                    st.attached = None;
                }
                // The socket's writer has gone before its handler noticed: held for resume
                // like a dropped socket. Detached without that, nothing kept LMS hearing from
                // the player, so LMS dropped it after about 15 s and the client couldn't
                // resume.
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    info!(mac = %self.device.mac_string(), "player socket gone; holding session for resume");
                    detached = Some(a.generation);
                    st.attached = None;
                }
            }
        }
        // Keep the log bounded, discarding only frames already handed to a socket.
        while st.log_bytes > limit {
            match st.log.front() {
                Some((seq, f)) if *seq <= st.sent_upto => {
                    st.log_bytes -= f.len();
                    st.log.pop_front();
                }
                _ => break,
            }
        }
        if st.log_bytes > limit {
            Delivery::Overflow
        } else if let Some(g) = detached {
            Delivery::Detached(g)
        } else {
            Delivery::Ok
        }
    }

    /// LMS asked the player for its name (`setd` id 0 with no data): allow one reply.
    fn note_setd(&self, payload: &[u8]) {
        if payload == [0] {
            let mut st = self.state.lock().unwrap();
            st.name_queries = (st.name_queries + 1).min(MAX_NAME_QUERIES);
        }
    }

    /// Have LMS proxy internet streams for this player, once per session, as soon as LMS
    /// knows the player (its first frame after HELO). LMS otherwise tells a capable player
    /// to fetch a station from the broadcaster itself ("direct streaming"), and these
    /// players only ever talk to the gateway, so such a `strm` is refused and the station
    /// is silent. The player pref (`mp3StreamingMethod`, 1 = proxied, LMS's own setting
    /// under Player › Audio) covers every HTTP stream, not only MP3, and is kept by LMS.
    fn ensure_proxied_streaming(self: &Arc<Self>, gw: &crate::AppState) {
        {
            let mut st = self.state.lock().unwrap();
            if !st.seen_helo || st.proxied_pref != PrefState::NotYet {
                return;
            }
            st.proxied_pref = PrefState::Pending;
        }
        let (gw, session) = (gw.clone(), Arc::clone(self));
        tokio::spawn(async move {
            session.set_proxied_pref(&gw).await;
        });
    }

    /// The `playerpref` command itself; the outcome is remembered so a failure is retried
    /// at the next chance and a success is not repeated.
    async fn set_proxied_pref(&self, gw: &Gateway) -> bool {
        let mac = self.device.mac_string();
        let cmd: Vec<String> = ["playerpref", "mp3StreamingMethod", "1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let ok = match gw.lms_command(&mac, &cmd).await {
            Ok(_) => {
                info!(mac = %mac, "LMS proxies internet streams for this player");
                true
            }
            Err(e) => {
                warn!(mac = %mac, error = %e, "could not have LMS proxy internet streams for this player");
                false
            }
        };
        self.state.lock().unwrap().proxied_pref = if ok {
            PrefState::Done
        } else {
            PrefState::NotYet
        };
        ok
    }

    /// Give a brand-new device's player full volume, once, as soon as LMS knows the player
    /// (the same moment as [`Self::ensure_proxied_streaming`]). LMS starts every player it
    /// has never seen at 50; these players are meant to start at 100. Only a device that
    /// has never had it (see `Accounts::needs_initial_level`), and it is recorded only once
    /// LMS has accepted it, so a failure is tried again at the next chance (this session or
    /// a later one) and a success is never repeated, whatever the owner sets afterwards.
    fn ensure_initial_level(self: &Arc<Self>, gw: &crate::AppState) {
        {
            let mut st = self.state.lock().unwrap();
            if !st.seen_helo || st.initial_level != PrefState::NotYet {
                return;
            }
            st.initial_level = PrefState::Pending;
        }
        let (gw, session) = (gw.clone(), Arc::clone(self));
        tokio::spawn(async move {
            let ok = session.set_initial_level(&gw).await;
            session.state.lock().unwrap().initial_level = if ok {
                PrefState::Done
            } else {
                PrefState::NotYet
            };
        });
    }

    /// The `mixer volume 100` itself, if the device still needs it. True once done.
    async fn set_initial_level(&self, gw: &Gateway) -> bool {
        let device = self.device.device_id;
        // Off the async workers: a stalled disk must not hold up other sessions.
        if !gw.db(move |a| a.needs_initial_level(device)).await {
            return true;
        }
        let mac = self.device.mac_string();
        let cmd: Vec<String> = ["mixer", "volume", "100"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        match gw.lms_command(&mac, &cmd).await {
            Ok(_) => {
                gw.db(move |a| a.mark_initial_level_set(device)).await;
                info!(mac = %mac, "new player: LMS volume set to 100");
                true
            }
            Err(e) => {
                warn!(mac = %mac, error = %e, "could not set a new player's LMS volume to 100");
                false
            }
        }
    }

    /// Whether the last `strm` pointed elsewhere and was refused; cleared on reading.
    fn take_refused_direct(&self) -> bool {
        std::mem::take(&mut self.state.lock().unwrap().refused_direct)
    }

    /// After a refused direct stream: make sure LMS proxies for this player, then have it
    /// replay the current entry, which it now serves itself. At most [`MAX_REPLAYS`] times
    /// in a row, in case the server will not proxy; a stream that is accepted resets that.
    fn replay_proxied(self: &Arc<Self>, gw: &crate::AppState) {
        {
            let mut st = self.state.lock().unwrap();
            if st.replays >= MAX_REPLAYS {
                warn!(mac = %self.device.mac_string(), "LMS keeps direct-streaming; not asking again");
                return;
            }
            st.replays += 1;
            st.proxied_pref = PrefState::Pending;
        }
        let (gw, session) = (gw.clone(), Arc::clone(self));
        tokio::spawn(async move {
            if !session.set_proxied_pref(&gw).await {
                return;
            }
            let mac = session.device.mac_string();
            let cmd: Vec<String> = ["playlist", "jump", "+0"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            match gw.lms_command(&mac, &cmd).await {
                Ok(_) => info!(mac = %mac, "asked LMS to replay the stream through itself"),
                Err(e) => warn!(mac = %mac, error = %e, "could not ask LMS to replay the stream"),
            }
        });
    }

    /// Record the stream a `strm s` asks the player to fetch, if policy allows it. Returns
    /// the payload to pass on instead, when the stream is to be re-encoded to Opus.
    fn note_strm(&self, gw: &Gateway, payload: &[u8]) -> Option<Vec<u8>> {
        let Ok(s) = Strm::parse(payload) else {
            return None;
        };
        if s.command != b's' {
            return None;
        }
        let ip = Ipv4Addr::from(s.server_ip);
        let direct_elsewhere = !(ip.is_unspecified() || gw.cfg.lms_ips.contains(&ip));
        let path = if !direct_elsewhere {
            match validate_stream_request(&s.request, &self.device.mac) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(mac = %self.device.mac_string(), error = %e, "refused stream request");
                    None
                }
            }
        } else {
            // LMS asked the player to fetch an internet station itself (direct streaming).
            // These players only ever talk to the gateway, so the reader has LMS proxy it
            // instead (see `replay_proxied`).
            warn!(mac = %self.device.mac_string(), %ip, "refused direct stream to non-LMS address");
            None
        };
        let mut st = self.state.lock().unwrap();
        st.stream_seq += 1;
        st.stream_path = path;
        st.stream_format = s.format;
        st.stream_served = st
            .stream_path
            .is_some()
            .then(|| crate::stream_info::Served::from_strm(&s));
        st.refused_direct = direct_elsewhere;
        if !direct_elsewhere {
            st.replays = 0;
        }
        // The previous stream's upstream is abandoned: cancelled, so its LMS connection
        // and any encoder (and its slot) go now, not when the stream next times out.
        if let Some(old) = st.relay.take() {
            old.cancel();
        }
        st.live = None; // and an unused encode slot given back
        if st.stream_path.is_none() {
            // Refused: the player can't fetch it anyway, so it isn't told where it was
            // (another address, or a request that isn't LMS's own).
            return Some(
                Strm {
                    server_ip: 0,
                    request: Vec::new(),
                    ..s
                }
                .encode_payload(),
            );
        }
        let bitrate = crate::live::wanted(s.format, st.quality, gw.live_ffmpeg().is_some())?;
        let Some(permit) = gw.live_slot() else {
            info!(mac = %self.device.mac_string(), "no free live-encode slot; sending the stream as it is");
            return None;
        };
        let rewritten = crate::live::rewrite_strm(payload)?;
        st.live = Some((st.quality, bitrate, permit));
        Some(rewritten)
    }

    /// The upstream response for stream `n` is in: tell the client what LMS served (with a
    /// FLAC stream's format, `flac`, read from its first bytes) and what the gateway sends.
    /// Sent now to an attached socket, and kept to send again after a resume greeting.
    /// Nothing for a stream that is no longer the current one.
    pub fn announce_stream(
        &self,
        n: u64,
        flac: Option<crate::stream_info::FlacFormat>,
        sent: crate::stream_info::Sent,
    ) {
        let mut st = self.state.lock().unwrap();
        if st.ended || n != st.stream_seq {
            return;
        }
        let Some(served) = st.stream_served.clone() else {
            return;
        };
        let served = if served.codec == "flac" {
            served.with_flac(flac.unwrap_or_default())
        } else {
            served
        };
        let msg = crate::stream_info::message(n, &served, sent);
        debug!(mac = %self.device.mac_string(), stream = n, message = %msg, "stream message");
        st.stream_message = Some(msg.clone());
        if let Some(a) = &st.attached {
            // A socket too slow for this is kicked by the frames behind it; its resume
            // greeting brings the message again.
            let _ = a.tx.try_send(Outbound::Text(msg));
        }
    }
}

/// Keep only well-formed capability tokens: printable ASCII, comma-separated, each either a
/// bare codec name (`flc`) or `Key=value`, with bounded lengths. Anything else is dropped.
pub(crate) fn sanitise_capabilities(caps: &str) -> String {
    let mut out = String::new();
    for tok in caps.split(',').take(MAX_CAPABILITY_TOKENS) {
        if !capability_ok(tok) || out.len() + tok.len() + 1 > MAX_CAPABILITIES {
            continue;
        }
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(tok);
    }
    out
}

fn capability_ok(tok: &str) -> bool {
    let word = |s: &str, max: usize| {
        !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    if tok.len() > 128 || !tok.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return false;
    }
    match tok.split_once('=') {
        Some((key, value)) => word(key, 32) && !value.contains('=') && !value.contains("://"),
        None => word(tok, 16),
    }
}

/// The SETD id-0 payload to forward for a client's name reply (`data` excludes the id).
/// A name that is empty, too long, has control characters, or looks like a URL or path is
/// replaced by the device's name, so a player can't give itself a name LMS might fetch.
/// Whatever is kept goes through the same cleaning as device names
/// ([`crate::accounts::clean_device_name`]): no markup, bidi or other format characters.
fn name_reply(data: &[u8], fallback: &str) -> Vec<u8> {
    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    let acceptable = |n: &str| {
        let lower = n.to_ascii_lowercase();
        !n.trim().is_empty()
            && n.chars().count() <= MAX_NAME
            && !n.chars().any(char::is_control)
            && !lower.contains("://")
            && !lower.contains(":\\")
            && !n.starts_with('/')
            && !n.starts_with('\\')
            && !lower.starts_with("file:")
    };
    let name = std::str::from_utf8(&data[..end])
        .ok()
        .filter(|n| acceptable(n))
        .and_then(crate::accounts::clean_name)
        .unwrap_or_else(|| crate::accounts::clean_device_name(fallback))
        .chars()
        .take(MAX_NAME)
        .collect::<String>();
    let mut out = vec![0];
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out
}

/// For the session's lifetime: while the attached client is silent (its socket may have
/// died without either side noticing yet), report to LMS on its behalf, as while detached,
/// so LMS doesn't drop the player and end the session before the client can resume.
async fn speak_for_silent_client(gw: AppState, session: Arc<Session>) {
    let mut every = tokio::time::interval(DETACHED_HEARTBEAT);
    every.tick().await;
    loop {
        every.tick().await;
        if session.is_ended() {
            return;
        }
        if session.attached_but_silent(gw.cfg.client_silence) {
            session.heartbeat(0).await;
        }
    }
}

/// Read LMS → client frames for the session's lifetime.
async fn read_lms(gw: AppState, session: Arc<Session>, mut rd: tokio::net::tcp::OwnedReadHalf) {
    let mut reader = ServerFrameReader::default();
    let mut buf = vec![0u8; 32 * 1024];
    let reason = loop {
        let n = match rd.read(&mut buf).await {
            Ok(0) | Err(_) => break EndReason::LmsClosed,
            Ok(n) => n,
        };
        reader.push(&buf[..n]);
        let mut overflow = false;
        loop {
            let (op, mut payload) = match reader.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "bad framing from LMS");
                    overflow = true; // treat as fatal for the session
                    break;
                }
            };
            // LMS knows the player once it sends it anything: have it proxy internet streams.
            session.ensure_proxied_streaming(&gw);
            // ... and give a brand-new device's player its initial volume.
            session.ensure_initial_level(&gw);
            match &op {
                // Never let a remote device be redirected to another server.
                b"serv" => {
                    warn!(mac = %session.device.mac_string(), "dropped serv from LMS");
                    continue;
                }
                b"strm" => {
                    if let Ok(s) = Strm::parse(&payload)
                        && s.command == b't'
                        && session.is_detached()
                    {
                        // Answer the status poll ourselves; a stale poll is useless later.
                        session.heartbeat(s.replay_gain).await;
                        continue;
                    }
                    if let Some(p) = session.note_strm(&gw, &payload) {
                        payload = p;
                    }
                    if session.take_refused_direct() {
                        session.replay_proxied(&gw);
                    }
                }
                b"setd" => session.note_setd(&payload),
                _ => {}
            }
            match session.pass_from_lms(op, &payload, gw.cfg.detached_queue_limit) {
                None | Some(Delivery::Ok) => {}
                Some(Delivery::Detached(generation)) => session.hold_for_resume(&gw, generation),
                Some(Delivery::Overflow) => {
                    warn!(mac = %session.device.mac_string(), "detached queue overflow");
                    session.end(&gw, EndReason::QueueOverflow).await;
                    return;
                }
            }
        }
        if overflow {
            break EndReason::LmsClosed;
        }
        if session.is_ended() {
            return;
        }
    };
    session.end(&gw, reason).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Role;
    use tokio::net::TcpListener;

    const MAC: [u8; 6] = [0x02, 0x4c, 0x41, 0, 0, 1];

    /// A session wired to a local listener that swallows whatever is forwarded.
    async fn session() -> (Arc<Session>, u64) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let conn = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (mut peer, _) = l.accept().await.unwrap();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            while matches!(peer.read(&mut buf).await, Ok(n) if n > 0) {}
        });
        let (_, wr) = conn.into_split();
        let device = Principal {
            user_id: 1,
            username: "listener".into(),
            role: Role::Listener,
            device_id: 1,
            device_name: "Test device".into(),
            mac: MAC,
        };
        let s = Arc::new(Session::new(device, wr, std::sync::Weak::new()));
        let g = s.attach(0).unwrap().generation;
        (s, g)
    }

    fn frame(op: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = op.to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn helo(caps: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 36];
        p[0] = DEVICE_ID_SOFTWARE;
        p[2..8].copy_from_slice(&MAC);
        p.extend_from_slice(caps);
        frame(b"HELO", &p)
    }

    #[test]
    fn capabilities_keep_only_well_formed_tokens() {
        let caps = "Model=squeezelite,ModelName=Lyrion Anywhere,MaxSampleRate=192000,flc,mp3,\
                    evil\r\nX-Header: 1,<script>,a=b=c,Url=http://lms.example.lan/x,,\u{e9}t\u{e9},\
                    Key With Space=1,toolongcodecname0123";
        assert_eq!(
            sanitise_capabilities(caps),
            "Model=squeezelite,ModelName=Lyrion Anywhere,MaxSampleRate=192000,flc,mp3"
        );
        let many = "flc,".repeat(500);
        assert_eq!(
            sanitise_capabilities(&many).split(',').count(),
            MAX_CAPABILITY_TOKENS
        );
        let huge = vec![format!("A={}", "x".repeat(100)); 60].join(",");
        assert!(sanitise_capabilities(&huge).len() <= MAX_CAPABILITIES);
    }

    #[test]
    fn hostile_names_are_replaced() {
        for bad in [
            &b"http://lms.example.lan/imageproxy/x"[..],
            b"/srv/music/a.flac",
            b"file:///etc/passwd",
            b"C:\\music",
            b"tab\there",
            b"",
            b"   ",
            &[0xff, 0xfe],
            &[b'x'; 65],
        ] {
            let mut d = bad.to_vec();
            d.push(0);
            assert_eq!(name_reply(&d, "Fallback"), b"\0Fallback\0", "{bad:?}");
        }
        // Kept names are cleaned like device names: markup and punctuation beyond the basic
        // set go (the colon here), as do bidi and zero-width characters.
        assert_eq!(name_reply(b"Kitchen: left\0", "F"), b"\0Kitchen left\0");
        assert_eq!(name_reply(b"Caf\xc3\xa9", "F"), "\0Café\0".as_bytes());
        for (raw, want) in [
            ("<b>Den</b>", "bDenb"),
            ("Kid\u{202e}s room", "Kids room"),
            ("Zero\u{200b}width\u{2066}", "Zerowidth"),
            ("Bob's (spare) & co.", "Bob's (spare) & co."),
            ("Kanji 台所", "Kanji 台所"),
        ] {
            let mut d = raw.as_bytes().to_vec();
            d.push(0);
            let want = format!("\0{want}\0");
            assert_eq!(name_reply(&d, "F"), want.as_bytes(), "{raw:?}");
        }
        // Nothing left after cleaning: the device's name, itself cleaned.
        for bad in ["\u{202e}\u{200b}", "<>!?"] {
            let mut d = bad.as_bytes().to_vec();
            d.push(0);
            assert_eq!(name_reply(&d, "Phone\u{202e}"), b"\0Phone\0", "{bad:?}");
        }
    }

    #[tokio::test]
    async fn only_player_opcodes_are_forwarded() {
        let refused: [&[u8; 4]; 12] = [
            b"IR  ",
            b"BUTN",
            b"KNOB",
            b"BODY",
            b"UREQ",
            b"SHUT",
            b"SETD",
            b"ALSS",
            b"RAWI",
            b"ANIC",
            b"stat",
            b"\0\0\0\0",
        ];
        for op in refused {
            let (s, g) = session().await;
            assert!(matches!(
                s.client_bytes(g, &helo(b"flc")).await,
                ClientInput::Ok
            ));
            let r = s.client_bytes(g, &frame(op, &[0; 8])).await;
            assert!(matches!(r, ClientInput::Policy(_)), "{op:?}: {r:?}");
        }
        let (s, g) = session().await;
        let mut ok = helo(b"flc");
        for f in [
            frame(b"STAT", &[0; 53]),
            frame(b"RESP", b"HTTP/1.0 200 OK\r\n\r\n"),
            frame(b"META", b"StreamTitle='x';"),
            frame(b"DSCO", &[0]),
            frame(b"BYE!", &[0]),
        ] {
            ok.extend(f);
        }
        assert!(matches!(s.client_bytes(g, &ok).await, ClientInput::Ok));
    }

    #[tokio::test]
    async fn setd_only_answers_a_name_query() {
        let (s, g) = session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        let reply = frame(b"SETD", b"\0Kitchen\0");
        assert!(matches!(
            s.client_bytes(g, &reply).await,
            ClientInput::Policy(_)
        ));
        let (s, g) = session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        s.note_setd(&[0]);
        assert!(matches!(s.client_bytes(g, &reply).await, ClientInput::Ok));
        // One query, one answer.
        assert!(matches!(
            s.client_bytes(g, &reply).await,
            ClientInput::Policy(_)
        ));
        // A rename from LMS isn't a query.
        let (s, g) = session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        s.note_setd(b"\0New name\0");
        assert!(matches!(
            s.client_bytes(g, &reply).await,
            ClientInput::Policy(_)
        ));
        // Other setting IDs are never accepted.
        let (s, g) = session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        s.note_setd(&[0]);
        let other = frame(b"SETD", b"\x04\x01");
        assert!(matches!(
            s.client_bytes(g, &other).await,
            ClientInput::Policy(_)
        ));
    }

    #[tokio::test]
    async fn helo_must_be_a_software_player() {
        let (s, g) = session().await;
        let mut h = helo(b"flc");
        h[8] = 4; // Squeezebox 2
        assert!(matches!(
            s.client_bytes(g, &h).await,
            ClientInput::Policy(_)
        ));
    }

    #[tokio::test]
    async fn malformed_fixed_size_frames_are_refused() {
        for f in [
            frame(b"STAT", &[0; 10]),
            frame(b"DSCO", &[]),
            frame(b"DSCO", &[0, 0]),
            frame(b"BYE!", &[0, 0]),
        ] {
            let (s, g) = session().await;
            s.client_bytes(g, &helo(b"flc")).await;
            let r = s.client_bytes(g, &f).await;
            assert!(matches!(r, ClientInput::Policy(_)), "{f:?}: {r:?}");
        }
    }

    /// Regression: after a resume took over, the old socket's reader kept feeding frames
    /// into the session.
    #[tokio::test]
    async fn a_replaced_socket_cannot_feed_the_session() {
        let (s, old) = session().await;
        assert!(matches!(
            s.client_bytes(old, &helo(b"flc")).await,
            ClientInput::Ok
        ));
        let new = s.attach(0).unwrap();
        let stat = frame(b"STAT", &[0; 53]);
        assert!(matches!(
            s.client_bytes(old, &stat).await,
            ClientInput::Stale
        ));
        assert!(matches!(
            s.client_bytes(new.generation, &stat).await,
            ClientInput::Ok
        ));
    }

    #[tokio::test]
    async fn a_replaced_socket_is_kicked() {
        let (s, _) = session().await;
        let first = s.attach(0).unwrap();
        let _second = s.attach(0).unwrap();
        assert_eq!(
            *first.kick.borrow(),
            Some((4000, "replaced by a newer connection"))
        );
    }

    /// Regression: a socket whose outbound channel had closed (its writer gone before the
    /// socket handler noticed) was detached without the grace timer or stand-in
    /// heartbeats, so LMS dropped the player after about 15 s and the client could not
    /// resume. It now takes the same path as a socket that couldn't keep up.
    #[tokio::test]
    async fn a_closed_socket_channel_is_held_for_resume() {
        let (s, g) = session().await; // (its receiver already dropped)
        let d = s.deliver(frame(b"strm", &[b'q'; 24]), usize::MAX);
        assert!(matches!(d, Delivery::Detached(x) if x == g));
        assert!(s.is_detached());
        // The frame is kept for the resume.
        let resumed = s.attach(0).unwrap();
        assert_eq!(resumed.rx.len(), 1);
    }

    fn stat_frame(event: &[u8; 4], jiffies: u32, elapsed_ms: u32) -> Vec<u8> {
        ClientMsg::Stat(Stat {
            event: *event,
            jiffies,
            elapsed_seconds: elapsed_ms / 1000,
            elapsed_milliseconds: elapsed_ms,
            ..Default::default()
        })
        .encode()
    }

    /// The last report was `ago` before now.
    fn age_last_stat(s: &Session, ago: Duration) {
        let mut st = s.state.lock().unwrap();
        let (stat, _) = st.last_stat.clone().unwrap();
        st.last_stat = Some((stat, Instant::now().checked_sub(ago).unwrap()));
    }

    fn heartbeat_elapsed(s: &Session) -> (u32, u32) {
        let f = s.synthesised_stat(0).unwrap();
        let stat = Stat::parse(&f[8..]).unwrap();
        (stat.elapsed_milliseconds, stat.elapsed_seconds)
    }

    /// The jiffies at which the heartbeat says the track started (its jiffies less its
    /// elapsed time): what LMS works out the player's play point from.
    fn heartbeat_start(s: &Session) -> u32 {
        let f = s.synthesised_stat(0).unwrap();
        let stat = Stat::parse(&f[8..]).unwrap();
        stat.jiffies.wrapping_sub(stat.elapsed_milliseconds)
    }

    fn two_minute_track() -> crate::history::TrackInfo {
        crate::history::TrackInfo {
            id: 7,
            key: "0".repeat(64),
            album_id: None,
            artist_id: None,
            year: None,
            duration: Some(120.0),
        }
    }

    /// Regression (live, 2026-10-15): while the client is away (the rolling cache lets that
    /// be minutes) it plays on, past the end of the track LMS knows and into the next. The
    /// heartbeats used to stop the elapsed time at that track's end while their clock ran
    /// on, so to LMS the player looked stalled: in a sync group it sent `skipAhead`
    /// corrections every 2 s for five minutes, and corrected the other members against that
    /// frozen reference. While the client was playing, elapsed advances with the clock, so
    /// the play point LMS works out stays put; only a pause or a stop freezes it.
    #[tokio::test]
    async fn heartbeats_keep_advancing_past_the_end_of_the_track() {
        let (s, g) = session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        let r = s
            .client_bytes(g, &stat_frame(b"STMs", 1_000, 100_000))
            .await;
        assert!(matches!(r, ClientInput::Ok));
        let start = 1_000u32.wrapping_sub(100_000);
        age_last_stat(&s, Duration::from_secs(30));
        assert!(heartbeat_elapsed(&s).0 >= 130_000);
        // LMS says it is a two-minute track: elapsed still runs on past its end.
        s.state
            .lock()
            .unwrap()
            .tracker
            .resolved(1, Some(two_minute_track()), Instant::now());
        assert!(heartbeat_elapsed(&s).0 >= 130_000);
        assert_eq!(heartbeat_start(&s), start, "never looks stalled");
        age_last_stat(&s, Duration::from_secs(600));
        assert!(heartbeat_elapsed(&s).0 >= 700_000);
        assert_eq!(heartbeat_start(&s), start);
        // What the client itself reports is passed on.
        let r = s
            .client_bytes(g, &stat_frame(b"STMt", 700_000, 125_000))
            .await;
        assert!(matches!(r, ClientInput::Ok));
        age_last_stat(&s, Duration::from_secs(10));
        assert!(heartbeat_elapsed(&s).0 >= 135_000);
        assert_eq!(heartbeat_start(&s), 700_000 - 125_000);
        // Paused or stopped: frozen, as before.
        for stop in [b"STMp", b"STMf", b"STMn"] {
            let (s, g) = session().await;
            s.client_bytes(g, &helo(b"flc")).await;
            s.client_bytes(g, &stat_frame(b"STMs", 1_000, 100_000))
                .await;
            s.client_bytes(g, &stat_frame(stop, 2_000, 101_000)).await;
            age_last_stat(&s, Duration::from_secs(30));
            assert_eq!(heartbeat_elapsed(&s), (101_000, 101), "{stop:?}");
        }
    }

    /// Regression (live, 2026-10-15): a client that reported a track's end (`STMu`) plays
    /// on from its own cache while away; the heartbeats for it must not look stalled
    /// either. With a known two-minute track, 30 s and 300 s later the elapsed time has
    /// moved on with the clock and the start LMS works out is the same.
    #[tokio::test]
    async fn heartbeats_advance_after_a_track_end_while_away() {
        let (s, g) = session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        s.client_bytes(g, &stat_frame(b"STMs", 1_000, 100_000))
            .await;
        s.state
            .lock()
            .unwrap()
            .tracker
            .resolved(1, Some(two_minute_track()), Instant::now());
        s.client_bytes(g, &stat_frame(b"STMu", 21_000, 120_000))
            .await;
        let start = 21_000u32.wrapping_sub(120_000);
        let mut last = 120_000;
        for ago in [30, 300] {
            age_last_stat(&s, Duration::from_secs(ago));
            let (ms, _) = heartbeat_elapsed(&s);
            assert!(ms >= 120_000 + ago as u32 * 1000, "{ago} s: {ms}");
            assert!(ms > last, "never frozen");
            last = ms;
            assert_eq!(heartbeat_start(&s), start, "{ago} s");
        }
    }

    /// A `strm` frame from LMS with this command and interval (or start time).
    fn strm_cmd(command: u8, interval: u32) -> Vec<u8> {
        Strm {
            command,
            autostart: b'0',
            format: b'f',
            pcm_sample_size: b'?',
            pcm_sample_rate: b'?',
            pcm_channels: b'?',
            pcm_endianness: b'?',
            threshold: 0,
            spdif_enable: 0,
            transition_period: 0,
            transition_type: b'0',
            flags: 0,
            output_threshold: 0,
            slaves: 0,
            replay_gain: interval,
            server_port: 9000,
            server_ip: 0,
            request: Vec::new(),
        }
        .encode_payload()
    }

    /// Regression (live, 2026-10-15): while the client is away, LMS's sync corrections for
    /// it (`strm a` skip ahead, `strm p` with an interval, `strm u` at a time) were kept and
    /// replayed on resume, minutes late: the player would have jumped by their sum. They
    /// are dropped while detached. Everything else (stop, start, pause, unpause now, volume)
    /// is kept and delivered on resume, in order.
    #[tokio::test]
    async fn sync_corrections_are_not_kept_for_a_detached_client() {
        let (s, _) = session().await;
        s.state.lock().unwrap().attached = None;
        let frames = [
            (*b"strm", strm_cmd(b'a', 902), false),
            (*b"strm", strm_cmd(b'q', 0), true),
            (*b"strm", strm_cmd(b'a', 2902), false),
            (*b"strm", strm_cmd(b'p', 500), false),
            (*b"strm", strm_cmd(b'u', 123_456), false),
            (*b"strm", strm_cmd(b'a', 902), false),
            (*b"strm", strm_cmd(b'p', 0), true),
            (*b"strm", strm_cmd(b'u', 0), true),
            (*b"strm", strm_cmd(b's', 0), true),
            (*b"audg", vec![0; 18], true),
        ];
        for (op, payload, kept) in &frames {
            match s.pass_from_lms(*op, payload, 1 << 20) {
                Some(Delivery::Ok) => assert!(kept),
                None => assert!(!kept),
                _ => panic!("unexpected delivery"),
            }
        }
        let mut a = s.attach(0).unwrap();
        let mut got = Vec::new();
        while let Ok(Outbound::Frame(f)) = a.rx.try_recv() {
            got.push(f);
        }
        let want: Vec<Vec<u8>> = frames
            .iter()
            .filter(|(_, _, kept)| *kept)
            .map(|(op, p, _)| encode_server_frame(*op, p))
            .collect();
        assert_eq!(got, want);
        // Attached, every correction goes straight through as before.
        for (op, payload, _) in &frames {
            assert!(s.pass_from_lms(*op, payload, 1 << 20).is_some());
        }
        let mut n = 0;
        while let Ok(Outbound::Frame(_)) = a.rx.try_recv() {
            n += 1;
        }
        assert_eq!(n, frames.len());
    }

    /// A session wired to a listener that records what reaches LMS.
    async fn recording_session() -> (Arc<Session>, u64, Arc<Mutex<Vec<u8>>>) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let conn = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
        let (mut peer, _) = l.accept().await.unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let rec = seen.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = peer.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                rec.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        let (_, wr) = conn.into_split();
        let device = Principal {
            user_id: 1,
            username: "listener".into(),
            role: Role::Listener,
            device_id: 1,
            device_name: "Test device".into(),
            mac: MAC,
        };
        let s = Arc::new(Session::new(device, wr, std::sync::Weak::new()));
        let g = s.attach(0).unwrap().generation;
        (s, g, seen)
    }

    /// The STAT events and jiffies LMS has received, once a `DSCO` sent after them as
    /// marker number `mark` (counting from 1, one per call) has arrived.
    async fn lms_stats(
        s: &Session,
        g: u64,
        seen: &Mutex<Vec<u8>>,
        mark: usize,
    ) -> Vec<([u8; 4], u32)> {
        let marker = frame(b"DSCO", &[0]);
        assert!(matches!(s.client_bytes(g, &marker).await, ClientInput::Ok));
        for _ in 0..500 {
            let bytes = seen.lock().unwrap().clone();
            let (mut out, mut marks) = (Vec::new(), 0);
            let mut rest = &bytes[..];
            while rest.len() >= 8 {
                let len = u32::from_be_bytes(rest[4..8].try_into().unwrap()) as usize;
                if rest.len() < 8 + len {
                    break;
                }
                match &rest[..4] {
                    b"STAT" => {
                        let stat = Stat::parse(&rest[8..8 + len]).unwrap();
                        out.push((stat.event, stat.jiffies));
                    }
                    b"DSCO" => marks += 1,
                    _ => {}
                }
                rest = &rest[8 + len..];
            }
            if marks == mark {
                return out;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("marker {mark} never reached LMS");
    }

    /// Regression (live, 2026-10-15): a client resuming after a long gap sent every
    /// heartbeat it had held meanwhile, all at once (about 250 after four minutes). LMS had
    /// been told a later clock by the gateway's own heartbeats, re-based its clock for the
    /// player on each stale one, and threw the sync group out. After the gateway has
    /// spoken for the client, a client `STMt` older than what it told LMS goes no further;
    /// newer ones, and every other event however old, are forwarded as before.
    #[tokio::test]
    async fn stale_heartbeats_after_a_resume_never_reach_lms() {
        let (s, g, seen) = recording_session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        s.client_bytes(g, &stat_frame(b"STMs", 1_000, 100_000))
            .await;
        // Away for 30 s: the gateway tells LMS the clock is now about 31 000.
        age_last_stat(&s, Duration::from_secs(30));
        s.heartbeat(0).await;
        let told = lms_stats(&s, g, &seen, 1).await;
        assert_eq!(told.len(), 2);
        assert_eq!(&told[1].0, b"STMt");
        assert!(told[1].1 >= 31_000);
        // Back: the client's held frames arrive, oldest first.
        let g = s.attach(0).unwrap().generation;
        for (event, jiffies) in [
            (b"STMt", 2_000),
            (b"STMt", 3_000),
            (b"STMs", 3_500),
            (b"STMt", 4_000),
        ] {
            let r = s.client_bytes(g, &stat_frame(event, jiffies, 0)).await;
            assert!(matches!(r, ClientInput::Ok));
        }
        let after = lms_stats(&s, g, &seen, 2).await;
        assert_eq!(
            after[2..],
            [(*b"STMs", 3_500)],
            "only the event, none of the stale heartbeats"
        );
        // A fresh heartbeat, newer than what LMS was told, goes through.
        let fresh = told[1].1 + 500;
        s.client_bytes(g, &stat_frame(b"STMt", fresh, 0)).await;
        let after = lms_stats(&s, g, &seen, 3).await;
        assert_eq!(after[2..], [(*b"STMs", 3_500), (*b"STMt", fresh)]);
        // The dropped ones never became the report the gateway speaks from.
        assert_eq!(
            s.state
                .lock()
                .unwrap()
                .last_stat
                .as_ref()
                .unwrap()
                .0
                .jiffies,
            fresh
        );
    }

    /// Negative: a gateway that never spoke for the client forwards every heartbeat,
    /// whatever its clock says (a client's clock is its own business).
    #[tokio::test]
    async fn heartbeats_are_forwarded_when_the_gateway_never_spoke() {
        let (s, g, seen) = recording_session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        for (event, jiffies) in [(b"STMs", 5_000), (b"STMt", 6_000), (b"STMt", 1_000)] {
            s.client_bytes(g, &stat_frame(event, jiffies, 0)).await;
        }
        let g = s.attach(0).unwrap().generation;
        s.client_bytes(g, &stat_frame(b"STMt", 500, 0)).await;
        assert_eq!(
            lms_stats(&s, g, &seen, 1).await,
            [
                (*b"STMs", 5_000),
                (*b"STMt", 6_000),
                (*b"STMt", 1_000),
                (*b"STMt", 500)
            ]
        );
    }

    /// The guard is short-lived: a client whose clock went back (a restarted app resuming
    /// its session) is not silenced for long. Jiffies wrap after 49 days; a heartbeat just
    /// past the wrap is newer, not older.
    #[tokio::test]
    async fn the_stale_heartbeat_guard_expires_and_handles_the_wrap() {
        let (s, g, seen) = recording_session().await;
        s.client_bytes(g, &helo(b"flc")).await;
        s.client_bytes(g, &stat_frame(b"STMs", u32::MAX - 1_000, 0))
            .await;
        s.heartbeat(0).await;
        let g = s.attach(0).unwrap().generation;
        s.client_bytes(g, &stat_frame(b"STMt", u32::MAX - 2_000, 0))
            .await;
        s.client_bytes(g, &stat_frame(b"STMt", 500, 0)).await;
        let seen_now = lms_stats(&s, g, &seen, 1).await;
        assert_eq!(seen_now.last(), Some(&(*b"STMt", 500)), "{seen_now:?}");
        assert_eq!(seen_now.len(), 3, "the stale one dropped: {seen_now:?}");
        // The guard set by a heartbeat long ago no longer applies.
        s.heartbeat(0).await;
        {
            let mut st = s.state.lock().unwrap();
            let (j, _) = st.told.unwrap();
            st.told = Some((j, Instant::now().checked_sub(STALE_GUARD).unwrap()));
        }
        s.client_bytes(g, &stat_frame(b"STMt", 10, 0)).await;
        let seen_now = lms_stats(&s, g, &seen, 2).await;
        assert_eq!(seen_now.last(), Some(&(*b"STMt", 10)), "{seen_now:?}");
    }

    /// Small deterministic PRNG (xorshift64*), so failures are reproducible from the seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
    }

    /// Random, truncated and oversize client bytes, in random chunks: never a panic, and
    /// never anything but acceptance or a policy refusal.
    #[tokio::test]
    async fn random_client_bytes_never_panic() {
        const OPS: [&[u8; 4]; 12] = [
            b"HELO", b"STAT", b"RESP", b"META", b"DSCO", b"BYE!", b"SETD", b"IR  ", b"BUTN",
            b"UREQ", b"SHUT", b"KNOB",
        ];
        for seed in 1..=200u64 {
            let mut r = Rng(seed);
            let (s, g) = session().await;
            if r.below(4) != 0 {
                let n = r.below(64) as usize;
                let caps = r.bytes(n);
                s.client_bytes(g, &helo(&caps)).await;
            }
            let mut stream = Vec::new();
            for _ in 0..r.below(20) {
                match r.below(10) {
                    0 => {
                        let n = r.below(64) as usize;
                        stream.extend(r.bytes(n));
                    }
                    1 => {
                        // Oversize length claim.
                        let op = OPS[r.below(OPS.len() as u64) as usize];
                        stream.extend_from_slice(op);
                        stream.extend_from_slice(&(r.next() as u32 | 0x10_0000).to_be_bytes());
                    }
                    _ => {
                        let op = OPS[r.below(OPS.len() as u64) as usize];
                        let len = r.below(80) as usize;
                        let mut f = frame(op, &r.bytes(len));
                        if r.below(5) == 0 {
                            f.truncate(r.below(f.len() as u64 + 1) as usize);
                        }
                        stream.extend(f);
                    }
                }
                if r.below(8) == 0 {
                    s.note_setd(&[0]);
                }
            }
            let mut rest = &stream[..];
            while !rest.is_empty() {
                let n = (1 + r.below(100) as usize).min(rest.len());
                let (chunk, tail) = rest.split_at(n);
                rest = tail;
                match s.client_bytes(g, chunk).await {
                    ClientInput::Ok => {}
                    ClientInput::Policy(_) => break,
                    other => panic!("seed {seed}: unexpected {other:?}"),
                }
            }
        }
    }
}
