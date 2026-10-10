//! Resource limits: artwork streaming and concurrency, event socket caps, LMS response
//! caps, download retries, and the server's header/body time limits and connection cap.

mod common;
use common::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use anywhere_gateway::limits::{self, ServeLimits};
use reqwest::StatusCode as S;
use serde_json::json;

const MIB: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// A scriptable fake LMS (raw HTTP, so it can misbehave in every way)
// ---------------------------------------------------------------------------

/// What the fake LMS does for one request: given the path and how many requests that path
/// has had before, write a raw HTTP response (or nothing, then close).
type Script = Arc<dyn Fn(&str, usize) -> Reply + Send + Sync>;

enum Reply {
    /// Close the connection without answering.
    Drop,
    /// Headers and an optional fixed body, then close.
    Full(String, Vec<u8>),
    /// Headers, then `n` chunks of `size` bytes (chunked encoding), then end.
    Chunked(String, usize, usize),
    /// A valid JSON-RPC answer of about `n` chunks of 64 KiB (chunked encoding).
    BigJson(usize),
    /// Headers and a first chunk, then nothing more, for ever.
    Stall(String),
}

struct RawLms {
    addr: SocketAddr,
    hits: Arc<Mutex<Vec<String>>>,
}

async fn raw_lms(script: Script) -> RawLms {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let hits: Arc<Mutex<Vec<String>>> = Default::default();
    let counts: Arc<Mutex<std::collections::HashMap<String, usize>>> = Default::default();
    let h = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let (script, h, counts) = (script.clone(), h.clone(), counts.clone());
            tokio::spawn(async move {
                let Some((path, _body)) = read_request(&mut s).await else {
                    return;
                };
                h.lock().unwrap().push(path.clone());
                let n = {
                    let mut c = counts.lock().unwrap();
                    let n = c.entry(path.clone()).or_default();
                    *n += 1;
                    *n - 1
                };
                match script(&path, n) {
                    Reply::Drop => {}
                    Reply::Full(head, body) => {
                        let _ = s.write_all(head.as_bytes()).await;
                        let _ = s.write_all(&body).await;
                    }
                    Reply::Chunked(head, n, size) => {
                        if s.write_all(head.as_bytes()).await.is_err() {
                            return;
                        }
                        let chunk = vec![b'x'; size];
                        for _ in 0..n {
                            let mut c = format!("{size:x}\r\n").into_bytes();
                            c.extend_from_slice(&chunk);
                            c.extend_from_slice(b"\r\n");
                            if s.write_all(&c).await.is_err() {
                                return;
                            }
                        }
                        let _ = s.write_all(b"0\r\n\r\n").await;
                    }
                    Reply::BigJson(n) => {
                        let h = head("200 OK", "application/json", None);
                        let mut parts = vec![h.into_bytes()];
                        let chunk = |b: &[u8]| {
                            let mut c = format!("{:x}\r\n", b.len()).into_bytes();
                            c.extend_from_slice(b);
                            c.extend_from_slice(b"\r\n");
                            c
                        };
                        parts.push(chunk(
                            br#"{"id":1,"result":{"count":1,"albums_loop":[{"id":7,"album":""#,
                        ));
                        for p in parts.drain(..) {
                            if s.write_all(&p).await.is_err() {
                                return;
                            }
                        }
                        let filler = chunk(&vec![b'x'; 64 * 1024]);
                        for _ in 0..n {
                            if s.write_all(&filler).await.is_err() {
                                return;
                            }
                        }
                        let _ = s.write_all(&chunk(br#""}]}}"#)).await;
                        let _ = s.write_all(b"0\r\n\r\n").await;
                    }
                    Reply::Stall(head) => {
                        let _ = s.write_all(head.as_bytes()).await;
                        let _ = s.write_all(b"4\r\nIMGD\r\n").await;
                        futures_util::future::pending::<()>().await;
                    }
                }
                let _ = s.shutdown().await;
            });
        }
    });
    RawLms { addr, hits }
}

