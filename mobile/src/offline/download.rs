//! The download manager (SPEC §8.3; the web app's `offline/manager.svelte.ts` is the
//! behavioural reference): a queue that fetches two tracks at a time through
//! `GET /v1/download/track/{id}[/info]`, into app-private storage.
//!
//! - **Resumable:** bytes are appended to a partial file as they arrive; an interrupted
//!   download resumes with `Range: bytes=<have>-`. A 416, or a `Content-Range` that doesn't
//!   start where asked, starts the file again.
//! - **Integrity:** the stable key from `/info` names the file; the size from `/info`
//!   (originals) and `Content-Length`/`Content-Range` must agree, the body may not run past
//!   them, and a short body is resumed rather than kept.
//! - **One fetch per file:** two queued ids that `/info` says are the same file (a rescan
//!   gave the track a new id) never fetch it at once: the second waits for the first (a
//!   claim on the stable key, held by the library), then finds the file on the device.
//!   Cancelling one leaves the partial file the other is still writing.
//! - **Refusals:** 401 renews the access token once and retries; 403 (no download
//!   permission) fails for good and is reported so the app can say so; 404 is "not
//!   downloadable"; 429 waits as `Retry-After` says; other failures back off and retry.
//! - **Policy:** the format by network (Wi-Fi or mobile data) as on the web, "Wi-Fi only",
//!   and a storage cap (smart downloads make room first; the user's own are never removed).
//! - **Durable queue:** what is waiting survives a restart (`queue.json`).
//!
//! The manager is an actor on its own thread and runtime; the app talks to it through
//! [`Downloader`] and hears back through [`DownloadObserver`]. Nothing it stores holds a
//! token: files are named by stable key, the queue by LMS track id.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http_body_util::BodyExt;
use hyper::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE, RETRY_AFTER};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::art::Art;
use crate::error::ApiError;
use crate::session::{Core, Session};
use crate::transport::{Http, Response};

use super::index::{
    self, DownloadFormat, DownloadOrigin, OfflineTrack, clean, clean_codec, valid_ext, valid_key,
};
use super::plays::now_secs;
use super::store::{OfflineLibrary, read_limited, write_atomic};

/// Downloads running at once (the gateway allows three per user).
pub const CONCURRENCY: usize = 2;
/// Give up after this many failed attempts in a row (the user can retry).
pub const MAX_ATTEMPTS: u32 = 5;
/// … and after this many "too many downloads, wait" answers.
pub const MAX_WAITS: u32 = 20;
/// Most downloads queued.
pub const MAX_QUEUE: usize = 10_000;
/// Longest wait a server may ask for.
const MAX_WAIT: Duration = Duration::from_secs(600);
/// Waiting for an original's headers (LMS must answer within 20 s).
const ORIGINAL_HEADERS: Duration = Duration::from_secs(60);
/// Waiting for a transcode's headers (the gateway makes it first, for up to 10 minutes).
const TRANSCODE_HEADERS: Duration = Duration::from_secs(11 * 60);
/// Between two pieces of a body.
const BODY_READ: Duration = Duration::from_secs(60);
/// Progress reports at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(300);
/// The cover cached for offline browsing.
const ART_PX: u32 = 600;
/// A download of unknown size checks the storage cap after this many bytes.
const CAP_CHECK_EVERY: u64 = 1024 * 1024;

// --- Public types ---------------------------------------------------------------------

/// The format asked for: a fixed one, or by network ("auto").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum FormatChoice {
    Auto,
    Original,
    Mp3,
    Opus,
}

impl FormatChoice {
    fn as_str(self) -> &'static str {
        match self {
            FormatChoice::Auto => "auto",
            FormatChoice::Original => "original",
            FormatChoice::Mp3 => "mp3",
            FormatChoice::Opus => "opus",
        }
    }

    fn parse(s: &str) -> Option<FormatChoice> {
        match s {
            "auto" => Some(FormatChoice::Auto),
            "original" => Some(FormatChoice::Original),
            "mp3" => Some(FormatChoice::Mp3),
            "opus" => Some(FormatChoice::Opus),
            _ => None,
        }
    }
}

/// The network the phone is on, as the app sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NetworkKind {
    /// Wi-Fi or Ethernet (unmetered).
    Wifi,
    /// Mobile data (metered).
    Mobile,
    /// No connection.
    Offline,
}

/// The user's download settings (per device).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DownloadPolicy {
    pub wifi_format: DownloadFormat,
    pub mobile_format: DownloadFormat,
    /// Download only on Wi-Fi: on mobile data, downloads wait.
    pub wifi_only: bool,
    /// Most bytes all downloads may use (0: no limit).
    pub cap_bytes: u64,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        DownloadPolicy {
            wifi_format: DownloadFormat::Original,
            mobile_format: DownloadFormat::Opus,
            wifi_only: false,
            cap_bytes: 0,
        }
    }
}

/// One track to download.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DownloadRequest {
    pub track_id: i64,
    pub album_id: Option<i64>,
    pub origin: DownloadOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum DownloadState {
    Queued,
    Active,
    /// Retrying after a failure, or waiting for the server or the network.
    Waiting,
    Paused,
    Done,
    Failed,
}

/// Why a download failed for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum DownloadFailure {
    /// The account may not download (or not in this format).
    NotAllowed,
    /// Not a library file (a stream or radio), or unknown to LMS.
    NotDownloadable,
    /// The sign-in has ended.
    SignedOut,
    /// The device is full.
    StorageFull,
    /// The storage cap is reached (and nothing may be removed to make room).
    LimitReached,
    /// The server sent something that doesn't fit the contract.
    BadData,
    /// The server or the network kept failing.
    Failed,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DownloadItem {
    pub track_id: i64,
    pub album_id: Option<i64>,
    /// Known once the server has described the track.
    pub title: String,
    pub state: DownloadState,
    pub received: u64,
    /// 0 while unknown.
    pub total: u64,
    pub format: Option<DownloadFormat>,
    pub origin: DownloadOrigin,
    pub failure: Option<DownloadFailure>,
    /// Why it is waiting or failed, fit to show.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DownloadsSnapshot {
    pub items: Vec<DownloadItem>,
    /// The server refused downloads for this account (the admin turned them off).
    pub forbidden: bool,
    /// Downloads are waiting for Wi-Fi (or for any connection).
    pub waiting_for_network: bool,
    pub network: NetworkKind,
    pub used_bytes: u64,
    pub cap_bytes: u64,
    /// Downloads queued, running or waiting (not paused, finished or failed).
    pub busy: u32,
}

