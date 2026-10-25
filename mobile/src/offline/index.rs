//! The offline index: what this device has downloaded for one user on one server.
//!
//! A small JSON file (`index.json`) listing each track's stable key, current LMS id, title,
//! artist, album, number, duration, format, the original file's type (with its rate and depth
//! when known), file extension and size, plus whether the album's artwork is cached. It
//! never holds tokens, server addresses or file paths: a
//! track's file is named by its stable key (64 hex digits) and an extension from a fixed
//! list, so nothing read from the index can point outside the download folder.
//!
//! The file is untrusted on reading (storage can be damaged, or a newer or older app may
//! have written it): every entry is checked, anything malformed is dropped rather than
//! failing the whole index, text is cut to length, and the file and its entry count are
//! bounded. Writes go to a temporary file that replaces the index atomically, keeping the
//! previous version as a backup that is read if the index itself is unreadable.

use std::collections::HashSet;

use serde_json::{Value, json};

/// Most tracks the index holds (about 3 TB of FLAC: far beyond any phone).
pub const MAX_TRACKS: usize = 100_000;
/// Largest index file read (100,000 tracks at about 500 bytes each, with room to spare).
pub const MAX_INDEX_BYTES: u64 = 96 * 1024 * 1024;
/// Longest title, artist or album kept.
pub const MAX_TEXT: usize = 300;
/// The index format written by this version.
pub const VERSION: u64 = 1;

/// What a track was downloaded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum DownloadFormat {
    /// The library file as it is.
    Original,
    /// MP3, 320 kb/s (made by the gateway).
    Mp3,
    /// Opus, 160 kb/s in Ogg (made by the gateway).
    Opus,
}

impl DownloadFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadFormat::Original => "original",
            DownloadFormat::Mp3 => "mp3",
            DownloadFormat::Opus => "opus",
        }
    }

    pub fn parse(s: &str) -> Option<DownloadFormat> {
        match s {
            "original" => Some(DownloadFormat::Original),
            "mp3" => Some(DownloadFormat::Mp3),
            "opus" => Some(DownloadFormat::Opus),
            _ => None,
        }
    }
}

/// Why a track is on the device: chosen by the user (kept until removed), or by a smart
/// download (removed again when it falls out of its list or the storage cap needs room).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum DownloadOrigin {
    Manual,
    /// "Keep recently added albums offline".
    Recent,
    /// "Keep favourites offline".
    Favourite,
}

impl DownloadOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadOrigin::Manual => "manual",
            DownloadOrigin::Recent => "recent",
            DownloadOrigin::Favourite => "favourite",
        }
    }

    pub fn parse(s: &str) -> Option<DownloadOrigin> {
        match s {
            "manual" => Some(DownloadOrigin::Manual),
            "recent" => Some(DownloadOrigin::Recent),
            "favourite" => Some(DownloadOrigin::Favourite),
            _ => None,
        }
    }

    /// Smart downloads may be removed automatically; the user's own never are.
    pub fn evictable(self) -> bool {
        self != DownloadOrigin::Manual
    }
}

/// A downloaded track, as kept in the index.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflineTrack {
    /// The gateway's stable key (64 hex digits): survives LMS rescans.
    pub key: String,
    /// The LMS track id when it was downloaded (may change on a rescan).
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<i64>,
    pub track_number: Option<u32>,
    /// 0 when unknown.
    pub duration_secs: f64,
    pub format: DownloadFormat,
    /// LMS's file type for originals (e.g. "flc"), else the format.
    pub codec: String,
    /// The library file the download was made from, whatever the format: LMS's type and,
    /// when known, its rate and depth, so an Opus or MP3 copy of a lossless track reads as
    /// *Transcoded*. `None` in entries written before it was kept.
    pub original: Option<crate::quality::QualityFile>,
    /// The file's extension (a fixed list).
    pub ext: String,
    pub size: u64,
    /// Unix seconds.
    pub downloaded_at: u64,
    pub origin: DownloadOrigin,
}

impl OfflineTrack {
    /// How the album groups and finds its cached artwork: the LMS album id, or (for a track
    /// LMS didn't give one) the track's own key.
    pub fn album_ref(&self) -> String {
        match self.album_id {
            Some(id) => format!("a{id}"),
            None => format!("t{}", &self.key[..16]),
        }
    }
}

