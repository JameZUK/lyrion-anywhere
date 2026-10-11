//! Track analysis (plan §4, milestone A3): `GET /v1/analysis/track/{id}`.
//!
//! The waveform, loudness (EBU R128), peaks, silence and fades of a library track, computed
//! by the core ([`anywhere_core::analysis`]) and cached on disk.
//!
//! - **Who:** the same rules as downloads: a valid access token and the user's download
//!   permission (403 without it). Library files only (LMS `url` `file://…`); streams and
//!   radio are 404. Nothing in the answer names a path.
//! - **Ids:** checked as for downloads (a positive integer), then looked up in LMS with a
//!   time limit, so only a track LMS knows is fetched.
//! - **Formats:** what the core decodes: FLAC, MP3, AAC/ALAC in MP4, Ogg Vorbis, Ogg Opus,
//!   WAV. Other types are refused (415) before anything is fetched.
//! - **Answer:** JSON ([`TrackAnalysis::to_json`]) by default; the binary form
//!   ([`TrackAnalysis::to_bytes`], `application/vnd.lyrion-anywhere.analysis`) with
//!   `?format=binary` or that type in `Accept`. `ETag` plus `If-None-Match` gives 304.
//! - **While it runs:** the request waits up to [`Config::analysis_wait`]; if the result
//!   isn't ready by then it gets **202** with `Retry-After` and `{"status":"pending"}`, and
//!   the work carries on. Asking again joins the same job (one job per file at a time).
//! - **Limits:** [`Config::analysis_jobs`] analyses run at once (the rest queue, at most
//!   [`Config::analysis_queue`] in all); one user may have [`Config::analysis_per_user`]
//!   jobs of their own queued or running, and [`ANALYSIS_REQUESTS_PER_USER`] requests open:
//!   beyond either, 429 with `Retry-After`. Larger than [`Config::analysis_max_bytes`] or
//!   longer than [`Config::analysis_max_duration`] is 413 (refused before fetching when LMS
//!   knows the size or duration). A job that runs past [`Config::analysis_timeout`]
//!   (fetch and decode together) is abandoned: 504.
//! - **CPU:** decoding and analysis run on a blocking thread, never on the async runtime;
//!   the deadline is checked between packets.
//! - **Failures:** a file the core can't decode (corrupt, truncated to nothing, hostile) is
//!   422; that, 413 and 504 are remembered for [`Config::analysis_retry_after`] so the same
//!   file isn't fetched and decoded again and again. A decoder panic is caught.
//! - **Cache:** the binary form on disk under the data directory, at most
//!   [`Config::analysis_cache_bytes`], least recently used first. Entries are named by a
//!   hash of the stable track key (salted, survives rescans), the file's size and
//!   modification time from LMS, and [`ANALYSIS_VERSION`]: a changed file or a new analysis
//!   version is analysed afresh.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anywhere_core::analysis::{
    ANALYSIS_VERSION, AnalyseError, AnalysisConfig, TrackAnalysis, analyse_source,
    encode::MAX_BINARY_BYTES,
};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::download::{self, Refusal, TempFile, Track};
use crate::limits::{Limiter, Permit};
use crate::transcode::Cache;
use crate::{AppState, Config};

/// The binary form's media type.
pub const BINARY_TYPE: &str = "application/vnd.lyrion-anywhere.analysis";
/// Cache file extension.
const EXTS: &[&str] = &["laan"];
/// Requests one user may have open at once (waiting for a result or not).
pub const ANALYSIS_REQUESTS_PER_USER: usize = 8;
/// Suggested wait before asking again for a pending analysis, s.
const PENDING_RETRY: &str = "3";
/// Remembered failures, at most.
const FAILED_CAP: usize = 4096;

pub const NOT_AVAILABLE: &str = "analysis is not available on this server";

/// What a client asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Json,
    Binary,
}

/// The query string: nothing, or exactly `format=json` or `format=binary`.
pub fn parse_query(q: Option<&str>) -> Option<Option<Encoding>> {
    match q {
        None | Some("") => Some(None),
        Some("format=json") => Some(Some(Encoding::Json)),
        Some("format=binary") => Some(Some(Encoding::Binary)),
        _ => None,
    }
}