/// Hears about downloads, on the downloader's thread.
#[uniffi::export(with_foreign)]
pub trait DownloadObserver: Send + Sync {
    fn on_downloads(&self, snapshot: DownloadsSnapshot);
    /// Tracks were added to (or removed from) the offline library.
    fn on_library_changed(&self);
}

// --- The server's track description ------------------------------------------------------

/// `GET /v1/download/track/{id}/info`, checked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TrackInfo {
    pub id: i64,
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<i64>,
    pub tracknum: Option<u32>,
    pub duration: f64,
    /// LMS's file type (e.g. "flc").
    pub codec: String,
    /// The original's rate (Hz) and depth (bits), when the gateway gives them
    /// (`original.rate`, `original.bits`; optional).
    pub rate: Option<u32>,
    pub bits: Option<u32>,
    /// The original's size, when LMS knows it.
    pub size: Option<u64>,
    pub ext: String,
    pub formats: Vec<DownloadFormat>,
}

/// Check `/info` against the contract. The id must be the one asked for; the key 64 hex
/// digits; the extension one of the allowed few (anything the server says is never used
/// to build a path otherwise).
pub(crate) fn parse_info(v: &Value, expect_id: i64) -> Result<TrackInfo, &'static str> {
    let o = v.as_object().ok_or("not an object")?;
    if o.get("id").and_then(Value::as_i64) != Some(expect_id) {
        return Err("id");
    }
    let key = o
        .get("key")
        .and_then(Value::as_str)
        .filter(|k| valid_key(k))
        .ok_or("key")?;
    let orig = o
        .get("original")
        .and_then(Value::as_object)
        .ok_or("original")?;
    let size = match orig.get("size") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|n| *n < 1 << 40).ok_or("size")?),
    }
    .filter(|n| *n > 0);
    let mut formats = Vec::new();
    for f in o
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(f) = f.as_str().and_then(DownloadFormat::parse)
            && !formats.contains(&f)
        {
            formats.push(f);
        }
    }
    if formats.is_empty() {
        return Err("formats");
    }
    let ext = orig
        .get("ext")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|e| valid_ext(e))
        .unwrap_or_default();
    let text = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .map(clean)
            .unwrap_or_default()
    };
    let positive = |v: Option<&Value>| {
        v.and_then(Value::as_i64)
            .filter(|n| (1..=999_999_999_999).contains(n))
    };
    Ok(TrackInfo {
        id: expect_id,
        key: key.to_string(),
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        album_id: positive(o.get("album_id")),
        tracknum: o
            .get("tracknum")
            .and_then(Value::as_u64)
            .filter(|n| *n < 100_000)
            .map(|n| n as u32),
        duration: o
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite() && *d > 0.0 && *d < 1e6)
            .unwrap_or(0.0),
        codec: clean_codec(orig.get("format").and_then(Value::as_str).unwrap_or("")),
        rate: orig
            .get("rate")
            .and_then(Value::as_u64)
            .filter(|n| (1000..=50_000_000).contains(n))
            .map(|n| n as u32),
        bits: orig
            .get("bits")
            .and_then(Value::as_u64)
            .filter(|n| (1..=64).contains(n))
            .map(|n| n as u32),
        size,
        ext,
        formats,
    })
}

/// The format to fetch: the one asked for if offered, else Original, else the first.
pub(crate) fn pick_format(want: DownloadFormat, offered: &[DownloadFormat]) -> DownloadFormat {
    if offered.contains(&want) {
        want
    } else if offered.contains(&DownloadFormat::Original) {
        DownloadFormat::Original
    } else {
        offered.first().copied().unwrap_or(DownloadFormat::Original)
    }
}

