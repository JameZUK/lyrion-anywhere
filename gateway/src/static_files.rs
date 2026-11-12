//! Revalidation of the web app's static files.
//!
//! The gateway serves the built web app with `tower_http::services::ServeDir` ([`service`]).
//! Files under `/_app/immutable/` carry their content hash in their name and are cached for
//! a year ([`crate::security_headers`]); every other file keeps its name across releases
//! (the page, the service worker, the player worker, the worklet, the WebAssembly) and is
//! served with `Cache-Control: no-cache`, so browsers revalidate it on every use.
//!
//! `ServeDir`'s validators are the file's modification time and size. The release zip gives
//! every file the same date (2020-01-01, for reproducible builds), and a file's size often
//! survives a release (the page changes only in the hashes it references), so two releases'
//! copies of `index.html` could share an `ETag` and a `Last-Modified` while their content
//! differed. A browser revalidating the old page then got 304 and kept running it, with
//! scripts the server no longer had. This middleware wraps `ServeDir` for the revalidated
//! files: the `ETag` is a hash of the content, `If-None-Match` is answered against it, and
//! `If-Modified-Since` is ignored for them (a date cannot tell two releases apart).
//!
//! # Cost (review M4)
//!
//! Anyone may ask for these files, signed in or not, so answering must not cost a read and a
//! hash of the whole file each time (a loop of `HEAD /player/pkg/…wasm?<random>` did, on the
//! async workers). With the web app's folder known ([`service`]):
//! - the tag is kept per file, keyed by what changes whenever its content does: its device
//!   and inode, size, modification time and change time (`ctime`, which every write and
//!   every rename moves on and which nobody can set back). A deploy that keeps a file's
//!   inode, size and date (an unzip over the old copy) still changes its `ctime`;
//! - a file changed less than [`RACY_MARGIN`] before it was hashed is hashed again on each
//!   request until it is older than that, so a change within one tick of the clock that
//!   stamps `ctime` cannot leave a stale tag behind (git's "racily clean" rule);
//! - the file is looked at (`stat`) before `ServeDir` opens it and again after: only when
//!   both agree is the kept tag used for what `ServeDir` serves;
//! - hashing is done on tokio's blocking pool, [`HASHING`] files at a time, reading the file
//!   in pieces; the body is streamed by `ServeDir` as ever, never held whole, and a 304 or a
//!   HEAD reads nothing at all.
//!
//! Anything that can't be tied to a file that way (a request that changed the file under
//! it, a platform without inode numbers, the stateless [`content_etags`]) falls back to
//! hashing the body `ServeDir` produced, off the async workers, as before.
//!
//! The content-hashed files are left to `ServeDir` untouched; so is anything that is not a
//! plain whole-file GET or HEAD (range requests, errors, other methods).
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, Response, StatusCode, header},
    middleware::Next,
    response::IntoResponse,
};
use sha2::{Digest, Sha256};

/// Files larger than this are served as `ServeDir` serves them (its validators, a streamed
/// body); the web app's largest revalidated file, the WebAssembly, is under 2 MiB.
pub const MAX_HASHED: usize = 16 * 1024 * 1024;

/// A file changed this recently before it was hashed has its tag computed again on the next
/// request (see the module documentation).
pub const RACY_MARGIN: Duration = Duration::from_secs(2);

/// Files hashed at once, across the process.
const HASHING: usize = 2;

/// Tags kept; beyond this the table is emptied (the web app has about a hundred files).
const MAX_KEPT: usize = 4096;

/// Read size while hashing.
const PIECE: usize = 64 * 1024;

/// Whether a path is a content-hashed app file, which `ServeDir`'s validators suit.
pub fn immutable(path: &str) -> bool {
    path.starts_with("/_app/immutable/")
}

/// The entity tag for a file's content: a strong validator.
pub fn content_tag(body: &[u8]) -> HeaderValue {
    tag_of(Sha256::digest(body).as_slice())
}

fn tag_of(digest: &[u8]) -> HeaderValue {
    HeaderValue::from_str(&format!("\"{}\"", hex::encode(&digest[..16])))
        .expect("hex digits and quotes are a valid header value")
}

/// Whether `If-None-Match` lists this tag (weak comparison, as RFC 9110 §13.1.2 requires) or
/// is `*`. Tolerates any bytes in the header.
pub fn none_match(headers: &HeaderMap, tag: &HeaderValue) -> bool {
    let tag = tag.to_str().unwrap_or_default();
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == tag || t == "*")
}