/// The encoding: the query's, else binary if `Accept` names [`BINARY_TYPE`], else JSON.
pub fn choose_encoding(query: Option<Encoding>, headers: &HeaderMap) -> Encoding {
    if let Some(e) = query {
        return e;
    }
    let wants_binary = headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|item| {
            let mut parts = item.split(';').map(str::trim);
            let ty = parts.next().unwrap_or("");
            // `q=0` means "not this one".
            let refused = parts.any(|p| {
                p.strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            });
            ty.eq_ignore_ascii_case(BINARY_TYPE) && !refused
        });
    if wants_binary {
        Encoding::Binary
    } else {
        Encoding::Json
    }
}

/// LMS file types the core can decode, with the extension hint for the demuxer.
pub fn decodable(lms_type: &str) -> Option<&'static str> {
    Some(match lms_type {
        "flc" => "flac",
        "mp3" => "mp3",
        "mp4" | "alc" | "m4a" => "m4a",
        "aac" => "aac",
        "ogg" | "ogf" => "ogg",
        "ops" => "opus",
        "wav" => "wav",
        _ => return None,
    })
}

/// The cache entry name for one version of a track's file.
pub fn cache_name(key: &str, size: Option<u64>, mtime: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        key,
        "analysis",
        &ANALYSIS_VERSION.to_string(),
        &size.map(|s| s.to_string()).unwrap_or_default(),
        mtime,
    ] {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    format!("{}.laan", hex::encode(h.finalize()))
}

/// The entity tag for an entry in an encoding.
fn etag(name: &str, enc: Encoding) -> String {
    let suffix = match enc {
        Encoding::Json => "j",
        Encoding::Binary => "b",
    };
    format!("\"{}-{suffix}\"", &name[..32])
}

/// Whether `If-None-Match` lists this tag (or `*`).
fn not_modified(headers: &HeaderMap, tag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == tag || t == "*")
}

/// Why there is no analysis for a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    NotFound,
    NotAnalysable,
    Unsupported,
    TooLarge,
    TooLong,
    Corrupt,
    TimedOut,
    LmsUnavailable,
    LmsTimedOut,
    Internal,
}

impl Failure {
    /// Worth remembering: asking again would only repeat the same work.
    fn lasting(self) -> bool {
        matches!(
            self,
            Failure::TooLarge | Failure::TooLong | Failure::Corrupt | Failure::TimedOut
        )
    }

    fn response(self) -> Response {
        let (status, msg) = match self {
            Failure::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Failure::NotAnalysable => (StatusCode::NOT_FOUND, "not analysable"),
            Failure::Unsupported => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "this format can't be analysed",
            ),
            Failure::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "track too large to analyse"),
            Failure::TooLong => (StatusCode::PAYLOAD_TOO_LARGE, "track too long to analyse"),
            Failure::Corrupt => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "the audio could not be analysed",
            ),
            Failure::TimedOut => (StatusCode::GATEWAY_TIMEOUT, "analysis took too long"),
            Failure::LmsUnavailable => (StatusCode::BAD_GATEWAY, "music server unavailable"),
            Failure::LmsTimedOut => (StatusCode::GATEWAY_TIMEOUT, "music server timed out"),
            Failure::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
        };
        error(status, msg)
    }
}

impl From<Refusal> for Failure {
    fn from(r: Refusal) -> Failure {
        match r {
            Refusal::NotFound => Failure::NotFound,
            Refusal::NotDownloadable => Failure::NotAnalysable,
            Refusal::Unavailable => Failure::LmsUnavailable,
            Refusal::TimedOut => Failure::LmsTimedOut,
            Refusal::TooLarge => Failure::TooLarge,
            _ => Failure::Internal,
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    let mut r = (status, Json(json!({ "error": message }))).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

fn too_many(message: &str) -> Response {
    let mut r = error(StatusCode::TOO_MANY_REQUESTS, message);
    r.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    r
}

type Outcome = Result<Arc<Vec<u8>>, Failure>;

/// The analysis service: cache, limits and jobs in flight.
pub struct Analysis {
    cache: Cache,
    /// Analyses running at once.
    jobs: tokio::sync::Semaphore,
    /// Jobs queued or running: per user who started them, and in all.
    queue: Arc<Limiter>,
    /// Requests open, per user.
    requests: Arc<Limiter>,
    inflight: Mutex<HashMap<String, watch::Receiver<Option<Outcome>>>>,
    failed: Mutex<HashMap<String, (Instant, Failure)>>,
    max_bytes: u64,
    max_duration: Duration,
    timeout: Duration,
    wait: Duration,
    retry_after: Duration,
}

impl Analysis {
    /// The service, if analysis is on (it needs a cache directory).
    pub fn new(cfg: &Config) -> Option<Analysis> {
        let dir = cfg.analysis_cache_dir.as_ref()?;
        let cache = match Cache::open_with(dir, cfg.analysis_cache_bytes, EXTS) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, dir = %dir.display(), "analysis cache unavailable; analysis off");
                return None;
            }
        };
        Some(Analysis {
            cache,
            jobs: tokio::sync::Semaphore::new(cfg.analysis_jobs.max(1)),
            queue: Limiter::new(cfg.analysis_per_user, cfg.analysis_queue),
            requests: Limiter::new(ANALYSIS_REQUESTS_PER_USER, usize::MAX),
            inflight: Mutex::new(HashMap::new()),
            failed: Mutex::new(HashMap::new()),
            max_bytes: cfg.analysis_max_bytes,
            max_duration: cfg.analysis_max_duration,
            timeout: cfg.analysis_timeout,
            wait: cfg.analysis_wait,
            retry_after: cfg.analysis_retry_after,
        })
    }

