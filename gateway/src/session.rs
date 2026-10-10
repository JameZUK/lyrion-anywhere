//! A player session: the gateway's Slimproto connection to LMS for one device.
//!
//! The session outlives individual client sockets. While a socket is attached, LMS frames
//! go straight to it; while detached (the client's network dropped), frames are queued for
//! up to [`Config::resume_grace`](crate::Config::resume_grace) and delivered on resume.
//! LMS never sees the player leave.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;
use tracing::{info, warn};

use anywhere_core::proto::{
    ClientFrameReader, ClientMsg, Helo, ServerFrameReader, Stat, Strm, encode_server_frame,
    validate_stream_request,
};

use crate::devices::{Device, random_id};
use crate::relay::Relay;
use crate::{AppState, Gateway};

/// What the attached socket's writer task should do next.
#[derive(Debug)]
pub enum Outbound {
    Frame(Vec<u8>),
    Close(u16, &'static str),
}

/// Per-socket queue towards the client. Bounded: a client that can't keep up is detached
/// (it can resume) rather than letting memory grow.
pub const OUTBOUND_QUEUE: usize = 1024;

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
}

struct Attachment {
    generation: u64,
    tx: mpsc::Sender<Outbound>,
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
    /// `None` when the current `strm` was refused by policy.
    stream_path: Option<String>,
    relay: Option<Arc<tokio::sync::Mutex<Relay>>>,
    /// Client → LMS framing; persists across sockets so a frame split by a drop is safe.
    from_client: ClientFrameReader,
    /// The client's most recent status report, for heartbeats while detached.
    last_stat: Option<(Stat, Instant)>,
    /// Whether the client last reported itself playing (elapsed time advances).
    playing: bool,
}

/// How often the gateway reports status to LMS on behalf of a detached client. LMS drops
/// players it hasn't heard from for roughly 15 s.
const DETACHED_HEARTBEAT: Duration = Duration::from_secs(1);

pub struct Session {
    pub id: String,
    pub device: Arc<Device>,
    secret: String,
    lms_tx: tokio::sync::Mutex<OwnedWriteHalf>,
    state: Mutex<State>,
}

pub enum ClientInput {
    /// Frames accepted and forwarded.
    Ok,
    /// The client broke protocol; the session has been ended.
    Policy(String),
    /// Writing to LMS failed; the session has been ended.
    LmsGone,
}

impl Session {
    /// Connect to LMS and start the session's LMS reader.
    pub async fn open(gw: &AppState, device: Arc<Device>) -> std::io::Result<Arc<Session>> {
        let lms = tokio::time::timeout(
            gw.cfg.dial_timeout,
            TcpStream::connect(&gw.cfg.lms_slimproto),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "LMS connect timeout"))??;
        let _ = lms.set_nodelay(true);
        let (rd, wr) = lms.into_split();
        let session = Arc::new(Session {
            id: random_id(),
            device,
            secret: random_id(),
            lms_tx: tokio::sync::Mutex::new(wr),
            state: Mutex::new(State {
                attached: None,
                next_generation: 0,
                log: VecDeque::new(),
                log_bytes: 0,
                last_seq: 0,
                sent_upto: 0,
                seen_helo: false,
                ended: false,
                stream_seq: 0,
                stream_path: None,
                relay: None,
                from_client: ClientFrameReader::default(),
                last_stat: None,
                playing: false,
            }),
        });

        // One session per device: LMS would otherwise see two players with the same MAC.
        let previous: Vec<Arc<Session>> = {
            let mut sessions = gw.sessions.lock().unwrap();
            let prev = sessions
                .values()
                .filter(|s| Arc::ptr_eq(&s.device, &session.device))
                .cloned()
                .collect();
            sessions.insert(session.id.clone(), session.clone());
            prev
        };
        for p in previous {
            p.end(gw, EndReason::Replaced).await;
        }

