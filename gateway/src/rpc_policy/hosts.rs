//! Is a host, or an address written in text, on the home network or the server itself?
//!
//! Used by response filtering, request validation and the image proxy. Hosts are read the
//! way URL parsers and `inet_aton` read them, so `2130706433`, `0x7f000001`, `127.1` and
//! `0177.0.0.1` are all 127.0.0.1. Names that can only be local (single labels, `.lan`,
//! `.localhost`, …) and public wildcard-DNS names that resolve to whatever address the name
//! embeds (`10.0.0.5.nip.io`) count as local too.

use std::net::{Ipv4Addr, Ipv6Addr};

/// Suffixes of names that only resolve on a home network.
const LOCAL_SUFFIXES: &[&str] = &[
    ".local",
    ".lan",
    ".home",
    ".internal",
    ".intranet",
    ".localdomain",
    ".localhost",
    ".home.arpa",
];

/// Public DNS services that answer with an address taken from the name, or always with a
/// loopback address.
const WILDCARD_DNS: &[&str] = &[
    "nip.io",
    "sslip.io",
    "xip.io",
    "localtest.me",
    "lvh.me",
    "vcap.me",
    "traefik.me",
    "local.gd",
    "localhost.direct",
    "lacolhost.com",
];

/// Private, loopback, link-local, CGNAT, benchmarking, multicast and reserved IPv4.
pub(crate) fn v4_local(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..128).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 224
}

/// Loopback, unspecified, unique-local, link-local, site-local and multicast IPv6, and IPv6
/// forms that carry a local IPv4 address (mapped, compatible, NAT64).
pub(crate) fn v6_local(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    let embedded = Ipv4Addr::new(
        (seg[6] >> 8) as u8,
        seg[6] as u8,
        (seg[7] >> 8) as u8,
        seg[7] as u8,
    );
    let mapped_or_compat = seg[..5].iter().all(|&s| s == 0) && (seg[5] == 0 || seg[5] == 0xffff);
    let nat64 = seg[0] == 0x64 && seg[1] == 0xff9b && seg[2..6].iter().all(|&s| s == 0);
    ip.is_loopback()
        || ip.is_unspecified()
        || (seg[0] & 0xfe00) == 0xfc00
        || (seg[0] & 0xffc0) == 0xfe80
        || (seg[0] & 0xffc0) == 0xfec0
        || (seg[0] & 0xff00) == 0xff00
        || ((mapped_or_compat || nat64) && v4_local(embedded))
}

/// Is a resolved address local?
pub(crate) fn ip_is_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4_local(v4),
        std::net::IpAddr::V6(v6) => v6_local(v6),
    }
}

/// One part of a numeric IPv4 host: decimal, `0x` hex or leading-zero octal.
fn ipv4_part(p: &str) -> Option<u64> {
    if p.is_empty() || p.len() > 34 {
        return None;
    }
    let (digits, radix) = if let Some(h) = p.strip_prefix("0x").or_else(|| p.strip_prefix("0X")) {
        (h, 16)
    } else if p.len() > 1 && p.starts_with('0') {
        (&p[1..], 8)
    } else {
        (p, 10)
    };
    if digits.is_empty() {
        return (radix == 16).then_some(0);
    }
    u64::from_str_radix(digits, radix)
        .ok()
        .filter(|&n| n <= u32::MAX as u64)
}

/// A host read as an IPv4 address the way `inet_aton` and URL parsers do: one to four
/// parts, the last filling the remaining bytes.
pub(crate) fn parse_ipv4_loose(host: &str) -> Option<Ipv4Addr> {
    let host = host.strip_suffix('.').unwrap_or(host);
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let nums: Vec<u64> = parts.iter().map(|p| ipv4_part(p)).collect::<Option<_>>()?;
    let (last, head) = nums.split_last()?;
    if head.iter().any(|&n| n > 255) {
        return None;
    }
    let room = 8 * (4 - head.len() as u32);
    if room < 32 && *last >= 1u64 << room {
        return None;
    }
    let mut v: u64 = 0;
    for (i, n) in head.iter().enumerate() {
        v |= n << (24 - 8 * i);
    }
    Some(Ipv4Addr::from((v | last) as u32))
}