/// `bytes a-b/total` → (start, end, total if known).
pub(crate) fn content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (span, total) = rest.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    let num = |s: &str| -> Option<u64> {
        (!s.is_empty() && s.len() <= 15 && s.bytes().all(|c| c.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (a, b) = (num(a)?, num(b)?);
    if b < a {
        return None;
    }
    let total = if total == "*" {
        None
    } else {
        Some(num(total)?)
    };
    if total.is_some_and(|t| b >= t) {
        return None;
    }
    Some((a, b, total))
}

/// `Retry-After` in seconds, capped; `None` if missing or not a number of seconds.
fn retry_after(resp: &Response) -> Option<Duration> {
    let s = resp.headers().get(RETRY_AFTER)?.to_str().ok()?.trim();
    let n: u64 = (s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()?;
    Some(Duration::from_secs(n.max(1)).min(MAX_WAIT))
}

/// Exponential backoff: 2 s, 4 s, 8 s … up to a minute.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_millis((1000u64 << attempt.min(6)).min(60_000))
}

// --- Failures --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Failure {
    /// Give up: retrying won't help.
    Permanent(DownloadFailure, String),
    /// Try again (after `wait`, or the backoff). `counts`: towards the attempt limit
    /// (a "wait your turn" from the server doesn't).
    Retry {
        message: String,
        wait: Option<Duration>,
        counts: bool,
    },
    /// The partial file is unusable: discard it and start again.
    Restart(String),
}

fn retry(message: &str) -> Failure {
    Failure::Retry {
        message: message.into(),
        wait: None,
        counts: true,
    }
}

const MSG_NOT_ALLOWED: &str = "Your account isn't allowed to download.";
const MSG_FORMAT: &str = "Your account isn't allowed to download this format.";
const MSG_NOT_DOWNLOADABLE: &str =
    "Not available for download (only tracks in your library can be downloaded).";
const MSG_SIGNED_OUT: &str = "Signed out. Sign in again to download.";
const MSG_FULL: &str = "Not enough storage space on this device.";
const MSG_LIMIT: &str = "The storage limit for downloads is reached.";
const MSG_BAD: &str = "The server sent unexpected track details.";
const MSG_BUSY: &str = "Waiting: the server is busy with other downloads.";
const MSG_LOST: &str = "Connection lost. Retrying…";
const MSG_GAVE_UP: &str = "Download failed. Try again later.";

/// What a non-success answer means.
async fn refusal(resp: Response) -> Failure {
    let status = resp.status().as_u16();
    let wait = retry_after(&resp);
    let body = resp.read_limited(64 * 1024).await.unwrap_or_default();
    let error = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    classify(status, &error, wait)
}

pub(crate) fn classify(status: u16, error: &str, wait: Option<Duration>) -> Failure {
    match status {
        401 => Failure::Permanent(DownloadFailure::SignedOut, MSG_SIGNED_OUT.into()),
        403 if error.contains("format") => {
            Failure::Permanent(DownloadFailure::NotAllowed, MSG_FORMAT.into())
        }
        403 => Failure::Permanent(DownloadFailure::NotAllowed, MSG_NOT_ALLOWED.into()),
        404 => Failure::Permanent(
            DownloadFailure::NotDownloadable,
            MSG_NOT_DOWNLOADABLE.into(),
        ),
        416 => Failure::Restart("Starting again.".into()),
        429 => Failure::Retry {
            message: MSG_BUSY.into(),
            wait: Some(wait.unwrap_or(Duration::from_secs(5))),
            counts: false,
        },
        408 | 500..=599 => Failure::Retry {
            message: format!("Server error (HTTP {status}). Retrying…"),
            wait: None,
            counts: true,
        },
        _ => Failure::Permanent(
            DownloadFailure::Failed,
            format!("The server refused the download (HTTP {status})."),
        ),
    }
}

fn token_failure(e: ApiError) -> Failure {
    match e {
        ApiError::Unauthorised => {
            Failure::Permanent(DownloadFailure::SignedOut, MSG_SIGNED_OUT.into())
        }
        _ => retry(MSG_LOST),
    }
}

fn io_failure(e: &std::io::Error) -> Failure {
    if e.raw_os_error() == Some(28) || e.kind() == std::io::ErrorKind::StorageFull {
        Failure::Permanent(DownloadFailure::StorageFull, MSG_FULL.into())
    } else {
        Failure::Permanent(
            DownloadFailure::Failed,
            "Could not save the download.".into(),
        )
    }
}

// --- The worker ----------------------------------------------------------------------

/// Everything one download needs.
#[derive(Clone)]
struct Job {
    id: i64,
    epoch: u64,
    album_id: Option<i64>,
    choice: FormatChoice,
    origin: DownloadOrigin,
    core: Arc<Core>,
    session: Arc<Session>,
    http: Http,
    library: Arc<OfflineLibrary>,
    policy: DownloadPolicy,
    network: NetworkKind,
    tx: mpsc::UnboundedSender<Msg>,
}

impl Job {
    fn send(&self, ev: WorkerEv) {
        let _ = self.tx.send(Msg::Worker {
            id: self.id,
            epoch: self.epoch,
            ev,
        });
    }

    fn want(&self) -> DownloadFormat {
        match self.choice {
            FormatChoice::Original => DownloadFormat::Original,
            FormatChoice::Mp3 => DownloadFormat::Mp3,
            FormatChoice::Opus => DownloadFormat::Opus,
            FormatChoice::Auto => match self.network {
                NetworkKind::Mobile => self.policy.mobile_format,
                _ => self.policy.wifi_format,
            },
        }
    }

    /// A GET with the access token, renewing it once on 401.
    async fn get(
        &self,
        url: url::Url,
        range: Option<u64>,
        headers: Duration,
    ) -> Result<Response, Failure> {
        let build = |token: &str| {
            let mut r = self.http.get(url.clone()).bearer_auth(token);
            if let Some(from) = range {
                r = r.header_value(RANGE, &format!("bytes={from}-"));
            }
            r
        };
        let token = self.core.access_token().await.map_err(token_failure)?;
        let resp = build(&token)
            .send_within(headers)
            .await
            .map_err(|_| retry(MSG_LOST))?;
        if resp.status().as_u16() != 401 {
            return Ok(resp);
        }
        let fresh = self.core.renew(Some(&token)).await.map_err(token_failure)?;
        build(&fresh)
            .send_within(headers)
            .await
            .map_err(|_| retry(MSG_LOST))
    }

    async fn info(&self) -> Result<TrackInfo, Failure> {
        let url = self
            .core
            .server()
            .endpoint(&format!("v1/download/track/{}/info", self.id));
        let resp = self.get(url, None, ORIGINAL_HEADERS).await?;
        if !resp.status().is_success() {
            return Err(refusal(resp).await);
        }
        let body = resp
            .read_limited(64 * 1024)
            .await
            .map_err(|_| retry(MSG_LOST))?;
        let v: Value = serde_json::from_slice(&body)
            .map_err(|_| Failure::Permanent(DownloadFailure::BadData, MSG_BAD.into()))?;
        parse_info(&v, self.id)
            .map_err(|_| Failure::Permanent(DownloadFailure::BadData, MSG_BAD.into()))
    }

    /// Make room for `need` more bytes under the cap: smart downloads go first (oldest
    /// first), and only for the user's own downloads or when nothing else is needed.
    fn ensure_room(&self, need: u64) -> Result<(), Failure> {
        let cap = self.policy.cap_bytes;
        if cap == 0 {
            return Ok(());
        }
        let mut used = self.library.used_bytes();
        if used + need <= cap {
            return Ok(());
        }
        if self.origin == DownloadOrigin::Manual {
            let mut gone = Vec::new();
            for t in self.library.evictable() {
                if used + need <= cap {
                    break;
                }
                used = used.saturating_sub(t.size);
                gone.push(t.key);
            }
            if !gone.is_empty() && self.library.remove(gone).is_ok() {
                self.send(WorkerEv::LibraryChanged);
            }
            if used + need <= cap {
                return Ok(());
            }
        }
        Err(Failure::Permanent(
            DownloadFailure::LimitReached,
            MSG_LIMIT.into(),
        ))
    }

    async fn attempt(&self) -> Result<(), Failure> {
        let info = self.info().await?;
        self.send(WorkerEv::Described {
            title: info.title.clone(),
            album_id: info.album_id,
            key: info.key.clone(),
        });
        let format = pick_format(self.want(), &info.formats);

        // One fetch per file: another id for the same file (after a rescan) being fetched
        // now is waited for, and then found here below.
        let _claim = self.library.claim(&info.key).await;

        // Already here (perhaps under an older id, before a rescan).
        if let Some((have, _)) = self.library.file_for(&info.key) {
            if self.origin == DownloadOrigin::Manual && have.origin != DownloadOrigin::Manual {
                self.library.set_origin(&info.key, DownloadOrigin::Manual);
            }
            self.send(WorkerEv::Progress {
                received: have.size,
                total: have.size,
                format: have.format,
            });
            return Ok(());
        }

        let ext = match format {
            DownloadFormat::Original if !info.ext.is_empty() => info.ext.clone(),
            DownloadFormat::Original => {
                return Err(Failure::Permanent(DownloadFailure::BadData, MSG_BAD.into()));
            }
            DownloadFormat::Mp3 => "mp3".into(),
            DownloadFormat::Opus => "opus".into(),
        };
        // A partial in another format is no use.
        for f in [
            DownloadFormat::Original,
            DownloadFormat::Mp3,
            DownloadFormat::Opus,
        ] {
            if f != format {
                let _ = fs::remove_file(self.library.partial_path(&info.key, f));
            }
        }
        let part = self.library.partial_path(&info.key, format);
        let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let known = if format == DownloadFormat::Original {
            info.size
        } else {
            None
        };
        if let Some(size) = known {
            if have > size {
                let _ = fs::remove_file(&part);
                return Err(Failure::Restart("Starting again.".into()));
            }
            self.ensure_room(size - have)?;
        } else {
            self.ensure_room(0)?;
        }

        let url = {
            let mut u = self
                .core
                .server()
                .endpoint(&format!("v1/download/track/{}", info.id));
            u.query_pairs_mut().append_pair("format", format.as_str());
            u
        };
        let headers = if format == DownloadFormat::Original {
            ORIGINAL_HEADERS
        } else {
            TRANSCODE_HEADERS
        };
        let resp = self.get(url, (have > 0).then_some(have), headers).await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let f = refusal(resp).await;
            if matches!(f, Failure::Restart(_)) {
                // A 416: what we have can't be resumed.
                let _ = fs::remove_file(&part);
            }
            return Err(f);
        }
        let length = resp
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        let (offset, total) = if have > 0 && status == 206 {
            let range = resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(content_range);
            match range {
                Some((start, _, total)) if start == have => {
                    (have, total.or(length.map(|l| have + l)))
                }
                _ => {
                    let _ = fs::remove_file(&part);
                    return Err(Failure::Restart("Starting again.".into()));
                }
            }
        } else if status == 200 {
            // The whole file (nothing to resume, or the server ignored the range). Without
            // a length, an original's size from `/info` is what may arrive.
            (0, length.or(known))
        } else {
            return Err(Failure::Permanent(
                DownloadFailure::BadData,
                format!("Unexpected answer from the server (HTTP {status})."),
            ));
        };
        if let (Some(size), Some(total)) = (known, total)
            && size != total
        {
            let _ = fs::remove_file(&part);
            return Err(retry("The download's size didn't match. Retrying…"));
        }
        if let Some(t) = total {
            self.ensure_room(t.saturating_sub(offset))?;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(&part)
            .map_err(|e| io_failure(&e))?;
        let mut received = offset;
        self.send(WorkerEv::Progress {
            received,
            total: total.unwrap_or(0),
            format,
        });
        let mut last_report = Instant::now();
        let mut unchecked = 0u64;
        let mut body = resp.into_body();
        loop {
            let frame = match tokio::time::timeout(BODY_READ, body.frame()).await {
                Ok(Some(Ok(f))) => f,
                Ok(None) => break,
                // Cut off or stalled: keep what arrived; the next attempt resumes.
                Ok(Some(Err(_))) | Err(_) => {
                    let _ = file.sync_data();
                    return Err(retry(MSG_LOST));
                }
            };
            let Ok(data) = frame.into_data() else {
                continue;
            };
            if total.is_some_and(|t| received + data.len() as u64 > t) {
                drop(file);
                let _ = fs::remove_file(&part);
                return Err(retry("The server sent more than it said. Retrying…"));
            }
            file.write_all(&data).map_err(|e| io_failure(&e))?;
            received += data.len() as u64;
            unchecked += data.len() as u64;
            if total.is_none() && self.policy.cap_bytes > 0 && unchecked >= CAP_CHECK_EVERY {
                // A transcode of unknown size: stop at the cap.
                unchecked = 0;
                self.ensure_room(0)?;
            }
            if last_report.elapsed() >= PROGRESS_EVERY {
                last_report = Instant::now();
                self.send(WorkerEv::Progress {
                    received,
                    total: total.unwrap_or(0),
                    format,
                });
            }
        }
        file.sync_all().map_err(|e| io_failure(&e))?;
        drop(file);
        if total.is_some_and(|t| received < t) {
            return Err(retry(MSG_LOST)); // resumes from `received`
        }
        if received == 0 || known.is_some_and(|k| k != received) {
            let _ = fs::remove_file(&part);
            return Err(retry("The download's size didn't match. Retrying…"));
        }
        let dest = self.library.final_path(&info.key, &ext);
        fs::rename(&part, &dest).map_err(|e| io_failure(&e))?;
        let track = OfflineTrack {
            key: info.key.clone(),
            track_id: info.id,
            title: info.title.clone(),
            artist: info.artist.clone(),
            album: info.album.clone(),
            album_id: info.album_id.or(self.album_id),
            track_number: info.tracknum,
            duration_secs: info.duration,
            format,
            codec: if format == DownloadFormat::Original {
                info.codec.clone()
            } else {
                format.as_str().into()
            },
            original: (!info.codec.is_empty()).then(|| crate::quality::QualityFile {
                kind: Some(info.codec.clone()),
                rate: info.rate,
                bits: info.bits,
                kbps: None,
                channels: None,
            }),
            ext,
            size: received,
            downloaded_at: now_secs(),
            origin: self.origin,
        };
        if let Err(e) = self.library.add(track.clone()) {
            let _ = fs::remove_file(&dest);
            return Err(Failure::Permanent(DownloadFailure::Failed, e.to_string()));
        }
        self.send(WorkerEv::Progress {
            received,
            total: received,
            format,
        });
        self.send(WorkerEv::LibraryChanged);
        self.cache_art(&track).await;
        Ok(())
    }

    /// The album's cover, once per album (a failure leaves the placeholder).
    async fn cache_art(&self, t: &OfflineTrack) {
        let r = t.album_ref();
        if self.library.has_art(&r) {
            return;
        }
        let art = Art::new(self.session.clone());
        if let Ok(img) = art
            .fetch(format!("music/{}/cover", t.track_id), ART_PX)
            .await
            && self.library.save_art(&r, &img.bytes)
        {
            self.send(WorkerEv::LibraryChanged);
        }
    }

    /// Run one download to the end: retries, waits and backoff included.
    async fn run(self) {
        let mut attempts = 0u32;
        let mut waits = 0u32;
        let outcome = loop {
            match self.attempt().await {
                Ok(()) => break Ok(()),
                Err(Failure::Permanent(f, m)) => break Err((f, m)),
                Err(Failure::Restart(m)) => {
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        break Err((DownloadFailure::Failed, MSG_GAVE_UP.into()));
                    }
                    self.send(WorkerEv::Waiting(m));
                }
                Err(Failure::Retry {
                    message,
                    wait,
                    counts,
                }) => {
                    if counts {
                        attempts += 1;
                    } else {
                        waits += 1;
                    }
                    if attempts >= MAX_ATTEMPTS || waits > MAX_WAITS {
                        break Err((DownloadFailure::Failed, MSG_GAVE_UP.into()));
                    }
                    self.send(WorkerEv::Waiting(message));
                    tokio::time::sleep(wait.unwrap_or_else(|| backoff(attempts))).await;
                }
            }
        };
        self.send(WorkerEv::Finished(outcome));
    }
}

