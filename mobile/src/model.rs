//! Typed records for the app, parsed leniently from LMS answers: every LMS field may be
//! missing, and numbers often arrive as strings. Text is kept as plain text (control
//! characters removed, length capped) and is only ever displayed, never interpreted.

use serde_json::Value;

use crate::art::{cover_key, image_key};
use crate::commands::is_player_id;

pub use crate::commands::{
    AlbumSort, ArtistRole, InfoKind, LibraryFilter, Placement, RepeatMode, Selection, ShuffleMode,
};

const MAX_TEXT: usize = 1024;

/// Display text: control characters removed, at most 1024 characters.
pub(crate) fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect()
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(clean_text(s)).filter(|s| !s.trim().is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// An integer that may be sent as a number or a string. "12,34" (LMS lists) reads as 12.
pub(crate) fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.split(',').next()?.trim().parse().ok(),
        _ => None,
    }
}

pub(crate) fn float_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite())
}

fn positive_id(v: &Value) -> Option<i64> {
    int_of(v).filter(|i| *i > 0 && *i < 1_000_000_000_000_000)
}

fn loop_of<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| i.is_object())
}

fn total_of(v: &Value, items: usize) -> u32 {
    int_of(&v["count"])
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(items as u32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UserRole {
    Admin,
    Listener,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct User {
    pub username: String,
    pub role: UserRole,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Device {
    pub id: i64,
    pub name: String,
    /// This device's own LMS player id (used once the phone plays itself).
    pub player_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SignedIn {
    pub user: User,
    pub device: Device,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountInfo {
    pub username: String,
    pub is_admin: bool,
    pub two_step_enabled: bool,
    pub downloads_enabled: bool,
    /// The server lets users change their own password (`features.change_password` in
    /// `GET /v1/me`); false on older gateways.
    pub can_change_password: bool,
    /// The server lets users see and sign out their own devices ("Your devices";
    /// `features.devices` in `GET /v1/me`); false on older gateways.
    #[uniffi(default = false)]
    pub can_manage_devices: bool,
}

/// One of this user's signed-in devices, for "Your devices"
/// ([`crate::Session::my_devices`]).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MyDevice {
    pub id: i64,
    pub name: String,
    /// `android`, `pwa` or `browser`, if the device said.
    pub kind: Option<String>,
    /// E.g. "Pixel 8" or "Firefox on Linux", if the device said.
    pub platform: Option<String>,
    /// When it first signed in (Unix seconds), if known.
    pub created_at: Option<i64>,
    /// When it was last used (Unix seconds, to the minute), if known.
    pub last_seen: Option<i64>,
    /// The device asking: this one.
    pub current: bool,
}

/// One of this user's other devices, still signed in after a password change.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OwnDevice {
    pub id: i64,
    pub name: String,
    /// E.g. "Pixel 8" or "Firefox on Linux", if the device said.
    pub platform: Option<String>,
    /// When it was last seen (Unix seconds), if known.
    pub last_seen: Option<i64>,
}

/// What [`crate::Session::change_password`] did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PasswordChanged {
    /// How many other devices were signed out with it.
    pub signed_out: u32,
    /// The user's other devices that are still signed in.
    pub other_devices: Vec<OwnDevice>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<i64>,
    pub year: Option<i32>,
    /// Artwork key for [`crate::Art::fetch`], if the album has a cover.
    pub artwork: Option<String>,
    /// A various-artists compilation.
    pub compilation: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AlbumPage {
    pub total: u32,
    pub items: Vec<Album>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Artist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ArtistPage {
    pub total: u32,
    pub items: Vec<Artist>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Genre {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GenrePage {
    pub total: u32,
    pub items: Vec<Genre>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Track {
    /// Library id; `None` for remote streams (radio).
    pub id: Option<i64>,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<i64>,
    pub artist_id: Option<i64>,
    pub duration_secs: Option<f64>,
    pub track_number: Option<u32>,
    pub artwork: Option<String>,
    /// Position in the player's queue (queue listings only).
    pub queue_index: Option<u32>,
    /// Disc number and count, for multi-disc albums.
    pub disc_number: Option<u32>,
    pub disc_count: Option<u32>,
    pub genre: Option<String>,
    /// The file's format, e.g. "FLAC 24/96" or "MP3".
    pub format: Option<String>,
    pub year: Option<i32>,
    /// The quality badge for Now playing ("FLAC 24/96 · Hi-Res"), when LMS gave a type.
    #[uniffi(default = None)]
    pub quality: Option<Quality>,
}

/// The quality badge (Wave 1 §5): codec and resolution, and whether it is Hi-Res (more than
/// 16-bit or more than 48 kHz, on lossless sources only). The web app has the same rules
/// (web/app/src/lib/lms/quality.ts) and the tests share vectors.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Quality {
    /// "FLAC", "MP3", …
    pub codec: String,
    /// "FLAC 24/96", "FLAC 44.1 kHz", "MP3 320", "DSD64", "Opus".
    pub label: String,
    pub lossless: bool,
    pub hi_res: bool,
    pub sample_rate: Option<u32>,
    pub bits: Option<u32>,
    pub kbps: Option<u32>,
}

/// Players playing in step: their ids (two or more), as LMS lists them.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncGroup {
    pub player_ids: Vec<String>,
}

/// A page of years that have albums (0 is "no year").
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct YearPage {
    pub total: u32,
    pub items: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FolderKind {
    Folder,
    Track,
    Playlist,
    Other,
}

/// A music folder entry. Never a path: LMS's `url` is stripped by the gateway, and a name
/// that still looks like a path is cut to its last part.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FolderEntry {
    pub id: i64,
    pub name: String,
    pub kind: FolderKind,
    pub artwork: Option<String>,
    /// As LMS shows it ("3:07"), if well-formed.
    pub duration: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FolderPage {
    pub total: u32,
    pub items: Vec<FolderEntry>,
}

/// An LMS virtual library ("Libraries" in My Music).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VirtualLibrary {
    pub id: String,
    pub name: String,
}

/// A favourite: a folder of favourites, or something to play (a station, a track, …).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Favourite {
    /// LMS item id (checked), for opening or playing it.
    pub id: String,
    pub name: String,
    pub artwork: Option<String>,
    pub is_folder: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FavouritePage {
    pub title: Option<String>,
    pub total: u32,
    pub items: Vec<Favourite>,
}

/// A saved playlist.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SavedPlaylist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlaylistPage {
    pub total: u32,
    pub items: Vec<SavedPlaylist>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    /// How many of each LMS found (at least as many as listed).
    #[uniffi(default = 0)]
    pub artists_total: u32,
    #[uniffi(default = 0)]
    pub albums_total: u32,
    #[uniffi(default = 0)]
    pub tracks_total: u32,
    /// The "Top result": an exact (or else prefix) name match, preferring an artist, then an
    /// album, then a track (see [`top_result`]).
    #[uniffi(default = None)]
    pub top: Option<TopResult>,
}

/// What kind of result a "Top result" is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SearchKind {
    Artist,
    Album,
    Track,
}

/// The "Top result" of a search: the result at `index` in the list of its `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TopResult {
    pub kind: SearchKind,
    pub index: u32,
}

/// A page of tracks (search, paged by type).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrackPage {
    pub total: u32,
    pub items: Vec<Track>,
}

/// One entry of an A–Z index: the list's items starting with `key` are the `count` items
/// from `offset` on (LMS's `indexList`, counts added up in list order).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct IndexEntry {
    pub key: String,
    pub offset: u32,
    pub count: u32,
}

/// Someone credited on a track, in a role. Without an id (LMS's names and ids didn't line
/// up) the name is shown but can't be followed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Credit {
    pub role: ArtistRole,
    pub name: String,
    pub artist_id: Option<i64>,
}

