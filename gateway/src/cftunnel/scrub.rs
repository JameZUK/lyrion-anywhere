//! The log scrubber: every line of `cloudflared` output kept in the log ring, and every
//! message the tunnel code logs, goes through [`scrub`] first, so no secret or identifier
//! reaches the settings page, the gateway's log or a copied log.
//!
//! Masked:
//! - anything after `token`, `TUNNEL_TOKEN` or `Bearer` on a line (any case);
//! - runs of 40 or more base64 characters (tunnel tokens, API tokens, secrets);
//! - UUIDs (tunnel and connection ids) and runs of 32 or more hex digits (account, zone and
//!   record ids);
//! - any secret named to [`scrub_with`] (the API token while the *create* exchange runs, the
//!   tunnel token while the tunnel runs), wherever it appears.
//!
//! Control characters are dropped and lines are cut at [`MAX_LINE`] characters.

/// What a masked piece is replaced with.
pub const MASK: &str = "[masked]";

/// Longest line kept, in characters (the rest is cut, with `…`).
pub const MAX_LINE: usize = 1000;

/// Scrub one line.
pub fn scrub(line: &str) -> String {
    scrub_with(line, &[])
}

/// Scrub one line, also masking each of `secrets` (empty ones and ones shorter than 8
/// characters are ignored, so a stray short value can't mask half a line).
pub fn scrub_with(line: &str, secrets: &[&str]) -> String {
    let mut s: String = line
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    for secret in secrets {
        if secret.len() >= 8 {
            s = s.replace(secret, MASK);
        }
    }
    let s = mask_after_keyword(&s);
    let s = mask_long_base64(&s);
    let s = mask_uuids(&s);
    let s = mask_long_hex(&s);
    cut(&s)
}

fn cut(s: &str) -> String {
    if s.chars().count() <= MAX_LINE {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_LINE).collect();
    out.push('…');
    out
}

/// `token`, `TUNNEL_TOKEN` (which contains `token`) and `Bearer`: everything after the first
/// of them on the line goes, keeping the separator that follows it.
fn mask_after_keyword(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let at = ["token", "bearer"]
        .iter()
        .filter_map(|k| lower.find(k).map(|i| i + k.len()))
        .min();
    let Some(mut end) = at else {
        return s.to_string();
    };
    // Keep the keyword's own tail (e.g. `tokens`, `token_file`) and the separator, so the
    // line still says what was there.
    let b = s.as_bytes();
    while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
        end += 1;
    }
    while end < b.len() && matches!(b[end], b' ' | b'=' | b':' | b'"' | b'\'') {
        end += 1;
    }
    if end >= b.len() {
        return s.to_string();
    }
    format!("{}{MASK}", &s[..end])
}

fn is_b64(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'_' | b'-')
}

/// Runs of 40 or more base64 characters (either alphabet; `=` ends a run, so `key=value`
/// stays apart and padding is left as it is), at least 20 of
/// them letters or digits (so a box drawn with `-----` survives).
fn mask_long_base64(s: &str) -> String {
    replace_runs(s, is_b64, |run| {
        run.len() >= 40 && run.bytes().filter(u8::is_ascii_alphanumeric).count() >= 20
    })
}

/// Runs of 32 or more hex digits standing alone (not part of a longer word).
fn mask_long_hex(s: &str) -> String {
    replace_runs(
        s,
        |b| b.is_ascii_alphanumeric(),
        |run| run.len() >= 32 && run.bytes().all(|b| b.is_ascii_hexdigit()),
    )
}

/// Replace each maximal run of bytes for which `class` holds, when `doomed(run)`.
fn replace_runs(s: &str, class: impl Fn(u8) -> bool, doomed: impl Fn(&str) -> bool) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if class(b[i]) {
            let start = i;
            while i < b.len() && class(b[i]) {
                i += 1;
            }
            // The class is ASCII only, so these are character boundaries.
            let run = &s[start..i];
            out.push_str(if doomed(run) { MASK } else { run });
        } else {
            let start = i;
            while i < b.len() && !class(b[i]) {
                i += 1;
            }
            out.push_str(&s[start..i]);
        }
    }
    out
}