/// Does the name embed a local IPv4 address as four dotted labels (`10.0.0.5.example`) or
/// four dashed parts of one label (`ip-10-0-0-5`)?
fn embeds_local_ipv4(labels: &[&str]) -> bool {
    let quad_local = |parts: &[&str]| {
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
            && parts
                .iter()
                .map(|p| p.parse::<u8>().ok())
                .collect::<Option<Vec<u8>>>()
                .is_some_and(|o| v4_local(Ipv4Addr::new(o[0], o[1], o[2], o[3])))
    };
    labels.windows(4).any(quad_local)
        || labels.iter().any(|l| {
            let dashed: Vec<&str> = l.split('-').collect();
            dashed.windows(4).any(quad_local)
        })
}

/// Decode `%xx` escapes (URL parsers decode them in hosts); anything malformed is kept.
pub(crate) fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = b
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Is this host (as written in a URL, without brackets or port) local?
pub(crate) fn host_is_local(host: &str) -> bool {
    let h = percent_decoded(host).to_ascii_lowercase();
    let h = h.trim_end_matches('.');
    if h.is_empty() {
        return true;
    }
    if h.contains(':') {
        // An IPv6 literal, perhaps with a zone id (`fe80::1%eth0`). Anything else with a
        // colon is not a valid host: treat it as local rather than guess.
        let addr = h.split('%').next().unwrap_or("");
        return addr.parse::<Ipv6Addr>().map_or(true, v6_local);
    }
    if let Some(v4) = parse_ipv4_loose(h) {
        return v4_local(v4);
    }
    let labels: Vec<&str> = h.split('.').collect();
    // A name ending in a number is read as an IPv4 address; one that isn't valid can't be
    // trusted to be public.
    if labels.last().is_some_and(|l| {
        !l.is_empty()
            && (l.bytes().all(|b| b.is_ascii_digit())
                || l.strip_prefix("0x")
                    .is_some_and(|x| x.bytes().all(|b| b.is_ascii_hexdigit())))
    }) {
        return true;
    }
    labels.len() == 1
        || LOCAL_SUFFIXES.iter().any(|s| h.ends_with(s))
        || WILDCARD_DNS
            .iter()
            .any(|d| h == *d || h.strip_suffix(d).is_some_and(|p| p.ends_with('.')))
        || embeds_local_ipv4(&labels)
}

/// The host of a URL authority (`user@host:port`, `[v6%zone]:port`).
pub(crate) fn authority_host(authority: &str) -> &str {
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    }
}

