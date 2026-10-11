//! Artwork (`GET /v1/art/<path>`), fetched with the bearer token (never in a URL) and
//! capped in size.
//!
//! Records carry an artwork *key*, which doubles as a stable cache key for the app's image
//! loader:
//! - `music/<id>/cover`: a library cover, sized on request (`cover_300x300`);
//! - `imageproxy/<encoded http(s) URL>/image`: a remote image LMS named, sized on request;
//! - `html/images/…`, `plugins/…`: LMS and plugin icons, as they are.
//!
//! Keys are validated again before every fetch, with the same rules as the web app
//! (`artUrl`, `imagePath`): the path must stay inside `/v1/art/`, so a hostile key can
//! never point the token at another endpoint.

use std::sync::Arc;

use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::error::ApiError;
use crate::session::Session;
use crate::transport;

/// The default size cap: a large cover is well under 2 MiB.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// LMS resizes artwork on its single thread: don't flood it.
const IN_FLIGHT: usize = 6;
const MAX_KEY: usize = 2048;

/// A cover by LMS cover id or track id.
pub(crate) fn cover_key(v: &Value) -> Option<String> {
    let s = match v {
        Value::Number(n) => n.as_u64().filter(|n| *n > 0)?.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let digits = (1..=15).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    let hex = (8..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());
    (digits || hex).then(|| format!("music/{s}/cover"))
}

/// An icon or image LMS named in a menu or a track (`image`, `icon`, `artwork_url`).
pub(crate) fn image_key(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    if s.len() > MAX_KEY {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        if s.chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        {
            return None;
        }
        return Some(format!(
            "imageproxy/{}/image",
            utf8_percent_encode(s, NON_ALPHANUMERIC)
        ));
    }
    let p = s.trim_start_matches('/');
    valid_path(p).then(|| p.to_string())
}

/// A well-formed percent-encoded segment that decodes to an http(s) URL.
fn proxy_segment(seg: &str) -> bool {
    let b = seg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = |j: usize| b.get(j).is_some_and(u8::is_ascii_hexdigit);
                if !(hex(i + 1) && hex(i + 2)) {
                    return false;
                }
                i += 3;
            }
            c if c.is_ascii_alphanumeric() || b"._~!$&'()*+,;=:@-".contains(&c) => i += 1,
            _ => return false,
        }
    }
    let Ok(url) = percent_decode_str(seg).decode_utf8() else {
        return false;
    };
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() > 8
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
}

/// `image`, `image_300x300`, `image_300x300_o`, optionally with an image extension.
fn proxy_name(name: &str) -> bool {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, Some(e)),
        None => (name, None),
    };
    if ext.is_some_and(|e| !matches!(e, "jpg" | "jpeg" | "png" | "gif" | "webp")) {
        return false;
    }
    let Some(rest) = stem.strip_prefix("image") else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(size) = rest.strip_prefix('_') else {
        return false;
    };
    let mut parts = size.splitn(3, ['x', '_']);
    let num = |s: Option<&str>| {
        s.is_some_and(|s| (1..=4).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
    };
    let (w, h) = (parts.next(), parts.next());
    let mode_ok = parts
        .next()
        .is_none_or(|m| m.len() == 1 && m.bytes().all(|b| b.is_ascii_lowercase()));
    size.contains('x') && num(w) && num(h) && mode_ok
}

