//! Offline downloads (spec §5.3, §8): `GET /v1/download/track/{id}/info` and
//! `GET /v1/download/track/{id}?format=original|mp3|opus`.
//!
//! - **Who:** a valid access token, and the user's download permission (on by default; an
//!   admin can turn it off or limit the formats). Read from the database on every request,
//!   so a change applies at once.
//! - **What:** only local library files (LMS `url` starting `file://`). Streaming-service
//!   and radio tracks are refused.
//! - **Unknown ids:** LMS's `/music/<id>/download` hangs for a track id that doesn't exist,
//!   so every id is first checked to be a positive integer and looked up with JSON-RPC
//!   (`titles … track_id:<id>`, with a time limit). Only a track LMS knows is ever fetched.
//! - **Original:** LMS's `/music/<id>/download` (never `/stream.mp3`, which can stream for
//!   ever), relayed without buffering, `Range` passed through, with time limits on
//!   connecting, on the response headers and on every read.
//! - **Transcodes:** see [`crate::transcode`]. Served from the cache, with `Range`.
//! - **Limits:** [`crate::Config::downloads_per_user`] at once per user (429 beyond),
//!   audit entries at most once an hour per track and device.
//! - **Stable key:** LMS renumbers tracks on a rescan, so clients store downloads by `key`:
//!   SHA-256 of a gateway secret and the track's library URL. It identifies the file without
//!   revealing its path.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::AppState;
use crate::accounts::{DownloadPolicy, Principal};
use crate::transcode::{self, Format, Tags};

/// Largest original fetched for transcoding.
const MAX_SOURCE: u64 = 4 << 30;
/// How often a running download re-checks that its device is still signed in.
const RECHECK: Duration = Duration::from_secs(5);
/// One audit entry per track and device per this long.
const AUDIT_EVERY: Duration = Duration::from_secs(3600);
const AUDIT_CAP: usize = 10_000;
const CHUNK: usize = 64 * 1024;
/// Pause before the one retry of a download request LMS failed at the connection level or
/// answered with a 5xx.
const RETRY_PAUSE: Duration = Duration::from_millis(250);

pub const NOT_ALLOWED: &str = "downloads are not allowed for this account";
pub const FORMAT_NOT_ALLOWED: &str = "this format is not allowed for this account";

fn error(status: StatusCode, message: &str) -> Response {
    let mut r = (status, Json(json!({ "error": message }))).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// A track id from the URL: a positive decimal integer, no sign, no leading zero, at most
/// 12 digits. Anything else is refused before LMS is asked.
pub fn parse_track_id(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 12 || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    s.parse().ok().filter(|n| *n > 0)
}

/// The query string: nothing, or exactly `format=<original|mp3|opus>`.
pub fn parse_format_query(q: Option<&str>) -> Option<Format> {
    match q {
        None | Some("") => Some(Format::Original),
        Some(q) => Format::parse(q.strip_prefix("format=")?),
    }
}

/// A single byte range as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// `bytes=a-`
    From(u64),
    /// `bytes=a-b`
    FromTo(u64, u64),
    /// `bytes=-n` (the last n bytes)
    Suffix(u64),
}

/// A `Range` header that is not one well-formed byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadRange;

/// Parse a `Range` header. `Ok(None)` without one; `Err` for anything but one well-formed
/// byte range (several ranges are not supported).
pub fn parse_range(h: Option<&HeaderValue>) -> Result<Option<ByteRange>, BadRange> {
    let Some(h) = h else { return Ok(None) };
    let s = h.to_str().map_err(|_| BadRange)?.trim();
    if s.len() > 64 {
        return Err(BadRange);
    }
    let spec = s
        .get(..6)
        .filter(|p| p.eq_ignore_ascii_case("bytes="))
        .map(|_| &s[6..])
        .ok_or(BadRange)?
        .trim();
    let num = |t: &str| -> Result<u64, BadRange> {
        if t.is_empty() || t.len() > 19 || !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(BadRange);
        }
        t.parse().map_err(|_| BadRange)
    };
    let (a, b) = spec.split_once('-').ok_or(BadRange)?;
    let r = match (a.trim(), b.trim()) {
        ("", n) => ByteRange::Suffix(num(n)?),
        (a, "") => ByteRange::From(num(a)?),
        (a, b) => {
            let (a, b) = (num(a)?, num(b)?);
            if a > b {
                return Err(BadRange);
            }
            ByteRange::FromTo(a, b)
        }
    };
    if r == ByteRange::Suffix(0) {
        return Err(BadRange);
    }
    Ok(Some(r))
}

/// The inclusive byte span a range selects from `total` bytes, or `None` if unsatisfiable.
pub fn resolve(r: ByteRange, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    match r {
        ByteRange::From(a) => (a < total).then_some((a, total - 1)),
        ByteRange::FromTo(a, b) => (a < total).then_some((a, b.min(total - 1))),
        ByteRange::Suffix(n) => Some((total - n.min(total), total - 1)),
    }
}

