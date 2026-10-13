//! `/v1/stream/<session>/<n>`: resumable relay of one LMS audio stream.
//!
//! The upstream request to LMS belongs to the session, not to the client's HTTP request, so
//! a client whose connection drops can fetch again with `?offset=` and continue. While no
//! client is reading, the gateway stops reading upstream; TCP back-pressure holds LMS, so
//! nothing is lost and nothing large is buffered. The last [`Config::replay_window`] bytes
//! are kept to cover data that was in flight when the connection broke.
//!
//! The client acknowledges what it has actually received (a `{"ack":…}` message on the
//! player socket), and the gateway never reads more than one replay window beyond the last
//! acknowledgement. Network buffers between the two can't then hold more than the window,
//! so a broken download can always resume.
//!
//! Locking: the relay's bookkeeping sits behind a plain mutex that is never held across an
//! `.await`, so acknowledgements (which arrive on the player socket's read loop) never wait
//! for network I/O. Only the upstream reader has its own async lock.
//!
//! A relay stops promptly when its session ends ([`Relay::cancel`]), when an upstream read
//! stalls for [`UPSTREAM_READ_TIMEOUT`], and when the client neither acknowledges nor can
//! take more data for [`IDLE_TIMEOUT`]. A response ends cleanly only when LMS finished the
//! stream cleanly and the client has had all of it. Every other end (a stall, a failure, a
//! cancel, a newer request taking over) is an error, so the client can't mistake it for the
//! end of the track: it resumes from `?offset=` instead, or reports the stream as failed.
//!
//! FLAC streams have their embedded artwork and padding dropped on the way in
//! ([`crate::flac_strip`]), before anything is counted or remembered: every offset here
//! (`?offset=`, acknowledgements, the replay window) is in the stripped stream's terms, so
//! resuming works unchanged. LMS is asked for a stream only once, so the stripped stream is
//! the same for every request.
//!
//! [`Config::replay_window`]: crate::Config::replay_window

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::{Notify, watch};
use tracing::{debug, info, warn};

use crate::AppState;
use crate::flac_strip::{StripInfo, StripStream};

const CHUNK: usize = 64 * 1024;

/// Longest wait for LMS to answer the stream request or deliver the next chunk.
pub const UPSTREAM_READ_TIMEOUT: Duration = Duration::from_secs(60);
/// A response whose client has a full window unacknowledged ends after this long without
/// an acknowledgement; the client can fetch again with `?offset=`.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How often a paused response re-checks its state (a backstop for missed wake-ups).
const RECHECK: Duration = Duration::from_millis(500);

type Upstream = BoxStream<'static, Result<Bytes, std::io::Error>>;

struct UpstreamSlot {
    stream: Option<Upstream>,
    /// The request to LMS has been made (it is never made twice).
    opened: bool,
}

struct RelayState {
    content_type: String,
    /// Total bytes read from LMS so far.
    received: u64,
    /// The most recent bytes, ending at `received`.
    window: VecDeque<u8>,
    window_cap: usize,
    done: bool,
    /// LMS's stream ended with an error or stalled (not a clean end of the track).
    failed: bool,
    /// Incremented per client request; an older request stops when superseded.
    consumer: u64,
    /// Bytes the client has confirmed receiving.
    acked: u64,
}

pub struct Relay {
    state: Mutex<RelayState>,
    upstream: tokio::sync::Mutex<UpstreamSlot>,
    ack_changed: Notify,
    cancelled: watch::Sender<bool>,
    idle_timeout: Duration,
    read_timeout: Duration,
    /// Set for FLAC streams: PICTURE and PADDING blocks are dropped, and this says how
    /// many bytes that removed once the metadata has been seen.
    flac: Option<Arc<OnceLock<StripInfo>>>,
    /// Set for streams re-encoded to Opus ([`crate::live`]).
    live: Option<LiveSetup>,
}