    /// The cached analysis for an entry, if there is a readable one.
    fn cached(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.cache.lookup(name)?;
        let ok = std::fs::read(&path)
            .ok()
            .filter(|b| b.len() <= MAX_BINARY_BYTES && TrackAnalysis::from_bytes(b).is_ok());
        if ok.is_none() {
            tracing::warn!(entry = name, "unreadable analysis cache entry removed");
            self.cache.remove(name);
        }
        ok
    }

    fn remembered_failure(&self, name: &str) -> Option<Failure> {
        let mut failed = self.failed.lock().unwrap();
        match failed.get(name) {
            Some((at, f)) if at.elapsed() < self.retry_after => Some(*f),
            Some(_) => {
                failed.remove(name);
                None
            }
            None => None,
        }
    }

    fn remember_failure(&self, name: &str, f: Failure) {
        let mut failed = self.failed.lock().unwrap();
        if failed.len() >= FAILED_CAP {
            let ttl = self.retry_after;
            failed.retain(|_, (at, _)| at.elapsed() < ttl);
            if failed.len() >= FAILED_CAP {
                failed.clear();
            }
        }
        failed.insert(name.to_string(), (Instant::now(), f));
    }

    /// Store a result in the cache (best effort).
    fn store(&self, name: &str, bytes: &[u8]) {
        let tmp = self.cache.temp_path("laan");
        if std::fs::write(&tmp, bytes).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        if std::fs::rename(&tmp, self.cache.dir().join(name)).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        self.cache.insert(name, bytes.len() as u64);
    }