/// File extensions a download may be stored with. Anything else from the server is
/// refused, so a file name is always `<64 hex>.<one of these>`.
pub const EXTENSIONS: [&str; 14] = [
    "flac", "mp3", "opus", "ogg", "oga", "m4a", "mp4", "aac", "wav", "aif", "aiff", "wv", "ape",
    "wma",
];

pub fn valid_key(k: &str) -> bool {
    k.len() == 64 && k.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn valid_ext(e: &str) -> bool {
    EXTENSIONS.contains(&e)
}

/// Printable text, cut to [`MAX_TEXT`] characters: control characters and bidirectional
/// overrides (which could disguise text in the UI) are removed.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}'))
        .take(MAX_TEXT)
        .collect()
}

/// LMS's file type ("flc", "mp3"), as letters and digits only.
pub fn clean_codec(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn track_json(t: &OfflineTrack) -> Value {
    json!({
        "key": t.key,
        "id": t.track_id,
        "title": t.title,
        "artist": t.artist,
        "album": t.album,
        "album_id": t.album_id,
        "tracknum": t.track_number,
        "duration": t.duration_secs,
        "format": t.format.as_str(),
        "codec": t.codec,
        "orig": t.original.as_ref().map(original_json),
        "ext": t.ext,
        "size": t.size,
        "at": t.downloaded_at,
        "origin": t.origin.as_str(),
    })
}

fn original_json(f: &crate::quality::QualityFile) -> Value {
    json!({ "kind": f.kind, "rate": f.rate, "bits": f.bits })
}

/// The original's format, checked: its type as letters and digits (none: no original), and
/// a rate and depth only when they are in range. Anything else reads as unknown.
pub fn original_from(v: Option<&Value>) -> Option<crate::quality::QualityFile> {
    let o = v?.as_object()?;
    let kind = clean_codec(o.get("kind")?.as_str()?);
    if kind.is_empty() {
        return None;
    }
    let num = |k: &str, lo: u64, hi: u64| {
        o.get(k)
            .and_then(Value::as_u64)
            .filter(|n| (lo..=hi).contains(n))
            .map(|n| n as u32)
    };
    Some(crate::quality::QualityFile {
        kind: Some(kind),
        rate: num("rate", 1000, 50_000_000),
        bits: num("bits", 1, 64),
        kbps: None,
        channels: None,
    })
}

/// One entry, checked; `None` for anything malformed.
fn track_from(v: &Value) -> Option<OfflineTrack> {
    let o = v.as_object()?;
    let key = o.get("key")?.as_str()?;
    if !valid_key(key) {
        return None;
    }
    let ext = o.get("ext")?.as_str()?;
    if !valid_ext(ext) {
        return None;
    }
    let positive = |k: &str| {
        o.get(k)
            .and_then(Value::as_i64)
            .filter(|n| (1..=999_999_999_999).contains(n))
    };
    let track_id = positive("id")?;
    let text = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .map(clean)
            .unwrap_or_default()
    };
    let duration = o
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && *d > 0.0 && *d < 1e6)
        .unwrap_or(0.0);
    let size = o.get("size")?.as_u64().filter(|n| *n > 0 && *n < 1 << 40)?;
    Some(OfflineTrack {
        key: key.to_string(),
        track_id,
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        album_id: positive("album_id"),
        track_number: o
            .get("tracknum")
            .and_then(Value::as_u64)
            .filter(|n| *n < 100_000)
            .map(|n| n as u32),
        duration_secs: duration,
        format: DownloadFormat::parse(o.get("format")?.as_str()?)?,
        codec: clean_codec(o.get("codec").and_then(Value::as_str).unwrap_or("")),
        original: original_from(o.get("orig")),
        ext: ext.to_string(),
        size,
        downloaded_at: o.get("at").and_then(Value::as_u64).unwrap_or(0),
        origin: o
            .get("origin")
            .and_then(Value::as_str)
            .and_then(DownloadOrigin::parse)
            .unwrap_or(DownloadOrigin::Manual),
    })
}

/// What reading an index gave.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// The tracks (malformed entries dropped), and how many were dropped.
    Tracks(Vec<OfflineTrack>, usize),
    /// Not an index at all (corrupt, or another format): don't trust any of it.
    Unreadable,
}

