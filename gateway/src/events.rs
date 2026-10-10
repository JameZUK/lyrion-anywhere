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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
        let rx = self.tx.subscribe();
        let mut running = self.running.lock().unwrap();
        if !*running {
            *running = true;
            let hub = self.clone();
            tokio::spawn(async move { hub.listen(cli_addr).await });
        }
        rx
    }

    async fn listen(self: Arc<Self>, addr: String) {
        let mut backoff = Duration::from_millis(500);
        loop {
            match self.listen_once(&addr).await {
                Ok(()) => info!("LMS CLI connection closed"),
                Err(e) => warn!(error = %e, "LMS CLI connection failed"),
            }
            if self.tx.receiver_count() == 0 {
                *self.running.lock().unwrap() = false;
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
        let mut line = String::new();
        loop {
            line.clear();
            let n = (&mut lines)
                .take(MAX_LINE as u64)
                .read_line(&mut line)
                .await?;
            if n == 0 {
                return Ok(());
            }
            if self.tx.receiver_count() == 0 {
                return Ok(()); // nobody listening: drop the connection
            }
            if let Some(ev) = parse_line(&line) {
                let _ = self.tx.send(ev);
            }
        }
    }
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
}