// --- The actor -------------------------------------------------------------------------

enum WorkerEv {
    Described {
        title: String,
        album_id: Option<i64>,
        key: String,
    },
    Progress {
        received: u64,
        total: u64,
        format: DownloadFormat,
    },
    Waiting(String),
    LibraryChanged,
    Finished(Result<(), (DownloadFailure, String)>),
}

enum Msg {
    Enqueue(
        Vec<DownloadRequest>,
        FormatChoice,
        Option<std::sync::mpsc::Sender<u32>>,
    ),
    Pause(Option<i64>),
    Resume(Option<i64>),
    Cancel(Option<i64>),
    ClearFinished,
    Policy(DownloadPolicy),
    Network(NetworkKind),
    Shutdown,
    Worker {
        id: i64,
        epoch: u64,
        ev: WorkerEv,
    },
}

struct Item {
    req: DownloadRequest,
    choice: FormatChoice,
    state: DownloadState,
    title: String,
    received: u64,
    total: u64,
    format: Option<DownloadFormat>,
    failure: Option<DownloadFailure>,
    message: String,
    /// The stable key, once the server has said (names its partial file).
    key: String,
    /// Bumped whenever the item is stopped, so a stopped worker's messages are ignored.
    epoch: u64,
    task: Option<tokio::task::AbortHandle>,
}