/// How many albums an artist has in a role ("Appears as: Composer (12)").
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RoleCount {
    pub role: ArtistRole,
    pub albums: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub connected: bool,
    pub powered: bool,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlaybackMode {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlayerStatus {
    pub name: String,
    pub connected: bool,
    pub powered: bool,
    pub mode: PlaybackMode,
    /// Position in the current track, in seconds.
    pub elapsed_secs: f64,
    pub duration_secs: Option<f64>,
    /// 0–100. LMS reports a negative volume while muted; this is its magnitude.
    pub volume: u32,
    pub muted: bool,
    pub shuffle: ShuffleMode,
    pub repeat: RepeatMode,
    pub queue_index: Option<u32>,
    pub queue_length: u32,
    pub can_seek: bool,
    pub current: Option<Track>,
    /// The track after the current one ("Up next"), if one is queued and plays next.
    #[uniffi(default = None)]
    pub next: Option<Track>,
    /// Seconds left on the sleep timer, while one is set.
    #[uniffi(default = None)]
    pub sleep_secs: Option<f64>,
    /// LMS's "Output level is fixed at 100%" (`digitalVolumeControl` 0) is set for this
    /// player: LMS plays it at full level and its volume should not be offered as a slider.
    #[uniffi(default = false)]
    pub fixed_level: bool,
}

pub(crate) fn album_from(v: &Value) -> Option<Album> {
    let id = positive_id(&v["id"]).or_else(|| positive_id(&v["album_id"]))?;
    Some(Album {
        id,
        title: text_of(&v["album"]).unwrap_or_default(),
        artist: text_of(&v["artist"]),
        artist_id: positive_id(&v["artist_id"]),
        year: int_of(&v["year"])
            .filter(|y| *y > 0 && *y < 10_000)
            .map(|y| y as i32),
        artwork: v
            .get("artwork_track_id")
            .and_then(cover_key)
            .or_else(|| v.get("artwork_url").and_then(image_key)),
        compilation: int_of(&v["compilation"]) == Some(1),
    })
}

pub(crate) fn albums_from(v: &Value) -> AlbumPage {
    let items: Vec<Album> = loop_of(v, "albums_loop").filter_map(album_from).collect();
    AlbumPage {
        total: total_of(v, items.len()),
        items,
    }
}

pub(crate) fn artists_from(v: &Value) -> ArtistPage {
    let items: Vec<Artist> = loop_of(v, "artists_loop")
        .filter_map(|a| {
            Some(Artist {
                id: positive_id(&a["id"])?,
                name: text_of(&a["artist"]).unwrap_or_default(),
            })
        })
        .collect();
    ArtistPage {
        total: total_of(v, items.len()),
        items,
    }
}

pub(crate) fn genres_from(v: &Value) -> GenrePage {
    let items: Vec<Genre> = loop_of(v, "genres_loop")
        .filter_map(|g| {
            Some(Genre {
                id: positive_id(&g["id"])?,
                name: text_of(&g["genre"]).unwrap_or_default(),
            })
        })
        .collect();
    GenrePage {
        total: total_of(v, items.len()),
        items,
    }
}

pub(crate) fn track_from(v: &Value) -> Track {
    let remote = int_of(&v["remote"]) == Some(1);
    Track {
        id: if remote { None } else { positive_id(&v["id"]) },
        title: text_of(&v["title"])
            .or_else(|| text_of(&v["remote_title"]))
            .unwrap_or_default(),
        artist: text_of(&v["artist"]),
        album: text_of(&v["album"]),
        album_id: positive_id(&v["album_id"]),
        artist_id: positive_id(&v["artist_id"]),
        duration_secs: float_of(&v["duration"]).filter(|d| *d > 0.0),
        track_number: int_of(&v["tracknum"]).and_then(|n| u32::try_from(n).ok()),
        artwork: v
            .get("coverid")
            .and_then(cover_key)
            .or_else(|| v.get("artwork_track_id").and_then(cover_key))
            .or_else(|| v.get("artwork_url").and_then(image_key))
            .or_else(|| {
                // A library track's own id names its cover.
                (!remote).then(|| v.get("id").and_then(cover_key)).flatten()
            }),
        queue_index: int_of(&v["playlist index"]).and_then(|n| u32::try_from(n).ok()),
        disc_number: small_uint(&v["disc"]),
        disc_count: small_uint(&v["disccount"]),
        genre: text_of(&v["genre"]),
        format: format_of(v),
        year: int_of(&v["year"])
            .filter(|y| *y > 0 && *y < 10_000)
            .map(|y| y as i32),
        quality: quality_of(v),
    }
}

/// The quality badge for a track's format details, or `None` without a usable type.
pub(crate) fn quality_of(v: &Value) -> Option<Quality> {
    let kind = v["type"].as_str()?.trim().to_ascii_lowercase();
    if kind.is_empty() || kind.len() > 12 {
        return None;
    }
    let (codec, lossless) = match kind.as_str() {
        "flc" | "flac" => ("FLAC".to_string(), true),
        "alc" | "alac" => ("ALAC".into(), true),
        "wav" => ("WAV".into(), true),
        "aif" | "aiff" => ("AIFF".into(), true),
        "pcm" => ("PCM".into(), true),
        "ape" => ("APE".into(), true),
        "wvp" => ("WavPack".into(), true),
        "wmal" => ("WMA Lossless".into(), true),
        "dsf" | "dff" => ("DSD".into(), true),
        "mp3" => ("MP3".into(), false),
        "aac" | "mp4" | "m4a" => ("AAC".into(), false),
        "ogg" => ("Ogg Vorbis".into(), false),
        "ops" | "opus" => ("Opus".into(), false),
        "wma" | "wmap" => ("WMA".into(), false),
        "mpc" => ("Musepack".into(), false),
        k if (1..=6).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            (k.to_ascii_uppercase(), false)
        }
        _ => return None,
    };
    let rate = whole_in(&v["samplerate"], 1000, 50_000_000);
    let bits = whole_in(&v["samplesize"], 1, 64);
    let kbps = kbps_of(&v["bitrate"]);
    let label = match (codec.as_str(), lossless, bits, rate, kbps) {
        ("DSD", _, _, Some(r), _) => format!("DSD{}", (r as f64 / 44_100.0).round() as u32),
        (_, true, Some(b), Some(r), _) => format!("{codec} {b}/{}", khz(r as i64)),
        (_, true, None, Some(r), _) => format!("{codec} {} kHz", khz(r as i64)),
        (_, false, _, _, Some(k)) => format!("{codec} {k}"),
        _ => codec.clone(),
    };
    let hi_res = lossless && (bits.is_some_and(|b| b > 16) || rate.is_some_and(|r| r > 48_000));
    Some(Quality {
        codec,
        label,
        lossless,
        hi_res,
        sample_rate: rate,
        bits,
        kbps,
    })
}

/// A whole number from LMS (a number or digits, optionally with a fraction) within
/// `lo..=hi`, rounded; anything else is `None`.
fn whole_in(v: &Value, lo: u32, hi: u32) -> Option<u32> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let t = s.trim();
            let (int, frac) = t.split_once('.').unwrap_or((t, ""));
            let ok = (1..=9).contains(&int.len())
                && int.bytes().all(|b| b.is_ascii_digit())
                && frac.bytes().all(|b| b.is_ascii_digit())
                && !(t.contains('.') && frac.is_empty());
            if !ok {
                return None;
            }
            t.parse::<f64>().ok()?
        }
        _ => return None,
    };
    (n.is_finite() && n >= lo as f64 && n <= hi as f64).then(|| n.round() as u32)
}

