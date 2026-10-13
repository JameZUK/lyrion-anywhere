//! Artist biographies and photos, and album reviews (docs/plans/ux-research.md X5 part 1):
//! the gateway's answers to `GET /v1/info/artist/{id}` and `GET /v1/info/album/{id}`
//! (cleaned there), checked again here as untrusted input before the app shows them. The
//! requests are [`crate::library::MusicLibrary::artist_info`] and
//! [`crate::library::MusicLibrary::album_info`].
//!
//! - Text is plain text: control characters other than line breaks are removed, blank
//!   lines collapsed, at most [`MAX_TEXT`] characters. Apps show it as text, never markup.
//! - The photo is an artwork key (`imageproxy/<encoded URL>/image`) for [`crate::art::Art`],
//!   only if the gateway's path is one of its own image-proxy paths. Anything else is no
//!   photo.

use serde_json::Value;

/// Longest text accepted, in characters (the gateway keeps to the same limit).
pub const MAX_TEXT: usize = 20_000;
/// Longest photo credit accepted, in characters.
pub const MAX_CREDITS: usize = 120;

/// An artist's biography and photo; every part may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct ArtistInfo {
    /// Plain text; paragraphs are separated by a blank line.
    pub biography: Option<String>,
    /// An artwork key for [`crate::art::Art::fetch`] (sized there).
    pub photo: Option<String>,
    /// Where the photo comes from (for example "Discogs"), only with a photo.
    pub credits: Option<String>,
}

/// An album's review; it may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct AlbumInfo {
    /// Plain text; paragraphs are separated by a blank line.
    pub review: Option<String>,
}