impl Item {
    fn view(&self) -> DownloadItem {
        DownloadItem {
            track_id: self.req.track_id,
            album_id: self.req.album_id,
            title: self.title.clone(),
            state: self.state,
            received: self.received,
            total: self.total,
            format: self.format,
            origin: self.req.origin,
            failure: self.failure,
            message: self.message.clone(),
        }
    }

    fn running(&self) -> bool {
        matches!(self.state, DownloadState::Active | DownloadState::Waiting) && self.task.is_some()
    }

    fn stop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.epoch += 1;
    }
}

struct Actor {
    core: Arc<Core>,
    session: Arc<Session>,
    http: Http,
    library: Arc<OfflineLibrary>,
    observer: Arc<dyn DownloadObserver>,
    items: Vec<Item>,
    policy: DownloadPolicy,
    network: NetworkKind,
    forbidden: bool,
    tx: mpsc::UnboundedSender<Msg>,
    snapshot: Arc<Mutex<DownloadsSnapshot>>,
    last_publish: Option<Instant>,
}

impl Actor {
    fn allowed_network(&self) -> bool {
        match self.network {
            NetworkKind::Offline => false,
            NetworkKind::Mobile => !self.policy.wifi_only,
            NetworkKind::Wifi => true,
        }
    }

    fn item(&mut self, id: i64) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.req.track_id == id)
    }

    fn enqueue(&mut self, reqs: Vec<DownloadRequest>, choice: FormatChoice) -> u32 {
        let mut n = 0;
        for r in reqs {
            if r.track_id <= 0 || r.track_id > 999_999_999_999 {
                continue;
            }
            let album_id = r.album_id.filter(|a| *a > 0 && *a <= 999_999_999_999);
            // Already on the device: nothing to fetch (the user's choice keeps a smart
            // download for good).
            if let Some(have) = self.library.track_by_id(r.track_id) {
                if r.origin == DownloadOrigin::Manual && have.origin != DownloadOrigin::Manual {
                    self.library.set_origin(&have.key, DownloadOrigin::Manual);
                    self.observer.on_library_changed();
                }
                continue;
            }
            if let Some(it) = self.item(r.track_id) {
                // The user's choice outranks a smart download of the same track.
                if r.origin == DownloadOrigin::Manual {
                    it.req.origin = DownloadOrigin::Manual;
                }
                if matches!(
                    it.state,
                    DownloadState::Failed | DownloadState::Paused | DownloadState::Done
                ) {
                    it.state = DownloadState::Queued;
                    it.failure = None;
                    it.message.clear();
                    it.choice = choice;
                    n += 1;
                }
                continue;
            }
            if self.items.len() >= MAX_QUEUE {
                break;
            }
            self.items.push(Item {
                req: DownloadRequest {
                    track_id: r.track_id,
                    album_id,
                    origin: r.origin,
                },
                choice,
                state: DownloadState::Queued,
                title: String::new(),
                received: 0,
                total: 0,
                format: None,
                failure: None,
                message: String::new(),
                key: String::new(),
                epoch: 0,
                task: None,
            });
            n += 1;
        }
        n
    }

    fn pump(&mut self) {
        if !self.allowed_network() {
            return;
        }
        loop {
            let running = self.items.iter().filter(|i| i.running()).count();
            if running >= CONCURRENCY {
                return;
            }
            let Some(idx) = self
                .items
                .iter()
                .position(|i| i.state == DownloadState::Queued)
            else {
                return;
            };
            let job = Job {
                id: self.items[idx].req.track_id,
                epoch: self.items[idx].epoch,
                album_id: self.items[idx].req.album_id,
                choice: self.items[idx].choice,
                origin: self.items[idx].req.origin,
                core: self.core.clone(),
                session: self.session.clone(),
                http: self.http.clone(),
                library: self.library.clone(),
                policy: self.policy.clone(),
                network: self.network,
                tx: self.tx.clone(),
            };
            let task = tokio::spawn(job.run());
            let it = &mut self.items[idx];
            it.state = DownloadState::Active;
            it.message.clear();
            it.task = Some(task.abort_handle());
        }
    }

    /// Stop running downloads (keeping their bytes) and queue them again: the network
    /// no longer allows downloading.
    fn hold_running(&mut self, why: &str) {
        for it in self.items.iter_mut().filter(|i| i.running()) {
            it.stop();
            it.state = DownloadState::Queued;
            it.message = why.into();
        }
    }

    fn on_worker(&mut self, id: i64, epoch: u64, ev: WorkerEv) {
        if let WorkerEv::LibraryChanged = ev {
            self.observer.on_library_changed();
            return;
        }
        let Some(it) = self.items.iter_mut().find(|i| i.req.track_id == id) else {
            return;
        };
        if it.epoch != epoch {
            return; // from a worker that was stopped
        }
        match ev {
            WorkerEv::Described {
                title,
                album_id,
                key,
            } => {
                it.title = title;
                it.key = key;
                it.req.album_id = it.req.album_id.or(album_id);
                it.state = DownloadState::Active;
            }
            WorkerEv::Progress {
                received,
                total,
                format,
            } => {
                it.received = received;
                it.total = total;
                it.format = Some(format);
                it.state = DownloadState::Active;
                it.message.clear();
            }
            WorkerEv::Waiting(m) => {
                it.state = DownloadState::Waiting;
                it.message = m;
            }
            WorkerEv::Finished(r) => {
                it.task = None;
                match r {
                    Ok(()) => {
                        it.state = DownloadState::Done;
                        it.failure = None;
                        it.message.clear();
                    }
                    Err((f, m)) => {
                        it.state = DownloadState::Failed;
                        it.failure = Some(f);
                        it.message = m.clone();
                        if f == DownloadFailure::NotAllowed && m == MSG_NOT_ALLOWED {
                            self.forbidden = true;
                        }
                    }
                }
                self.save_queue();
            }
            WorkerEv::LibraryChanged => {}
        }
    }

    fn pause(&mut self, id: Option<i64>) {
        for it in self.items.iter_mut() {
            if id.is_some_and(|i| i != it.req.track_id) {
                continue;
            }
            if matches!(
                it.state,
                DownloadState::Queued | DownloadState::Active | DownloadState::Waiting
            ) {
                it.stop();
                it.state = DownloadState::Paused;
                it.message.clear();
            }
        }
    }

    fn resume(&mut self, id: Option<i64>) {
        if id.is_none() {
            // Resuming everything also means trying again after a refusal.
            self.forbidden = false;
        }
        for it in self.items.iter_mut() {
            if id.is_some_and(|i| i != it.req.track_id) {
                continue;
            }
            if matches!(it.state, DownloadState::Paused | DownloadState::Failed) {
                it.state = DownloadState::Queued;
                it.failure = None;
                it.message.clear();
            }
        }
    }

    /// Stop downloads and throw away what they had stored.
    fn cancel(&mut self, id: Option<i64>) {
        let lib = self.library.clone();
        // Files another id still being fetched shares (the same file after a rescan): its
        // partial file is that download's too.
        let shared: HashSet<String> = self
            .items
            .iter()
            .filter(|i| id.is_some_and(|c| c != i.req.track_id))
            .filter(|i| i.state != DownloadState::Done)
            .map(|i| i.key.clone())
            .collect();
        let mut keep = Vec::new();
        for mut it in self.items.drain(..) {
            if id.is_some_and(|i| i != it.req.track_id) {
                keep.push(it);
                continue;
            }
            it.stop();
            if it.state != DownloadState::Done
                && index::valid_key(&it.key)
                && !shared.contains(&it.key)
            {
                for f in [
                    DownloadFormat::Original,
                    DownloadFormat::Mp3,
                    DownloadFormat::Opus,
                ] {
                    let _ = fs::remove_file(lib.partial_path(&it.key, f));
                }
            }
        }
        self.items = keep;
        if id.is_none() {
            // Nothing queued: no partial file is wanted.
            if let Ok(rd) = fs::read_dir(lib.partial_dir()) {
                for e in rd.flatten() {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
    }

    fn save_queue(&self) {
        let list: Vec<Value> = self
            .items
            .iter()
            .filter(|i| !matches!(i.state, DownloadState::Done | DownloadState::Failed))
            .map(|i| {
                json!({
                    "id": i.req.track_id,
                    "album_id": i.req.album_id,
                    "choice": i.choice.as_str(),
                    "origin": i.req.origin.as_str(),
                    "paused": i.state == DownloadState::Paused,
                })
            })
            .collect();
        let _ = write_atomic(
            &self.library.queue_path(),
            &serde_json::to_vec(&Value::Array(list)).expect("JSON"),
            false,
        );
    }

    fn publish(&mut self, force: bool) {
        if !force
            && self
                .last_publish
                .is_some_and(|t| t.elapsed() < PROGRESS_EVERY)
        {
            return;
        }
        self.last_publish = Some(Instant::now());
        let items: Vec<DownloadItem> = self.items.iter().map(Item::view).collect();
        let busy = items
            .iter()
            .filter(|i| {
                matches!(
                    i.state,
                    DownloadState::Queued | DownloadState::Active | DownloadState::Waiting
                )
            })
            .count() as u32;
        let snap = DownloadsSnapshot {
            items,
            forbidden: self.forbidden,
            waiting_for_network: busy > 0 && !self.allowed_network(),
            network: self.network,
            used_bytes: self.library.used_bytes(),
            cap_bytes: self.policy.cap_bytes,
            busy,
        };
        *self.snapshot.lock().unwrap() = snap.clone();
        self.observer.on_downloads(snap);
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Msg>) {
        self.pump();
        self.publish(true);
        while let Some(msg) = rx.recv().await {
            let mut force = true;
            match msg {
                Msg::Enqueue(reqs, choice, reply) => {
                    let n = self.enqueue(reqs, choice);
                    if let Some(r) = reply {
                        let _ = r.send(n);
                    }
                    self.save_queue();
                }
                Msg::Pause(id) => {
                    self.pause(id);
                    self.save_queue();
                }
                Msg::Resume(id) => {
                    self.resume(id);
                    self.save_queue();
                }
                Msg::Cancel(id) => {
                    self.cancel(id);
                    self.save_queue();
                }
                Msg::ClearFinished => {
                    self.items.retain(|i| {
                        !matches!(i.state, DownloadState::Done | DownloadState::Failed)
                    });
                }
                Msg::Policy(p) => {
                    let was = self.allowed_network();
                    self.policy = p;
                    if was && !self.allowed_network() {
                        self.hold_running("Waiting for Wi-Fi.");
                    }
                }
                Msg::Network(n) => {
                    let was = self.allowed_network();
                    self.network = n;
                    if was && !self.allowed_network() {
                        let why = if n == NetworkKind::Offline {
                            "Waiting for a connection."
                        } else {
                            "Waiting for Wi-Fi."
                        };
                        self.hold_running(why);
                    }
                }
                Msg::Shutdown => {
                    for it in self.items.iter_mut() {
                        it.stop();
                    }
                    return;
                }
                Msg::Worker { id, epoch, ev } => {
                    force = matches!(
                        ev,
                        WorkerEv::Finished(_) | WorkerEv::Waiting(_) | WorkerEv::Described { .. }
                    );
                    self.on_worker(id, epoch, ev);
                }
            }
            self.pump();
            self.publish(force);
        }
    }
}

/// Restore the queue saved by an earlier run (malformed entries dropped).
fn load_queue(lib: &OfflineLibrary) -> Vec<(DownloadRequest, FormatChoice, bool)> {
    let Some(bytes) = read_limited(&lib.queue_path(), 4 * 1024 * 1024) else {
        return Vec::new();
    };
    let Ok(Value::Array(list)) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    list.iter()
        .filter_map(|e| {
            let id = e
                .get("id")?
                .as_i64()
                .filter(|n| (1..=999_999_999_999).contains(n))?;
            if !seen.insert(id) {
                return None;
            }
            let album_id = e
                .get("album_id")
                .and_then(Value::as_i64)
                .filter(|n| (1..=999_999_999_999).contains(n));
            let choice = e
                .get("choice")
                .and_then(Value::as_str)
                .and_then(FormatChoice::parse)
                .unwrap_or(FormatChoice::Auto);
            let origin = e
                .get("origin")
                .and_then(Value::as_str)
                .and_then(DownloadOrigin::parse)
                .unwrap_or(DownloadOrigin::Manual);
            let paused = e.get("paused").and_then(Value::as_bool).unwrap_or(false);
            Some((
                DownloadRequest {
                    track_id: id,
                    album_id,
                    origin,
                },
                choice,
                paused,
            ))
        })
        .take(MAX_QUEUE)
        .collect()
}

/// Delete partial files with names this version doesn't make (`<64 hex>.<format>`).
fn tidy_partials(lib: &OfflineLibrary, queue_empty: bool) {
    let Ok(rd) = fs::read_dir(lib.partial_dir()) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let ok = name
            .split_once('.')
            .is_some_and(|(k, f)| index::valid_key(k) && DownloadFormat::parse(f).is_some());
        if !ok || queue_empty {
            let _ = fs::remove_file(e.path());
        }
    }
}

// --- The app-facing object -------------------------------------------------------------

/// Downloads for the signed-in user into their [`OfflineLibrary`].
#[derive(uniffi::Object)]
pub struct Downloader {
    tx: mpsc::UnboundedSender<Msg>,
    snapshot: Arc<Mutex<DownloadsSnapshot>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[uniffi::export]
impl Downloader {
    /// Start the downloader (its own thread). Downloads saved by an earlier run carry on
    /// once the network allows ([`Downloader::set_network`]); the app should set the
    /// policy and network straight away.
    #[uniffi::constructor]
    pub fn new(
        session: Arc<Session>,
        library: Arc<OfflineLibrary>,
        policy: DownloadPolicy,
        network: NetworkKind,
        observer: Arc<dyn DownloadObserver>,
    ) -> Result<Arc<Self>, ApiError> {
        // Only into the signed-in account's own library.
        library.check_owner(&session)?;
        let http = Http::private()?;
        let (tx, rx) = mpsc::unbounded_channel();
        let snapshot = Arc::new(Mutex::new(DownloadsSnapshot {
            items: Vec::new(),
            forbidden: false,
            waiting_for_network: false,
            network,
            used_bytes: library.used_bytes(),
            cap_bytes: policy.cap_bytes,
            busy: 0,
        }));
        let saved = load_queue(&library);
        tidy_partials(&library, saved.is_empty());
        let mut actor = Actor {
            core: session.core().clone(),
            session,
            http,
            library,
            observer,
            items: Vec::new(),
            policy,
            network,
            forbidden: false,
            tx: tx.clone(),
            snapshot: snapshot.clone(),
            last_publish: None,
        };
        for (req, choice, paused) in saved {
            let before = actor.items.len();
            actor.enqueue(vec![req], choice);
            // Only if this one was queued: one skipped (already on the device, say) must
            // not pause the one before it.
            if paused
                && actor.items.len() > before
                && let Some(it) = actor.items.last_mut()
            {
                it.state = DownloadState::Paused;
            }
        }
        let thread = std::thread::Builder::new()
            .name("anywhere-downloads".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("downloads runtime");
                rt.block_on(actor.run(rx));
            })
            .map_err(|_| ApiError::invalid("could not start downloads"))?;
        Ok(Arc::new(Downloader {
            tx,
            snapshot,
            thread: Mutex::new(Some(thread)),
        }))
    }

    /// Queue tracks (in order). Tracks already on the device are recognised by their stable
    /// key and not fetched again. Invalid ids are ignored.
    pub fn enqueue(&self, requests: Vec<DownloadRequest>, choice: FormatChoice) {
        let _ = self.tx.send(Msg::Enqueue(requests, choice, None));
    }

    /// Pause one download (`None`: all), keeping what it has.
    pub fn pause(&self, track_id: Option<i64>) {
        let _ = self.tx.send(Msg::Pause(track_id));
    }

    /// Resume (or retry) one download (`None`: all).
    pub fn resume(&self, track_id: Option<i64>) {
        let _ = self.tx.send(Msg::Resume(track_id));
    }

    /// Stop one download (`None`: all) and throw away what it had.
    pub fn cancel(&self, track_id: Option<i64>) {
        let _ = self.tx.send(Msg::Cancel(track_id));
    }

    /// Forget finished and failed entries.
    pub fn clear_finished(&self) {
        let _ = self.tx.send(Msg::ClearFinished);
    }

    pub fn set_policy(&self, policy: DownloadPolicy) {
        let _ = self.tx.send(Msg::Policy(policy));
    }

    pub fn set_network(&self, network: NetworkKind) {
        let _ = self.tx.send(Msg::Network(network));
    }

    pub fn snapshot(&self) -> DownloadsSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    /// Stop everything (what was stored is kept, and resumes next time).
    pub fn shutdown(&self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(t) = self.thread.lock().unwrap().take()
            && t.thread().id() != std::thread::current().id()
        {
            let _ = t.join();
        }
    }
}

impl Downloader {
    /// Queue and wait until the actor has taken them (tests).
    pub fn enqueue_counted(&self, requests: Vec<DownloadRequest>, choice: FormatChoice) -> u32 {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = self.tx.send(Msg::Enqueue(requests, choice, Some(tx)));
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0)
    }
}