        tokio::spawn(read_lms(gw.clone(), session.clone(), rd));
        Ok(session)
    }

    pub fn secret_matches(&self, candidate: &str) -> bool {
        crate::secret_eq(&self.secret, candidate)
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn is_ended(&self) -> bool {
        self.state.lock().unwrap().ended
    }

    /// Attach a client socket that has received `received` frames of this session so far
    /// (0 for a new session), replacing any existing socket. Frames after `received` are
    /// queued for it straight away.
    pub fn attach(&self, received: u64) -> Result<(u64, mpsc::Receiver<Outbound>), AttachError> {
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
            let _ = old
                .tx
                .try_send(Outbound::Close(4000, "replaced by a newer connection"));
        }
        for f in resend {
            let _ = tx.try_send(Outbound::Frame(f)); // capacity reserved above
        }
        st.sent_upto = st.last_seq;
        st.next_generation += 1;
        let generation = st.next_generation;
        st.attached = Some(Attachment { generation, tx });
        Ok((generation, rx))
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
        tokio::spawn(async move {
            tokio::time::sleep(gw.cfg.resume_grace).await;
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

    /// Bytes from the client socket: frame them, enforce policy, forward to LMS.
    pub async fn client_bytes(&self, data: &[u8]) -> ClientInput {
        let frames = {
            let mut st = self.state.lock().unwrap();
            st.from_client.push(data);
            let mut frames = Vec::new();
            loop {
                let (op, payload) = match st.from_client.next_frame() {
                    Ok(Some(f)) => f,
                    Ok(None) => break,
                    Err(e) => return ClientInput::Policy(e.to_string()),
                };
                if &op == b"HELO" {
                    if st.seen_helo {
                        return ClientInput::Policy("repeated HELO".into());
                    }
                    match Helo::parse(&payload) {
                        Ok(h) if h.mac == self.device.mac => st.seen_helo = true,
                        Ok(_) => {
                            return ClientInput::Policy("HELO MAC does not match device".into());
                        }
                        Err(_) => return ClientInput::Policy("malformed HELO".into()),
                    }
                } else if !st.seen_helo {
                    return ClientInput::Policy("first frame must be HELO".into());
                } else if &op == b"STAT"
                    && let Ok(stat) = Stat::parse(&payload)
                {
                    match &stat.event {
                        b"STMs" | b"STMr" => st.playing = true,
                        b"STMp" | b"STMf" | b"STMu" | b"STMn" => st.playing = false,
                        _ => {}
                    }
                    st.last_stat = Some((stat, Instant::now()));
                }
                let mut frame = Vec::with_capacity(8 + payload.len());
                frame.extend_from_slice(&op);
                frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                frame.extend_from_slice(&payload);
                frames.push(frame);
            }
            if st.from_client.buffered() > ClientFrameReader::MAX_FRAME + 8 {
                return ClientInput::Policy("oversize frame".into());
            }
            frames
        };
        let mut lms = self.lms_tx.lock().await;
        for f in frames {
            if lms.write_all(&f).await.is_err() {
                return ClientInput::LmsGone;
            }
        }
        ClientInput::Ok
    }

    /// The current stream number and validated path (if allowed).
    pub fn stream(&self) -> (u64, Option<String>) {
        let st = self.state.lock().unwrap();
        (st.stream_seq, st.stream_path.clone())
    }

    /// The client acknowledges `bytes` of stream `n`. Acks for other streams are ignored.
    pub async fn ack(&self, n: u64, bytes: u64) {
        let relay = {
            let st = self.state.lock().unwrap();
            if st.stream_seq != n {
                return;
            }
            st.relay.clone()
        };
        if let Some(relay) = relay {
            relay.lock().await.ack(bytes);
        }
    }

    /// The relay for stream `n`, created on first use.
    pub fn relay(&self, n: u64, window: usize) -> Option<Arc<tokio::sync::Mutex<Relay>>> {
        let mut st = self.state.lock().unwrap();
        if st.ended || n != st.stream_seq || st.stream_path.is_none() {
            return None;
        }
        Some(
            st.relay
                .get_or_insert_with(|| Arc::new(tokio::sync::Mutex::new(Relay::new(window))))
                .clone(),
        )
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
    /// clock (and, if it was playing, the elapsed time) advanced by wall time.
    fn synthesised_stat(&self, server_timestamp: u32) -> Option<Vec<u8>> {
        let st = self.state.lock().unwrap();
        if !st.seen_helo {
            return None;
        }
        let (mut stat, at) = st.last_stat.clone()?;
        let dt = at.elapsed().as_millis() as u64;
        stat.event = *b"STMt";
        stat.jiffies = stat.jiffies.wrapping_add(dt as u32);
        if st.playing {
            let ms = stat.elapsed_milliseconds as u64 + dt;
            stat.elapsed_milliseconds = ms as u32;
            stat.elapsed_seconds = (ms / 1000) as u32;
        }
        stat.server_timestamp = server_timestamp;
        Some(ClientMsg::Stat(stat).encode())
    }

    async fn heartbeat(&self, server_timestamp: u32) {
        if let Some(frame) = self.synthesised_stat(server_timestamp) {
            let _ = self.lms_tx.lock().await.write_all(&frame).await;
        }
    }

    /// End the session: close LMS's connection, tell any attached client why.
    pub async fn end(&self, gw: &Gateway, reason: EndReason) {
        let attached = {
            let mut st = self.state.lock().unwrap();
            if st.ended {
                return;
            }
            st.ended = true;
            st.log.clear();
            st.log_bytes = 0;
            st.relay = None;
            st.attached.take()
        };
        gw.remove_session(&self.id);
        if let Some(a) = attached {
            let msg = match reason {
                EndReason::LmsClosed => Outbound::Close(1001, "server closed connection"),
                EndReason::Replaced => Outbound::Close(4000, "replaced by a newer connection"),
                EndReason::GraceExpired => Outbound::Close(1001, "session expired"),
                EndReason::QueueOverflow => Outbound::Close(1013, "client too slow"),
                EndReason::Policy => Outbound::Close(1008, "policy violation"),
            };
            let _ = a.tx.try_send(msg);
        }
        let _ = self.lms_tx.lock().await.shutdown().await;
        info!(mac = %self.device.mac_string(), ?reason, "session ended");
    }

    /// Log one LMS frame and send it to the attached socket, if any.
    /// Returns false if the session must end (too much undelivered while detached).
    fn deliver(&self, frame: Vec<u8>, limit: usize) -> bool {
        let mut st = self.state.lock().unwrap();
        if st.ended {
            return true;
        }
        st.last_seq += 1;
        let seq = st.last_seq;
        st.log_bytes += frame.len();
        st.log.push_back((seq, frame.clone()));
        if let Some(a) = &st.attached {
            match a.tx.try_send(Outbound::Frame(frame)) {
                Ok(()) => st.sent_upto = seq,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Client can't keep up: drop the socket; it may resume and catch up.
                    warn!(mac = %self.device.mac_string(), "client too slow; detaching socket");
                    let _ = a.tx.try_send(Outbound::Close(1013, "client too slow"));
                    st.attached = None;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => st.attached = None,
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
        st.log_bytes <= limit
    }

    /// Record the stream a `strm s` asks the player to fetch, if policy allows it.
    fn note_strm(&self, gw: &Gateway, payload: &[u8]) {
        let Ok(s) = Strm::parse(payload) else { return };
        if s.command != b's' {
            return;
        }
        let ip = Ipv4Addr::from(s.server_ip);
        let path = if ip.is_unspecified() || gw.cfg.lms_ips.contains(&ip) {
            match validate_stream_request(&s.request, &self.device.mac) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(mac = %self.device.mac_string(), error = %e, "refused stream request");
                    None
                }
            }
        } else {
            warn!(mac = %self.device.mac_string(), %ip, "refused direct stream to non-LMS address");
            None
        };
        let mut st = self.state.lock().unwrap();
        st.stream_seq += 1;
        st.stream_path = path;
        st.relay = None; // the previous stream's upstream is abandoned
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
            let (op, payload) = match reader.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "bad framing from LMS");
                    overflow = true; // treat as fatal for the session
                    break;
                }
            };
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
                    session.note_strm(&gw, &payload);
                }
                _ => {}
            }
            if !session.deliver(
                encode_server_frame(op, &payload),
                gw.cfg.detached_queue_limit,
            ) {
                warn!(mac = %session.device.mac_string(), "detached queue overflow");
                session.end(&gw, EndReason::QueueOverflow).await;
                return;
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
