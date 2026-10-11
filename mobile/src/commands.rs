//! LMS command builders, limited to what the gateway's allow-list accepts. They build the
//! same shapes as the web app (`web/app/src/lib/lms/commands.ts`); an integration test runs
//! every one through the real gateway's policy. Ids and text are checked here, so bad
//! input never leaves the device.

use serde_json::{Value, json};

use crate::error::ApiError;

pub const STATUS_TAGS: &str = "tags:aAlcdeJKjNoqrsTIyx";
pub const QUEUE_TAGS: &str = "tags:alcdJKj";
pub const ALBUM_TAGS: &str = "tags:aljyS";
pub const TRACK_TAGS: &str = "tags:adtlcJKj";

/// Largest page the gateway accepts.
pub const MAX_PAGE: u32 = 500;

pub type Command = Vec<Value>;

/// LMS library ids are positive integers (at most 15 digits, like the web app).
pub fn id(v: i64) -> Result<i64, ApiError> {
    if v > 0 && v < 1_000_000_000_000_000 {
        Ok(v)
    } else {
        Err(ApiError::invalid("bad id"))
    }
}

/// LMS player ids look like MAC addresses.
pub fn is_player_id(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() == 17
        && b.iter().enumerate().all(|(i, c)| {
            if i % 3 == 2 {
                *c == b':'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

pub fn player_id(v: &str) -> Result<String, ApiError> {
    if is_player_id(v) {
        Ok(v.to_ascii_lowercase())
    } else {
        Err(ApiError::invalid("bad player id"))
    }
}

fn count(n: u32) -> u32 {
    n.clamp(1, MAX_PAGE)
}

/// Text parameters: control characters become spaces, trimmed, at most 200 characters
/// (the gateway allows 256 per argument).
pub fn text(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    t.trim().chars().take(200).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AlbumSort {
    Title,
    Artist,
    Year,
    Newest,
    Random,
}

impl AlbumSort {
    fn param(self) -> &'static str {
        match self {
            AlbumSort::Title => "sort:album",
            AlbumSort::Artist => "sort:artistalbum",
            AlbumSort::Year => "sort:yearalbum",
            AlbumSort::Newest => "sort:new",
            AlbumSort::Random => "sort:random",
        }
    }
}

// --- library -----------------------------------------------------------------------------

pub fn players() -> Command {
    vec![json!("players"), json!(0), json!(100)]
}

pub fn albums(start: u32, n: u32, sort: AlbumSort, search: Option<&str>) -> Command {
    let mut c = vec![
        json!("albums"),
        json!(start),
        json!(count(n)),
        json!(ALBUM_TAGS),
        json!(sort.param()),
    ];
    if let Some(s) = search.map(text).filter(|s| !s.is_empty()) {
        c.push(json!(format!("search:{s}")));
    }
    c
}

pub fn album_by_id(album: i64) -> Result<Command, ApiError> {
    Ok(vec![
        json!("albums"),
        json!(0),
        json!(1),
        json!(ALBUM_TAGS),
        json!(format!("album_id:{}", id(album)?)),
    ])
}

pub fn artist_albums(artist: i64) -> Result<Command, ApiError> {
    Ok(vec![
        json!("albums"),
        json!(0),
        json!(MAX_PAGE),
        json!(ALBUM_TAGS),
        json!("sort:yearalbum"),
        json!(format!("artist_id:{}", id(artist)?)),
    ])
}

pub fn artists(start: u32, n: u32, search: Option<&str>) -> Command {
    let mut c = vec![json!("artists"), json!(start), json!(count(n))];
    if let Some(s) = search.map(text).filter(|s| !s.is_empty()) {
        c.push(json!(format!("search:{s}")));
    }
    c
}

pub fn album_tracks(album: i64) -> Result<Command, ApiError> {
    Ok(vec![
        json!("titles"),
        json!(0),
        json!(MAX_PAGE),
        json!(TRACK_TAGS),
        json!("sort:tracknum"),
        json!(format!("album_id:{}", id(album)?)),
    ])
}

pub fn search(term: &str, n: u32) -> Result<Command, ApiError> {
    let t = text(term);
    if t.is_empty() {
        return Err(ApiError::invalid("enter something to search for"));
    }
    Ok(vec![
        json!("search"),
        json!(0),
        json!(count(n)),
        json!(format!("term:{t}")),
    ])
}

pub fn genres(start: u32, n: u32) -> Command {
    vec![json!("genres"), json!(start), json!(count(n))]
}

// --- playback (player commands) ----------------------------------------------------------

pub fn status() -> Command {
    vec![json!("status"), json!("-"), json!(1), json!(STATUS_TAGS)]
}

pub fn queue(start: u32, n: u32) -> Command {
    vec![
        json!("status"),
        json!(start),
        json!(count(n)),
        json!(QUEUE_TAGS),
    ]
}

pub fn play() -> Command {
    vec![json!("play")]
}

pub fn pause(on: Option<bool>) -> Command {
    match on {
        None => vec![json!("pause")],
        Some(on) => vec![json!("pause"), json!(u8::from(on))],
    }
}

pub fn stop() -> Command {
    vec![json!("stop")]
}

pub fn skip(forward: bool) -> Command {
    vec![
        json!("playlist"),
        json!("index"),
        json!(if forward { "+1" } else { "-1" }),
    ]
}

pub fn jump(index: u32) -> Command {
    vec![json!("playlist"), json!("index"), json!(index)]
}

pub fn seek(seconds: f64) -> Result<Command, ApiError> {
    if !seconds.is_finite() {
        return Err(ApiError::invalid("bad position"));
    }
    let s = (seconds.max(0.0) * 10.0).round() / 10.0;
    Ok(vec![json!("time"), json!(s)])
}

pub fn volume(v: u32) -> Command {
    vec![json!("mixer"), json!("volume"), json!(v.min(100))]
}

pub fn mute_toggle() -> Command {
    vec![json!("mixer"), json!("muting"), json!("toggle")]
}

pub fn power(on: bool) -> Command {
    vec![json!("power"), json!(u8::from(on))]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShuffleMode {
    Off,
    Songs,
    Albums,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

impl ShuffleMode {
    pub(crate) fn from_lms(v: i64) -> Self {
        match v {
            1 => ShuffleMode::Songs,
            2 => ShuffleMode::Albums,
            _ => ShuffleMode::Off,
        }
    }
}

impl RepeatMode {
    pub(crate) fn from_lms(v: i64) -> Self {
        match v {
            1 => RepeatMode::One,
            2 => RepeatMode::All,
            _ => RepeatMode::Off,
        }
    }
}

pub fn shuffle(mode: ShuffleMode) -> Command {
    let m = match mode {
        ShuffleMode::Off => 0,
        ShuffleMode::Songs => 1,
        ShuffleMode::Albums => 2,
    };
    vec![json!("playlist"), json!("shuffle"), json!(m)]
}

pub fn repeat(mode: RepeatMode) -> Command {
    let m = match mode {
        RepeatMode::Off => 0,
        RepeatMode::One => 1,
        RepeatMode::All => 2,
    };
    vec![json!("playlist"), json!("repeat"), json!(m)]
}

/// What to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Selection {
    Album { id: i64 },
    Track { id: i64 },
    Artist { id: i64 },
}

/// Where it goes: replace the queue and play, play next, or add to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Placement {
    PlayNow,
    PlayNext,
    AddToQueue,
}

pub fn enqueue(
    sel: Selection,
    placement: Placement,
    play_index: Option<u32>,
) -> Result<Command, ApiError> {
    let (key, value) = match sel {
        Selection::Album { id } => ("album_id", id),
        Selection::Track { id } => ("track_id", id),
        Selection::Artist { id } => ("artist_id", id),
    };
    let verb = match placement {
        Placement::PlayNow => "load",
        Placement::PlayNext => "insert",
        Placement::AddToQueue => "add",
    };
    let mut c = vec![
        json!("playlistcontrol"),
        json!(format!("cmd:{verb}")),
        json!(format!("{key}:{}", id(value)?)),
    ];
    if let (Placement::PlayNow, Some(i)) = (placement, play_index) {
        c.push(json!(format!("play_index:{i}")));
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_shapes_as_the_web_app() {
        assert_eq!(
            albums(0, 100, AlbumSort::Title, None),
            vec![
                json!("albums"),
                json!(0),
                json!(100),
                json!("tags:aljyS"),
                json!("sort:album")
            ]
        );
        assert_eq!(
            enqueue(Selection::Album { id: 7 }, Placement::PlayNow, Some(2)).unwrap(),
            vec![
                json!("playlistcontrol"),
                json!("cmd:load"),
                json!("album_id:7"),
                json!("play_index:2")
            ]
        );
        assert_eq!(
            enqueue(Selection::Track { id: 9 }, Placement::PlayNext, Some(2)).unwrap(),
            vec![
                json!("playlistcontrol"),
                json!("cmd:insert"),
                json!("track_id:9")
            ]
        );
        assert_eq!(seek(12.345).unwrap(), vec![json!("time"), json!(12.3)]);
        assert_eq!(
            volume(250),
            vec![json!("mixer"), json!("volume"), json!(100)]
        );
        assert_eq!(queue(0, 10_000)[2], json!(500));
        assert_eq!(artists(0, 0, None)[2], json!(1));
    }

    #[test]
    fn bad_ids_and_text_never_leave_the_device() {
        assert!(id(0).is_err());
        assert!(id(-3).is_err());
        assert!(id(i64::MAX).is_err());
        assert!(album_tracks(0).is_err());
        assert!(enqueue(Selection::Artist { id: -1 }, Placement::AddToQueue, None).is_err());
        assert!(seek(f64::NAN).is_err());
        assert!(seek(f64::INFINITY).is_err());
        assert_eq!(seek(-5.0).unwrap()[1], json!(0.0));
        assert!(search("   ", 10).is_err());
        assert_eq!(text("a\r\nb\u{0}"), "a  b");
        assert_eq!(text(&"x".repeat(1000)).chars().count(), 200);
        // An empty search is no search at all.
        assert_eq!(albums(0, 10, AlbumSort::Title, Some("  ")).len(), 5);
    }

    #[test]
    fn player_ids() {
        assert_eq!(player_id("AA:BB:CC:DD:EE:01").unwrap(), "aa:bb:cc:dd:ee:01");
        for bad in [
            "",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:0g",
            "aa-bb-cc-dd-ee-01",
            "aa:bb:cc:dd:ee:01 ",
            "aa:bb:cc:dd:ee:01:02",
            "../../etc/passwd",
        ] {
            assert!(player_id(bad).is_err(), "{bad}");
        }
    }
}
