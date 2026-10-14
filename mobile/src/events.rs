//! The live event stream (`WS /v1/events`): player and library change notices, used as
//! hints to re-query state.
//!
//! - The first message is the access token; the gateway answers `{"subscribed":true}`.
//! - Reconnects with backoff (about 1, 2, 4 … 30 s, ±20 %) after network failures and
//!   dead sockets (nothing heard, not even a keep-alive ping, for 90 s).
//! - A 1008 "unauthorised" close (an expired token) renews the token and reconnects. Any
//!   other 1008 (revoked, replaced by a newer connection) is final, like in the web app,
//!   so two connections of one device never evict each other in a loop.
//! - `{"lms":"busy","since":…}` and `{"lms":"ok"}` say when the music server behind the
//!   gateway stops and starts answering ([`ServerEvent::LmsBusy`], [`ServerEvent::LmsOk`]).
//! - Messages are small JSON; anything malformed or oversized is ignored.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{Connector, connect_async_tls_with_config};

use crate::error::ApiError;
use crate::session::{Core, Session};
use crate::transport;

const MAX_MESSAGE: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The gateway pings every 30 s; silence for this long means the socket is dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// A change on the server.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ServerEvent {
    /// Something changed on a player: `name` is the LMS event (`playlist`, `mixer`,
    /// `pause`, `play`, `stop`, `power`, `client`, `sync`, `time`, `newmetadata`).
    PlayerChanged {
        player_id: String,
        name: String,
        args: Vec<String>,
    },
    /// The library changed (`rescan`).
    LibraryChanged { name: String, args: Vec<String> },
    /// Events may have been missed (or the stream (re)connected): re-query everything.
    Resync,
    /// The music server (LMS) stopped answering at `since_ms` (Unix ms; 0 if not given):
    /// it is busy, the gateway is not. Commands are refused (`ApiError::ServerBusy`) until
    /// [`ServerEvent::LmsOk`]. On (re)connecting assume it answers; the gateway says so
    /// right after subscribing if it doesn't.
    LmsBusy { since_ms: u64 },
    /// The music server answers again: refresh.
    LmsOk,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum EventsState {
    Connecting,
    Live,
    /// Disconnected; trying again after `delay_ms`.
    Retrying {
        delay_ms: u64,
    },
    /// Ended for good: signed out, revoked, or replaced by a newer connection.
    Stopped {
        reason: String,
    },
}

/// Receives events (on a background thread; hand them to the UI thread yourself).
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: ServerEvent);
    fn on_state(&self, state: EventsState);
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Parsed {
    Subscribed,
    Event(ServerEvent),
}

/// Parse one message from the gateway; anything unexpected is `None`.
pub(crate) fn parse_message(text: &str) -> Option<Parsed> {
    if text.len() > MAX_MESSAGE {
        return None;
    }
    let v: Value = serde_json::from_str(text).ok()?;
    let o = v.as_object()?;
    if o.get("subscribed") == Some(&Value::Bool(true)) {
        return Some(Parsed::Subscribed);
    }
    if o.get("resync") == Some(&Value::Bool(true)) {
        return Some(Parsed::Event(ServerEvent::Resync));
    }
    if let Some(lms) = o.get("lms") {
        return match lms.as_str()? {
            "busy" => {
                let since_ms = match o.get("since") {
                    None => 0,
                    Some(v) => v.as_u64()?,
                };
                Some(Parsed::Event(ServerEvent::LmsBusy { since_ms }))
            }
            "ok" => Some(Parsed::Event(ServerEvent::LmsOk)),
            _ => None,
        };
    }
    let tokens: Vec<String> = o
        .get("event")?
        .as_array()?
        .iter()
        .map(|t| t.as_str().map(crate::model::clean_text))
        .collect::<Option<_>>()?;
    let (name, args) = tokens.split_first()?;
    if name.is_empty() || args.len() > 32 {
        return None;
    }
    match o.get("player")? {
        Value::Null => Some(Parsed::Event(ServerEvent::LibraryChanged {
            name: name.clone(),
            args: args.to_vec(),
        })),
        Value::String(p) if crate::commands::is_player_id(p) => {
            Some(Parsed::Event(ServerEvent::PlayerChanged {
                player_id: p.to_ascii_lowercase(),
                name: name.clone(),
                args: args.to_vec(),
            }))
        }
        _ => None,
    }
}

/// Backoff with jitter: ~1 s, 2 s, 4 s … up to ~30 s. `random` is in 0..=1.
pub(crate) fn backoff(attempt: u32, random: f64) -> Duration {
    let random = if random.is_nan() { 0.5 } else { random };
    let base = 1000f64 * 2f64.powi(attempt.min(5) as i32);
    Duration::from_millis((base.min(30_000.0) * (0.8 + 0.4 * random.clamp(0.0, 1.0))) as u64)
}