/// Paths the gateway relays, checked as the web app checks them.
pub(crate) fn valid_path(p: &str) -> bool {
    if p.is_empty()
        || p.len() > MAX_KEY
        || p.starts_with('/')
        || p.contains('\\')
        || p.contains("//")
        || p.chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '?' || c == '#')
    {
        return false;
    }
    let segs: Vec<&str> = p.split('/').collect();
    if let ["imageproxy", seg, name] = segs.as_slice() {
        return proxy_segment(seg) && proxy_name(name);
    }
    if p.contains('%') || segs.iter().any(|s| *s == "." || *s == ".." || s.is_empty()) {
        return false;
    }
    if let ["music", id, name] = segs.as_slice() {
        let id_ok = cover_key(&Value::String((*id).to_string())).is_some() || *id == "current";
        let name_ok = {
            let (stem, ext) = match name.rsplit_once('.') {
                Some((s, e)) => (s, Some(e)),
                None => (*name, None),
            };
            let ext_ok = ext.is_none_or(|e| matches!(e, "jpg" | "jpeg" | "png" | "gif" | "webp"));
            let base = stem
                .strip_prefix("cover")
                .or_else(|| stem.strip_prefix("thumb"));
            ext_ok
                && base.is_some_and(|rest| rest.is_empty() || proxy_name(&format!("image{rest}")))
        };
        // `music/current/…` needs a player query, which keys never carry.
        return id_ok && *id != "current" && name_ok;
    }
    let icon_ext = p.rsplit_once('.').is_some_and(|(_, e)| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "svg"
        )
    });
    let safe_chars = p
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
    let prefix_ok = p.starts_with("html/images/")
        || p.starts_with("plugins/cache/icons/")
        || matches!(segs.as_slice(), ["plugins", name, "html", ..] if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    icon_ext && safe_chars && prefix_ok && segs.len() >= 3
}

/// The path to fetch for a key at a size (pixels, clamped to 50–1000).
pub fn art_path(key: &str, size_px: u32) -> Result<String, ApiError> {
    let px = size_px.clamp(50, 1000);
    let bad = || ApiError::invalid("bad artwork key");
    let p = if let Some(id) = key
        .strip_prefix("music/")
        .and_then(|k| k.strip_suffix("/cover"))
    {
        cover_key(&Value::String(id.to_string())).ok_or_else(bad)?;
        format!("music/{id}/cover_{px}x{px}")
    } else if key.starts_with("imageproxy/") && key.ends_with("/image") {
        format!("{key}_{px}x{px}")
    } else {
        key.to_string()
    };
    if valid_path(&p) { Ok(p) } else { Err(bad()) }
}

/// Image bytes and their type.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ArtImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// Artwork fetching for one session.
#[derive(uniffi::Object)]
pub struct Art {
    session: Arc<Session>,
    max_bytes: u64,
    in_flight: Semaphore,
}

#[uniffi::export(async_runtime = "tokio")]
impl Art {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Self::with_limit(session, DEFAULT_MAX_BYTES)
    }

    /// Fetch artwork by key (from a record) at a size in pixels. Fails with `Server` for
    /// anything that isn't an image or is over the size cap.
    pub async fn fetch(&self, key: String, size_px: u32) -> Result<ArtImage, ApiError> {
        let path = art_path(&key, size_px)?;
        let server = self.session.core().server().clone();
        let url = server
            .base()
            .join(&format!("v1/art/{path}"))
            .map_err(|_| ApiError::invalid("bad artwork key"))?;
        // Whatever the URL parser made of it must still be exactly this path under /v1/art/.
        if url.path() != format!("/v1/art/{path}")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::invalid("bad artwork key"));
        }
        let _slot = self
            .in_flight
            .acquire()
            .await
            .map_err(|_| ApiError::network("closed"))?;
        let core = self.session.core().clone();
        let limit = self.max_bytes;
        core.authed(|token| {
            let req = core.http().get(url.clone()).bearer_auth(&token);
            async move {
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    return Err(transport::error_from_response(resp).await);
                }
                let mime = resp
                    .headers()
                    .get(hyper::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| {
                        v.split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_ascii_lowercase()
                    })
                    .unwrap_or_default();
                if !mime.starts_with("image/") {
                    return Err(ApiError::bad_response("not an image"));
                }
                let bytes = resp.read_limited(limit as usize).await?;
                Ok(ArtImage {
                    bytes,
                    mime_type: mime,
                })
            }
        })
        .await
    }
}

