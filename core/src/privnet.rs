//! Private networks: the one rule behind *trusted networks* (docs/plans/trusted-networks.md).
//!
//! An admin may let the gateway accept unencrypted connections from networks they trust, such
//! as their VPN; the apps may connect over plain `http://` to a literal address on such a
//! network. Both decisions come down to the same question, answered here: is this address, or
//! this whole network, inside one of the private ranges? The ranges are RFC 1918 and
//! link-local IPv4, the shared-address range VPNs such as Tailscale use, and IPv6's unique
//! local and link-local ranges. Loopback is deliberately not one of them: it has its own rule
//! everywhere, and listing it would be a mistake worth pointing out.
//!
//! Pure, like the rest of the core: parsing and arithmetic only. The same vectors
//! (`web/app/tests/fixtures/private-networks.json`) are run against the plugin's Perl check
//! and the web app's JavaScript check.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// An IP network in CIDR form (`10.8.0.0/24`), with its host bits cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

/// Why a string isn't a network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Nothing but whitespace.
    Empty,
    /// Not `address` or `address/prefix`.
    Syntax,
    /// The prefix length is longer than the address.
    Prefix,
}

impl Cidr {
    /// The network of `addr` with `prefix` bits; `None` if the prefix is too long for the
    /// address family. Host bits are cleared.
    pub fn new(addr: IpAddr, prefix: u8) -> Option<Cidr> {
        let addr = match addr {
            IpAddr::V4(a) => {
                if prefix > 32 {
                    return None;
                }
                IpAddr::V4(Ipv4Addr::from_bits(u32::from(a) & mask32(prefix)))
            }
            IpAddr::V6(a) => {
                if prefix > 128 {
                    return None;
                }
                IpAddr::V6(Ipv6Addr::from_bits(u128::from(a) & mask128(prefix)))
            }
        };
        Some(Cidr { addr, prefix })
    }

    /// A single address as a network (`/32` or `/128`).
    pub fn host(addr: IpAddr) -> Cidr {
        let prefix = if addr.is_ipv4() { 32 } else { 128 };
        Cidr { addr, prefix }
    }

    /// Parse `address` (a single host) or `address/prefix`. Surrounding whitespace is
    /// ignored; nothing else is: no zone ids, no lists, no trailing slash.
    pub fn parse(s: &str) -> Result<Cidr, ParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ParseError::Empty);
        }
        let (addr, prefix) = match s.split_once('/') {
            None => (s, None),
            Some((a, p)) => (a, Some(p)),
        };
        let addr: IpAddr = addr.parse().map_err(|_| ParseError::Syntax)?;
        match prefix {
            None => Ok(Cidr::host(addr)),
            Some(p) => {
                if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::Syntax);
                }
                let p: u8 = p.parse().map_err(|_| ParseError::Prefix)?;
                Cidr::new(addr, p).ok_or(ParseError::Prefix)
            }
        }
    }

    /// The network address.
    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    /// The prefix length.
    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    /// Whether `ip` is inside this network. An address of the other family never is: a
    /// caller that wants IPv4-mapped IPv6 addresses treated as IPv4 converts them first.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                (u32::from(ip) & mask32(self.prefix)) == u32::from(net)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                (u128::from(ip) & mask128(self.prefix)) == u128::from(net)
            }
            _ => false,
        }
    }

    /// Whether every address of this network is inside `other`.
    pub fn within(&self, other: &Cidr) -> bool {
        self.prefix >= other.prefix && other.contains(self.addr)
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

fn mask32(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix.min(32)))
    }
}

fn mask128(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix.min(128)))
    }
}

const fn v4(a: u8, b: u8, c: u8, d: u8, prefix: u8) -> Cidr {
    Cidr {
        addr: IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
        prefix,
    }
}

const fn v6(first: u16, prefix: u8) -> Cidr {
    Cidr {
        addr: IpAddr::V6(Ipv6Addr::new(first, 0, 0, 0, 0, 0, 0, 0)),
        prefix,
    }
}

/// The private ranges: RFC 1918 (`10/8`, `172.16/12`, `192.168/16`), IPv4 link-local
/// (`169.254/16`), the shared address space VPNs such as Tailscale hand out (`100.64/10`,
/// RFC 6598), IPv6 unique local (`fc00::/7`) and IPv6 link-local (`fe80::/10`).
pub const PRIVATE_RANGES: [Cidr; 7] = [
    v4(10, 0, 0, 0, 8),
    v4(172, 16, 0, 0, 12),
    v4(192, 168, 0, 0, 16),
    v4(169, 254, 0, 0, 16),
    v4(100, 64, 0, 0, 10),
    v6(0xfc00, 7),
    v6(0xfe80, 10),
];