/// `Content-Range: bytes a-b/total` from LMS, checked.
fn parse_content_range(v: Option<&HeaderValue>) -> Option<(u64, u64, u64)> {
    let s = v?.to_str().ok()?.strip_prefix("bytes ")?;
    let (span, total) = s.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    let (a, b, total): (u64, u64, u64) = (a.parse().ok()?, b.parse().ok()?, total.parse().ok()?);
    (a <= b && b < total).then_some((a, b, total))
}

/// ASCII characters kept as they are in a download's file name.
fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || " -_.,()&+!'[]".contains(c)
}

fn tidy(s: String) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let last = out.chars().last();
        if (c == ' ' && last == Some(' ')) || (c == '.' && last == Some('.')) {
            continue;
        }
        out.push(c);
    }
    let out: String = out
        .trim_matches([' ', '.', '_'])
        .chars()
        .take(100)
        .collect();
    out.trim_matches([' ', '.']).to_string()
}

/// The `Content-Disposition` value for a download: `attachment; filename="…"` with a safe
/// ASCII name (no quotes, backslashes, path separators or control characters), plus an
/// RFC 5987 `filename*` with the Unicode name when it differs.
pub fn content_disposition(
    tracknum: Option<i64>,
    artist: &str,
    title: &str,
    id: u64,
    ext: &str,
) -> String {
    let mut stem = String::new();
    if let Some(n) = tracknum.filter(|n| (1..1000).contains(n)) {
        stem.push_str(&format!("{n:02} "));
    }
    if !artist.trim().is_empty() {
        stem.push_str(artist);
        stem.push_str(" - ");
    }
    stem.push_str(title);
    let ascii = tidy(
        stem.chars()
            .map(|c| {
                if plain(c) {
                    c
                } else if c.is_whitespace() || c.is_control() {
                    ' '
                } else {
                    '_'
                }
            })
            .collect(),
    );
    let ascii = if ascii.chars().any(|c| c.is_ascii_alphanumeric()) {
        ascii
    } else {
        format!("track {id}")
    };
    let bidi = |c: char| matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    let unicode = tidy(
        stem.chars()
            .map(|c| {
                if c.is_control() || c.is_whitespace() || bidi(c) {
                    ' '
                } else if matches!(c, '/' | '\\' | '"' | ':' | '*' | '?' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect(),
    );
    let mut v = format!("attachment; filename=\"{ascii}.{ext}\"");
    if !unicode.is_empty() && unicode != ascii {
        let mut enc = String::new();
        for b in format!("{unicode}.{ext}").bytes() {
            if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
                enc.push(b as char);
            } else {
                enc.push_str(&format!("%{b:02X}"));
            }
        }
        v.push_str(&format!("; filename*=UTF-8''{enc}"));
    }
    v
}

/// What the original file is, from LMS's type (and the URL's extension as a fallback):
/// (format, content type, extension).
pub fn original_kind(lms_type: &str, url: &str) -> (String, &'static str, String) {
    let (ct, ext) = match lms_type {
        "flc" => ("audio/flac", "flac"),
        "mp3" => ("audio/mpeg", "mp3"),
        "mp4" | "alc" | "m4a" => ("audio/mp4", "m4a"),
        "aac" => ("audio/aac", "aac"),
        "ogg" => ("audio/ogg", "ogg"),
        "ops" => ("audio/ogg", "opus"),
        "ogf" => ("audio/ogg", "oga"),
        "wav" => ("audio/wav", "wav"),
        "aif" => ("audio/aiff", "aiff"),
        "wma" => ("audio/x-ms-wma", "wma"),
        "wvp" => ("audio/x-wavpack", "wv"),
        "ape" => ("audio/x-ape", "ape"),
        "mpc" => ("audio/x-musepack", "mpc"),
        "dsf" => ("audio/x-dsf", "dsf"),
        "dff" => ("audio/x-dff", "dff"),
        _ => ("application/octet-stream", ""),
    };
    let ext = if ext.is_empty() {
        url.rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .filter(|e| (1..=5).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .unwrap_or_else(|| "bin".into())
    } else {
        ext.to_string()
    };
    let format = if !lms_type.is_empty()
        && lms_type.len() <= 8
        && lms_type.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        lms_type.to_string()
    } else {
        "unknown".into()
    };
    (format, ct, ext)
}

/// The stable key: hex SHA-256 of the salt and the library URL.
pub fn stable_key(salt: &str, url: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update([0u8]);
    h.update(url.as_bytes());
    hex::encode(h.finalize())
}

// ---------------------------------------------------------------------------
// Tracks
// ---------------------------------------------------------------------------

/// A library track, as LMS describes it.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<i64>,
    /// The track's (first) artist, and its year, for listening history (`crate::history`).
    pub artist_id: Option<i64>,
    pub year: Option<i64>,
    pub tracknum: Option<i64>,
    pub duration: Option<f64>,
    pub kind: String,
    pub size: Option<u64>,
    pub mtime: String,
}

fn text(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.chars().filter(|c| !c.is_control()).take(1000).collect(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn int(v: &Value, k: &str) -> Option<i64> {
    match v.get(k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn float(v: &Value, k: &str) -> Option<f64> {
    match v.get(k)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite() && *f >= 0.0)
}

#[derive(Debug)]
pub(crate) enum Refusal {
    NotFound,
    NotDownloadable,
    Unavailable,
    TimedOut,
    /// Larger than the caller's limit.
    TooLarge,
    Range(Option<u64>),
    FormatUnavailable,
    TranscodeFailed,
    TranscodeTimedOut,
}

impl Refusal {
    fn response(self) -> Response {
        match self {
            Refusal::NotFound => error(StatusCode::NOT_FOUND, "not found"),
            Refusal::NotDownloadable => error(StatusCode::NOT_FOUND, "not downloadable"),
            Refusal::FormatUnavailable => error(StatusCode::NOT_FOUND, "format not available"),
            Refusal::Unavailable => error(StatusCode::BAD_GATEWAY, "music server unavailable"),
            Refusal::TimedOut => error(StatusCode::GATEWAY_TIMEOUT, "music server timed out"),
            Refusal::TooLarge => error(StatusCode::PAYLOAD_TOO_LARGE, "track too large"),
            Refusal::TranscodeFailed => error(StatusCode::BAD_GATEWAY, "transcoding failed"),
            Refusal::TranscodeTimedOut => {
                error(StatusCode::GATEWAY_TIMEOUT, "transcoding timed out")
            }
            Refusal::Range(total) => {
                let mut r = error(StatusCode::RANGE_NOT_SATISFIABLE, "bad range");
                if let Some(t) = total
                    && let Ok(v) = HeaderValue::from_str(&format!("bytes */{t}"))
                {
                    r.headers_mut().insert(header::CONTENT_RANGE, v);
                }
                r
            }
        }
    }
}

impl crate::Gateway {
    /// Look a track up in the library (with a time limit). Only local files qualify.
    pub(crate) async fn library_track(&self, id: u64) -> Result<Track, Refusal> {
        let cmd = [
            "titles".to_string(),
            "0".into(),
            "1".into(),
            format!("track_id:{id}"),
            "tags:aelotdufnxsy".into(),
        ];
        let result = self
            .lms_query_retrying("", &cmd, self.cfg.download_lookup_timeout)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "track lookup failed");
                Refusal::Unavailable
            })?;
        let item = result
            .get("titles_loop")
            .and_then(Value::as_array)
            .and_then(|l| l.first())
            .ok_or(Refusal::NotFound)?;
        if int(item, "id") != Some(id as i64) {
            return Err(Refusal::NotFound);
        }
        let url = match item.get("url") {
            Some(Value::String(u)) => u.clone(),
            _ => return Err(Refusal::NotDownloadable),
        };
        if int(item, "remote").unwrap_or(0) != 0 || !url.starts_with("file://") {
            return Err(Refusal::NotDownloadable);
        }
        Ok(Track {
            id,
            url,
            title: text(item, "title"),
            artist: text(item, "artist"),
            album: text(item, "album"),
            album_id: int(item, "album_id"),
            artist_id: int(item, "artist_id").filter(|n| *n > 0),
            year: int(item, "year").filter(|y| (1..=9999).contains(y)),
            tracknum: int(item, "tracknum"),
            duration: float(item, "duration"),
            kind: text(item, "type"),
            size: int(item, "filesize").and_then(|n| u64::try_from(n).ok()),
            mtime: text(item, "modificationTime"),
        })
    }

    pub(crate) fn download_salt(&self) -> Option<String> {
        if let Some(s) = self.download_salt.get() {
            return Some(s.clone());
        }
        let s = self.accounts.download_key_salt().ok()?;
        Some(self.download_salt.get_or_init(|| s).clone())
    }

    /// Formats the server can produce at all: the original, plus what ffmpeg can make.
    pub fn server_formats(&self) -> Vec<Format> {
        let mut out = vec![Format::Original];
        if let Some(t) = &self.transcoder {
            out.extend(
                [Format::Mp3, Format::Opus]
                    .into_iter()
                    .filter(|f| t.ffmpeg.supports(*f)),
            );
        }
        out
    }

    /// The formats offered for this track to this user.
    fn offered(&self, policy: &DownloadPolicy, track: &Track) -> Vec<Format> {
        let transcodable = transcode::demuxer(&track.kind).is_some();
        self.server_formats()
            .into_iter()
            .filter(|f| *f == Format::Original || transcodable)
            .filter(|f| policy.allows(f.as_str()))
            .collect()
    }

    /// One of the user's concurrent download slots, if they have one free.
    fn download_slot(self: &std::sync::Arc<Self>, user: i64) -> Option<Slot> {
        let mut active = self.active_downloads.lock().unwrap();
        let n = active.entry(user).or_default();
        if *n >= self.cfg.downloads_per_user {
            return None;
        }
        *n += 1;
        Some(Slot {
            gw: self.clone(),
            user,
        })
    }

    /// Audit a download, at most once per track and device per hour.
    fn audit_download(&self, p: &Principal, id: u64, format: Format) {
        let now = Instant::now();
        {
            let mut seen = self.download_audit.lock().unwrap();
            if seen
                .get(&(p.device_id, id))
                .is_some_and(|t| now.duration_since(*t) < AUDIT_EVERY)
            {
                return;
            }
            if seen.len() >= AUDIT_CAP {
                seen.retain(|_, t| now.duration_since(*t) < AUDIT_EVERY);
                if seen.len() >= AUDIT_CAP {
                    seen.clear();
                }
            }
            seen.insert((p.device_id, id), now);
        }
        self.accounts.audit(
            "download",
            Some(&p.username),
            Some(p.device_id),
            &format!("track {id} {}", format.as_str()),
        );
    }
}

/// A held download slot; freed when the response body is finished or dropped.
pub(crate) struct Slot {
    gw: AppState,
    user: i64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut active = self.gw.active_downloads.lock().unwrap();
        if let Some(n) = active.get_mut(&self.user) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                active.remove(&self.user);
            }
        }
    }
}

