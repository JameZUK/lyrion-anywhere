//! Revalidation of the web app's static files.
//!
//! The gateway serves the built web app with `tower_http::services::ServeDir`. Files under
//! `/_app/immutable/` carry their content hash in their name and are cached for a year
//! ([`crate::security_headers`]); every other file keeps its name across releases (the page,
//! the service worker, the player worker, the worklet, the WebAssembly) and is served with
//! `Cache-Control: no-cache`, so browsers revalidate it on every use.
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
//! The content-hashed files are left to `ServeDir` untouched; so is anything that is not a
//! plain whole-file GET or HEAD (range requests, errors, other methods).
use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, Response, StatusCode, header},
    middleware::Next,
    response::IntoResponse,
};
use sha2::{Digest, Sha256};

/// Files larger than this are served as `ServeDir` serves them (its validators, a streamed
/// body); the web app's largest revalidated file, the WebAssembly, is under 2 MiB.
pub const MAX_HASHED: usize = 16 * 1024 * 1024;

/// Whether a path is a content-hashed app file, which `ServeDir`'s validators suit.
pub fn immutable(path: &str) -> bool {
    path.starts_with("/_app/immutable/")
}

/// The entity tag for a file's content: a strong validator.
pub fn content_tag(body: &[u8]) -> HeaderValue {
    let digest = Sha256::digest(body);
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

/// Middleware around `ServeDir`: a content-hash `ETag` and `If-None-Match` for the
/// revalidated files (see the module documentation).
pub async fn content_etags(mut req: Request, next: Next) -> Response<Body> {
    let path = req.uri().path().to_owned();
    let head = *req.method() == Method::HEAD;
    if !(head || *req.method() == Method::GET)
        || immutable(&path)
        || req.headers().contains_key(header::RANGE)
    {
        return next.run(req).await;
    }
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
    let whole = resp.status() == StatusCode::OK
        && resp
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|n| n <= MAX_HASHED);
    if !whole {
        return if head { without_body(resp) } else { resp };
    }
    let (mut parts, body) = resp.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_HASHED).await {
        Ok(b) => b,
        // The file changed under ServeDir, or the disk failed: an honest error beats a torso.
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let tag = content_tag(&bytes);
    if none_match(&client, &tag) {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NOT_MODIFIED;
        r.headers_mut().insert(header::ETAG, tag);
        return r;
    }
    parts.headers.insert(header::ETAG, tag);
    let body = if head {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    Response::from_parts(parts, body)
}

/// The response for a HEAD request: the headers of the GET, no body.
fn without_body(resp: Response<Body>) -> Response<Body> {
    let (parts, _) = resp.into_parts();
    Response::from_parts(parts, Body::empty())
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
}
