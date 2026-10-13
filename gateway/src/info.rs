//! Artist biographies and photos, and album reviews (docs/plans/ux-research.md X5 part 1):
//! `GET /v1/info/artist/{id}` and `GET /v1/info/album/{id}`.
//!
//! - **Who:** any signed-in user, within their `/v1/me/…` allowance ([`history::caller`]).
//! - **Request:** a library artist or album id and nothing else (a positive integer; no
//!   query string). No name, path or URL is ever taken from the client.
//! - **Source:** the Music & Artist Information plugin, when installed, asked with exactly
//!   `musicartistinfo biography|artistphoto artist_id:<id>` or
//!   `musicartistinfo albumreview album_id:<id>`. The plugin may look these up online under
//!   its owner's settings; we call no such service ourselves.
//! - **Answers:** `{"biography": text|null, "photo": path|null, "credits": text|null}` and
//!   `{"review": text|null}`. Text is cleaned here ([`clean_text`]): HTML tags removed
//!   (with scripts and styles), entities decoded, control and formatting characters
//!   removed, whitespace collapsed (paragraphs kept), lines naming a server path or a LAN
//!   address dropped ([`rpc_policy::sensitive_text`]), and at most [`MAX_TEXT`] characters.
//! - **Photos:** the plugin names an external image URL. It is never handed out: if it is
//!   an http(s) URL that is not on the home network ([`art::local_url`]), the gateway learns
//!   it into the image proxy's allow-list and answers with the proxy path
//!   `/v1/art/imageproxy/<url-encoded URL>/image`, which LMS fetches only after the usual
//!   checks (`art::art`, including the DNS check). Anything else is no photo.
//! - **Limits:** at most [`LOOKUPS`] requests with plugin lookups at once (beyond that, 429
//!   with `Retry-After`), each lookup within [`crate::Config::info_lookup_timeout`].
//!   Answers are remembered for [`HIT_TTL`] (nothing found: [`MISS_TTL`]), at most
//!   [`CACHE_ENTRIES`] of them. Failures and timeouts are never remembered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::AppState;
use crate::history::{caller, error, ok};
use crate::{art, download, rpc_policy};

/// Requests with plugin lookups running at once, for everyone.
pub const LOOKUPS: usize = 4;
/// Default time allowed for one plugin lookup.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);
/// A lookup that failed or timed out (the plugin's online source hanging for that artist)
/// is remembered this long, so each visit doesn't wait for the time limit again.
pub const STALL_TTL: Duration = Duration::from_secs(10 * 60);
/// How long an answer with something in it is remembered.
pub const HIT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long "nothing found" is remembered.
pub const MISS_TTL: Duration = Duration::from_secs(60 * 60);
/// Answers remembered, at most.
pub const CACHE_ENTRIES: usize = 1024;
/// Longest cleaned text, in characters.
pub const MAX_TEXT: usize = 20_000;
/// Longest photo credit, in characters.
pub const MAX_CREDITS: usize = 120;
/// Bytes of plugin text looked at, at most (the rest is ignored before any parsing).
const MAX_INPUT: usize = 512 * 1024;
/// Longest photo URL accepted (as the image proxy).
const MAX_URL: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Artist(u64),
    Album(u64),
}

struct Entry {
    at: Instant,
    ttl: Duration,
    value: Arc<Value>,
    /// The photo's real URL, learned into the image proxy again on every hit (the proxy's
    /// allow-list is bounded and may have forgotten it).
    photo: Option<String>,
}

/// Remembered answers and the plugin lookup limit.
pub struct InfoCache {
    entries: Mutex<HashMap<Key, Entry>>,
    lookups: Semaphore,
}

impl Default for InfoCache {
    fn default() -> Self {
        InfoCache {
            entries: Mutex::new(HashMap::new()),
            lookups: Semaphore::new(LOOKUPS),
        }
    }
}

impl InfoCache {
    fn get(&self, key: Key, now: Instant) -> Option<(Arc<Value>, Option<String>)> {
        let e = self.entries.lock().unwrap();
        e.get(&key)
            .filter(|x| now.duration_since(x.at) < x.ttl)
            .map(|x| (x.value.clone(), x.photo.clone()))
    }

    fn put(&self, key: Key, value: Arc<Value>, photo: Option<String>, hit: bool, now: Instant) {
        self.put_for(key, value, photo, if hit { HIT_TTL } else { MISS_TTL }, now);
    }