/// The web app in `dir`: `ServeDir`, with content-hash tags for the revalidated files, kept
/// per file (see the module documentation).
pub fn service(
    dir: PathBuf,
) -> tower::util::BoxCloneSyncService<Request, Response<Body>, Infallible> {
    let root = Arc::new(dir.clone());
    tower::util::BoxCloneSyncService::new(
        tower::ServiceBuilder::new()
            .layer(axum::middleware::from_fn_with_state(root, revalidate))
            .service(tower_http::services::ServeDir::new(dir)),
    )
}

/// Middleware around `ServeDir` when its folder is not known: content-hash tags for the
/// revalidated files, the body hashed each time (off the async workers). [`service`] is
/// the same with tags kept per file; use that.
pub async fn content_etags(req: Request, next: Next) -> Response<Body> {
    revalidate_in(None, req, next).await
}

async fn revalidate(State(root): State<Arc<PathBuf>>, req: Request, next: Next) -> Response<Body> {
    revalidate_in(Some(root), req, next).await
}

async fn revalidate_in(root: Option<Arc<PathBuf>>, mut req: Request, next: Next) -> Response<Body> {
    let head = *req.method() == Method::HEAD;
    if !(head || *req.method() == Method::GET)
        || immutable(req.uri().path())
        || req.headers().contains_key(header::RANGE)
    {
        return next.run(req).await;
    }
    // The file `ServeDir` is about to serve, and what it is now.
    let before = match &root {
        Some(root) => {
            let (root, uri_path) = (root.clone(), req.uri().path().to_owned());
            tokio::task::spawn_blocking(move || {
                let path = resolve(&root, &uri_path)?;
                let id = FileId::of(&std::fs::metadata(&path).ok()?)?;
                Some((path, id))
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };
    // The client's validators are for this layer; ServeDir would compare them with its own
    // (and answer a stale 304 to If-Modified-Since). It is asked for the whole file; HEAD is
    // answered from the GET so the tag can be computed.
    let client = std::mem::take(req.headers_mut());
    let mut forwarded = client.clone();
    forwarded.remove(header::IF_NONE_MATCH);
    forwarded.remove(header::IF_MODIFIED_SINCE);
    *req.headers_mut() = forwarded;
    if head {
        *req.method_mut() = Method::GET;
    }
    let resp = next.run(req).await;
    let length = resp
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let whole = resp.status() == StatusCode::OK && length.is_some_and(|n| n <= MAX_HASHED as u64);
    if !whole {
        return if head { without_body(resp) } else { resp };
    }
    let kept = match (before, length) {
        (Some((path, id)), Some(n)) if id.size == n => kept_tag(path, id).await,
        _ => None,
    };
    let (mut parts, body) = resp.into_parts();
    let (tag, body) = match kept {
        Some(tag) => (tag, body),
        None => {
            let bytes = match axum::body::to_bytes(body, MAX_HASHED).await {
                Ok(b) => b,
                // The file changed under ServeDir, or the disk failed: an honest error beats
                // a torso.
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            let Some(tag) = hashed(bytes.clone()).await else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            (tag, Body::from(bytes))
        }
    };
    if none_match(&client, &tag) {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NOT_MODIFIED;
        r.headers_mut().insert(header::ETAG, tag);
        return r;
    }
    parts.headers.insert(header::ETAG, tag);
    let body = if head { Body::empty() } else { body };
    Response::from_parts(parts, body)
}

/// The response for a HEAD request: the headers of the GET, no body.
fn without_body(resp: Response<Body>) -> Response<Body> {
    let (parts, _) = resp.into_parts();
    Response::from_parts(parts, Body::empty())
}

/// The file `ServeDir::new(root)` serves for `uri_path` with a 200, if any, worked out as it
/// does: the path percent-decoded (it must be UTF-8), `.` skipped, any `..`, root or prefix
/// component refused, and a directory's `index.html` when the path ends with a slash (without
/// one, `ServeDir` redirects). Does blocking I/O (a `stat`).
fn resolve(root: &Path, uri_path: &str) -> Option<PathBuf> {
    let decoded = percent_decoded(uri_path.trim_start_matches('/'))?;
    let mut path = root.to_path_buf();
    for c in Path::new(&decoded).components() {
        match c {
            Component::Normal(part)
                if Path::new(part)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))) =>
            {
                path.push(part)
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    let meta = std::fs::metadata(&path).ok()?;
    if meta.is_dir() {
        if !uri_path.ends_with('/') {
            return None;
        }
        path.push("index.html");
    } else if uri_path.ends_with('/') && uri_path != "/" {
        return None;
    }
    Some(path)
}

/// `%xx` escapes decoded (a malformed one kept as it is), as `percent_encoding` decodes;
/// `None` unless the result is UTF-8.
fn percent_decoded(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|c| hex(*c)),
                b.get(i + 2).and_then(|c| hex(*c)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// What a file's content can't change without: device, inode, size, modification and change
/// times (nanoseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl FileId {
    #[cfg(unix)]
    fn of(m: &std::fs::Metadata) -> Option<FileId> {
        use std::os::unix::fs::MetadataExt;
        m.is_file().then(|| FileId {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        })
    }

    /// Without inode numbers and change times a file can't be told apart from its next
    /// version: its body is hashed each time.
    #[cfg(not(unix))]
    fn of(_: &std::fs::Metadata) -> Option<FileId> {
        None
    }

    /// When the file last changed in any way.
    fn changed(&self) -> SystemTime {
        let (s, ns) = self.ctime;
        let since = Duration::new(s.max(0) as u64, ns.clamp(0, 999_999_999) as u32);
        SystemTime::UNIX_EPOCH + since
    }
}

/// A tag kept for a file.
struct Kept {
    id: FileId,
    tag: HeaderValue,
    /// The file had last changed at least [`RACY_MARGIN`] before it was hashed.
    settled: bool,
    /// Times this file's content has been hashed (tests).
    hashes: u64,
}

fn kept() -> &'static Mutex<HashMap<PathBuf, Kept>> {
    static KEPT: OnceLock<Mutex<HashMap<PathBuf, Kept>>> = OnceLock::new();
    KEPT.get_or_init(Default::default)
}

fn hashing() -> &'static tokio::sync::Semaphore {
    static SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(HASHING))
}

/// How many times this file's content has been hashed for its tag since the process started
/// (for tests).
#[doc(hidden)]
pub fn times_hashed(path: &Path) -> u64 {
    kept().lock().unwrap().get(path).map_or(0, |k| k.hashes)
}

/// The tag for `path`, which was `id` before `ServeDir` opened it: kept, or hashed now. `None`
/// if the file is not `id` any more (changed meanwhile), or could not be read.
async fn kept_tag(path: PathBuf, id: FileId) -> Option<HeaderValue> {
    let lookup = path.clone();
    let now = tokio::task::spawn_blocking(move || {
        let now = FileId::of(&std::fs::metadata(&lookup).ok()?)?;
        let hit = kept()
            .lock()
            .unwrap()
            .get(&lookup)
            .filter(|k| k.id == now && k.settled)
            .map(|k| k.tag.clone());
        Some((now, hit))
    })
    .await
    .ok()??;
    match now {
        (now, _) if now != id => None,
        (_, Some(tag)) => Some(tag),
        (_, None) => {
            let _slot = hashing().acquire().await.ok()?;
            tokio::task::spawn_blocking(move || hash_file(&path, id))
                .await
                .ok()?
        }
    }
}

/// Hash `path`, which must be `id` throughout, and keep its tag.
fn hash_file(path: &Path, id: FileId) -> Option<HeaderValue> {
    let started = SystemTime::now();
    let mut f = std::fs::File::open(path).ok()?;
    if FileId::of(&f.metadata().ok()?)? != id {
        return None;
    }
    let mut h = Sha256::new();
    let mut buf = vec![0u8; PIECE];
    let mut read = 0u64;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        read += n as u64;
        h.update(&buf[..n]);
    }
    if read != id.size || FileId::of(&f.metadata().ok()?)? != id {
        return None; // written to while it was read
    }
    let tag = tag_of(h.finalize().as_slice());
    let settled = id
        .changed()
        .checked_add(RACY_MARGIN)
        .is_some_and(|t| t <= started);
    let mut kept = kept().lock().unwrap();
    if kept.len() >= MAX_KEPT && !kept.contains_key(path) {
        kept.clear();
    }
    let hashes = kept.get(path).map_or(0, |k| k.hashes) + 1;
    kept.insert(
        path.to_path_buf(),
        Kept {
            id,
            tag: tag.clone(),
            settled,
            hashes,
        },
    );
    Some(tag)
}