fn random01() -> f64 {
    getrandom::u32().map_or(0.5, |r| f64::from(r) / f64::from(u32::MAX))
}

enum Outcome {
    /// Try again after a backoff.
    Retry,
    /// Reconnect at once (the token was just renewed).
    Renewed,
    Stop(String),
}

async fn connect_once(
    core: &Arc<Core>,
    listener: &dyn EventListener,
    attempt: &mut u32,
) -> Outcome {
    let token = match core.access_token().await {
        Ok(t) => t,
        Err(ApiError::Unauthorised) => return Outcome::Stop("signed out".into()),
        Err(_) => return Outcome::Retry,
    };
    let server = core.server().clone();
    let Ok(request) = server.events_url().as_str().into_client_request() else {
        return Outcome::Stop("bad server address".into());
    };
    let connector = if server.is_secure() {
        match transport::tls_config(&[b"http/1.1"]) {
            Ok(cfg) => Connector::Rustls(Arc::new(cfg)),
            Err(_) => return Outcome::Stop("TLS is not available".into()),
        }
    } else {
        Connector::Plain
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let connecting = connect_async_tls_with_config(request, Some(config), false, Some(connector));
    let mut ws = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok((ws, _))) => ws,
        _ => return Outcome::Retry,
    };
    if ws.send(Message::Text(token.clone().into())).await.is_err() {
        return Outcome::Retry;
    }
    loop {
        let msg = match tokio::time::timeout(IDLE_TIMEOUT, ws.next()).await {
            Ok(Some(Ok(m))) => m,
            // Silence, end of stream or a broken socket.
            _ => return Outcome::Retry,
        };
        match msg {
            Message::Text(t) => match parse_message(t.as_str()) {
                Some(Parsed::Subscribed) => {
                    *attempt = 0;
                    listener.on_state(EventsState::Live);
                    // Anything may have changed while we were away.
                    listener.on_event(ServerEvent::Resync);
                }
                Some(Parsed::Event(e)) => listener.on_event(e),
                None => {}
            },
            Message::Close(frame) => {
                let (code, reason) = frame
                    .map(|f| (f.code, f.reason.to_string()))
                    .unwrap_or((CloseCode::Status, String::new()));
                return match (code, reason.as_str()) {
                    (CloseCode::Policy, "unauthorised") => match core.renew(Some(&token)).await {
                        Ok(_) => Outcome::Renewed,
                        Err(ApiError::Unauthorised) => Outcome::Stop("signed out".into()),
                        Err(_) => Outcome::Retry,
                    },
                    (CloseCode::Policy, r) => Outcome::Stop(crate::model::clean_text(r)),
                    _ => Outcome::Retry,
                };
            }
            // Pings are answered by the WebSocket layer; nothing else is expected.
            _ => {}
        }
    }
}

async fn run(core: Arc<Core>, listener: Arc<dyn EventListener>) {
    let mut attempt = 0u32;
    let mut renewed_in_a_row = 0u32;
    loop {
        listener.on_state(EventsState::Connecting);
        let outcome = connect_once(&core, &*listener, &mut attempt).await;
        let retry = match outcome {
            Outcome::Stop(reason) => {
                listener.on_state(EventsState::Stopped { reason });
                return;
            }
            // One immediate retry with a fresh token; a server that keeps refusing gets
            // the normal backoff.
            Outcome::Renewed if renewed_in_a_row == 0 => {
                renewed_in_a_row += 1;
                false
            }
            Outcome::Renewed | Outcome::Retry => true,
        };
        if attempt == 0 {
            renewed_in_a_row = 0;
        }
        if retry {
            let delay = backoff(attempt, random01());
            attempt = attempt.saturating_add(1);
            listener.on_state(EventsState::Retrying {
                delay_ms: delay.as_millis() as u64,
            });
            tokio::time::sleep(delay).await;
        }
    }
}

/// A running subscription; ends when cancelled or dropped.
#[derive(uniffi::Object)]
pub struct EventSubscription {
    task: Mutex<Option<tokio::task::AbortHandle>>,
}

#[uniffi::export]
impl EventSubscription {
    /// Stop receiving events and close the socket.
    pub fn cancel(&self) {
        if let Some(t) = self.task.lock().unwrap().take() {
            t.abort();
        }
    }

    pub fn is_active(&self) -> bool {
        self.task
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|t| !t.is_finished())
    }
}