    fn put_for(
        &self,
        key: Key,
        value: Arc<Value>,
        photo: Option<String>,
        ttl: Duration,
        now: Instant,
    ) {
        let mut e = self.entries.lock().unwrap();
        if e.len() >= CACHE_ENTRIES && !e.contains_key(&key) {
            e.retain(|_, x| now.duration_since(x.at) < x.ttl);
            if e.len() >= CACHE_ENTRIES {
                // Still full of fresh entries: drop the oldest.
                if let Some(oldest) = e.iter().min_by_key(|(_, x)| x.at).map(|(k, _)| *k) {
                    e.remove(&oldest);
                }
            }
        }
        e.insert(
            key,
            Entry {
                at: now,
                ttl,
                value,
                photo,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Text cleaning
// ---------------------------------------------------------------------------

/// Elements whose content is never text.
const SKIPPED: &[&str] = &[
    "script", "style", "noscript", "template", "iframe", "object", "svg", "math", "textarea",
    "title", "head", "select",
];
/// Elements that end a paragraph.
const PARAGRAPH: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "table",
    "blockquote",
    "section",
    "article",
    "header",
    "footer",
    "pre",
    "hr",
    "dl",
    "figure",
];
/// Elements that end a line.
const LINE: &[&str] = &["br", "li", "tr", "dt", "dd"];
/// Elements that separate words.
const SPACE: &[&str] = &["td", "th", "img"];

/// Named entities decoded (others are left as written).
const ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", ' '),
    ("ndash", '–'),
    ("mdash", '—'),
    ("hellip", '…'),
    ("lsquo", '‘'),
    ("rsquo", '’'),
    ("sbquo", '‚'),
    ("ldquo", '“'),
    ("rdquo", '”'),
    ("bdquo", '„'),
    ("laquo", '«'),
    ("raquo", '»'),
    ("copy", '©'),
    ("reg", '®'),
    ("trade", '™'),
    ("deg", '°'),
    ("middot", '·'),
    ("bull", '•'),
    ("times", '×'),
    ("frac12", '½'),
    ("pound", '£'),
    ("euro", '€'),
    ("eacute", 'é'),
    ("egrave", 'è'),
    ("ecirc", 'ê'),
    ("euml", 'ë'),
    ("aacute", 'á'),
    ("agrave", 'à'),
    ("acirc", 'â'),
    ("auml", 'ä'),
    ("aring", 'å'),
    ("iacute", 'í'),
    ("iuml", 'ï'),
    ("oacute", 'ó'),
    ("ograve", 'ò'),
    ("ocirc", 'ô'),
    ("ouml", 'ö'),
    ("oslash", 'ø'),
    ("uacute", 'ú'),
    ("ugrave", 'ù'),
    ("uuml", 'ü'),
    ("ntilde", 'ñ'),
    ("ccedil", 'ç'),
    ("szlig", 'ß'),
    ("Eacute", 'É'),
    ("Auml", 'Ä'),
    ("Ouml", 'Ö'),
    ("Uuml", 'Ü'),
    ("Aring", 'Å'),
    ("Oslash", 'Ø'),
];

/// Position of `needle` (ASCII, lower case) in `hay`, ignoring ASCII case.
fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// The entity at the start of `s` (which starts with `&`): the character and the bytes it
/// spans.
fn entity(s: &str) -> Option<(char, usize)> {
    let end = s.as_bytes().iter().take(12).position(|b| *b == b';')?;
    let body = &s[1..end];
    let c = if let Some(num) = body.strip_prefix('#') {
        let n = match num.strip_prefix(['x', 'X']) {
            Some(hex) if !hex.is_empty() && hex.len() <= 6 => u32::from_str_radix(hex, 16).ok()?,
            None if !num.is_empty()
                && num.len() <= 7
                && num.bytes().all(|b| b.is_ascii_digit()) =>
            {
                num.parse().ok()?
            }
            _ => return None,
        };
        // Line breaks and tabs are whitespace; any other control character (NUL included)
        // is dropped later.
        char::from_u32(n)?
    } else {
        ENTITIES.iter().find(|(name, _)| *name == body)?.1
    };
    Some((c, end + 1))
}

/// Text with HTML tags removed (scripts, styles and comments with their content), block
/// elements turned into line and paragraph breaks, and entities decoded. Linear in the
/// input; never panics.
fn strip_html(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len().min(MAX_INPUT));
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' => {
                let rest = &b[i + 1..];
                if rest.starts_with(b"!--") {
                    i = match find_ci(&rest[3..], b"-->") {
                        Some(j) => i + 1 + 3 + j + 3,
                        None => b.len(),
                    };
                    continue;
                }
                let tagish = rest
                    .first()
                    .is_some_and(|c| c.is_ascii_alphabetic() || b"/!?".contains(c));
                if !tagish {
                    out.push('<');
                    i += 1;
                    continue;
                }
                // The end of the tag, skipping `>` inside quoted attribute values. An
                // unterminated tag swallows the rest (it can't be told from text safely).
                let mut quote = None;
                let mut end = None;
                for (j, c) in rest.iter().enumerate() {
                    match (quote, *c) {
                        (Some(q), c) if c == q => quote = None,
                        (Some(_), _) => {}
                        (None, b'"' | b'\'') => quote = Some(*c),
                        (None, b'>') => {
                            end = Some(j);
                            break;
                        }
                        _ => {}
                    }
                }
                let Some(end) = end else { break };
                let inner = &rest[..end];
                let closing = inner.first() == Some(&b'/');
                let name: String = inner
                    .iter()
                    .skip(usize::from(closing))
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .map(|c| c.to_ascii_lowercase() as char)
                    .collect();
                i += 1 + end + 1;
                let self_closing = inner.last() == Some(&b'/');
                if !closing && !self_closing && SKIPPED.contains(&name.as_str()) {
                    let close = format!("</{name}");
                    i = match find_ci(&b[i..], close.as_bytes()) {
                        Some(j) => {
                            // Past the closing tag's `>`.
                            let after = i + j;
                            match b[after..].iter().position(|c| *c == b'>') {
                                Some(k) => after + k + 1,
                                None => b.len(),
                            }
                        }
                        None => b.len(),
                    };
                    continue;
                }
                if PARAGRAPH.contains(&name.as_str()) {
                    out.push_str("\n\n");
                } else if LINE.contains(&name.as_str()) {
                    out.push('\n');
                } else if SPACE.contains(&name.as_str()) {
                    out.push(' ');
                }
            }
            b'&' => match entity(&s[i..]) {
                Some((c, n)) => {
                    out.push(c);
                    i += n;
                }
                None => {
                    out.push('&');
                    i += 1;
                }
            },
            _ => {
                // `i` is always on a character boundary: every other branch steps over
                // ASCII bytes only, or a whole entity.
                let c = s[i..].chars().next().unwrap_or('\u{fffd}');
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

/// Invisible characters that can disguise text (bidirectional overrides, zero-width
/// characters, the byte order mark).
pub(crate) fn formatting(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}' | '\u{00ad}')
}

/// Plugin text (HTML or plain) made safe to show as plain text: see the module notes.
/// `None` if nothing is left.
pub fn clean_text(raw: &str, max: usize) -> Option<String> {
    let mut cut = raw.len().min(MAX_INPUT);
    while !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    let text = strip_html(&raw[..cut]).replace("\r\n", "\n");
    // Lines, with whitespace collapsed and anything invisible removed; paragraphs are
    // separated by one empty line.
    let mut paragraphs: Vec<Vec<String>> = vec![vec![]];
    for line in text.split(['\n', '\r', '\u{2028}', '\u{2029}']) {
        let line: String = line
            .chars()
            .map(|c| if c == '\t' || c == '\u{a0}' { ' ' } else { c })
            .filter(|c| !c.is_control() && !formatting(*c))
            .collect();
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if paragraphs.last().is_some_and(|p| !p.is_empty()) {
                paragraphs.push(vec![]);
            }
            continue;
        }
        if rpc_policy::sensitive_text(&line) {
            continue;
        }
        if let Some(p) = paragraphs.last_mut() {
            p.push(line);
        }
    }
    let joined = paragraphs
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.join("\n"))
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() <= max {
        return Some(joined);
    }
    // Too long: cut at a word boundary near the limit.
    let head: String = joined.chars().take(max.saturating_sub(1)).collect();
    let at = head
        .char_indices()
        .rev()
        .take(200)
        .find(|(_, c)| c.is_whitespace())
        .map_or(head.len(), |(i, _)| i);
    let mut out = head[..at].trim_end().to_string();
    out.push('…');
    // A cut can make the last line look like something it wasn't (a web address cut down
    // to a single-label name): judge it again.
    loop {
        let (rest, last) = out.rsplit_once('\n').unwrap_or(("", &out));
        if !rpc_policy::sensitive_text(last) {
            break;
        }
        out = rest.trim_end().to_string();
    }
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// Photos
// ---------------------------------------------------------------------------

