//! `/v1/player`: authenticate a socket and attach it to a new or resumed session.

use std::sync::Arc;
use std::time::Duration;

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
    let (auth, quality, grace, text_keepalive) = match first {
        Ok(Some(Ok(Message::Text(t)))) => (
            parse_auth(&gw, t.as_str()),
            auth_quality(t.as_str()),
            auth_grace(t.as_str()),
            auth_keepalive(t.as_str()),
        ),
        _ => (None, Default::default(), Ok(None), false),
    };
    let Some(auth) = auth else {
        warn!("player authentication failed");
        return close(ws, close_code::POLICY, "unauthorised").await;
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
    // Off the async workers (M7): a stalled disk must not hold up other sessions.
    let device = session.device.device_id;
    let seen = gw.accounts.claim_seen(device);
    gw.db(move |a| {
        a.touch_device(device);
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
                    .map(|f| (u16::from(f.code), f.reason.to_string()))
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