/// Untrusted text made safe to show: control and invisible formatting characters removed
/// (line breaks kept, at most one blank line in a row), trimmed, capped. `None` if empty.
pub(crate) fn plain_text(v: Option<&Value>, max: usize) -> Option<String> {
    let s = v?.as_str()?;
    let mut out = String::new();
    let mut count = 0;
    let mut newlines = 0;
    for c in s.chars() {
        if c == '\n' {
            newlines += 1;
            continue;
        }
        if c.is_control()
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
        {
            continue;
        }
        if newlines > 0 && !out.is_empty() {
            out.push_str(if newlines > 1 { "\n\n" } else { "\n" });
            count += newlines.min(2);
        }
        newlines = 0;
        out.push(c);
        count += 1;
        if count >= max {
            break;
        }
    }
    // Spaces around line breaks.
    let mut lines: Vec<&str> = Vec::new();
    for l in out.split('\n').map(str::trim) {
        if !(l.is_empty() && lines.last().is_none_or(|p| p.is_empty())) {
            lines.push(l);
        }
    }
    let out = lines.join("\n").trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// The artwork key for the gateway's photo path, if it is an image-proxy path.
fn photo_key(v: Option<&Value>) -> Option<String> {
    let path = v?.as_str()?;
    let key = path.strip_prefix("/v1/art/")?;
    let ok = key.starts_with("imageproxy/")
        && key.ends_with("/image")
        && key.matches('/').count() == 2
        && crate::art::valid_path(key);
    ok.then(|| key.to_string())
}

/// The gateway's artist answer, checked.
pub fn parse_artist_info(v: &Value) -> ArtistInfo {
    let photo = photo_key(v.get("photo"));
    let credits = photo
        .as_ref()
        .and_then(|_| plain_text(v.get("credits"), MAX_CREDITS).map(|t| t.replace('\n', " ")));
    ArtistInfo {
        biography: plain_text(v.get("biography"), MAX_TEXT),
        photo,
        credits,
    }
}

/// The gateway's album answer, checked.
pub fn parse_album_info(v: &Value) -> AlbumInfo {
    AlbumInfo {
        review: plain_text(v.get("review"), MAX_TEXT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PHOTO: &str = "/v1/art/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image";

    #[test]
    fn accepts_the_gateway_answers() {
        let a = parse_artist_info(&json!({
            "biography": "First paragraph.\n\nSecond.\nLine two.",
            "photo": PHOTO,
            "credits": "Discogs"
        }));
        assert_eq!(
            a,
            ArtistInfo {
                biography: Some("First paragraph.\n\nSecond.\nLine two.".into()),
                photo: Some(
                    "imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image".into()
                ),
                credits: Some("Discogs".into()),
            }
        );
        // The key can be fetched, sized.
        assert_eq!(
            crate::art::art_path(a.photo.as_deref().unwrap(), 600).unwrap(),
            "imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image_600x600"
        );
        assert_eq!(
            parse_artist_info(&json!({"biography": null, "photo": null, "credits": null})),
            ArtistInfo::default()
        );
        assert_eq!(
            parse_album_info(&json!({"review": "Great."}))
                .review
                .as_deref(),
            Some("Great.")
        );
        assert_eq!(
            parse_album_info(&json!({"review": null})),
            AlbumInfo::default()
        );
    }

    #[test]
    fn photos_must_be_gateway_proxy_paths() {
        for bad in [
            json!("https://images.example.com/a.jpg"),
            json!("imageproxy/https%3A%2F%2Fimages.example.com%2Fa.jpg/image"),
            json!("/v1/art/music/1/cover"),
            json!("/v1/art/html/images/x.png"),
            json!("/v1/me"),
            json!("/v1/art/imageproxy/https%3A%2F%2Fx.example/image/../../../v1/me"),
            json!("/v1/art/imageproxy/file%3A%2F%2F%2Fetc%2Fpasswd/image"),
            json!("/v1/art/imageproxy/https%3A%2F%2Fx.example/image?x=1"),
            json!("/v1/art/imageproxy/https%3A%2F%2Fx y.example/image"),
            json!("//evil.example/v1/art/imageproxy/https%3A%2F%2Fx.example/image"),
            json!(5),
            json!(["x"]),
        ] {
            let a = parse_artist_info(&json!({"photo": bad, "credits": "Discogs"}));
            assert_eq!(a.photo, None, "{bad}");
            assert_eq!(a.credits, None, "{bad}");
        }
    }

    #[test]
    fn text_is_checked_again() {
        let a = parse_artist_info(&json!({
            "biography": "  A\u{0}b\u{1b}[2J\u{202e}c \n\n\n\n  d  \r\n e\t",
            "photo": PHOTO,
            "credits": "Line\nbreak"
        }));
        assert_eq!(a.biography.as_deref(), Some("Ab[2Jc\n\nd\ne"));
        assert_eq!(a.credits.as_deref(), Some("Line break"));
        for bad in [json!(""), json!("  \n\u{0} "), json!(42), json!({"x": 1})] {
            assert_eq!(parse_album_info(&json!({"review": bad})).review, None);
        }
        let long = "x".repeat(MAX_TEXT * 2);
        let r = parse_album_info(&json!({ "review": long })).review.unwrap();
        assert_eq!(r.chars().count(), MAX_TEXT);
    }

    #[test]
    fn hostile_answers_never_panic() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let atoms = [
            "\n",
            "\r",
            " ",
            "\u{0}",
            "é",
            "🎵",
            "/v1/art/",
            "imageproxy/",
            "%2F",
            "https",
            "/image",
            "..",
            "x",
            "\u{202e}",
        ];
        for _ in 0..5_000 {
            let n = (next() % 16) as usize;
            let s: String = (0..n)
                .map(|_| atoms[(next() % atoms.len() as u64) as usize])
                .collect();
            let a = parse_artist_info(&json!({"biography": s, "photo": s, "credits": s}));
            if let Some(b) = a.biography {
                assert!(!b.chars().any(|c| c.is_control() && c != '\n'));
                assert!(!b.contains("\n\n\n"));
            }
            if let Some(p) = a.photo {
                assert!(crate::art::valid_path(&p));
            }
        }
    }
}
