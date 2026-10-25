//! `/v1/player`: authenticate a socket and attach it to a new or resumed session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use anywhere_core::proto::ClientFrameReader;

use crate::accounts::Principal;
use crate::session::{AttachError, ClientInput, EndReason, Kick, Outbound, Session};
use crate::{AppState, origin_allowed};

/// Bound on a single WebSocket message from a client.
const MAX_WS_MESSAGE: usize = ClientFrameReader::MAX_FRAME + 1024;
const MAX_AUTH_MESSAGE: usize = 1024;
const MAX_CONTROL_MESSAGE: usize = 256;
/// The longest resume grace a client may ask for at sign-in, in seconds. The gateway grants
/// at most its own maximum ([`crate::Config::max_resume_grace`]).
pub const MAX_GRACE_REQUEST: u64 = 3600;

/// The text messages allowed after authentication, each exactly one of:
/// - `{"ack":{"stream":n,"bytes":b}}`: bytes of stream `n` the client has received;
/// - `{"quality":"original"|"high"|"low"}`: the level of the streams that follow
///   (docs/plans/streaming-quality.md).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlMessage {
    ack: Option<StreamAck>,
    quality: Option<String>,
}

enum Control {
    Ack(StreamAck),
    Quality(crate::live::Quality),
}

/// A control message, or `None` for anything else (too long, malformed, unknown fields,
/// both or neither field, an unknown level).
fn parse_control(t: &str) -> Option<Control> {
    if t.len() > MAX_CONTROL_MESSAGE {
        return None;
    }
    match serde_json::from_str::<ControlMessage>(t).ok()? {
        ControlMessage {
            ack: Some(a),
            quality: None,
        } => Some(Control::Ack(a)),
        ControlMessage {
            ack: None,
            quality: Some(q),
        } => crate::live::Quality::parse(&q).map(Control::Quality),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamAck {
    stream: u64,
    bytes: u64,
}

pub async fn player_upgrade(
    State(gw): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    slot: Option<axum::Extension<crate::limits::ConnSlot>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&gw.cfg, &headers) {
        warn!("refused cross-origin websocket");
        return (StatusCode::FORBIDDEN, "origin not allowed\n").into_response();
    }
    let addr = crate::client_addr(&gw.cfg, peer, &headers);
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        // The connection's place under the caps is held until the socket closes.
        .on_upgrade(move |socket| async move {
            handle(gw, socket, addr).await;
            drop(slot);
        })
}

#[derive(Deserialize)]
struct AuthMessage {
    token: String,
    resume: Option<ResumeRequest>,
    /// The level of the streams to come (`original`, `high`, `low`); none means
    /// `original`.
    quality: Option<String>,
}

#[derive(Deserialize)]
struct ResumeRequest {
    session: String,
    secret: String,
    /// Binary messages (frames) the client received in this session before the drop.
    received: u64,
}

enum Auth {
    New(Principal),
    Resume(Principal, ResumeRequest),
}

fn auth_device(a: &Auth) -> &Principal {
    match a {
        Auth::New(p) | Auth::Resume(p, _) => p,
    }
}

/// The level asked for at sign-in. An unknown one is taken as `original` (never refused:
/// a refusal here would look like a failed sign-in).
fn auth_quality(text: &str) -> crate::live::Quality {
    if text.len() > MAX_AUTH_MESSAGE || !text.starts_with('{') {
        return Default::default();
    }
    serde_json::from_str::<AuthMessage>(text)
        .ok()
        .and_then(|m| m.quality)
        .and_then(|q| crate::live::Quality::parse(&q))
        .unwrap_or_default()
}

/// Whether text keepalives were asked for at sign-in (`"keepalive": true`). Anything else,
/// including a malformed value, means no: only clients that understand them receive them.
fn auth_keepalive(text: &str) -> bool {
    if text.len() > MAX_AUTH_MESSAGE || !text.starts_with('{') {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("keepalive").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// The resume grace asked for at sign-in (`"grace"`, whole seconds): `Ok(None)` when not
/// asked (or for the bare-token form), `Err` for anything but an integer 0–3600 (a string,
/// a fraction, a negative or too large a number, `null`, …), which is a policy violation.
/// (A message that isn't valid JSON fails authentication anyway.)
fn auth_grace(text: &str) -> Result<Option<u64>, ()> {
    if text.len() > MAX_AUTH_MESSAGE || !text.starts_with('{') {
        return Ok(None);
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(None);
    };
    match v.get("grace") {
        None => Ok(None),
        Some(g) => match g.as_u64() {
            Some(n) if n <= MAX_GRACE_REQUEST => Ok(Some(n)),
            _ => Err(()),
        },
    }
}

/// The close code for a client whose player protocol is too old, and its reason (the apps
/// map it to a plain message, never showing it raw).
pub const CLOSE_TOO_OLD: u16 = 4426;
pub const TOO_OLD_REASON: &str = "update the app";
/// How often a refused device is audited and logged at most (it is always refused). Apps
/// from before protocol 2 take 4426 for a dropped connection and sign in again every few
/// seconds; one row each time would push the rest of the audit trail out within days.
pub const TOO_OLD_NOTE_EVERY: Duration = Duration::from_secs(3600);
/// Refused devices remembered at most (a fuller map is pruned, then cleared).
const TOO_OLD_SEEN_CAP: usize = 4096;

/// The player protocol given at sign-in (`"protocol"`): `Ok` for a whole JSON number of at
/// least [`crate::PROTOCOL`] (a higher one than this gateway knows is accepted: the newer
/// client decides), else `Err` with what was given, for the log and the audit trail: a
/// whole number as it is, anything else only as its type (`string`, `float`, `bool`,
/// `null`, `array`, `object`), and `none` for no `protocol` (or the bare-token form of
/// protocol 1). Only ASCII from a fixed set ever comes out, so a client cannot write
/// bidi overrides, line separators or anything long into what an admin reads.
fn auth_protocol(text: &str) -> Result<u64, String> {
    use serde_json::Value;
    let none = || "none".to_string();
    if text.len() > MAX_AUTH_MESSAGE || !text.starts_with('{') {
        return Err(none());
    }
    let v = serde_json::from_str::<Value>(text).map_err(|_| none())?;
    let Some(p) = v.get("protocol") else {
        return Err(none());
    };
    match p {
        Value::Number(n) => match (n.as_u64(), n.as_i64()) {
            (Some(n), _) if n >= u64::from(crate::PROTOCOL) => Ok(n),
            (Some(n), _) => Err(n.to_string()),
            (None, Some(n)) => Err(n.to_string()),
            (None, None) => Err("float".into()),
        },
        Value::String(_) => Err("string".into()),
        Value::Bool(_) => Err("bool".into()),
        Value::Null => Err("null".into()),
        Value::Array(_) => Err("array".into()),
        Value::Object(_) => Err("object".into()),
    }
}

/// The protocol a refused client is recorded as speaking, from what [`auth_protocol`]
/// reported: 1 for none (protocol 1's forms carried no number), a whole number below 2 as
/// it is, and 0 for anything else, so the device lists show "needs updating" whatever it
/// sent.
fn old_protocol(given: &str) -> u64 {
    if given == "none" {
        1
    } else {
        given.parse::<u64>().unwrap_or(0)
    }
}

/// What to do about one refused sign-in (it is refused either way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TooOldNote {
    /// Write the audit row and the warning: the first for this device in
    /// [`TOO_OLD_NOTE_EVERY`].
    pub audit: bool,
    /// Record the protocol in the device lists: it differs from what was last recorded.
    pub record: bool,
}

/// Refused devices lately audited: device → (when, the protocol recorded for it).
#[derive(Default)]
pub(crate) struct TooOldSeen(std::sync::Mutex<std::collections::HashMap<i64, (Instant, u64)>>);

impl TooOldSeen {
    pub(crate) fn note(&self, device: i64, recorded: u64, now: Instant) -> TooOldNote {
        let mut seen = self.0.lock().unwrap();
        let fresh = |t: &Instant| now.saturating_duration_since(*t) < TOO_OLD_NOTE_EVERY;
        if let Some((at, p)) = seen.get_mut(&device)
            && fresh(at)
        {
            let record = *p != recorded;
            *p = recorded;
            return TooOldNote {
                audit: false,
                record,
            };
        }
        if seen.len() >= TOO_OLD_SEEN_CAP && !seen.contains_key(&device) {
            seen.retain(|_, (t, _)| fresh(t));
            if seen.len() >= TOO_OLD_SEEN_CAP {
                seen.clear();
            }
        }
        seen.insert(device, (now, recorded));
        TooOldNote {
            audit: true,
            record: true,
        }
    }

    /// The device attached with a current protocol: its next refusal is news again.
    pub(crate) fn forget(&self, device: i64) {
        self.0.lock().unwrap().remove(&device);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// The grace granted for `asked` seconds: at most the server's maximum.
fn granted_grace(gw: &AppState, asked: u64) -> Duration {
    Duration::from_secs(asked).min(gw.cfg.max_resume_grace)
}

fn parse_auth(gw: &AppState, text: &str) -> Option<Auth> {
    if text.len() > MAX_AUTH_MESSAGE {
        return None;
    }
    if text.starts_with('{') {
        let m: AuthMessage = serde_json::from_str(text).ok()?;
        let dev = gw.accounts.authenticate(&m.token)?;
        Some(match m.resume {
            Some(r) => Auth::Resume(dev, r),
            None => Auth::New(dev),
        })
    } else {
        gw.accounts.authenticate(text).map(Auth::New)
    }
}

async fn close(mut ws: WebSocket, code: u16, reason: &str) {
    let _ = ws
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn handle(gw: AppState, mut ws: WebSocket, addr: std::net::IpAddr) {
    let first = tokio::time::timeout(gw.cfg.auth_timeout, ws.recv()).await;
    let (auth, protocol, quality, grace, text_keepalive) = match first {
        Ok(Some(Ok(Message::Text(t)))) => (
            parse_auth(&gw, t.as_str()),
            auth_protocol(t.as_str()),
            auth_quality(t.as_str()),
            auth_grace(t.as_str()),
            auth_keepalive(t.as_str()),
        ),
        _ => (
            None,
            Err("none".into()),
            Default::default(),
            Ok(None),
            false,
        ),
    };
    let Some(auth) = auth else {
        warn!("player authentication failed");
        return close(ws, close_code::POLICY, "unauthorised").await;
    };
    // An old client: told to update, before anything is opened or resumed (a session it
    // could resume is left to its grace).
    let protocol = match protocol {
        Ok(p) => p,
        Err(given) => {
            let device = auth_device(&auth);
            let (user, id) = (device.username.clone(), device.device_id);
            // The device lists show who is behind: no number is protocol 1 (which had
            // none), and anything that is not a protocol at all is 0.
            let recorded = old_protocol(&given);
            let note = gw.too_old.note(id, recorded, Instant::now());
            if note.audit {
                warn!(
                    user = %device.username,
                    device = id,
                    mac = %device.mac_string(),
                    protocol = %given,
                    "player protocol too old; asked to update the app (noted once an hour per device)"
                );
            }
            if note.audit || note.record {
                gw.db(move |a| {
                    if note.audit {
                        a.audit("player.too_old", Some(&user), Some(id), &given);
                    }
                    if note.record {
                        a.record_protocol(id, recorded);
                    }
                })
                .await;
            }
            return close(ws, CLOSE_TOO_OLD, TOO_OLD_REASON).await;
        }
    };
    let grace = match grace {
        Ok(g) => g,
        Err(()) => {
            // A policy violation: nothing is opened, and a session this client could resume
            // is ended, as for any other violation.
            if let Auth::Resume(device, r) = &auth
                && let Some(s) = gw.session(&r.session).filter(|s| {
                    s.device.device_id == device.device_id && s.secret_matches(&r.secret)
                })
            {
                s.end(&gw, EndReason::Policy).await;
            }
            warn!(mac = %auth_device(&auth).mac_string(), "player sign-in with a malformed resume grace");
            return close(ws, close_code::POLICY, "policy violation").await;
        }
    };

    let (session, resumed, received) = match auth {
        Auth::New(device) => match Session::open(&gw, device.clone()).await {
            Ok(s) => (s, false, 0),
            Err(e) => {
                warn!(mac = %device.mac_string(), error = %e, "cannot reach LMS slimproto");
                return close(ws, close_code::AGAIN, "server unavailable").await;
            }
        },
        Auth::Resume(device, r) => {
            // Wrong session, another device's session, a bad secret and an expired session
            // are indistinguishable to the client.
            let s = gw
                .session(&r.session)
                .filter(|s| s.device.device_id == device.device_id && s.secret_matches(&r.secret));
            match s {
                Some(s) => (s, true, r.received),
                None => {
                    warn!(mac = %device.mac_string(), "resume refused");
                    return close(ws, 4404, "no such session").await;
                }
            }
        }
    };

    // Revoked while the session was being opened (after the token was checked, before
    // the session was listed for revocation to find): end it now.
    if !gw.accounts.device_active(session.device.device_id) {
        warn!(mac = %session.device.mac_string(), "device revoked while connecting");
        session.end(&gw, crate::session::EndReason::Revoked).await;
        return close(ws, close_code::POLICY, "unauthorised").await;
    }

    session.set_quality(quality);
    // Asked for: at most the server's maximum. Not asked: a new session keeps the default
    // and a resumed one what it was granted before.
    if let Some(asked) = grace {
        session.set_grace(granted_grace(&gw, asked));
    }
    let attached = match session.attach(received) {
        Ok(a) => a,
        Err(AttachError::Gap) => {
            warn!(mac = %session.device.mac_string(), "resume gap too old; ending session");
            session.end(&gw, EndReason::Replaced).await;
            return close(ws, 4404, "no such session").await;
        }
        Err(AttachError::Invalid) => {
            warn!(mac = %session.device.mac_string(), "resume claimed frames never sent");
            return close(ws, 4404, "no such session").await;
        }
        Err(AttachError::Gone) => return close(ws, 4404, "no such session").await,
    };
    let generation = attached.generation;
    let greeting = serde_json::json!({
        "session": session.id,
        "secret": session.secret(),
        "mac": session.device.mac_string(),
        "name": session.name(),
        "resumed": resumed,
        // The resume grace granted, in whole seconds.
        "grace": session.grace().as_secs(),
    })
    .to_string();
    if ws.send(Message::Text(greeting.into())).await.is_err() {
        session.detach(&gw, generation);
        return;
    }
    // The stream in progress, if any, right after the greeting (newer ones follow from
    // the queue).
    if let Some(m) = attached.stream_message
        && ws.send(Message::Text(m.into())).await.is_err()
    {
        session.detach(&gw, generation);
        return;
    }
    // Off the async workers (M7): a stalled disk must not hold up other sessions.
    let device = session.device.device_id;
    let seen = gw.accounts.claim_seen(device);
    gw.too_old.forget(device);
    gw.db(move |a| {
        a.touch_device(device);
        a.record_protocol(device, protocol);
        if seen {
            a.write_seen(device, addr);
        }
    })
    .await;
    info!(device = %session.name(), user = %session.device.username, mac = %session.device.mac_string(), resumed, "player attached");

    let (sink, stream) = ws.split();
    let keepalive = Keepalive {
        every: gw.cfg.player_keepalive,
        text: text_keepalive,
    };
    let mut writer = tokio::spawn(write_loop(
        sink,
        attached.rx,
        attached.kick.clone(),
        keepalive,
    ));
    let outcome = read_loop(&session, generation, stream, attached.kick).await;
    match outcome {
        ReadEnd::Policy(why) => {
            warn!(mac = %session.device.mac_string(), reason = %why, "player closed for policy violation");
            // A client that breaks protocol loses its whole session, not just this socket.
            session.end(&gw, EndReason::Policy).await;
        }
        ReadEnd::LmsGone => session.end(&gw, EndReason::LmsClosed).await,
        ReadEnd::SocketGone => session.detach(&gw, generation),
        // Replaced, too slow, or the session ended: the session has already dealt with it.
        ReadEnd::Kicked => {}
    }
    // The writer ends once its queue is closed or after sending a close frame. A writer
    // stuck on a client that has stopped reading is abandoned, which closes the socket.
    if tokio::time::timeout(WRITER_GRACE, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

/// How long a closing socket's writer may take to flush its close frame.
const WRITER_GRACE: Duration = Duration::from_secs(6);
const CLOSE_SEND_TIMEOUT: Duration = Duration::from_secs(5);

enum ReadEnd {
    Policy(String),
    LmsGone,
    SocketGone,
    Kicked,
}

/// Resolves when the session asks this socket to close, with the close code and reason.
async fn kicked(kick: &mut watch::Receiver<Kick>) -> (u16, &'static str) {
    let k = match kick.wait_for(|k| k.is_some()).await {
        Ok(k) => *k,
        Err(_) => None,
    };
    match k {
        Some(k) => k,
        // The session dropped this socket without a reason: its queue closes too.
        None => std::future::pending().await,
    }
}

async fn read_loop(
    session: &Arc<Session>,
    generation: u64,
    mut stream: futures_util::stream::SplitStream<WebSocket>,
    mut kick: watch::Receiver<Kick>,
) -> ReadEnd {
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            _ = kicked(&mut kick) => return ReadEnd::Kicked,
        };
        let Some(msg) = msg else {
            return ReadEnd::SocketGone;
        };
        // A pong only answers the gateway's own keepalive ping (a browser sends it even
        // while its scripts are suspended), so it says nothing about the player.
        if msg.as_ref().is_ok_and(|m| !matches!(m, Message::Pong(_))) {
            session.heard_from(generation);
        }
        let data = match msg {
            Ok(Message::Binary(b)) => b,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Text(t)) => match parse_control(t.as_str()) {
                Some(Control::Ack(a)) => {
                    session.ack(generation, a.stream, a.bytes);
                    continue;
                }
                Some(Control::Quality(q)) => {
                    session.set_quality(q);
                    continue;
                }
                None => return ReadEnd::Policy("unexpected text message".into()),
            },
            // Who hung up, and why, is the first question when a player keeps dropping
            // (live, 2026-10-15: a phone's socket went every minute with no cause logged).
            Ok(Message::Close(frame)) => {
                let (code, reason) = frame
                    .map(|f| (f.code, f.reason.to_string()))
                    .unwrap_or((0, String::new()));
                info!(mac = %session.device.mac_string(), code, reason = %reason, "player closed its socket");
                return ReadEnd::SocketGone;
            }
            Err(e) => {
                info!(mac = %session.device.mac_string(), error = %e, "player socket failed");
                return ReadEnd::SocketGone;
            }
        };
        if session.is_ended() {
            return ReadEnd::SocketGone;
        }
        match session.client_bytes(generation, &data).await {
            ClientInput::Ok => {}
            ClientInput::Policy(why) => return ReadEnd::Policy(why),
            ClientInput::LmsGone => return ReadEnd::LmsGone,
            // Replaced or detached meanwhile: this socket no longer speaks for the player.
            ClientInput::Stale => return ReadEnd::Kicked,
        }
    }
}

enum WriteStep {
    Send(Outbound),
    Kicked((u16, &'static str)),
    Keepalive,
    Done,
}

/// The keepalives a player socket gets.
struct Keepalive {
    every: Duration,
    /// Also as text (for clients that asked at sign-in).
    text: bool,
}

/// The text keepalive: never a greeting (no `session`), never a frame (not binary).
pub const KEEPALIVE_TEXT: &str = r#"{"keepalive":true}"#;

async fn write_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<Outbound>,
    mut kick: watch::Receiver<Kick>,
    keepalive: Keepalive,
) {
    let mut tick = tokio::time::interval(keepalive.every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    loop {
        let step = tokio::select! {
            biased;
            out = rx.recv() => match out {
                Some(out) => WriteStep::Send(out),
                // The queue is dropped along with a kick: still say why.
                None => match *kick.borrow() {
                    Some(k) => WriteStep::Kicked(k),
                    None => WriteStep::Done,
                },
            },
            k = kicked(&mut kick) => WriteStep::Kicked(k),
            _ = tick.tick() => WriteStep::Keepalive,
        };
        let (code, reason) = match step {
            WriteStep::Done => return,
            WriteStep::Kicked(k) => k,
            WriteStep::Keepalive => {
                let mut msgs = vec![Message::Ping(Default::default())];
                if keepalive.text {
                    msgs.push(Message::Text(KEEPALIVE_TEXT.into()));
                }
                for m in msgs {
                    let sent = tokio::select! {
                        r = sink.send(m) => Ok(r),
                        k = kicked(&mut kick) => Err(k),
                    };
                    match sent {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => return,
                        Err(k) => {
                            let close = Message::Close(Some(CloseFrame {
                                code: k.0,
                                reason: k.1.into(),
                            }));
                            let _ =
                                tokio::time::timeout(CLOSE_SEND_TIMEOUT, sink.send(close)).await;
                            return;
                        }
                    }
                }
                continue;
            }
            WriteStep::Send(Outbound::Close(code, reason)) => (code, reason),
            WriteStep::Send(Outbound::Text(t)) => {
                let sent = tokio::select! {
                    r = sink.send(Message::Text(t.into())) => Ok(r),
                    k = kicked(&mut kick) => Err(k),
                };
                match sent {
                    Ok(Ok(())) => continue,
                    Ok(Err(_)) => return,
                    Err(k) => k,
                }
            }
            WriteStep::Send(Outbound::Frame(f)) => {
                let sent = tokio::select! {
                    r = sink.send(Message::Binary(f.into())) => Ok(r),
                    k = kicked(&mut kick) => Err(k),
                };
                match sent {
                    Ok(Ok(())) => continue,
                    Ok(Err(_)) => return,
                    Err(k) => k,
                }
            }
        };
        let close = Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }));
        let _ = tokio::time::timeout(CLOSE_SEND_TIMEOUT, sink.send(close)).await;
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::Quality;

    /// Text keepalives only for a client that asked with a real `true`: anything else (an
    /// old client, a string, a number, a bare token, oversize or broken JSON) gets none.
    #[test]
    fn text_keepalives_only_when_asked_for() {
        assert!(auth_keepalive(r#"{"token":"t","keepalive":true}"#));
        for no in [
            r#"{"token":"t"}"#,
            r#"{"token":"t","keepalive":false}"#,
            r#"{"token":"t","keepalive":"true"}"#,
            r#"{"token":"t","keepalive":1}"#,
            r#"{"token":"t","keepalive":null}"#,
            r#"{"token":"t","keepalive":true"#,
            "bare-token",
        ] {
            assert!(!auth_keepalive(no), "{no}");
        }
        let big = format!(
            r#"{{"token":"{}","keepalive":true}}"#,
            "x".repeat(MAX_AUTH_MESSAGE)
        );
        assert!(!auth_keepalive(&big));
        // The keepalive can never be taken for a greeting or a control message.
        let v: serde_json::Value = serde_json::from_str(KEEPALIVE_TEXT).unwrap();
        assert!(v.get("session").is_none());
        assert!(parse_control(KEEPALIVE_TEXT).is_none());
    }

    #[test]
    fn control_messages() {
        assert!(matches!(
            parse_control(r#"{"ack":{"stream":3,"bytes":100}}"#),
            Some(Control::Ack(StreamAck {
                stream: 3,
                bytes: 100
            }))
        ));
        for (m, q) in [
            ("original", Quality::Original),
            ("high", Quality::High),
            ("low", Quality::Low),
        ] {
            let msg = format!(r#"{{"quality":"{m}"}}"#);
            assert!(
                matches!(parse_control(&msg), Some(Control::Quality(x)) if x == q),
                "{m}"
            );
        }
        for bad in [
            "",
            "{}",
            "null",
            "[]",
            r#"{"quality":"ultra"}"#,
            r#"{"quality":"High"}"#,
            r#"{"quality":""}"#,
            r#"{"quality":1}"#,
            r#"{"quality":["low"]}"#,
            r#"{"quality":null}"#,
            r#"{"ack":null}"#,
            r#"{"quality":"low","ack":{"stream":1,"bytes":1}}"#,
            r#"{"quality":"low","extra":true}"#,
            r#"{"ack":{"stream":1,"bytes":1,"x":2}}"#,
            r#"{"ack":{"stream":-1,"bytes":1}}"#,
        ] {
            assert!(parse_control(bad).is_none(), "{bad}");
        }
        let long = format!(
            r#"{{"quality":"low","pad":"{}"}}"#,
            "x".repeat(MAX_CONTROL_MESSAGE)
        );
        assert!(parse_control(&long).is_none());
    }

    #[test]
    fn sign_in_grace_is_an_integer_up_to_an_hour() {
        assert_eq!(auth_grace("bare-token"), Ok(None));
        assert_eq!(auth_grace(r#"{"token":"t"}"#), Ok(None));
        assert_eq!(auth_grace(r#"{"token":"t","grace":0}"#), Ok(Some(0)));
        assert_eq!(auth_grace(r#"{"token":"t","grace":600}"#), Ok(Some(600)));
        assert_eq!(auth_grace(r#"{"token":"t","grace":3600}"#), Ok(Some(3600)));
        assert_eq!(
            auth_grace(
                r#"{"token":"t","resume":{"session":"s","secret":"x","received":1},"grace":120}"#
            ),
            Ok(Some(120))
        );
        // Not valid JSON: authentication fails anyway, so no verdict here.
        assert_eq!(auth_grace(r#"{"token":"t","grace":"#), Ok(None));
        for bad in [
            r#"{"token":"t","grace":3601}"#,
            r#"{"token":"t","grace":-1}"#,
            r#"{"token":"t","grace":60.0}"#,
            r#"{"token":"t","grace":60.5}"#,
            r#"{"token":"t","grace":6e2}"#,
            r#"{"token":"t","grace":"600"}"#,
            r#"{"token":"t","grace":null}"#,
            r#"{"token":"t","grace":true}"#,
            r#"{"token":"t","grace":[600]}"#,
            r#"{"token":"t","grace":{"s":600}}"#,
            r#"{"token":"t","grace":18446744073709551615}"#,
            r#"{"token":"t","grace":99999999999999999999999}"#,
        ] {
            assert_eq!(auth_grace(bad), Err(()), "{bad}");
        }
    }

    /// Protocol 2 or above, as a whole JSON number; anything else is an old (or broken)
    /// client, and what it gave is reported as a whole number or only its type.
    #[test]
    fn sign_in_protocol_is_a_whole_number_of_at_least_two() {
        assert_eq!(auth_protocol(r#"{"token":"t","protocol":2}"#), Ok(2));
        assert_eq!(auth_protocol(r#"{"token":"t","protocol":3}"#), Ok(3));
        assert_eq!(
            auth_protocol(r#"{"token":"t","protocol":18446744073709551615}"#),
            Ok(u64::MAX)
        );
        assert_eq!(
            auth_protocol(
                r#"{"token":"t","resume":{"session":"s","secret":"x","received":1},"protocol":2}"#
            ),
            Ok(2)
        );
        for (bad, given) in [
            (r#"{"token":"t"}"#, "none"),
            ("bare-token", "none"),
            ("", "none"),
            (r#"{"token":"t","protocol":2"#, "none"),
            (r#"{"token":"t","protocol":1}"#, "1"),
            (r#"{"token":"t","protocol":0}"#, "0"),
            (r#"{"token":"t","protocol":-1}"#, "-1"),
            (
                r#"{"token":"t","protocol":-9223372036854775808}"#,
                "-9223372036854775808",
            ),
            (r#"{"token":"t","protocol":2.0}"#, "float"),
            (r#"{"token":"t","protocol":2e0}"#, "float"),
            (r#"{"token":"t","protocol":-0.5}"#, "float"),
            (r#"{"token":"t","protocol":"2"}"#, "string"),
            (r#"{"token":"t","protocol":true}"#, "bool"),
            (r#"{"token":"t","protocol":null}"#, "null"),
            (r#"{"token":"t","protocol":[2]}"#, "array"),
            (r#"{"token":"t","protocol":{}}"#, "object"),
            (r#"{"token":"t","protocol":99999999999999999999}"#, "float"),
        ] {
            assert_eq!(auth_protocol(bad), Err(given.to_string()), "{bad}");
        }
        let big = format!(
            r#"{{"token":"{}","protocol":2}}"#,
            "t".repeat(MAX_AUTH_MESSAGE)
        );
        assert_eq!(auth_protocol(&big), Err("none".into()));
        // What a refused client is recorded as speaking: 1 for none, a small whole number
        // as it is, and 0 for anything else (still shown as needing an update).
        assert_eq!(old_protocol("none"), 1);
        assert_eq!(old_protocol("0"), 0);
        assert_eq!(old_protocol("1"), 1);
        for junk in [
            "-1", "float", "string", "bool", "null", "array", "object", "",
        ] {
            assert_eq!(old_protocol(junk), 0, "{junk}");
        }
    }

    fn json_form(protocol: serde_json::Value) -> String {
        serde_json::json!({"token": "t", "protocol": protocol}).to_string()
    }

    /// Security (log injection): what a refused client sent never reaches the log or the
    /// audit trail as text. Bidi overrides, line and paragraph separators, control
    /// characters and long strings all come out as a type tag; nothing but ASCII from a
    /// fixed set (or a whole number) ever does.
    #[test]
    fn a_refused_protocol_is_recorded_as_ascii_only() {
        let long = "x".repeat(900);
        let hostile = [
            "\u{202e}gnissap",
            "line\u{2028}break",
            "para\u{2029}graph",
            "\u{200f}\u{2066}x\u{2069}",
            "ctl\n\r\t\u{7}\u{1b}[31m",
            long.as_str(),
        ];
        for h in hostile {
            for (form, tag) in [
                (json_form(serde_json::json!(h)), "string"),
                (json_form(serde_json::json!([h])), "array"),
                (json_form(serde_json::json!({ h: h })), "object"),
            ] {
                if form.len() > MAX_AUTH_MESSAGE {
                    continue;
                }
                assert_eq!(auth_protocol(&form), Err(tag.to_string()), "{form}");
            }
        }
        // A 1 kB string: too long a sign-in form, so `none`; still nothing of it.
        let kb = json_form(serde_json::json!("\u{202e}".repeat(342)));
        assert!(kb.len() > 1024);
        assert_eq!(auth_protocol(&kb), Err("none".into()));
        let kb = json_form(serde_json::json!("y".repeat(980)));
        assert_eq!(auth_protocol(&kb), Err("string".into()));
        // Escaped in the JSON itself: the same.
        assert_eq!(
            auth_protocol("{\"token\":\"t\",\"protocol\":\"\\u202e\\u2028\\u2029\"}"),
            Err("string".into())
        );
        // Any number is short and plain.
        for n in ["-1", "1", "0", "-9223372036854775808", "1e308", "-1e-308"] {
            let given = auth_protocol(&format!(r#"{{"token":"t","protocol":{n}}}"#)).unwrap_err();
            assert!(
                given
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && given.len() <= 20,
                "{given}"
            );
        }
    }

    /// Security (audit flooding): a refused device is audited and warned about once an
    /// hour, whatever it sends meanwhile; another device gets its own; once the hour is
    /// up it is audited again. A change of what it speaks is recorded, unaudited.
    #[test]
    fn a_refused_device_is_noted_once_an_hour() {
        let both = TooOldNote {
            audit: true,
            record: true,
        };
        let neither = TooOldNote {
            audit: false,
            record: false,
        };
        let seen = TooOldSeen::default();
        let t0 = Instant::now();
        assert_eq!(seen.note(7, 1, t0), both);
        for i in 1..10u64 {
            assert_eq!(seen.note(7, 1, t0 + Duration::from_secs(i * 5)), neither);
        }
        // Another device: its own note.
        assert_eq!(seen.note(8, 1, t0 + Duration::from_secs(10)), both);
        // A change of protocol within the hour: recorded, not audited.
        assert_eq!(
            seen.note(7, 0, t0 + Duration::from_secs(60)),
            TooOldNote {
                audit: false,
                record: true
            }
        );
        // After the window: audited again, and then not for another hour.
        let later = t0 + TOO_OLD_NOTE_EVERY;
        assert_eq!(seen.note(7, 0, later), both);
        assert_eq!(seen.note(7, 0, later), neither);
        // A current attach forgets it: the next refusal is news.
        seen.forget(7);
        assert_eq!(seen.note(7, 0, later), both);
        // A clock that went back is never a reason to audit again.
        assert_eq!(seen.note(7, 0, t0), neither);
    }

    /// The map stays bounded however many devices are refused, and expired entries are
    /// what goes first.
    #[test]
    fn the_refused_devices_map_is_bounded() {
        let seen = TooOldSeen::default();
        let t0 = Instant::now();
        for d in 0..(TOO_OLD_SEEN_CAP as i64 * 3) {
            assert!(seen.note(d, 1, t0).audit);
            assert!(seen.len() <= TOO_OLD_SEEN_CAP);
        }
        let seen = TooOldSeen::default();
        for d in 0..(TOO_OLD_SEEN_CAP as i64 - 1) {
            seen.note(d, 1, t0);
        }
        let later = t0 + TOO_OLD_NOTE_EVERY;
        seen.note(-1, 1, later);
        seen.note(-2, 1, later);
        assert_eq!(seen.len(), 2);
        assert!(!seen.note(-1, 1, later).audit);
    }

    #[test]
    fn sign_in_level_defaults_to_original() {
        assert_eq!(
            auth_quality(r#"{"token":"t","quality":"low"}"#),
            Quality::Low
        );
        assert_eq!(
            auth_quality(r#"{"token":"t","quality":"high"}"#),
            Quality::High
        );
        for text in [
            r#"{"token":"t"}"#,
            r#"{"token":"t","quality":"ultra"}"#,
            r#"{"token":"t","quality":5}"#,
            "a-bare-token",
            "",
        ] {
            assert_eq!(auth_quality(text), Quality::Original, "{text}");
        }
        let long = format!(
            r#"{{"token":"{}","quality":"low"}}"#,
            "t".repeat(MAX_AUTH_MESSAGE)
        );
        assert_eq!(auth_quality(&long), Quality::Original);
    }
}