/// The tag for a body already read, hashed off the async workers.
async fn hashed(bytes: bytes::Bytes) -> Option<HeaderValue> {
    let _slot = hashing().acquire().await.ok()?;
    tokio::task::spawn_blocking(move || content_tag(&bytes))
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_strong_quoted_and_content_defined() {
        let a = content_tag(b"<!doctype html>a");
        let b = content_tag(b"<!doctype html>b");
        assert_ne!(a, b, "same length, different content, different tag");
        assert_eq!(a, content_tag(b"<!doctype html>a"));
        let s = a.to_str().unwrap();
        assert!(
            s.starts_with('"') && s.ends_with('"') && s.len() == 34,
            "{s}"
        );
        assert!(!s.starts_with("W/"));
    }

    #[test]
    fn if_none_match_lists_and_weak_forms() {
        let tag = content_tag(b"x");
        let other = content_tag(b"y");
        let mut h = HeaderMap::new();
        assert!(!none_match(&h, &tag));
        h.insert(header::IF_NONE_MATCH, tag.clone());
        assert!(none_match(&h, &tag));
        assert!(!none_match(&h, &other));
        let list = format!("\"a\", W/{} , \"b\"", tag.to_str().unwrap());
        h.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&list).unwrap());
        assert!(none_match(&h, &tag));
        h.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
        assert!(none_match(&h, &tag));
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_bytes(b"\xff\xfe garbage \"\"").unwrap(),
        );
        assert!(!none_match(&h, &tag));
        h.append(header::IF_NONE_MATCH, tag.clone());
        assert!(none_match(&h, &tag), "any of several header lines");
    }

    #[test]
    fn only_the_hashed_directory_is_immutable() {
        assert!(immutable("/_app/immutable/start.abc.js"));
        assert!(!immutable("/"));
        assert!(!immutable("/index.html"));
        assert!(!immutable("/_app/version.json"));
        assert!(!immutable("/player/pkg/anywhere_wasm_bg.wasm"));
        assert!(!immutable("/x/_app/immutable/y.js"));
    }

    /// The file is the one `ServeDir` serves, worked out the same way: decoded, `..` and
    /// absolute parts refused, a directory's page only with a trailing slash.
    #[test]
    fn requests_resolve_to_the_file_servedir_serves() {
        let root = std::env::temp_dir().join(format!("anywhere-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("player/pkg")).unwrap();
        std::fs::write(root.join("index.html"), b"page").unwrap();
        std::fs::write(root.join("player/index.html"), b"sub").unwrap();
        std::fs::write(root.join("player/pkg/a b.wasm"), b"wasm").unwrap();
        let r = |p: &str| resolve(&root, p);
        assert_eq!(r("/"), Some(root.join("index.html")));
        assert_eq!(r("/index.html"), Some(root.join("index.html")));
        assert_eq!(r("/./index.html"), Some(root.join("index.html")));
        assert_eq!(r("/player/"), Some(root.join("player/index.html")));
        assert_eq!(
            r("/player/pkg/a%20b.wasm"),
            Some(root.join("player/pkg/a b.wasm"))
        );
        for refused in [
            "/player",
            "/index.html/",
            "/../index.html",
            "/%2e%2e/index.html",
            "/player/%2e%2e/%2e%2e/etc/passwd",
            "//etc/passwd",
            "/%2Fetc%2Fpasswd",
            "/missing.js",
            "/%ff.html",
            "/player/pkg/a%2",
        ] {
            assert_eq!(r(refused), None, "{refused}");
        }
        assert_eq!(percent_decoded("%41%zz%4"), Some("A%zz%4".into()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tag is kept only for a file that had settled when it was hashed, and only for as
    /// long as nothing about the file changes.
    #[test]
    fn tags_are_kept_only_for_settled_unchanged_files() {
        let dir = std::env::temp_dir().join(format!("anywhere-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("page.html");
        std::fs::write(&p, b"one").unwrap();
        let id = FileId::of(&std::fs::metadata(&p).unwrap()).unwrap();
        // Just written: hashed, but not settled, so not served from the table.
        assert_eq!(hash_file(&p, id), Some(content_tag(b"one")));
        assert!(!kept().lock().unwrap()[&p].settled);
        // A file that isn't `id` any more is not hashed under it.
        std::fs::write(&p, b"three").unwrap();
        assert_eq!(hash_file(&p, id), None);
        let id2 = FileId::of(&std::fs::metadata(&p).unwrap()).unwrap();
        assert_eq!(hash_file(&p, id2), Some(content_tag(b"three")));
        assert_eq!(times_hashed(&p), 2);
        // A file that last changed well before it was hashed has settled.
        let old = FileId {
            ctime: (id2.ctime.0 - 10, id2.ctime.1),
            ..id2
        };
        assert!(old.changed() + RACY_MARGIN <= SystemTime::now());
        assert!(id2.changed() + RACY_MARGIN > SystemTime::now());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
