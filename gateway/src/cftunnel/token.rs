//! The tunnel token's shape, checked before it is stored or handed to `cloudflared`.
//!
//! The token Cloudflare's dashboard shows (and `GET …/cfd_tunnel/{id}/token` returns) is
//! base64 (standard or URL alphabet, padding optional) of a JSON object with exactly three
//! fields: `a`, the account id (32 hex characters), `t`, the tunnel id (a UUID), and `s`, the
//! tunnel secret (base64, 32–128 characters). The token as a whole is 64–4096 characters.
//! Nothing else is accepted, and the decoded fields are not kept: only the token itself is
//! stored, as it was given.

use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Shortest and longest token accepted, in characters.
pub const TOKEN_LEN: std::ops::RangeInclusive<usize> = 64..=4096;

const INDIFFERENT: GeneralPurposeConfig =
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
const STANDARD: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, INDIFFERENT);
const URL_SAFE: GeneralPurpose = GeneralPurpose::new(&alphabet::URL_SAFE, INDIFFERENT);

/// Decode base64 in either alphabet (not a mix of the two), with or without padding.
pub fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let std_only = s.bytes().any(|b| b == b'+' || b == b'/');
    let url_only = s.bytes().any(|b| b == b'-' || b == b'_');
    if std_only && url_only {
        return None;
    }
    let engine = if url_only { &URL_SAFE } else { &STANDARD };
    engine.decode(s).ok()
}