/// Does the text contain a local IPv4 address in dotted-quad form, or a local IPv6 address,
/// on its own (not as part of a longer number such as a version `10.0.0.5.1`)? Only strict
/// forms are looked for in free text, so titles like "10.5" or "127.1" are left alone, and
/// `0.x.x.x` (version-like, and never reachable) is ignored.
pub(crate) fn has_local_ip_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let v4_char = |c: u8| c.is_ascii_digit() || c == b'.';
    let v6_char = |c: u8| c.is_ascii_hexdigit() || c == b':' || c == b'.';
    let mut i = 0;
    while i < b.len() {
        // IPv4: a maximal run of digits and dots, not glued to letters or digits.
        if b[i].is_ascii_digit()
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.'))
        {
            let end = (i..b.len()).find(|&j| !v4_char(b[j])).unwrap_or(b.len());
            // A sentence's full stop is not part of the address.
            let run = s[i..end].trim_end_matches('.');
            let glued = b.get(end).is_some_and(|c| c.is_ascii_alphabetic());
            let parts: Vec<&str> = run.split('.').collect();
            if !glued
                && parts.len() == 4
                && let Ok(ip) = run.parse::<Ipv4Addr>()
                && ip.octets()[0] != 0
                && v4_local(ip)
            {
                return true;
            }
        }
        // IPv6: a maximal run of hex digits, colons and dots with at least two colons.
        if v6_char(b[i]) && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b':')) {
            let end = (i..b.len()).find(|&j| !v6_char(b[j])).unwrap_or(b.len());
            let run = &s[i..end];
            if run.matches(':').count() >= 2 {
                let glued = b
                    .get(end)
                    .is_some_and(|c| c.is_ascii_alphanumeric() && *c != b'%');
                if !glued
                    && let Ok(ip) = run.parse::<Ipv6Addr>()
                    && !ip.is_unspecified()
                    && v6_local(ip)
                {
                    return true;
                }
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_is_parsed_like_inet_aton() {
        let ip = |s: &str| parse_ipv4_loose(s).map(|a| a.to_string());
        for (s, want) in [
            ("127.0.0.1", "127.0.0.1"),
            ("2130706433", "127.0.0.1"),
            ("0x7f000001", "127.0.0.1"),
            ("0X7F.1", "127.0.0.1"),
            ("127.1", "127.0.0.1"),
            ("10.0.1", "10.0.0.1"),
            ("0177.0.0.1", "127.0.0.1"),
            ("012.0.0.1", "10.0.0.1"),
            ("0", "0.0.0.0"),
            ("0x", "0.0.0.0"),
            ("8.8.8.8.", "8.8.8.8"),
            ("4294967295", "255.255.255.255"),
        ] {
            assert_eq!(ip(s).as_deref(), Some(want), "{s}");
        }
        for s in [
            "",
            "256.0.0.1",
            "1.2.3.4.5",
            "4294967296",
            "1.16777216",
            "08.0.0.1",
            "0x1g",
            "a.b.c.d",
            "1..2",
            "example.com",
        ] {
            assert_eq!(ip(s), None, "{s}");
        }
    }

    #[test]
    fn hosts_are_classified() {
        for local in [
            "nas",
            "LOCALHOST",
            "localhost.",
            "nas.example.lan",
            "printer.local",
            "foo.localhost",
            "router.home.arpa",
            "10.0.0.5",
            "100.64.0.1",
            "198.19.255.255",
            "169.254.0.1",
            "172.31.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fe80::1%eth0",
            "fe80::1%25eth0",
            "fd00::1",
            "fec0::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "::10.0.0.1",
            "64:ff9b::10.0.0.1",
            "10%2e0%2e0%2e5",
            "10.0.0.5.nip.io",
            "anything.sslip.io",
            "nip.io",
            "LOCALTEST.ME",
            "x.lvh.me",
            "10-0-0-5.example.com",
            "ip-192-168-1-2.eu-west-1.compute.example",
            "a.192.168.1.2.example.com",
            "example.999",
            "nas:9000",
            "",
        ] {
            assert!(host_is_local(local), "{local}");
        }
        for public in [
            "example.com",
            "cdn.example.com",
            "8.8.8.8",
            "134744072",
            "100.128.0.1",
            "172.32.0.1",
            "2001:db8::1",
            "::ffff:8.8.8.8",
            "ec2-54-1-2-3.compute.example.com",
            "notnip.io",
            "music.example.com",
            "1.2.3.4.example.com",
        ] {
            assert!(!host_is_local(public), "{public}");
        }
    }

    #[test]
    fn authorities_are_split() {
        assert_eq!(authority_host("user:pw@10.0.0.1:80"), "10.0.0.1");
        assert_eq!(authority_host("[fe80::1%25eth0]:9000"), "fe80::1%25eth0");
        assert_eq!(authority_host("example.com"), "example.com");
        assert_eq!(authority_host("[unterminated"), "unterminated");
    }

    #[test]
    fn literals_in_text_are_found_strictly() {
        for yes in [
            "10.0.0.5",
            "at 10.0.0.5:9000.",
            "(192.168.1.1)",
            "on 10.0.0.5.",
            "[::1]:9000",
            "fe80::1%eth0",
            "x fd00::5 y",
            "::ffff:127.0.0.1",
        ] {
            assert!(has_local_ip_literal(yes), "{yes}");
        }
        for no in [
            "10.0.0.5.1",
            "v10.0.0.5",
            "10.0.0.5a",
            "1.10.0.0.5",
            "0.0.0.0",
            "0.9.1.2",
            "8.8.8.8",
            "10:30",
            "12:34:56",
            "02:4c:41:00:00:01",
            "::",
            "2001:db8::1",
            "10.5",
            "127.1",
            "deadbeef::1",
        ] {
            assert!(!has_local_ip_literal(no), "{no}");
        }
    }

    #[test]
    fn arbitrary_input_never_panics() {
        let mut x: u64 = 0x1234_5678_9abc_def1;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let atoms = [
            "1",
            "0",
            "0x",
            "255",
            "256",
            "4294967296",
            ".",
            "..",
            ":",
            "::",
            "%",
            "%2e",
            "%zz",
            "[",
            "]",
            "@",
            "-",
            "a",
            "fe80",
            "ffff",
            "é",
            "€",
            "nip.io",
            "lan",
            "10",
            " ",
            "/",
            "99999999999999999999999999",
        ];
        for _ in 0..50_000 {
            let n = (next() % 12) as usize;
            let s: String = (0..n)
                .map(|_| atoms[(next() % atoms.len() as u64) as usize])
                .collect();
            let _ = host_is_local(&s);
            let _ = parse_ipv4_loose(&s);
            let _ = authority_host(&s);
            let _ = has_local_ip_literal(&s);
        }
    }
}