/// LMS's bit rate ("320kbps", "873kbps VBR", "320000") in kbps.
fn kbps_of(v: &Value) -> Option<u32> {
    let n = match v {
        Value::Number(n) => {
            let x = n.as_f64()?;
            if x >= 8000.0 { x / 1000.0 } else { x }
        }
        Value::String(s) if s.len() <= 40 => {
            let t = s.trim_start();
            let digits: String = t
                .bytes()
                .take_while(u8::is_ascii_digit)
                .map(char::from)
                .collect();
            if digits.is_empty() || digits.len() > 7 {
                return None;
            }
            let rest =
                t[digits.len()..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
            let rest = rest.trim_start().to_ascii_lowercase();
            let n: f64 = digits.parse().ok()?;
            if rest.starts_with("kbps") {
                n
            } else if rest.starts_with("bps") || (rest.is_empty() && n >= 8000.0) {
                n / 1000.0
            } else if rest.is_empty() {
                n
            } else {
                return None;
            }
        }
        _ => return None,
    };
    (n.is_finite() && (8.0..=100_000.0).contains(&n)).then(|| n.round() as u32)
}

/// A count or number LMS lists (disc, disc count): 1–999.
fn small_uint(v: &Value) -> Option<u32> {
    int_of(v)
        .filter(|n| (1..1000).contains(n))
        .map(|n| n as u32)
}

/// "FLAC 24/96", "ALAC 16/44.1" or "MP3", from LMS's content type, sample size and rate.
pub(crate) fn format_of(v: &Value) -> Option<String> {
    let kind = v["type"].as_str()?.trim().to_ascii_lowercase();
    let name = match kind.as_str() {
        "flc" | "flac" => "FLAC".to_string(),
        "alc" => "ALAC".into(),
        "mp3" => "MP3".into(),
        "aac" | "mp4" | "m4a" => "AAC".into(),
        "wav" => "WAV".into(),
        "aif" | "aiff" => "AIFF".into(),
        "ogg" => "Ogg Vorbis".into(),
        "ops" | "opus" => "Opus".into(),
        "wma" | "wmal" | "wmap" => "WMA".into(),
        "dsf" | "dff" => "DSD".into(),
        "ape" => "APE".into(),
        "wvp" => "WavPack".into(),
        "mpc" => "Musepack".into(),
        k if (1..=6).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            k.to_ascii_uppercase()
        }
        _ => return None,
    };
    let bits = int_of(&v["samplesize"]).filter(|b| (1..=64).contains(b));
    let rate = int_of(&v["samplerate"]).filter(|r| (1000..=1_000_000).contains(r));
    Some(match (bits, rate) {
        (Some(b), Some(r)) => format!("{name} {b}/{}", khz(r)),
        (None, Some(r)) => format!("{name} {} kHz", khz(r)),
        _ => name,
    })
}

/// 44100 → "44.1", 96000 → "96".
fn khz(hz: i64) -> String {
    if hz % 1000 == 0 {
        (hz / 1000).to_string()
    } else {
        format!("{:.1}", hz as f64 / 1000.0)
    }
}

/// Sync groups: well-formed, distinct player ids only; groups of one and players in two
/// groups (nonsense) are left out.
pub(crate) fn sync_groups_from(v: &Value) -> Vec<SyncGroup> {
    let mut out: Vec<SyncGroup> = Vec::new();
    for g in loop_of(v, "syncgroups_loop").take(100) {
        let Some(list) = g["sync_members"].as_str().filter(|s| s.len() <= 4096) else {
            continue;
        };
        let mut ids: Vec<String> = Vec::new();
        for raw in list.split(',') {
            let id = raw.trim().to_ascii_lowercase();
            if is_player_id(&id) && !ids.contains(&id) {
                ids.push(id);
            }
        }
        let taken = out
            .iter()
            .any(|x| x.player_ids.iter().any(|p| ids.contains(p)));
        if ids.len() > 1 && !taken {
            out.push(SyncGroup { player_ids: ids });
        }
    }
    out
}

pub(crate) fn years_from(v: &Value) -> YearPage {
    let items: Vec<u32> = loop_of(v, "years_loop")
        .filter_map(|y| int_of(&y["year"]))
        .filter(|y| (0..10_000).contains(y))
        .map(|y| y as u32)
        .collect();
    YearPage {
        total: total_of(v, items.len()),
        items,
    }
}

/// A name that is (or contains) a filesystem path: shown as its last part only.
fn base_name(s: &str) -> String {
    let b = s.as_bytes();
    let absolute = s.starts_with(['/', '\\'])
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'/' | b'\\'));
    if absolute {
        s.split(['/', '\\'])
            .rfind(|p| !p.is_empty())
            .unwrap_or("")
            .to_string()
    } else {
        s.to_string()
    }
}

fn folder_duration(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let mut parts = s.split(':');
    let first = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    let ok = (1..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_digit())
        && (1..=2).contains(&rest.len())
        && rest
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| s.to_string())
}

pub(crate) fn folder_from(v: &Value) -> FolderPage {
    let items: Vec<FolderEntry> = loop_of(v, "folder_loop")
        .filter_map(|x| {
            let id = positive_id(&x["id"])?;
            let raw = text_of(&x["title"]).or_else(|| text_of(&x["filename"]));
            let name = raw.map(|n| base_name(&n)).unwrap_or_default();
            let kind = match x["type"].as_str() {
                Some("folder") => FolderKind::Folder,
                Some("track") => FolderKind::Track,
                Some("playlist") => FolderKind::Playlist,
                _ => FolderKind::Other,
            };
            Some(FolderEntry {
                id,
                name,
                kind,
                artwork: (kind == FolderKind::Track)
                    .then(|| x.get("coverid").and_then(cover_key))
                    .flatten(),
                duration: (kind == FolderKind::Track)
                    .then(|| folder_duration(&x["duration"]))
                    .flatten(),
            })
        })
        .collect();
    FolderPage {
        total: total_of(v, items.len()),
        items,
    }
}

pub(crate) fn libraries_from(v: &Value) -> Vec<VirtualLibrary> {
    let mut out: Vec<VirtualLibrary> = Vec::new();
    for x in loop_of(v, "folder_loop") {
        let Some(id) = x["id"]
            .as_str()
            .filter(|s| crate::commands::library_id(s).is_ok())
        else {
            continue;
        };
        if out.iter().any(|l| l.id == id) {
            continue;
        }
        let name = text_of(&x["name"])
            .map(|n| n.trim().chars().take(200).collect())
            .unwrap_or_else(|| "Library".to_string());
        out.push(VirtualLibrary {
            id: id.to_string(),
            name,
        });
    }
    out
}

pub(crate) fn favourites_from(v: &Value) -> FavouritePage {
    let items: Vec<Favourite> = loop_of(v, "loop_loop")
        .filter_map(|x| {
            let id = x["id"]
                .as_str()
                .filter(|s| crate::commands::item_id(s).is_ok())?;
            let has_items = int_of(&x["hasitems"]) == Some(1);
            let audio = int_of(&x["isaudio"]) == Some(1);
            Some(Favourite {
                id: id.to_string(),
                name: text_of(&x["name"]).unwrap_or_default(),
                artwork: x
                    .get("image")
                    .and_then(image_key)
                    .or_else(|| x.get("icon").and_then(image_key)),
                is_folder: has_items && !audio,
            })
        })
        .collect();
    FavouritePage {
        title: text_of(&v["title"]),
        total: total_of(v, items.len()),
        items,
    }
}

pub(crate) fn playlists_from(v: &Value) -> PlaylistPage {
    let items: Vec<SavedPlaylist> = loop_of(v, "playlists_loop")
        .filter_map(|p| {
            Some(SavedPlaylist {
                id: positive_id(&p["id"])?,
                name: text_of(&p["playlist"]).unwrap_or_default(),
            })
        })
        .collect();
    PlaylistPage {
        total: total_of(v, items.len()),
        items,
    }
}

pub(crate) fn tracks_from(v: &Value, key: &str) -> Vec<Track> {
    loop_of(v, key).map(track_from).collect()
}

/// Most rows of each kind taken from one search answer.
const MAX_SEARCH_ROWS: usize = 500;

/// A count LMS reports (`*_count`), as a `u32`; never fewer than the rows listed.
fn count_of(v: &Value, key: &str, listed: usize) -> u32 {
    let listed = u32::try_from(listed).unwrap_or(u32::MAX);
    int_of(&v[key])
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
        .max(listed)
}