/// Encode as standard base64 with padding (the tunnel secret sent to Cloudflare).
pub fn encode_base64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn is_hex32(v: Option<&serde_json::Value>) -> bool {
    v.and_then(|v| v.as_str())
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A dashed UUID, lower or upper case (the tunnel id).
fn is_uuid(v: Option<&serde_json::Value>) -> bool {
    v.and_then(|v| v.as_str())
        .is_some_and(|s| super::scrub::is_uuid(s.as_bytes()))
}

/// Is this a tunnel token? (Leading and trailing whitespace is not part of it: trim first.)
pub fn valid_tunnel_token(token: &str) -> bool {
    if !TOKEN_LEN.contains(&token.len()) {
        return false;
    }
    let Some(bytes) = decode_base64(token) else {
        return false;
    };
    let Ok(serde_json::Value::Object(obj)) = serde_json::from_slice::<serde_json::Value>(&bytes)
    else {
        return false;
    };
    if obj.len() != 3 || !is_hex32(obj.get("a")) || !is_uuid(obj.get("t")) {
        return false;
    }
    let Some(secret) = obj.get("s").and_then(|v| v.as_str()) else {
        return false;
    };
    (32..=128).contains(&secret.len()) && decode_base64(secret).is_some_and(|b| !b.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::{STANDARD as STD_PAD, URL_SAFE_NO_PAD};

    const A: &str = "0123456789abcdef0123456789abcdef";
    const T: &str = "fedcba98-7654-4321-8fed-cba987654321";

    fn secret() -> String {
        STD_PAD.encode([0xfbu8; 32])
    }

    fn token_of(json: &str) -> String {
        STD_PAD.encode(json)
    }

    fn good_json() -> String {
        format!(r#"{{"a":"{A}","t":"{T}","s":"{}"}}"#, secret())
    }

    #[test]
    fn good_tokens_are_accepted_in_both_alphabets_with_or_without_padding() {
        let json = good_json();
        let std = STD_PAD.encode(&json);
        assert!(valid_tunnel_token(&std), "{std}");
        assert!(valid_tunnel_token(std.trim_end_matches('=')));
        let url = base64::engine::general_purpose::URL_SAFE.encode(&json);
        assert!(valid_tunnel_token(&url));
        assert!(valid_tunnel_token(&URL_SAFE_NO_PAD.encode(&json)));
        // Field order does not matter, an upper-case UUID is still a UUID.
        let reordered = format!(
            r#"{{"s":"{}","t":"{}","a":"{A}"}}"#,
            secret(),
            T.to_uppercase()
        );
        assert!(valid_tunnel_token(&token_of(&reordered)));
        // A secret in the URL alphabet, unpadded.
        let s = URL_SAFE_NO_PAD.encode([0xfbu8; 40]);
        assert!(s.contains('-') || s.contains('_'));
        let j = format!(r#"{{"a":"{A}","t":"{T}","s":"{s}"}}"#);
        assert!(valid_tunnel_token(&token_of(&j)));
    }

    #[test]
    fn every_bad_shape_is_refused() {
        let json = good_json();
        let good = token_of(&json);
        let s = secret();
        let bad_json = [
            // Missing, extra or mistyped fields.
            format!(r#"{{"a":"{A}","t":"{T}"}}"#),
            format!(r#"{{"a":"{A}","s":"{s}"}}"#),
            format!(r#"{{"t":"{T}","s":"{s}"}}"#),
            format!(r#"{{"a":"{A}","t":"{T}","s":"{s}","x":1}}"#),
            format!(r#"{{"a":1,"t":"{T}","s":"{s}"}}"#),
            format!(r#"{{"a":"{A}","t":null,"s":"{s}"}}"#),
            format!(r#"{{"a":"{A}","t":"{T}","s":42}}"#),
            // Ids of the wrong length or not hex.
            format!(r#"{{"a":"{}","t":"{T}","s":"{s}"}}"#, &A[..31]),
            format!(r#"{{"a":"{A}0","t":"{T}","s":"{s}"}}"#),
            format!(r#"{{"a":"{A}","t":"{}g","s":"{s}"}}"#, &T[..35]),
            format!(r#"{{"a":"{A}","t":"{}","s":"{s}"}}"#, T.replace('-', "")),
            format!(
                r#"{{"a":"{A}","t":"{}","s":"{s}"}}"#,
                T.replacen('-', "", 1)
            ),
            format!(r#"{{"a":"{A}","t":"{A}","s":"{s}"}}"#),
            // A secret too short, too long, or not base64.
            format!(
                r#"{{"a":"{A}","t":"{T}","s":"{}"}}"#,
                STD_PAD.encode([1u8; 12])
            ),
            format!(
                r#"{{"a":"{A}","t":"{T}","s":"{}"}}"#,
                STD_PAD.encode([1u8; 120])
            ),
            format!(r#"{{"a":"{A}","t":"{T}","s":"{}"}}"#, "!".repeat(44)),
            // Not an object.
            format!(r#"["{A}","{T}","{s}"]"#),
            format!(r#""{A}{T}{s}""#),
        ];
        for j in &bad_json {
            assert!(!valid_tunnel_token(&token_of(j)), "{j}");
        }
        // Not base64 at all, a mix of both alphabets, bad padding, whitespace inside.
        let mut mixed = good.clone();
        mixed.replace_range(0..1, "-");
        mixed.replace_range(1..2, "+");
        for t in [
            "x".repeat(80),
            format!("{good}!"),
            mixed,
            format!("{good}==="),
            format!("{} {}", &good[..40], &good[40..]),
            format!("={good}"),
            String::new(),
        ] {
            assert!(!valid_tunnel_token(&t), "{t}");
        }
        // Too short or too long overall, even when well formed inside.
        assert!(!valid_tunnel_token(&good[..60]));
        let long = format!(r#"{{"a":"{A}","t":"{T}","s":"{s}"{}}}"#, " ".repeat(4000));
        assert!(!valid_tunnel_token(&token_of(&long)));
    }

    #[test]
    fn base64_decoding_never_panics_on_random_bytes() {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..5000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let len = (x % 300) as usize;
            let mut bytes = Vec::with_capacity(len);
            let mut y = x;
            for _ in 0..len {
                y = y.wrapping_mul(6364136223846793005).wrapping_add(1);
                bytes.push((y >> 56) as u8);
            }
            let s = String::from_utf8_lossy(&bytes);
            let _ = decode_base64(&s);
            let _ = valid_tunnel_token(&s);
            // Mostly-valid base64 too: a real token with a byte flipped.
            let mut t = token_of(&good_json()).into_bytes();
            let i = (x as usize) % t.len();
            t[i] = b"A+/-_=xyz9"[(x >> 40) as usize % 10];
            let _ = valid_tunnel_token(&String::from_utf8_lossy(&t));
        }
    }
}