/// Percent-encode everything but ASCII letters and digits (as the clients do).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The real image URL behind the plugin's `url`, if it may be fetched through the image
/// proxy: an http(s) URL (or an `imageproxy/<url>` path naming one), not on the home
/// network, of sensible length, without whitespace or control characters.
pub fn photo_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let url = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        art::proxy_target(raw)?
    };
    let ok = url.len() > "https://".len()
        && url.len() <= MAX_URL
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || formatting(c) || c == '\\')
        && !art::local_url(&url);
    ok.then_some(url)
}

/// The path clients fetch a photo from.
pub fn photo_path(url: &str) -> String {
    format!("/v1/art/imageproxy/{}/image", encode(url))
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

fn text_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    match v.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// The answer for an artist from the plugin's two answers (`None`: that lookup failed).
fn artist_answer(bio: Option<&Value>, photo: Option<&Value>) -> (Value, Option<String>) {
    let biography = bio
        .and_then(|v| text_field(v, "biography"))
        .and_then(|t| clean_text(t, MAX_TEXT));
    let url = photo.and_then(|v| text_field(v, "url")).and_then(photo_url);
    let credits = url.as_ref().and(
        photo
            .and_then(|v| text_field(v, "credits"))
            .and_then(|t| clean_text(t, MAX_CREDITS))
            .map(|t| t.replace('\n', " ")),
    );
    let v = json!({
        "biography": biography,
        "photo": url.as_deref().map(photo_path),
        "credits": credits,
    });
    (v, url)
}

fn album_answer(review: &Value) -> Value {
    json!({
        "review": text_field(review, "albumreview").and_then(|t| clean_text(t, MAX_TEXT)),
    })
}

/// Teach the image proxy a photo URL (so its path can be fetched).
fn learn_photo(gw: &AppState, url: Option<&str>) {
    if let Some(u) = url {
        gw.known_images.lock().unwrap().insert(u.to_string());
    }
}

/// Checks shared by both endpoints: the caller, the id and no query string.
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
fn check(
    gw: &AppState,
    headers: &HeaderMap,
    id: &str,
    query: Option<&str>,
    what: &str,
) -> Result<u64, Response> {
    caller(gw, headers)?;
    let Some(id) = download::parse_track_id(id) else {
        return Err(error(StatusCode::NOT_FOUND, what));
    };
    if query.is_some() {
        return Err(error(StatusCode::BAD_REQUEST, "unknown parameter"));
    }
    Ok(id)
}

fn busy() -> Response {
    let mut r = error(
        StatusCode::TOO_MANY_REQUESTS,
        "too many information lookups; try again shortly",
    );
    r.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    r
}

async fn lookup(gw: &AppState, cmd: [String; 3]) -> Option<Value> {
    match gw
        .lms_command_within("", &cmd, gw.cfg.info_lookup_timeout)
        .await
    {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::debug!(error = %e, what = %cmd[1], "information lookup failed");
            None
        }
    }
}