/// Read one HTTP request: (path, body).
async fn read_request(s: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut b = [0u8; 4096];
    let end = loop {
        let n = s.read(&mut b).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&b[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let path = head.split(' ').nth(1)?.to_string();
    let len: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body = buf[end..].to_vec();
    while body.len() < len {
        let n = s.read(&mut b).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&b[..n]);
    }
    Some((path, body))
}

fn head(status: &str, ct: &str, len: Option<usize>) -> String {
    match len {
        Some(n) => format!(
            "HTTP/1.1 {status}\r\ncontent-type: {ct}\r\ncontent-length: {n}\r\nconnection: close\r\n\r\n"
        ),
        None => format!(
            "HTTP/1.1 {status}\r\ncontent-type: {ct}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
        ),
    }
}

fn json_reply(v: serde_json::Value) -> Reply {
    let body = json!({"id": 1, "method": "slim.request", "result": v})
        .to_string()
        .into_bytes();
    Reply::Full(head("200 OK", "application/json", Some(body.len())), body)
}

async fn with_lms(script: Script, mutate: impl FnOnce(&mut Config)) -> (Harness, RawLms) {
    let lms = raw_lms(script).await;
    let addr = lms.addr.to_string();
    let h = harness_with(move |c| {
        c.lms_http = addr;
        mutate(c);
    })
    .await;
    (h, lms)
}

async fn art(h: &Harness, token: &str, path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(h.url(&format!("/v1/art/{path}")))
        .bearer_auth(token)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// M1: artwork
// ---------------------------------------------------------------------------

/// Regression: the relay read the whole image into memory before sending any of it, so
/// an endless (or huge) image without Content-Length grew the gateway without bound.
/// Now it is streamed: the first bytes arrive while LMS is still sending.
#[tokio::test]
async fn artwork_is_streamed_not_buffered() {
    let script: Script = Arc::new(|_, _| Reply::Stall(head("200 OK", "image/jpeg", None)));
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = tokio::time::timeout(
        Duration::from_secs(3),
        art(&h, &h.tok_a, "music/12/cover.jpg"),
    )
    .await
    .expect("headers must arrive before LMS finishes");
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    assert!(
        r.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
    let mut body = r.bytes_stream();
    let first = tokio::time::timeout(Duration::from_secs(3), body.next())
        .await
        .expect("first chunk relayed at once")
        .unwrap()
        .unwrap();
    assert_eq!(first.as_ref(), b"IMGD");
}

/// An image without Content-Length that runs past the size limit is cut off (the client sees
/// a broken body), not buffered.
#[tokio::test]
async fn oversize_artwork_without_length_is_cut_off() {
    let n = anywhere_gateway::art::MAX_IMAGE / (64 * 1024) + 16;
    let script: Script =
        Arc::new(move |_, _| Reply::Chunked(head("200 OK", "image/png", None), n, 64 * 1024));
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = art(&h, &h.tok_a, "music/12/cover.png").await;
    if r.status() == S::OK {
        let mut got = 0usize;
        let mut body = r.bytes_stream();
        let mut failed = false;
        while let Some(c) = body.next().await {
            match c {
                Ok(c) => got += c.len(),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "an oversize image must not end cleanly");
        assert!(
            got <= anywhere_gateway::art::MAX_IMAGE,
            "{got} bytes relayed"
        );
    } else {
        assert_eq!(r.status(), S::BAD_GATEWAY);
    }
}

#[tokio::test]
async fn oversize_artwork_with_length_is_refused_up_front() {
    let script: Script = Arc::new(|_, _| {
        Reply::Full(
            head(
                "200 OK",
                "image/jpeg",
                Some(anywhere_gateway::art::MAX_IMAGE + 1),
            ),
            vec![],
        )
    });
    let (h, _lms) = with_lms(script, |_| {}).await;
    assert_eq!(
        art(&h, &h.tok_a, "music/12/cover.jpg").await.status(),
        S::BAD_GATEWAY
    );
}

#[tokio::test]
async fn artwork_keeps_its_length_and_policy_headers() {
    let script: Script =
        Arc::new(|_, _| Reply::Full(head("200 OK", "image/jpeg", Some(9)), b"IMAGEDATA".into()));
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = art(&h, &h.tok_a, "music/12/cover.jpg").await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["content-length"], "9");
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    assert_eq!(r.headers()["cache-control"], "private, max-age=86400");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"IMAGEDATA");
}

/// Regression: any number of artwork requests ran at once (60 large covers took 250 MB).
/// Now each user has a few slots; beyond them requests wait briefly, then get 429.
#[tokio::test]
async fn artwork_requests_are_limited_per_user() {
    let script: Script = Arc::new(|_, _| Reply::Stall(head("200 OK", "image/jpeg", None)));
    let (h, _lms) = with_lms(script, |c| {
        c.art_per_user = 2;
        c.art_queue_wait = Duration::from_millis(200);
    })
    .await;
    let held1 = art(&h, &h.tok_a, "music/1/cover.jpg").await;
    let held2 = art(&h, &h.tok_a, "music/2/cover.jpg").await;
    assert_eq!(held1.status(), S::OK);
    assert_eq!(held2.status(), S::OK);
    let third = art(&h, &h.tok_a, "music/3/cover.jpg").await;
    assert_eq!(third.status(), S::TOO_MANY_REQUESTS);
    assert!(third.headers().contains_key("retry-after"));
    // Another user is not affected.
    assert_eq!(art(&h, &h.tok_b, "music/4/cover.jpg").await.status(), S::OK);
    // A slot comes free when a response is dropped.
    drop(held1);
    let mut ok = false;
    for _ in 0..50 {
        if art(&h, &h.tok_a, "music/5/cover.jpg").await.status() == S::OK {
            ok = true;
            break;
        }
    }
    assert!(ok, "slot never freed");
    drop(held2);
}

#[tokio::test]
async fn artwork_requests_are_limited_overall() {
    let script: Script = Arc::new(|_, _| Reply::Stall(head("200 OK", "image/jpeg", None)));
    let (h, _lms) = with_lms(script, |c| {
        c.art_total = 2;
        c.art_queue_wait = Duration::from_millis(200);
    })
    .await;
    let _a = art(&h, &h.tok_a, "music/1/cover.jpg").await;
    let _b = art(&h, &h.tok_a, "music/2/cover.jpg").await;
    assert_eq!(
        art(&h, &h.tok_b, "music/3/cover.jpg").await.status(),
        S::TOO_MANY_REQUESTS
    );
}

/// A request that waits for a slot gets it when one frees up within the wait.
#[tokio::test]
async fn a_queued_artwork_request_is_served_when_a_slot_frees() {
    let script: Script = Arc::new(|p: &str, _| {
        if p.contains("/1/") {
            Reply::Stall(head("200 OK", "image/jpeg", None))
        } else {
            Reply::Full(head("200 OK", "image/jpeg", Some(4)), b"IMGD".into())
        }
    });
    let (h, _lms) = with_lms(script, |c| {
        c.art_per_user = 1;
        c.art_queue_wait = Duration::from_secs(3);
    })
    .await;
    let held = art(&h, &h.tok_a, "music/1/cover.jpg").await;
    let url = h.url("/v1/art/music/2/cover.jpg");
    let tok = h.tok_a.clone();
    let queued = tokio::spawn(async move {
        reqwest::Client::new()
            .get(url)
            .bearer_auth(tok)
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(held);
    assert_eq!(queued.await.unwrap(), S::OK);
}

// ---------------------------------------------------------------------------
// M4: event sockets
// ---------------------------------------------------------------------------

async fn events(h: &Harness, token: &str) -> Ws {
    let req = format!("ws://{}/v1/events", h.addr)
        .into_client_request()
        .unwrap();
    let (mut ws, _) = connect_async(req).await.unwrap();
    ws.send(Message::Text(token.to_string().into()))
        .await
        .unwrap();
    match next(&mut ws).await {
        Message::Text(t) => assert!(t.as_str().contains("subscribed")),
        m => panic!("{m:?}"),
    }
    ws
}

async fn expect_replaced(ws: &mut Ws) {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no close in time")
        {
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(f.code, CloseCode::Policy);
                assert_eq!(f.reason.as_str(), anywhere_gateway::events::REPLACED);
                return;
            }
            Some(Ok(Message::Ping(_))) => continue,
            other => panic!("expected a close, got {other:?}"),
        }
    }
}

async fn still_open(ws: &mut Ws) {
    ws.send(Message::Ping(Default::default())).await.unwrap();
    loop {
        match next(ws).await {
            Message::Pong(_) => return,
            Message::Ping(_) => continue,
            m => panic!("socket should be open, got {m:?}"),
        }
    }
}

/// Regression: one token could hold any number of event sockets (1,500 in a test). Now a
/// device beyond its limit loses its oldest socket, closed as "replaced" (1008, which the
/// web app doesn't reconnect after, so tabs can't evict each other in a loop).
#[tokio::test]
async fn event_sockets_are_capped_per_device() {
    let h = harness_with(|c| c.events_per_device = 2).await;
    let mut first = events(&h, &h.tok_a).await;
    let mut second = events(&h, &h.tok_a).await;
    let mut third = events(&h, &h.tok_a).await;
    expect_replaced(&mut first).await;
    still_open(&mut second).await;
    still_open(&mut third).await;
    wait_for(|| {
        h.gw.event_sockets(h.accounts.authenticate(&h.tok_a).unwrap().user_id) == 2
    })
    .await;
    // Another user's sockets are separate.
    let mut other = events(&h, &h.tok_b).await;
    still_open(&mut other).await;
    still_open(&mut second).await;
}

#[tokio::test]
async fn event_sockets_are_capped_per_user() {
    let h = harness_with(|c| {
        c.events_per_device = 10;
        c.events_per_user = 3;
    })
    .await;
    let mut tokens = vec![h.tok_a.clone()];
    for i in 0..3 {
        let t = h
            .accounts
            .login("alice", PW, None, &format!("extra-{i}"), None, "test")
            .unwrap();
        tokens.push(t.access);
    }
    let mut socks = Vec::new();
    for t in &tokens {
        socks.push(events(&h, t).await);
    }
    expect_replaced(&mut socks[0]).await;
    for s in socks.iter_mut().skip(1) {
        still_open(s).await;
    }
}

#[tokio::test]
async fn closed_event_sockets_free_their_place() {
    let h = harness_with(|c| c.events_per_device = 2).await;
    for _ in 0..5 {
        let mut ws = events(&h, &h.tok_a).await;
        ws.close(None).await.unwrap();
        wait_for(|| {
            h.gw.event_sockets(h.accounts.authenticate(&h.tok_a).unwrap().user_id) == 0
        })
        .await;
    }
    let mut a = events(&h, &h.tok_a).await;
    let mut b = events(&h, &h.tok_a).await;
    still_open(&mut a).await;
    still_open(&mut b).await;
}

// ---------------------------------------------------------------------------
// L5: LMS JSON-RPC responses
// ---------------------------------------------------------------------------

async fn rpc(h: &Harness) -> reqwest::Response {
    reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(&json!({ "cmd": ["albums", 0, 50, "tags:al"] }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .unwrap()
}

/// Regression: an LMS answer of any size was read whole (300 MB took over 1 GB), then
/// relayed. Now the read stops at the limit with 502.
#[tokio::test]
async fn oversize_lms_answers_are_refused_while_reading() {
    // A valid answer of 17 MiB, no Content-Length.
    let script: Script = Arc::new(|_, _| Reply::BigJson(17 * 16));
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = rpc(&h).await;
    assert_eq!(r.status(), S::BAD_GATEWAY);
}

#[tokio::test]
async fn oversize_lms_answers_are_refused_by_length() {
    // Says 64 MiB, sends a few bytes, then nothing: refused on the length alone.
    let script: Script =
        Arc::new(|_, _| Reply::Stall(head("200 OK", "application/json", Some(64 * MIB))));
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = tokio::time::timeout(Duration::from_secs(3), rpc(&h))
        .await
        .expect("refused without waiting for the body");
    assert_eq!(r.status(), S::BAD_GATEWAY);
}

/// Just under the limit is still relayed.
#[tokio::test]
async fn large_lms_answers_under_the_limit_pass() {
    let script: Script = Arc::new(|_, _| Reply::BigJson(15 * 16));
    let (h, _lms) = with_lms(script, |_| {}).await;
    assert_eq!(rpc(&h).await.status(), S::OK);
}

#[tokio::test]
async fn ordinary_lms_answers_still_pass() {
    let script: Script = Arc::new(|_, _| {
        json_reply(json!({"count": 1, "albums_loop": [{"id": 7, "album": "Album"}]}))
    });
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = rpc(&h).await;
    assert_eq!(r.status(), S::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v["result"]["albums_loop"][0]["album"], "Album");
}

// ---------------------------------------------------------------------------
// L9: download retries
// ---------------------------------------------------------------------------

const AUDIO: &[u8] = b"fLaCAUDIO-fLaCAUDIO-fLaCAUDIO";

fn track_json() -> serde_json::Value {
    json!({"count": 1, "titles_loop": [{"id": 1, "title": "Track", "artist": "Artist",
        "album": "Album", "type": "flc", "url": "file:///srv/music/a.flac",
        "filesize": AUDIO.len(), "modificationTime": "x"}]})
}

fn audio_reply() -> Reply {
    Reply::Full(
        head("200 OK", "audio/flac", Some(AUDIO.len())),
        AUDIO.to_vec(),
    )
}

async fn download(h: &Harness) -> reqwest::Response {
    reqwest::Client::new()
        .get(h.url("/v1/download/track/1"))
        .bearer_auth(&h.tok_a)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .unwrap()
}

/// Regression: right after aborted downloads LMS can fail the next request once (502);
/// the gateway passed that straight on. A failed GET is now tried once more.
#[tokio::test]
async fn a_download_lms_fails_once_is_retried() {
    let script: Script = Arc::new(|p: &str, n| match p {
        "/jsonrpc.js" => json_reply(track_json()),
        _ if n == 0 => Reply::Full(head("502 Bad Gateway", "text/plain", Some(0)), vec![]),
        _ => audio_reply(),
    });
    let (h, lms) = with_lms(script, |_| {}).await;
    let r = download(&h).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.bytes().await.unwrap().as_ref(), AUDIO);
    let tries = lms
        .hits
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.ends_with("/download"))
        .count();
    assert_eq!(tries, 2);
}

#[tokio::test]
async fn a_download_whose_connection_drops_is_retried() {
    let script: Script = Arc::new(|p: &str, n| match p {
        "/jsonrpc.js" => json_reply(track_json()),
        _ if n == 0 => Reply::Drop,
        _ => audio_reply(),
    });
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = download(&h).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.bytes().await.unwrap().as_ref(), AUDIO);
}

#[tokio::test]
async fn a_track_lookup_lms_fails_once_is_retried() {
    let script: Script = Arc::new(|p: &str, n| match p {
        "/jsonrpc.js" if n == 0 => Reply::Full(
            head("503 Service Unavailable", "text/plain", Some(0)),
            vec![],
        ),
        "/jsonrpc.js" => json_reply(track_json()),
        _ => audio_reply(),
    });
    let (h, _lms) = with_lms(script, |_| {}).await;
    let r = reqwest::Client::new()
        .get(h.url("/v1/download/track/1/info"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
}

/// Only one retry: a server that keeps failing gets 502 after two tries, not a loop.
#[tokio::test]
async fn a_download_is_retried_only_once() {
    let script: Script = Arc::new(|p: &str, _| match p {
        "/jsonrpc.js" => json_reply(track_json()),
        _ => Reply::Full(
            head("500 Internal Server Error", "text/plain", Some(0)),
            vec![],
        ),
    });
    let (h, lms) = with_lms(script, |_| {}).await;
    assert_eq!(download(&h).await.status(), S::BAD_GATEWAY);
    let tries = lms
        .hits
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.ends_with("/download"))
        .count();
    assert_eq!(tries, 2);
}

/// A plain JSON-RPC command (which may change something) is never retried.
#[tokio::test]
async fn rpc_commands_are_not_retried() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let script: Script = Arc::new(move |_, _| {
        c.fetch_add(1, Ordering::SeqCst);
        Reply::Full(
            head("503 Service Unavailable", "text/plain", Some(0)),
            vec![],
        )
    });
    let (h, _lms) = with_lms(script, |_| {}).await;
    assert_eq!(rpc(&h).await.status(), S::BAD_GATEWAY);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// L6: header and body time limits, connection cap
// ---------------------------------------------------------------------------

/// The gateway served by [`limits::serve`] (as `main` does), with short limits.
async fn served(h: &Harness, limits: ServeLimits) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(limits::serve(
        l,
        router(h.gw.clone()),
        limits,
        std::future::pending(),
    ));
    addr
}

fn short() -> ServeLimits {
    ServeLimits {
        header_timeout: Duration::from_millis(300),
        ..ServeLimits::default()
    }
}

/// Wait for the server to close the connection; returns what it sent first.
async fn closed_within(s: &mut TcpStream, limit: Duration) -> Vec<u8> {
    let mut got = Vec::new();
    let mut b = [0u8; 1024];
    tokio::time::timeout(limit, async {
        loop {
            match s.read(&mut b).await {
                Ok(0) | Err(_) => return,
                Ok(n) => got.extend_from_slice(&b[..n]),
            }
        }
    })
    .await
    .expect("connection still open");
    got
}

/// Regression: a client could hold a connection open for ever by sending its request head
/// slowly (300 partial heads stayed open over 65 s).
#[tokio::test]
async fn slow_request_heads_are_cut_off() {
    let h = harness().await;
    let addr = served(&h, short()).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();
    // Trickle a header byte now and then: still cut off at the limit.
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = s.write_all(b"X").await;
    }
    closed_within(&mut s, Duration::from_secs(2)).await;
}