impl Drop for Downloader {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_json() -> Value {
        json!({"id": 123, "key": "ab".repeat(32), "title": "T", "artist": "A", "album": "Al",
            "album_id": 7, "tracknum": 3, "duration": 241.2,
            "original": {"format": "flc", "content_type": "audio/flac", "size": 1000, "ext": "flac"},
            "formats": ["original", "mp3", "opus"]})
    }

    #[test]
    fn info_is_checked() {
        let i = parse_info(&info_json(), 123).unwrap();
        assert_eq!(i.key, "ab".repeat(32));
        assert_eq!(i.size, Some(1000));
        assert_eq!(i.ext, "flac");
        assert_eq!(i.codec, "flc");
        assert_eq!(i.formats.len(), 3);
        assert_eq!(parse_info(&info_json(), 124), Err("id"));
        for (k, v, err) in [
            ("key", json!("../x"), "key"),
            ("key", json!("AB".repeat(32)), "key"),
            ("formats", json!(["wav"]), "formats"),
            ("formats", json!([]), "formats"),
            ("original", json!("x"), "original"),
        ] {
            let mut j = info_json();
            j[k] = v;
            assert_eq!(parse_info(&j, 123), Err(err), "{k}");
        }
        let mut j = info_json();
        j["original"]["size"] = json!(-5);
        assert_eq!(parse_info(&j, 123), Err("size"));
        j["original"]["size"] = Value::Null;
        j["original"]["ext"] = json!("sh");
        j["title"] = json!("x\u{202e}y\n");
        j["album_id"] = json!(0);
        let i = parse_info(&j, 123).unwrap();
        assert_eq!(i.size, None);
        assert_eq!(i.ext, "", "unknown extensions are never used");
        assert_eq!(i.title, "xy");
        assert_eq!(i.album_id, None);
        assert!(parse_info(&json!([1]), 123).is_err());
    }