    /// Bytes held in the cache (tests, diagnostics).
    pub fn cache_bytes(&self) -> u64 {
        self.cache.total()
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /v1/analysis/track/{id}[?format=json|binary]`
pub async fn track(
    State(gw): State<AppState>,
    UrlPath(id): UrlPath<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(principal) = crate::auth::principal(&gw, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    let Some(id) = download::parse_track_id(&id) else {
        return Failure::NotFound.response();
    };
    let Some(query) = parse_query(query.as_deref()) else {
        return error(StatusCode::BAD_REQUEST, "unknown parameter");
    };
    let policy = match gw.accounts().download_policy(principal.user_id) {
        Ok(p) => p,
        Err(_) => return Failure::Internal.response(),
    };
    if !policy.enabled {
        return error(StatusCode::FORBIDDEN, download::NOT_ALLOWED);
    }
    let Some(svc) = gw.analysis.as_ref() else {
        return error(StatusCode::NOT_FOUND, NOT_AVAILABLE);
    };
    let Some(_request) = svc.requests.try_acquire(principal.user_id) else {
        return too_many("too many analysis requests at once");
    };
    let enc = choose_encoding(query, &headers);
    let track = match gw.library_track(id).await {
        Ok(t) => t,
        Err(r) => return Failure::from(r).response(),
    };
    let Some(ext) = decodable(&track.kind) else {
        return Failure::Unsupported.response();
    };
    if track.size.is_some_and(|s| s > svc.max_bytes) {
        return Failure::TooLarge.response();
    }
    if track
        .duration
        .is_some_and(|d| d > svc.max_duration.as_secs_f64())
    {
        return Failure::TooLong.response();
    }
    let Some(salt) = gw.download_salt() else {
        return Failure::Internal.response();
    };
    let name = cache_name(
        &download::stable_key(&salt, &track.url),
        track.size,
        &track.mtime,
    );
    let tag = etag(&name, enc);

    if let Some(bytes) = svc.cached(&name) {
        return answer(&bytes, enc, &tag, &headers);
    }
    if let Some(f) = svc.remembered_failure(&name) {
        return f.response();
    }

    // Join the job for this file, or start one.
    let mut rx = {
        let mut inflight = svc.inflight.lock().unwrap();
        match inflight.get(&name) {
            Some(rx) => rx.clone(),
            None => {
                let Some(permit) = svc.queue.try_acquire(principal.user_id) else {
                    return too_many("too many analyses at once");
                };
                let (tx, rx) = watch::channel(None);
                inflight.insert(name.clone(), rx.clone());
                tokio::spawn(run_job(gw.clone(), track, ext, name.clone(), tx, permit));
                rx
            }
        }
    };
    let done = tokio::time::timeout(svc.wait, rx.wait_for(Option::is_some)).await;
    match done {
        Ok(Ok(v)) => match v.clone() {
            Some(Ok(bytes)) => answer(&bytes, enc, &tag, &headers),
            Some(Err(f)) => f.response(),
            None => Failure::Internal.response(),
        },
        // The job vanished without an answer (it panicked).
        Ok(Err(_)) => Failure::Internal.response(),
        Err(_) => {
            let mut r =
                (StatusCode::ACCEPTED, Json(json!({ "status": "pending" }))).into_response();
            let h = r.headers_mut();
            h.insert(header::RETRY_AFTER, HeaderValue::from_static(PENDING_RETRY));
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            );
            r
        }
    }
}

fn answer(bytes: &[u8], enc: Encoding, tag: &str, req: &HeaderMap) -> Response {
    let mut r = if not_modified(req, tag) {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NOT_MODIFIED;
        r
    } else {
        match enc {
            Encoding::Binary => {
                let mut r = Response::new(Body::from(bytes.to_vec()));
                r.headers_mut()
                    .insert(header::CONTENT_TYPE, HeaderValue::from_static(BINARY_TYPE));
                r
            }
            Encoding::Json => match TrackAnalysis::from_bytes(bytes) {
                Ok(a) => {
                    let mut r = Response::new(Body::from(a.to_json()));
                    r.headers_mut().insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    );
                    r
                }
                Err(_) => return Failure::Internal.response(),
            },
        }
    };
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(tag) {
        h.insert(header::ETAG, v);
    }
    h.insert(header::VARY, HeaderValue::from_static("Accept"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    r
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// One analysis, run in the background; every request for the file waits on `tx`.
async fn run_job(
    gw: AppState,
    track: Track,
    ext: &'static str,
    name: String,
    tx: watch::Sender<Option<Outcome>>,
    _permit: Permit,
) {
    let Some(svc) = gw.analysis.as_ref() else {
        return;
    };
    // However this ends, later requests start afresh (a request that joins in between
    // still sees the outcome, which is sent first).
    struct Unlist<'a>(&'a Analysis, &'a str);
    impl Drop for Unlist<'_> {
        fn drop(&mut self) {
            self.0.inflight.lock().unwrap().remove(self.1);
        }
    }
    let _unlist = Unlist(svc, &name);
    let outcome = match svc.jobs.acquire().await {
        Ok(_running) => match svc.cached(&name) {
            // Finished by an earlier job meanwhile.
            Some(bytes) => Ok(Arc::new(bytes)),
            None => analyse(&gw, svc, &track, ext).await.map(Arc::new),
        },
        Err(_) => Err(Failure::Internal),
    };
    match &outcome {
        Ok(bytes) => svc.store(&name, bytes),
        Err(f) => {
            tracing::info!(track = track.id, failure = ?f, "track analysis failed");
            if f.lasting() {
                svc.remember_failure(&name, *f);
            }
        }
    }
    tx.send_replace(Some(outcome));
}

/// Fetch the original into a temporary file and analyse it off the async runtime, all
/// within the time limit.
async fn analyse(
    gw: &AppState,
    svc: &Analysis,
    track: &Track,
    ext: &'static str,
) -> Result<Vec<u8>, Failure> {
    let deadline = Instant::now() + svc.timeout;
    let input = TempFile(svc.cache.temp_path("src"));
    match tokio::time::timeout(
        svc.timeout,
        download::save_original(gw, track.id, &input.0, svc.max_bytes),
    )
    .await
    {
        Err(_) => return Err(Failure::TimedOut),
        Ok(Err(r)) => return Err(r.into()),
        Ok(Ok(())) => {}
    }
    let file = std::fs::File::open(&input.0).map_err(|_| Failure::Internal)?;
    let max_secs = svc.max_duration.as_secs_f64();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let job = tokio::task::spawn_blocking(move || {
        let should_stop = || stop.load(Ordering::Relaxed) || Instant::now() >= deadline;
        analyse_source(
            Box::new(file),
            Some(ext),
            AnalysisConfig::default(),
            max_secs,
            &should_stop,
        )
    });
    // If this task is dropped (shutdown), the blocking work stops at its next packet.
    let _cancel_on_drop = CancelOnDrop(cancel);
    let result = job.await.map_err(|_| Failure::Corrupt)?;
    drop(input);
    match result {
        Ok(a) => Ok(a.to_bytes()),
        Err(AnalyseError::TooLong) => Err(Failure::TooLong),
        Err(AnalyseError::Stopped) => Err(Failure::TimedOut),
        Err(e) => {
            tracing::debug!(track = track.id, error = %e, "track not analysable");
            Err(Failure::Corrupt)
        }
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Whether a path is inside the analysis cache (for tests).
pub fn is_cache_file(dir: &Path, name: &str) -> bool {
    crate::transcode::valid_name_with(name, EXTS) && dir.join(name).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_exact() {
        assert_eq!(parse_query(None), Some(None));
        assert_eq!(parse_query(Some("")), Some(None));
        assert_eq!(parse_query(Some("format=json")), Some(Some(Encoding::Json)));
        assert_eq!(
            parse_query(Some("format=binary")),
            Some(Some(Encoding::Binary))
        );
        for bad in [
            "format=",
            "format=cbor",
            "format=json&x=1",
            "x=1&format=json",
            "format=JSON",
            "format=json;",
            "format=binary%00",
            "format",
            "&",
        ] {
            assert_eq!(parse_query(Some(bad)), None, "{bad}");
        }
    }

    fn accept(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ACCEPT, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn encoding_from_accept() {
        let none = HeaderMap::new();
        assert_eq!(choose_encoding(None, &none), Encoding::Json);
        assert_eq!(choose_encoding(None, &accept("*/*")), Encoding::Json);
        assert_eq!(
            choose_encoding(None, &accept(BINARY_TYPE)),
            Encoding::Binary
        );
        assert_eq!(
            choose_encoding(
                None,
                &accept(&format!("application/json, {BINARY_TYPE};q=0.9"))
            ),
            Encoding::Binary
        );
        assert_eq!(
            choose_encoding(None, &accept(&format!("{BINARY_TYPE};q=0"))),
            Encoding::Json
        );
        assert_eq!(
            choose_encoding(Some(Encoding::Json), &accept(BINARY_TYPE)),
            Encoding::Json
        );
    }

    #[test]
    fn only_decodable_types() {
        for t in [
            "flc", "mp3", "mp4", "alc", "m4a", "aac", "ogg", "ops", "wav",
        ] {
            assert!(decodable(t).is_some(), "{t}");
        }
        for t in [
            "", "wma", "ape", "wvp", "dsf", "dff", "mpc", "aif", "xyz", "../flc",
        ] {
            assert!(decodable(t).is_none(), "{t}");
        }
    }

    #[test]
    fn cache_names_track_the_file_version() {
        let a = cache_name("k", Some(10), "Mon");
        assert!(crate::transcode::valid_name_with(&a, EXTS));
        assert_ne!(a, cache_name("k", Some(11), "Mon"));
        assert_ne!(a, cache_name("k", Some(10), "Tue"));
        assert_ne!(a, cache_name("j", Some(10), "Mon"));
        // No ambiguity from joining the parts.
        assert_ne!(cache_name("k1", None, "x"), cache_name("k", Some(1), "x"));
    }

    #[test]
    fn etags_and_conditional_requests() {
        let name = cache_name("k", Some(1), "t");
        let j = etag(&name, Encoding::Json);
        let b = etag(&name, Encoding::Binary);
        assert_ne!(j, b);
        let mut h = HeaderMap::new();
        assert!(!not_modified(&h, &j));
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("\"x\", {j}")).unwrap(),
        );
        assert!(not_modified(&h, &j));
        assert!(!not_modified(&h, &b));
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("W/{b}")).unwrap(),
        );
        assert!(not_modified(&h, &b));
    }
}