/// A live encode for this stream: `ffmpeg`, the bit rate, the slot it holds (handed to
/// the encoder when the stream is opened) and its byte counts.
struct LiveSetup {
    ffmpeg: std::path::PathBuf,
    bitrate_k: u32,
    permit: Mutex<Option<tokio::sync::OwnedSemaphorePermit>>,
    counts: Arc<crate::live::Counts>,
}

impl RelayState {
    /// Reading further upstream would put more than a window of unacknowledged data in
    /// flight.
    fn too_far_ahead(&self) -> bool {
        self.received >= self.acked + self.window_cap as u64
    }

    fn window_start(&self) -> u64 {
        self.received - self.window.len() as u64
    }

    fn remember(&mut self, chunk: &[u8]) {
        self.received += chunk.len() as u64;
        self.window.extend(chunk);
        let excess = self.window.len().saturating_sub(self.window_cap);
        self.window.drain(..excess);
    }

    fn replay(&self, from: u64, max: usize) -> Bytes {
        let start = (from - self.window_start()) as usize;
        let end = (start + max).min(self.window.len());
        Bytes::from(self.window.range(start..end).copied().collect::<Vec<u8>>())
    }
}

impl Relay {
    pub fn new(window_cap: usize) -> Self {
        Self::with_timeouts(window_cap, IDLE_TIMEOUT, UPSTREAM_READ_TIMEOUT)
    }

    fn with_timeouts(window_cap: usize, idle_timeout: Duration, read_timeout: Duration) -> Self {
        Relay {
            state: Mutex::new(RelayState {
                content_type: "application/octet-stream".into(),
                received: 0,
                window: VecDeque::new(),
                window_cap,
                done: false,
                failed: false,
                consumer: 0,
                acked: 0,
            }),
            upstream: tokio::sync::Mutex::new(UpstreamSlot {
                stream: None,
                opened: false,
            }),
            ack_changed: Notify::new(),
            cancelled: watch::channel(false).0,
            idle_timeout,
            read_timeout,
            flac: None,
            live: None,
        }
    }

    /// Re-encode this (FLAC) stream to Ogg Opus at `bitrate_k` ([`crate::live`]), holding
    /// `permit` (a live-encode slot) while it runs.
    pub fn with_live(
        mut self,
        ffmpeg: &std::path::Path,
        bitrate_k: u32,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Self {
        self.live = Some(LiveSetup {
            ffmpeg: ffmpeg.to_path_buf(),
            bitrate_k,
            permit: Mutex::new(Some(permit)),
            counts: Arc::default(),
        });
        self
    }

    /// Drop embedded artwork and padding from the start of this (FLAC) stream.
    pub fn with_flac_strip(mut self) -> Self {
        self.flac = Some(Arc::new(OnceLock::new()));
        self
    }

    /// The byte count LMS sent for `client` bytes the client received: they differ by
    /// what stripping removed. Unchanged for other streams, and until the metadata has
    /// been dealt with.
    ///
    /// The client's count is first clamped to what this relay has produced: a player that
    /// played the rest of a track from its replay cache (rolling cache B2) reports the whole
    /// stream while the relay read only its start, and LMS must never hear of bytes it
    /// didn't send (spike S4).
    pub fn upstream_bytes(&self, client: u64) -> u64 {
        let client = client.min(self.state.lock().unwrap().received);
        if let Some(live) = &self.live {
            return live.counts.upstream_bytes(client);
        }
        match self.flac.as_ref().and_then(|f| f.get()) {
            Some(info) => info.upstream_bytes(client),
            None => client,
        }
    }

    /// Record the client's acknowledgement (clamped to what was actually sent). Never
    /// waits: safe to call from the player socket's read loop.
    pub fn ack(&self, bytes: u64) {
        let mut st = self.state.lock().unwrap();
        let bytes = bytes.min(st.received);
        if bytes > st.acked {
            st.acked = bytes;
            drop(st);
            self.ack_changed.notify_waiters();
        }
    }

    /// Stop every response for this stream and drop the upstream connection.
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
        self.ack_changed.notify_waiters();
    }

    fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }
}

/// Resolves once the relay is cancelled.
async fn cancelled(rx: &mut watch::Receiver<bool>) {
    if rx.wait_for(|c| *c).await.is_err() {
        std::future::pending::<()>().await;
    }
}

#[derive(Deserialize)]
pub struct StreamQuery {
    offset: Option<u64>,
    /// Send at most this many bytes, then end the response cleanly (the player's replay
    /// cache asks for a stream's first bytes this way, to look them up before fetching
    /// the rest with `?offset=`). 1 to [`MAX_LIMIT`].
    limit: Option<u64>,
}

/// Largest `?limit=`.
pub const MAX_LIMIT: u64 = 16 * 1024 * 1024;

pub async fn stream(
    State(gw): State<AppState>,
    Path((session, n)): Path<(String, String)>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(device) = token.and_then(|t| gw.accounts.authenticate(t)) else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"lyrion-anywhere\"")],
            "unauthorised\n",
        )
            .into_response();
    };
    // Another device's session looks exactly like a missing one.
    let Some(sess) = gw
        .session(&session)
        .filter(|s| s.device.device_id == device.device_id)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(n) = n.parse::<u64>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if q.limit.is_some_and(|l| l == 0 || l > MAX_LIMIT) {
        return (StatusCode::BAD_REQUEST, "bad limit\n").into_response();
    }
    let (_, path) = sess.stream();
    let (Some(path), Some(relay)) = (
        path,
        sess.relay(
            n,
            gw.cfg.replay_window,
            gw.cfg.strip_flac_pictures,
            gw.live_ffmpeg(),
        ),
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let offset = q.offset.unwrap_or(0);

    {
        let mut up = relay.upstream.lock().await;
        if !up.opened {
            if offset != 0 {
                return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            }
            let url = format!("http://{}{}", gw.cfg.lms_http, path);
            let mut cancel = relay.cancelled.subscribe();
            let sent = tokio::select! {
                r = tokio::time::timeout(relay.read_timeout, gw.http.get(&url).send()) => r,
                _ = cancelled(&mut cancel) => return StatusCode::NOT_FOUND.into_response(),
            };
            match sent {
                Ok(Ok(resp)) if resp.status() == reqwest::StatusCode::OK => {
                    relay.state.lock().unwrap().content_type = resp
                        .headers()
                        .get(header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .filter(|ct| ct.starts_with("audio/") || *ct == "application/octet-stream")
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let body = resp
                        .bytes_stream()
                        .map(|r| r.map_err(std::io::Error::other));
                    up.stream = Some(match (&relay.live, &relay.flac) {
                        (Some(live), _) => {
                            // The slot is used once: an encoder that failed to start gave
                            // it back, and a retry must not run one outside the limit.
                            let Some(permit) = live.permit.lock().unwrap().take() else {
                                warn!("live encoder already tried for this stream");
                                return StatusCode::BAD_GATEWAY.into_response();
                            };
                            match crate::live::encode(
                                &live.ffmpeg,
                                live.bitrate_k,
                                body.boxed(),
                                Some(permit),
                                live.counts.clone(),
                            ) {
                                Ok(s) => {
                                    relay.state.lock().unwrap().content_type = "audio/ogg".into();
                                    s
                                }
                                Err(e) => {
                                    // The player was told Opus: it can't take the original.
                                    warn!(error = %e, "cannot start the live encoder");
                                    return StatusCode::BAD_GATEWAY.into_response();
                                }
                            }
                        }
                        (None, Some(info)) => StripStream::new(body.boxed(), info.clone()).boxed(),
                        (None, None) => body.boxed(),
                    });
                    up.opened = true;
                }
                Ok(Ok(resp)) => {
                    warn!(status = %resp.status(), "LMS stream status");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "LMS stream fetch failed");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                Err(_) => {
                    warn!("LMS stream request timed out");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        }
    }

    let (my_turn, content_type) = {
        let mut st = relay.state.lock().unwrap();
        if offset > st.received {
            return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
        }
        if offset < st.window_start() {
            // Too far back to replay: the client must give up on this stream.
            return StatusCode::GONE.into_response();
        }
        st.consumer += 1;
        debug!(
            offset,
            received = st.received,
            consumer = st.consumer,
            "stream request"
        );
        (st.consumer, st.content_type.clone())
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(body(
            relay,
            offset,
            my_turn,
            q.limit.map(|l| offset.saturating_add(l)),
        )))
        .unwrap()
}

struct Cursor {
    relay: Arc<Relay>,
    pos: u64,
    my_turn: u64,
    cancel: watch::Receiver<bool>,
    /// When the client last had a full window unacknowledged with no progress.
    stalled_since: Option<Instant>,
    last_acked: u64,
    /// The response ends cleanly here (`?limit=`).
    end: Option<u64>,
}

impl Cursor {
    /// How much this response may still send.
    fn room(&self) -> usize {
        self.end.map_or(CHUNK, |e| {
            e.saturating_sub(self.pos).min(CHUNK as u64) as usize
        })
    }
}

enum Step {
    Yield(Bytes),
    /// The track's audio is complete: end the response cleanly.
    End,
    /// Stop without the rest of the audio: end the response with an error.
    Fail(&'static str),
    Wait,
    Read,
}

fn io_error(why: &'static str) -> std::io::Error {
    std::io::Error::other(why)
}

/// The response body for one client request, starting at `offset`.
fn body(
    relay: Arc<Relay>,
    offset: u64,
    my_turn: u64,
    end: Option<u64>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let cancel = relay.cancelled.subscribe();
    let cur = Cursor {
        relay,
        pos: offset,
        my_turn,
        cancel,
        stalled_since: None,
        last_acked: 0,
        end,
    };
    futures_util::stream::unfold(cur, |mut cur| async move {
        next_chunk(&mut cur).await.map(|item| (item, cur))
    })
}

/// What the current request can do now, decided under the state lock.
fn step(cur: &mut Cursor) -> Step {
    let st = cur.relay.state.lock().unwrap();
    if st.consumer != cur.my_turn {
        debug!(pos = cur.pos, "stream request superseded");
        return Step::Fail("superseded by a newer request");
    }
    if cur.room() == 0 {
        // As much as was asked for: the client fetches the rest with `?offset=`.
        return Step::End;
    }
    if cur.pos < st.received {
        if cur.pos < st.window_start() {
            // Another request read on past this one's replay window.
            return Step::Fail("past the replay window");
        }
        let chunk = st.replay(cur.pos, cur.room());
        cur.pos += chunk.len() as u64;
        cur.stalled_since = None;
        return Step::Yield(chunk);
    }
    if st.done {
        // A resumed request after LMS failed must not look like the end of the track.
        return if st.failed {
            Step::Fail("the music server's stream failed")
        } else {
            Step::End
        };
    }
    if st.too_far_ahead() {
        if st.acked != cur.last_acked {
            cur.last_acked = st.acked;
            cur.stalled_since = None;
        }
        return Step::Wait;
    }
    Step::Read
}

async fn next_chunk(cur: &mut Cursor) -> Option<Result<Bytes, std::io::Error>> {
    loop {
        if cur.relay.is_cancelled() {
            debug!(pos = cur.pos, "stream cancelled");
            return Some(Err(io_error("stream cancelled")));
        }
        match step(cur) {
            Step::Yield(chunk) => return Some(Ok(chunk)),
            Step::End => return None,
            Step::Fail(why) => return Some(Err(io_error(why))),
            Step::Wait => {
                // Wait for the client to acknowledge more before reading further from LMS.
                let since = *cur.stalled_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= cur.relay.idle_timeout {
                    // End with an error, never cleanly: a clean end looks like the end of
                    // the track, and the player would cut it short. An error makes it
                    // fetch again from `?offset=` when it next wants data.
                    warn!(pos = cur.pos, "stream idle without acknowledgement; ending");
                    return Some(Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "client idle without acknowledgement",
                    )));
                }
                let relay = cur.relay.clone();
                let changed = relay.ack_changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                // An ack may have landed before the waiter was registered.
                match step(cur) {
                    Step::Wait => {}
                    Step::Yield(chunk) => return Some(Ok(chunk)),
                    Step::End => return None,
                    Step::Fail(why) => return Some(Err(io_error(why))),
                    Step::Read => continue,
                }
                tokio::select! {
                    _ = changed => {}
                    _ = tokio::time::sleep(RECHECK) => {}
                    _ = cancelled(&mut cur.cancel) => {}
                }
                // Hand control back to the HTTP layer so a vanished client is noticed.
                return Some(Ok(Bytes::new()));
            }
            Step::Read => {
                if let Some(item) = read_upstream(cur).await {
                    return item;
                }
            }
        }
    }
}

/// Read the next chunk from LMS. `None` means "look again" (another request read first,
/// or the state changed); `Some(None)` ends the body cleanly (LMS finished).
async fn read_upstream(cur: &mut Cursor) -> Option<Option<Result<Bytes, std::io::Error>>> {
    let relay = cur.relay.clone();
    let mut up = tokio::select! {
        g = relay.upstream.lock() => g,
        _ = cancelled(&mut cur.cancel) => return None,
    };
    {
        let st = relay.state.lock().unwrap();
        // Superseded, finished, or moved on while waiting for the reader: `step` decides.
        if st.consumer != cur.my_turn || st.done || cur.pos != st.received || st.too_far_ahead() {
            return None;
        }
    }
    let Some(stream) = up.stream.as_mut() else {
        return Some(Some(Err(io_error("no stream from the music server"))));
    };
    let next = tokio::select! {
        r = tokio::time::timeout(relay.read_timeout, stream.next()) => r,
        _ = cancelled(&mut cur.cancel) => return None,
    };
    let mut st = relay.state.lock().unwrap();
    match next {
        Ok(Some(Ok(chunk))) => {
            st.remember(&chunk);
            // Within the limit; the rest stays in the window for the next request.
            let chunk = chunk.slice(..chunk.len().min(cur.room()));
            cur.pos += chunk.len() as u64;
            cur.stalled_since = None;
            Some(Some(Ok(chunk)))
        }
        Ok(Some(Err(e))) => {
            warn!(received = st.received, error = %e, "LMS stream failed");
            st.done = true;
            st.failed = true;
            up.stream = None;
            Some(Some(Err(std::io::Error::other(e))))
        }
        Ok(None) => {
            info!(received = st.received, "LMS stream finished");
            st.done = true;
            up.stream = None;
            Some(None)
        }
        Err(_) => {
            warn!(received = st.received, "LMS stream stalled; giving up");
            st.done = true;
            st.failed = true;
            up.stream = None;
            Some(Some(Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "upstream read timed out",
            ))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(r: &Relay) -> std::sync::MutexGuard<'_, RelayState> {
        r.state.lock().unwrap()
    }

    #[test]
    fn acks_pace_reading_and_are_clamped() {
        let r = Relay::new(10);
        assert!(!state(&r).too_far_ahead());
        state(&r).remember(b"0123456789");
        assert!(state(&r).too_far_ahead(), "a full window unacknowledged");
        r.ack(4);
        assert!(!state(&r).too_far_ahead());
        r.ack(2); // acks never go backwards
        assert_eq!(state(&r).acked, 4);
        r.ack(1_000_000); // can't acknowledge more than was sent
        assert_eq!(state(&r).acked, 10);
    }

    #[test]
    fn window_keeps_only_the_most_recent_bytes() {
        let r = Relay::new(10);
        let mut st = state(&r);
        st.remember(b"0123456789");
        st.remember(b"abcde");
        assert_eq!(st.received, 15);
        assert_eq!(st.window_start(), 5);
        assert_eq!(&st.replay(5, 100)[..], b"56789abcde");
        assert_eq!(&st.replay(12, 2)[..], b"cd");
    }

    type Feed = tokio::sync::mpsc::UnboundedSender<Result<Bytes, std::io::Error>>;

    /// A relay whose upstream is fed by hand, and one consumer's body.
    async fn rig(
        window: usize,
        idle: Duration,
        read: Duration,
    ) -> (
        Arc<Relay>,
        Feed,
        std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) {
        let relay = Arc::new(Relay::with_timeouts(window, idle, read));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let up =
            futures_util::stream::unfold(
                rx,
                |mut rx| async move { rx.recv().await.map(|x| (x, rx)) },
            );
        {
            let mut slot = relay.upstream.lock().await;
            slot.stream = Some(up.boxed());
            slot.opened = true;
        }
        state(&relay).consumer = 1;
        let b = Box::pin(body(relay.clone(), 0, 1, None));
        (relay, tx, b)
    }

    /// Drain the body until it ends; returns (bytes, ended with an error).
    async fn drain(
        b: &mut std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> (usize, bool) {
        let mut n = 0;
        while let Some(item) = b.next().await {
            match item {
                Ok(c) => n += c.len(),
                Err(_) => return (n, true),
            }
        }
        (n, false)
    }

    #[tokio::test]
    async fn cancel_ends_a_body_waiting_on_upstream() {
        let (relay, _feed, mut b) = rig(1024, IDLE_TIMEOUT, UPSTREAM_READ_TIMEOUT).await;
        let task = tokio::spawn(async move { drain(&mut b).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        relay.cancel();
        let r = tokio::time::timeout(Duration::from_secs(2), task).await;
        assert!(r.is_ok(), "body must end promptly once cancelled");
    }

    #[tokio::test]
    async fn cancel_ends_a_body_waiting_for_acks() {
        let (relay, feed, mut b) = rig(4, IDLE_TIMEOUT, UPSTREAM_READ_TIMEOUT).await;
        feed.send(Ok(Bytes::from_static(b"abcd"))).unwrap();
        feed.send(Ok(Bytes::from_static(b"efgh"))).unwrap();
        let task = tokio::spawn(async move { drain(&mut b).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        relay.cancel();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("body must end promptly once cancelled")
            .unwrap();
        assert_eq!(n, 4, "nothing beyond the window without an ack");
    }

    #[tokio::test]
    async fn body_ends_with_an_error_when_the_client_stops_acknowledging() {
        // Regression: this used to end cleanly, which the player took for the end of the
        // track, cutting it short. An error makes it resume from `?offset=` instead.
        let (relay, feed, mut b) = rig(4, Duration::from_millis(200), UPSTREAM_READ_TIMEOUT).await;
        feed.send(Ok(Bytes::from_static(b"abcd"))).unwrap();
        feed.send(Ok(Bytes::from_static(b"efgh"))).unwrap();
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut b))
            .await
            .expect("an idle body must end");
        assert_eq!((n, err), (4, true));
        // The client resumes where it was: the rest arrives once it acknowledges.
        state(&relay).consumer = 2;
        let mut again = Box::pin(body(relay.clone(), 4, 2, None));
        relay.ack(4);
        feed.send(Ok(Bytes::from_static(b"ijkl"))).unwrap();
        drop(feed);
        let first = tokio::time::timeout(Duration::from_secs(3), again.next())
            .await
            .expect("the resumed body must deliver")
            .expect("not ended")
            .expect("not an error");
        assert_eq!(&first[..], b"efgh");
    }

    type BoxBody = std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

    fn boxed_body(relay: Arc<Relay>, offset: u64, turn: u64) -> BoxBody {
        Box::pin(body(relay, offset, turn, None))
    }

    /// Regression: after LMS's stream failed, a resumed request was served the replay and
    /// then ended cleanly, so the player took the cut-off track for a finished one.
    #[tokio::test]
    async fn a_resume_after_lms_failed_ends_with_an_error() {
        let (relay, feed, mut b) = rig(1024, IDLE_TIMEOUT, UPSTREAM_READ_TIMEOUT).await;
        feed.send(Ok(Bytes::from_static(b"abcd"))).unwrap();
        drop(feed); // LMS's stream ends: here, cleanly
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut b))
            .await
            .unwrap();
        assert_eq!((n, err), (4, false), "a clean end from LMS stays clean");
        // The same stream, but LMS's side broke: every later request fails too.
        state(&relay).failed = true;
        state(&relay).consumer = 2;
        let mut again = boxed_body(relay.clone(), 2, 2);
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut again))
            .await
            .unwrap();
        assert_eq!((n, err), (2, true), "the replay, then an error");
    }

    #[tokio::test]
    async fn a_stalled_upstream_fails_the_body_and_later_requests() {
        // (A stall and an LMS error take the same path; a reqwest error can't be made by hand.)
        let (relay, feed, mut b) = rig(1024, IDLE_TIMEOUT, Duration::from_millis(200)).await;
        feed.send(Ok(Bytes::from_static(b"efgh"))).unwrap();
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut b))
            .await
            .unwrap();
        assert_eq!((n, err), (4, true), "the stall fails the reading request");
        state(&relay).consumer = 2;
        let mut again = boxed_body(relay.clone(), 4, 2);
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut again))
            .await
            .unwrap();
        assert_eq!((n, err), (0, true), "and a resume after it fails as well");
        drop(feed);
    }

    #[tokio::test]
    async fn cancelled_and_superseded_bodies_end_with_an_error() {
        let (relay, feed, mut b) = rig(1024, IDLE_TIMEOUT, UPSTREAM_READ_TIMEOUT).await;
        feed.send(Ok(Bytes::from_static(b"abcd"))).unwrap();
        let first = b.next().await.unwrap().unwrap();
        assert_eq!(&first[..], b"abcd");
        // A newer request takes over: the old body must not end as if complete.
        state(&relay).consumer = 2;
        let (_, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut b))
            .await
            .unwrap();
        assert!(err, "superseded ends with an error");
        let mut newer = boxed_body(relay.clone(), 4, 2);
        let task = tokio::spawn(async move { drain(&mut newer).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        relay.cancel();
        let (_, err) = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        assert!(err, "cancelled ends with an error");
        drop(feed);
    }

    #[tokio::test]
    async fn acks_keep_an_idle_body_alive() {
        let (relay, feed, mut b) = rig(4, Duration::from_millis(300), UPSTREAM_READ_TIMEOUT).await;
        for c in [b"abcd", b"efgh", b"ijkl"] {
            feed.send(Ok(Bytes::from_static(c))).unwrap();
        }
        drop(feed);
        let r2 = relay.clone();
        tokio::spawn(async move {
            for acked in [4u64, 8, 12] {
                tokio::time::sleep(Duration::from_millis(200)).await;
                r2.ack(acked);
            }
        });
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), drain(&mut b))
            .await
            .unwrap();
        assert_eq!((n, err), (12, false));
    }

    #[tokio::test]
    async fn stalled_upstream_read_times_out_and_never_blocks_acks() {
        let (relay, feed, mut b) = rig(1024, IDLE_TIMEOUT, Duration::from_millis(300)).await;
        feed.send(Ok(Bytes::from_static(b"abcd"))).unwrap();
        let task = tokio::spawn(async move { drain(&mut b).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // The body is now parked on the upstream read; an ack must not wait for it.
        let t = Instant::now();
        relay.ack(4);
        assert!(t.elapsed() < Duration::from_millis(50));
        let (n, err) = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("stalled upstream must time out")
            .unwrap();
        assert_eq!((n, err), (4, true));
        drop(feed);
    }
}