    /// The original's rate and depth are optional, and junk reads as unknown, never as a
    /// refusal.
    #[test]
    fn info_takes_the_originals_rate_and_depth_when_given() {
        let i = parse_info(&info_json(), 123).unwrap();
        assert_eq!((i.rate, i.bits), (None, None));
        let mut j = info_json();
        j["original"]["rate"] = json!(96_000);
        j["original"]["bits"] = json!(24);
        let i = parse_info(&j, 123).unwrap();
        assert_eq!((i.rate, i.bits), (Some(96_000), Some(24)));
        for (rate, bits) in [
            (json!(0), json!(0)),
            (json!(-44_100), json!(-1)),
            (json!("96000"), json!("24")),
            (json!(1e300), json!(65)),
            (json!(u64::MAX), json!(null)),
        ] {
            j["original"]["rate"] = rate;
            j["original"]["bits"] = bits;
            let i = parse_info(&j, 123).unwrap();
            assert_eq!((i.rate, i.bits), (None, None));
        }
    }

    #[test]
    fn format_choice_falls_back_like_the_web() {
        use DownloadFormat::*;
        assert_eq!(pick_format(Opus, &[Original, Opus]), Opus);
        assert_eq!(pick_format(Mp3, &[Original, Opus]), Original);
        assert_eq!(pick_format(Mp3, &[Opus]), Opus);
    }

