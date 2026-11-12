//! `anywhere-cli`: native test player for the Lyrion Anywhere core.
//!
//! Connects straight to an LMS over Slimproto (TCP 3483) and plays through the default
//! audio device, or a silent real-time sink with `--null`. Used to validate the core's
//! player behaviour before it runs through the gateway or in the browser.
//!
//! ```text
//! anywhere-cli --server lms.example.lan [--name "Anywhere CLI"] [--mac 02:4c:41:00:00:01] [--null]
//! ANYWHERE_PASSWORD=… anywhere-cli --gateway https://music.example.com --user NAME [--ca-file ca.pem] [--null]
//! ```
//!
//! With `--gateway`, the player connects through a Lyrion Anywhere gateway instead of
//! directly. The password comes from `ANYWHERE_PASSWORD` or standard input (not echoed at a
//! terminal), never argv. Plain `http://` is only for a gateway on this machine, named by
//! its loopback address; redirects are never followed. Text from either server is printed
//! with control and invisible characters escaped.

mod gateway;

use std::io::{IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anywhere_core::player::{Action, Player, PlayerConfig, StreamHost};
use anywhere_core::proto::{direct_stream_target, format_mac, parse_mac};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

enum Event {
    Control(Vec<u8>),
    ControlClosed,
    StreamHeaders(u64, Vec<u8>),
    StreamData(u64, Vec<u8>),
    StreamEnd(u64),
    StreamError(u64, String),
}

struct Args {
    server: String,
    gateway: Option<String>,
    user: Option<String>,
    ca_file: Option<String>,
    name: String,
    mac: [u8; 6],
    codecs: Vec<String>,
    null: bool,
    verbose: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        server: String::new(),
        gateway: None,
        user: None,
        ca_file: None,
        name: "Anywhere CLI".into(),
        mac: [0x02, 0x4c, 0x41, 0x00, 0x00, 0x01],
        codecs: vec!["flc".into(), "mp3".into(), "ogg".into()],
        null: false,
        verbose: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--server" => a.server = it.next().ok_or("--server needs a value")?,
            "--gateway" => a.gateway = Some(it.next().ok_or("--gateway needs a value")?),
            "--user" => a.user = Some(it.next().ok_or("--user needs a value")?),
            "--ca-file" => a.ca_file = Some(it.next().ok_or("--ca-file needs a value")?),
            "--name" => a.name = it.next().ok_or("--name needs a value")?,
            "--mac" => {
                let v = it.next().ok_or("--mac needs a value")?;
                a.mac = parse_mac(&v).ok_or("bad --mac")?;
            }
            "--codecs" => {
                a.codecs = it
                    .next()
                    .ok_or("--codecs needs a value")?
                    .split(',')
                    .map(String::from)
                    .collect()
            }
            "--null" => a.null = true,
            "-v" | "--verbose" => a.verbose = true,
            "-h" | "--help" => {
                println!(
                    "anywhere-cli (--server HOST[:PORT] | --gateway URL --user NAME [--ca-file F]) \
                     [--name NAME] [--mac 02:..] [--codecs flc,mp3,ogg] [--null] [-v]\n\
                     With --gateway the password is read from $ANYWHERE_PASSWORD or standard input\n\
                     (not echoed at a terminal); http:// only for 127.0.0.1 or [::1]."
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.server.is_empty() == a.gateway.is_none() {
        return Err("give exactly one of --server or --gateway".into());
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}", shown(&e));
            std::process::exit(2);
        }
    };
    if let Err(e) = run(args) {
        eprintln!("error: {}", shown(&e));
        std::process::exit(1);
    }
}

/// Most characters of a server's text printed.
const SHOWN_MAX: usize = 300;

/// Text that came from a server (LMS or the gateway), made safe to print: control and
/// invisible characters (terminal escape sequences, bidirectional overrides) are printed
/// escaped instead of reaching the terminal, and it is cut at [`SHOWN_MAX`] characters.
pub(crate) fn shown(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i == SHOWN_MAX {
            out.push('…');
            break;
        }
        if anywhere_core::names::invisible(c) {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

/// Most bytes of a stream's response headers (LMS sends a few hundred). A server that sends
/// more before ending them is refused rather than buffered without end.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Splits a stream's response headers from its body as the bytes arrive.
#[derive(Default)]
struct HeaderSplit {
    head: Vec<u8>,
}

/// A stream's response headers, and the start of its body.
type HeadersAndBody = (Vec<u8>, Vec<u8>);

impl HeaderSplit {
    /// Feed the next bytes: `Ok(None)` while the headers are still arriving, then once
    /// `Ok(Some((headers, start of the body)))`. Headers longer than [`MAX_HEADER_BYTES`]
    /// are an error.
    fn push(&mut self, data: &[u8]) -> Result<Option<HeadersAndBody>, String> {
        // The end may straddle the previous read: look from three bytes back.
        let from = self.head.len().saturating_sub(3);
        self.head.extend_from_slice(data);
        let end = self.head[from..]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| from + i + 4);
        match end {
            Some(end) if end <= MAX_HEADER_BYTES => {
                let body = self.head.split_off(end);
                Ok(Some((std::mem::take(&mut self.head), body)))
            }
            None if self.head.len() < MAX_HEADER_BYTES => Ok(None),
            _ => Err(format!(
                "the stream's response headers are longer than {MAX_HEADER_BYTES} bytes"
            )),
        }
    }
}

/// Runs `stty` with the given arguments on the terminal (standard input) and returns what
/// it printed.
type Stty<'a> = dyn Fn(&[&str]) -> Result<String, String> + 'a;

fn stty(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("stty")
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("stty: {e}"))?;
    if !out.status.success() {
        return Err("stty failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The terminal's echo, off until this is dropped, so a password typed at the prompt isn't
/// shown. Made only once the terminal's settings are saved and echo is off.
struct NoEcho<'a> {
    saved: String,
    stty: &'a Stty<'a>,
}

impl<'a> NoEcho<'a> {
    fn on(stty: &'a Stty<'a>) -> Result<NoEcho<'a>, String> {
        let saved = stty(&["-g"])?;
        // `stty -g` prints its own settings format, which is passed back as one argument:
        // anything else (which could read as an option) is not.
        let plain = |c: char| c.is_ascii_alphanumeric() || c == ':' || c == '=';
        if saved.is_empty() || !saved.chars().all(plain) {
            return Err("stty: unexpected settings".into());
        }
        stty(&["-echo"])?;
        Ok(NoEcho { saved, stty })
    }
}

impl Drop for NoEcho<'_> {
    fn drop(&mut self) {
        let _ = (self.stty)(&[&self.saved]);
    }
}

fn resolve(server: &str, default_port: u16) -> Result<SocketAddr, String> {
    let with_port = if server.contains(':') {
        server.to_string()
    } else {
        format!("{server}:{default_port}")
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("resolve {with_port}: {e}"))?
        .next()
        .ok_or_else(|| format!("no address for {with_port}"))
}

enum Control {
    Direct { sock: TcpStream, addr: SocketAddr },
    Gateway(gateway::GatewayConn),
}

/// The password: `$ANYWHERE_PASSWORD`, or a line of standard input. Typed at a terminal,
/// it isn't echoed; where echo can't be turned off, it isn't asked for.
fn read_password() -> Result<String, String> {
    if let Ok(p) = std::env::var("ANYWHERE_PASSWORD") {
        return Ok(p);
    }
    let stdin = std::io::stdin();
    let terminal = stdin.is_terminal();
    let _quiet = if terminal {
        Some(NoEcho::on(&stty).map_err(|e| {
            format!("{e}: can't hide the password as it is typed; set ANYWHERE_PASSWORD")
        })?)
    } else {
        None
    };
    eprint!("password: ");
    let mut line = String::new();
    stdin.read_line(&mut line).map_err(|e| e.to_string())?;
    if terminal {
        eprintln!(); // the Enter wasn't echoed
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn run(args: Args) -> Result<(), String> {
    let epoch = Instant::now();
    let now = move || epoch.elapsed().as_millis() as u64;

    let (tx, rx) = mpsc::channel::<Event>();
    let mut control = if let Some(url) = &args.gateway {
        let user = args.user.as_deref().ok_or("--gateway needs --user")?;
        let conn = gateway::connect(
            url,
            user,
            &read_password()?,
            args.ca_file.as_deref(),
            tx.clone(),
        )?;
        println!(
            "connected via gateway {url} as {} ({}), assigned by the gateway",
            conn.name,
            format_mac(&conn.mac)
        );
        Control::Gateway(conn)
    } else {
        let addr = resolve(&args.server, 3483)?;
        let sock = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map_err(|e| format!("connect {addr}: {e}"))?;
        sock.set_nodelay(true).ok();
        spawn_control_reader(sock.try_clone().map_err(|e| e.to_string())?, tx.clone());
        println!(
            "connected to {addr} as {} ({})",
            args.name,
            format_mac(&args.mac)
        );
        Control::Direct { sock, addr }
    };

    // Through the gateway, the player identity is the one it assigned to this device.
    let (mac, name) = match &control {
        Control::Gateway(g) => (g.mac, g.name.clone()),
        Control::Direct { .. } => (args.mac, args.name.clone()),
    };
    let player = Arc::new(Mutex::new(Player::new(PlayerConfig {
        mac,
        uuid: [0; 16],
        name,
        codecs: args.codecs.clone(),
        max_sample_rate: 192_000,
        firmware: env!("CARGO_PKG_VERSION").into(),
    })));
    {
        let mut p = player.lock().unwrap();
        // Verbose runs print the core's diagnostics (as the web app's Connection log does).
        p.set_diagnostics(args.verbose);
        p.connected(now());
    }

    let mut audio = Audio::new(player.clone(), args.null);
    let mut stream: Option<StreamHandle> = None;
    let mut stream_id = 0u64;
    let mut last_status = Instant::now();

    loop {
        let event = rx.recv_timeout(Duration::from_millis(10));
        let t = now();
        let mut p = player.lock().unwrap();
        match event {
            Ok(Event::Control(b)) => p.on_control_data(&b, t)?,
            Ok(Event::ControlClosed) => return Err("server closed the control connection".into()),
            Ok(Event::StreamHeaders(id, h)) if id == stream_id => p.on_stream_headers(&h, t),
            Ok(Event::StreamData(id, d)) if id == stream_id => p.on_stream_data(&d, t),
            Ok(Event::StreamEnd(id)) if id == stream_id => p.on_stream_end(t),
            Ok(Event::StreamError(id, e)) if id == stream_id => {
                eprintln!("stream error: {}", shown(&e));
                p.on_stream_error(t)
            }
            _ => {}
        }
        // Drain any further queued events before ticking.
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::Control(b) => p.on_control_data(&b, t)?,
                Event::ControlClosed => return Err("server closed the control connection".into()),
                Event::StreamHeaders(id, h) if id == stream_id => p.on_stream_headers(&h, t),
                Event::StreamData(id, d) if id == stream_id => p.on_stream_data(&d, t),
                Event::StreamEnd(id) if id == stream_id => p.on_stream_end(t),
                Event::StreamError(id, e) if id == stream_id => {
                    eprintln!("stream error: {}", shown(&e));
                    p.on_stream_error(t)
                }
                _ => {}
            }
        }
        p.tick(t);
        if args.verbose {
            for d in p.take_diagnostics() {
                println!("diag {d:?}");
            }
        }

        for action in p.take_actions() {
            match action {
                Action::Send(bytes) => {
                    if args.verbose {
                        log_client_frame(&bytes);
                    }
                    match &mut control {
                        Control::Direct { sock, .. } => sock
                            .write_all(&bytes)
                            .map_err(|e| format!("control write: {e}"))?,
                        Control::Gateway(g) => g.send(bytes)?,
                    }
                }
                Action::OpenStream {
                    host,
                    port,
                    request,
                    stream_number,
                    ..
                } => {
                    if let Some(s) = stream.take() {
                        s.cancel.store(true, Ordering::Relaxed);
                    }
                    stream_id += 1;
                    match &control {
                        Control::Direct {
                            addr: control_addr, ..
                        } => {
                            let named = match host {
                                StreamHost::Server => [0; 4],
                                StreamHost::Ip(b) => b,
                            };
                            // Only the server we are connected to, and only our own stream
                            // (as the gateway enforces): never wherever the server says.
                            let addr = match direct_stream_target(
                                named,
                                port,
                                &request,
                                &mac,
                                control_addr.ip(),
                            ) {
                                Ok(a) => a,
                                Err(e) => {
                                    eprintln!("refused stream request: {}", shown(&e));
                                    p.on_stream_error(t);
                                    continue;
                                }
                            };
                            if args.verbose {
                                // The server's request line: printed made safe.
                                let line = request.split(|b| *b == b'\r').next().unwrap_or(&[]);
                                println!(
                                    "-> stream {addr} {}",
                                    shown(&String::from_utf8_lossy(line))
                                );
                            }
                            stream = Some(spawn_stream(stream_id, addr, request, tx.clone()));
                        }
                        Control::Gateway(g) => {
                            if args.verbose {
                                println!("-> gateway stream #{stream_number}");
                            }
                            stream = Some(g.spawn_stream(stream_id, stream_number, tx.clone()));
                        }
                    }
                }
                Action::CloseStream => {
                    if let Some(s) = stream.take() {
                        s.cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        }

        if let Some(s) = &stream {
            s.want.store(p.wants_stream_data(), Ordering::Relaxed);
        }
        let rate = p.output_rate();
        let status = p.status();
        drop(p);

        audio.ensure_rate(rate)?;

        if last_status.elapsed() >= Duration::from_secs(2) {
            last_status = Instant::now();
            println!(
                "[{:?}] elapsed {:.1}s, buffered {:.1}s, input {} KiB, {} Hz, vol {:.2}{}",
                status.output,
                status.elapsed_ms as f64 / 1000.0,
                status.buffered_ms as f64 / 1000.0,
                status.input_bytes / 1024,
                status.sample_rate,
                status.volume.0,
                status
                    .last_error
                    .map(|e| format!(", last error: {}", shown(&e)))
                    .unwrap_or_default()
            );
        }
    }
}

fn log_client_frame(bytes: &[u8]) {
    let op = String::from_utf8_lossy(&bytes[0..4]);
    if op == "STAT" && bytes.len() >= 12 {
        let ev = String::from_utf8_lossy(&bytes[8..12]);
        if ev != "STMt" {
            println!("<- STAT {ev}");
        }
    } else {
        println!("<- {op}");
    }
}

fn spawn_control_reader(mut sock: TcpStream, tx: Sender<Event>) {
    thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match sock.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(Event::ControlClosed);
                    return;
                }
                Ok(n) => {
                    if tx.send(Event::Control(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

struct StreamHandle {
    want: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

/// Fetch an LMS stream: send the request verbatim, split off the response headers.
fn spawn_stream(id: u64, addr: SocketAddr, request: Vec<u8>, tx: Sender<Event>) -> StreamHandle {
    let want = Arc::new(AtomicBool::new(true));
    let cancel = Arc::new(AtomicBool::new(false));
    let (w, c) = (want.clone(), cancel.clone());
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut sock = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            sock.set_read_timeout(Some(Duration::from_secs(30))).ok();
            sock.write_all(&request).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 64 * 1024];
            let mut headers = HeaderSplit::default();
            let mut in_headers = true;
            loop {
                if c.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if !w.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                let n = sock.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    let _ = tx.send(Event::StreamEnd(id));
                    return Ok(());
                }
                if in_headers {
                    if let Some((head, body)) = headers.push(&buf[..n])? {
                        in_headers = false;
                        let _ = tx.send(Event::StreamHeaders(id, head));
                        if !body.is_empty() {
                            let _ = tx.send(Event::StreamData(id, body));
                        }
                    }
                } else if tx.send(Event::StreamData(id, buf[..n].to_vec())).is_err() {
                    return Ok(());
                }
            }
        })();
        if let Err(e) = result {
            let _ = tx.send(Event::StreamError(id, e));
        }
    });
    StreamHandle { want, cancel }
}

/// Audio output at the player's current sample rate.
struct Audio {
    player: Arc<Mutex<Player>>,
    null: bool,
    rate: Option<u32>,
    stream: Option<cpal::Stream>,
    null_stop: Option<Arc<AtomicBool>>,
}

impl Audio {
    fn new(player: Arc<Mutex<Player>>, null: bool) -> Self {
        Audio {
            player,
            null,
            rate: None,
            stream: None,
            null_stop: None,
        }
    }

    fn ensure_rate(&mut self, rate: Option<u32>) -> Result<(), String> {
        let Some(rate) = rate else { return Ok(()) };
        if self.rate == Some(rate) {
            return Ok(());
        }
        self.stream = None;
        if let Some(stop) = self.null_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        println!(
            "audio output at {rate} Hz{}",
            if self.null { " (null sink)" } else { "" }
        );
        if self.null {
            self.null_stop = Some(spawn_null_sink(self.player.clone(), rate));
        } else {
            self.stream = Some(open_device(self.player.clone(), rate)?);
        }
        self.rate = Some(rate);
        Ok(())
    }
}

fn open_device(player: Arc<Mutex<Player>>, rate: u32) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default audio output device")?;
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate: rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out: &mut [f32], _| {
                let mut p = player.lock().unwrap();
                let n = p.pull_audio(out);
                p.played(n as u64);
                out[n * 2..].fill(0.0);
            },
            |e| eprintln!("audio error: {e}"),
            None,
        )
        .map_err(|e| format!("open audio device at {rate} Hz: {e}"))?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Consumes audio in real time without a sound device (for headless tests).
fn spawn_null_sink(player: Arc<Mutex<Player>>, rate: u32) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    thread::spawn(move || {
        let start = Instant::now();
        let mut consumed: u64 = 0;
        let mut buf = vec![0f32; 4096 * 2];
        while !s.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
            let due = start.elapsed().as_micros() as u64 * rate as u64 / 1_000_000;
            while consumed < due {
                let want = ((due - consumed) as usize).min(4096);
                let mut p = player.lock().unwrap();
                let n = p.pull_audio(&mut buf[..want * 2]);
                p.played(n as u64);
                // Silence still advances the clock, as a real device would.
                consumed += want as u64;
            }
        }
    });
    stop
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Security (review low): server strings were printed raw, so a hostile server could
    /// send terminal escape sequences (retitle or clear the terminal, hide text) or
    /// bidirectional overrides. They are printed escaped, and long ones cut.
    #[test]
    fn server_text_is_printed_safely() {
        let hostile = "Kitchen\u{1b}]0;owned\u{7}\u{1b}[2J\r\nx\u{202e}gpj.exe\u{200b}";
        let s = shown(hostile);
        assert!(!s.chars().any(anywhere_core::names::invisible), "{s:?}");
        assert!(s.starts_with("Kitchen\\u{1b}]0;owned\\u{7}"), "{s:?}");
        assert!(s.contains("\\u{202e}gpj.exe"), "{s:?}");
        assert_eq!(shown("Sigur Rós – 日本"), "Sigur Rós – 日本");
        let long = shown(&"é".repeat(10_000));
        assert_eq!(long.chars().count(), SHOWN_MAX + 1);
        assert!(long.ends_with('…'));
    }

    /// Security (review low): in direct mode the stream's response headers were buffered
    /// until the blank line, with no limit, so a server that never ended them filled memory.
    #[test]
    fn stream_headers_are_bounded() {
        let head = b"HTTP/1.0 200 OK\r\nContent-Type: audio/flac\r\n\r\n";
        // The end of the headers in every place across two reads.
        for cut in 0..=head.len() {
            let mut h = HeaderSplit::default();
            let first = h.push(&head[..cut]).unwrap();
            let (headers, mut body) = match first {
                Some(done) => {
                    assert_eq!(cut, head.len());
                    done
                }
                None => h.push(&head[cut..]).unwrap().expect("the headers end"),
            };
            if cut == head.len() {
                assert!(body.is_empty());
            }
            body.extend_from_slice(b"fLaC");
            assert_eq!(headers, head.to_vec(), "cut at {cut}");
            assert_eq!(body, b"fLaC".to_vec(), "cut at {cut}");
        }
        let mut h = HeaderSplit::default();
        let (headers, body) = h.push(b"HTTP/1.0 200 OK\r\n\r\nfLaC").unwrap().unwrap();
        assert_eq!((headers.len(), body), (19, b"fLaC".to_vec()));
        // Headers that never end are refused once past the limit, not kept for ever.
        let mut h = HeaderSplit::default();
        let mut pushed = 0;
        let refused = loop {
            match h.push(&[b'x'; 4096]) {
                Ok(None) => pushed += 4096,
                Ok(Some(_)) => panic!("no end was sent"),
                Err(e) => break e,
            }
            assert!(pushed <= MAX_HEADER_BYTES, "still buffering");
        };
        assert!(refused.contains("longer than"), "{refused}");
        // Headers that end past the limit are refused too.
        let mut big = vec![b'x'; MAX_HEADER_BYTES];
        big.extend_from_slice(b"\r\n\r\nbody");
        assert!(HeaderSplit::default().push(&big).is_err());
    }

    /// Security (review low): the password was echoed as it was typed. Echo is turned off
    /// for the prompt and the terminal's settings put back after; if echo can't be turned
    /// off, the password isn't asked for.
    #[test]
    fn the_password_prompt_hides_what_is_typed() {
        let calls = RefCell::new(Vec::new());
        let fake = |args: &[&str]| -> Result<String, String> {
            calls.borrow_mut().push(args.join(" "));
            Ok(if args == ["-g"] {
                "500:5:bf:8a3b:3:1c".into()
            } else {
                String::new()
            })
        };
        {
            let _quiet = NoEcho::on(&fake).unwrap();
            assert_eq!(*calls.borrow(), ["-g", "-echo"]);
        }
        assert_eq!(*calls.borrow(), ["-g", "-echo", "500:5:bf:8a3b:3:1c"]);
        // Echo can't be turned off, or the settings can't be saved: no prompt at all.
        let echo_stays = |args: &[&str]| -> Result<String, String> {
            if args == ["-echo"] {
                Err("stty failed".into())
            } else {
                Ok("500:5".into())
            }
        };
        assert!(NoEcho::on(&echo_stays).is_err());
        let no_terminal = |_: &[&str]| -> Result<String, String> { Err("stty failed".into()) };
        assert!(NoEcho::on(&no_terminal).is_err());
        // A saved state that isn't stty's own format is never passed back to it.
        for odd in ["", "-echo raw", "500:5 ; x", "a\nb"] {
            let calls = RefCell::new(Vec::new());
            let fake = |args: &[&str]| -> Result<String, String> {
                calls.borrow_mut().push(args.join(" "));
                Ok(odd.to_string())
            };
            assert!(NoEcho::on(&fake).is_err(), "{odd:?}");
            assert_eq!(*calls.borrow(), ["-g"], "{odd:?}");
        }
    }
}
