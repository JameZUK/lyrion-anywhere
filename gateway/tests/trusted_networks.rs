//! Trusted networks (`--trusted-network`, docs/plans/trusted-networks.md) against the real
//! `anywhere-gateway serve`, bound so that requests arrive from this host's own LAN address,
//! a non-loopback peer: plain HTTP from there is refused by default, admitted once that
//! network is trusted, still refused the moment a proxy header is sent, and the refresh
//! cookie is set without `Secure` only on such a request while loopback keeps its `Secure`
//! cookie. A network that isn't private stops the gateway from starting.
#![cfg(target_os = "linux")]

mod common;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_anywhere-gateway");
const USER: &str = "alice";
const PW: &str = "correct horse battery staple";
const CLIENT: (&str, &str) = ("x-anywhere-client", "1");

/// A port free a moment ago on `ip`.
fn free_port(ip: IpAddr) -> u16 {
    TcpListener::bind((ip, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// This host's own non-loopback address (the one its default route would use; asking
/// sends nothing).
fn own_lan_address() -> IpAddr {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    s.connect("192.0.2.1:9")
        .expect("this test needs a host with a non-loopback IPv4 address");
    let ip = s.local_addr().unwrap().ip();
    assert!(
        !ip.is_loopback() && !ip.is_unspecified(),
        "this test needs a host with a non-loopback IPv4 address"
    );
    ip
}

struct Gw {
    child: Child,
    data: PathBuf,
}

impl Drop for Gw {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

/// An account to sign in with, made the way an admin would before the plugin is set up.
fn create_user(data: &Path) {
    let mut child = Command::new(BIN)
        .arg("admin")
        .arg("--data-dir")
        .arg(data)
        .args(["create-user", USER])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PW}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "create-user: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `anywhere-gateway serve` listening on `listen`, with its log on standard output (piped,
/// read once it is stopped) and an account to sign in with.
fn start(listen: SocketAddr, probe: SocketAddr, extra: &[&str]) -> Gw {
    let data = common::scratch_dir("anywhere-trusted-networks", "data");
    create_user(&data);
    let admin: SocketAddr = format!("127.0.0.1:{}", free_port(IpAddr::V4(Ipv4Addr::LOCALHOST)))
        .parse()
        .unwrap();
    let child = Command::new(BIN)
        .arg("serve")
        .arg("--data-dir")
        .arg(&data)
        .arg("--listen")
        .arg(listen.to_string())
        .arg("--admin-listen")
        .arg(admin.to_string())
        .args(["--no-transcode", "--no-analysis", "--no-tunnel"])
        .args(extra)
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let gw = Gw { child, data };
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if TcpStream::connect(probe).is_ok() && gw.data.join("admin.secret").exists() {
            return gw;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the gateway never started");
}

impl Gw {
    /// Stop the gateway and return everything it logged.
    fn stop_and_read_log(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut log = String::new();
        if let Some(mut out) = self.child.stdout.take() {
            let _ = out.read_to_string(&mut log);
        }
        log
    }
}

/// A plain HTTP/1.1 request (closing the connection), the whole answer as text.
fn http(at: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut s = TcpStream::connect(at).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nhost: {at}\r\nconnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() {
        req.push_str(&format!(
            "content-type: application/json\r\ncontent-length: {}\r\n",
            body.len()
        ));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

fn get(at: SocketAddr, path: &str, headers: &[(&str, &str)]) -> String {
    http(at, "GET", path, headers, "")
}

fn login(at: SocketAddr) -> String {
    http(
        at,
        "POST",
        "/v1/auth/login",
        &[CLIENT],
        &format!(r#"{{"username":"{USER}","password":"{PW}","device_name":"Test browser"}}"#),
    )
}

/// The `Set-Cookie` header of an answer, if any (header names arrive in lower case).
fn set_cookie(answer: &str) -> Option<&str> {
    let head = answer.split_once("\r\n\r\n")?.0;
    head.lines()
        .find_map(|l| l.strip_prefix("set-cookie:"))
        .map(str::trim)
}

#[test]
fn plain_http_from_the_lan_is_refused_by_default() {
    let lan = own_lan_address();
    let public = SocketAddr::new(lan, free_port(lan));
    let _gw = start(public, public, &[]);
    for path in ["/healthz", "/v1/me", "/"] {
        let out = get(public, path, &[]);
        assert!(out.starts_with("HTTP/1.1 403"), "{path}: {out}");
        assert!(out.ends_with("TLS required\n"), "{path}: {out}");
    }
    let out = login(public);
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    assert!(set_cookie(&out).is_none(), "{out}");
}

#[test]
fn a_trusted_network_admits_plain_http_from_inside_it_and_nothing_a_peer_sends_widens_it() {
    let lan = own_lan_address();
    // Bound to every interface, so that the same gateway can be asked from loopback (a
    // peer outside the trusted network) and from the LAN address (inside it).
    let port = free_port(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let any = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let from_lan = SocketAddr::new(lan, port);
    let from_loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let network = format!("{lan}/32");
    let mut gw = start(any, from_loopback, &["--trusted-network", &network]);

    // Inside the trusted network: served, and signing in sets a cookie a browser on plain
    // HTTP will keep (no `Secure`), otherwise unchanged.
    let out = get(from_lan, "/healthz", &[]);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let out = login(from_lan);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let cookie = set_cookie(&out).expect("a refresh cookie");
    assert!(cookie.contains("Path=/v1/auth; Max-Age="), "{cookie}");
    assert!(cookie.ends_with("HttpOnly; SameSite=Strict"), "{cookie}");
    assert!(!cookie.contains("Secure"), "{cookie}");

    // A proxy header from inside the trusted network: not a trusted proxy, so refused, as
    // every direct peer always was.
    for headers in [
        &[("x-forwarded-for", "203.0.113.9")][..],
        &[("x-forwarded-proto", "https")],
        &[("x-forwarded-proto", "http")],
        &[
            ("x-forwarded-for", "127.0.0.1"),
            ("x-forwarded-proto", "https"),
        ],
    ] {
        let out = get(from_lan, "/healthz", headers);
        assert!(out.starts_with("HTTP/1.1 403"), "{headers:?}: {out}");
    }

    // Loopback, outside the trusted network: served as ever, with the `Secure` cookie it
    // always had.
    let out = login(from_loopback);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let cookie = set_cookie(&out).expect("a refresh cookie");
    assert!(
        cookie.ends_with("HttpOnly; Secure; SameSite=Strict"),
        "{cookie}"
    );
    let out = get(from_loopback, "/healthz", &[("x-forwarded-proto", "http")]);
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");

    // The gateway said so when it started.
    let log = gw.stop_and_read_log();
    assert!(
        log.contains(
            "unencrypted (plain HTTP) connections are accepted from these trusted networks"
        ) && log.contains(&network),
        "{log}"
    );
}

#[test]
fn a_network_that_is_not_private_stops_the_gateway() {
    for (bad, why) in [
        ("8.8.8.0/24", "not a private network"),
        ("203.0.113.7", "not a private network"),
        ("10.0.0.0/7", "not a private network"),
        ("2001:db8::/32", "not a private network"),
        ("127.0.0.0/8", "loopback"),
        ("0.0.0.0/0", "loopback"),
        ("::/0", "loopback"),
        ("::ffff:10.0.0.0/104", "written as IPv6"),
        ("10.8.0.0/33", "prefix length"),
        ("10.8.0/24", "not a network"),
        ("music.example.com", "not a network"),
        ("10.8.0.0/24,10.9.0.0/24", "not a network"),
    ] {
        let data = common::scratch_dir("anywhere-trusted-networks", "refused");
        let out = Command::new(BIN)
            .arg("serve")
            .arg("--data-dir")
            .arg(&data)
            .args([
                "--listen",
                "127.0.0.1:0",
                "--no-tunnel",
                "--trusted-network",
                bad,
            ])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&data);
        assert!(!out.status.success(), "{bad}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--trusted-network"), "{bad}: {err}");
        assert!(err.contains(why), "{bad}: {err}");
    }
}

/// `--trusted-network` is repeatable, and each value is normalised to its network address.
#[test]
fn several_networks_are_accepted_and_normalised() {
    let lan = own_lan_address();
    let public = SocketAddr::new(lan, free_port(lan));
    let IpAddr::V4(v4) = lan else {
        panic!("an IPv4 LAN address");
    };
    // A /24 written with host bits set, holding this host's address, plus two others. The
    // host octet is never 0: a runner at .56 once wrapped 200 + 56 to .0, the network address
    // itself, and the "normalised, not as typed" check below failed (android-v1.17.1's tag run).
    let [a, b, c, d] = v4.octets();
    let with_host_bits = format!("{a}.{b}.{c}.{}/24", 200u8.wrapping_add(d).max(1));
    let mut gw = start(
        public,
        public,
        &[
            "--trusted-network",
            &with_host_bits,
            "--trusted-network",
            "fd12:3456:789a::/48",
            "--trusted-network",
            "100.64.0.0/10",
        ],
    );
    let out = get(public, "/healthz", &[]);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let log = gw.stop_and_read_log();
    assert!(log.contains(&format!("{a}.{b}.{c}.0/24")), "{log}");
    assert!(log.contains("fd12:3456:789a::/48"), "{log}");
    assert!(log.contains("100.64.0.0/10"), "{log}");
    assert!(
        !log.contains(&with_host_bits),
        "normalised, not as typed: {log}"
    );
}
