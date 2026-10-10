//! `WS /v1/events`: LMS notifications for the caller's permitted players.
//!
//! The gateway keeps one connection to the LMS CLI (`listen 1`) while anyone is subscribed,
//! parses each notification, keeps only a fixed set of event types, and redacts anything
//! that looks like a URL or path. Each subscriber gets only events for players it may
//! control, plus server-wide rescan notices. Clients use events as hints to re-query state
//! through `/v1/rpc`.
//!
//! Protocol: the client's first message is its access token (text, within the auth
//! timeout). The gateway then sends text messages `{"player":"<mac>","event":[...]}`, or
//! `{"player":null,...}` for server events, and `{"resync":true}` if the client fell behind.
//! A socket whose device is revoked is closed (1008).

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::rpc_policy::is_player_id;
use crate::{AppState, origin_allowed};

/// One parsed LMS notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub player: Option<String>,
    pub tokens: Vec<String>,
}

const PLAYER_EVENTS: &[&str] = &[
    "playlist",
    "mixer",
    "power",
    "client",
    "sync",
    "pause",
    "play",
    "stop",
    "time",
    "newmetadata",
];
const SERVER_EVENTS: &[&str] = &["rescan"];
const MAX_LINE: usize = 16 * 1024;
const RECHECK: Duration = Duration::from_secs(30);

fn unescape(t: &str) -> String {
    let b = t.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(Ok(v)) = b
                .get(i + 1..i + 3)
                .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap_or("zz"), 16))
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Anything that could reveal a file path or a stream URL becomes empty.
fn redact(t: String) -> String {
    let lower = t.to_ascii_lowercase();
    if lower.contains("://")
        || lower.starts_with('/')
        || lower.starts_with("file:")
        || lower.contains(":\\")
    {
        String::new()
    } else {
        t.chars().filter(|c| !c.is_control()).take(512).collect()
    }
}

/// Parse a CLI notification line into an event we're willing to forward.
pub fn parse_line(line: &str) -> Option<Event> {
    let mut tokens = line.trim_end().split(' ').map(unescape);
    let first = tokens.next()?;
    let (player, name) = if is_player_id(&first.to_ascii_lowercase()) {
        (Some(first.to_ascii_lowercase()), tokens.next()?)
    } else {
        (None, first)
    };
    let allowed = match &player {
        Some(_) => PLAYER_EVENTS.contains(&name.as_str()),
        None => SERVER_EVENTS.contains(&name.as_str()),
    };
    if !allowed {
        return None;
    }
    let mut out = vec![name];
    out.extend(tokens.take(16).map(redact));
    Some(Event {
        player,
        tokens: out,
    })
}

/// The shared LMS CLI listener, started when the first client subscribes.
pub struct Hub {
    tx: broadcast::Sender<Event>,
    running: std::sync::Mutex<bool>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub {
            tx: broadcast::channel(256).0,
            running: std::sync::Mutex::new(false),
        }
    }
}

impl Hub {
    fn subscribe(self: &Arc<Self>, cli_addr: String) -> broadcast::Receiver<Event> {
        // Subscribing and the listener's idle check happen under the same lock, so a new
        // subscriber is never left without a listener.
        let mut running = self.running.lock().unwrap();
        let rx = self.tx.subscribe();
        if !*running {
            *running = true;
            let hub = self.clone();
            tokio::spawn(async move { hub.listen(cli_addr).await });
        }
        rx
    }

    /// Called by the listener: stop (and say so) if nobody is subscribed.
    fn stop_if_idle(&self) -> bool {
        let mut running = self.running.lock().unwrap();
        if self.tx.receiver_count() == 0 {
            *running = false;
            true
        } else {
            false
        }
    }