/// 8-4-4-4-12 hex digits, not inside a longer hex word.
fn mask_uuids(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut copied = 0;
    while i + 36 <= b.len() {
        let before_ok = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let after_ok = i + 36 == b.len() || !b[i + 36].is_ascii_alphanumeric();
        if before_ok && after_ok && is_uuid(&b[i..i + 36]) {
            out.push_str(&s[copied..i]);
            out.push_str(MASK);
            i += 36;
            copied = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[copied..]);
    out
}

/// Is this exactly a UUID's 36 characters?
pub fn is_uuid(b: &[u8]) -> bool {
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TUNNEL_TOKEN: &str = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiZmVkY2JhOTg3NjU0MzIxMGZlZGNiYTk4NzY1NDMyMTAiLCJzIjoiWm1GclpTMXpaV055WlhRdFptRnJaUzF6WldOeVpYUXRabUZyWlE9PSJ9";

    #[test]
    fn a_tunnel_token_is_masked_wherever_it_appears() {
        let line = format!("ERR something went wrong with {TUNNEL_TOKEN} here");
        let out = scrub(&line);
        assert!(!out.contains(&TUNNEL_TOKEN[..20]), "{out}");
        assert!(out.contains(MASK), "{out}");
        assert!(out.starts_with("ERR something went wrong with "), "{out}");
    }

    #[test]
    fn anything_after_token_or_bearer_is_masked() {
        for (line, kept) in [
            ("TUNNEL_TOKEN=abc123 more", "TUNNEL_TOKEN="),
            ("--token abc", "--token "),
            ("token: \"short\"", "token: \""),
            (
                "Authorization: Bearer abcdefgh1234",
                "Authorization: Bearer ",
            ),
            ("authorization: bearer x", "authorization: bearer "),
            ("Token=zzz", "Token="),
            ("using token_file /x/y", "using token_file "),
        ] {
            let out = scrub(line);
            assert_eq!(out, format!("{kept}{MASK}"), "{line}");
        }
        // Nothing after the keyword: nothing to mask.
        assert_eq!(scrub("no token"), "no token");
    }

    #[test]
    fn an_api_token_is_masked_by_shape_and_by_name() {
        let api = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-Ab";
        assert_eq!(api.len(), 40);
        let out = scrub(&format!("request failed for {api}"));
        assert_eq!(out, format!("request failed for {MASK}"));
        // A shorter one is masked when named.
        let short = "Abc_123-xyz_45678901";
        let out = scrub_with(&format!("x {short} y"), &[short]);
        assert_eq!(out, format!("x {MASK} y"));
    }

    #[test]
    fn ids_are_masked() {
        let hex = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            scrub(&format!("account {hex}.")),
            format!("account {MASK}.")
        );
        assert_eq!(
            scrub(&format!("accounts {hex} cfd_tunnel")),
            format!("accounts {MASK} cfd_tunnel")
        );
        // In a path, the whole path may go (it reads as base64): never the id alone left.
        assert!(!scrub(&format!("GET /accounts/{hex}/cfd_tunnel")).contains(hex));
        let uuid = "6f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
        assert_eq!(
            scrub(&format!(
                "INF Registered tunnel connection connIndex=0 connection={uuid} location=lhr01"
            )),
            format!(
                "INF Registered tunnel connection connIndex=0 connection={MASK} location=lhr01"
            )
        );
        assert_eq!(
            scrub(&format!("{uuid}.cfargotunnel.com")),
            format!("{MASK}.cfargotunnel.com")
        );
        assert_eq!(scrub(uuid), MASK);
        assert_eq!(scrub(&uuid.to_uppercase()), MASK);
    }

    #[test]
    fn ordinary_lines_are_left_alone() {
        for line in [
            "2026-10-18T10:00:00Z INF Starting metrics server on 127.0.0.1:20241/metrics",
            "INF Version 2026.10.1 (Checksum abc)",
            "INF +--------------------------------------------------------------------+",
            "INF |  https://quiet-river-lamp-moon.trycloudflare.com              |",
            "WRN Unable to reach the origin service. The service may be down",
            "INF Generated Connector ID: [short]",
            "connIndex=0 ip=198.51.100.7 location=lhr01 protocol=quic",
            "",
        ] {
            assert_eq!(scrub(line), line);
        }
    }

    #[test]
    fn control_characters_are_dropped_and_long_lines_cut() {
        assert_eq!(scrub("a\x1b[31mred\x1b[0m\r"), "a[31mred[0m");
        assert_eq!(scrub("a\tb"), "a b");
        let long = "word ".repeat(400);
        let out = scrub(&long);
        assert_eq!(out.chars().count(), MAX_LINE + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn scrubbing_never_panics_on_arbitrary_text() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let alphabet: Vec<char> = "abcdef0123456789-_=+/ :\"'TOKENtokenBearer\u{e9}\u{1f3b5}\n\t."
            .chars()
            .collect();
        for _ in 0..3000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let len = (x % 200) as usize;
            let mut s = String::new();
            let mut y = x;
            for _ in 0..len {
                y = y
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.push(alphabet[(y >> 33) as usize % alphabet.len()]);
            }
            let out = scrub(&s);
            assert!(out.chars().count() <= MAX_LINE + 1);
            assert!(!out.chars().any(char::is_control));
        }
    }
}
