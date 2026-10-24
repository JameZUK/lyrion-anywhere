//! The client side of the gateway's player protocol (docs/PROTOCOL.md, "Player socket",
//! "Resume" and "Streams"): pure functions, so every rule is unit-tested.
//!
//! Everything the gateway sends is treated as untrusted: the greeting is parsed strictly
//! (bounded size, exact field shapes) because its values end up in URLs and in HELO.

use super::StreamQuality;
use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Value, json};

/// Largest greeting accepted (the real one is about 150 bytes).
pub const MAX_GREETING: usize = 4 * 1024;
/// Largest binary message accepted from the gateway: one Slimproto frame (the core's
/// reader refuses frames over 32 KiB itself).
pub const MAX_FRAME_MESSAGE: usize = 34 * 1024;
/// Give up resuming slightly before the gateway's default 60 s grace runs out (an older
/// gateway, or one that didn't say what it granted).
pub const RESUME_FOR: Duration = Duration::from_secs(55);
/// Stop resuming this long before the granted grace runs out.
pub const RESUME_MARGIN: Duration = Duration::from_secs(5);
/// The resume grace asked for (seconds), from a gateway that offers it
/// (`features.resume_grace`). The gateway grants at most its own maximum (600 s by default).
pub const GRACE_ASK: u64 = 600;
/// The most a greeting's `grace` may say (the gateway's own limit on requests).
pub const MAX_GRACE: u64 = 3600;
/// Acknowledge stream bytes at least this often; the gateway stays within its replay
/// window (4 MiB by default) of the last acknowledgement.
pub const ACK_EVERY: u64 = 256 * 1024;
/// Messages for LMS held while disconnected; the oldest are dropped beyond this.
pub const OUTBOX_LIMIT: usize = 2000;
/// Without a grant from the gateway (60 s), how long a failing stream download keeps
/// retrying before the track is reported as failed.
pub const STREAM_RETRY_DEFAULT: Duration = Duration::from_secs(60);

/// The gateway's reply to a new or resumed player session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
    /// 32 lowercase hex characters.
    pub session: String,
    /// 32 lowercase hex characters; a credential (never logged).
    pub secret: String,
    /// The player identity this device must use in HELO.
    pub mac: [u8; 6],
    /// The player's name in LMS (cleaned).
    pub name: String,
    pub resumed: bool,
    /// The resume grace granted, in seconds (`None`: an older gateway, which holds 60 s).
    pub grace: Option<u64>,
}

/// How long to keep resuming a dropped session, from the grace the greeting granted:
/// [`RESUME_MARGIN`] less than the grant, or [`RESUME_FOR`] without one.
pub fn resume_for(grace: Option<u64>) -> Duration {
    match grace {
        Some(g) => Duration::from_secs(g.min(MAX_GRACE)).saturating_sub(RESUME_MARGIN),
        None => RESUME_FOR,
    }
}

/// How long a failing stream download keeps retrying: as long as the gateway holds the
/// session (the grace granted), since the stream can resume until then.
pub fn stream_retry_budget(grace: Option<u64>) -> Duration {
    match grace {
        Some(g) => Duration::from_secs(g.min(MAX_GRACE)).max(Duration::from_secs(5)),
        None => STREAM_RETRY_DEFAULT,
    }
}

/// Whether a stream download that has been failing for `failing_for` may try again.
pub fn stream_retry_allowed(failing_for: Duration, budget: Duration) -> bool {
    failing_for < budget
}

/// What `GET /v1/me` says the gateway offers (only what the player needs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    /// A session may change its stream level.
    pub stream_quality: bool,
    /// The player socket's sign-in takes `grace`.
    pub resume_grace: bool,
}

/// The features from `/v1/me` (anything but `true` is off).
pub fn parse_features(me: &Value) -> Features {
    let on = |k: &str| me["features"][k].as_bool() == Some(true);
    Features {
        stream_quality: on("stream_quality"),
        resume_grace: on("resume_grace"),
    }
}

/// Hold `frame` for LMS until the session resumes, within `limit` frames (when full, the
/// oldest goes). A heartbeat (`STMt`) is never held: once stale it is worse than useless.
/// The gateway reports for the player while it is away, and the core sends a fresh one
/// within a second of the link returning; a backlog of held ones, delivered at once with
/// jiffies a second apart, makes LMS re-base its clock for the player on each and throws
/// a sync group out (live, 2026-10-15). The events that matter (`STMs`, `STMd`, `STMo`,
/// `STMr`, `STMu`…) are all kept.
pub fn push_outbox<F: AsRef<[u8]>>(outbox: &mut VecDeque<F>, frame: F, limit: usize) {
    if is_heartbeat(frame.as_ref()) {
        return;
    }
    outbox.push_back(frame);
    while outbox.len() > limit {
        outbox.pop_front();
    }
}

/// A `STAT` frame for `STMt` (opcode, length, then the event code).
pub fn is_heartbeat(frame: &[u8]) -> bool {
    is_stat_event(frame, b"STMt")
}

/// A `STAT` frame for `event` (opcode, length, then the event code).
pub fn is_stat_event(frame: &[u8], event: &[u8; 4]) -> bool {
    frame.len() >= 12 && &frame[..4] == b"STAT" && &frame[8..12] == event
}

/// A frame for LMS held while the link is down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub frame: Vec<u8>,
    /// An `STMo` for an underrun mid-track (the track still arriving: the gap starved the
    /// output), as opposed to the one sent in place of `STMu` when everything received has
    /// played out (see [`release_outbox`]).
    pub gap_underrun: bool,
}

impl AsRef<[u8]> for Held {
    fn as_ref(&self) -> &[u8] {
        &self.frame
    }
}

/// What [`release_outbox`] gives back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Released {
    /// The frames to send LMS, oldest first.
    pub frames: Vec<Vec<u8>>,
    /// The output is still stalled from an underrun whose `STMo` was dropped: the player
    /// must not report its recovery either (`Player::withdraw_underrun_report`).
    pub stall_unreported: bool,
}

/// Empty the outbox when the session resumes after a gap, for sending. Everything is sent
/// in order, except any heartbeat (`STMt`, never held by [`push_outbox`]; stale whatever
/// put it there), and in a sync group (`grouped`, as last known), where a mid-track `STMo`
/// held through the gap is dropped, with the `STMr` that reported its recovery, if one
/// followed it.
///
/// Why: that `STMo` said "I have run out" at the time, but LMS only hears it now, after the
/// others have played on through the gap. While LMS is still streaming the track, a
/// member's `STMo` makes it pause the whole group and wait for every member to rebuffer
/// (4 to 15 s of silence on the other speakers in the live check), then restart them at
/// the stalled member's own stale position. A rejoining member already handles its stall
/// itself: it skips what the others played meanwhile, or has LMS restart the group at the
/// group's position ([`rejoin_command`]). So that report no longer describes anything LMS
/// should act on. Everything else is kept:
/// - alone (not grouped), LMS rebuffering the one player is harmless and expected;
/// - the `STMo` sent in place of `STMu` (everything received played out with the link
///   down: not `gap_underrun`) is about the end of the track, which LMS must hear;
/// - frames sent while the link is up never wait here, so an underrun after the rejoin is
///   reported as usual.
pub fn release_outbox(outbox: &mut VecDeque<Held>, grouped: bool) -> Released {
    let mut out = Released {
        frames: Vec::with_capacity(outbox.len()),
        stall_unreported: false,
    };
    // A dropped `STMo` whose recovery (`STMr`) is still to come.
    let mut drop_resume = false;
    for h in outbox.drain(..) {
        if is_heartbeat(&h.frame) {
            continue;
        }
        // Only ever an `STMo`, whatever the tag says.
        if grouped && h.gap_underrun && is_stat_event(&h.frame, b"STMo") {
            drop_resume = true;
            out.stall_unreported = true;
            continue;
        }
        if is_stat_event(&h.frame, b"STMo") {
            drop_resume = false;
            out.stall_unreported = false;
        } else if is_stat_event(&h.frame, b"STMr") {
            out.stall_unreported = false;
            if std::mem::take(&mut drop_resume) {
                continue;
            }
        }
        out.frames.push(h.frame);
    }
    out
}

fn hex32(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    (s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| s.to_string())
}

/// Invisible formatting characters that can disguise text (bidi overrides, zero-width).
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2069}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
}

