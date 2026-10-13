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

/// Hold `frame` for LMS until the session resumes, within `limit` frames. When full, the
/// oldest `STMt` (a heartbeat, of no use once stale) goes first, so the events that matter
/// (`STMs`, `STMd`, `STMo`, `STMr`, `STMu`…) survive a long gap; only an outbox full of
/// those drops its oldest.
pub fn push_outbox(outbox: &mut VecDeque<Vec<u8>>, frame: Vec<u8>, limit: usize) {
    outbox.push_back(frame);
    while outbox.len() > limit {
        match outbox.iter().position(|f| is_heartbeat(f)) {
            Some(i) => {
                outbox.remove(i);
            }
            None => {
                outbox.pop_front();
            }
        }
    }
}

/// A `STAT` frame for `STMt` (opcode, length, then the event code).
pub fn is_heartbeat(frame: &[u8]) -> bool {
    frame.len() >= 12 && &frame[..4] == b"STAT" && &frame[8..12] == b"STMt"
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_full_outbox_drops_heartbeats_before_events() {
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
            "every heartbeat went first"
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

    #[test]
    fn control_messages_are_well_formed() {
        let v: Value = serde_json::from_str(&ack_message(3, 1 << 20)).unwrap();
        assert_eq!(v, json!({"ack": {"stream": 3, "bytes": 1048576}}));
        let v: Value = serde_json::from_str(&resume_message("t\"ok", "s", "x", 9)).unwrap();
        assert_eq!(v["token"], "t\"ok");
        assert_eq!(v["resume"]["received"], 9);
    }
}
