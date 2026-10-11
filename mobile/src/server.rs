//! The server address the user typed (or scanned), validated before anything is sent to it.
//!
//! Rules (security first):
//! - `https://` is required. Plain `http://` is accepted only for a loopback host (local
//!   development), which is also the only plain-HTTP traffic the gateway itself accepts.
//! - No user name or password in the URL, no query, no fragment and no path: the gateway
//!   serves its API at `/v1/` on the root of its host.
//! - Nothing but a host and an optional port; no whitespace or control characters.
//!
//! A bare host name (`music.example.com`) is read as `https://music.example.com/`.

use std::net::IpAddr;
use std::sync::Arc;

use url::{Host, Url};

use crate::error::ApiError;

const MAX_URL: usize = 2048;

/// A validated gateway address.
#[derive(Debug, uniffi::Object)]
pub struct Server {
    base: Url,
}

#[uniffi::export]
impl Server {
    /// Validate a server address. Errors are `InvalidInput` with a reason fit to show.
    #[uniffi::constructor]
    pub fn new(url: String) -> Result<Arc<Self>, ApiError> {
        Ok(Arc::new(Server {
            base: parse_server_url(&url)?,
        }))
    }

    /// The normalised address, e.g. `https://music.example.com/`.
    pub fn url(&self) -> String {
        self.base.to_string()
    }

    /// The host (and port, if not the default), for display.
    pub fn display_name(&self) -> String {
        let host = self.base.host_str().unwrap_or_default();
        match self.base.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        }
    }

    /// Whether traffic to this server is encrypted (false only for loopback development).
    pub fn is_secure(&self) -> bool {
        self.base.scheme() == "https"
    }
}

impl Server {
    pub(crate) fn base(&self) -> &Url {
        &self.base
    }

    /// An API URL. `path` is a fixed literal such as `v1/rpc` (never user input).
    pub(crate) fn endpoint(&self, path: &str) -> Url {
        debug_assert!(!path.starts_with('/') && !path.contains(".."));
        self.base.join(path).expect("fixed API path joins")
    }

    /// The events socket URL (`wss://…/v1/events`, or `ws://` on loopback).
    pub(crate) fn events_url(&self) -> Url {
        let mut u = self.endpoint("v1/events");
        let scheme = if self.is_secure() { "wss" } else { "ws" };
        u.set_scheme(scheme).expect("ws schemes are valid");
        u
    }
}

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => IpAddr::V4(*ip).is_loopback(),
        Host::Ipv6(ip) => IpAddr::V6(*ip).is_loopback(),
    }
}

/// Validate and normalise a server address (see the module notes).
pub fn parse_server_url(input: &str) -> Result<Url, ApiError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(ApiError::invalid("enter your server's address"));
    }
    if s.len() > MAX_URL {
        return Err(ApiError::invalid("that address is too long"));
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ApiError::invalid("the address must not contain spaces"));
    }
    if s.contains('\\') {
        return Err(ApiError::invalid(
            "the address must not contain backslashes",
        ));
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    // Judge what was typed, not what the URL parser makes of it ("/x/.." normalises to "/").
    let after_scheme = &with_scheme[with_scheme.find("://").map_or(0, |i| i + 3)..];
    if let Some(i) = after_scheme.find(['/', '?', '#']) {
        match &after_scheme[i..] {
            "/" => {}
            t if t.starts_with('?') || t.starts_with("/?") => {
                return Err(ApiError::invalid("the address must not include a query"));
            }
            t if t.starts_with('#') || t.starts_with("/#") => {
                return Err(ApiError::invalid("the address must not include a '#' part"));
            }
            _ => {
                return Err(ApiError::invalid(
                    "the address must be just the server, without a path",
                ));
            }
        }
    }
    let url =
        Url::parse(&with_scheme).map_err(|_| ApiError::invalid("that isn't a valid address"))?;
    let host = url
        .host()
        .ok_or_else(|| ApiError::invalid("the address needs a host name"))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&host) => {}
        "http" => {
            return Err(ApiError::invalid(
                "use https:// — unencrypted connections are not allowed",
            ));
        }
        _ => return Err(ApiError::invalid("the address must start with https://")),
    }
    if let Host::Domain(d) = &host
        && d.is_empty()
    {
        return Err(ApiError::invalid("the address needs a host name"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError::invalid(
            "the address must not include a user name or password",
        ));
    }
    if url.query().is_some() {
        return Err(ApiError::invalid("the address must not include a query"));
    }
    if url.fragment().is_some() {
        return Err(ApiError::invalid("the address must not include a '#' part"));
    }
    if url.path() != "/" {
        return Err(ApiError::invalid(
            "the address must be just the server, without a path",
        ));
    }
    Ok(url)
}