impl Art {
    /// With a different size cap (tests).
    pub fn with_limit(session: Arc<Session>, max_bytes: u64) -> Arc<Self> {
        Arc::new(Art {
            session,
            max_bytes,
            in_flight: Semaphore::new(IN_FLIGHT),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_from_lms_values() {
        assert_eq!(cover_key(&json!(12)).as_deref(), Some("music/12/cover"));
        assert_eq!(
            cover_key(&json!("DeadBeef")).as_deref(),
            Some("music/DeadBeef/cover")
        );
        assert_eq!(cover_key(&json!(0)), None);
        assert_eq!(cover_key(&json!(-1)), None);
        assert_eq!(cover_key(&json!("12/../x")), None);
        assert_eq!(cover_key(&json!("abc")), None);
        assert_eq!(
            image_key(&json!("https://cdn.example.com/a b.jpg")),
            None,
            "whitespace in a URL"
        );
        let k = image_key(&json!("https://cdn.example.com/a.jpg")).unwrap();
        assert_eq!(
            k,
            "imageproxy/https%3A%2F%2Fcdn%2Eexample%2Ecom%2Fa%2Ejpg/image"
        );
        assert!(valid_path(&format!("{k}_300x300")));
        assert_eq!(
            image_key(&json!("/html/images/radio.png")).as_deref(),
            Some("html/images/radio.png")
        );
        assert_eq!(
            image_key(&json!("plugins/Spotty/html/images/icon.svg")).as_deref(),
            Some("plugins/Spotty/html/images/icon.svg")
        );
        assert_eq!(image_key(&json!("/srv/music/cover.jpg")), None);
        assert_eq!(image_key(&json!(5)), None);
    }

    #[test]
    fn sized_paths() {
        assert_eq!(
            art_path("music/12/cover", 300).unwrap(),
            "music/12/cover_300x300"
        );
        assert_eq!(
            art_path("music/12/cover", 5).unwrap(),
            "music/12/cover_50x50"
        );
        assert_eq!(
            art_path("music/12/cover", 99_999).unwrap(),
            "music/12/cover_1000x1000"
        );
        assert_eq!(
            art_path("html/images/radio.png", 300).unwrap(),
            "html/images/radio.png"
        );
    }

    #[test]
    fn hostile_keys_are_refused() {
        for k in [
            "",
            "/v1/rpc",
            "../rpc",
            "music/../../v1/rpc",
            "music/12/../../rpc/cover",
            "music/12/cover/../../../v1/me",
            "music/%2e%2e/cover",
            "music/current/cover",
            "music/12/cover?player=aa",
            "music/12/cover#x",
            "music/12//cover",
            "music\\12\\cover",
            "html/images/../../v1/me.png",
            "html/images/x.html",
            "html/images/%2e%2e/x.png",
            "plugins/../html/x.png",
            "plugins/a b/html/x.png",
            "imageproxy/file%3A%2F%2F%2Fetc%2Fpasswd/image",
            "imageproxy/https%3A%2F%2Fx.example/image/../../v1/me",
            "imageproxy/https%3A%2F%2Fx.example/evil",
            "imageproxy/https%3A%2F%2Fx.example%2/image",
            "imageproxy/https%3A%2F%2Fx%20y.example/image",
            "imageproxy/https%3A%2F%2Fx.example/image_300x300_o.exe",
            "srv/music/a.jpg",
            "music/12/cover.html",
            "music/12/cover_1x",
        ] {
            assert!(art_path(k, 300).is_err(), "{k:?} was accepted");
        }
    }

    #[test]
    fn arbitrary_keys_never_panic() {
        let alphabet: Vec<char> = "music/cover_x0123%2eE./\\?#imageproxyhtps:é "
            .chars()
            .collect();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            let len = (seed % 48) as usize;
            let mut s = String::new();
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            if let Ok(p) = art_path(&s, 300) {
                assert!(
                    !p.contains("..")
                        && !p.contains('?')
                        && !p.contains('#')
                        && !p.starts_with('/'),
                    "{p}"
                );
            }
            let _ = image_key(&Value::String(s));
        }
    }
}