    async fn listen(self: Arc<Self>, addr: String) {
        let mut backoff = Duration::from_millis(500);
        loop {
            match self.listen_once(&addr).await {
                Ok(()) => info!("LMS CLI connection closed"),
                Err(e) => warn!(error = %e, "LMS CLI connection failed"),
            }
            if self.stop_if_idle() {
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }

    async fn listen_once(&self, addr: &str) -> std::io::Result<()> {
        let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timeout"))??;
        let (rd, mut wr) = stream.into_split();
        wr.write_all(b"listen 1\n").await?;
        let mut lines = BufReader::new(rd);
        let mut line = Vec::new();
        loop {
            let Some(complete) = read_line_capped(&mut lines, &mut line, MAX_LINE).await? else {
                return Ok(());
            };
            if self.tx.receiver_count() == 0 {
                return Ok(()); // nobody listening: drop the connection
            }
            if !complete {
                warn!("dropped oversize LMS CLI line");
                continue;
            }
            if let Some(ev) = parse_line(&String::from_utf8_lossy(&line)) {
                let _ = self.tx.send(ev);
            }
        }
    }
}

/// Read one line of at most `max` bytes into `buf`. Returns `None` at end of input,
/// `Some(true)` for a whole line and `Some(false)` for an oversize one, whose remainder
/// (up to and including its newline) is discarded so it can't be mistaken for a new line.
async fn read_line_capped<R: AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Option<bool>> {
    buf.clear();
    let n = (&mut *r).take(max as u64).read_until(b'\n', buf).await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.ends_with(b"\n") || n < max {
        return Ok(Some(true));
    }
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            break;
        }
        match available.iter().position(|b| *b == b'\n') {
            Some(i) => {
                r.consume(i + 1);
                break;
            }
            None => {
                let len = available.len();
                r.consume(len);
            }
        }
    }
    Ok(Some(false))
}

use tokio::io::AsyncReadExt as _;

pub async fn events_upgrade(
    State(gw): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&gw.cfg, &headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed\n").into_response();
    }
    ws.max_message_size(1024)
        .on_upgrade(move |socket| handle(gw, socket))
}

