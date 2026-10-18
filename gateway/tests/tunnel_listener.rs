//! The built-in Cloudflare Tunnel's own listener (`--tunnel-listen`), against the real
//! `anywhere-gateway serve`: opened only when the tunnel is on, loopback only, a busy port
//! leaves the tunnel unsupported with a sentence saying why, and the public listener keeps
//! its per-address connection cap on a non-loopback address.
#![cfg(target_os = "linux")]

mod common;

use std::io::Read;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_anywhere-gateway");

/// A loopback port free a moment ago.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
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
    public: SocketAddr,
    admin: SocketAddr,
}

impl Drop for Gw {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn start(public: SocketAddr, tunnel: Option<&str>, extra: &[&str]) -> Gw {
    let data = common::scratch_dir("anywhere-tunnel-listener", "data");
    let admin: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let mut cmd = Command::new(BIN);
    cmd.arg("serve")
        .arg("--data-dir")
        .arg(&data)
        .arg("--listen")
        .arg(public.to_string())
        .arg("--admin-listen")
        .arg(admin.to_string())
        .args(["--no-transcode", "--no-analysis"])
        .args(extra)
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(t) = tunnel {
        cmd.arg("--tunnel-listen").arg(t);
    }
    let child = cmd.spawn().unwrap();
    let gw = Gw {
        child,
        data,
        public,
        admin,
    };
    gw.wait_up();
    gw
}

impl Gw {
    fn wait_up(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if TcpStream::connect(self.public).is_ok()
                && TcpStream::connect(self.admin).is_ok()
                && self.data.join("admin.secret").exists()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the gateway never started");
    }

    fn tunnel_status(&self) -> Value {
        let secret = std::fs::read_to_string(self.data.join("admin.secret")).unwrap();
        let out = http_get(
            self.admin,
            "/admin/v1/tunnel",
            &[("authorization", &format!("Bearer {}", secret.trim()))],
        );
        let body = out.split_once("\r\n\r\n").expect("an HTTP answer").1;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        serde_json::from_str(body).unwrap()
    }
}

/// A plain HTTP/1.1 GET (closing the connection), the whole answer as text.
fn http_get(at: SocketAddr, path: &str, headers: &[(&str, &str)]) -> String {
    use std::io::Write;
    let mut s = TcpStream::connect(at).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("GET {path} HTTP/1.1\r\nhost: {at}\r\nconnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

fn loopback(port: u16) -> SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}

#[test]
fn the_tunnel_listener_serves_the_gateway_and_the_status_names_it() {
    let tunnel = loopback(free_port());
    let gw = start(loopback(free_port()), Some(&tunnel.to_string()), &[]);
    let out = http_get(tunnel, "/healthz", &[]);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let v = gw.tunnel_status();
    assert_eq!(v["tunnel_listen"], tunnel.to_string(), "{v}");
    assert_eq!(v["error"], Value::Null, "{v}");
}

#[test]
fn no_tunnel_opens_no_tunnel_listener() {
    let tunnel = loopback(free_port());
    let gw = start(
        loopback(free_port()),
        Some(&tunnel.to_string()),
        &["--no-tunnel"],
    );
    let e = TcpStream::connect(tunnel).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::ConnectionRefused, "{e}");
    let v = gw.tunnel_status();
    assert_eq!(v["state"], "unsupported");
    assert_eq!(v["tunnel_listen"], Value::Null);
}

#[test]
fn a_busy_tunnel_port_leaves_the_gateway_running_and_the_tunnel_unsupported() {
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = held.local_addr().unwrap();
    let gw = start(loopback(free_port()), Some(&busy.to_string()), &[]);
    let out = http_get(gw.public, "/healthz", &[]);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let v = gw.tunnel_status();
    assert_eq!(v["supported"], false, "{v}");
    assert_eq!(v["state"], "unsupported", "{v}");
    assert_eq!(v["tunnel_listen"], Value::Null, "{v}");
    let e = v["error"].as_str().expect("an error sentence");
    assert!(
        e.starts_with(&format!(
            "The tunnel's own listener could not be opened on {busy} ("
        )) && e.ends_with('.'),
        "{e}"
    );
    drop(held);
}

#[test]
fn a_tunnel_listener_on_the_public_port_costs_only_the_tunnel() {
    let both = loopback(free_port());
    let gw = start(both, Some(&both.to_string()), &[]);
    let out = http_get(gw.public, "/healthz", &[]);
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let v = gw.tunnel_status();
    assert_eq!(v["state"], "unsupported", "{v}");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains(&both.to_string())),
        "{v}"
    );
}

#[test]
fn a_non_loopback_tunnel_listener_is_refused_at_start_up() {
    for bad in ["0.0.0.0:9444", "10.0.0.10:9444", "[::]:9444"] {
        let data = common::scratch_dir("anywhere-tunnel-listener", "refused");
        let out = Command::new(BIN)
            .arg("serve")
            .arg("--data-dir")
            .arg(&data)
            .args(["--listen", "127.0.0.1:0", "--tunnel-listen", bad])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&data);
        assert!(!out.status.success(), "{bad}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("--tunnel-listen must be a loopback address"),
            "{bad}: {err}"
        );
    }
}

/// Regression: with the tunnel on, a non-loopback public listen address used to be exempt
/// from the per-address connection cap for the gateway's whole run.
#[test]
fn the_public_listener_keeps_its_per_address_cap_on_a_non_loopback_address() {
    let lan = own_lan_address();
    let port = {
        let l = TcpListener::bind((lan, 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    let public = SocketAddr::new(lan, port);
    let gw = start(public, Some(&loopback(free_port()).to_string()), &[]);
    // wait_up's probe connections are closed; give the gateway a moment to count them out.
    std::thread::sleep(Duration::from_millis(200));
    // The default cap is 64 connections from one address; this host's own address is not
    // exempt, tunnel or not.
    let held: Vec<TcpStream> = (0..64)
        .map(|_| TcpStream::connect(public).unwrap())
        .collect();
    // The 64 are served: an idle one stays open.
    let mut first = held[0].try_clone().unwrap();
    first
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let mut b = [0u8; 1];
    assert!(
        first.read(&mut b).is_err(),
        "an allowed connection was closed"
    );
    // The 65th is closed at once.
    let mut extra = TcpStream::connect(public).unwrap();
    extra
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    match extra.read(&mut b) {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("the 65th connection was kept: {other:?}"),
    }
    drop(held);
    drop(gw);
}
