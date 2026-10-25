//! The server address the user typed (or scanned), validated before anything is sent to it.
//!
//! Rules (security first):
//! - `https://` is required, with two exceptions for plain `http://`: a loopback host (local
//!   development), and a *literal* address inside a private range (`anywhere_core::privnet`:
//!   a server on the user's own VPN, which the gateway accepts unencrypted only when its
//!   admin lists that network as trusted). Never `http://` to a name: a name could resolve to
//!   a public address and send the password in clear. The app asks before using such a
//!   server (docs/plans/trusted-networks.md).
//! - No user name or password in the URL, no query, no fragment and no path: the gateway
//!   serves its API at `/v1/` on the root of its host.
//! - Nothing but a host and an optional port; no whitespace or control characters.
//!
//! A bare host name (`music.example.com`) is read as `https://music.example.com/`.

use std::net::IpAddr;
use std::sync::Arc;

use anywhere_core::privnet;
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

    /// Whether traffic to this server is encrypted (false for loopback development and for a
    /// literal private address over `http://`).
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

    /// The events socket URL (`wss://…/v1/events`, or `ws://` when the server is `http://`).
    pub(crate) fn events_url(&self) -> Url {
        self.socket_url("v1/events")
    }

    /// The player socket URL (`wss://…/v1/player`, or `ws://` when the server is `http://`).
    pub(crate) fn player_url(&self) -> Url {
        self.socket_url("v1/player")
    }

    fn socket_url(&self, path: &str) -> Url {
        let mut u = self.endpoint(path);
        let scheme = if self.is_secure() { "wss" } else { "ws" };
        u.set_scheme(scheme).expect("ws schemes are valid");
        u
    }

    /// `/v1/stream/<session>/<n>[?offset=k]`. `session` must already be validated (32 hex
    /// characters); it is percent-encoded as a path segment regardless.
    /// `/v1/stream/<session>/<n>`, from byte `offset`, and at most `limit` bytes when given
    /// (the replay cache's look at a stream's first bytes).
    pub(crate) fn stream_url(&self, session: &str, n: u64, offset: u64, limit: Option<u64>) -> Url {
        let mut u = self.endpoint("v1/stream");
        u.path_segments_mut()
            .expect("http(s) URLs have a path")
            .push(session)
            .push(&n.to_string());
        let mut q = Vec::new();
        if offset > 0 {
            q.push(format!("offset={offset}"));
        }
        if let Some(l) = limit {
            q.push(format!("limit={l}"));
        }
        if !q.is_empty() {
            u.set_query(Some(&q.join("&")));
        }
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

/// A literal address (never a name) inside one of the private ranges.
fn is_private_literal(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(_) => false,
        Host::Ipv4(ip) => privnet::is_private(IpAddr::V4(*ip)),
        Host::Ipv6(ip) => privnet::is_private(IpAddr::V6(*ip)),
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
        "http" if is_loopback(&host) || is_private_literal(&host) => {}
        // A browser takes http:// to a name; this app does not, and says so plainly, with
        // where the address is set (a forum report, 2026-10-25: "I can connect my phone via
        // a browser but not using the Android app").
        "http" if matches!(host, Host::Domain(_)) => {
            return Err(ApiError::invalid(
                "over http:// this app needs the server's IP address, not a name: for \
                 example http://10.8.0.1:9443. Set the Public URL under LMS › Settings › \
                 Lyrion Anywhere to that address (the QR code then carries it too), or use \
                 https://. A name could lead somewhere public and carry your password in clear",
            ));
        }
        "http" => {
            return Err(ApiError::invalid(
                "use https:// — that address is not a private address, so an \
                 unencrypted connection to it is not allowed (over http:// only a private \
                 address such as http://10.8.0.1:9443, on your own VPN, is)",
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
    // Exactly `lyrion-anywhere://pair?…`: no user name, password or port either (links
    // arrive from outside the app, so nothing unexpected is let through, even if unused).
    if url.host_str() != Some("pair")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(url.path(), "" | "/")
        || url.fragment().is_some()
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
        // Plain HTTP on loopback (local development) and to a literal private address (a
        // server on the user's own VPN; the app asks first).
        assert_eq!(ok("http://127.0.0.1:9444"), "http://127.0.0.1:9444/");
        assert_eq!(ok("http://10.8.0.1:9443"), "http://10.8.0.1:9443/");
        assert_eq!(ok("http://10.0.0.10:9000/"), "http://10.0.0.10:9000/");
        assert_eq!(ok("HTTP://100.64.0.7"), "http://100.64.0.7/");
        assert_eq!(
            ok("http://[fd12:3456:789a::1]:9443"),
            "http://[fd12:3456:789a::1]:9443/"
        );
        assert_eq!(ok("http://localhost:9444"), "http://localhost:9444/");
        assert_eq!(ok("http://[::1]:9444"), "http://[::1]:9444/");
    }

    #[test]
    fn refuses_anything_but_a_plain_https_origin() {
        for s in [
            "",
            "   ",
            "http://music.example.com",
            "http://203.0.113.10:9000",
            "http://127.0.0.1.nip.io",
            "http://10.0.0.10.nip.io",
            "http://10.8.0.1.example.com",
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
        assert_eq!(s.player_url().as_str(), "wss://music.example.com/v1/player");
        assert_eq!(
            s.stream_url("0123456789abcdef0123456789abcdef", 3, 0, None)
                .as_str(),
            "https://music.example.com/v1/stream/0123456789abcdef0123456789abcdef/3"
        );
        assert_eq!(
            s.stream_url("ab", 1, 65536, None).as_str(),
            "https://music.example.com/v1/stream/ab/1?offset=65536"
        );
        assert_eq!(
            s.stream_url("ab", 1, 0, Some(98304)).as_str(),
            "https://music.example.com/v1/stream/ab/1?limit=98304"
        );
        assert_eq!(
            s.stream_url("ab", 1, 10, Some(5)).as_str(),
            "https://music.example.com/v1/stream/ab/1?offset=10&limit=5"
        );
        // Even an unvalidated session id can't leave its path segment.
        let u = s.stream_url("../../rpc?x=1#y", 1, 0, None);
        assert_eq!(u.path(), "/v1/stream/..%2F..%2Frpc%3Fx=1%23y/1");
        assert_eq!(u.query(), None);
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

    /// Links arrive from outside the app (QR codes, tapped links): anything but the exact
    /// documented form is refused. Regression: user info and a port on the `pair` host were
    /// accepted and ignored.
    #[test]
    fn hostile_pairing_links_are_refused() {
        for bad in [
            "lyrion-anywhere://user:pw@pair?code=ABCD2345",
            "lyrion-anywhere://user@pair?code=ABCD2345",
            "lyrion-anywhere://pair:8443?code=ABCD2345",
            "lyrion-anywhere://PAIR?code=ABCD2345",
            "lyrion-anywhere:pair?code=ABCD2345",
            "lyrion-anywhere:///pair?code=ABCD2345",
            "lyrion-anywhere://pair.evil.example?code=ABCD2345",
            "lyrion-anywhere://pair?=x&code=ABCD2345",
            "lyrion-anywhere://pair?code=ABCD2345&server=",
            "lyrion-anywhere://pair?server=javascript%3Aalert(1)&code=ABCD2345",
            "lyrion-anywhere://pair?server=file%3A%2F%2F%2Fetc%2Fpasswd&code=ABCD2345",
            "lyrion-anywhere://pair?server=intent%3A%2F%2Fx%23Intent%3Bend&code=ABCD2345",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com%2Fphish&code=ABCD2345",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com%3Fx%3D1&code=ABCD2345",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com%23x&code=ABCD2345",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com%5C%40evil.example&code=ABCD2345",
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com%20&code=ABCD2345%00",
            "lyrion-anywhere://pair?server=http%3A%2F%2F203.0.113.10&code=ABCD2345",
            "lyrion-anywhere://pair?server=http%3A%2F%2Fmusic.example.com%3A9443&code=ABCD2345",
            "lyrion-anywhere://pair?server=wss%3A%2F%2Fmusic.example.com&code=ABCD2345",
            // Codes: letters and digits only (ASCII), 4 to 32 of them.
            "lyrion-anywhere://pair?code=%EF%BC%A1%EF%BC%A2%EF%BC%A3%EF%BC%A4",
            "lyrion-anywhere://pair?code=ABCD%E2%80%AE2345",
            "lyrion-anywhere://pair?code=ABC",
            "lyrion-anywhere://pair?code=%3Cscript%3E",
            "lyrion-anywhere://pair?code=ABCD2345%26server%3Dhttps%3A%2F%2Fevil.example",
        ] {
            assert!(
                matches!(
                    parse_pairing_invite(bad.into()),
                    Err(ApiError::InvalidInput { .. })
                ),
                "{bad:?} was accepted"
            );
        }
        // Too long, whatever it holds.
        let long = format!(
            "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCD2345&{}",
            "x".repeat(MAX_URL)
        );
        assert!(parse_pairing_invite(long).is_err());
        // The scheme is case-insensitive (as URLs are); loopback over plain HTTP is left to
        // the app's build to allow or not.
        let p = parse_pairing_invite(
            "LYRION-ANYWHERE://pair?server=https%3A%2F%2Fmusic.example.com%3A8443&code=abcd-2345"
                .into(),
        )
        .unwrap();
        assert_eq!(p.server.as_deref(), Some("https://music.example.com:8443/"));
        assert_eq!(p.code, "ABCD2345");
        let p = parse_pairing_invite(
            "lyrion-anywhere://pair?server=http%3A%2F%2F127.0.0.1%3A9443&code=ABCD2345".into(),
        )
        .unwrap();
        assert_eq!(p.server.as_deref(), Some("http://127.0.0.1:9443/"));
    }

    /// The shared vectors (web/app/tests/fixtures/private-networks.json), as `http://`
    /// addresses: every private literal is accepted, everything else refused, except
    /// loopback, which keeps its own rule (accepted, as before).
    #[test]
    fn plain_http_follows_the_shared_private_address_vectors() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/app/tests/fixtures/private-networks.json"
        );
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let list = |k: &str| -> Vec<String> {
            json["addresses"][k]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        };
        let as_url = |a: &str| {
            if a.contains(':') {
                format!("http://[{a}]:9443")
            } else {
                format!("http://{a}:9443")
            }
        };
        let private = list("private");
        assert!(!private.is_empty());
        for a in &private {
            let u = parse_server_url(&as_url(a)).unwrap_or_else(|e| panic!("{a}: {e}"));
            assert_eq!(u.scheme(), "http");
            let s = Server::new(as_url(a)).unwrap();
            assert!(!s.is_secure(), "{a}");
            assert!(s.events_url().as_str().starts_with("ws://"), "{a}");
            assert!(s.player_url().as_str().starts_with("ws://"), "{a}");
            // The same address over https is, as ever, fine.
            assert!(parse_server_url(&as_url(a).replacen("http", "https", 1)).is_ok());
        }
        let not_private = list("not_private");
        assert!(!not_private.is_empty());
        for a in &not_private {
            let loopback = a.parse::<IpAddr>().unwrap().is_loopback();
            let r = parse_server_url(&as_url(a));
            if loopback {
                assert!(r.is_ok(), "loopback {a} keeps its own rule");
            } else {
                match r {
                    Err(ApiError::InvalidInput { detail }) => {
                        assert!(detail.contains("not a private address"), "{a}: {detail}")
                    }
                    other => panic!("{a} over http was not refused: {other:?}"),
                }
            }
        }
    }

    /// Names are never trusted over plain HTTP, whatever they look like or resolve to.
    #[test]
    fn plain_http_to_a_name_is_refused_with_a_reason() {
        for s in [
            "http://music.example.com",
            "http://music.example.com:9443/",
            "http://lms.example.lan",
            "http://10.8.0.1.nip.io",
            "http://fd00--1.example.com",
            "http://localhost.example.com",
        ] {
            match parse_server_url(s) {
                Err(ApiError::InvalidInput { detail }) => {
                    // What to do instead, in the words a person needs: the IP address,
                    // where it is set, and https:// as the other way.
                    assert!(detail.contains("IP address, not a name"), "{s}: {detail}");
                    assert!(detail.contains("http://10.8.0.1:9443"), "{s}: {detail}");
                    assert!(detail.contains("Public URL"), "{s}: {detail}");
                    assert!(detail.contains("https://"), "{s}: {detail}");
                    assert!(detail.contains("password"), "{s}: {detail}");
                }
                other => panic!("{s} was not refused: {other:?}"),
            }
        }
        // Spelling an address oddly doesn't make it private: the URL parser reads these as
        // the public address they are, and they are refused as one.
        for s in ["http://0x08080808", "http://134744072", "http://8.8.2056"] {
            assert!(parse_server_url(s).is_err(), "{s}");
        }
        // Spelling a private one oddly is read as that private address (normalised).
        assert_eq!(ok("http://0x0a080001:9443"), "http://10.8.0.1:9443/");
    }

    #[test]
    fn pairing_links_may_name_a_private_address_over_http() {
        let p = parse_pairing_invite(
            "lyrion-anywhere://pair?server=http%3A%2F%2F10.8.0.1%3A9443&code=ABCD2345".into(),
        )
        .unwrap();
        assert_eq!(p.server.as_deref(), Some("http://10.8.0.1:9443/"));
        let p = parse_pairing_invite(
            "lyrion-anywhere://pair?server=http%3A%2F%2F%5Bfd00%3A%3A5%5D&code=ABCD2345".into(),
        )
        .unwrap();
        assert_eq!(p.server.as_deref(), Some("http://[fd00::5]/"));
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
                if u.scheme() == "http" {
                    // Plain HTTP only ever to loopback or a literal private address.
                    let host = u.host().unwrap();
                    assert!(is_loopback(&host) || is_private_literal(&host), "{s:?}");
                }
                assert_eq!(u.path(), "/");
                assert!(u.query().is_none() && u.fragment().is_none());
                assert!(u.username().is_empty() && u.password().is_none());
            }
        }
    }
}
