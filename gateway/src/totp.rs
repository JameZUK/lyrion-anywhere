//! Time-based one-time passwords (RFC 6238): HMAC-SHA1, 30-second steps, 6 digits —
//! the parameters every common authenticator app uses.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

pub const STEP_SECS: u64 = 30;
pub const DIGITS: u32 = 6;
/// Accept codes from one step either side, for clock drift.
pub const SKEW_STEPS: u64 = 1;
pub const SECRET_LEN: usize = 20;

/// A new random 160-bit secret.
pub fn new_secret() -> [u8; SECRET_LEN] {
    let mut s = [0u8; SECRET_LEN];
    getrandom::fill(&mut s).expect("OS random source");
    s
}

/// The code for a time step (RFC 4226 dynamic truncation).
pub fn code_at(secret: &[u8], step: u64) -> u32 {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("any key length");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let offset = (h[h.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        h[offset] & 0x7f,
        h[offset + 1],
        h[offset + 2],
        h[offset + 3],
    ]);
    bin % 10u32.pow(DIGITS)
}

/// If `code` is valid at `unix_secs` (within the skew) and newer than `last_step`, the step
/// it matched. Codes are single use: a step at or before `last_step` is refused.
pub fn verify(secret: &[u8], code: &str, unix_secs: u64, last_step: u64) -> Option<u64> {
    let code = code.trim();
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let want: u32 = code.parse().ok()?;
    let now = unix_secs / STEP_SECS;
    let mut matched = None;
    // Check every candidate (no early exit), so timing doesn't reveal which step matched.
    for step in now.saturating_sub(SKEW_STEPS)..=now + SKEW_STEPS {
        if subtle::ConstantTimeEq::ct_eq(&code_at(secret, step), &want).into() && step > last_step {
            matched = Some(step);
        }
    }
    matched
}

/// RFC 4648 base32 without padding, as authenticator apps expect.
pub fn base32(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut buffer: u64 = 0;
    let mut bits = 0;
    for &b in data {
        buffer = (buffer << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The `otpauth://` URI authenticator apps import (usually via a QR code).
pub fn otpauth_uri(secret: &[u8], issuer: &str, account: &str) -> String {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        enc(issuer),
        enc(account),
        base32(secret),
        enc(issuer)
    )
}

/// The URI as an SVG QR code (for the account page; inlined as a data URI).
pub fn qr_svg(uri: &str) -> String {
    use qrcode::render::svg;
    qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map(|code| {
            code.render::<svg::Color<'_>>()
                .min_dimensions(200, 200)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc6238_sha1_test_vectors() {
        // RFC 6238 appendix B gives 8-digit codes; ours are the last 6 digits.
        for (t, eight) in [
            (59u64, 94287082u32),
            (1111111109, 7081804),
            (1111111111, 14050471),
            (1234567890, 89005924),
            (2000000000, 69279037),
            (20000000000, 65353130),
        ] {
            assert_eq!(
                code_at(RFC_SECRET, t / STEP_SECS),
                eight % 1_000_000,
                "t={t}"
            );
        }
    }

    #[test]
    fn verify_accepts_current_and_adjacent_steps_only() {
        let t = 1_700_000_000u64;
        let step = t / STEP_SECS;
        let code = |s: u64| format!("{:06}", code_at(RFC_SECRET, s));
        assert_eq!(verify(RFC_SECRET, &code(step), t, 0), Some(step));
        assert_eq!(verify(RFC_SECRET, &code(step - 1), t, 0), Some(step - 1));
        assert_eq!(verify(RFC_SECRET, &code(step + 1), t, 0), Some(step + 1));
        assert_eq!(verify(RFC_SECRET, &code(step - 2), t, 0), None);
        assert_eq!(verify(RFC_SECRET, &code(step + 2), t, 0), None);
    }

    #[test]
    fn codes_are_single_use() {
        let t = 1_700_000_000u64;
        let step = t / STEP_SECS;
        let c = format!("{:06}", code_at(RFC_SECRET, step));
        assert_eq!(verify(RFC_SECRET, &c, t, step), None, "already used");
        assert_eq!(verify(RFC_SECRET, &c, t, step - 1), Some(step));
    }

    #[test]
    fn malformed_codes_are_refused() {
        for bad in [
            "",
            "12345",
            "1234567",
            "12a456",
            "１２３４５６",
            "-12345",
            " 12 345",
        ] {
            assert_eq!(verify(RFC_SECRET, bad, 1_700_000_000, 0), None, "{bad:?}");
        }
    }

    #[test]
    fn base32_matches_rfc4648() {
        for (raw, enc) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(raw.as_bytes()), enc);
        }
        assert_eq!(base32(RFC_SECRET), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn otpauth_uri_is_escaped() {
        let uri = otpauth_uri(RFC_SECRET, "Lyrion Anywhere", "alice");
        assert_eq!(
            uri,
            "otpauth://totp/Lyrion%20Anywhere:alice?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ\
             &issuer=Lyrion%20Anywhere&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn qr_code_is_svg() {
        let svg = qr_svg(&otpauth_uri(RFC_SECRET, "Lyrion Anywhere", "alice"));
        assert!(svg.contains("<svg") && svg.contains("</svg>"));
        assert!(!svg.contains("<script"));
    }
}