/// LMS's `search` answer (`extended:1` adds counts and, depending on the version, more
/// detail per row; everything beyond the id and name is optional).
pub(crate) fn search_from(v: &Value) -> SearchResults {
    let artists: Vec<Artist> = loop_of(v, "contributors_loop")
        .take(MAX_SEARCH_ROWS)
        .filter_map(|a| {
            Some(Artist {
                id: positive_id(&a["contributor_id"]).or_else(|| positive_id(&a["id"]))?,
                name: text_of(&a["contributor"])
                    .or_else(|| text_of(&a["artist"]))
                    .unwrap_or_default(),
            })
        })
        .collect();
    let albums: Vec<Album> = loop_of(v, "albums_loop")
        .take(MAX_SEARCH_ROWS)
        .filter_map(|a| {
            Some(Album {
                id: positive_id(&a["album_id"]).or_else(|| positive_id(&a["id"]))?,
                title: text_of(&a["album"]).unwrap_or_default(),
                artist: text_of(&a["artist"]),
                artist_id: positive_id(&a["artist_id"]),
                year: int_of(&a["year"])
                    .filter(|y| *y > 0 && *y < 10_000)
                    .map(|y| y as i32),
                artwork: a
                    .get("artwork_track_id")
                    .and_then(cover_key)
                    .or_else(|| a.get("artwork").and_then(cover_key))
                    .or_else(|| a.get("coverid").and_then(cover_key)),
                compilation: int_of(&a["compilation"]) == Some(1),
            })
        })
        .collect();
    let tracks: Vec<Track> = loop_of(v, "tracks_loop")
        .take(MAX_SEARCH_ROWS)
        .filter_map(|t| {
            let id = positive_id(&t["track_id"]).or_else(|| positive_id(&t["id"]))?;
            Some(Track {
                id: Some(id),
                title: text_of(&t["track"])
                    .or_else(|| text_of(&t["title"]))
                    .unwrap_or_default(),
                artist: text_of(&t["artist"]),
                album: text_of(&t["album"]),
                album_id: positive_id(&t["album_id"]),
                artist_id: positive_id(&t["artist_id"]),
                duration_secs: float_of(&t["duration"]).filter(|d| *d > 0.0),
                track_number: int_of(&t["tracknum"]).and_then(|n| u32::try_from(n).ok()),
                artwork: t
                    .get("coverid")
                    .and_then(cover_key)
                    .or_else(|| cover_key(&Value::from(id))),
                queue_index: None,
                disc_number: None,
                disc_count: None,
                genre: None,
                format: None,
                year: None,
                quality: None,
            })
        })
        .collect();
    SearchResults {
        artists_total: count_of(v, "contributors_count", artists.len()),
        albums_total: count_of(v, "albums_count", albums.len()),
        tracks_total: count_of(v, "tracks_count", tracks.len()),
        top: None,
        artists,
        albums,
        tracks,
    }
}