/// A display name: no control or invisible formatting characters, at most 64 characters.
pub fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .take(64)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Parse the greeting. `None` for anything unexpected (the connection is then dropped).
pub fn parse_greeting(text: &str) -> Option<Greeting> {
    if text.len() > MAX_GREETING {
        return None;
    }
    let v: Value = serde_json::from_str(text).ok()?;
    let o = v.as_object()?;
    let mac = anywhere_core::proto::parse_mac(o.get("mac")?.as_str()?)?;
    // A multicast or all-zero MAC is not a player identity the gateway would assign.
    if mac[0] & 1 == 1 || mac == [0; 6] {
        return None;
    }
    let name = clean_name(o.get("name")?.as_str()?);
    Some(Greeting {
        session: hex32(o.get("session")?)?,
        secret: hex32(o.get("secret")?)?,
        mac,
        name: if name.trim().is_empty() {
            "Phone".into()
        } else {
            name
        },
        resumed: o.get("resumed")?.as_bool()?,
        // Bounded and exact, or treated as absent (an older gateway's 60 s): a hostile
        // value can't make the player resume for longer than the protocol allows.
        grace: o
            .get("grace")
            .and_then(Value::as_u64)
            .filter(|g| *g <= MAX_GRACE),
    })
}

/// The first message of a resumed session.
pub fn resume_message(token: &str, session: &str, secret: &str, received: u64) -> String {
    sign_in_message(
        token,
        Some((session, secret, received)),
        StreamQuality::Original,
    )
}

/// The first message on the player socket: the token, the session to resume if any, and
/// the stream level wanted (docs/plans/streaming-quality.md). A bare token for a new
/// session at the original level, as before; gateways ignore a level they don't know.
pub fn sign_in_message(
    token: &str,
    resume: Option<(&str, &str, u64)>,
    quality: StreamQuality,
) -> String {
    sign_in_message_with_grace(token, resume, quality, None)
}

/// [`sign_in_message`], asking for a resume grace of `grace` seconds. Only for a gateway
/// that lists `resume_grace` among its features (it is never sent where it isn't
/// understood). Capped at [`MAX_GRACE`].
pub fn sign_in_message_with_grace(
    token: &str,
    resume: Option<(&str, &str, u64)>,
    quality: StreamQuality,
    grace: Option<u64>,
) -> String {
    if resume.is_none() && quality == StreamQuality::Original && grace.is_none() {
        return token.to_string();
    }
    let mut v = json!({ "token": token });
    if let Some((session, secret, received)) = resume {
        v["resume"] = json!({ "session": session, "secret": secret, "received": received });
    }
    if quality != StreamQuality::Original {
        v["quality"] = json!(quality.as_str());
    }
    if let Some(g) = grace {
        v["grace"] = json!(g.min(MAX_GRACE));
    }
    v.to_string()
}

/// A change of stream level for the streams that follow (only to a gateway that lists
/// `stream_quality` among its features: older ones end the session on it).
pub fn quality_message(quality: StreamQuality) -> String {
    json!({ "quality": quality.as_str() }).to_string()
}

/// A stream acknowledgement.
pub fn ack_message(stream: u64, bytes: u64) -> String {
    json!({ "ack": { "stream": stream, "bytes": bytes } }).to_string()
}

/// What to do after the player socket closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Don't reconnect: refused, a protocol error, or replaced by another connection.
    Stop,
    /// The access token expired: renew it, then reconnect (resuming if possible).
    Reauthenticate,
    /// The session is gone: connect afresh (new HELO).
    NewSession,
    /// Reconnect and resume the session.
    Resume,
}

/// Close codes (docs/PROTOCOL.md, "Close codes"). `None`: no close frame (connection lost).
pub fn close_action(code: Option<u16>, reason: &str) -> CloseAction {
    match code {
        Some(1008) if reason == "unauthorised" => CloseAction::Reauthenticate,
        Some(1008) | Some(4000) => CloseAction::Stop,
        Some(4404) | Some(1001) => CloseAction::NewSession,
        _ => CloseAction::Resume,
    }
}

/// After a failed stream fetch with HTTP `status` (0 for a network error): retry from
/// the current offset, or fail the track.
pub fn stream_should_retry(status: u16) -> bool {
    status == 0 || status >= 500
}

/// Retries of a failing request that confirms a clean stream end before that end is taken
/// as the track's end after all (see [`confirm_gives_up`]).
pub const CONFIRM_RETRIES: u32 = 3;

/// What to do when a stream response ends cleanly after bringing `brought` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanEnd {
    /// It is the end of the track.
    End,
    /// Maybe not: ask again from the offset reached (`?offset=`) before believing it.
    Confirm,
}

/// The gateway ends a response cleanly only at the true end of the track, and with an
/// error otherwise (for instance after a long wait for acknowledgements). A proxy on the
/// way can pass that error on as a clean end (issue #12, through a Cloudflare Tunnel),
/// which would look like the end of the track and skip the rest of it. At the true end the
/// gateway answers a request from the offset reached at once, with an empty body;
/// otherwise the rest of the audio follows. Only a response that brought nothing is taken
/// as the end: a track costs one extra, empty request at most, and asking again always
/// needs progress, so this never loops.
pub fn clean_end_action(brought: u64) -> CleanEnd {
    if brought > 0 {
        CleanEnd::Confirm
    } else {
        CleanEnd::End
    }
}

/// The request confirming a clean end failed before bringing anything (HTTP `status`, 0
/// for a network error), after `retries` retries of it: whether to take the earlier clean
/// end as the track's end, as before confirmations, rather than retry or fail the track.
/// A refusal (anything [`stream_should_retry`] would not retry) gives up at once.
pub fn confirm_gives_up(status: u16, retries: u32) -> bool {
    !stream_should_retry(status) || retries >= CONFIRM_RETRIES
}

/// Backoff with jitter: about 0.5, 1, 2 and 4 s, then every 5 s, ±20 %. `random` is in
/// 0..=1 (NaN counts as 0.5).
pub fn backoff(attempt: u32, random: f64) -> Duration {
    let random = if random.is_nan() {
        0.5
    } else {
        random.clamp(0.0, 1.0)
    };
    let base = (500f64 * 2f64.powi(attempt.min(4) as i32)).min(5000.0);
    Duration::from_millis((base * (0.8 + 0.4 * random)) as u64)
}

pub fn random01() -> f64 {
    getrandom::u32().map_or(0.5, |r| f64::from(r) / f64::from(u32::MAX))
}

/// The LMS status query that names the track a new stream is for (see
/// [`stream_track_id`]).
pub fn stream_status_command() -> Vec<Value> {
    // x: remote (a stream, not a library file); e: album id; d: duration.
    vec![json!("status"), json!("-"), json!(2), json!("tags:xed")]
}

/// Whether this player's LMS `status` says it is in a sync group (`sync_master` or
/// `sync_slaves`, which LMS adds only for grouped players). Anything but a non-empty, short
/// string counts as not grouped.
pub fn in_sync_group(status: &Value) -> bool {
    let named = |k: &str| {
        status
            .get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty() && s.len() <= 4096)
    };
    named("sync_master") || named("sync_slaves")
}

/// The group's playing position (seconds) from LMS's `time ?` answer (`_time`), if sane.
pub fn group_time(answer: &Value) -> Option<f64> {
    let t = match answer.get("_time")? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if s.len() <= 20 => s.parse::<f64>().ok()?,
        _ => return None,
    };
    (t.is_finite() && (0.0..1e7).contains(&t)).then_some(t)
}

/// A library track LMS is streaming, as [`stream_track`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamTrack {
    pub id: i64,
    pub album_id: Option<i64>,
    pub duration_s: Option<f64>,
}

/// Which library track a stream LMS just started is for, from this player's `status - 2`
/// (the playing track and the one after it, in play order).
///
/// - A **fresh** stream (nothing audible: a new play, a skip, a seek) is the current
///   track.
/// - Otherwise it is the gapless **next** track: the one after the current, or the same
///   one again when repeating a single track.
///
/// The answer is untrusted: the item's playlist position must be the one expected, and its
/// id a positive integer. Anything doubtful (a wrap to the start of the playlist, a remote
/// stream, a malformed answer) gives `None`, and levelling falls back to tags.
pub fn stream_track_id(status: &Value, fresh: bool) -> Option<i64> {
    stream_track(status, fresh).map(|t| t.id)
}