#[tokio::test]
async fn idle_keepalive_connections_are_closed() {
    let h = harness().await;
    let addr = served(&h, short()).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let got = closed_within(&mut s, Duration::from_secs(2)).await;
    assert!(String::from_utf8_lossy(&got).starts_with("HTTP/1.1 200"));
}

/// Regression: a slow request body held its connection (and handler) for 90 s.
#[tokio::test]
async fn slow_request_bodies_get_408() {
    let h = harness_with(|c| c.body_timeout = Duration::from_millis(300)).await;
    let addr = served(&h, ServeLimits::default()).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "POST /v1/rpc HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\n\
         Content-Type: application/json\r\nContent-Length: 100\r\n\r\n{{\"cmd\":",
        h.tok_a
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let got = closed_within(&mut s, Duration::from_secs(2)).await;
    assert!(
        String::from_utf8_lossy(&got).starts_with("HTTP/1.1 408"),
        "{}",
        String::from_utf8_lossy(&got)
    );
    assert!(
        h.lms.rpc_calls().is_empty(),
        "nothing reached LMS on a partial body"
    );
}

#[tokio::test]
async fn oversize_request_bodies_get_413() {
    let h = harness().await;
    let addr = served(&h, ServeLimits::default()).await;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .body(vec![b' '; limits::MAX_BODY + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn ordinary_requests_work_through_the_limited_server() {
    let h = harness().await;
    let addr = served(&h, short()).await;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/rpc"))
        .bearer_auth(&h.tok_a)
        .json(&json!({ "cmd": ["albums", 0, 50, "tags:al"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
}

/// WebSockets outlive the header and body time limits.
#[tokio::test]
async fn websockets_are_not_cut_by_the_time_limits() {
    let h = harness_with(|c| {
        c.body_timeout = Duration::from_millis(300);
        c.ws_keepalive = Duration::from_secs(60);
    })
    .await;
    let addr = served(&h, short()).await;
    let (mut ws, _) = connect_async(format!("ws://{addr}/v1/events"))
        .await
        .unwrap();
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    assert!(matches!(next(&mut ws).await, Message::Text(_)));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    still_open(&mut ws).await;
}

/// A download that takes longer than the limits is streamed to the end.
#[tokio::test]
async fn slow_downloads_are_not_cut_by_the_time_limits() {
    let h = harness_with(|c| c.body_timeout = Duration::from_millis(300)).await;
    let addr = served(&h, short()).await;
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/v1/download/track/1"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::OK);
    let mut body = r.bytes_stream();
    let mut got = Vec::new();
    while let Some(c) = body.next().await {
        got.extend_from_slice(&c.unwrap());
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert_eq!(got.len(), h.lms.audio.len());
}

#[tokio::test]
async fn connections_beyond_the_cap_wait() {
    let h = harness().await;
    let addr = served(
        &h,
        ServeLimits {
            max_connections: 2,
            ..ServeLimits::default()
        },
    )
    .await;
    let mut a = TcpStream::connect(addr).await.unwrap();
    let _b = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut b = [0u8; 64];
    assert!(
        tokio::time::timeout(Duration::from_millis(300), c.read(&mut b))
            .await
            .is_err(),
        "a third connection must not be served while two are open"
    );
    a.shutdown().await.unwrap();
    drop(a);
    let got = closed_within(&mut c, Duration::from_secs(3)).await;
    let all = [&b[..], &got[..]].concat();
    assert!(
        String::from_utf8_lossy(&all).contains("200 OK"),
        "{}",
        String::from_utf8_lossy(&all)
    );
}
