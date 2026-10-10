//! `GET /v1/art/<path>`: cover art and icons from LMS, within fixed path patterns.
//!
//! - **Covers:** `music/<id>/cover…` (numeric or hex track id, optional `_WxH_mode`
//!   and extension), and `music/current/cover…?player=<mac>` for players the caller may
//!   control.
//! - **Image proxy:** `imageproxy/<url>/image…`. LMS fetches `<url>` itself, so it is only
//!   relayed for URLs LMS handed out in an earlier response. Otherwise a user could make LMS
//!   fetch anything on the home network (SSRF).
//! - **Icons:** `plugins/<Name>/html/…`, `plugins/cache/icons/…`, `html/images/…` image files.
//!
//! Only `image/*` responses are relayed, at most [`MAX_IMAGE`] bytes, with a sandboxing CSP
//! so an SVG can't run script on the gateway's origin. Browsers fetch art with their bearer
//! token (no credentials in URLs) and show it via blob URLs.

use std::collections::{HashSet, VecDeque};

use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::Uri;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::AppState;
use crate::rpc_policy::is_player_id;

pub const MAX_IMAGE: usize = 20 * 1024 * 1024;
/// Image-proxy URLs remembered from responses.
const KNOWN_PROXY_CAP: usize = 20_000;

fn image_name(name: &str, stem: &str) -> bool {
    let rest = match name.strip_prefix(stem) {
        Some(r) => r,
        None => return false,
    };
    let (rest, ext) = match rest.rsplit_once('.') {
        Some((r, e)) => (r, Some(e)),
        None => (rest, None),
    };
    if let Some(e) = ext
        && !matches!(e, "jpg" | "jpeg" | "png" | "gif" | "webp")
    {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    // _WxH or _WxH_m (resize mode letter)
    let Some(spec) = rest.strip_prefix('_') else {
        return false;
    };
    let mut parts = spec.split('_');
    let size = parts.next().unwrap_or("");
    let mode = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let Some((w, h)) = size.split_once('x') else {
        return false;
    };
    let dim = |d: &str| !d.is_empty() && d.len() <= 4 && d.bytes().all(|b| b.is_ascii_digit());
    dim(w)
        && dim(h)
        && mode.is_none_or(|m| m.len() == 1 && m.bytes().all(|b| b.is_ascii_alphabetic()))
}

fn safe_file(path: &str, exts: &[&str]) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.contains("..")
        && !path.contains("//")
        && !path.starts_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
        && path
            .rsplit_once('.')
            .is_some_and(|(_, e)| exts.contains(&e))
}

/// What to fetch from LMS for a requested art path, if it's allowed.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// A fixed LMS path.
    Path(String),
    /// The current track's cover for a player (needs permission).
    Current { player: String, name: String },
    /// An image-proxy request (needs the URL to have been handed out by LMS).
    Proxy {
        url: String,
        name: String,
        encoded: String,
    },
}

pub fn target(path: &str) -> Option<Target> {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["music", "current", name] if image_name(name, "cover") => Some(Target::Current {
            player: String::new(),
            name: name.to_string(),
        }),
        ["music", id, name]
            if !id.is_empty()
                && id.len() <= 16
                && id.bytes().all(|b| b.is_ascii_hexdigit())
                && (image_name(name, "cover") || image_name(name, "thumb")) =>
        {
            Some(Target::Path(format!("/music/{id}/{name}")))
        }
        ["imageproxy", encoded, name] if image_name(name, "image") => {
            let url = percent_decode(encoded)?;
            (url.starts_with("http://") || url.starts_with("https://")).then(|| Target::Proxy {
                url,
                name: name.to_string(),
                encoded: encoded.to_string(),
            })
        }
        ["plugins", "cache", "icons", rest @ ..]
            if safe_file(&rest.join("/"), &["png", "jpg", "jpeg", "gif", "svg"]) =>
        {
            Some(Target::Path(format!("/{path}")))
        }
        ["plugins", name, "html", rest @ ..]
            if !name.is_empty()
                && name.len() <= 64
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && safe_file(&rest.join("/"), &["png", "jpg", "jpeg", "gif", "svg"]) =>
        {
            Some(Target::Path(format!("/{path}")))
        }
        ["html", "images", rest @ ..]
            if safe_file(&rest.join("/"), &["png", "jpg", "gif", "svg"]) =>
        {
            Some(Target::Path(format!("/{path}")))
        }
        _ => None,
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    (!s.chars().any(char::is_control) && s.len() <= 2048).then_some(s)
}

