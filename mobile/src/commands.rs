//! LMS command builders, limited to what the gateway's allow-list accepts. They build the
//! same shapes as the web app (`web/app/src/lib/lms/commands.ts`); an integration test runs
//! every one through the real gateway's policy. Ids and text are checked here, so bad
//! input never leaves the device.

use serde_json::{Value, json};

use crate::error::ApiError;

pub const STATUS_TAGS: &str = "tags:aAlcdeJKjNoqrsTIyx";
pub const QUEUE_TAGS: &str = "tags:alcdJKj";
/// Albums: artist, album, artwork, year, artist id, and the compilation flag.
pub const ALBUM_TAGS: &str = "tags:aljySw";
/// Tracks: as the web app's, plus album and artist ids, genre, disc, format, sample size
/// and rate, and year (for the album page's details and multi-disc sections).
pub const TRACK_TAGS: &str = "tags:adtlcJKjegiqosITy";

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

/// A year as LMS lists them: 0 ("no year") up to four digits.
pub fn year(v: u32) -> Result<u32, ApiError> {
    if v <= 9999 {
        Ok(v)
    } else {
        Err(ApiError::invalid("bad year"))
    }
}

/// An LMS virtual library id: short hex (the gateway refuses anything else).
pub fn library_id(v: &str) -> Result<&str, ApiError> {
    if (1..=32).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(v)
    } else {
        Err(ApiError::invalid("bad library id"))
    }
}

/// LMS favourite and menu item ids, e.g. `f047f379.3`.
pub fn item_id(v: &str) -> Result<&str, ApiError> {
    if (1..=128).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Ok(v)
    } else {
        Err(ApiError::invalid("bad item id"))
    }
}

/// An app or radio command word, as LMS lists them (`spotty`, `bbcsounds`, …).
pub fn app_command(v: &str) -> Result<&str, ApiError> {
    if (1..=64).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        Ok(v)
    } else {
        Err(ApiError::invalid("bad app"))
    }
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

/// LMS contributor roles (the gateway passes on only these).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ArtistRole {
    AlbumArtist,
    Artist,
    Composer,
    Conductor,
    Band,
    TrackArtist,
}

impl ArtistRole {
    pub(crate) fn lms(self) -> &'static str {
        match self {
            ArtistRole::AlbumArtist => "ALBUMARTIST",
            ArtistRole::Artist => "ARTIST",
            ArtistRole::Composer => "COMPOSER",
            ArtistRole::Conductor => "CONDUCTOR",
            ArtistRole::Band => "BAND",
            ArtistRole::TrackArtist => "TRACKARTIST",
        }
    }
}

/// What an album or artist list is narrowed to. Every value is checked before use; a
/// value a list doesn't support is left out (as the web app does).
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct LibraryFilter {
    #[uniffi(default = None)]
    pub genre_id: Option<i64>,
    #[uniffi(default = None)]
    pub year: Option<u32>,
    #[uniffi(default = None)]
    pub artist_id: Option<i64>,
    #[uniffi(default = None)]
    pub compilation: Option<bool>,
    #[uniffi(default = None)]
    pub role: Option<ArtistRole>,
    /// An LMS virtual library, or none for the whole library.
    #[uniffi(default = None)]
    pub library_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Genre,
    Year,
    Artist,
    Compilation,
    Role,
    Library,
}

/// `key:value` parameters for the filter, in the web app's order, for `allowed` keys only.
fn filter_params(f: &LibraryFilter, allowed: &[Key]) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::new();
    let has = |k: Key| allowed.contains(&k);
    if let (true, Some(g)) = (has(Key::Genre), f.genre_id) {
        out.push(json!(format!("genre_id:{}", id(g)?)));
    }
    if let (true, Some(y)) = (has(Key::Year), f.year) {
        out.push(json!(format!("year:{}", year(y)?)));
    }
    if let (true, Some(a)) = (has(Key::Artist), f.artist_id) {
        out.push(json!(format!("artist_id:{}", id(a)?)));
    }
    if let (true, Some(c)) = (has(Key::Compilation), f.compilation) {
        out.push(json!(format!("compilation:{}", u8::from(c))));
    }
    if let (true, Some(r)) = (has(Key::Role), f.role) {
        out.push(json!(format!("role_id:{}", r.lms())));
    }
    if let (true, Some(l)) = (has(Key::Library), f.library_id.as_deref()) {
        out.push(json!(format!("library_id:{}", library_id(l)?)));
    }
    Ok(out)
}