/// [`stream_track_id`], with the track's album and duration where LMS gave sane ones.
pub fn stream_track(status: &Value, fresh: bool) -> Option<StreamTrack> {
    let int = |v: &Value| -> Option<i64> {
        match v {
            Value::Number(n) => n.as_i64(),
            Value::String(s) if (1..=15).contains(&s.len()) => s.parse().ok(),
            _ => None,
        }
    };
    let repeat_one = status.get("playlist repeat").and_then(int) == Some(1);
    let offset = if fresh || repeat_one { 0 } else { 1 };
    let item = status
        .get("playlist_loop")?
        .as_array()?
        .get(offset as usize)?;
    if let (Some(cur), Some(at)) = (
        status.get("playlist_cur_index").and_then(int),
        item.get("playlist index").and_then(int),
    ) && cur.checked_add(offset) != Some(at)
    {
        return None;
    }
    if item.get("remote").and_then(int).unwrap_or(0) != 0 {
        return None;
    }
    let id_ok = |id: &i64| (1..=999_999_999_999).contains(id);
    let id = item.get("id").and_then(int).filter(id_ok)?;
    let duration_s = match item.get("duration") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if s.len() <= 20 => s.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|d| d.is_finite() && *d > 0.0 && *d < 1e7);
    Some(StreamTrack {
        id,
        album_id: item.get("album_id").and_then(int).filter(id_ok),
        duration_s,
    })
}

// --- Restarting after a lost session (rolling cache, owner's decision 3) -----------------

/// Where a stream LMS started sits in this player's queue: its playlist position and
/// library track, and (for a fresh stream) where in the track LMS says it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamPosition {
    pub index: u32,
    pub track_id: i64,
    /// LMS's `time` (ms) when asked; for a fresh stream after a seek, where it began.
    pub lms_time_ms: Option<u64>,
}

/// [`stream_track`] with the item's playlist position, which must be given and be the one
/// expected (the current index, or the one after it for a gapless next track). Untrusted:
/// anything doubtful gives `None`, and the player then never restarts that track itself.
pub fn stream_position(status: &Value, fresh: bool) -> Option<StreamPosition> {
    let track = stream_track(status, fresh)?;
    let repeat_one = status.get("playlist repeat").and_then(small_int) == Some(1);
    let offset = usize::from(!(fresh || repeat_one));
    let item = status.get("playlist_loop")?.as_array()?.get(offset)?;
    let index = item.get("playlist index").and_then(small_int)?;
    let cur = status.get("playlist_cur_index").and_then(small_int)?;
    if cur.checked_add(offset as u32) != Some(index) {
        return None;
    }
    Some(StreamPosition {
        index,
        track_id: track.id,
        lms_time_ms: if fresh {
            lms_seconds(status.get("time"))
        } else {
            None
        },
    })
}

/// A playlist index: a non-negative integer that fits LMS's queues (up to a million).
fn small_int(v: &Value) -> Option<u32> {
    let n = match v {
        Value::Number(n) => n.as_u64()?,
        Value::String(s) if (1..=7).contains(&s.len()) => s.parse().ok()?,
        _ => return None,
    };
    (n <= 1_000_000).then_some(n as u32)
}

/// Seconds from LMS (a number or a short numeric string) as ms, if sane.
fn lms_seconds(v: Option<&Value>) -> Option<u64> {
    let t = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if s.len() <= 20 => s.parse::<f64>().ok()?,
        _ => return None,
    };
    (t.is_finite() && (0.0..1e7).contains(&t)).then_some((t * 1000.0) as u64)
}

/// The longest stall a rejoin moves the group past: no resumed session lasts longer (the
/// most grace a gateway may grant). Anything longer is not a position to trust.
pub const MAX_REJOIN_STALL_MS: u64 = MAX_GRACE * 1000;
/// A rejoin that would land this close to the track's end (or past it) moves nothing.
pub const REJOIN_END_MARGIN_MS: u64 = 1_000;

/// Where a stalled member of a sync group stopped, for having LMS restart the group where
/// the others are now (see [`rejoin_command`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejoinPoint {
    /// The stalled track's playlist position and library id.
    pub index: u32,
    pub track_id: i64,
    /// How far into the track the output stopped (ms).
    pub stalled_at_ms: u64,
    /// How long it has been stalled, up to the moment the command is sent (ms).
    pub stalled_ms: u64,
}

/// The `time` command that restarts the group where the others are now: where this player
/// stalled plus the time it has been stalled, from LMS's answer to
/// [`stream_status_command`] (the current track). LMS's own `time ?` is no good for this:
/// with this player the group's master, it lags by however long LMS last heard from it
/// (13.7 to 13.9 s in the live check after a 14 s stall), so the whole group would be sent
/// back to replay what this player missed.
///
/// `None` (nothing is sent; LMS is left to it) when the point can't be vouched for:
/// - LMS isn't playing (someone paused or stopped the group during the gap: the others are
///   not where the time stalled says);
/// - LMS's current track is not the stalled one (the queue changed, someone skipped, or
///   the group moved on to the next track): a seek would land in the wrong track;
/// - the point is at or within [`REJOIN_END_MARGIN_MS`] of the track's end (the others
///   have finished it, or are about to): LMS moves the group on to the next track itself,
///   and a seek past the end would fight that;
/// - the stall is longer than [`MAX_REJOIN_STALL_MS`].
///
/// The answer is untrusted; anything malformed means no command.
pub fn rejoin_command(point: &RejoinPoint, status: &Value) -> Option<Vec<Value>> {
    if status.get("mode")?.as_str()? != "play" {
        return None;
    }
    if point.stalled_ms > MAX_REJOIN_STALL_MS {
        return None;
    }
    let pos = stream_position(status, true)?;
    if pos.index != point.index || pos.track_id != point.track_id {
        return None;
    }
    let target = point.stalled_at_ms.checked_add(point.stalled_ms)?;
    if let Some(d) = stream_track(status, true)?.duration_s {
        let end = (d * 1000.0) as u64;
        if target.saturating_add(REJOIN_END_MARGIN_MS) >= end {
            return None;
        }
    }
    let secs = target as f64 / 1000.0;
    Some(vec![json!("time"), json!(format!("{secs:.3}"))])
}

/// Where to restart playback if the session is lost: the track heard last, by playlist
/// position and library id, and how far into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPoint {
    pub index: u32,
    pub track_id: i64,
    pub elapsed_ms: u64,
    /// Paused there (by the user, on the phone, while the server couldn't be reached).
    pub paused: bool,
    /// The user played, paused, skipped or sought on the phone while the server couldn't be
    /// reached (offline controls): the phone's state is the truth, whatever LMS shows.
    pub local: bool,
}

/// What the player remembers while its link to LMS is down, for restarting on a new
/// session if the gap outlasts the grace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GapWatch {
    /// The last point heard (`None`: nothing known to restart).
    pub point: Option<RestartPoint>,
    /// No restart: playback wasn't going when the link went, or the user held it since.
    pub blocked: bool,
}

/// One step of watching a gap (on losing the link, then every tick while it is down):
/// - starting a watch, it is blocked unless the player is playing and not held (the user
///   had paused or stopped);
/// - the user holding the output during the gap blocks it for good;
/// - while playing on from what is buffered, the point follows what is heard (it may move
///   to the next track, if its position is known).
pub fn watch_gap(
    watch: Option<GapWatch>,
    playing: bool,
    held: bool,
    heard: Option<RestartPoint>,
) -> GapWatch {
    let mut w = watch.unwrap_or(GapWatch {
        point: None,
        blocked: !playing || held,
    });
    if held {
        w.blocked = true;
    }
    if !w.blocked && playing && heard.is_some() {
        w.point = heard;
    }
    w
}

/// Asks LMS about the remembered queue position before restarting there.
pub fn restart_status_command(index: u32) -> Vec<Value> {
    vec![json!("status"), json!(index), json!(1), json!("tags:x")]
}