fn has_something(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|m| m.values().any(|x| !x.is_null()))
}

/// `GET /v1/info/artist/{id}`
pub async fn artist(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let id = match check(&gw, &headers, &id, query.as_deref(), "no such artist") {
        Ok(id) => id,
        Err(r) => return r,
    };
    let key = Key::Artist(id);
    if let Some((v, photo)) = gw.info.get(key, Instant::now()) {
        learn_photo(&gw, photo.as_deref());
        return ok((*v).clone());
    }
    let Ok(_permit) = gw.info.lookups.try_acquire() else {
        return busy();
    };
    let ask = |what: &str| {
        [
            "musicartistinfo".to_string(),
            what.to_string(),
            format!("artist_id:{id}"),
        ]
    };
    let (bio, photo) = tokio::join!(
        lookup(&gw, ask("biography")),
        lookup(&gw, ask("artistphoto"))
    );
    let complete = bio.is_some() && photo.is_some();
    let (v, url) = artist_answer(bio.as_ref(), photo.as_ref());
    learn_photo(&gw, url.as_deref());
    if complete {
        let hit = has_something(&v);
        gw.info
            .put(key, Arc::new(v.clone()), url, hit, Instant::now());
    } else {
        // Failed or timed out (in part): what there is, briefly, so the page doesn't wait
        // for the time limit on every visit.
        gw.info
            .put_for(key, Arc::new(v.clone()), url, STALL_TTL, Instant::now());
    }
    ok(v)
}