fn in_library(library: Option<&str>) -> LibraryFilter {
    LibraryFilter {
        library_id: library.map(str::to_owned),
        ..Default::default()
    }
}

fn search_param(search: Option<&str>) -> Option<Value> {
    search
        .map(text)
        .filter(|s| !s.is_empty())
        .map(|s| json!(format!("search:{s}")))
}

// --- library -----------------------------------------------------------------------------

pub fn players() -> Command {
    vec![json!("players"), json!(0), json!(100)]
}

/// All sync groups (the gateway keeps only players the caller may control).
pub fn sync_groups() -> Command {
    vec![json!("syncgroups"), json!("?")]
}

pub fn albums(
    start: u32,
    n: u32,
    sort: AlbumSort,
    search: Option<&str>,
    filter: &LibraryFilter,
) -> Result<Command, ApiError> {
    let mut c = vec![
        json!("albums"),
        json!(start),
        json!(count(n)),
        json!(ALBUM_TAGS),
        json!(sort.param()),
    ];
    c.extend(search_param(search));
    c.extend(filter_params(
        filter,
        &[
            Key::Genre,
            Key::Year,
            Key::Artist,
            Key::Compilation,
            Key::Role,
            Key::Library,
        ],
    )?);
    Ok(c)
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

/// An artist's albums, oldest first; with a role, only those where they have that role.
pub fn artist_albums(artist: i64, role: Option<ArtistRole>) -> Result<Command, ApiError> {
    let mut c = vec![
        json!("albums"),
        json!(0),
        json!(MAX_PAGE),
        json!(ALBUM_TAGS),
        json!("sort:yearalbum"),
        json!(format!("artist_id:{}", id(artist)?)),
    ];
    if let Some(r) = role {
        c.push(json!(format!("role_id:{}", r.lms())));
    }
    Ok(c)
}

pub fn artists(
    start: u32,
    n: u32,
    search: Option<&str>,
    filter: &LibraryFilter,
) -> Result<Command, ApiError> {
    let mut c = vec![json!("artists"), json!(start), json!(count(n))];
    c.extend(search_param(search));
    c.extend(filter_params(
        filter,
        &[Key::Genre, Key::Role, Key::Library],
    )?);
    Ok(c)
}

/// Does the library have any artists in this role? (One row is enough to tell.)
pub fn has_role(role: ArtistRole, library: Option<&str>) -> Result<Command, ApiError> {
    let f = LibraryFilter {
        role: Some(role),
        ..in_library(library)
    };
    let mut c = vec![json!("artists"), json!(0), json!(1)];
    c.extend(filter_params(&f, &[Key::Role, Key::Library])?);
    Ok(c)
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

pub fn genres(
    start: u32,
    n: u32,
    library: Option<&str>,
    search: Option<&str>,
) -> Result<Command, ApiError> {
    let mut c = vec![json!("genres"), json!(start), json!(count(n))];
    c.extend(search_param(search));
    c.extend(filter_params(&in_library(library), &[Key::Library])?);
    Ok(c)
}

pub fn genre_by_id(genre: i64) -> Result<Command, ApiError> {
    Ok(vec![
        json!("genres"),
        json!(0),
        json!(1),
        json!(format!("genre_id:{}", id(genre)?)),
    ])
}

pub fn years(start: u32, n: u32, library: Option<&str>) -> Result<Command, ApiError> {
    let mut c = vec![
        json!("years"),
        json!(start),
        json!(count(n)),
        json!("hasAlbums:1"),
    ];
    c.extend(filter_params(&in_library(library), &[Key::Library])?);
    Ok(c)
}

/// One folder of the music folder (the top level without an id). Never a path.
pub fn music_folder(start: u32, n: u32, folder: Option<i64>) -> Result<Command, ApiError> {
    let mut c = vec![json!("musicfolder"), json!(start), json!(count(n))];
    if let Some(f) = folder {
        c.push(json!(format!("folder_id:{}", id(f)?)));
    }
    c.push(json!("tags:cdt"));
    Ok(c)
}

/// LMS's virtual libraries, if any.
pub fn libraries() -> Command {
    vec![json!("libraries")]
}

pub fn favourites(start: u32, n: u32, item: Option<&str>) -> Result<Command, ApiError> {
    let mut c = vec![
        json!("favorites"),
        json!("items"),
        json!(start),
        json!(count(n)),
    ];
    if let Some(i) = item {
        c.push(json!(format!("item_id:{}", item_id(i)?)));
    }
    Ok(c)
}

/// Favourites in SlimBrowse form (`menu:1`), where library items carry their ids.
pub fn favourite_menu(item: Option<&str>) -> Result<Command, ApiError> {
    let mut c = vec![json!("favorites"), json!("items"), json!(0), json!(200)];
    if let Some(i) = item {
        c.push(json!(format!("item_id:{}", item_id(i)?)));
    }
    c.push(json!("menu:1"));
    Ok(c)
}

pub fn playlists(start: u32, n: u32) -> Command {
    vec![json!("playlists"), json!(start), json!(count(n))]
}

pub fn playlist_tracks(playlist: i64) -> Result<Command, ApiError> {
    Ok(vec![
        json!("playlists"),
        json!("tracks"),
        json!(0),
        json!(MAX_PAGE),
        json!(format!("playlist_id:{}", id(playlist)?)),
        json!(TRACK_TAGS),
    ])
}

// --- apps, radio and context menus (SlimBrowse) --------------------------------------------

pub fn apps_list() -> Command {
    vec![json!("apps"), json!(0), json!(100)]
}

pub fn radios_list() -> Command {
    vec![json!("radios"), json!(0), json!(100)]
}

/// The player's home menu, where plugins add entries (filtered by the gateway).
pub fn home_menu() -> Command {
    vec![json!("menu"), json!(0), json!(500), json!("direct:1")]
}

/// What a "More…" (context) menu is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InfoKind {
    Track,
    Album,
    Artist,
    Genre,
    Playlist,
    Year,
    Folder,
}

impl InfoKind {
    pub(crate) fn command(self) -> &'static str {
        match self {
            InfoKind::Track => "trackinfo",
            InfoKind::Album => "albuminfo",
            InfoKind::Artist => "artistinfo",
            InfoKind::Genre => "genreinfo",
            InfoKind::Playlist => "playlistinfo",
            InfoKind::Year => "yearinfo",
            InfoKind::Folder => "folderinfo",
        }
    }

    /// The selector parameter (`track_id` = 7, `year` = 1999), checked.
    pub(crate) fn selector(self, item: i64) -> Result<(String, String), ApiError> {
        let key = match self {
            InfoKind::Year => {
                let y = u32::try_from(item).map_err(|_| ApiError::invalid("bad year"))?;
                return Ok(("year".into(), year(y)?.to_string()));
            }
            InfoKind::Track => "track_id",
            InfoKind::Album => "album_id",
            InfoKind::Artist => "artist_id",
            InfoKind::Genre => "genre_id",
            InfoKind::Playlist => "playlist_id",
            InfoKind::Folder => "folder_id",
        };
        Ok((key.into(), id(item)?.to_string()))
    }
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

// --- queue editing ------------------------------------------------------------------------

/// Take out the queue entry at `index`.
pub fn remove_from_queue(index: u32) -> Command {
    vec![json!("playlist"), json!("delete"), json!(index)]
}

/// Move the entry at `from` so that it ends up at index `to` (in the new order), as LMS's
/// `playlist move` does.
pub fn move_in_queue(from: u32, to: u32) -> Command {
    vec![json!("playlist"), json!("move"), json!(from), json!(to)]
}

pub fn clear_queue() -> Command {
    vec![json!("playlist"), json!("clear")]
}

// --- synchronisation (player groups) -------------------------------------------------------

/// Sent for the player whose group `other` joins: `<X> sync <Y>` makes Y join X's group
/// (Y's own queue is replaced by the group's).
pub fn sync_with(other: &str) -> Result<Command, ApiError> {
    Ok(vec![json!("sync"), json!(player_id(other)?)])
}

/// Sent for the player that leaves its group (it stops, keeping a copy of the queue).
pub fn unsync() -> Command {
    vec![json!("sync"), json!("-")]
}

/// The gateway's limit on a comma-separated id list (`track_id:1,2,3`).
pub const ID_LIST_MAX: usize = 200;

/// Replace the queue with these library tracks (`play_index` starting there), or append
/// them. Every id is checked, and the list must fit [`ID_LIST_MAX`] (see [`chunk_ids`]).
pub fn load_tracks(ids: &[i64], add: bool, play_index: Option<u32>) -> Result<Command, ApiError> {
    let list = ids
        .iter()
        .map(|v| id(*v).map(|v| v.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .join(",");
    if list.is_empty() || list.len() > ID_LIST_MAX {
        return Err(ApiError::invalid("bad track list"));
    }
    let mut c = vec![
        json!("playlistcontrol"),
        json!(if add { "cmd:add" } else { "cmd:load" }),
        json!(format!("track_id:{list}")),
    ];
    if let (false, Some(i)) = (add, play_index) {
        c.push(json!(format!("play_index:{i}")));
    }
    Ok(c)
}

/// Split ids, in order, into lists whose comma-separated form fits `max` characters.
pub fn chunk_ids(ids: &[i64], max: usize) -> Vec<Vec<i64>> {
    let mut out = Vec::new();
    let mut cur: Vec<i64> = Vec::new();
    let mut len = 0;
    for &v in ids {
        let n = v.to_string().len();
        if !cur.is_empty() && len + 1 + n > max {
            out.push(std::mem::take(&mut cur));
            len = 0;
        }
        len += usize::from(!cur.is_empty()) + n;
        cur.push(v);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// What to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Selection {
    Album { id: i64 },
    Track { id: i64 },
    Artist { id: i64 },
    Playlist { id: i64 },
    Genre { id: i64 },
    Year { year: u32 },
    Folder { id: i64 },
}

impl Selection {
    /// The `key:value` selector, checked.
    fn selector(self) -> Result<String, ApiError> {
        Ok(match self {
            Selection::Album { id: v } => format!("album_id:{}", id(v)?),
            Selection::Track { id: v } => format!("track_id:{}", id(v)?),
            Selection::Artist { id: v } => format!("artist_id:{}", id(v)?),
            Selection::Playlist { id: v } => format!("playlist_id:{}", id(v)?),
            Selection::Genre { id: v } => format!("genre_id:{}", id(v)?),
            Selection::Year { year: y } => format!("year:{}", year(y)?),
            Selection::Folder { id: v } => format!("folder_id:{}", id(v)?),
        })
    }
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
    let verb = match placement {
        Placement::PlayNow => "load",
        Placement::PlayNext => "insert",
        Placement::AddToQueue => "add",
    };
    let mut c = vec![
        json!("playlistcontrol"),
        json!(format!("cmd:{verb}")),
        json!(sel.selector()?),
    ];
    if let (Placement::PlayNow, Some(i)) = (placement, play_index) {
        c.push(json!(format!("play_index:{i}")));
    }
    Ok(c)
}

/// Play, play next or add a favourite (radio stations, tracks, app items).
pub fn play_favourite(item: &str, placement: Placement) -> Result<Command, ApiError> {
    let verb = match placement {
        Placement::PlayNow => "play",
        Placement::PlayNext => "insert",
        Placement::AddToQueue => "add",
    };
    Ok(vec![
        json!("favorites"),
        json!("playlist"),
        json!(verb),
        json!(format!("item_id:{}", item_id(item)?)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> LibraryFilter {
        LibraryFilter::default()
    }

    #[test]
    fn same_shapes_as_the_web_app() {
        assert_eq!(
            albums(0, 100, AlbumSort::Title, None, &none()).unwrap(),
            vec![
                json!("albums"),
                json!(0),
                json!(100),
                json!("tags:aljySw"),
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
        assert_eq!(artists(0, 0, None, &none()).unwrap()[2], json!(1));
    }

    #[test]
    fn browse_builders_match_the_web_app() {
        // Parameters in the web app's order: search, then genre, year, artist,
        // compilation, role, library.
        let all = LibraryFilter {
            genre_id: Some(4),
            year: Some(1999),
            artist_id: Some(3),
            compilation: Some(true),
            role: Some(ArtistRole::Composer),
            library_id: Some("ab12".into()),
        };
        assert_eq!(
            albums(10, 60, AlbumSort::Newest, Some(" x "), &all).unwrap(),
            vec![
                json!("albums"),
                json!(10),
                json!(60),
                json!("tags:aljySw"),
                json!("sort:new"),
                json!("search:x"),
                json!("genre_id:4"),
                json!("year:1999"),
                json!("artist_id:3"),
                json!("compilation:1"),
                json!("role_id:COMPOSER"),
                json!("library_id:ab12"),
            ]
        );
        // Artists take only genre, role and library.
        assert_eq!(
            artists(0, 200, None, &all).unwrap(),
            vec![
                json!("artists"),
                json!(0),
                json!(200),
                json!("genre_id:4"),
                json!("role_id:COMPOSER"),
                json!("library_id:ab12"),
            ]
        );
        assert_eq!(
            has_role(ArtistRole::Band, None).unwrap(),
            vec![json!("artists"), json!(0), json!(1), json!("role_id:BAND")]
        );
        assert_eq!(
            genres(0, 100, Some("ab12"), Some("ja")).unwrap(),
            vec![
                json!("genres"),
                json!(0),
                json!(100),
                json!("search:ja"),
                json!("library_id:ab12")
            ]
        );
        assert_eq!(
            genre_by_id(4).unwrap(),
            vec![json!("genres"), json!(0), json!(1), json!("genre_id:4")]
        );
        assert_eq!(
            years(0, 100, None).unwrap(),
            vec![json!("years"), json!(0), json!(100), json!("hasAlbums:1")]
        );
        assert_eq!(
            music_folder(0, 200, None).unwrap(),
            vec![
                json!("musicfolder"),
                json!(0),
                json!(200),
                json!("tags:cdt")
            ]
        );
        assert_eq!(
            music_folder(0, 200, Some(12)).unwrap(),
            vec![
                json!("musicfolder"),
                json!(0),
                json!(200),
                json!("folder_id:12"),
                json!("tags:cdt")
            ]
        );
        assert_eq!(libraries(), vec![json!("libraries")]);
        assert_eq!(sync_groups(), vec![json!("syncgroups"), json!("?")]);
        assert_eq!(
            favourites(0, 500, Some("f047f379.3")).unwrap(),
            vec![
                json!("favorites"),
                json!("items"),
                json!(0),
                json!(500),
                json!("item_id:f047f379.3")
            ]
        );
        assert_eq!(
            playlist_tracks(5).unwrap(),
            vec![
                json!("playlists"),
                json!("tracks"),
                json!(0),
                json!(500),
                json!("playlist_id:5"),
                json!(TRACK_TAGS)
            ]
        );
        assert_eq!(
            artist_albums(3, Some(ArtistRole::AlbumArtist)).unwrap()[6],
            json!("role_id:ALBUMARTIST")
        );
        assert_eq!(
            play_favourite("f0.1", Placement::PlayNext).unwrap(),
            vec![
                json!("favorites"),
                json!("playlist"),
                json!("insert"),
                json!("item_id:f0.1")
            ]
        );
        assert_eq!(
            home_menu(),
            vec![json!("menu"), json!(0), json!(500), json!("direct:1")]
        );
        for (sel, want) in [
            (Selection::Playlist { id: 5 }, "playlist_id:5"),
            (Selection::Genre { id: 4 }, "genre_id:4"),
            (Selection::Year { year: 0 }, "year:0"),
            (Selection::Folder { id: 12 }, "folder_id:12"),
        ] {
            assert_eq!(
                enqueue(sel, Placement::AddToQueue, None).unwrap()[2],
                json!(want)
            );
        }
        assert_eq!(
            InfoKind::Year.selector(1999).unwrap(),
            ("year".into(), "1999".into())
        );
        assert_eq!(InfoKind::Folder.command(), "folderinfo");
    }

    #[test]
    fn bad_ids_and_text_never_leave_the_device() {
        assert!(id(0).is_err());
        assert!(id(-3).is_err());
        assert!(id(i64::MAX).is_err());
        assert!(album_tracks(0).is_err());
        assert!(enqueue(Selection::Artist { id: -1 }, Placement::AddToQueue, None).is_err());
        assert!(enqueue(Selection::Year { year: 10_000 }, Placement::PlayNow, None).is_err());
        assert!(enqueue(Selection::Folder { id: 0 }, Placement::PlayNow, None).is_err());
        assert!(seek(f64::NAN).is_err());
        assert!(seek(f64::INFINITY).is_err());
        assert_eq!(seek(-5.0).unwrap()[1], json!(0.0));
        assert!(search("   ", 10).is_err());
        assert_eq!(text("a\r\nb\u{0}"), "a  b");
        assert_eq!(text(&"x".repeat(1000)).chars().count(), 200);
        // An empty search is no search at all.
        assert_eq!(
            albums(0, 10, AlbumSort::Title, Some("  "), &none())
                .unwrap()
                .len(),
            5
        );
    }

    /// Filter values are checked: nothing malformed is ever sent.
    #[test]
    fn hostile_filters_and_item_ids_are_refused() {
        for bad in [
            LibraryFilter {
                library_id: Some("../x".into()),
                ..none()
            },
            LibraryFilter {
                library_id: Some(String::new()),
                ..none()
            },
            LibraryFilter {
                library_id: Some("a".repeat(33)),
                ..none()
            },
            LibraryFilter {
                library_id: Some("ab12 library_id:cd".into()),
                ..none()
            },
            LibraryFilter {
                genre_id: Some(0),
                ..none()
            },
            LibraryFilter {
                year: Some(12_345),
                ..none()
            },
            LibraryFilter {
                artist_id: Some(-7),
                ..none()
            },
        ] {
            assert!(
                albums(0, 10, AlbumSort::Title, None, &bad).is_err(),
                "{bad:?}"
            );
        }
        assert!(genres(0, 10, Some("zz"), None).is_err());
        assert!(years(0, 10, Some("x/y")).is_err());
        assert!(music_folder(0, 10, Some(0)).is_err());
        assert!(playlist_tracks(-1).is_err());
        assert!(genre_by_id(0).is_err());
        for bad in ["", "a b", "a:b", "../x", "x\u{0}", &"a".repeat(129)] {
            assert!(item_id(bad).is_err(), "{bad:?}");
            assert!(favourites(0, 10, Some(bad)).is_err(), "{bad:?}");
            assert!(play_favourite(bad, Placement::PlayNow).is_err(), "{bad:?}");
        }
        for bad in ["", "Spotty", "a-b", "a b", "../x", &"a".repeat(65)] {
            assert!(app_command(bad).is_err(), "{bad:?}");
        }
        assert!(app_command("bbc_sounds2").is_ok());
        assert!(InfoKind::Year.selector(-1).is_err());
        assert!(InfoKind::Year.selector(10_000).is_err());
        assert!(InfoKind::Track.selector(0).is_err());
    }

    #[test]
    fn queue_edit_and_sync_shapes_match_the_web_app() {
        assert_eq!(
            remove_from_queue(2),
            vec![json!("playlist"), json!("delete"), json!(2)]
        );
        assert_eq!(
            move_in_queue(3, 0),
            vec![json!("playlist"), json!("move"), json!(3), json!(0)]
        );
        assert_eq!(clear_queue(), vec![json!("playlist"), json!("clear")]);
        assert_eq!(
            sync_with("AA:BB:CC:DD:EE:01").unwrap(),
            vec![json!("sync"), json!("aa:bb:cc:dd:ee:01")]
        );
        assert_eq!(unsync(), vec![json!("sync"), json!("-")]);
        for bad in ["", "-", "all", "aa:bb:cc:dd:ee", "aa:bb:cc:dd:ee:01 x"] {
            assert!(sync_with(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            load_tracks(&[11, 12, 13], false, Some(1)).unwrap(),
            vec![
                json!("playlistcontrol"),
                json!("cmd:load"),
                json!("track_id:11,12,13"),
                json!("play_index:1")
            ]
        );
        // `add` never takes a play index.
        assert_eq!(
            load_tracks(&[14, 15], true, Some(1)).unwrap(),
            vec![
                json!("playlistcontrol"),
                json!("cmd:add"),
                json!("track_id:14,15")
            ]
        );
    }

    #[test]
    fn track_lists_are_checked_and_fit_the_gateway() {
        assert!(load_tracks(&[], false, None).is_err());
        assert!(load_tracks(&[1, 0], false, None).is_err());
        assert!(load_tracks(&[-108_699_300_821_096], false, None).is_err());
        let long: Vec<i64> = (0..30).map(|i| 9_000_000 + i).collect();
        assert!(load_tracks(&long, true, None).is_err());
        let chunks = chunk_ids(&long, ID_LIST_MAX);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), long, "order kept, nothing lost");
        for c in &chunks {
            assert!(load_tracks(c, true, None).is_ok());
        }
        // Exactly at the limit: 25 seven-digit ids and 24 commas = 199 characters.
        let fits: Vec<i64> = (0..25).map(|i| 1_000_000 + i).collect();
        assert_eq!(chunk_ids(&fits, ID_LIST_MAX).len(), 1);
        assert!(chunk_ids(&[], ID_LIST_MAX).is_empty());
        // An id longer than the limit on its own still gets a list (and is refused later).
        assert_eq!(chunk_ids(&[123_456], 3), vec![vec![123_456]]);
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