/// A name or term as compared for the "Top result": case and spacing don't matter.
fn fold(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The "Top result" for `term`: an exact name match, or else one the name starts with;
/// an artist before an album before a track. None if nothing matches either way.
pub fn top_result(
    term: &str,
    artists: &[Artist],
    albums: &[Album],
    tracks: &[Track],
) -> Option<TopResult> {
    let t = fold(term);
    if t.is_empty() {
        return None;
    }
    let lists: [(SearchKind, Vec<String>); 3] = [
        (
            SearchKind::Artist,
            artists.iter().map(|a| fold(&a.name)).collect(),
        ),
        (
            SearchKind::Album,
            albums.iter().map(|a| fold(&a.title)).collect(),
        ),
        (
            SearchKind::Track,
            tracks.iter().map(|x| fold(&x.title)).collect(),
        ),
    ];
    let exact = |n: &String| *n == t;
    let prefix = |n: &String| n.starts_with(&t);
    for test in [&exact as &dyn Fn(&String) -> bool, &prefix] {
        for (kind, names) in &lists {
            if let Some(i) = names.iter().position(test) {
                return Some(TopResult {
                    kind: *kind,
                    index: u32::try_from(i).ok()?,
                });
            }
        }
    }
    None
}

/// Most entries read from an A–Z index (LMS has one per first character).
const MAX_INDEX: usize = 1000;

/// LMS's `indexList` (`tags:Z`): `[first character, count]` pairs in list order, made into
/// offsets. Reading stops at the first malformed pair, since every later offset would be
/// wrong; empty keys read as "#".
pub(crate) fn index_from(v: &Value) -> Vec<IndexEntry> {
    let Some(list) = v.get("indexList").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut offset: u32 = 0;
    for e in list.iter().take(MAX_INDEX) {
        let Some(p) = e.as_array().filter(|p| p.len() == 2) else {
            break;
        };
        let (k, n) = (&p[0], &p[1]);
        let key = match k {
            Value::String(s) => clean_text(s).trim().chars().take(4).collect::<String>(),
            Value::Number(n) => n.to_string().chars().take(4).collect(),
            Value::Null => String::new(),
            _ => break,
        };
        let Some(count) = int_of(n).and_then(|c| u32::try_from(c).ok()) else {
            break;
        };
        if count == 0 {
            continue;
        }
        // The end must fit too, or the entry would point past anything LMS can list.
        let Some(end) = offset.checked_add(count) else {
            break;
        };
        out.push(IndexEntry {
            key: if key.is_empty() { "#".into() } else { key },
            offset,
            count,
        });
        offset = end;
    }
    out
}

/// The roles credited on a track, in the order shown, with LMS's field names.
const CREDIT_ROLES: [(ArtistRole, &str); 6] = [
    (ArtistRole::AlbumArtist, "albumartist"),
    (ArtistRole::Artist, "artist"),
    (ArtistRole::TrackArtist, "trackartist"),
    (ArtistRole::Composer, "composer"),
    (ArtistRole::Conductor, "conductor"),
    (ArtistRole::Band, "band"),
];
/// Most people read per role, and in all.
const MAX_PER_ROLE: usize = 50;
const MAX_CREDITS: usize = 200;

/// A role's ids (`"12,34"` or a number): all of them, or None if any is malformed.
fn credit_ids(v: &Value) -> Option<Vec<i64>> {
    match v {
        Value::Number(_) => positive_id(v).map(|i| vec![i]),
        Value::String(s) => s
            .split(',')
            .take(MAX_PER_ROLE + 1)
            .map(|p| {
                let p = p.trim();
                (!p.is_empty() && p.len() <= 15 && p.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| p.parse::<i64>().ok())
                    .flatten()
                    .filter(|i| *i > 0)
            })
            .collect(),
        _ => None,
    }
}

/// Everyone credited on a track (`titles … tags:AS`), role by role. LMS joins a role's
/// names with ", " and its ids with ","; where they don't line up (a name with a comma in
/// it), the role's names are kept together, without an id.
pub(crate) fn credits_from(v: &Value) -> Vec<Credit> {
    let Some(t) = loop_of(v, "titles_loop").next() else {
        return Vec::new();
    };
    let mut out: Vec<Credit> = Vec::new();
    for (role, field) in CREDIT_ROLES {
        let Some(names) = text_of(&t[field]) else {
            continue;
        };
        let ids = credit_ids(&t[format!("{field}_ids").as_str()]).unwrap_or_default();
        let parts: Vec<&str> = names.split(", ").map(str::trim).collect();
        let pairs: Vec<(String, Option<i64>)> = if ids.len() == 1 {
            vec![(names.trim().to_string(), Some(ids[0]))]
        } else if ids.len() > 1 && ids.len() == parts.len() && ids.len() <= MAX_PER_ROLE {
            parts
                .into_iter()
                .zip(ids)
                .map(|(n, i)| (n.to_string(), Some(i)))
                .collect()
        } else {
            vec![(names.trim().to_string(), None)]
        };
        for (name, artist_id) in pairs.into_iter().take(MAX_PER_ROLE) {
            if name.is_empty() || out.len() >= MAX_CREDITS {
                continue;
            }
            let dup = out
                .iter()
                .any(|c| c.role == role && c.artist_id == artist_id && c.name == name);
            if !dup {
                out.push(Credit {
                    role,
                    name,
                    artist_id,
                });
            }
        }
    }
    out
}

pub(crate) fn players_from(v: &Value) -> Vec<Player> {
    loop_of(v, "players_loop")
        .filter_map(|p| {
            let id = p["playerid"].as_str().filter(|s| is_player_id(s))?;
            Some(Player {
                id: id.to_ascii_lowercase(),
                name: text_of(&p["name"]).unwrap_or_else(|| id.to_string()),
                model: text_of(&p["modelname"]).or_else(|| text_of(&p["model"])),
                connected: int_of(&p["connected"]) == Some(1),
                powered: int_of(&p["power"]) == Some(1),
                playing: int_of(&p["isplaying"]) == Some(1),
            })
        })
        .collect()
}

pub(crate) fn status_from(v: &Value) -> PlayerStatus {
    let raw_volume = int_of(&v["mixer volume"]).unwrap_or(0);
    let current = loop_of(v, "playlist_loop")
        .next()
        .map(track_from)
        .map(|mut t| {
            if t.title.is_empty() {
                t.title = text_of(&v["current_title"]).unwrap_or_default();
            }
            t
        });
    PlayerStatus {
        name: text_of(&v["player_name"]).unwrap_or_default(),
        connected: int_of(&v["player_connected"]) == Some(1),
        powered: int_of(&v["power"]) == Some(1),
        mode: match v["mode"].as_str() {
            Some("play") => PlaybackMode::Playing,
            Some("pause") => PlaybackMode::Paused,
            _ => PlaybackMode::Stopped,
        },
        elapsed_secs: float_of(&v["time"]).unwrap_or(0.0).max(0.0),
        duration_secs: float_of(&v["duration"]).filter(|d| *d > 0.0),
        volume: raw_volume.unsigned_abs().min(100) as u32,
        muted: raw_volume < 0,
        shuffle: ShuffleMode::from_lms(int_of(&v["playlist shuffle"]).unwrap_or(0)),
        repeat: RepeatMode::from_lms(int_of(&v["playlist repeat"]).unwrap_or(0)),
        queue_index: int_of(&v["playlist_cur_index"]).and_then(|n| u32::try_from(n).ok()),
        queue_length: int_of(&v["playlist_tracks"])
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        can_seek: int_of(&v["can_seek"]).is_none_or(|c| c == 1) && int_of(&v["remote"]) != Some(1),
        current,
        next: next_of(v),
        sleep_secs: float_of(&v["will_sleep_in"]).filter(|s| s.is_finite() && *s > 0.0),
        fixed_level: int_of(&v["digital_volume_control"]) == Some(0),
    }
}

/// "Up next": the second entry of a `status - 2` answer, if it is the track at the current
/// index + 1. Not under "repeat this track" (the same track plays again).
fn next_of(v: &Value) -> Option<Track> {
    if int_of(&v["playlist repeat"]) == Some(1) {
        return None;
    }
    let cur = int_of(&v["playlist_cur_index"]).filter(|i| *i >= 0)?;
    let next = v["playlist_loop"].as_array()?.get(1)?;
    if !next.is_object() {
        return None;
    }
    if let Some(at) = next.get("playlist index").filter(|x| !x.is_null())
        && int_of(at) != Some(cur + 1)
    {
        return None;
    }
    let mut t = track_from(next);
    t.queue_index = u32::try_from(cur + 1).ok();
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The quality badge: the web app's vectors (web/app/tests/quality.test.ts).
    #[test]
    fn quality_badge_and_the_hi_res_rule() {
        let cases = [
            (
                json!({"type": "flc", "samplesize": "24", "samplerate": "96000"}),
                "FLAC 24/96",
                true,
            ),
            (
                json!({"type": "flc", "samplesize": "16", "samplerate": "44100"}),
                "FLAC 16/44.1",
                false,
            ),
            (
                json!({"type": "flc", "samplesize": 16, "samplerate": 48000}),
                "FLAC 16/48",
                false,
            ),
            (
                json!({"type": "flc", "samplesize": "24", "samplerate": "44100"}),
                "FLAC 24/44.1",
                true,
            ),
            (
                json!({"type": "flc", "samplesize": "16", "samplerate": "88200"}),
                "FLAC 16/88.2",
                true,
            ),
            (
                json!({"type": "alc", "samplesize": "24", "samplerate": "192000"}),
                "ALAC 24/192",
                true,
            ),
            (
                json!({"type": "wav", "samplerate": "44100"}),
                "WAV 44.1 kHz",
                false,
            ),
            (
                json!({"type": "aif", "samplerate": "96000"}),
                "AIFF 96 kHz",
                true,
            ),
            (
                json!({"type": "dsf", "samplesize": "1", "samplerate": "2822400"}),
                "DSD64",
                true,
            ),
            (json!({"type": "FLC "}), "FLAC", false),
        ];
        for (v, label, hi_res) in cases {
            let q = quality_of(&v).unwrap();
            assert_eq!(q.label, label, "{v}");
            assert_eq!(q.hi_res, hi_res, "{v}");
            assert!(q.lossless);
        }
        let lossy = [
            (
                json!({"type": "mp3", "bitrate": "320kbps", "samplerate": "44100"}),
                "MP3 320",
            ),
            (json!({"type": "mp3", "bitrate": "873kbps VBR"}), "MP3 873"),
            (json!({"type": "aac", "bitrate": "256000"}), "AAC 256"),
            (json!({"type": "ops", "bitrate": 128}), "Opus 128"),
            (json!({"type": "ogg"}), "Ogg Vorbis"),
        ];
        for (v, label) in lossy {
            let q = quality_of(&v).unwrap();
            assert_eq!(q.label, label, "{v}");
            assert!(!q.lossless && !q.hi_res);
        }
        // High-rate lossy is never Hi-Res.
        let q = quality_of(&json!({"type": "aac", "samplesize": "24", "samplerate": "96000", "bitrate": "512kbps"})).unwrap();
        assert!(!q.hi_res && !q.lossless);
    }

    #[test]
    fn hostile_format_details_give_no_badge_or_a_plain_one() {
        for bad in [
            json!(null),
            json!({}),
            json!({"type": ""}),
            json!({"type": 42}),
            json!({"type": "<script>"}),
            json!({"type": "x".repeat(50)}),
            json!({"type": "a b"}),
            json!({"type": "flac;rm"}),
        ] {
            assert!(quality_of(&bad).is_none(), "{bad}");
        }
        let q =
            quality_of(&json!({"type": "xyz", "samplesize": "24", "samplerate": "96000"})).unwrap();
        assert_eq!(
            (q.label.as_str(), q.lossless, q.hi_res),
            ("XYZ", false, false)
        );
        assert_eq!(
            quality_of(&json!({"type": "flc", "samplesize": "9999", "samplerate": "-1"}))
                .unwrap()
                .label,
            "FLAC"
        );
        assert_eq!(
            quality_of(&json!({"type": "flc", "samplesize": "NaN", "samplerate": "1e9"}))
                .unwrap()
                .label,
            "FLAC"
        );
        assert_eq!(
            quality_of(&json!({"type": "mp3", "bitrate": "<b>320</b>"}))
                .unwrap()
                .label,
            "MP3"
        );
        for b in [
            json!(""),
            json!("fast"),
            json!("kbps"),
            json!("9".repeat(60)),
            json!("-320kbps"),
            json!(-5),
            json!({}),
        ] {
            assert_eq!(kbps_of(&b), None, "{b}");
        }
        // Every label is short plain text.
        for t in [
            "flc", "alc", "wav", "aif", "ape", "wvp", "wmal", "dsf", "dff", "mp3", "aac", "ogg",
            "ops", "wma", "mpc", "zz9",
        ] {
            for bits in [json!(null), json!("16"), json!("24"), json!("32")] {
                for rate in [
                    json!(null),
                    json!("44100"),
                    json!("96000"),
                    json!("352800"),
                    json!("11289600"),
                ] {
                    let q = quality_of(&json!({"type": t, "samplesize": bits, "samplerate": rate, "bitrate": "320kbps"})).unwrap();
                    assert!(
                        q.label.len() <= 40
                            && q.label
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || " ./-".contains(c)),
                        "{}",
                        q.label
                    );
                }
            }
        }
    }

    #[test]
    fn up_next_is_the_track_at_the_current_index_plus_one() {
        let cur = json!({"id": 1, "title": "Now", "playlist index": 3});
        let next = json!({"id": 2, "title": "Next", "playlist index": 4});
        let s = |extra: Value| {
            let mut v =
                json!({"playlist_cur_index": "3", "playlist_loop": [cur.clone(), next.clone()]});
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            status_from(&v)
        };
        let n = s(json!({})).next.unwrap();
        assert_eq!((n.title.as_str(), n.queue_index), ("Next", Some(4)));
        assert_eq!(s(json!({"playlist_loop": [cur.clone()]})).next, None);
        assert_eq!(
            s(json!({"playlist_loop": [cur.clone(), {"id": 2, "playlist index": 9}]})).next,
            None
        );
        assert_eq!(s(json!({"playlist repeat": 1})).next, None);
        assert!(s(json!({"playlist repeat": 2})).next.is_some());
        assert_eq!(s(json!({"playlist_cur_index": "x"})).next, None);
        assert_eq!(s(json!({"playlist_cur_index": -1})).next, None);
        // No index on the entry: trusted to be the next one.
        assert!(
            s(json!({"playlist_loop": [cur.clone(), {"id": 2, "title": "Next"}]}))
                .next
                .is_some()
        );
        for junk in [json!(null), json!(7), json!("x"), json!([])] {
            assert_eq!(s(json!({"playlist_loop": [cur.clone(), junk]})).next, None);
        }
        // The current track is still the first entry.
        assert_eq!(s(json!({})).current.unwrap().title, "Now");
    }

    #[test]
    fn sleep_seconds_come_from_will_sleep_in() {
        let st = |v: Value| status_from(&json!({"will_sleep_in": v}));
        assert_eq!(st(json!(1799.4)).sleep_secs, Some(1799.4));
        assert_eq!(st(json!("60")).sleep_secs, Some(60.0));
        for none in [
            json!(0),
            json!("0"),
            json!(-3),
            json!("x"),
            json!(null),
            json!("inf"),
        ] {
            assert_eq!(st(none.clone()).sleep_secs, None, "{none}");
        }
    }

    #[test]
    fn albums_parse_leniently() {
        let v = json!({"count": "2", "albums_loop": [
            {"id": 7, "album": "Album", "artist": "Artist", "artist_id": "12,13", "year": "1999", "artwork_track_id": "abc12345"},
            {"id": "x"},
            {"id": 0},
            "garbage",
            {"id": 8, "year": 0}
        ]});
        let p = albums_from(&v);
        assert_eq!(p.total, 2);
        assert_eq!(p.items.len(), 2);
        let a = &p.items[0];
        assert_eq!(a.artist_id, Some(12));
        assert_eq!(a.year, Some(1999));
        assert_eq!(a.artwork.as_deref(), Some("music/abc12345/cover"));
        assert_eq!(p.items[1].year, None);
        assert_eq!(p.items[1].artwork, None);
    }

    #[test]
    fn hostile_text_is_kept_as_inert_text() {
        let v = json!({"titles_loop": [{"id": 1, "title": "a\u{0}\r\n<script>", "artist": "", "duration": "NaN"}]});
        let t = &tracks_from(&v, "titles_loop")[0];
        assert_eq!(t.title, "a<script>");
        assert_eq!(t.artist, None);
        assert_eq!(t.duration_secs, None);
        let long = json!({"titles_loop": [{"id": 1, "title": "x".repeat(100_000)}]});
        assert_eq!(tracks_from(&long, "titles_loop")[0].title.len(), MAX_TEXT);
    }

    #[test]
    fn status_reads_lms_shapes() {
        let v = json!({
            "player_name": "Kitchen", "player_connected": 1, "power": 1, "mode": "pause",
            "time": "12.5", "duration": 200, "mixer volume": -40, "playlist shuffle": 2,
            "playlist repeat": "1", "playlist_cur_index": "3", "playlist_tracks": 10,
            "playlist_loop": [{"id": 5, "title": "Song", "coverid": "deadbeef", "playlist index": 3}]
        });
        let s = status_from(&v);
        assert_eq!(s.mode, PlaybackMode::Paused);
        assert_eq!(s.volume, 40);
        assert!(s.muted);
        assert_eq!(s.shuffle, ShuffleMode::Albums);
        assert_eq!(s.repeat, RepeatMode::One);
        assert_eq!(s.queue_index, Some(3));
        assert!(s.can_seek);
        let cur = s.current.unwrap();
        assert_eq!(cur.artwork.as_deref(), Some("music/deadbeef/cover"));
        assert_eq!(cur.queue_index, Some(3));

        let radio = status_from(
            &json!({"mode": "play", "remote": 1, "current_title": "Radio",
            "playlist_loop": [{"id": -123, "remote": 1, "artwork_url": "http://10.0.0.10/x.png"}]}),
        );
        assert!(!radio.can_seek);
        let t = radio.current.unwrap();
        assert_eq!(t.id, None);
        assert_eq!(t.title, "Radio");
        assert!(t.artwork.is_some());
        assert_eq!(status_from(&json!("nonsense")).mode, PlaybackMode::Stopped);
    }

    #[test]
    fn status_reads_the_fixed_output_level() {
        // Absent (older servers, or a status without it) means the volume works as usual.
        assert!(!status_from(&json!({"mode": "play"})).fixed_level);
        assert!(!status_from(&json!({"digital_volume_control": 1})).fixed_level);
        assert!(!status_from(&json!({"digital_volume_control": "1"})).fixed_level);
        assert!(status_from(&json!({"digital_volume_control": 0})).fixed_level);
        assert!(status_from(&json!({"digital_volume_control": "0"})).fixed_level);
        // Anything that is not a number is not taken as "fixed".
        assert!(!status_from(&json!({"digital_volume_control": "x"})).fixed_level);
        assert!(!status_from(&json!({"digital_volume_control": null})).fixed_level);
        assert!(!status_from(&json!({"digital_volume_control": [0]})).fixed_level);
    }

    #[test]
    fn players_need_valid_ids() {
        let v = json!({"players_loop": [
            {"playerid": "AA:BB:CC:DD:EE:01", "name": "One", "connected": 1, "isplaying": "1"},
            {"playerid": "../x", "name": "Bad"},
            {"name": "No id"}
        ]});
        let p = players_from(&v);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, "aa:bb:cc:dd:ee:01");
        assert!(p[0].playing && p[0].connected && !p[0].powered);
    }

    #[test]
    fn tracks_carry_disc_genre_and_format() {
        let v = json!({"titles_loop": [
            {"id": 1, "title": "A", "disc": "2", "disccount": 3, "genre": "Jazz", "type": "flc",
             "samplesize": 24, "samplerate": "96000", "year": 1999, "album_id": "7", "artist_id": 3},
            {"id": 2, "title": "B", "type": "mp3", "samplerate": 44100, "disc": 0},
            {"id": 3, "title": "C", "type": "<script>", "samplesize": 999},
            {"id": 4, "title": "D", "type": "alc", "samplesize": 16, "samplerate": 44100}
        ]});
        let t = tracks_from(&v, "titles_loop");
        assert_eq!(t[0].disc_number, Some(2));
        assert_eq!(t[0].disc_count, Some(3));
        assert_eq!(t[0].genre.as_deref(), Some("Jazz"));
        assert_eq!(t[0].format.as_deref(), Some("FLAC 24/96"));
        assert_eq!(t[0].year, Some(1999));
        assert_eq!(t[0].album_id, Some(7));
        assert_eq!(t[1].format.as_deref(), Some("MP3 44.1 kHz"));
        assert_eq!(t[1].disc_number, None);
        assert_eq!(t[2].format, None);
        assert_eq!(t[3].format.as_deref(), Some("ALAC 16/44.1"));
    }

    #[test]
    fn music_folder_entries_never_show_paths() {
        let v = json!({"count": 6, "folder_loop": [
            {"id": 12, "type": "folder", "title": "Jazz"},
            {"id": 13, "type": "track", "title": "/srv/music/Jazz/01 One.flac", "coverid": "abcdef12", "duration": "3:07"},
            {"id": 14, "type": "track", "filename": "C:\\Music\\x.mp3", "duration": "../../x"},
            {"id": 15, "type": "playlist", "title": "Mix"},
            {"id": "16", "type": "unknown"},
            {"id": 0, "type": "folder", "title": "Bad"},
            {"id": "x"}
        ]});
        let p = folder_from(&v);
        assert_eq!(p.total, 6);
        assert_eq!(p.items.len(), 5);
        assert_eq!(p.items[0].kind, FolderKind::Folder);
        assert_eq!(p.items[1].name, "01 One.flac");
        assert_eq!(p.items[1].duration.as_deref(), Some("3:07"));
        assert_eq!(p.items[1].artwork.as_deref(), Some("music/abcdef12/cover"));
        assert_eq!(p.items[2].name, "x.mp3");
        assert_eq!(p.items[2].duration, None);
        assert_eq!(p.items[3].kind, FolderKind::Playlist);
        assert_eq!(p.items[4].kind, FolderKind::Other);
        // Ordinary names with slashes are names, not paths.
        assert_eq!(base_name("AC/DC"), "AC/DC");
    }

    #[test]
    fn libraries_favourites_playlists_and_years() {
        let libs = libraries_from(&json!({"folder_loop": [
            {"id": "ab12", "name": " Classical "},
            {"id": "ab12", "name": "Duplicate"},
            {"id": "../x", "name": "Bad"},
            {"id": 7, "name": "Number"},
            {"id": "cd34"}
        ]}));
        assert_eq!(
            libs,
            vec![
                VirtualLibrary {
                    id: "ab12".into(),
                    name: "Classical".into()
                },
                VirtualLibrary {
                    id: "cd34".into(),
                    name: "Library".into()
                }
            ]
        );

        let fav = favourites_from(&json!({"title": "Favourites", "count": 3, "loop_loop": [
            {"id": "f0.0", "name": "Radio", "isaudio": 1, "hasitems": 0, "image": "http://radio.example.com/logo.png"},
            {"id": "f0.1", "name": "Folder", "isaudio": 0, "hasitems": 1},
            {"id": "f0 2", "name": "Bad id"},
            {"name": "No id"}
        ]}));
        assert_eq!(fav.items.len(), 2);
        assert!(!fav.items[0].is_folder);
        assert!(
            fav.items[0]
                .artwork
                .as_deref()
                .unwrap()
                .starts_with("imageproxy/")
        );
        assert!(fav.items[1].is_folder);

        let pl = playlists_from(&json!({"count": "2", "playlists_loop": [
            {"id": 5, "playlist": "Road trip"}, {"id": -1, "playlist": "Bad"}
        ]}));
        assert_eq!(pl.total, 2);
        assert_eq!(
            pl.items,
            vec![SavedPlaylist {
                id: 5,
                name: "Road trip".into()
            }]
        );

        let y = years_from(&json!({"count": 4, "years_loop": [
            {"year": 1999}, {"year": "0"}, {"year": 123456}, {"year": "x"}
        ]}));
        assert_eq!(y.items, vec![1999, 0]);
    }

    #[test]
    fn sync_groups_need_valid_distinct_players() {
        let g = sync_groups_from(&json!({"syncgroups_loop": [
            {"sync_members": "AA:BB:CC:DD:EE:01,aa:bb:cc:dd:ee:02,aa:bb:cc:dd:ee:01"},
            {"sync_members": "aa:bb:cc:dd:ee:03"},
            {"sync_members": "aa:bb:cc:dd:ee:02,aa:bb:cc:dd:ee:04"},
            {"sync_members": "../x,aa:bb:cc:dd:ee:05"},
            {"sync_members": 7}
        ]}));
        assert_eq!(
            g,
            vec![SyncGroup {
                player_ids: vec!["aa:bb:cc:dd:ee:01".into(), "aa:bb:cc:dd:ee:02".into()]
            }]
        );
    }

    #[test]
    fn robust_on_arbitrary_json() {
        let samples = [
            json!(null),
            json!(1),
            json!([]),
            json!({"albums_loop": {"a": 1}}),
            json!({"albums_loop": [[], null, 1, {"id": 1e300}]}),
            json!({"count": -5, "artists_loop": [{"id": "9999999999999999999999"}]}),
            json!({"playlist_loop": [{"duration": "inf", "tracknum": -1}], "mixer volume": i64::MIN}),
            json!({"contributors_loop": [{"contributor_id": 3}], "tracks_loop": [{"track_id": "4"}]}),
        ];
        for v in samples {
            let _ = albums_from(&v);
            let _ = artists_from(&v);
            let _ = genres_from(&v);
            let _ = tracks_from(&v, "playlist_loop");
            let _ = search_from(&v);
            let _ = players_from(&v);
            let _ = years_from(&v);
            let _ = folder_from(&v);
            let _ = libraries_from(&v);
            let _ = favourites_from(&v);
            let _ = playlists_from(&v);
            let _ = sync_groups_from(&v);
            let _ = index_from(&v);
            let _ = credits_from(&v);
            let s = status_from(&v);
            assert!(s.volume <= 100);
        }
    }

    #[test]
    fn index_lists_become_offsets() {
        let idx = index_from(
            &json!({"count": 9, "indexList": [["A", 3], ["B", "2"], ["", 1], [7, 0], ["Ö", 3]]}),
        );
        assert_eq!(
            idx,
            vec![
                IndexEntry {
                    key: "A".into(),
                    offset: 0,
                    count: 3
                },
                IndexEntry {
                    key: "B".into(),
                    offset: 3,
                    count: 2
                },
                IndexEntry {
                    key: "#".into(),
                    offset: 5,
                    count: 1
                },
                IndexEntry {
                    key: "Ö".into(),
                    offset: 6,
                    count: 3
                },
            ]
        );
        // A malformed pair ends the index: every offset after it would be wrong.
        for bad in [
            json!(["C", -1]),
            json!(["C"]),
            json!(["C", 1, 2]),
            json!({"C": 1}),
            json!([["C"], 1]),
            json!(["C", "x"]),
        ] {
            let v = json!({"indexList": [["A", 2], bad, ["D", 4]]});
            assert_eq!(index_from(&v).len(), 1, "{v}");
        }
        // Counts that would overflow stop the index rather than wrap.
        let v = json!({"indexList": [["A", u32::MAX - 1], ["B", 1], ["C", 1]]});
        let idx = index_from(&v);
        assert_eq!(idx.len(), 2);
        assert_eq!(idx[1].offset, u32::MAX - 1);
        assert_eq!(
            index_from(&json!({"indexList": [["A", u64::MAX]]})).len(),
            0
        );
        // Keys are display text: control characters gone, short.
        let v = json!({"indexList": [["\u{0}\u{1b}[2JLONGKEY", 1]]});
        assert_eq!(index_from(&v)[0].key, "[2JL");
        // Bounded however long LMS's list is.
        let long: Vec<Value> = (0..5000).map(|i| json!([format!("{i}"), 1])).collect();
        assert_eq!(index_from(&json!({ "indexList": long })).len(), MAX_INDEX);
        assert!(index_from(&json!({"indexList": "A"})).is_empty());
    }

    #[test]
    fn credits_by_role_with_ids_lined_up() {
        let v = json!({"titles_loop": [{
            "id": 7, "title": "Song",
            "albumartist": "Some Band", "albumartist_ids": "11",
            "artist": "Singer One, Singer Two", "artist_ids": "21,22",
            "trackartist": "Guest", "trackartist_ids": 31,
            "composer": "Writer, Jr., Other Writer", "composer_ids": "41,42",
            "conductor": "Maestro", "conductor_ids": "x",
            "band": "", "band_ids": "61"
        }]});
        let c = credits_from(&v);
        let got: Vec<(ArtistRole, &str, Option<i64>)> = c
            .iter()
            .map(|c| (c.role, c.name.as_str(), c.artist_id))
            .collect();
        assert_eq!(
            got,
            vec![
                (ArtistRole::AlbumArtist, "Some Band", Some(11)),
                (ArtistRole::Artist, "Singer One", Some(21)),
                (ArtistRole::Artist, "Singer Two", Some(22)),
                (ArtistRole::TrackArtist, "Guest", Some(31)),
                // Three names for two ids: kept together, not guessed.
                (ArtistRole::Composer, "Writer, Jr., Other Writer", None),
                // A malformed id: shown, but not followable.
                (ArtistRole::Conductor, "Maestro", None),
            ]
        );
        // A single id takes the whole name, commas and all.
        let v = json!({"titles_loop": [{"composer": "Earth, Wind", "composer_ids": "5"}]});
        assert_eq!(credits_from(&v)[0].name, "Earth, Wind");
        assert_eq!(credits_from(&v)[0].artist_id, Some(5));
        // Hostile ids never become ids.
        for bad in ["-1", "0", "1;2", "1,,2", "99999999999999999999", "1 OR 1=1"] {
            let v = json!({"titles_loop": [{"artist": "A, B", "artist_ids": bad}]});
            assert!(
                credits_from(&v).iter().all(|c| c.artist_id.is_none()),
                "{bad}"
            );
        }
        assert!(credits_from(&json!({"titles_loop": []})).is_empty());
        // Bounded however many names a role has.
        let names: Vec<String> = (0..500).map(|i| format!("N{i}")).collect();
        let ids: Vec<String> = (1..=500).map(|i| i.to_string()).collect();
        let v = json!({"titles_loop": [{"artist": names.join(", "), "artist_ids": ids.join(",")}]});
        assert!(credits_from(&v).len() <= MAX_PER_ROLE);
    }

    #[test]
    fn extended_search_rows_and_counts() {
        let v = json!({
            "contributors_count": 12, "albums_count": "3", "tracks_count": -4,
            "contributors_loop": [{"contributor": "Blue Band", "contributor_id": 3}],
            "albums_loop": [{"album": "Blue", "album_id": "7", "artist": "Blue Band", "artwork_track_id": "abcdef12", "year": 1999}],
            "tracks_loop": [
                {"track": "Blue Moon", "track_id": 1, "album": "Blue", "artist": "Blue Band", "duration": "61.5", "coverid": "abcdef12"},
                {"track": "Nobody", "track_id": "x"}
            ]
        });
        let r = search_from(&v);
        assert_eq!(
            (r.artists_total, r.albums_total, r.tracks_total),
            (12, 3, 1)
        );
        assert_eq!(r.albums[0].artist.as_deref(), Some("Blue Band"));
        assert_eq!(r.albums[0].year, Some(1999));
        assert!(r.albums[0].artwork.is_some());
        assert_eq!(r.tracks.len(), 1);
        assert_eq!(r.tracks[0].duration_secs, Some(61.5));
        // The older, plain shape still reads.
        let plain = search_from(&json!({"tracks_loop": [{"track": "One", "track_id": 4}]}));
        assert_eq!(plain.tracks[0].title, "One");
        assert_eq!(plain.tracks_total, 1);
    }

    fn artist(name: &str) -> Artist {
        Artist {
            id: 1,
            name: name.into(),
        }
    }

    fn album(title: &str) -> Album {
        Album {
            id: 1,
            title: title.into(),
            artist: None,
            artist_id: None,
            year: None,
            artwork: None,
            compilation: false,
        }
    }

    fn track(title: &str) -> Track {
        let mut t = track_from(&json!({"id": 1}));
        t.title = title.into();
        t
    }

    #[test]
    fn the_top_result_prefers_exact_then_artist_album_track() {
        let top = |term: &str, ar: &[&str], al: &[&str], tr: &[&str]| {
            top_result(
                term,
                &ar.iter().map(|n| artist(n)).collect::<Vec<_>>(),
                &al.iter().map(|n| album(n)).collect::<Vec<_>>(),
                &tr.iter().map(|n| track(n)).collect::<Vec<_>>(),
            )
        };
        let at = |kind, index| Some(TopResult { kind, index });
        // An exact track beats a prefix artist.
        assert_eq!(
            top("blue", &["Bluegrass Band"], &[], &["Red", "Blue"]),
            at(SearchKind::Track, 1)
        );
        // Exact everywhere: the artist.
        assert_eq!(
            top(" BLUE  ", &["Blue"], &["blue"], &["Blue"]),
            at(SearchKind::Artist, 0)
        );
        // An exact album beats an exact track.
        assert_eq!(
            top("blue", &["Red"], &["Blue"], &["Blue"]),
            at(SearchKind::Album, 0)
        );
        // Prefixes only: artist first.
        assert_eq!(
            top("blu", &["Red", "Blues Trio"], &["Blue"], &[]),
            at(SearchKind::Artist, 1)
        );
        assert_eq!(top("blue", &["Red"], &["Green"], &["True blue"]), None);
        assert_eq!(top("   ", &["Blue"], &[], &[]), None);
    }

    /// A small deterministic generator of arbitrary JSON (xorshift), shaped like LMS's
    /// answers often enough to reach the parsers' inner paths.
    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
            xs[(self.next() % xs.len() as u64) as usize]
        }

        fn value(&mut self, depth: u32) -> Value {
            const KEYS: &[&str] = &[
                "indexList",
                "titles_loop",
                "contributors_loop",
                "albums_loop",
                "tracks_loop",
                "artist",
                "artist_ids",
                "composer",
                "composer_ids",
                "albumartist",
                "albumartist_ids",
                "band_ids",
                "count",
                "tracks_count",
                "album_id",
                "track_id",
                "contributor_id",
                "track",
                "album",
                "duration",
                "coverid",
                "year",
            ];
            match self.next() % if depth > 4 { 5 } else { 8 } {
                0 => Value::Null,
                1 => json!(self.next().is_multiple_of(2)),
                2 => json!(self.next() as i64),
                3 => json!(f64::from_bits(self.next())),
                4 => {
                    let s = self.pick(&["", "A", "12,34", "-1", "x, y", "\u{0}", "1e309", "Ö"]);
                    json!(s.repeat((self.next() % 3) as usize + 1))
                }
                5 | 6 => {
                    let n = self.next() % 6;
                    Value::Array((0..n).map(|_| self.value(depth + 1)).collect())
                }
                _ => {
                    let n = self.next() % 6;
                    let mut m = serde_json::Map::new();
                    for _ in 0..n {
                        let k = self.pick(KEYS).to_string();
                        m.insert(k, self.value(depth + 1));
                    }
                    Value::Object(m)
                }
            }
        }
    }

    /// The new parsers never panic on anything, and keep their bounds.
    #[test]
    fn new_parsers_survive_arbitrary_input() {
        let mut g = Gen(0x9E37_79B9_7F4A_7C15);
        for _ in 0..20_000 {
            let v = g.value(0);
            let idx = index_from(&v);
            assert!(idx.len() <= MAX_INDEX);
            assert!(
                idx.windows(2)
                    .all(|w| w[1].offset >= w[0].offset + w[0].count)
            );
            let c = credits_from(&v);
            assert!(c.len() <= MAX_CREDITS);
            assert!(c.iter().all(|c| c.artist_id.is_none_or(|i| i > 0)));
            let s = search_from(&v);
            assert!(s.tracks_total as usize >= s.tracks.len());
            let _ = top_result("a", &s.artists, &s.albums, &s.tracks);
            // Wrapped where LMS puts them.
            let _ = credits_from(&json!({ "titles_loop": [v.clone()] }));
            let _ = index_from(&json!({ "indexList": v }));
        }
    }
}
