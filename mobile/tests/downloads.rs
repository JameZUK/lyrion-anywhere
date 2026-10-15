//! Offline downloads, end to end.
//!
//! - Against a **scripted gateway** (exact failure modes): resuming a cut download with
//!   `Range`, a 416, a `Content-Range` that starts elsewhere, a size that disagrees with
//!   `/info`, more data than announced, 401 (renew and retry), 403 (shown, permanent), 429
//!   (waits as told), pause and cancel, the storage cap, Wi-Fi only, and the cover cache.
//! - Against the **real gateway** in front of a fake LMS: a download through
//!   `/v1/download/track/{id}`, resuming with `Range` through it, the transcode refusal,
//!   refusals (no permission 403, unknown or non-library ids 404), and a device revoked
//!   mid-way.
//! - Plays recorded offline sent to `POST /v1/plays` (and kept by a server without it), and
//!   to the real gateway's endpoint: recorded once (a resent key is a duplicate), refused
//!   when the stable key names another file or too little was heard.

mod common;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_mobile::ApiError;
use anywhere_mobile::offline::store::scope_name;
use anywhere_mobile::{
    DownloadFailure, DownloadFormat, DownloadObserver, DownloadOrigin, DownloadPolicy,
    DownloadRequest, DownloadState, Downloader, DownloadsSnapshot, FormatChoice, NetworkKind,
    OfflineLibrary, OfflineTrackRef, Server, Session, resolve_offline_tracks, sync_plays,
};
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use common::*;
use serde_json::{Value, json};
use tokio::net::TcpListener;

/// The stable key the scripted gateway gives a track.
fn key_of(id: i64) -> String {
    format!("{id:064x}")
}