/// Bounded set of image URLs LMS has handed out.
#[derive(Default)]
pub struct KnownImages {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl KnownImages {
    pub fn insert(&mut self, url: String) {
        if self.set.insert(url.clone()) {
            self.order.push_back(url);
            while self.order.len() > KNOWN_PROXY_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
    pub fn contains(&self, url: &str) -> bool {
        self.set.contains(url)
    }
}

/// Response fields that hold artwork or icons (LMS and SlimBrowse names).
pub const IMAGE_KEYS: &[&str] = &[
    "image",
    "icon",
    "icon-id",
    "artwork_url",
    "coverart",
    "cover",
];

/// The URL in an `…imageproxy/<url-encoded URL>/…` string, if it is http(s).
pub(crate) fn proxy_target(s: &str) -> Option<String> {
    let i = s.find("imageproxy/")?;
    let url = s[i + "imageproxy/".len()..]
        .split('/')
        .next()
        .and_then(percent_decode)?;
    (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
}

/// A URL the image proxy must never fetch: one on the home network or the server itself.
fn local_url(url: &str) -> bool {
    crate::rpc_policy::sensitive_text(url)
}

/// Collect image URLs from an (already filtered) LMS response: absolute http(s) URLs and
/// `imageproxy/…` targets, only in image fields. Any other text (track titles, player
/// names) is user-controlled and never teaches the proxy a URL, and local URLs are never
/// learned at all.
pub fn learn_images(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                if let Value::String(s) = x {
                    if !IMAGE_KEYS.contains(&k.as_str()) {
                        continue;
                    }
                    let url = if s.starts_with("http://") || s.starts_with("https://") {
                        Some(s.clone())
                    } else {
                        proxy_target(s)
                    };
                    if let Some(url) = url.filter(|u| !local_url(u)) {
                        out.push(url);
                    }
                } else {
                    learn_images(x, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|x| learn_images(x, out)),
        _ => {}
    }
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

pub async fn art(
    State(gw): State<AppState>,
    uri: Uri,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    // Validate the raw path: a decoded one would turn an image-proxy URL's `%2F` into path
    // separators, and would hide other encodings from the checks below.
    let Some(path) = uri.path().strip_prefix("/v1/art/").map(str::to_string) else {
        return status(StatusCode::NOT_FOUND);
    };
    let principal = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| gw.accounts().authenticate(t));
    let Some(principal) = principal else {
        return status(StatusCode::UNAUTHORIZED);
    };
    let lms_path = match target(&path) {
        Some(Target::Path(p)) => p,
        Some(Target::Current { name, .. }) => {
            let player = query
                .as_deref()
                .and_then(|q| q.strip_prefix("player="))
                .map(|p| {
                    p.replace("%3A", ":")
                        .replace("%3a", ":")
                        .to_ascii_lowercase()
                })
                .unwrap_or_default();
            let allowed = is_player_id(&player)
                && gw
                    .accounts()
                    .player_scope(&principal)
                    .is_ok_and(|s| s.allows(&player));
            if !allowed {
                return status(StatusCode::NOT_FOUND);
            }
            format!("/music/current/{name}?player={player}")
        }
        Some(Target::Proxy { url, name, encoded }) => {
            if local_url(&url) || !gw.known_images.lock().unwrap().contains(&url) {
                tracing::warn!(user = %principal.username, "refused image proxy for an unknown URL");
                return status(StatusCode::NOT_FOUND);
            }
            format!("/imageproxy/{encoded}/{name}")
        }
        None => return status(StatusCode::NOT_FOUND),
    };
    if !matches!(target(&path), Some(Target::Current { .. })) && query.is_some() {
        return status(StatusCode::NOT_FOUND);
    }

    let resp = match gw
        .http
        .get(format!("http://{}{}", gw.cfg.lms_http, lms_path))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => {
            return status(StatusCode::NOT_FOUND);
        }
        _ => return status(StatusCode::BAD_GATEWAY),
    };
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let Some(ct) = [
        "image/jpeg",
        "image/png",
        "image/gif",
        "image/webp",
        "image/svg+xml",
    ]
    .into_iter()
    .find(|t| ct.starts_with(t)) else {
        return status(StatusCode::BAD_GATEWAY);
    };
    if resp
        .content_length()
        .is_some_and(|n| n as usize > MAX_IMAGE)
    {
        return status(StatusCode::BAD_GATEWAY);
    }
    let Ok(bytes) = resp.bytes().await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    if bytes.len() > MAX_IMAGE {
        return status(StatusCode::BAD_GATEWAY);
    }
    let mut r = Response::new(Body::from(bytes));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    // An image is inert data: no scripts, no subresources, even if it's an SVG.
    h.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
    );
    let cache = if lms_path.starts_with("/music/current/") {
        "no-store"
    } else {
        "private, max-age=86400"
    };
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_of(p: &str) -> Option<String> {
        match target(p) {
            Some(Target::Path(x)) => Some(x),
            _ => None,
        }
    }

    #[test]
    fn covers_and_icons_are_allowed() {
        for p in [
            "music/2109213/cover.jpg",
            "music/2109213/cover",
            "music/2109213/cover_150x150_o",
            "music/544ab3d5/cover_300x300_o.jpg",
            "music/12/thumb.png",
            "plugins/Spotty/html/images/93aac68fb06348598c1e67734dfaceee.png",
            "plugins/LastMix/html/icon.png",
            "plugins/BBCSounds/html/images/BBCSoundsIcon_svg.png",
            "plugins/cache/icons/radiolocal.png",
            "html/images/favorites.png",
        ] {
            assert_eq!(path_of(p), Some(format!("/{p}")), "{p}");
        }
        assert!(matches!(
            target("music/current/cover.jpg"),
            Some(Target::Current { .. })
        ));
    }

    #[test]
    fn everything_else_is_refused() {
        for p in [
            "",
            "music",
            "music/../jsonrpc.js",
            "music/12/../../jsonrpc.js",
            "music/12/cover.php",
            "music/12/cover_x.jpg",
            "music/12/cover_99999x1.jpg",
            "music/12/cover_1x1_ab.jpg",
            "music/12/download",
            "music/12/cover.jpg/x",
            "music/xyz!/cover.jpg",
            "music/12345678901234567/cover.jpg",
            "plugins/Spotty/html/../../prefs/server.prefs",
            "plugins/Spotty/settings/basic.html",
            "plugins/Spotty/html/x.html",
            "plugins/Spotty/html/images/a.png?x",
            "plugins/Sp..otty/html/a.png",
            "plugins/a b/html/a.png",
            "plugins/X/html//a.png",
            "html/images/../../server.log",
            "plugins/Spotty/html/../../../../etc/x.png",
            "html/images/../x.png",
            "plugins/cache/icons/../../x.png",
            "html/docs/index.html",
            "jsonrpc.js",
            "stream.mp3",
            "settings/index.html",
            "imageproxy/file%3A%2F%2F%2Fetc%2Fpasswd/image",
            "imageproxy/ftp%3A%2F%2Fx/image",
            "imageproxy/https%3A%2F%2Fx/not-image",
            "imageproxy/%ZZ/image",
            "imageproxy/https%3A%2F%2Fx%0A/image",
        ] {
            assert!(target(p).is_none(), "{p} allowed");
        }
    }

    #[test]
    fn image_proxy_targets_are_decoded() {
        let t = target("imageproxy/https%3A%2F%2Fcdn.example.com%2Fa.jpg/image_300x300_o").unwrap();
        assert_eq!(
            t,
            Target::Proxy {
                url: "https://cdn.example.com/a.jpg".into(),
                name: "image_300x300_o".into(),
                encoded: "https%3A%2F%2Fcdn.example.com%2Fa.jpg".into(),
            }
        );
    }

    #[test]
    fn image_urls_are_learned_from_responses() {
        let v = serde_json::json!({
            "loop_loop": [
                {"name": "A", "image": "https://cdn.example.com/a.jpg"},
                {"name": "B", "icon": "/imageproxy/https%3A%2F%2Fcdn.example.com%2Fb.png/image.png"},
                {"name": "C", "image": "html/images/favorites.png"},
                {"name": "D", "url": "http://not-an-image-field.example/x"}
            ]
        });
        let mut out = vec![];
        learn_images(&v, &mut out);
        out.sort();
        assert_eq!(
            out,
            vec![
                "https://cdn.example.com/a.jpg",
                "https://cdn.example.com/b.png"
            ]
        );
    }

    /// Regression: any `imageproxy/<url>` anywhere in a response (a track title, a player
    /// name) taught the proxy that URL, so a user could make LMS fetch from the home network.
    #[test]
    fn images_are_learned_only_from_image_fields_and_never_local() {
        let evil = "imageproxy/http%3A%2F%2F10.0.0.1%2Fx/image.png";
        let v = serde_json::json!({
            "players_loop": [{"name": format!("Kitchen {evil}")}],
            "titles_loop": [{"title": format!("/{evil}"), "album": evil}],
            "loop_loop": [
                {"icon": format!("/{evil}")},
                {"image": "http://10.0.0.1/admin.png"},
                {"image": "http://nas.example.lan/a.jpg"},
                {"artwork_url": "https://cdn.example.com/ok.jpg"}
            ]
        });
        let mut out = vec![];
        learn_images(&v, &mut out);
        assert_eq!(out, vec!["https://cdn.example.com/ok.jpg"]);
    }

    #[test]
    fn known_images_are_bounded() {
        let mut k = KnownImages::default();
        for i in 0..KNOWN_PROXY_CAP + 10 {
            k.insert(format!("https://x/{i}"));
        }
        assert!(!k.contains("https://x/0"));
        assert!(k.contains(&format!("https://x/{}", KNOWN_PROXY_CAP + 9)));
        assert!(k.set.len() <= KNOWN_PROXY_CAP);
    }
}