async fn close(mut ws: WebSocket, code: u16, reason: &str) {
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn handle(gw: AppState, mut ws: WebSocket) {
    let first = tokio::time::timeout(gw.cfg.auth_timeout, ws.recv()).await;
    let principal = match first {
        Ok(Some(Ok(Message::Text(t)))) if t.len() <= 512 => gw.accounts().authenticate(t.as_str()),
        _ => None,
    };
    let Some(principal) = principal else {
        return close(ws, close_code::POLICY, "unauthorised").await;
    };
    let Ok(mut scope) = gw.accounts().player_scope(&principal) else {
        return close(ws, close_code::ERROR, "internal error").await;
    };
    let mut rx = gw.events.subscribe(gw.cfg.lms_cli.clone());
    let _ = ws
        .send(Message::Text(
            json!({ "subscribed": true }).to_string().into(),
        ))
        .await;
    let mut recheck = tokio::time::interval(RECHECK);
    recheck.tick().await;
    let mut keepalive = tokio::time::interval(gw.cfg.ws_keepalive);
    keepalive.tick().await;
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let wanted = match &ev.player {
                        Some(p) => scope.allows(p),
                        None => true,
                    };
                    if wanted {
                        let msg = json!({ "player": ev.player, "event": ev.tokens }).to_string();
                        if ws.send(Message::Text(msg.into())).await.is_err() {
                            return;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if ws.send(Message::Text(json!({ "resync": true }).to_string().into())).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            msg = ws.recv() => match msg {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => return close(ws, close_code::POLICY, "unexpected message").await,
            },
            _ = keepalive.tick() => {
                if ws.send(Message::Ping(Default::default())).await.is_err() {
                    return;
                }
            }
            _ = recheck.tick() => {
                // Revocation and permission changes take effect without reconnecting.
                if !gw.accounts().device_active(principal.device_id) {
                    return close(ws, close_code::POLICY, "access revoked").await;
                }
                if let Ok(s) = gw.accounts().player_scope(&principal) {
                    scope = s;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_events_are_parsed_and_unescaped() {
        let e =
            parse_line("aa%3Abb%3Acc%3Add%3Aee%3A01 playlist newsong Song%20Title 3\n").unwrap();
        assert_eq!(e.player.as_deref(), Some("aa:bb:cc:dd:ee:01"));
        assert_eq!(e.tokens, vec!["playlist", "newsong", "Song Title", "3"]);
        let e = parse_line("aa%3Abb%3Acc%3Add%3Aee%3A01 mixer volume 40").unwrap();
        assert_eq!(e.tokens, vec!["mixer", "volume", "40"]);
    }

    #[test]
    fn paths_and_urls_are_redacted() {
        let e = parse_line(
            "aa%3Abb%3Acc%3Add%3Aee%3A01 playlist open file%3A%2F%2F%2Fsrv%2Fmusic%2Fa.flac",
        )
        .unwrap();
        assert_eq!(e.tokens, vec!["playlist", "open", ""]);
        let e = parse_line("aa%3Abb%3Acc%3Add%3Aee%3A01 playlist play http%3A%2F%2Fstream.example%2Fx%3Ftoken%3Dsecret").unwrap();
        assert!(!e.tokens.join(" ").contains("secret"));
        let e = parse_line("aa%3Abb%3Acc%3Add%3Aee%3A01 playlist play %2Fsrv%2Fmusic%2Fa.flac")
            .unwrap();
        assert_eq!(e.tokens[2], "");
        let e =
            parse_line("aa%3Abb%3Acc%3Add%3Aee%3A01 playlist play C%3A%5Cmusic%5Ca.flac").unwrap();
        assert_eq!(e.tokens[2], "");
    }

    #[test]
    fn only_known_event_types_pass() {
        for line in [
            "aa%3Abb%3Acc%3Add%3Aee%3A01 prefset server audiodir %2Fsrv",
            "aa%3Abb%3Acc%3Add%3Aee%3A01 playerpref volume 50",
            "aa%3Abb%3Acc%3Add%3Aee%3A01 displaynotify x",
            "pref audiodir %2Fsrv",
            "listen 1",
            "",
            "not-a-player playlist newsong x",
        ] {
            assert_eq!(parse_line(line), None, "{line}");
        }
        assert_eq!(parse_line("rescan done").unwrap().player, None);
    }

    #[test]
    fn garbage_never_panics() {
        for line in [
            "%",
            "%Z",
            "%%%%",
            "aa%3A playlist",
            "\u{0} \u{1}",
            &"x ".repeat(10_000),
        ] {
            let _ = parse_line(line);
        }
    }

    /// Regression: the remainder of an oversize CLI line was parsed as a line of its own,
    /// so an attacker-influenced string (e.g. a track title) could forge an event.
    #[tokio::test]
    async fn oversize_lines_are_dropped_whole() {
        let forged = "aa%3Abb%3Acc%3Add%3Aee%3A01 playlist newsong forged";
        let input = format!(
            "{}{forged}\nrescan done\n{}\n",
            "x".repeat(MAX_LINE),
            "y".repeat(3 * MAX_LINE)
        );
        let mut r = BufReader::with_capacity(4096, input.as_bytes());
        let mut buf = Vec::new();
        let mut lines = Vec::new();
        while let Some(complete) = read_line_capped(&mut r, &mut buf, MAX_LINE).await.unwrap() {
            if complete {
                lines.push(String::from_utf8_lossy(&buf).into_owned());
            }
        }
        assert_eq!(lines, vec!["rescan done\n".to_string()]);
    }

    #[tokio::test]
    async fn short_lines_and_a_final_unterminated_line_are_read() {
        let mut r = BufReader::new(&b"rescan done\nlast"[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 64).await.unwrap(),
            Some(true)
        );
        assert_eq!(buf, b"rescan done\n");
        assert_eq!(
            read_line_capped(&mut r, &mut buf, 64).await.unwrap(),
            Some(true)
        );
        assert_eq!(buf, b"last");
        assert_eq!(read_line_capped(&mut r, &mut buf, 64).await.unwrap(), None);
    }

    /// The listener's idle check and a new subscription can't interleave: either the
    /// listener sees the subscriber and carries on, or it stops first and the subscriber
    /// starts a new one.
    #[tokio::test]
    async fn listener_only_stops_when_nobody_is_subscribed() {
        let hub = Arc::new(Hub::default());
        // Nothing listens on port 9 here; the listener just retries.
        let rx = hub.subscribe("127.0.0.1:9".into());
        assert!(*hub.running.lock().unwrap());
        assert!(!hub.stop_if_idle(), "a subscriber is waiting");
        assert!(*hub.running.lock().unwrap());
        drop(rx);
        assert!(hub.stop_if_idle());
        assert!(!*hub.running.lock().unwrap());
        let _rx = hub.subscribe("127.0.0.1:9".into());
        assert!(
            *hub.running.lock().unwrap(),
            "a new subscriber restarts the listener"
        );
    }
}