fn audio(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn temp_root() -> String {
    let d = std::env::temp_dir().join(format!(
        "anywhere-dl-{}",
        anywhere_mobile::offline::plays::new_key()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

// --- A scripted gateway ------------------------------------------------------------------

/// What the next download answer does (popped per audio request; `Normal` when empty).
#[derive(Clone, Debug)]
enum Step {
    /// Honour `Range` and send it all.
    Normal,
    /// Send this many bytes of the answer, then break the connection.
    CutAfter(usize),
    /// 416, whatever was asked.
    Unsatisfiable,
    /// 206 with a `Content-Range` that doesn't start where asked.
    WrongStart,
    /// A `Content-Length` that disagrees with `/info`.
    WrongLength,
    /// More bytes than `/info` announced (chunked, so no length to stop at).
    TooLong,
    /// 429 with `Retry-After: 1`.
    Busy,
    /// 403, downloads off for this account.
    Forbidden,
    /// 404.
    Missing,
    /// 503.
    Unavailable,
    /// Hold the answer until notified.
    Stall(Arc<tokio::sync::Notify>),
}

#[derive(Default)]
struct Script {
    file: Vec<u8>,
    /// `original.size` in `/info` (`None`: null, as for an unknown size).
    info_size: Option<u64>,
    steps: VecDeque<Step>,
    /// Refuse the first access token (401) once, so the client must renew.
    expire_first: bool,
    /// (path, Range header, bearer token) of every request.
    log: Vec<(String, Option<String>, String)>,
    /// `/v1/plays` answers 404.
    no_plays: bool,
    plays: Vec<Value>,
}

struct Fake {
    addr: SocketAddr,
    script: Arc<Mutex<Script>>,
}

fn json_resp(status: StatusCode, v: Value) -> Response {
    (
        status,
        [("content-type", "application/json")],
        v.to_string(),
    )
        .into_response()
}

fn issued(n: usize) -> Value {
    json!({"access_token": format!("access-{n}"), "refresh_token": format!("refresh-{n}"),
        "expires_in": 900, "user": {"username": "alice", "role": "listener"},
        "device": {"id": 1, "name": "Test phone"}})
}

impl Fake {
    async fn start(file: Vec<u8>, steps: Vec<Step>) -> Fake {
        let script = Arc::new(Mutex::new(Script {
            info_size: Some(file.len() as u64),
            file,
            steps: steps.into(),
            ..Script::default()
        }));
        let tokens = Arc::new(AtomicUsize::new(0));
        let s = script.clone();
        let app = Router::new().fallback(any(move |req: Request| {
            let (s, tokens) = (s.clone(), tokens.clone());
            async move { handle(s, tokens, req).await }
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Fake { addr, script }
    }

    async fn session(&self) -> Arc<Session> {
        let server = Server::new(format!("http://{}", self.addr)).unwrap();
        let s = Session::new(server, MemStore::new(), "Test phone".into()).unwrap();
        s.login("alice".into(), PW.into(), None).await.unwrap();
        s
    }

    fn audio_requests(&self) -> Vec<(Option<String>, String)> {
        self.script
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|(p, _, _)| p.starts_with("/v1/download/track/") && !p.ends_with("/info"))
            .map(|(_, r, t)| (r.clone(), t.clone()))
            .collect()
    }
}

async fn handle(s: Arc<Mutex<Script>>, tokens: Arc<AtomicUsize>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let headers: HeaderMap = req.headers().clone();
    let range = headers
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string();
    s.lock()
        .unwrap()
        .log
        .push((path.clone(), range.clone(), token.clone()));
    match path.as_str() {
        "/v1/auth/login" | "/v1/auth/refresh" => {
            let n = tokens.fetch_add(1, Ordering::SeqCst) + 1;
            return json_resp(StatusCode::OK, issued(n));
        }
        "/v1/plays" => {
            let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                .await
                .unwrap();
            let v: Value = serde_json::from_slice(&body).unwrap();
            let mut st = s.lock().unwrap();
            if st.no_plays {
                return json_resp(StatusCode::NOT_FOUND, json!({"error": "not found"}));
            }
            let out: Vec<Value> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|p| json!({"key": p["key"], "result": "recorded"}))
                .collect();
            st.plays.extend(v.as_array().unwrap().iter().cloned());
            return json_resp(StatusCode::OK, Value::Array(out));
        }
        _ => {}
    }
    if path.starts_with("/v1/art/") {
        return (
            [("content-type", "image/jpeg")],
            b"\xff\xd8\xff\xe0cover".to_vec(),
        )
            .into_response();
    }
    {
        let mut st = s.lock().unwrap();
        if st.expire_first && token == "access-1" {
            st.expire_first = false;
            return json_resp(
                StatusCode::UNAUTHORIZED,
                json!({"error": "invalid credentials"}),
            );
        }
    }
    let Some(rest) = path.strip_prefix("/v1/download/track/") else {
        return json_resp(StatusCode::NOT_FOUND, json!({"error": "not found"}));
    };
    if let Some(id) = rest.strip_suffix("/info") {
        let st = s.lock().unwrap();
        return json_resp(
            StatusCode::OK,
            json!({"id": id.parse::<i64>().unwrap(), "key": key_of(id.parse().unwrap()), "title": "Song", "artist": "Band",
                "album": "Record", "album_id": 7, "tracknum": 2, "duration": 61.0,
                "original": {"format": "flc", "content_type": "audio/flac", "size": st.info_size, "ext": "flac"},
                "formats": ["original", "mp3", "opus"]}),
        );
    }
    let (file, step) = {
        let mut st = s.lock().unwrap();
        (
            st.file.clone(),
            st.steps.pop_front().unwrap_or(Step::Normal),
        )
    };
    let total = file.len();
    let start = range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse::<usize>().ok());
    let partial = |from: usize| -> Response {
        let mut r = Response::new(Body::from(file[from..].to_vec()));
        *r.status_mut() = StatusCode::PARTIAL_CONTENT;
        let h = r.headers_mut();
        h.insert(
            "content-range",
            format!("bytes {from}-{}/{total}", total - 1)
                .parse()
                .unwrap(),
        );
        h.insert("content-type", "audio/flac".parse().unwrap());
        r
    };
    match step {
        Step::Normal => match start {
            Some(a) if a < total => partial(a),
            Some(_) => json_resp(
                StatusCode::RANGE_NOT_SATISFIABLE,
                json!({"error": "bad range"}),
            ),
            None => ([("content-type", "audio/flac")], file).into_response(),
        },
        Step::CutAfter(n) => {
            let from = start.unwrap_or(0);
            let piece = file[from..(from + n).min(total)].to_vec();
            // The piece, a moment for it to arrive, then the connection breaks.
            let stream = futures_util::StreamExt::chain(
                futures_util::stream::iter(vec![Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    piece,
                ))]),
                futures_util::stream::once(async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Err(std::io::Error::other("cut"))
                }),
            );
            let mut r = Response::new(Body::from_stream(stream));
            r.headers_mut().insert(
                "content-length",
                (total - from).to_string().parse().unwrap(),
            );
            if from > 0 {
                *r.status_mut() = StatusCode::PARTIAL_CONTENT;
                r.headers_mut().insert(
                    "content-range",
                    format!("bytes {from}-{}/{total}", total - 1)
                        .parse()
                        .unwrap(),
                );
            }
            r
        }
        Step::Unsatisfiable => json_resp(
            StatusCode::RANGE_NOT_SATISFIABLE,
            json!({"error": "bad range"}),
        ),
        Step::WrongStart => {
            let mut r = partial(0);
            r.headers_mut().insert(
                "content-range",
                format!("bytes 1-{}/{total}", total - 1).parse().unwrap(),
            );
            r
        }
        Step::WrongLength => (
            [("content-type", "audio/flac")],
            file[..total - 10].to_vec(),
        )
            .into_response(),
        Step::TooLong => {
            let mut more = file.clone();
            more.extend_from_slice(b"EXTRA");
            let stream = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(
                axum::body::Bytes::from(more),
            )]);
            // Chunked (no length): only `/info`'s size says how much should come.
            Response::new(Body::from_stream(stream))
        }
        Step::Busy => {
            let mut r = json_resp(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "too many downloads at once"}),
            );
            r.headers_mut().insert("retry-after", "1".parse().unwrap());
            r
        }
        Step::Forbidden => json_resp(
            StatusCode::FORBIDDEN,
            json!({"error": "downloads are not allowed for this account"}),
        ),
        Step::Missing => json_resp(StatusCode::NOT_FOUND, json!({"error": "not downloadable"})),
        Step::Unavailable => json_resp(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "music server unavailable"}),
        ),
        Step::Stall(n) => {
            n.notified().await;
            ([("content-type", "audio/flac")], file).into_response()
        }
    }
}

