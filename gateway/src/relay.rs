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
//! [`Config::replay_window`]: crate::Config::replay_window

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::AppState;

const CHUNK: usize = 64 * 1024;

pub struct Relay {
    upstream: Option<BoxStream<'static, Result<Bytes, reqwest::Error>>>,
    content_type: String,
    /// Total bytes read from LMS so far.
    received: u64,
    /// The most recent bytes, ending at `received`.
    window: VecDeque<u8>,
    window_cap: usize,
    done: bool,
    /// Incremented per client request; an older request stops when superseded.
    consumer: u64,
    /// Bytes the client has confirmed receiving.
    acked: u64,
    ack_changed: Arc<tokio::sync::Notify>,
}

impl Relay {
    pub fn new(window_cap: usize) -> Self {
        Relay {
            upstream: None,
            content_type: "application/octet-stream".into(),
            received: 0,
            window: VecDeque::new(),
            window_cap,
            done: false,
            consumer: 0,
            acked: 0,
            ack_changed: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Record the client's acknowledgement (clamped to what was actually sent).
    pub fn ack(&mut self, bytes: u64) {
        let bytes = bytes.min(self.received);
        if bytes > self.acked {
            self.acked = bytes;
            self.ack_changed.notify_waiters();
        }
    }

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

#[derive(Deserialize)]
pub struct StreamQuery {
    offset: Option<u64>,
}

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
    let Some(device) = token.and_then(|t| gw.devices.authenticate(t)) else {
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
        .filter(|s| Arc::ptr_eq(&s.device, &device))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(n) = n.parse::<u64>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (_, path) = sess.stream();
    let (Some(path), Some(relay)) = (path, sess.relay(n, gw.cfg.replay_window)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let offset = q.offset.unwrap_or(0);

    let (my_turn, content_type) = {
        let mut r = relay.lock().await;
        if r.upstream.is_none() && !r.done {
            if offset != 0 {
                return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            }
            let url = format!("http://{}{}", gw.cfg.lms_http, path);
            match gw.http.get(&url).send().await {
                Ok(resp) if resp.status() == reqwest::StatusCode::OK => {
                    r.content_type = resp
                        .headers()
                        .get(header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .filter(|ct| ct.starts_with("audio/") || *ct == "application/octet-stream")
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    r.upstream = Some(resp.bytes_stream().boxed());
                }
                Ok(resp) => {
                    warn!(status = %resp.status(), "LMS stream status");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                Err(e) => {
                    warn!(error = %e, "LMS stream fetch failed");
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        }
        if offset > r.received {
            return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
        }
        if offset < r.window_start() {
            // Too far back to replay: the client must give up on this stream.
            return StatusCode::GONE.into_response();
        }
        r.consumer += 1;
        debug!(
            offset,
            received = r.received,
            consumer = r.consumer,
            "stream request"
        );
        (r.consumer, r.content_type.clone())
    };

    let body = futures_util::stream::unfold((relay, offset), move |(relay, pos)| async move {
        let mut r = relay.lock().await;
        if r.consumer != my_turn {
            debug!(pos, "stream request superseded");
            return None; // superseded by a newer request for this stream
        }
        if pos < r.received {
            let chunk = r.replay(pos, CHUNK);
            let next = pos + chunk.len() as u64;
            drop(r);
            return Some((Ok::<Bytes, std::io::Error>(chunk), (relay, next)));
        }
        if r.done {
            return None;
        }
        if r.too_far_ahead() {
            // Wait for the client to acknowledge more before reading further from LMS.
            let changed = r.ack_changed.clone();
            let waiting = changed.notified();
            drop(r);
            let _ = tokio::time::timeout(Duration::from_millis(500), waiting).await;
            return Some((Ok(Bytes::new()), (relay, pos)));
        }
        let next = match r.upstream.as_mut() {
            Some(up) => up.next().await,
            None => None,
        };
        match next {
            Some(Ok(chunk)) => {
                r.remember(&chunk);
                let next = pos + chunk.len() as u64;
                drop(r);
                Some((Ok(chunk), (relay, next)))
            }
            Some(Err(e)) => {
                warn!(received = r.received, error = %e, "LMS stream failed");
                r.done = true;
                r.upstream = None;
                drop(r);
                Some((Err(std::io::Error::other(e)), (relay, pos)))
            }
            None => {
                info!(received = r.received, "LMS stream finished");
                r.done = true;
                r.upstream = None;
                None
            }
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(body))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acks_pace_reading_and_are_clamped() {
        let mut r = Relay::new(10);
        assert!(!r.too_far_ahead());
        r.remember(b"0123456789");
        assert!(r.too_far_ahead(), "a full window unacknowledged");
        r.ack(4);
        assert!(!r.too_far_ahead());
        r.ack(2); // acks never go backwards
        assert_eq!(r.acked, 4);
        r.ack(1_000_000); // can't acknowledge more than was sent
        assert_eq!(r.acked, 10);
    }

    #[test]
    fn window_keeps_only_the_most_recent_bytes() {
        let mut r = Relay::new(10);
        r.remember(b"0123456789");
        r.remember(b"abcde");
        assert_eq!(r.received, 15);
        assert_eq!(r.window_start(), 5);
        assert_eq!(&r.replay(5, 100)[..], b"56789abcde");
        assert_eq!(&r.replay(12, 2)[..], b"cd");
    }
}
