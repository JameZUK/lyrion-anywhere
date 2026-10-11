//! Typed records for the app, parsed leniently from LMS answers: every LMS field may be
//! missing, and numbers often arrive as strings. Text is kept as plain text (control
//! characters removed, length capped) and is only ever displayed, never interpreted.

use serde_json::Value;

use crate::art::{cover_key, image_key};
use crate::commands::is_player_id;

pub use crate::commands::{AlbumSort, Placement, RepeatMode, Selection, ShuffleMode};

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
fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.split(',').next()?.trim().parse().ok(),
        _ => None,
    }
}

fn float_of(v: &Value) -> Option<f64> {
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
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
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
    }
}

pub(crate) fn tracks_from(v: &Value, key: &str) -> Vec<Track> {
    loop_of(v, key).map(track_from).collect()
}

pub(crate) fn search_from(v: &Value) -> SearchResults {
    SearchResults {
        artists: loop_of(v, "contributors_loop")
            .filter_map(|a| {
                Some(Artist {
                    id: positive_id(&a["contributor_id"])?,
                    name: text_of(&a["contributor"]).unwrap_or_default(),
                })
            })
            .collect(),
        albums: loop_of(v, "albums_loop")
            .filter_map(|a| {
                Some(Album {
                    id: positive_id(&a["album_id"])?,
                    title: text_of(&a["album"]).unwrap_or_default(),
                    artist: None,
                    artist_id: None,
                    year: None,
                    artwork: None,
                })
            })
            .collect(),
        tracks: loop_of(v, "tracks_loop")
            .filter_map(|t| {
                let id = positive_id(&t["track_id"])?;
                Some(Track {
                    id: Some(id),
                    title: text_of(&t["track"]).unwrap_or_default(),
                    artist: None,
                    album: None,
                    album_id: None,
                    artist_id: None,
                    duration_secs: None,
                    track_number: None,
                    artwork: cover_key(&Value::from(id)),
                    queue_index: None,
                })
            })
            .collect(),
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            let s = status_from(&v);
            assert!(s.volume <= 100);
        }
    }
}