// --- Observer --------------------------------------------------------------------------

#[derive(Default)]
struct Seen {
    last: Mutex<Option<DownloadsSnapshot>>,
    library_changes: AtomicUsize,
}

impl DownloadObserver for Seen {
    fn on_downloads(&self, snapshot: DownloadsSnapshot) {
        *self.last.lock().unwrap() = Some(snapshot);
    }

    fn on_library_changed(&self) {
        self.library_changes.fetch_add(1, Ordering::SeqCst);
    }
}

impl Seen {
    fn snap(&self) -> DownloadsSnapshot {
        self.last.lock().unwrap().clone().expect("a snapshot")
    }

    fn state_of(&self, id: i64) -> Option<DownloadState> {
        let s = self.last.lock().unwrap().clone()?;
        s.items.iter().find(|i| i.track_id == id).map(|i| i.state)
    }
}

async fn wait_state(seen: &Seen, id: i64, want: DownloadState, secs: u64) {
    for _ in 0..secs * 50 {
        if seen.state_of(id) == Some(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "track {id} never reached {want:?}; last: {:?}",
        seen.last.lock().unwrap()
    );
}

fn manual(id: i64) -> DownloadRequest {
    DownloadRequest {
        track_id: id,
        album_id: None,
        origin: DownloadOrigin::Manual,
    }
}

struct Setup {
    fake: Fake,
    lib: Arc<OfflineLibrary>,
    seen: Arc<Seen>,
    dl: Arc<Downloader>,
    root: String,
    /// The server address as the session normalised it (the library's scope).
    url: String,
}

async fn setup(file: Vec<u8>, steps: Vec<Step>, policy: DownloadPolicy) -> Setup {
    let fake = Fake::start(file, steps).await;
    let session = fake.session().await;
    let root = temp_root();
    let url = session.server().url();
    let lib = OfflineLibrary::open(root.clone(), url.clone(), "alice".into()).unwrap();
    let seen = Arc::new(Seen::default());
    let dl = Downloader::new(
        session,
        lib.clone(),
        policy,
        NetworkKind::Wifi,
        seen.clone(),
    )
    .unwrap();
    Setup {
        fake,
        lib,
        seen,
        dl,
        root,
        url,
    }
}

fn partial_path(s: &Setup, url: &str, id: i64, format: &str) -> PathBuf {
    PathBuf::from(&s.root)
        .join(scope_name(url, "alice"))
        .join("partial")
        .join(format!("{}.{format}", key_of(id)))
}

fn stored(s: &Setup) -> Vec<u8> {
    let t = s.lib.tracks()[0].clone();
    std::fs::read(
        PathBuf::from(&s.root)
            .join(scope_name(&s.url, "alice"))
            .join("tracks")
            .join(format!("{}.{}", t.key, t.ext)),
    )
    .unwrap()
}

// --- Scripted cases ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_download_resumes_with_range_and_is_stored_whole() {
    let file = audio(300_000);
    let s = setup(
        file.clone(),
        vec![Step::CutAfter(100_000)],
        DownloadPolicy::default(),
    )
    .await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 20).await;
    assert_eq!(stored(&s), file);
    let reqs = s.fake.audio_requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].0, None);
    assert_eq!(
        reqs[1].0.as_deref(),
        Some("bytes=100000-"),
        "resumed, not restarted"
    );
    let t = &s.lib.tracks()[0];
    assert_eq!(
        (t.track_id, t.size, t.format),
        (5, 300_000, DownloadFormat::Original)
    );
    assert_eq!(
        (t.title.as_str(), t.album_id, t.ext.as_str()),
        ("Song", Some(7), "flac")
    );
    // The cover was cached for offline browsing, and the item reports the whole size.
    assert!(s.lib.artwork("a7".into()).is_some());
    let item = s.seen.snap().items[0].clone();
    assert_eq!((item.received, item.total), (300_000, 300_000));
    assert!(s.seen.library_changes.load(Ordering::SeqCst) >= 1);
    // Nothing but the stable key and a fixed extension names the file; no token anywhere.
    let scope = PathBuf::from(&s.root).join(scope_name(&s.url, "alice"));
    for name in walk(&scope) {
        assert!(
            !name.contains("access-") && !name.contains("refresh-"),
            "{name}"
        );
    }
    let index = std::fs::read_to_string(scope.join("index.json")).unwrap();
    assert!(!index.contains("access-") && !index.contains("127.0.0.1"));
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        out.push(e.file_name().to_string_lossy().into_owned());
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_416_or_a_misplaced_range_starts_again() {
    let file = audio(50_000);
    let s = setup(
        file.clone(),
        vec![Step::CutAfter(20_000), Step::Unsatisfiable, Step::Normal],
        DownloadPolicy::default(),
    )
    .await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 20).await;
    assert_eq!(stored(&s), file);
    let reqs = s.fake.audio_requests();
    assert_eq!(reqs[1].0.as_deref(), Some("bytes=20000-"));
    assert_eq!(reqs[2].0, None, "after a 416, from the start");

    let s = setup(
        file.clone(),
        vec![Step::CutAfter(20_000), Step::WrongStart, Step::Normal],
        DownloadPolicy::default(),
    )
    .await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 20).await;
    assert_eq!(stored(&s), file, "never spliced at the wrong place");
    assert_eq!(s.fake.audio_requests()[2].0, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sizes_that_disagree_are_never_kept() {
    let file = audio(40_000);
    let s = setup(
        file.clone(),
        vec![Step::WrongLength, Step::TooLong, Step::Normal],
        DownloadPolicy::default(),
    )
    .await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 30).await;
    assert_eq!(stored(&s), file);
    assert_eq!(s.fake.audio_requests().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_token_is_renewed_once() {
    let file = audio(10_000);
    let s = setup(file.clone(), vec![], DownloadPolicy::default()).await;
    s.fake.script.lock().unwrap().expire_first = true;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 10).await;
    assert_eq!(stored(&s), file);
    let log = s.fake.script.lock().unwrap().log.clone();
    assert!(log.iter().any(|(p, ..)| p == "/v1/auth/refresh"));
    assert!(s.fake.audio_requests().iter().all(|(_, t)| t == "access-2"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_permission_fails_for_good_and_is_reported() {
    let s = setup(
        audio(1000),
        vec![Step::Forbidden; 4],
        DownloadPolicy::default(),
    )
    .await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Failed, 10).await;
    let snap = s.seen.snap();
    assert!(snap.forbidden);
    assert_eq!(snap.items[0].failure, Some(DownloadFailure::NotAllowed));
    assert_eq!(
        snap.items[0].message,
        "Your account isn't allowed to download."
    );
    assert_eq!(s.fake.audio_requests().len(), 1, "a 403 is never retried");
    assert!(s.lib.tracks().is_empty());

    let s = setup(audio(1000), vec![Step::Missing], DownloadPolicy::default()).await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Failed, 10).await;
    assert_eq!(
        s.seen.snap().items[0].failure,
        Some(DownloadFailure::NotDownloadable)
    );
    assert!(!s.seen.snap().forbidden);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_many_downloads_waits_as_told() {
    let file = audio(5000);
    let s = setup(
        file.clone(),
        vec![Step::Busy, Step::Unavailable],
        DownloadPolicy::default(),
    )
    .await;
    let started = std::time::Instant::now();
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Waiting, 5).await;
    assert!(s.seen.snap().items[0].message.contains("busy"));
    wait_state(&s.seen, 5, DownloadState::Done, 20).await;
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "Retry-After honoured"
    );
    assert_eq!(stored(&s), file);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_keeps_the_bytes_and_cancel_throws_them_away() {
    let file = audio(80_000);
    let s = setup(
        file.clone(),
        vec![Step::CutAfter(30_000)],
        DownloadPolicy::default(),
    )
    .await;
    let url = s.url.clone();
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    // The first piece arrives, then the connection breaks: pause while it waits to retry.
    wait_for("the partial file", || {
        std::fs::metadata(partial_path(&s, &url, 5, "original")).is_ok_and(|m| m.len() == 30_000)
    })
    .await;
    s.dl.pause(Some(5));
    wait_state(&s.seen, 5, DownloadState::Paused, 5).await;
    assert!(
        partial_path(&s, &url, 5, "original").exists(),
        "kept for resuming"
    );
    // Resume: it carries on from 30,000.
    s.dl.resume(Some(5));
    wait_state(&s.seen, 5, DownloadState::Done, 20).await;
    assert_eq!(stored(&s), file);
    assert_eq!(
        s.fake.audio_requests().last().unwrap().0.as_deref(),
        Some("bytes=30000-")
    );

    // Cancel mid-way: the partial file goes.
    let s = setup(
        file,
        vec![
            Step::CutAfter(30_000),
            Step::Stall(Arc::new(tokio::sync::Notify::new())),
        ],
        DownloadPolicy::default(),
    )
    .await;
    let url = s.url.clone();
    s.dl.enqueue(vec![manual(6)], FormatChoice::Original);
    wait_for("the partial file", || {
        partial_path(&s, &url, 6, "original").exists()
    })
    .await;
    s.dl.cancel(Some(6));
    wait_for("the cancel", || {
        s.seen.snap().items.is_empty() && !partial_path(&s, &url, 6, "original").exists()
    })
    .await;
    assert!(s.lib.tracks().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_storage_cap_holds_and_only_smart_downloads_make_room() {
    let policy = DownloadPolicy {
        cap_bytes: 25_000,
        ..DownloadPolicy::default()
    };
    let s = setup(audio(10_000), vec![], policy).await;
    let smart = |id| DownloadRequest {
        track_id: id,
        album_id: None,
        origin: DownloadOrigin::Recent,
    };
    let ids = |lib: &OfflineLibrary| {
        let mut v: Vec<i64> = lib.tracks().iter().map(|t| t.track_id).collect();
        v.sort();
        v
    };
    // Two smart downloads fit; a third doesn't, and never pushes the others out.
    s.dl.enqueue(vec![smart(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 10).await;
    s.dl.enqueue(vec![smart(6)], FormatChoice::Original);
    wait_state(&s.seen, 6, DownloadState::Done, 10).await;
    s.dl.enqueue(vec![smart(7)], FormatChoice::Original);
    wait_state(&s.seen, 7, DownloadState::Failed, 10).await;
    let item = s
        .seen
        .snap()
        .items
        .into_iter()
        .find(|i| i.track_id == 7)
        .unwrap();
    assert_eq!(item.failure, Some(DownloadFailure::LimitReached));
    assert_eq!(ids(&s.lib), vec![5, 6]);
    // The user's own downloads make room by removing smart ones, oldest first.
    s.dl.enqueue(vec![manual(8)], FormatChoice::Original);
    wait_state(&s.seen, 8, DownloadState::Done, 10).await;
    assert_eq!(ids(&s.lib), vec![6, 8]);
    s.dl.enqueue(vec![manual(9)], FormatChoice::Original);
    wait_state(&s.seen, 9, DownloadState::Done, 10).await;
    assert_eq!(ids(&s.lib), vec![8, 9]);
    // … but never remove the user's own: the cap is reached.
    s.dl.enqueue(vec![manual(10)], FormatChoice::Original);
    wait_state(&s.seen, 10, DownloadState::Failed, 10).await;
    assert_eq!(ids(&s.lib), vec![8, 9]);
    assert!(s.lib.used_bytes() <= 25_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wifi_only_waits_on_mobile_data_and_formats_follow_the_network() {
    let file = audio(4000);
    let policy = DownloadPolicy {
        wifi_only: true,
        wifi_format: DownloadFormat::Original,
        mobile_format: DownloadFormat::Opus,
        cap_bytes: 0,
    };
    let s = setup(file.clone(), vec![], policy.clone()).await;
    s.dl.set_network(NetworkKind::Mobile);
    s.dl.enqueue(vec![manual(5)], FormatChoice::Auto);
    wait_for("waiting for Wi-Fi", || {
        s.seen
            .last
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|x| x.waiting_for_network)
    })
    .await;
    assert!(
        s.fake.audio_requests().is_empty(),
        "nothing fetched on mobile data"
    );
    // Mobile data allowed: "auto" picks the mobile format.
    s.dl.set_policy(DownloadPolicy {
        wifi_only: false,
        ..policy
    });
    wait_state(&s.seen, 5, DownloadState::Done, 10).await;
    let log = s.fake.script.lock().unwrap().log.clone();
    assert!(log.iter().any(|(p, ..)| p == "/v1/download/track/5"));
    assert_eq!(s.lib.tracks()[0].format, DownloadFormat::Opus);
    assert_eq!(s.lib.tracks()[0].ext, "opus");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_queue_survives_a_restart() {
    let file = audio(20_000);
    let gate = Arc::new(tokio::sync::Notify::new());
    let s = setup(
        file.clone(),
        vec![Step::Stall(gate.clone()), Step::Stall(gate.clone())],
        DownloadPolicy::default(),
    )
    .await;
    s.dl.enqueue(
        vec![manual(5), manual(6), manual(7)],
        FormatChoice::Original,
    );
    wait_state(&s.seen, 7, DownloadState::Queued, 5).await;
    s.dl.pause(Some(7));
    wait_state(&s.seen, 7, DownloadState::Paused, 5).await;
    s.dl.shutdown();
    gate.notify_waiters();
    // A new downloader (the app restarted) picks the queue up again.
    let session = s.fake.session().await;
    let seen = Arc::new(Seen::default());
    let dl = Downloader::new(
        session,
        s.lib.clone(),
        DownloadPolicy::default(),
        NetworkKind::Wifi,
        seen.clone(),
    )
    .unwrap();
    wait_state(&seen, 5, DownloadState::Done, 10).await;
    wait_state(&seen, 7, DownloadState::Paused, 5).await;
    assert_eq!(seen.snap().items.len(), 3);
    dl.shutdown();
}

/// Regression: a paused entry that was skipped on restart (already on the device) paused
/// the entry before it instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_skipped_paused_entry_never_pauses_another() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let s = setup(audio(20_000), vec![], DownloadPolicy::default()).await;
    s.dl.enqueue(vec![manual(5)], FormatChoice::Original);
    wait_state(&s.seen, 5, DownloadState::Done, 10).await;
    s.dl.shutdown();
    s.fake.script.lock().unwrap().steps = vec![Step::Stall(gate.clone())].into();
    // Track 6 to fetch, then 5 (on the device already) saved as paused.
    std::fs::write(
        std::path::Path::new(&s.root)
            .join(scope_name(&s.url, "alice"))
            .join("queue.json"),
        serde_json::to_vec(&json!([
            {"id": 6, "choice": "original", "origin": "manual", "paused": false},
            {"id": 5, "choice": "original", "origin": "manual", "paused": true},
        ]))
        .unwrap(),
    )
    .unwrap();
    let seen = Arc::new(Seen::default());
    let dl = Downloader::new(
        s.fake.session().await,
        s.lib.clone(),
        DownloadPolicy::default(),
        NetworkKind::Wifi,
        seen.clone(),
    )
    .unwrap();
    for _ in 0..250 {
        if seen.state_of(6).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let state = seen.state_of(6);
    assert!(
        matches!(state, Some(s) if s != DownloadState::Paused),
        "track 6: {state:?}"
    );
    dl.shutdown();
    gate.notify_waiters();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_are_sent_once_and_kept_by_an_older_server() {
    let s = setup(audio(10), vec![], DownloadPolicy::default()).await;
    let session = s.fake.session().await;
    // Recorded through the player in real use; here directly.
    let lib = s.lib.clone();
    lib.record_play(5, &key_of(5), 1_800_000_000, 200);
    lib.record_play(6, &key_of(6), 1_800_000_300, 3); // too short
    assert_eq!(lib.pending_plays(), 1);
    s.fake.script.lock().unwrap().no_plays = true;
    let out = sync_plays(session.clone(), lib.clone()).await.unwrap();
    assert!(out.unsupported);
    assert_eq!((out.sent, out.pending), (0, 1));
    assert_eq!(
        lib.pending_plays(),
        1,
        "kept for a server that has /v1/plays"
    );
    s.fake.script.lock().unwrap().no_plays = false;
    let out = sync_plays(session.clone(), lib.clone()).await.unwrap();
    assert_eq!((out.sent, out.pending), (1, 0));
    let sent = s.fake.script.lock().unwrap().plays.clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["track_id"], 5);
    assert_eq!(sent[0]["seconds"], 200);
    assert_eq!(sent[0]["played_at"], 1_800_000_000u64);
    assert_eq!(sent[0]["track_key"], json!(key_of(5)));
    // Nothing left: nothing sent.
    let out = sync_plays(session, lib).await.unwrap();
    assert_eq!(out.sent, 0);
    assert_eq!(s.fake.script.lock().unwrap().plays.len(), 1);
}

/// M14 regression: a library opened for another account (a stale handle held across an
/// account switch) is refused by everything that sends from it or downloads into it, and
/// nothing reaches the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_accounts_library_is_refused() {
    let fake = Fake::start(audio(10), vec![]).await;
    // Signed in as alice.
    let session = fake.session().await;
    let url = session.server().url();
    let root = temp_root();
    let bobs = OfflineLibrary::open(root.clone(), url.clone(), "bob".into()).unwrap();
    let elsewhere =
        OfflineLibrary::open(root, "https://music.example.com/".into(), "alice".into()).unwrap();
    for lib in [&bobs, &elsewhere] {
        lib.record_play(5, &key_of(5), 1_800_000_000, 200);
        assert!(lib.rate_offline(5, key_of(5), 80, 1_800_000_000));
    }
    fake.script.lock().unwrap().log.clear();
    for lib in [bobs.clone(), elsewhere.clone()] {
        let r = sync_plays(session.clone(), lib.clone()).await;
        assert_eq!(r, Err(ApiError::WrongAccount));
        assert_eq!(lib.pending_plays(), 1, "kept for its own account");
        let r = anywhere_mobile::sync_ratings(session.clone(), lib.clone()).await;
        assert_eq!(r, Err(ApiError::WrongAccount));
        assert_eq!(lib.pending_ratings(), 1);
        let dl = Downloader::new(
            session.clone(),
            lib.clone(),
            DownloadPolicy::default(),
            NetworkKind::Wifi,
            Arc::new(Seen::default()),
        );
        assert!(matches!(dl, Err(ApiError::WrongAccount)));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let log = fake.script.lock().unwrap().log.clone();
    assert!(log.is_empty(), "nothing sent: {log:?}");
    // Alice's own library works as before.
    let mine = OfflineLibrary::open(temp_root(), url, "alice".into()).unwrap();
    mine.record_play(5, &key_of(5), 1_800_000_000, 200);
    let out = sync_plays(session.clone(), mine).await.unwrap();
    assert_eq!(out.sent, 1);
}

// --- The real gateway ----------------------------------------------------------------------

async fn real_setup(
    h: &Harness,
) -> (
    Arc<Session>,
    Arc<OfflineLibrary>,
    Arc<Seen>,
    Arc<Downloader>,
    String,
) {
    let (session, _) = h.alice().await;
    let root = temp_root();
    let lib = OfflineLibrary::open(root.clone(), session.server().url(), "alice".into()).unwrap();
    let seen = Arc::new(Seen::default());
    let dl = Downloader::new(
        session.clone(),
        lib.clone(),
        DownloadPolicy::default(),
        NetworkKind::Wifi,
        seen.clone(),
    )
    .unwrap();
    (session, lib, seen, dl, root)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_and_resumes_through_the_gateway() {
    let h = Harness::start().await;
    let song = flac::encode(44_100, 16, &flac::noise(44_100 * 2, 16, 7));
    h.lms.add_track(1, "flc", song.clone());
    let (session, lib, seen, dl, root) = real_setup(&h).await;
    dl.enqueue(vec![manual(1)], FormatChoice::Original);
    wait_state(&seen, 1, DownloadState::Done, 20).await;
    let t = lib.tracks()[0].clone();
    assert_eq!(
        (t.track_id, t.size, t.ext.as_str(), t.codec.as_str()),
        (1, song.len() as u64, "flac", "flc")
    );
    assert_eq!(t.key.len(), 64);
    let scope = PathBuf::from(&root).join(scope_name(&session.server().url(), "alice"));
    let path = scope.join("tracks").join(format!("{}.flac", t.key));
    assert_eq!(std::fs::read(&path).unwrap(), song);

    // Resume through the gateway: leave a partial file whose first half is marked (zeros),
    // so the result shows the gateway sent only the rest (Range), not the whole file.
    lib.remove(vec![t.key.clone()]).unwrap();
    let half = song.len() / 2;
    let part = scope.join("partial").join(format!("{}.original", t.key));
    std::fs::write(&part, vec![0u8; half]).unwrap();
    dl.enqueue(vec![manual(1)], FormatChoice::Original);
    wait_for("the resumed download", || !lib.tracks().is_empty()).await;
    let got = std::fs::read(&path).unwrap();
    assert_eq!(got.len(), song.len());
    assert!(got[..half].iter().all(|b| *b == 0), "kept the bytes it had");
    assert_eq!(&got[half..], &song[half..], "fetched only the rest");
    dl.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_gateways_refusals_are_honoured() {
    let h = Harness::start().await;
    h.lms.add_track(
        1,
        "flc",
        flac::encode(44_100, 16, &flac::noise(4410, 16, 1)),
    );
    let (_session, lib, seen, dl, _) = real_setup(&h).await;
    // Unknown to LMS: 404, never retried.
    dl.enqueue(vec![manual(424242)], FormatChoice::Original);
    wait_state(&seen, 424242, DownloadState::Failed, 10).await;
    assert_eq!(seen.state_of(424242), Some(DownloadState::Failed));
    assert_eq!(
        seen.snap().items[0].failure,
        Some(DownloadFailure::NotDownloadable)
    );
    // Downloads turned off for alice: 403, shown.
    h.accounts.set_downloads("alice", false, None).unwrap();
    dl.enqueue(vec![manual(1)], FormatChoice::Original);
    wait_state(&seen, 1, DownloadState::Failed, 10).await;
    let snap = seen.snap();
    assert!(snap.forbidden);
    let item = snap.items.iter().find(|i| i.track_id == 1).unwrap();
    assert_eq!(item.failure, Some(DownloadFailure::NotAllowed));
    assert!(lib.tracks().is_empty());
    // Turned on again by the admin: "retry all" clears the refusal and downloads.
    h.accounts.set_downloads("alice", true, None).unwrap();
    dl.resume(None);
    wait_state(&seen, 1, DownloadState::Done, 10).await;
    assert!(!seen.snap().forbidden);
    assert_eq!(lib.tracks().len(), 1);
    dl.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_device_stops_downloading() {
    let h = Harness::start().await;
    h.lms.add_track(
        1,
        "flc",
        flac::encode(44_100, 16, &flac::noise(4410, 16, 2)),
    );
    let (session, lib, seen, dl, _) = real_setup(&h).await;
    let device = session.current().unwrap().device.id;
    h.accounts.revoke_device(device).unwrap();
    session.debug_invalidate_access_token();
    dl.enqueue(vec![manual(1)], FormatChoice::Original);
    wait_state(&seen, 1, DownloadState::Failed, 10).await;
    assert_eq!(
        seen.snap().items[0].failure,
        Some(DownloadFailure::SignedOut)
    );
    assert!(lib.tracks().is_empty());
    dl.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn favourites_are_read_through_the_allow_list() {
    let h = Harness::start().await;
    let (session, _) = h.alice().await;
    let lib = anywhere_mobile::MusicLibrary::new(session);
    // The fake LMS's favourites are a station and a folder, neither a library item.
    let found = lib.favourite_library_items().await.unwrap();
    assert!(found.track_ids.is_empty() && found.album_ids.is_empty());
    let calls = h.lms.rpc_calls();
    let favs: Vec<&Value> = calls.iter().filter(|c| c[1][0] == "favorites").collect();
    assert_eq!(favs.len(), 2, "the top level and its one folder");
    assert!(
        favs.iter()
            .all(|c| c[1].as_array().unwrap().contains(&json!("menu:1")))
    );
    assert!(
        favs[1][1]
            .as_array()
            .unwrap()
            .contains(&json!("item_id:f0.1"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plays_sync_back_through_the_real_gateway() {
    let h = Harness::start().await;
    h.lms.add_track(
        1,
        "flc",
        flac::encode(44_100, 16, &flac::noise(4410, 16, 3)),
    );
    h.lms.add_track(
        2,
        "flc",
        flac::encode(44_100, 16, &flac::noise(4410, 16, 4)),
    );
    let (session, lib, seen, dl, root) = real_setup(&h).await;
    dl.enqueue(vec![manual(1)], FormatChoice::Original);
    wait_state(&seen, 1, DownloadState::Done, 20).await;
    dl.shutdown();
    let key = lib.tracks()[0].key.clone();
    let hour_ago = anywhere_mobile::offline::plays::now_secs() - 3600;
    // Heard long enough (no known duration: four minutes count), the same file.
    let counted = lib.record_play(1, &key, hour_ago, 300).unwrap();
    // Track 2 with track 1's stable key: as if a rescan gave the id to another file.
    lib.record_play(2, &key, hour_ago + 400, 300).unwrap();
    // Too little heard: the server's counting rule refuses it.
    lib.record_play(1, &key, hour_ago + 800, 30).unwrap();
    let out = sync_plays(session.clone(), lib.clone()).await.unwrap();
    assert_eq!((out.sent, out.refused, out.pending), (1, 2, 0), "{out:?}");
    assert!(!out.unsupported);
    let recorded = recorded_plays(&h.lms);
    assert_eq!(recorded.len(), 1);
    assert_eq!((recorded[0].0, recorded[0].1), (1, hour_ago));

    // The same play sent again (its answer was lost): a duplicate, never counted twice.
    let scope = PathBuf::from(&root).join(scope_name(&session.server().url(), "alice"));
    std::fs::write(
        scope.join("plays.json"),
        anywhere_mobile::offline::plays::serialise(&[counted]),
    )
    .unwrap();
    let again = OfflineLibrary::open(root, session.server().url(), "alice".into()).unwrap();
    assert_eq!(again.pending_plays(), 1);
    let out = sync_plays(session, again.clone()).await.unwrap();
    assert_eq!((out.sent, out.pending), (1, 0), "{out:?}");
    assert_eq!(recorded_plays(&h.lms).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_tracks_resolve_to_current_ids_through_the_gateway() {
    // "Continue on <player>" (SPEC §8.4): only an id that still names the download's file.
    let h = Harness::start().await;
    for id in [1u64, 2] {
        h.lms.add_track(
            id,
            "flc",
            flac::encode(44_100, 16, &flac::noise(4410, 16, id)),
        );
    }
    let (session, lib, seen, dl, _) = real_setup(&h).await;
    dl.enqueue(vec![manual(1), manual(2)], FormatChoice::Original);
    wait_state(&seen, 1, DownloadState::Done, 20).await;
    wait_state(&seen, 2, DownloadState::Done, 20).await;
    dl.shutdown();
    let key = |id: i64| {
        lib.tracks()
            .into_iter()
            .find(|t| t.track_id == id)
            .unwrap()
            .key
    };
    let (k1, k2) = (key(1), key(2));
    assert_ne!(k1, k2);
    let r = |track_id: i64, key: &str| OfflineTrackRef {
        track_id,
        key: key.to_string(),
    };
    let refs = vec![
        r(2, &k2),
        // Track 1's id names another file than this (as after a rescan): not resolved.
        r(1, &k2),
        // Unknown to LMS: not resolved.
        r(424242, &k1),
        r(1, &k1),
    ];
    let got = resolve_offline_tracks(session.clone(), refs.clone())
        .await
        .unwrap();
    assert_eq!(got, vec![Some(2), None, None, Some(1)]);

    // Downloads turned off for alice: refused as a whole, nothing half-resolved.
    h.accounts.set_downloads("alice", false, None).unwrap();
    let res = resolve_offline_tracks(session.clone(), refs.clone()).await;
    assert!(
        matches!(res, Err(anywhere_mobile::ApiError::Forbidden { .. })),
        "{res:?}"
    );
    h.accounts.set_downloads("alice", true, None).unwrap();

    // Signed out: refused before anything is asked.
    session.logout().await.unwrap();
    let res = resolve_offline_tracks(session, refs).await;
    assert!(
        matches!(res, Err(anywhere_mobile::ApiError::Unauthorised)),
        "{res:?}"
    );
}