/// `GET /v1/info/album/{id}`
pub async fn album(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let id = match check(&gw, &headers, &id, query.as_deref(), "no such album") {
        Ok(id) => id,
        Err(r) => return r,
    };
    let key = Key::Album(id);
    if let Some((v, _)) = gw.info.get(key, Instant::now()) {
        return ok((*v).clone());
    }
    let Ok(_permit) = gw.info.lookups.try_acquire() else {
        return busy();
    };
    let cmd = [
        "musicartistinfo".to_string(),
        "albumreview".into(),
        format!("album_id:{id}"),
    ];
    let Some(r) = lookup(&gw, cmd).await else {
        let v = album_answer(&Value::Null);
        gw.info
            .put_for(key, Arc::new(v.clone()), None, STALL_TTL, Instant::now());
        return ok(v);
    };
    let v = album_answer(&r);
    let hit = has_something(&v);
    gw.info
        .put(key, Arc::new(v.clone()), None, hit, Instant::now());
    ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> Option<String> {
        clean_text(s, MAX_TEXT)
    }

    #[test]
    fn html_becomes_paragraphs_of_plain_text() {
        let raw = "<p>The <b>Invented</b> Band formed in 1990 &amp; split in 1999.</p>\
                   <p>Second <a href=\"https://www.example.com/x\">paragraph</a>.<br>Next line.</p>";
        assert_eq!(
            clean(raw).unwrap(),
            "The Invented Band formed in 1990 & split in 1999.\n\nSecond paragraph.\nNext line."
        );
    }

    #[test]
    fn plain_text_keeps_paragraphs_and_collapses_whitespace() {
        let raw =
            "  First   line\r\n\twrapped here \r\n\r\n\r\n\r\nSecond\u{a0}\u{a0}paragraph  \n";
        assert_eq!(
            clean(raw).unwrap(),
            "First line\nwrapped here\n\nSecond paragraph"
        );
    }

    #[test]
    fn scripts_styles_and_comments_are_removed_with_their_content() {
        let raw = "Before<script type=\"text/javascript\">alert('x<p>')</script>\
                   <STYLE>p{color:red}</STYLE><!-- hidden <b>note</b> -->\
                   <noscript>no</noscript><svg><text>svg</text></svg>After";
        assert_eq!(clean(raw).unwrap(), "BeforeAfter");
        // Unclosed: everything after it is dropped.
        assert_eq!(clean("Kept<script>alert(1)").unwrap(), "Kept");
        assert_eq!(clean("Kept<!-- open comment").unwrap(), "Kept");
        assert_eq!(clean("Kept<a href=\"x"), Some("Kept".into()));
        assert_eq!(clean("<script>only</script>"), None);
    }

    #[test]
    fn attributes_with_angle_brackets_and_stray_signs() {
        assert_eq!(
            clean("<a title=\"a>b\" href='c>d'>Link</a> 3 < 4 > 2 & so on").unwrap(),
            "Link 3 < 4 > 2 & so on"
        );
    }

    #[test]
    fn entities_are_decoded_once() {
        assert_eq!(
            clean("&lt;script&gt;alert(1)&lt;/script&gt; caf&eacute; &#233;&#xE9; &mdash; &quot;q&quot; &rsquo;").unwrap(),
            "<script>alert(1)</script> café éé — \"q\" ’"
        );
        // Double-encoded text stays encoded once; unknown and malformed entities as written.
        assert_eq!(
            clean("&amp;lt; &bogus; &#; &#x; &#99999999; & ;").unwrap(),
            "&lt; &bogus; &#; &#x; &#99999999; & ;"
        );
        // Encoded control characters and NUL never survive.
        assert_eq!(clean("a&#0;b&#7;c&#x1b;[31md").unwrap(), "abc[31md");
    }

    #[test]
    fn control_and_invisible_characters_are_removed() {
        let raw = "Name\u{0}\u{7}\u{1b}[2J\u{202e}gpj.exe\u{200b}\u{feff}\u{2066}x\u{7f}";
        assert_eq!(clean(raw).unwrap(), "Name[2Jgpj.exex");
        assert_eq!(clean("\u{0}\u{1}\u{202e}  \t"), None);
    }

    #[test]
    fn lines_naming_paths_or_lan_addresses_are_dropped() {
        let raw = "<p>Fine words about AC/DC in 1990/91.</p>\
                   <p>Local copy: file:///srv/music/bio.txt</p>\
                   <p>See http://10.0.0.10:9000/plugins/x</p>\
                   <p>Found in /srv/music/Artist/bio.txt</p>\
                   <p>More at https://www.example.com/artist.</p>";
        assert_eq!(
            clean(raw).unwrap(),
            "Fine words about AC/DC in 1990/91.\n\nMore at https://www.example.com/artist."
        );
        assert_eq!(clean("/srv/music/Artist/bio.txt"), None);
    }

    #[test]
    fn long_text_is_capped_at_a_word() {
        let raw = "word ".repeat(10_000);
        let out = clean(&raw).unwrap();
        assert!(out.chars().count() <= MAX_TEXT);
        assert!(out.ends_with("word…"), "{}", &out[out.len() - 20..]);
        let unbroken = "x".repeat(MAX_TEXT * 2);
        let out = clean(&unbroken).unwrap();
        assert_eq!(out.chars().count(), MAX_TEXT);
        // Multi-byte characters are cut on boundaries.
        let wide = "é🎵".repeat(MAX_INPUT);
        assert!(clean(&wide).unwrap().chars().count() <= MAX_TEXT);
        assert_eq!(clean_text("abcdef ghij", 6).unwrap(), "abcde…");
    }

    #[test]
    fn huge_and_pathological_input_is_fast() {
        let t = Instant::now();
        for raw in [
            "<".repeat(2_000_000),
            "<a".repeat(1_000_000),
            "<script>".repeat(200_000),
            "&".repeat(2_000_000),
            "&#x".repeat(500_000),
            "<!--".repeat(500_000),
            "<p>".repeat(500_000),
            "\n".repeat(2_000_000),
        ] {
            let _ = clean(&raw);
        }
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    }

    #[test]
    fn hostile_text_never_panics_and_stays_clean() {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let atoms = [
            "<",
            ">",
            "</",
            "<p>",
            "<br/>",
            "<script>",
            "</script>",
            "<!--",
            "-->",
            "&",
            "&amp;",
            "&#",
            "&#x",
            ";",
            "1",
            "f",
            "\"",
            "'",
            "é",
            "🎵",
            "\u{0}",
            "\u{202e}",
            "\r\n",
            "\n\n",
            " ",
            "\t",
            "/srv/music/a",
            "http://10.0.0.1/",
            "https://www.example.com/",
            "<style",
            "<svg/>",
            "x",
            "Text",
        ];
        for _ in 0..20_000 {
            let n = (next() % 24) as usize;
            let s: String = (0..n)
                .map(|_| atoms[(next() % atoms.len() as u64) as usize])
                .collect();
            let max = 1 + (next() % 64) as usize;
            if let Some(out) = clean_text(&s, max) {
                assert!(out.chars().count() <= max, "{s:?} -> {out:?}");
                assert!(
                    !out.chars()
                        .any(|c| (c.is_control() && c != '\n') || formatting(c)),
                    "{s:?} -> {out:?}"
                );
                assert!(!out.contains("\n\n\n") && !out.starts_with(char::is_whitespace));
                for line in out.lines() {
                    assert!(!rpc_policy::sensitive_text(line), "{s:?} -> {out:?}");
                }
            }
        }
        // Raw bytes interpreted as text, too.
        for _ in 0..5_000 {
            let n = (next() % 64) as usize;
            let bytes: Vec<u8> = (0..n).map(|_| next() as u8).collect();
            let _ = clean(&String::from_utf8_lossy(&bytes));
        }
    }

    #[test]
    fn photos_are_proxied_only_when_external() {
        let u = "https://images.example.com/artist/1.jpg";
        assert_eq!(photo_url(u).as_deref(), Some(u));
        assert_eq!(
            photo_path(u),
            "/v1/art/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fartist%2F1%2Ejpg/image"
        );
        // The path is one the image proxy accepts, naming the same URL.
        let p = photo_path(u);
        assert_eq!(
            art::target(p.strip_prefix("/v1/art/").unwrap()),
            Some(art::Target::Proxy {
                url: u.into(),
                name: "image".into(),
                encoded: "https%3A%2F%2Fimages%2Eexample%2Ecom%2Fartist%2F1%2Ejpg".into(),
            })
        );
        // An imageproxy path naming an external URL is unwrapped.
        assert_eq!(
            photo_url("/imageproxy/https%3A%2F%2Fimages.example.com%2Fa.png/image.png").as_deref(),
            Some("https://images.example.com/a.png")
        );
        for bad in [
            "",
            "http://10.0.0.10:9000/music/1/cover.jpg",
            "http://lms.example.lan/a.jpg",
            "http://nas/a.jpg",
            "http://localhost/a.jpg",
            "http://127.0.0.1/a.jpg",
            "http://10.0.0.5.nip.io/a.jpg",
            "file:///srv/music/Artist/artist.jpg",
            "/srv/music/Artist/artist.jpg",
            "/music/1/cover.jpg",
            "/plugins/MusicArtistInfo/html/x.png",
            "imageproxy/http%3A%2F%2F10.0.0.1%2Fa.jpg/image.png",
            "ftp://images.example.com/a.jpg",
            "javascript:alert(1)",
            "https://",
            "https://images.example.com/a b.jpg",
            "https://images.example.com/a\u{0}.jpg",
            "https://images.example.com\\@10.0.0.1/a.jpg",
        ] {
            assert_eq!(photo_url(bad), None, "{bad}");
        }
        let long = format!("https://images.example.com/{}", "a".repeat(MAX_URL));
        assert_eq!(photo_url(&long), None);
    }

    #[test]
    fn answers_take_only_known_fields() {
        let (v, url) = artist_answer(
            Some(&json!({"biography": "<p>Bio</p>", "artist": "x", "artist_id": "1"})),
            Some(
                &json!({"url": "https://images.example.com/a.jpg", "credits": "<b>Discogs</b>",
                         "width": "600", "height": "600"}),
            ),
        );
        assert_eq!(url.as_deref(), Some("https://images.example.com/a.jpg"));
        assert_eq!(
            v,
            json!({"biography": "Bio",
                   "photo": "/v1/art/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image",
                   "credits": "Discogs"})
        );
        // No photo: no credits either.
        let (v, url) = artist_answer(
            Some(&json!({"error": "nothing"})),
            Some(&json!({"url": "http://10.0.0.10/x.jpg", "credits": "Discogs"})),
        );
        assert_eq!(url, None);
        assert_eq!(
            v,
            json!({"biography": null, "photo": null, "credits": null})
        );
        assert!(!has_something(&v));
        // Wrong types are ignored.
        let (v, _) = artist_answer(Some(&json!({"biography": 5})), Some(&json!({"url": ["x"]})));
        assert!(!has_something(&v));
        assert_eq!(
            album_answer(&json!({"albumreview": "Great &amp; good", "album": "x"})),
            json!({"review": "Great & good"})
        );
        assert_eq!(album_answer(&json!({})), json!({"review": null}));
    }

    #[test]
    fn cache_expires_and_is_bounded() {
        let c = InfoCache::default();
        let t0 = Instant::now();
        c.put(
            Key::Album(1),
            Arc::new(json!({"review": null})),
            None,
            false,
            t0,
        );
        c.put(
            Key::Artist(1),
            Arc::new(json!({"biography": "x"})),
            None,
            true,
            t0,
        );
        assert!(c.get(Key::Album(1), t0).is_some());
        assert!(c.get(Key::Album(1), t0 + MISS_TTL).is_none());
        assert!(c.get(Key::Artist(1), t0 + MISS_TTL).is_some());
        assert!(c.get(Key::Artist(1), t0 + HIT_TTL).is_none());
        // Artist and album ids are kept apart.
        assert!(c.get(Key::Artist(2), t0).is_none());
        for i in 0..(CACHE_ENTRIES as u64 * 2) {
            c.put(
                Key::Album(i + 10),
                Arc::new(json!(i)),
                None,
                true,
                t0 + Duration::from_millis(i),
            );
        }
        assert!(c.entries.lock().unwrap().len() <= CACHE_ENTRIES);
        let last = CACHE_ENTRIES as u64 * 2 + 9;
        assert!(
            c.get(Key::Album(last), t0 + Duration::from_secs(1))
                .is_some()
        );
    }
}