/// The private range `ip` is in, if any. Loopback, unspecified, multicast, public and
/// IPv4-mapped IPv6 addresses are in none.
pub fn private_range_of(ip: IpAddr) -> Option<Cidr> {
    PRIVATE_RANGES.iter().copied().find(|r| r.contains(ip))
}

/// Whether `ip` is a private address (inside one of [`PRIVATE_RANGES`]).
pub fn is_private(ip: IpAddr) -> bool {
    private_range_of(ip).is_some()
}

/// Parse a network an admin typed for the *Trusted networks* setting: a private network in
/// CIDR form, or a single private address. Anything else is refused with a sentence that
/// says why, suitable for a settings page or a command-line error.
pub fn trusted_network(s: &str) -> Result<Cidr, String> {
    let net = match Cidr::parse(s) {
        Ok(n) => n,
        Err(ParseError::Empty) => return Err("enter a network, such as 10.8.0.0/24".into()),
        Err(ParseError::Syntax) => {
            return Err(format!(
                "{:?} is not a network: write it as an address, a slash and the prefix length, such as 10.8.0.0/24",
                s.trim()
            ));
        }
        Err(ParseError::Prefix) => {
            return Err(format!(
                "{:?} has a prefix length longer than the address (at most /32 for IPv4, /128 for IPv6)",
                s.trim()
            ));
        }
    };
    if net.addr.is_loopback()
        || net.contains(IpAddr::V4(Ipv4Addr::LOCALHOST))
        || net.contains(IpAddr::V6(Ipv6Addr::LOCALHOST))
    {
        return Err(format!(
            "{net} includes loopback, which is always allowed: leave it out"
        ));
    }
    if let IpAddr::V6(a) = net.addr
        && a.to_ipv4_mapped().is_some()
    {
        return Err(format!(
            "{net} is an IPv4 network written as IPv6: write it as 10.8.0.0/24"
        ));
    }
    if !PRIVATE_RANGES.iter().any(|r| net.within(r)) {
        return Err(format!(
            "{net} is not a private network: only networks within 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 100.64.0.0/10, fc00::/7 or fe80::/10 can be trusted"
        ));
    }
    Ok(net)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vectors shared with the plugin's Perl check and the web app's JavaScript check.
    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/app/tests/fixtures/private-networks.json"
        )))
        .expect("the vectors file is JSON")
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_ranges_in_the_vectors_are_these() {
        let v = vectors();
        let listed: Vec<String> = v["ranges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        let ours: Vec<String> = PRIVATE_RANGES.iter().map(ToString::to_string).collect();
        assert_eq!(listed, ours);
    }

    #[test]
    fn accepted_networks_normalise_as_the_vectors_say() {
        let v = vectors();
        for case in v["networks"]["accepted"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let want = case["normalised"].as_str().unwrap();
            let got = trusted_network(input).unwrap_or_else(|e| panic!("{input:?}: {e}"));
            assert_eq!(got.to_string(), want, "{input:?}");
            // Parsing the normalised form gives the same network.
            assert_eq!(Cidr::parse(want).unwrap(), got);
        }
    }

    #[test]
    fn refused_networks_are_refused_with_a_reason() {
        let v = vectors();
        for case in v["networks"]["refused"].as_array().unwrap() {
            let input = case.as_str().unwrap();
            let err = trusted_network(input)
                .err()
                .unwrap_or_else(|| panic!("{input:?} was accepted"));
            assert!(!err.is_empty() && !err.ends_with('.'), "{input:?}: {err:?}");
        }
    }

    #[test]
    fn the_refusals_say_why() {
        assert!(
            trusted_network("127.0.0.1")
                .unwrap_err()
                .contains("loopback")
        );
        assert!(trusted_network("::1").unwrap_err().contains("loopback"));
        assert!(
            trusted_network("0.0.0.0/0")
                .unwrap_err()
                .contains("loopback"),
            "the catch-all includes loopback"
        );
        assert!(
            trusted_network("8.8.8.0/24")
                .unwrap_err()
                .contains("not a private network")
        );
        assert!(
            trusted_network("::ffff:10.0.0.0/104")
                .unwrap_err()
                .contains("written as IPv6")
        );
        assert!(
            trusted_network("10.8.0.0/33")
                .unwrap_err()
                .contains("prefix length")
        );
        assert!(
            trusted_network("10.8.0/24")
                .unwrap_err()
                .contains("not a network")
        );
        assert!(trusted_network("").unwrap_err().contains("enter a network"));
        assert!(
            trusted_network("   ")
                .unwrap_err()
                .contains("enter a network")
        );
    }

    #[test]
    fn private_addresses_follow_the_vectors() {
        let v = vectors();
        for a in v["addresses"]["private"].as_array().unwrap() {
            let a = a.as_str().unwrap();
            assert!(is_private(ip(a)), "{a} is private");
            // ...and a single private address is a valid trusted network of its own.
            assert!(trusted_network(a).is_ok(), "{a}");
        }
        for a in v["addresses"]["not_private"].as_array().unwrap() {
            let a = a.as_str().unwrap();
            assert!(!is_private(ip(a)), "{a} is not private");
            assert!(trusted_network(a).is_err(), "{a}");
        }
    }

    #[test]
    fn containment_and_within() {
        let net = Cidr::parse("10.8.0.0/24").unwrap();
        assert!(net.contains(ip("10.8.0.1")));
        assert!(net.contains(ip("10.8.0.255")));
        assert!(!net.contains(ip("10.8.1.0")));
        assert!(
            !net.contains(ip("::ffff:10.8.0.1")),
            "the other family, even mapped"
        );
        let all4 = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(all4.contains(ip("203.0.113.9")));
        assert!(!all4.contains(ip("2001:db8::1")));
        let all6 = Cidr::parse("::/0").unwrap();
        assert!(all6.contains(ip("2001:db8::1")));
        assert!(net.within(&Cidr::parse("10.0.0.0/8").unwrap()));
        assert!(net.within(&net));
        assert!(!Cidr::parse("10.0.0.0/8").unwrap().within(&net));
        assert!(
            !Cidr::parse("10.0.0.0/7")
                .unwrap()
                .within(&Cidr::parse("10.0.0.0/8").unwrap())
        );
        let host = Cidr::host(ip("fd00::1"));
        assert_eq!(host.to_string(), "fd00::1/128");
        assert!(host.within(&Cidr::parse("fc00::/7").unwrap()));
    }

    #[test]
    fn host_bits_are_cleared_and_prefixes_bounded() {
        assert_eq!(
            Cidr::parse("10.8.0.5/24").unwrap().to_string(),
            "10.8.0.0/24"
        );
        assert_eq!(
            Cidr::parse("fd12:3456:789a::1/48").unwrap().to_string(),
            "fd12:3456:789a::/48"
        );
        assert_eq!(
            Cidr::parse("10.8.0.0/32").unwrap().to_string(),
            "10.8.0.0/32"
        );
        assert_eq!(Cidr::parse("10.8.0.0/0").unwrap().to_string(), "0.0.0.0/0");
        assert_eq!(Cidr::parse("fd00::/128").unwrap().to_string(), "fd00::/128");
        assert_eq!(Cidr::parse("10.8.0.0/33"), Err(ParseError::Prefix));
        assert_eq!(Cidr::parse("fd00::/129"), Err(ParseError::Prefix));
        assert_eq!(Cidr::parse("fd00::/999"), Err(ParseError::Prefix));
        assert_eq!(
            Cidr::parse("10.8.0.0/1000"),
            Err(ParseError::Syntax),
            "four digits never a prefix"
        );
        assert_eq!(Cidr::parse("10.8.0.0/+1"), Err(ParseError::Syntax));
        assert_eq!(Cidr::parse("10.8.0.0/ 24"), Err(ParseError::Syntax));
        assert_eq!(
            Cidr::parse("fe80::1%eth0"),
            Err(ParseError::Syntax),
            "no zone ids"
        );
        assert_eq!(Cidr::new(ip("10.0.0.0"), 33), None);
        assert_eq!(Cidr::new(ip("::"), 129), None);
    }

    #[test]
    fn arbitrary_input_never_panics() {
        let samples = [
            "",
            "/",
            "//",
            "/24",
            "10/8",
            "10.8.0.0//24",
            "10.8.0.0/24/",
            ".",
            "...",
            ":",
            ":::",
            "::/",
            "256.0.0.0/8",
            "10.8.0.0/-1",
            "10.8.0.0/٣",
            "१०.८.०.०/२४",
            "\u{0}",
            "10.8.0.0/24\u{0}",
            "٠.٠.٠.٠",
            "fe80::/10%",
            "[fd00::]/8",
            "fd00::/8 ",
            " fd00::/8",
            "fd00::/08",
            "010.8.0.0/24",
        ];
        for s in samples {
            let _ = Cidr::parse(s);
            let _ = trusted_network(s);
        }
        // Long inputs too.
        let long = "1".repeat(10_000);
        let _ = trusted_network(&long);
        let _ = trusted_network(&format!("10.8.0.0/{long}"));
    }
}