/// The commands that restart the remembered track at the remembered position on a new
/// session, from LMS's answer to [`restart_status_command`]; `None` when it mustn't:
/// - LMS shows the player playing (it restarted itself) or paused (someone paused it);
/// - the queue changed: that position no longer holds that track, or LMS's current track
///   is neither it nor the one before it (the phone may have played on into the next
///   track from its buffer, which LMS never heard about).
///
/// A point the user made on the phone ([`RestartPoint::local`]) is the truth whatever LMS
/// is doing: only the queue must still hold that track at that position. It is also
/// paused there if it was paused on the phone.
///
/// The answer is untrusted; anything malformed means no restart.
pub fn restart_commands(point: &RestartPoint, status: &Value) -> Option<Vec<Vec<Value>>> {
    if !point.local {
        if status.get("mode")?.as_str()? != "stop" {
            return None;
        }
        let cur = status.get("playlist_cur_index").and_then(small_int)?;
        if cur != point.index && cur.checked_add(1) != Some(point.index) {
            return None;
        }
    }
    let item = status.get("playlist_loop")?.as_array()?.first()?;
    if item.get("playlist index").and_then(small_int)? != point.index {
        return None;
    }
    let id = match item.get("id")? {
        Value::Number(n) => n.as_i64()?,
        Value::String(s) if (1..=15).contains(&s.len()) => s.parse().ok()?,
        _ => return None,
    };
    if id != point.track_id || item.get("remote").and_then(small_int).unwrap_or(0) != 0 {
        return None;
    }
    let mut cmds = vec![vec![json!("playlist"), json!("index"), json!(point.index)]];
    if point.elapsed_ms >= 1000 {
        let secs = point.elapsed_ms as f64 / 1000.0;
        cmds.push(vec![json!("time"), json!(format!("{secs:.1}"))]);
    }
    if point.paused && point.local {
        cmds.push(pause_command(true));
    }
    Some(cmds)
}

/// LMS's pause (`true`) or play (`false`) for this player, without toggling.
pub fn pause_command(paused: bool) -> Vec<Value> {
    vec![json!("pause"), json!(if paused { 1 } else { 0 })]
}