    #[test]
    fn content_ranges() {
        assert_eq!(
            content_range("bytes 100-199/200"),
            Some((100, 199, Some(200)))
        );
        assert_eq!(content_range("bytes 0-0/*"), Some((0, 0, None)));
        for bad in [
            "",
            "bytes 1-0/5",
            "bytes 0-5/5",
            "bytes -1-2/3",
            "items 0-1/2",
            "bytes 0-1/x",
            "bytes 99999999999999999-1/2",
        ] {
            assert_eq!(content_range(bad), None, "{bad}");
        }
    }

    #[test]
    fn refusals_are_classified() {
        assert_eq!(
            classify(403, "downloads are not allowed for this account", None),
            Failure::Permanent(DownloadFailure::NotAllowed, MSG_NOT_ALLOWED.into())
        );
        assert_eq!(
            classify(403, "this format is not allowed for this account", None),
            Failure::Permanent(DownloadFailure::NotAllowed, MSG_FORMAT.into())
        );
        assert!(matches!(
            classify(404, "", None),
            Failure::Permanent(DownloadFailure::NotDownloadable, _)
        ));
        assert!(matches!(classify(416, "", None), Failure::Restart(_)));
        assert_eq!(
            classify(429, "", Some(Duration::from_secs(9))),
            Failure::Retry {
                message: MSG_BUSY.into(),
                wait: Some(Duration::from_secs(9)),
                counts: false
            }
        );
        assert!(matches!(
            classify(503, "", None),
            Failure::Retry { counts: true, .. }
        ));
        assert!(matches!(
            classify(400, "", None),
            Failure::Permanent(DownloadFailure::Failed, _)
        ));
        assert!(matches!(
            classify(401, "", None),
            Failure::Permanent(DownloadFailure::SignedOut, _)
        ));
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(50), Duration::from_secs(60));
    }

    #[test]
    fn the_saved_queue_is_checked() {
        let root = crate::offline::store::tests::temp_root();
        let lib = OfflineLibrary::open(root, "https://s".into(), "u".into()).unwrap();
        fs::write(
            lib.queue_path(),
            serde_json::to_vec(&json!([
                {"id": 5, "album_id": 7, "choice": "opus", "origin": "recent", "paused": true},
                {"id": 5},
                {"id": -1},
                {"id": "6"},
                {"id": 8, "choice": "wav", "origin": "?"},
                "junk"
            ]))
            .unwrap(),
        )
        .unwrap();
        let q = load_queue(&lib);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].0.album_id, Some(7));
        assert_eq!(q[0].1, FormatChoice::Opus);
        assert_eq!(q[0].0.origin, DownloadOrigin::Recent);
        assert!(q[0].2);
        assert_eq!(
            (q[1].1, q[1].0.origin, q[1].2),
            (FormatChoice::Auto, DownloadOrigin::Manual, false)
        );
        fs::write(lib.queue_path(), b"\xff\x00").unwrap();
        assert!(load_queue(&lib).is_empty());
    }

    #[test]
    fn stray_partials_are_removed() {
        let root = crate::offline::store::tests::temp_root();
        let lib = OfflineLibrary::open(root, "https://s".into(), "u".into()).unwrap();
        let good = lib.partial_path(&"cd".repeat(32), DownloadFormat::Opus);
        let bad = lib.partial_dir().join("evil.sh");
        fs::write(&good, b"x").unwrap();
        fs::write(&bad, b"x").unwrap();
        tidy_partials(&lib, false);
        assert!(good.exists() && !bad.exists());
        tidy_partials(&lib, true);
        assert!(!good.exists());
    }
}