impl Drop for EventSubscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(uniffi::Object)]
pub struct Events {
    session: Arc<Session>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Events {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Events { session })
    }

    /// Start receiving events. Keep the returned subscription; cancel it (or let it go)
    /// to stop.
    pub async fn subscribe(&self, listener: Arc<dyn EventListener>) -> Arc<EventSubscription> {
        let task = tokio::spawn(run(self.session.core().clone(), listener));
        Arc::new(EventSubscription {
            task: Mutex::new(Some(task.abort_handle())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        assert_eq!(
            parse_message(r#"{"subscribed":true}"#),
            Some(Parsed::Subscribed)
        );
        assert_eq!(
            parse_message(r#"{"resync":true}"#),
            Some(Parsed::Event(ServerEvent::Resync))
        );
        assert_eq!(
            parse_message(r#"{"player":"AA:BB:CC:DD:EE:01","event":["mixer","volume","50"]}"#),
            Some(Parsed::Event(ServerEvent::PlayerChanged {
                player_id: "aa:bb:cc:dd:ee:01".into(),
                name: "mixer".into(),
                args: vec!["volume".into(), "50".into()],
            }))
        );
        assert_eq!(
            parse_message(r#"{"player":null,"event":["rescan","done"]}"#),
            Some(Parsed::Event(ServerEvent::LibraryChanged {
                name: "rescan".into(),
                args: vec!["done".into()]
            }))
        );
        for bad in [
            "",
            "null",
            "[]",
            "{}",
            r#"{"subscribed":"true"}"#,
            r#"{"player":"x","event":["mixer"]}"#,
            r#"{"player":"aa:bb:cc:dd:ee:01","event":[]}"#,
            r#"{"player":"aa:bb:cc:dd:ee:01","event":[1]}"#,
            r#"{"player":"aa:bb:cc:dd:ee:01","event":"mixer"}"#,
            r#"{"player":5,"event":["mixer"]}"#,
            r#"{"event":["mixer"]}"#,
        ] {
            assert_eq!(parse_message(bad), None, "{bad}");
        }
        let huge = format!(
            r#"{{"player":null,"event":["{}"]}}"#,
            "x".repeat(MAX_MESSAGE)
        );
        assert_eq!(parse_message(&huge), None);
    }

    #[test]
    fn the_music_server_being_busy_and_back() {
        assert_eq!(
            parse_message(r#"{"lms":"busy","since":1760000000000}"#),
            Some(Parsed::Event(ServerEvent::LmsBusy {
                since_ms: 1_760_000_000_000
            }))
        );
        assert_eq!(
            parse_message(r#"{"lms":"busy"}"#),
            Some(Parsed::Event(ServerEvent::LmsBusy { since_ms: 0 }))
        );
        assert_eq!(
            parse_message(r#"{"lms":"ok"}"#),
            Some(Parsed::Event(ServerEvent::LmsOk))
        );
        // Extra fields from a newer gateway are fine.
        assert_eq!(
            parse_message(r#"{"lms":"ok","since":5,"x":[1]}"#),
            Some(Parsed::Event(ServerEvent::LmsOk))
        );
        for bad in [
            r#"{"lms":"BUSY"}"#,
            r#"{"lms":"down"}"#,
            r#"{"lms":""}"#,
            r#"{"lms":true}"#,
            r#"{"lms":null}"#,
            r#"{"lms":["busy"]}"#,
            r#"{"lms":{"busy":true}}"#,
            r#"{"lms":"busy","since":-1}"#,
            r#"{"lms":"busy","since":1.5}"#,
            r#"{"lms":"busy","since":"1760000000000"}"#,
            r#"{"lms":"busy","since":null}"#,
            r#"{"lms":"busy","since":1e400}"#,
            r#"{"lms":"busy","since":18446744073709551616}"#,
            r#"["lms","busy"]"#,
            r#"{"lms":"busy""#,
        ] {
            assert_eq!(parse_message(bad), None, "{bad}");
        }
        let huge = format!(r#"{{"lms":"ok","pad":"{}"}}"#, "x".repeat(MAX_MESSAGE));
        assert_eq!(parse_message(&huge), None);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(0, 0.5), Duration::from_millis(1000));
        assert_eq!(backoff(1, 0.5), Duration::from_millis(2000));
        assert_eq!(backoff(10, 0.5), Duration::from_millis(30_000));
        assert_eq!(backoff(u32::MAX, 1.0), Duration::from_millis(36_000));
        assert_eq!(backoff(0, 0.0), Duration::from_millis(800));
        assert_eq!(backoff(0, f64::NAN), Duration::from_millis(1000));
    }

    #[test]
    fn arbitrary_messages_never_panic() {
        let alphabet: Vec<char> = "{}[]\":,nultrsubcdyevpakmo0123:\\é ".chars().collect();
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..20_000 {
            let len = (seed % 64) as usize;
            let mut s = String::new();
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let _ = parse_message(&s);
        }
    }
}
