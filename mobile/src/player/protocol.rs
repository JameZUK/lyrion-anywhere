//! The client side of the gateway's player protocol (docs/PROTOCOL.md, "Player socket",
//! "Resume" and "Streams"): pure functions, so every rule is unit-tested.
//!
//! Everything the gateway sends is treated as untrusted: the greeting is parsed strictly
//! (bounded size, exact field shapes) because its values end up in URLs and in HELO.

use std::time::Duration;

use serde_json::{Value, json};

/// Largest greeting accepted (the real one is about 150 bytes).
pub const MAX_GREETING: usize = 4 * 1024;
/// Largest binary message accepted from the gateway: one Slimproto frame (the core's
/// reader refuses frames over 32 KiB itself).
pub const MAX_FRAME_MESSAGE: usize = 34 * 1024;
/// Give up resuming slightly before the gateway's default 60 s grace runs out.
pub const RESUME_FOR: Duration = Duration::from_secs(55);
/// Acknowledge stream bytes at least this often; the gateway stays within its replay
/// window (4 MiB by default) of the last acknowledgement.
pub const ACK_EVERY: u64 = 256 * 1024;
/// Messages for LMS held while disconnected; the oldest are dropped beyond this.
pub const OUTBOX_LIMIT: usize = 2000;
/// Attempts at one stream download before the track is reported as failed.
pub const STREAM_RETRIES: u32 = 12;

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
    })
}

/// The first message of a resumed session.
pub fn resume_message(token: &str, session: &str, secret: &str, received: u64) -> String {
    json!({
        "token": token,
        "resume": { "session": session, "secret": secret, "received": received }
    })
    .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn control_messages_are_well_formed() {
        let v: Value = serde_json::from_str(&ack_message(3, 1 << 20)).unwrap();
        assert_eq!(v, json!({"ack": {"stream": 3, "bytes": 1048576}}));
        let v: Value = serde_json::from_str(&resume_message("t\"ok", "s", "x", 9)).unwrap();
        assert_eq!(v["token"], "t\"ok");
        assert_eq!(v["resume"]["received"], 9);
    }
}