/// What a pairing QR code (or a typed pairing code) holds.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairingInvite {
    /// The server to pair with (normalised), when the QR code names one.
    pub server: Option<String>,
    /// The one-time code, normalised (upper case, no spaces or dashes).
    pub code: String,
}

/// The scheme of pairing links: `lyrion-anywhere://pair?server=<https URL>&code=<code>`.
pub const PAIRING_SCHEME: &str = "lyrion-anywhere";

/// Normalise a pairing code the way the gateway does (spaces and dashes dropped, upper
/// case), and check it could be one: 4–32 letters and digits.
pub fn normalise_pairing_code(code: &str) -> Result<String, ApiError> {
    let c: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    if !(4..=32).contains(&c.len()) || !c.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::invalid("that isn't a valid pairing code"));
    }
    Ok(c)
}

/// Read a scanned QR code or a typed code: a `lyrion-anywhere://pair?…` link, or a bare code.
#[uniffi::export]
pub fn parse_pairing_invite(text: String) -> Result<PairingInvite, ApiError> {
    let t = text.trim();
    if t.len() > MAX_URL {
        return Err(ApiError::invalid("that isn't a pairing code"));
    }
    let prefix = format!("{PAIRING_SCHEME}:");
    if !t.to_ascii_lowercase().starts_with(&prefix) {
        return Ok(PairingInvite {
            server: None,
            code: normalise_pairing_code(t)?,
        });
    }
    let url = Url::parse(t).map_err(|_| ApiError::invalid("that isn't a pairing code"))?;
    if url.host_str() != Some("pair") || !matches!(url.path(), "" | "/") || url.fragment().is_some()
    {
        return Err(ApiError::invalid("that isn't a pairing code"));
    }
    let (mut server, mut code) = (None, None);
    for (k, v) in url.query_pairs() {
        let slot = match k.as_ref() {
            "server" => &mut server,
            "code" => &mut code,
            _ => return Err(ApiError::invalid("that isn't a pairing code")),
        };
        if slot.replace(v.into_owned()).is_some() {
            return Err(ApiError::invalid("that isn't a pairing code"));
        }
    }
    let code = normalise_pairing_code(&code.ok_or_else(|| ApiError::invalid("no pairing code"))?)?;
    let server = match server {
        Some(s) => Some(parse_server_url(&s)?.to_string()),
        None => None,
    };
    Ok(PairingInvite { server, code })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> String {
        parse_server_url(s)
            .unwrap_or_else(|e| panic!("{s}: {e}"))
            .to_string()
    }

    fn bad(s: &str) {
        assert!(
            matches!(parse_server_url(s), Err(ApiError::InvalidInput { .. })),
            "{s:?} was accepted"
        );
    }

    #[test]
    fn accepts_https_and_normalises() {
        assert_eq!(
            ok("https://music.example.com"),
            "https://music.example.com/"
        );
        assert_eq!(ok("music.example.com"), "https://music.example.com/");
        assert_eq!(
            ok("  https://Music.Example.COM/  "),
            "https://music.example.com/"
        );
        assert_eq!(
            ok("https://music.example.com:8443/"),
            "https://music.example.com:8443/"
        );
        assert_eq!(ok("https://10.0.0.10"), "https://10.0.0.10/");
        assert_eq!(ok("https://[2001:db8::1]:443"), "https://[2001:db8::1]/");
        // Plain HTTP only on loopback (local development).
        assert_eq!(ok("http://127.0.0.1:9444"), "http://127.0.0.1:9444/");
        assert_eq!(ok("http://localhost:9444"), "http://localhost:9444/");
        assert_eq!(ok("http://[::1]:9444"), "http://[::1]:9444/");
    }

    #[test]
    fn refuses_anything_but_a_plain_https_origin() {
        for s in [
            "",
            "   ",
            "http://music.example.com",
            "http://10.0.0.10:9000",
            "http://127.0.0.1.nip.io",
            "ftp://music.example.com",
            "file:///srv/music",
            "javascript:alert(1)",
            "ws://music.example.com",
            "wss://music.example.com",
            "https://user@music.example.com",
            "https://user:pw@music.example.com",
            "https://:pw@music.example.com",
            "https://music.example.com/#frag",
            "https://music.example.com/#",
            "https://music.example.com/?q=1",
            "https://music.example.com/?",
            "https://music.example.com/lms",
            "https://music.example.com/v1/rpc",
            "https://music.example.com/..",
            "https://music.example.com\\@evil.example",
            "https://music.example.com /x",
            "https://music\n.example.com",
            "https://music.\u{0}example.com",
            "https://",
            "https:///path",
            "data:text/html,hi",
        ] {
            bad(s);
        }
        bad(&format!("https://{}.example.com", "a".repeat(3000)));
    }

    #[test]
    fn endpoints_stay_under_v1_on_the_same_origin() {
        let s = Server::new("https://music.example.com".into()).unwrap();
        assert_eq!(
            s.endpoint("v1/auth/login").as_str(),
            "https://music.example.com/v1/auth/login"
        );
        assert_eq!(s.events_url().as_str(), "wss://music.example.com/v1/events");
        let local = Server::new("http://127.0.0.1:9444".into()).unwrap();
        assert_eq!(local.events_url().as_str(), "ws://127.0.0.1:9444/v1/events");
        assert!(!local.is_secure());
        assert_eq!(local.display_name(), "127.0.0.1:9444");
    }

    #[test]
    fn pairing_codes_and_links() {
        let p = parse_pairing_invite(" abcd-efgh 2345 ".into()).unwrap();
        assert_eq!(p.code, "ABCDEFGH2345");
        assert_eq!(p.server, None);

        let p = parse_pairing_invite(
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCD2345".into(),
        )
        .unwrap();
        assert_eq!(p.server.as_deref(), Some("https://music.example.com/"));
        assert_eq!(p.code, "ABCD2345");

        for bad in [
            "",
            "abc",
            "ABCD!2345",
            "lyrion-anywhere://pair?code=",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com",
            "lyrion-anywhere://evil?code=ABCD2345",
            "lyrion-anywhere://pair/x?code=ABCD2345",
            "lyrion-anywhere://pair?code=ABCD2345&code=EFGH2345",
            "lyrion-anywhere://pair?code=ABCD2345&token=x",
            "lyrion-anywhere://pair?code=ABCD2345#x",
            // The server in a link obeys the same rules as one typed in.
            "lyrion-anywhere://pair?server=http%3A%2F%2Fmusic.example.com&code=ABCD2345",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fu%40music.example.com&code=ABCD2345",
            "https://music.example.com/?code=ABCD2345",
        ] {
            assert!(
                matches!(
                    parse_pairing_invite(bad.into()),
                    Err(ApiError::InvalidInput { .. })
                ),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn arbitrary_input_never_panics() {
        // A small deterministic fuzz over awkward characters.
        let alphabet: Vec<char> = "ab:/@?#%[]\\. \u{0}\u{202e}1-hHtTpPsS:é".chars().collect();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            let len = (seed % 40) as usize;
            let mut s = String::new();
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let _ = parse_server_url(&s);
            let _ = parse_pairing_invite(s.clone());
            let _ = parse_pairing_invite(format!("lyrion-anywhere://pair?{s}"));
            if let Ok(u) = parse_server_url(&s) {
                assert!(u.scheme() == "https" || u.scheme() == "http");
                assert_eq!(u.path(), "/");
                assert!(u.query().is_none() && u.fragment().is_none());
                assert!(u.username().is_empty() && u.password().is_none());
            }
        }
    }
}