/// The phone's state after an outage in which the user acted on it, told to LMS on a
/// resumed session (LMS never lost the player, so its queue position is unchanged): its
/// pause state only. `None`: nothing was done on the phone.
pub fn resumed_commands(acted: bool, paused: bool) -> Option<Vec<Vec<Value>>> {
    acted.then(|| vec![pause_command(paused)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clean_end_that_brought_nothing_is_the_end() {
        assert_eq!(clean_end_action(0), CleanEnd::End);
        assert_eq!(clean_end_action(1), CleanEnd::Confirm);
        assert_eq!(clean_end_action(u64::MAX), CleanEnd::Confirm);
    }

    #[test]
    fn a_confirmation_gives_up_on_a_refusal_or_after_its_retries() {
        // Refused (the gateway never answers these mid-stream): the end it was.
        for status in [400, 401, 403, 404, 410, 416, 499] {
            assert!(confirm_gives_up(status, 0), "{status}");
        }
        // A network error or a 5xx is retried, a few times only.
        for status in [0, 500, 502, 503, 599, u16::MAX] {
            for retries in 0..CONFIRM_RETRIES {
                assert!(!confirm_gives_up(status, retries), "{status} {retries}");
            }
            assert!(confirm_gives_up(status, CONFIRM_RETRIES), "{status}");
            assert!(confirm_gives_up(status, u32::MAX), "{status}");
        }
    }

    fn point() -> RestartPoint {
        RestartPoint {
            index: 3,
            track_id: 42,
            elapsed_ms: 83_450,
            paused: false,
            local: false,
        }
    }

    fn stopped_at(cur: u32, index: u32, id: i64) -> Value {
        json!({
            "mode": "stop", "playlist_cur_index": cur.to_string(),
            "playlist_loop": [{"playlist index": index, "id": id}]
        })
    }

    #[test]
    fn restart_plays_the_same_track_at_the_same_position() {
        let cmds = restart_commands(&point(), &stopped_at(3, 3, 42)).unwrap();
        assert_eq!(
            cmds,
            vec![
                vec![json!("playlist"), json!("index"), json!(3)],
                vec![json!("time"), json!("83.5")],
            ]
        );
        // The phone had played on into track 3 from its buffer; LMS still shows track 2.
        assert!(restart_commands(&point(), &stopped_at(2, 3, 42)).is_some());
        // At the very start, no seek.
        let start = RestartPoint {
            elapsed_ms: 400,
            ..point()
        };
        assert_eq!(
            restart_commands(&start, &stopped_at(3, 3, 42))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            restart_status_command(3),
            vec![json!("status"), json!(3), json!(1), json!("tags:x")]
        );
    }

    #[test]
    fn no_restart_when_the_queue_changed_or_lms_is_busy() {
        // Another track at that position (the queue was edited).
        assert_eq!(restart_commands(&point(), &stopped_at(3, 3, 43)), None);
        // LMS's current track moved elsewhere (someone skipped or played something else).
        assert_eq!(restart_commands(&point(), &stopped_at(7, 3, 42)), None);
        assert_eq!(restart_commands(&point(), &stopped_at(4, 3, 42)), None);
        // The answer is about another position.
        assert_eq!(restart_commands(&point(), &stopped_at(3, 4, 42)), None);
        // LMS already plays it, or someone paused it.
        for mode in ["play", "pause", "", "STOP"] {
            let mut st = stopped_at(3, 3, 42);
            st["mode"] = json!(mode);
            assert_eq!(restart_commands(&point(), &st), None, "{mode}");
        }
        // A radio stream at that position.
        let mut st = stopped_at(3, 3, 42);
        st["playlist_loop"][0]["remote"] = json!(1);
        assert_eq!(restart_commands(&point(), &st), None);
    }

    #[test]
    fn the_phones_own_point_wins_unless_the_queue_changed() {
        let local = RestartPoint {
            local: true,
            ..point()
        };
        let expected = vec![
            vec![json!("playlist"), json!("index"), json!(3)],
            vec![json!("time"), json!("83.5")],
        ];
        // Whatever LMS is doing, and wherever its current track is.
        for (mode, cur) in [("play", 7), ("pause", 0), ("stop", 3), ("stop", 9)] {
            let mut st = stopped_at(cur, 3, 42);
            st["mode"] = json!(mode);
            assert_eq!(
                restart_commands(&local, &st),
                Some(expected.clone()),
                "{mode} {cur}"
            );
        }
        // Paused on the phone: paused there too.
        let paused = RestartPoint {
            paused: true,
            ..local
        };
        let mut with_pause = expected.clone();
        with_pause.push(vec![json!("pause"), json!(1)]);
        assert_eq!(
            restart_commands(&paused, &stopped_at(0, 3, 42)),
            Some(with_pause)
        );
        // A pause that wasn't the user's on the phone is never sent.
        assert_eq!(
            restart_commands(
                &RestartPoint {
                    paused: true,
                    ..point()
                },
                &stopped_at(3, 3, 42)
            )
            .unwrap()
            .len(),
            2
        );
        // The queue changed (someone else edited it): the phone's state is dropped.
        assert_eq!(restart_commands(&local, &stopped_at(3, 3, 43)), None);
        assert_eq!(restart_commands(&local, &stopped_at(3, 4, 42)), None);
        let mut radio = stopped_at(3, 3, 42);
        radio["playlist_loop"][0]["remote"] = json!(1);
        assert_eq!(restart_commands(&local, &radio), None);
        // Malformed answers: nothing, and only ever the remembered position.
        for st in [
            json!(null),
            json!({"playlist_loop": []}),
            json!({"mode": "play"}),
        ] {
            assert_eq!(restart_commands(&local, &st), None, "{st}");
        }
    }

    #[test]
    fn a_resumed_session_is_told_the_phones_pause_state_only_if_the_user_acted() {
        assert_eq!(resumed_commands(false, true), None);
        assert_eq!(
            resumed_commands(true, true),
            Some(vec![vec![json!("pause"), json!(1)]])
        );
        assert_eq!(
            resumed_commands(true, false),
            Some(vec![vec![json!("pause"), json!(0)]])
        );
    }

    #[test]
    fn restart_answers_are_untrusted() {
        let parts = [
            json!(null),
            json!(-1),
            json!(3),
            json!("3"),
            json!("42"),
            json!(1e300),
            json!("x"),
            json!([]),
            json!({}),
            json!("99999999999999999999"),
        ];
        let p = point();
        let local = RestartPoint {
            local: true,
            paused: true,
            ..p
        };
        for a in &parts {
            for b in &parts {
                for c in &parts {
                    let st = json!({"mode": "stop", "playlist_cur_index": a,
                        "playlist_loop": [{"playlist index": b, "id": c}]});
                    if let Some(cmds) = restart_commands(&p, &st) {
                        // Only ever the remembered position.
                        assert_eq!(cmds[0][2], json!(3));
                    }
                    if let Some(cmds) = restart_commands(&local, &st) {
                        assert_eq!(cmds[0][2], json!(3));
                    }
                    let _ = stream_position(&st, true);
                    let _ = stream_position(&st, false);
                }
            }
        }
        for st in [
            json!(null),
            json!([]),
            json!({"mode": 1}),
            json!({"mode": "stop"}),
            json!({"mode": "stop", "playlist_cur_index": 3, "playlist_loop": "x"}),
            json!({"mode": "stop", "playlist_cur_index": 3, "playlist_loop": []}),
        ] {
            assert_eq!(restart_commands(&p, &st), None, "{st}");
        }
    }

    #[test]
    fn stream_positions_from_status() {
        let st = json!({
            "playlist_cur_index": "4", "time": "61.25",
            "playlist_loop": [
                {"playlist index": 4, "id": 101},
                {"playlist index": "5", "id": "102"}
            ]
        });
        assert_eq!(
            stream_position(&st, true),
            Some(StreamPosition {
                index: 4,
                track_id: 101,
                lms_time_ms: Some(61_250)
            })
        );
        assert_eq!(
            stream_position(&st, false),
            Some(StreamPosition {
                index: 5,
                track_id: 102,
                lms_time_ms: None
            })
        );
        // Without positions, nothing is known.
        let mut bare = st.clone();
        bare["playlist_loop"][0]
            .as_object_mut()
            .unwrap()
            .remove("playlist index");
        assert_eq!(stream_position(&bare, true), None);
        let mut no_cur = st.clone();
        no_cur.as_object_mut().unwrap().remove("playlist_cur_index");
        assert_eq!(stream_position(&no_cur, true), None);
        // A silly time is dropped, not the position.
        for t in [json!(-1), json!("NaN"), json!(1e300), json!([1])] {
            let mut odd = st.clone();
            odd["time"] = t;
            assert_eq!(stream_position(&odd, true).unwrap().lms_time_ms, None);
        }
    }

    #[test]
    fn a_gap_watch_follows_what_is_heard_unless_the_user_held_it() {
        let at = |ms| {
            Some(RestartPoint {
                elapsed_ms: ms,
                ..point()
            })
        };
        // Playing when the link went: the point follows what is heard.
        let w = watch_gap(None, true, false, at(10_000));
        assert!(!w.blocked);
        let w = watch_gap(Some(w), true, false, at(25_000));
        assert_eq!(w.point, at(25_000));
        // Ran dry: the last point heard stays.
        let w = watch_gap(Some(w), false, false, None);
        assert_eq!(w.point, at(25_000));
        assert!(!w.blocked);
        // Held by the user during the gap: never restarted.
        let held = watch_gap(Some(w), true, true, at(26_000));
        assert!(held.blocked);
        let after = watch_gap(Some(held), true, false, at(27_000));
        assert!(after.blocked);
        // Paused or stopped when the link went: never restarted.
        let stopped = watch_gap(None, false, false, at(1_000));
        assert!(stopped.blocked);
        assert_eq!(watch_gap(Some(stopped), true, false, at(2_000)).point, None);
        assert!(watch_gap(None, true, true, at(1_000)).blocked);
    }

    #[test]
    fn stream_track_ids_from_status() {
        let st = json!({
            "playlist_cur_index": "4", "playlist repeat": 0,
            "playlist_loop": [
                {"playlist index": 4, "id": 101},
                {"playlist index": 5, "id": "102"}
            ]
        });
        assert_eq!(stream_track_id(&st, true), Some(101));
        assert_eq!(stream_track_id(&st, false), Some(102));
        // Repeating one track: the next stream is the same track.
        let mut one = st.clone();
        one["playlist repeat"] = json!("1");
        assert_eq!(stream_track_id(&one, false), Some(101));
        // The last track: no next item (a wrap isn't guessed).
        let mut last = st.clone();
        last["playlist_loop"].as_array_mut().unwrap().pop();
        assert_eq!(stream_track_id(&last, false), None);
        // The position must match what was asked for.
        let mut moved = st.clone();
        moved["playlist_loop"][1]["playlist index"] = json!(9);
        assert_eq!(stream_track_id(&moved, false), None);
        // Streams and odd ids.
        for (k, v) in [
            ("remote", json!(1)),
            ("id", json!(-3)),
            ("id", json!(0)),
            ("id", json!("12abc")),
            ("id", json!(1.5)),
            ("id", json!(1_000_000_000_000i64)),
            ("id", json!(null)),
            ("id", json!({"x": 1})),
        ] {
            let mut bad = st.clone();
            bad["playlist_loop"][0][k] = v;
            assert_eq!(stream_track_id(&bad, true), None, "{k}");
        }
        assert_eq!(stream_track_id(&json!({}), true), None);
        assert_eq!(stream_track_id(&json!({"playlist_loop": "x"}), true), None);
        assert_eq!(stream_track_id(&json!([1, 2]), true), None);
    }

    #[test]
    fn stream_track_album_and_duration() {
        let st = json!({
            "playlist_cur_index": 0,
            "playlist_loop": [
                {"playlist index": 0, "id": 5, "album_id": "77", "duration": "201.5"},
                {"playlist index": 1, "id": 6, "album_id": 78, "duration": 180}
            ]
        });
        assert_eq!(
            stream_track(&st, true),
            Some(StreamTrack {
                id: 5,
                album_id: Some(77),
                duration_s: Some(201.5)
            })
        );
        assert_eq!(
            stream_track(&st, false),
            Some(StreamTrack {
                id: 6,
                album_id: Some(78),
                duration_s: Some(180.0)
            })
        );
        // Hostile album ids and durations are dropped, never the track.
        for (album, duration) in [
            (json!(-1), json!("NaN")),
            (json!("x"), json!(-5)),
            (json!(1e300), json!("inf")),
            (json!({"a": 1}), json!([1])),
            (json!(0), json!(1e300)),
        ] {
            let mut bad = st.clone();
            bad["playlist_loop"][0]["album_id"] = album;
            bad["playlist_loop"][0]["duration"] = duration;
            assert_eq!(
                stream_track(&bad, true),
                Some(StreamTrack {
                    id: 5,
                    album_id: None,
                    duration_s: None
                })
            );
        }
        assert!(stream_status_command().contains(&json!("tags:xed")));
    }

    #[test]
    fn stream_track_id_survives_arbitrary_json() {
        let parts = [
            json!(null),
            json!(1),
            json!(-1),
            json!("1"),
            json!("x"),
            json!([]),
            json!({}),
            json!(1e300),
            json!(u64::MAX),
            json!(i64::MAX),
            json!(i64::MIN),
        ];
        let mut seed = 0x2545_f491u64;
        for _ in 0..5_000 {
            let mut pick = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                parts[(seed % parts.len() as u64) as usize].clone()
            };
            let st = json!({
                "playlist_cur_index": pick(),
                "playlist repeat": pick(),
                "playlist_loop": [
                    {"playlist index": pick(), "id": pick(), "remote": pick()},
                    {"playlist index": pick(), "id": pick()},
                    pick()
                ]
            });
            for fresh in [true, false] {
                if let Some(id) = stream_track_id(&st, fresh) {
                    assert!(id > 0);
                }
            }
        }
    }

    const GOOD: &str = r#"{"session":"0123456789abcdef0123456789abcdef","secret":"fedcba9876543210fedcba9876543210","mac":"02:4c:41:00:00:01","name":"Test phone","resumed":false}"#;

    #[test]
    fn greeting_parses() {
        let g = parse_greeting(GOOD).unwrap();
        assert_eq!(g.session, "0123456789abcdef0123456789abcdef");
        assert_eq!(g.mac, [2, 0x4c, 0x41, 0, 0, 1]);
        assert_eq!(g.name, "Test phone");
        assert!(!g.resumed);
    }

    #[test]
    fn hostile_greetings_are_refused() {
        let base: Value = serde_json::from_str(GOOD).unwrap();
        let with = |k: &str, v: Value| {
            let mut b = base.clone();
            b[k] = v;
            b.to_string()
        };
        let without = |k: &str| {
            let mut b = base.clone();
            b.as_object_mut().unwrap().remove(k);
            b.to_string()
        };
        let bad = [
            // Path or URL injection through the session id (it goes into stream URLs).
            with("session", json!("../../v1/rpc?x=0123456789abcdef0123")),
            with("session", json!("0123456789abcdef0123456789abcdeG")),
            with("session", json!("0123456789ABCDEF0123456789ABCDEF")),
            with("session", json!("0123456789abcdef")),
            with("session", json!(5)),
            with("secret", json!("short")),
            with("secret", json!(null)),
            with("mac", json!("not a mac")),
            with("mac", json!("01:00:5e:00:00:01")), // multicast
            with("mac", json!("00:00:00:00:00:00")),
            with("mac", json!(["02", "4c"])),
            with("name", json!(7)),
            with("resumed", json!("false")),
            without("session"),
            without("secret"),
            without("mac"),
            without("name"),
            without("resumed"),
            "[]".into(),
            "null".into(),
            "".into(),
            "{".into(),
        ];
        for b in bad {
            assert_eq!(parse_greeting(&b), None, "{b}");
        }
        let huge = with("name", json!("x".repeat(MAX_GREETING)));
        assert_eq!(parse_greeting(&huge), None);
    }

    #[test]
    fn greeting_names_are_cleaned_and_capped() {
        let base: Value = serde_json::from_str(GOOD).unwrap();
        let mut b = base.clone();
        b["name"] = json!(format!("Evil\u{202e}\u{7}name{}", "y".repeat(200)));
        let g = parse_greeting(&b.to_string()).unwrap();
        assert!(g.name.starts_with("Evilname"));
        assert!(g.name.chars().count() <= 64);
        assert!(!g.name.chars().any(|c| c.is_control() || c == '\u{202e}'));
        b["name"] = json!("\u{7}\u{200b}");
        assert_eq!(parse_greeting(&b.to_string()).unwrap().name, "Phone");
    }

    #[test]
    fn arbitrary_greetings_never_panic() {
        let alphabet: Vec<char> = "{}[]\":,0123456789abcdefsionrtmuld:\\é ".chars().collect();
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..30_000 {
            let len = (seed % 120) as usize;
            let mut s = String::new();
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            if let Some(g) = parse_greeting(&s) {
                assert_eq!(g.session.len(), 32);
            }
        }
        // Mutations of a valid greeting.
        let good = GOOD.as_bytes();
        for i in 0..good.len() {
            for b in [b'"', b'x', b'0', b'}', 0xff] {
                let mut m = good.to_vec();
                m[i] = b;
                if let Ok(s) = std::str::from_utf8(&m) {
                    let _ = parse_greeting(s);
                }
            }
        }
    }

    #[test]
    fn close_codes_map_as_documented() {
        assert_eq!(
            close_action(Some(1008), "unauthorised"),
            CloseAction::Reauthenticate
        );
        assert_eq!(
            close_action(Some(1008), "access revoked"),
            CloseAction::Stop
        );
        assert_eq!(
            close_action(Some(1008), "policy violation"),
            CloseAction::Stop
        );
        assert_eq!(close_action(Some(4000), ""), CloseAction::Stop);
        assert_eq!(close_action(Some(4404), ""), CloseAction::NewSession);
        assert_eq!(close_action(Some(1001), ""), CloseAction::NewSession);
        assert_eq!(close_action(Some(1013), ""), CloseAction::Resume);
        assert_eq!(close_action(Some(1006), ""), CloseAction::Resume);
        assert_eq!(close_action(None, ""), CloseAction::Resume);
    }

    #[test]
    fn stream_failures_retry_only_when_transient() {
        assert!(stream_should_retry(0));
        assert!(stream_should_retry(502));
        assert!(stream_should_retry(503));
        for s in [401, 404, 410, 416, 400, 302] {
            assert!(!stream_should_retry(s), "{s}");
        }
    }

    #[test]
    fn backoff_matches_the_web_player() {
        assert_eq!(backoff(0, 0.5), Duration::from_millis(500));
        assert_eq!(backoff(1, 0.5), Duration::from_millis(1000));
        assert_eq!(backoff(3, 0.5), Duration::from_millis(4000));
        assert_eq!(backoff(4, 0.5), Duration::from_millis(5000));
        assert_eq!(backoff(u32::MAX, 1.0), Duration::from_millis(6000));
        assert_eq!(backoff(0, 0.0), Duration::from_millis(400));
        assert_eq!(backoff(0, f64::NAN), Duration::from_millis(500));
        assert_eq!(backoff(0, 9.0), Duration::from_millis(600));
    }

    #[test]
    fn sync_membership_and_group_time_are_read_strictly() {
        assert!(in_sync_group(
            &json!({"sync_master": "02:4c:41:00:00:01", "sync_slaves": "02:4c:41:00:00:02"})
        ));
        assert!(in_sync_group(&json!({"sync_slaves": "aa:bb"})));
        for alone in [
            json!({}),
            json!({"sync_master": ""}),
            json!({"sync_master": "  "}),
            json!({"sync_master": 1}),
            json!({"sync_slaves": ["a"]}),
            json!({"sync_master": "x".repeat(5000)}),
            json!(null),
            json!([1]),
        ] {
            assert!(!in_sync_group(&alone), "{alone}");
        }
        assert_eq!(group_time(&json!({"_time": 12.5})), Some(12.5));
        assert_eq!(group_time(&json!({"_time": "61"})), Some(61.0));
        for bad in [
            json!({}),
            json!({"_time": -1}),
            json!({"_time": "NaN"}),
            json!({"_time": "inf"}),
            json!({"_time": 1e300}),
            json!({"_time": [1]}),
            json!({"_time": "1".repeat(30)}),
        ] {
            assert_eq!(group_time(&bad), None, "{bad}");
        }
    }

    #[test]
    fn the_resume_window_follows_the_grant() {
        assert_eq!(
            resume_for(None),
            Duration::from_secs(55),
            "an older gateway"
        );
        assert_eq!(resume_for(Some(60)), Duration::from_secs(55));
        assert_eq!(resume_for(Some(600)), Duration::from_secs(595));
        assert_eq!(resume_for(Some(120)), Duration::from_secs(115));
        assert_eq!(resume_for(Some(3)), Duration::ZERO, "never negative");
        assert_eq!(resume_for(Some(0)), Duration::ZERO);
        assert_eq!(
            resume_for(Some(u64::MAX)),
            Duration::from_secs(MAX_GRACE - 5)
        );
    }

    #[test]
    fn greetings_carry_the_grant_and_hostile_grants_are_ignored() {
        let base: Value = serde_json::from_str(GOOD).unwrap();
        assert_eq!(parse_greeting(GOOD).unwrap().grace, None, "older gateway");
        let with = |v: Value| {
            let mut b = base.clone();
            b["grace"] = v;
            parse_greeting(&b.to_string()).unwrap().grace
        };
        assert_eq!(with(json!(600)), Some(600));
        assert_eq!(with(json!(0)), Some(0));
        for bad in [
            json!(-1),
            json!(3601),
            json!(1e300),
            json!(60.5),
            json!("600"),
            json!(null),
            json!([600]),
            json!(u64::MAX),
        ] {
            assert_eq!(with(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn grace_is_asked_for_only_when_wanted() {
        assert_eq!(
            sign_in_message_with_grace("tok", None, StreamQuality::Original, None),
            "tok"
        );
        let v: Value = serde_json::from_str(&sign_in_message_with_grace(
            "tok",
            None,
            StreamQuality::Original,
            Some(GRACE_ASK),
        ))
        .unwrap();
        assert_eq!(v, json!({"token": "tok", "grace": 600}));
        let v: Value = serde_json::from_str(&sign_in_message_with_grace(
            "tok",
            Some(("s", "k", 4)),
            StreamQuality::High,
            Some(99_999),
        ))
        .unwrap();
        assert_eq!(v["grace"], 3600, "capped");
        assert_eq!(v["resume"]["received"], 4);
        assert_eq!(v["quality"], "high");
    }

    #[test]
    fn features_are_read_strictly() {
        let f =
            parse_features(&json!({"features": {"stream_quality": true, "resume_grace": true}}));
        assert!(f.stream_quality && f.resume_grace);
        for bad in [
            json!({}),
            json!({"features": {"resume_grace": "true"}}),
            json!({"features": {"resume_grace": 1}}),
            json!({"features": [true]}),
            json!(null),
        ] {
            assert!(!parse_features(&bad).resume_grace, "{bad}");
        }
    }

    #[test]
    fn stream_retries_last_as_long_as_the_grant() {
        assert_eq!(stream_retry_budget(None), STREAM_RETRY_DEFAULT);
        assert_eq!(stream_retry_budget(Some(600)), Duration::from_secs(600));
        assert_eq!(
            stream_retry_budget(Some(0)),
            Duration::from_secs(5),
            "a floor"
        );
        assert_eq!(
            stream_retry_budget(Some(u64::MAX)),
            Duration::from_secs(MAX_GRACE)
        );
        let budget = Duration::from_secs(600);
        assert!(stream_retry_allowed(Duration::ZERO, budget));
        // Well past the old 12 tries (about 50 s to 3 min).
        assert!(stream_retry_allowed(Duration::from_secs(300), budget));
        assert!(!stream_retry_allowed(Duration::from_secs(600), budget));
        assert!(!stream_retry_allowed(Duration::from_secs(601), budget));
    }

    fn stat(event: &[u8; 4], tag: u8) -> Vec<u8> {
        let mut f = b"STAT".to_vec();
        f.extend(53u32.to_be_bytes());
        f.extend(event);
        f.extend([tag; 49]);
        f
    }

    #[test]
    fn heartbeats_are_never_held_and_a_full_outbox_drops_its_oldest() {
        let mut o = VecDeque::new();
        push_outbox(&mut o, stat(b"STMs", 1), 4);
        for i in 0..3 {
            push_outbox(&mut o, stat(b"STMt", i), 4);
        }
        push_outbox(&mut o, stat(b"STMd", 2), 4);
        push_outbox(&mut o, stat(b"STMo", 3), 4);
        push_outbox(&mut o, stat(b"STMr", 4), 4);
        let events: Vec<&[u8]> = o.iter().map(|f| &f[8..12]).collect();
        assert_eq!(
            events,
            vec![&b"STMs"[..], b"STMd", b"STMo", b"STMr"],
            "no heartbeat was held"
        );
        // Full of events: the oldest goes.
        push_outbox(&mut o, stat(b"STMu", 5), 4);
        assert_eq!(&o[0][8..12], b"STMd");
        assert_eq!(o.len(), 4);
        // Short or other frames are never taken for heartbeats.
        assert!(!is_heartbeat(b"STAT"));
        assert!(!is_heartbeat(b"BYE!\0\0\0\x01STMt"));
        assert!(is_heartbeat(&stat(b"STMt", 0)));
        // Any frames at all: bounded, no panic.
        let mut o = VecDeque::new();
        for i in 0..5000u32 {
            let f = match i % 4 {
                0 => stat(b"STMt", i as u8),
                1 => vec![i as u8; (i % 13) as usize],
                2 => stat(b"STMs", 0),
                _ => b"STATxxxxSTMt".to_vec(),
            };
            push_outbox(&mut o, f, 100);
            assert!(o.len() <= 100);
        }
    }

    fn held(event: &[u8; 4], tag: u8, gap_underrun: bool) -> Held {
        Held {
            frame: stat(event, tag),
            gap_underrun,
        }
    }

    fn released_events(r: &Released) -> Vec<(String, u8)> {
        r.frames
            .iter()
            .map(|f| (String::from_utf8_lossy(&f[8..12]).into_owned(), f[12]))
            .collect()
    }

    /// Regression (live, 2026-10-15): after a four-minute gap the outbox released about 250
    /// held `STMt`s at once; LMS took each one's jiffies (a second apart, all arriving
    /// together) as a new clock epoch for the player, its sync play points went wrong and
    /// it then sent the group 178 `skipAhead` corrections. A held heartbeat is never sent:
    /// the gateway spoke for the player meanwhile, and the core sends a fresh one within a
    /// second of the link returning. Every event around them is kept, in order.
    #[test]
    fn held_heartbeats_are_never_released() {
        for grouped in [false, true] {
            let mut o = VecDeque::new();
            for i in 0..100u8 {
                push_outbox(&mut o, held(b"STMt", i, false), OUTBOX_LIMIT);
            }
            push_outbox(&mut o, held(b"STMs", 1, false), OUTBOX_LIMIT);
            for i in 100..250u8 {
                push_outbox(&mut o, held(b"STMt", i, false), OUTBOX_LIMIT);
            }
            push_outbox(&mut o, held(b"STMd", 2, false), OUTBOX_LIMIT);
            push_outbox(&mut o, held(b"STMt", 251, false), OUTBOX_LIMIT);
            let r = release_outbox(&mut o, grouped);
            assert!(o.is_empty());
            assert_eq!(
                released_events(&r),
                vec![("STMs".into(), 1), ("STMd".into(), 2)],
                "grouped: {grouped}"
            );
        }
        // However they got there, held heartbeats are not released.
        let mut o: VecDeque<Held> = [
            held(b"STMt", 1, false),
            held(b"STMu", 2, false),
            held(b"STMt", 3, false),
            held(b"STMf", 4, false),
            held(b"STMp", 5, false),
        ]
        .into();
        let r = release_outbox(&mut o, false);
        assert_eq!(
            released_events(&r),
            vec![("STMu".into(), 2), ("STMf".into(), 4), ("STMp".into(), 5)]
        );
    }

    /// Regression (live check, 2026-10-13): in a sync group, a mid-track `STMo` held through
    /// the gap is not sent on the rejoin (LMS would pause the whole group to rebuffer), nor
    /// the `STMr` that reported its recovery. Everything else goes, in order.
    #[test]
    fn a_grouped_rejoin_drops_the_stale_mid_track_stmo() {
        let mut o = VecDeque::new();
        for h in [
            held(b"STMt", 1, false),
            held(b"STMo", 2, true),
            held(b"STMt", 3, false),
        ] {
            push_outbox(&mut o, h, OUTBOX_LIMIT);
        }
        let r = release_outbox(&mut o, true);
        assert!(o.is_empty());
        assert_eq!(released_events(&r), vec![]);
        assert!(r.stall_unreported, "still stalled: its recovery isn't told");

        // Recovered during the gap: the STMr pairing with the dropped STMo goes too.
        let mut o: VecDeque<Held> = [
            held(b"STMo", 1, true),
            held(b"STMs", 2, false),
            held(b"STMr", 3, false),
            held(b"STMd", 4, false),
        ]
        .into();
        let r = release_outbox(&mut o, true);
        assert_eq!(
            released_events(&r),
            vec![("STMs".into(), 2), ("STMd".into(), 4)]
        );
        assert!(!r.stall_unreported);
    }

    /// Negative: alone, every held frame goes to LMS as before, the mid-track `STMo` and its
    /// `STMr` included.
    #[test]
    fn a_rejoin_alone_sends_everything_held() {
        let frames = [
            held(b"STMs", 1, false),
            held(b"STMo", 2, true),
            held(b"STMr", 3, false),
            held(b"STMo", 4, true),
        ];
        let mut o: VecDeque<Held> = frames.clone().into();
        let r = release_outbox(&mut o, false);
        assert_eq!(r.frames.len(), 4);
        assert!(r.frames.iter().zip(&frames).all(|(a, b)| *a == b.frame));
        assert!(!r.stall_unreported);
    }

    /// Negative: in a group, the end-of-track `STMo` (sent in place of `STMu`, everything
    /// received played out) and the events around it are kept; so is an `STMr` that isn't
    /// a dropped underrun's recovery (an answer to an unpause before the gap).
    #[test]
    fn a_grouped_rejoin_keeps_end_of_track_reports() {
        let mut o: VecDeque<Held> = [
            held(b"STMr", 1, false),
            held(b"STMs", 2, false),
            held(b"STMo", 3, true),
            held(b"STMr", 4, false),
            held(b"STMd", 5, false),
            held(b"STMo", 6, false),
        ]
        .into();
        let r = release_outbox(&mut o, true);
        assert_eq!(
            released_events(&r),
            vec![
                ("STMr".into(), 1),
                ("STMs".into(), 2),
                ("STMd".into(), 5),
                ("STMo".into(), 6)
            ]
        );
        assert!(!r.stall_unreported, "the stall LMS heard of is reported");
        // A kept STMo between a dropped one and an STMr: that STMr is the kept one's.
        let mut o: VecDeque<Held> = [
            held(b"STMo", 1, true),
            held(b"STMo", 2, false),
            held(b"STMr", 3, false),
        ]
        .into();
        let r = release_outbox(&mut o, true);
        assert_eq!(
            released_events(&r),
            vec![("STMo".into(), 2), ("STMr".into(), 3)]
        );
    }

    /// Robustness: any frames at all, tagged or not, held or put straight in the outbox,
    /// never panic; no heartbeat is ever released, and nothing else but `STMo`s and
    /// `STMr`s is ever dropped.
    #[test]
    fn releasing_arbitrary_frames_never_panics() {
        let mut seed = 0x9e37_79b9u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..500 {
            let mut o = VecDeque::new();
            for _ in 0..(next() % 40) {
                let r = next();
                let frame = match r % 6 {
                    0 => stat(b"STMo", r as u8),
                    1 => stat(b"STMr", r as u8),
                    2 => stat(b"STMt", r as u8),
                    3 => vec![r as u8; (r % 15) as usize],
                    4 => b"STAT\0\0\0\x04STMo".to_vec(),
                    _ => stat(b"STMd", 0),
                };
                let gap_underrun = (r >> 9) % 2 == 0;
                let h = Held {
                    frame,
                    gap_underrun,
                };
                if (r >> 10) % 2 == 0 {
                    push_outbox(&mut o, h, 30);
                } else {
                    o.push_back(h);
                }
            }
            let others = |frames: &mut dyn Iterator<Item = &Vec<u8>>| {
                frames
                    .filter(|f| {
                        !is_stat_event(f, b"STMo") && !is_stat_event(f, b"STMr") && !is_heartbeat(f)
                    })
                    .count()
            };
            let before = others(&mut o.iter().map(|h| &h.frame));
            let grouped = next() % 3 == 0;
            let r = release_outbox(&mut o, grouped);
            assert_eq!(others(&mut r.frames.iter()), before);
            assert!(!r.frames.iter().any(|f| is_heartbeat(f)));
            assert!(o.is_empty());
        }
    }

    fn rejoin_point() -> RejoinPoint {
        RejoinPoint {
            index: 3,
            track_id: 42,
            stalled_at_ms: 95_000,
            stalled_ms: 14_250,
        }
    }

    /// LMS's answer to `status - 2 tags:xed`: playing track 42 at position 3 (240.5 s
    /// long), its `time` lagging at the stalled member's own stale position.
    fn rejoin_status() -> Value {
        json!({"mode": "play", "time": 95.0, "playlist_cur_index": "3",
        "playlist_loop": [
            {"playlist index": 3, "id": 42, "duration": "240.5"},
            {"playlist index": 4, "id": 43, "duration": 200}
        ]})
    }

    /// Regression (live check, 2026-10-13): the group restarts where the others are now,
    /// the stall point plus the time stalled, not at LMS's lagging `time`.
    #[test]
    fn a_rejoin_restarts_the_group_where_the_others_are() {
        assert_eq!(
            rejoin_command(&rejoin_point(), &rejoin_status()),
            Some(vec![json!("time"), json!("109.250")])
        );
        // A track with no duration given: still the group's position.
        let mut st = rejoin_status();
        st["playlist_loop"][0]
            .as_object_mut()
            .unwrap()
            .remove("duration");
        assert_eq!(
            rejoin_command(&rejoin_point(), &st),
            Some(vec![json!("time"), json!("109.250")])
        );
        // Numbers or strings, as LMS gives them.
        let st = json!({"mode": "play", "playlist_cur_index": 3,
            "playlist_loop": [{"playlist index": "3", "id": "42", "duration": 240}]});
        assert!(rejoin_command(&rejoin_point(), &st).is_some());
    }

    /// Negative: the track changed during the gap (another track current, or the stalled
    /// one elsewhere in the queue), so no seek lands in the wrong track.
    #[test]
    fn no_rejoin_seek_when_the_track_changed() {
        let p = rejoin_point();
        let mut moved_on = rejoin_status();
        moved_on["playlist_cur_index"] = json!("4");
        moved_on["playlist_loop"] = json!([{"playlist index": 4, "id": 43, "duration": 200}]);
        assert_eq!(rejoin_command(&p, &moved_on), None);
        let mut replaced = rejoin_status();
        replaced["playlist_loop"][0]["id"] = json!(99);
        assert_eq!(rejoin_command(&p, &replaced), None);
        let mut shifted = rejoin_status();
        shifted["playlist_cur_index"] = json!("2");
        shifted["playlist_loop"][0]["playlist index"] = json!(2);
        assert_eq!(rejoin_command(&p, &shifted), None);
        let mut radio = rejoin_status();
        radio["playlist_loop"][0]["remote"] = json!(1);
        assert_eq!(rejoin_command(&p, &radio), None);
    }

    /// Negative: LMS paused or stopped the group during the gap: the others aren't where
    /// the time stalled says, so nothing moves.
    #[test]
    fn no_rejoin_seek_when_lms_is_not_playing() {
        for mode in ["pause", "stop", "", "PLAY"] {
            let mut st = rejoin_status();
            st["mode"] = json!(mode);
            assert_eq!(rejoin_command(&rejoin_point(), &st), None, "{mode}");
        }
        let mut st = rejoin_status();
        st.as_object_mut().unwrap().remove("mode");
        assert_eq!(rejoin_command(&rejoin_point(), &st), None);
    }

    /// The others have finished the track, or are about to: no seek to (or past) its end.
    #[test]
    fn no_rejoin_seek_past_the_end_of_the_track() {
        let st = rejoin_status();
        let end = 240_500;
        let at = |stalled_ms| RejoinPoint {
            stalled_ms,
            ..rejoin_point()
        };
        assert_eq!(rejoin_command(&at(end - 95_000), &st), None, "at the end");
        assert_eq!(rejoin_command(&at(end - 95_000 + 60_000), &st), None);
        assert_eq!(
            rejoin_command(&at(end - 95_000 - REJOIN_END_MARGIN_MS), &st),
            None,
            "within the margin"
        );
        let just_before = end - 95_000 - REJOIN_END_MARGIN_MS - 1;
        assert_eq!(
            rejoin_command(&at(just_before), &st),
            Some(vec![json!("time"), json!("239.499")])
        );
    }

    /// Bounded: an absurd stall (longer than any session lasts), or one that overflows,
    /// moves nothing.
    #[test]
    fn no_rejoin_seek_for_an_absurd_stall() {
        let mut st = rejoin_status();
        st["playlist_loop"][0]
            .as_object_mut()
            .unwrap()
            .remove("duration");
        let at = |stalled_at_ms, stalled_ms| RejoinPoint {
            stalled_at_ms,
            stalled_ms,
            ..rejoin_point()
        };
        assert!(rejoin_command(&at(0, MAX_REJOIN_STALL_MS), &st).is_some());
        assert_eq!(rejoin_command(&at(0, MAX_REJOIN_STALL_MS + 1), &st), None);
        assert_eq!(rejoin_command(&at(u64::MAX, 1), &st), None);
        assert_eq!(rejoin_command(&at(0, u64::MAX), &st), None);
    }

    /// Untrusted answers: arbitrary shapes never panic, and a command is only ever `time`
    /// at the point worked out, for the stalled track.
    #[test]
    fn rejoin_answers_are_untrusted() {
        let parts = [
            json!(null),
            json!(-1),
            json!(3),
            json!("3"),
            json!(42),
            json!("42"),
            json!(1e300),
            json!(f64::MIN_POSITIVE),
            json!("x"),
            json!([]),
            json!({}),
            json!("play"),
            json!("99999999999999999999"),
        ];
        let p = rejoin_point();
        let mut seed = 0x1234_5678u64;
        for _ in 0..20_000 {
            let mut pick = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                parts[(seed % parts.len() as u64) as usize].clone()
            };
            let st = json!({
                "mode": pick(),
                "playlist_cur_index": pick(),
                "playlist repeat": pick(),
                "playlist_loop": [
                    {"playlist index": pick(), "id": pick(), "remote": pick(),
                     "duration": pick()},
                    pick()
                ]
            });
            if let Some(cmd) = rejoin_command(&p, &st) {
                assert_eq!(cmd, vec![json!("time"), json!("109.250")]);
            }
        }
        for st in [
            json!(null),
            json!([]),
            json!("play"),
            json!({"mode": "play"}),
        ] {
            assert_eq!(rejoin_command(&p, &st), None);
        }
    }

    #[test]
    fn control_messages_are_well_formed() {
        let v: Value = serde_json::from_str(&ack_message(3, 1 << 20)).unwrap();
        assert_eq!(v, json!({"ack": {"stream": 3, "bytes": 1048576}}));
        let v: Value = serde_json::from_str(&resume_message("t\"ok", "s", "x", 9)).unwrap();
        assert_eq!(v["token"], "t\"ok");
        assert_eq!(v["resume"]["received"], 9);
    }
}