/// Who is asking, and whether they may download at all.
fn authorise(
    gw: &AppState,
    headers: &HeaderMap,
) -> Result<(Principal, DownloadPolicy), Box<Response>> {
    let principal = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| gw.accounts().authenticate(t))
        .ok_or_else(|| Box::new(error(StatusCode::UNAUTHORIZED, "invalid credentials")))?;
    let policy = gw
        .accounts()
        .download_policy(principal.user_id)
        .map_err(|_| Box::new(error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")))?;
    Ok((principal, policy))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /v1/download/track/{id}/info`
pub async fn info(
    State(gw): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let (_principal, policy) = match authorise(&gw, &headers) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let Some(id) = parse_track_id(&id) else {
        return Refusal::NotFound.response();
    };
    if !policy.enabled {
        return error(StatusCode::FORBIDDEN, NOT_ALLOWED);
    }
    let track = match gw.library_track(id).await {
        Ok(t) => t,
        Err(e) => return e.response(),
    };
    let Some(salt) = gw.download_salt() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
    };
    let (format, content_type, ext) = original_kind(&track.kind, &track.url);
    let formats: Vec<&str> = gw
        .offered(&policy, &track)
        .into_iter()
        .map(Format::as_str)
        .collect();
    let mut r = Json(json!({
        "id": track.id,
        "key": stable_key(&salt, &track.url),
        "title": track.title,
        "artist": track.artist,
        "album": track.album,
        "album_id": track.album_id,
        "tracknum": track.tracknum,
        "duration": track.duration,
        "original": {
            "format": format,
            "content_type": content_type,
            "size": track.size,
            "ext": ext,
        },
        "formats": formats,
    }))
    .into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

/// `GET /v1/download/track/{id}?format=…`
pub async fn track(
    State(gw): State<AppState>,
    UrlPath(id): UrlPath<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let (principal, policy) = match authorise(&gw, &headers) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let Some(id) = parse_track_id(&id) else {
        return Refusal::NotFound.response();
    };
    let Some(format) = parse_format_query(query.as_deref()) else {
        return error(StatusCode::BAD_REQUEST, "unknown format");
    };
    if !policy.enabled {
        return error(StatusCode::FORBIDDEN, NOT_ALLOWED);
    }
    if !policy.allows(format.as_str()) {
        return error(StatusCode::FORBIDDEN, FORMAT_NOT_ALLOWED);
    }
    let Ok(range) = parse_range(headers.get(header::RANGE)) else {
        return Refusal::Range(None).response();
    };
    let Some(slot) = gw.download_slot(principal.user_id) else {
        let mut r = error(StatusCode::TOO_MANY_REQUESTS, "too many downloads at once");
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        return r;
    };
    let track = match gw.library_track(id).await {
        Ok(t) => t,
        Err(e) => return e.response(),
    };
    let watch = Watch {
        gw: gw.clone(),
        device: principal.device_id,
        _slot: slot,
    };
    let result = match format {
        Format::Original => original(&gw, &track, range, watch).await,
        f => transcoded(&gw, &track, f, range, watch).await,
    };
    match result {
        Ok(r) => {
            gw.audit_download(&principal, id, format);
            r
        }
        Err(e) => e.response(),
    }
}

// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

/// What every chunk of a download body carries: the user's slot (freed when the body
/// ends) and a periodic check that the device hasn't been signed out meanwhile.
struct Watch {
    gw: AppState,
    device: i64,
    _slot: Slot,
}

fn guarded<S>(inner: S, skip: u64, len: u64, watch: Watch) -> Body
where
    S: Stream<Item = std::io::Result<Bytes>> + Send + Unpin + 'static,
{
    struct St<S> {
        inner: S,
        skip: u64,
        left: u64,
        watch: Watch,
        checked: Instant,
    }
    let st = St {
        inner,
        skip,
        left: len,
        watch,
        checked: Instant::now(),
    };
    let s = futures_util::stream::unfold(Some(st), |st| async move {
        let mut st = st?;
        loop {
            if st.left == 0 {
                return None;
            }
            if st.checked.elapsed() >= RECHECK {
                st.checked = Instant::now();
                if !st.watch.gw.accounts().device_active(st.watch.device) {
                    return Some((Err(std::io::Error::other("device signed out")), None));
                }
            }
            match st.inner.next().await {
                Some(Ok(mut chunk)) => {
                    if st.skip > 0 {
                        let n = st.skip.min(chunk.len() as u64);
                        st.skip -= n;
                        chunk = chunk.slice(n as usize..);
                        if chunk.is_empty() {
                            continue;
                        }
                    }
                    if chunk.len() as u64 > st.left {
                        chunk.truncate(st.left as usize);
                    }
                    st.left -= chunk.len() as u64;
                    return Some((Ok(chunk), Some(st)));
                }
                Some(Err(e)) => return Some((Err(e), None)),
                None => {
                    // Shorter than promised: fail, so the client sees an incomplete download.
                    return Some((Err(std::io::Error::other("source ended early")), None));
                }
            }
        }
    });
    Body::from_stream(s)
}

fn audio_response(
    body: Body,
    content_type: &str,
    span: Option<(u64, u64, u64)>,
    len: u64,
    disposition: &str,
) -> Response {
    let mut r = Response::new(body);
    if span.is_some() {
        *r.status_mut() = StatusCode::PARTIAL_CONTENT;
    }
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Some((a, b, total)) = span
        && let Ok(v) = HeaderValue::from_str(&format!("bytes {a}-{b}/{total}"))
    {
        h.insert(header::CONTENT_RANGE, v);
    }
    if let Ok(v) = HeaderValue::from_str(disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

/// LMS answered a download in a way that can't be relayed: logged with the reason.
fn unusable(track: u64, why: &str) -> Refusal {
    tracing::warn!(track, why, "unusable download answer from LMS");
    Refusal::Unavailable
}

/// Ask LMS for a track's original file (optionally a byte span).
async fn fetch_original(
    gw: &AppState,
    id: u64,
    span: Option<(u64, u64)>,
) -> Result<reqwest::Response, Refusal> {
    let mut retried = false;
    loop {
        let mut req = gw
            .dl_http
            .get(format!("http://{}/music/{id}/download", gw.cfg.lms_http));
        if let Some((a, b)) = span {
            req = req.header(header::RANGE, format!("bytes={a}-{b}"));
        }
        let outcome = tokio::time::timeout(gw.cfg.download_header_timeout, req.send()).await;
        // A GET of a file is safe to repeat: try once more on a connection-level failure or
        // a 5xx (LMS sometimes fails a request right after others were aborted).
        let transient = match &outcome {
            Ok(Err(e)) => {
                tracing::warn!(track = id, error = %crate::rpc::LmsFailure::from_reqwest(e).message, "download request to LMS failed");
                !e.is_timeout()
            }
            Ok(Ok(r)) if r.status().is_server_error() => {
                tracing::warn!(track = id, status = %r.status(), "LMS failed a download request");
                true
            }
            Err(_) => {
                tracing::warn!(track = id, "LMS did not answer a download request in time");
                false
            }
            Ok(Ok(_)) => false,
        };
        if transient && !retried {
            retried = true;
            tokio::time::sleep(RETRY_PAUSE).await;
            continue;
        }
        return match outcome {
            Err(_) => Err(Refusal::TimedOut),
            Ok(Err(e)) if e.is_timeout() => Err(Refusal::TimedOut),
            Ok(Err(_)) => Err(Refusal::Unavailable),
            Ok(Ok(r)) => Ok(r),
        };
    }
}

fn io_stream(r: reqwest::Response) -> impl Stream<Item = std::io::Result<Bytes>> + Send + Unpin {
    r.bytes_stream().map(|c| c.map_err(std::io::Error::other))
}

async fn original(
    gw: &AppState,
    track: &Track,
    range: Option<ByteRange>,
    watch: Watch,
) -> Result<Response, Refusal> {
    let (_, content_type, ext) = original_kind(&track.kind, &track.url);
    let disposition =
        content_disposition(track.tracknum, &track.artist, &track.title, track.id, &ext);
    // With a known size, an unsatisfiable range is refused without asking LMS.
    let asked = match (range, track.size) {
        (Some(r), Some(total)) => Some(resolve(r, total).ok_or(Refusal::Range(Some(total)))?),
        _ => None,
    };
    let resp = fetch_original(gw, track.id, asked).await?;
    let status = resp.status();
    match status {
        reqwest::StatusCode::PARTIAL_CONTENT if asked.is_some() => {
            let (a, b, total) = parse_content_range(resp.headers().get(header::CONTENT_RANGE))
                .ok_or_else(|| unusable(track.id, "bad Content-Range"))?;
            let (want_a, want_b) = asked.unwrap_or((a, b));
            // LMS may send more than asked: it answers `bytes=0-0` with the whole file (an end
            // of 0 reads as "no end"). A superset of the request is fine: relay only the part
            // asked for. Anything else (a different or shorter span) is refused.
            if a > want_a || b < want_b || resp.content_length() != Some(b - a + 1) {
                return Err(unusable(track.id, "wrong range"));
            }
            let len = want_b - want_a + 1;
            let body = guarded(io_stream(resp), want_a - a, len, watch);
            Ok(audio_response(
                body,
                content_type,
                Some((want_a, want_b, total)),
                len,
                &disposition,
            ))
        }
        reqwest::StatusCode::OK => {
            // Only ever relay a body of known length (D5: never an endless stream).
            let total = resp
                .content_length()
                .ok_or_else(|| unusable(track.id, "no Content-Length"))?;
            let span = match (asked, range) {
                (Some(span), _) => Some(span),
                (None, Some(r)) => Some(resolve(r, total).ok_or(Refusal::Range(Some(total)))?),
                (None, None) => None,
            };
            match span {
                Some((a, b)) if b < total => {
                    let len = b - a + 1;
                    let body = guarded(io_stream(resp), a, len, watch);
                    Ok(audio_response(
                        body,
                        content_type,
                        Some((a, b, total)),
                        len,
                        &disposition,
                    ))
                }
                Some(_) => Err(Refusal::Range(Some(total))),
                None => {
                    let body = guarded(io_stream(resp), 0, total, watch);
                    Ok(audio_response(
                        body,
                        content_type,
                        None,
                        total,
                        &disposition,
                    ))
                }
            }
        }
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE => Err(Refusal::Range(track.size)),
        reqwest::StatusCode::NOT_FOUND => Err(Refusal::NotFound),
        _ => {
            tracing::warn!(track = track.id, %status, "LMS refused a download");
            Err(Refusal::Unavailable)
        }
    }
}

/// A temporary file, removed when dropped.
pub(crate) struct TempFile(pub(crate) std::path::PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Save a track's original into `path` (for ffmpeg or analysis), refusing anything over
/// `max` bytes (whatever LMS claims, at most `max` are ever written).
pub(crate) async fn save_original(
    gw: &AppState,
    id: u64,
    path: &Path,
    max: u64,
) -> Result<(), Refusal> {
    save_original_within(gw, id, path, max, None)
        .await
        .map(drop)
}

/// [`save_original`], holding room in `cache` for it (see [`transcode::Cache::reserve`])
/// before anything is written: the room is held until the returned reservation is dropped.
pub(crate) async fn save_original_within<'c>(
    gw: &AppState,
    id: u64,
    path: &Path,
    max: u64,
    cache: Option<&'c transcode::Cache>,
) -> Result<Option<transcode::Reservation<'c>>, Refusal> {
    let resp = fetch_original(gw, id, None).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(Refusal::NotFound);
    }
    if resp.status() != reqwest::StatusCode::OK {
        tracing::warn!(track = id, status = %resp.status(), "LMS refused a download of an original");
        return Err(Refusal::Unavailable);
    }
    let total = resp
        .content_length()
        .ok_or_else(|| unusable(id, "no Content-Length"))?;
    if total > max.min(MAX_SOURCE) {
        return Err(Refusal::TooLarge);
    }
    let room = match cache {
        Some(c) => Some(c.reserve(total).await.ok_or(Refusal::TooLarge)?),
        None => None,
    };
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|_| Refusal::TranscodeFailed)?;
    let mut stream = resp.bytes_stream();
    let mut got = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            tracing::warn!(track = id, error = %crate::rpc::LmsFailure::from_reqwest(&e).message, "download from LMS broke off");
            if e.is_timeout() {
                Refusal::TimedOut
            } else {
                Refusal::Unavailable
            }
        })?;
        got += chunk.len() as u64;
        if got > total {
            return Err(unusable(id, "longer than promised"));
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| Refusal::TranscodeFailed)?;
    }
    if got != total {
        return Err(unusable(id, "shorter than promised"));
    }
    file.flush().await.map_err(|_| Refusal::TranscodeFailed)?;
    Ok(room)
}

/// The transcode of a track, from the cache or made now. Returns an open file, so it can be
/// served even if the cache evicts it meanwhile.
async fn transcode_file(
    gw: &AppState,
    track: &Track,
    format: Format,
    key: &str,
) -> Result<std::fs::File, Refusal> {
    let tc = gw.transcoder.as_ref().ok_or(Refusal::FormatUnavailable)?;
    let demuxer = transcode::demuxer(&track.kind).ok_or(Refusal::FormatUnavailable)?;
    if !tc.ffmpeg.supports(format) {
        return Err(Refusal::FormatUnavailable);
    }
    let name = transcode::cache_name(key, format, track.size, &track.mtime);
    let open = |p: &Path| std::fs::File::open(p).ok();
    if let Some(f) = tc.cache.lookup(&name).as_deref().and_then(open) {
        return Ok(f);
    }
    let lock = tc.lock_for(&name);
    let _one = lock.lock().await;
    if let Some(f) = tc.cache.lookup(&name).as_deref().and_then(open) {
        return Ok(f);
    }
    let input = TempFile(tc.cache.temp_path("src"));
    // The original counts against the cache's cap while it is on disk, and is fetched
    // before taking a transcode slot, so a slow download never holds up others' encodes.
    let room = save_original_within(gw, track.id, &input.0, MAX_SOURCE, Some(&tc.cache)).await?;
    let _job = tc
        .jobs
        .acquire()
        .await
        .map_err(|_| Refusal::TranscodeFailed)?;
    let output = TempFile(tc.cache.temp_path(format.ext()));
    let tags = Tags {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        track: track.tracknum,
    };
    let max = tc.max_output();
    let args = transcode::args(&transcode::Job {
        input: &input.0,
        output: &output.0,
        demuxer,
        format,
        tags: &tags,
        max_bytes: max,
    });
    match transcode::run(&tc.ffmpeg.path, args, tc.cache.dir(), tc.timeout).await {
        Ok(()) => {}
        Err(transcode::RunError::TimedOut) => {
            tracing::warn!(track = track.id, "transcode timed out");
            return Err(Refusal::TranscodeTimedOut);
        }
        Err(transcode::RunError::Failed) => {
            tracing::warn!(track = track.id, "transcode failed");
            return Err(Refusal::TranscodeFailed);
        }
    }
    drop(input);
    drop(room);
    let size = std::fs::metadata(&output.0)
        .map(|m| m.len())
        .map_err(|_| Refusal::TranscodeFailed)?;
    // Empty, or cut short by the size limit: not a usable file.
    if size == 0 || size >= max {
        return Err(Refusal::TranscodeFailed);
    }
    let path = tc.cache.dir().join(&name);
    std::fs::rename(&output.0, &path).map_err(|_| Refusal::TranscodeFailed)?;
    let file = std::fs::File::open(&path).map_err(|_| Refusal::TranscodeFailed)?;
    tc.cache.insert(&name, size);
    Ok(file)
}

async fn transcoded(
    gw: &AppState,
    track: &Track,
    format: Format,
    range: Option<ByteRange>,
    watch: Watch,
) -> Result<Response, Refusal> {
    let salt = gw.download_salt().ok_or(Refusal::TranscodeFailed)?;
    let key = stable_key(&salt, &track.url);
    let file = transcode_file(gw, track, format, &key).await?;
    let mut file = tokio::fs::File::from_std(file);
    let total = file
        .metadata()
        .await
        .map_err(|_| Refusal::TranscodeFailed)?
        .len();
    let span = match range {
        Some(r) => Some(resolve(r, total).ok_or(Refusal::Range(Some(total)))?),
        None => None,
    };
    let (start, len) = match span {
        Some((a, b)) => (a, b - a + 1),
        None => (0, total),
    };
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| Refusal::TranscodeFailed)?;
    let chunks = futures_util::stream::unfold(file, |mut f| async move {
        let mut buf = vec![0u8; CHUNK];
        match f.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), f))
            }
            Err(e) => Some((Err(e), f)),
        }
    });
    let body = guarded(Box::pin(chunks), 0, len, watch);
    let disposition = content_disposition(
        track.tracknum,
        &track.artist,
        &track.title,
        track.id,
        format.ext(),
    );
    Ok(audio_response(
        body,
        format.content_type(),
        span.map(|(a, b)| (a, b, total)),
        len,
        &disposition,
    ))
}

/// Recent download audit entries, per (device, track).
pub(crate) type AuditSeen = HashMap<(i64, u64), Instant>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_ids_are_positive_integers_only() {
        assert_eq!(parse_track_id("1"), Some(1));
        assert_eq!(parse_track_id("123456789012"), Some(123_456_789_012));
        for bad in [
            "",
            "0",
            "-1",
            "+1",
            "01",
            "1e3",
            "1.0",
            " 1",
            "1 ",
            "../1",
            "..",
            "1/2",
            "0x10",
            "1234567890123",
            "99999999999999999999999999",
            "１",
            "1\0",
            "%31",
        ] {
            assert_eq!(parse_track_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_format_query_is_strict() {
        assert_eq!(parse_format_query(None), Some(Format::Original));
        assert_eq!(parse_format_query(Some("")), Some(Format::Original));
        assert_eq!(parse_format_query(Some("format=opus")), Some(Format::Opus));
        assert_eq!(parse_format_query(Some("format=mp3")), Some(Format::Mp3));
        for bad in [
            "format=",
            "format=flac",
            "format=opus&x=1",
            "x=1",
            "format=mp3&format=opus",
            "format=MP3",
            "format=%6Dp3",
        ] {
            assert_eq!(parse_format_query(Some(bad)), None, "{bad}");
        }
    }

    fn hv(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).unwrap()
    }

    #[test]
    fn ranges_parse_and_resolve() {
        assert_eq!(parse_range(None), Ok(None));
        assert_eq!(
            parse_range(Some(&hv("bytes=0-99"))),
            Ok(Some(ByteRange::FromTo(0, 99)))
        );
        assert_eq!(
            parse_range(Some(&hv("bytes=100-"))),
            Ok(Some(ByteRange::From(100)))
        );
        assert_eq!(
            parse_range(Some(&hv("bytes=-10"))),
            Ok(Some(ByteRange::Suffix(10)))
        );
        assert_eq!(
            parse_range(Some(&hv("Bytes=1-2"))),
            Ok(Some(ByteRange::FromTo(1, 2)))
        );
        for bad in [
            "bytes=",
            "bytes=-",
            "bytes=5-1",
            "bytes=0-1,5-6",
            "bytes=a-b",
            "items=0-1",
            "bytes=-0",
            "bytes=1-2-3",
            "bytes=99999999999999999999-",
            "bytes= - ",
            "0-1",
            "bytes=+1-2",
            "bytes=0x1-2",
        ] {
            assert_eq!(parse_range(Some(&hv(bad))), Err(BadRange), "{bad}");
        }
        assert_eq!(resolve(ByteRange::FromTo(0, 99), 50), Some((0, 49)));
        assert_eq!(resolve(ByteRange::From(10), 50), Some((10, 49)));
        assert_eq!(resolve(ByteRange::From(50), 50), None);
        assert_eq!(resolve(ByteRange::Suffix(10), 50), Some((40, 49)));
        assert_eq!(resolve(ByteRange::Suffix(100), 50), Some((0, 49)));
        assert_eq!(resolve(ByteRange::From(0), 0), None);
    }

    #[test]
    fn range_parsing_never_panics_on_arbitrary_input() {
        let mut seed = 0x2545f4914f6cdd1du64;
        let alphabet = b"bytes=-0123456789, \t;xX";
        for _ in 0..5000 {
            let mut s = String::from("bytes=");
            for _ in 0..(seed % 40) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize] as char);
            }
            if let Ok(v) = HeaderValue::from_str(&s)
                && let Ok(Some(r)) = parse_range(Some(&v))
            {
                for total in [0, 1, 7, u64::MAX] {
                    if let Some((a, b)) = resolve(r, total) {
                        assert!(a <= b && b < total, "{s} {total}");
                    }
                }
            }
        }
        // Non-UTF-8 bytes too.
        let v = HeaderValue::from_bytes(b"bytes=\xff-\xfe").unwrap();
        assert_eq!(parse_range(Some(&v)), Err(BadRange));
    }

    #[test]
    fn content_ranges_from_lms_are_checked() {
        assert_eq!(
            parse_content_range(Some(&hv("bytes 0-9/100"))),
            Some((0, 9, 100))
        );
        for bad in [
            "bytes 9-0/100",
            "bytes 0-100/100",
            "bytes */100",
            "0-9/100",
            "bytes 0-9/x",
        ] {
            assert_eq!(parse_content_range(Some(&hv(bad))), None, "{bad}");
        }
    }

    #[test]
    fn file_names_are_safe() {
        let cd = content_disposition(Some(3), "Artist", "Title", 9, "flac");
        assert_eq!(cd, "attachment; filename=\"03 Artist - Title.flac\"");
        let hostile = [
            "a\"b",
            "x\r\nSet-Cookie: y=1",
            "../../etc/passwd",
            "..\\..\\windows",
            "C:\\x",
            "\u{202e}gpj.exe",
            "\0nul",
            "/abs/path",
            "....",
            "Björk 🎵",
            "",
        ];
        for t in hostile {
            let cd = content_disposition(Some(1), t, t, 42, "mp3");
            let ascii = cd
                .strip_prefix("attachment; filename=\"")
                .and_then(|r| r.split_once('"'))
                .map(|(n, _)| n)
                .unwrap();
            assert!(ascii.is_ascii(), "{t:?}: {cd}");
            assert!(
                !ascii.contains(['"', '\\', '/', '\r', '\n', '\0', ':']),
                "{t:?}: {cd}"
            );
            assert!(!ascii.starts_with('.'), "{t:?}: {cd}");
            assert!(ascii.ends_with(".mp3"));
            assert!(!cd.contains(['\r', '\n', '\0']), "{t:?}");
            assert!(HeaderValue::from_str(&cd).is_ok(), "{t:?}");
            assert!(cd.is_ascii(), "{t:?}: {cd}");
            // filename* is percent-encoded: no separators or quotes survive raw.
            if let Some((_, star)) = cd.split_once("filename*=UTF-8''") {
                assert!(!star.contains(['"', '\\', '/', ' ', ';']), "{t:?}: {cd}");
                assert!(
                    !star.contains("%2F") && !star.contains("%5C"),
                    "{t:?}: {cd}"
                );
                assert!(!star.contains("%E2%80%AE"), "no bidi override: {cd}");
            }
        }
        let cd = content_disposition(None, "", "", 42, "mp3");
        assert_eq!(cd, "attachment; filename=\"track 42.mp3\"");
        let cd = content_disposition(None, "Björk", "Jóga", 1, "flac");
        assert!(
            cd.contains("filename*=UTF-8''Bj%C3%B6rk%20-%20J%C3%B3ga.flac"),
            "{cd}"
        );
        let long = "x".repeat(5000);
        assert!(content_disposition(None, &long, &long, 1, "mp3").len() < 400);
    }

    #[test]
    fn original_kinds_come_from_a_table() {
        assert_eq!(
            original_kind("flc", "file:///srv/music/a.flac"),
            ("flc".into(), "audio/flac", "flac".into())
        );
        assert_eq!(original_kind("xyz", "file:///srv/music/a.DSF").2, "dsf");
        assert_eq!(original_kind("xyz", "file:///srv/music/a.b/c").2, "bin");
        assert_eq!(original_kind("xyz", "file:///srv/music/a.\"x").2, "bin");
        assert_eq!(original_kind("<b>", "file:///srv/a").0, "unknown");
    }

    #[test]
    fn keys_hide_the_path_and_depend_on_the_salt() {
        let url = "file:///srv/music/Artist/Album/01%20Track.flac";
        let k = stable_key("salt-a", url);
        assert_eq!(k.len(), 64);
        assert!(k.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(k, stable_key("salt-a", url));
        assert_ne!(k, stable_key("salt-b", url));
        assert_ne!(
            k,
            hex::encode(Sha256::digest(url.as_bytes())),
            "not a bare hash of the URL"
        );
        assert_ne!(k, stable_key("salt-a", "file:///srv/music/other.flac"));
    }
}
