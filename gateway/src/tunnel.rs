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
    let (auth, quality) = match first {
        Ok(Some(Ok(Message::Text(t)))) => (parse_auth(&gw, t.as_str()), auth_quality(t.as_str())),
        _ => (None, Default::default()),
    };
    let Some(auth) = auth else {
        warn!("player authentication failed");
        return close(ws, close_code::POLICY, "unauthorised").await;
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

    session.set_quality(quality);
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
    })
    .to_string();
    if ws.send(Message::Text(greeting.into())).await.is_err() {
        session.detach(&gw, generation);
        return;
    }
    gw.accounts.touch_device(session.device.device_id);
    gw.accounts.note_seen(session.device.device_id, addr);
    info!(device = %session.name(), user = %session.device.username, mac = %session.device.mac_string(), resumed, "player attached");

    let (sink, stream) = ws.split();
    let mut writer = tokio::spawn(write_loop(sink, attached.rx, attached.kick.clone()));
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
        if msg.is_ok() {
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
            Ok(Message::Close(_)) | Err(_) => return ReadEnd::SocketGone,
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
    Done,
}

async fn write_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<Outbound>,
    mut kick: watch::Receiver<Kick>,
) {
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
        };
        let (code, reason) = match step {
            WriteStep::Done => return,
            WriteStep::Kicked(k) => k,
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
