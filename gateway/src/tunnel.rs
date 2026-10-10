//! `/v1/player`: authenticate a socket and attach it to a new or resumed session.

use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{info, warn};

use anywhere_core::proto::ClientFrameReader;

use crate::accounts::Principal;
use crate::session::{AttachError, ClientInput, EndReason, Outbound, Session};
use crate::{AppState, origin_allowed};

/// Bound on a single WebSocket message from a client.
const MAX_WS_MESSAGE: usize = ClientFrameReader::MAX_FRAME + 1024;
const MAX_AUTH_MESSAGE: usize = 1024;
const MAX_CONTROL_MESSAGE: usize = 256;

/// The only text message allowed after authentication:
/// `{"ack":{"stream":n,"bytes":b}}`: bytes of stream `n` the client has received.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlMessage {
    ack: StreamAck,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamAck {
    stream: u64,
    bytes: u64,
}

pub async fn player_upgrade(
    State(gw): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&gw.cfg, &headers) {
        warn!("refused cross-origin websocket");
        return (StatusCode::FORBIDDEN, "origin not allowed\n").into_response();
    }
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| handle(gw, socket))
}

#[derive(Deserialize)]
struct AuthMessage {
    token: String,
    resume: Option<ResumeRequest>,
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

async fn handle(gw: AppState, mut ws: WebSocket) {
    let first = tokio::time::timeout(gw.cfg.auth_timeout, ws.recv()).await;
    let auth = match first {
        Ok(Some(Ok(Message::Text(t)))) => parse_auth(&gw, t.as_str()),
        _ => None,
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

    let (generation, rx) = match session.attach(received) {
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
    let greeting = serde_json::json!({
        "session": session.id,
        "secret": session.secret(),
        "mac": session.device.mac_string(),
        "name": session.device.device_name,
        "resumed": resumed,
    })
    .to_string();
    if ws.send(Message::Text(greeting.into())).await.is_err() {
        session.detach(&gw, generation);
        return;
    }
    gw.accounts.touch_device(session.device.device_id);
    info!(device = %session.device.device_name, user = %session.device.username, mac = %session.device.mac_string(), resumed, "player attached");

    let (sink, stream) = ws.split();
    let writer = tokio::spawn(write_loop(sink, rx));
    let outcome = read_loop(&session, stream).await;
    match outcome {
        ReadEnd::Policy(why) => {
            warn!(mac = %session.device.mac_string(), reason = %why, "player closed for policy violation");
            // A client that breaks protocol loses its whole session, not just this socket.
            session.end(&gw, EndReason::Policy).await;
        }
        ReadEnd::LmsGone => session.end(&gw, EndReason::LmsClosed).await,
        ReadEnd::SocketGone => session.detach(&gw, generation),
    }
    // The writer ends once its queue is closed or after sending a close frame.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
}

enum ReadEnd {
    Policy(String),
    LmsGone,
    SocketGone,
}

async fn read_loop(
    session: &Arc<Session>,
    mut stream: futures_util::stream::SplitStream<WebSocket>,
) -> ReadEnd {
    while let Some(msg) = stream.next().await {
        let data = match msg {
            Ok(Message::Binary(b)) => b,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Text(t)) => {
                let ack = (t.len() <= MAX_CONTROL_MESSAGE)
                    .then(|| serde_json::from_str::<ControlMessage>(t.as_str()).ok())
                    .flatten();
                match ack {
                    Some(m) => {
                        session.ack(m.ack.stream, m.ack.bytes).await;
                        continue;
                    }
                    None => return ReadEnd::Policy("unexpected text message".into()),
                }
            }
            Ok(Message::Close(_)) | Err(_) => return ReadEnd::SocketGone,
        };
        if session.is_ended() {
            return ReadEnd::SocketGone;
        }
        match session.client_bytes(&data).await {
            ClientInput::Ok => {}
            ClientInput::Policy(why) => return ReadEnd::Policy(why),
            ClientInput::LmsGone => return ReadEnd::LmsGone,
        }
    }
    ReadEnd::SocketGone
}

async fn write_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<Outbound>,
) {
    while let Some(out) = rx.recv().await {
        match out {
            Outbound::Frame(f) => {
                if sink.send(Message::Binary(f.into())).await.is_err() {
                    return;
                }
            }
            Outbound::Close(code, reason) => {
                let _ = sink
                    .send(Message::Close(Some(CloseFrame {
                        code,
                        reason: reason.into(),
                    })))
                    .await;
                return;
            }
        }
    }
}