/// Parse an index file's bytes. Never panics; bounded in what it accepts.
pub fn parse(bytes: &[u8]) -> Parsed {
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Parsed::Unreadable;
    }
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Parsed::Unreadable;
    };
    if v.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return Parsed::Unreadable;
    }
    let Some(list) = v.get("tracks").and_then(Value::as_array) else {
        return Parsed::Unreadable;
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut dropped = 0;
    for e in list {
        match track_from(e) {
            Some(t) if out.len() < MAX_TRACKS && seen.insert(t.key.clone()) => out.push(t),
            _ => dropped += 1,
        }
    }
    Parsed::Tracks(out, dropped)
}

/// An index file's bytes for `tracks`.
pub fn serialise(tracks: &[OfflineTrack]) -> Vec<u8> {
    let list: Vec<Value> = tracks.iter().take(MAX_TRACKS).map(track_json).collect();
    serde_json::to_vec(&json!({ "v": VERSION, "tracks": list })).expect("JSON encodes")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample(n: u8) -> OfflineTrack {
        OfflineTrack {
            key: format!("{:02x}", n).repeat(32),
            track_id: 100 + i64::from(n),
            title: format!("Track {n}"),
            artist: "Artist".into(),
            album: "Album".into(),
            album_id: Some(7),
            track_number: Some(u32::from(n)),
            duration_secs: 61.5,
            format: DownloadFormat::Original,
            codec: "flc".into(),
            original: Some(crate::quality::QualityFile {
                kind: Some("flc".into()),
                rate: Some(96_000),
                bits: Some(24),
                kbps: None,
                channels: None,
            }),
            ext: "flac".into(),
            size: 1000 + u64::from(n),
            downloaded_at: 1_800_000_000,
            origin: DownloadOrigin::Manual,
        }
    }

    #[test]
    fn round_trips() {
        let mut b = sample(2);
        b.album_id = None;
        b.track_number = None;
        b.format = DownloadFormat::Opus;
        b.origin = DownloadOrigin::Recent;
        b.ext = "opus".into();
        b.codec = "opus".into();
        let mut c = sample(3);
        c.original = None;
        let tracks = vec![sample(1), b, c];
        assert_eq!(parse(&serialise(&tracks)), Parsed::Tracks(tracks, 0));
    }

    /// An index written before the original's format was kept still reads, entry for entry,
    /// with no original.
    #[test]
    fn entries_without_the_original_still_read() {
        let mut e = track_json(&sample(1));
        e.as_object_mut().unwrap().remove("orig");
        let file = serde_json::to_vec(&json!({"v": 1, "tracks": [e]})).unwrap();
        let mut want = sample(1);
        want.original = None;
        assert_eq!(parse(&file), Parsed::Tracks(vec![want], 0));
    }

    /// A damaged or hostile original is unknown, never a reason to drop the track; numbers
    /// out of range are dropped one by one.
    #[test]
    fn the_original_is_checked() {
        let read = |orig: Value| {
            let mut e = track_json(&sample(1));
            e["orig"] = orig;
            let file = serde_json::to_vec(&json!({"v": 1, "tracks": [e]})).unwrap();
            match parse(&file) {
                Parsed::Tracks(t, 0) => t[0].original.clone(),
                other => panic!("{other:?}"),
            }
        };
        for junk in [
            json!(null),
            json!("flc"),
            json!([]),
            json!({}),
            json!({"kind": 7}),
            json!({"kind": ""}),
            json!({"kind": "\u{202e}/../"}),
        ] {
            assert_eq!(read(junk.clone()), None, "{junk}");
        }
        let f = read(json!({"kind": "F\u{202e}LC", "rate": 0, "bits": 99_999_999_999u64})).unwrap();
        assert_eq!(f.kind.as_deref(), Some("flc"));
        assert_eq!((f.rate, f.bits), (None, None));
        let f = read(json!({"kind": "dsf", "rate": 2_822_400, "bits": 1, "kbps": 5})).unwrap();
        assert_eq!((f.rate, f.bits, f.kbps), (Some(2_822_400), Some(1), None));
        let f = read(json!({"kind": "flc", "rate": -96000, "bits": 24.5})).unwrap();
        assert_eq!((f.rate, f.bits), (None, None));
    }

    #[test]
    fn malformed_entries_are_dropped_not_fatal() {
        let good = track_json(&sample(1));
        let mut bad = Vec::new();
        for (k, v) in [
            ("key", json!("../../etc/passwd")),
            ("key", json!("AB".repeat(32))),
            ("key", json!(1)),
            ("ext", json!("sh")),
            ("ext", json!("flac/../../x")),
            ("id", json!(0)),
            ("id", json!(-4)),
            ("id", json!("12")),
            ("size", json!(0)),
            ("size", json!(-1)),
            ("format", json!("wav")),
        ] {
            let mut e = good.clone();
            e[k] = v;
            bad.push(e);
        }
        bad.push(json!("not an object"));
        bad.push(json!(null));
        let n = bad.len();
        bad.push(good.clone());
        bad.push(good.clone()); // a duplicate key
        let file = serde_json::to_vec(&json!({"v": 1, "tracks": bad})).unwrap();
        match parse(&file) {
            Parsed::Tracks(t, dropped) => {
                assert_eq!(t, vec![sample(1)]);
                assert_eq!(dropped, n + 1);
            }
            Parsed::Unreadable => panic!("should read"),
        }
    }

    #[test]
    fn text_is_cleaned_and_bounded() {
        let mut e = track_json(&sample(1));
        e["title"] = json!(format!("A\u{202e}B\u{0}C\n{}", "x".repeat(1000)));
        e["codec"] = json!("F-L/C<script>");
        e["duration"] = json!(f64::MAX);
        e["origin"] = json!("someday");
        let file = serde_json::to_vec(&json!({"v": 1, "tracks": [e]})).unwrap();
        let Parsed::Tracks(t, 0) = parse(&file) else {
            panic!()
        };
        assert!(t[0].title.starts_with("ABC"));
        assert_eq!(t[0].title.chars().count(), MAX_TEXT);
        assert_eq!(t[0].codec, "flcscript");
        assert_eq!(t[0].duration_secs, 0.0);
        assert_eq!(t[0].origin, DownloadOrigin::Manual);
    }

    #[test]
    fn other_files_are_unreadable() {
        for f in [
            &b""[..],
            b"{",
            b"[]",
            b"{\"v\":2,\"tracks\":[]}",
            b"{\"v\":1}",
            b"{\"v\":1,\"tracks\":{}}",
            b"\xff\xfe\x00",
        ] {
            assert_eq!(parse(f), Parsed::Unreadable, "{f:?}");
        }
    }

    #[test]
    fn album_refs_never_hold_path_characters() {
        let mut t = sample(3);
        assert_eq!(t.album_ref(), "a7");
        t.album_id = None;
        assert_eq!(t.album_ref(), format!("t{}", "03".repeat(8)));
    }

    /// Fuzz: arbitrary bytes, and mutations of a real index, never panic, and whatever
    /// survives obeys every rule.
    #[test]
    fn fuzz_never_panics_and_keeps_invariants() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let base = serialise(&(1..=20).map(sample).collect::<Vec<_>>());
        let alphabet = b"{}[]\",:0123456789abcdefv-.etrunlsx\\ \n\xff";
        for round in 0..3000 {
            let bytes: Vec<u8> = match round % 3 {
                0 => (0..next() % 512)
                    .map(|_| alphabet[(next() as usize) % alphabet.len()])
                    .collect(),
                1 => {
                    let mut b = base.clone();
                    for _ in 0..(1 + next() % 8) {
                        let i = (next() as usize) % b.len();
                        b[i] = alphabet[(next() as usize) % alphabet.len()];
                    }
                    b
                }
                _ => {
                    let mut b = base.clone();
                    b.truncate((next() as usize) % b.len());
                    b
                }
            };
            if let Parsed::Tracks(tracks, _) = parse(&bytes) {
                let mut keys = HashSet::new();
                for t in &tracks {
                    assert!(valid_key(&t.key) && valid_ext(&t.ext));
                    assert!(keys.insert(t.key.clone()));
                    assert!(t.track_id > 0 && t.size > 0);
                    assert!(t.title.chars().count() <= MAX_TEXT);
                    assert!(t.duration_secs.is_finite());
                }
            }
        }
    }
}
