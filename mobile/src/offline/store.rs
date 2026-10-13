//! The downloads on this device for one user on one server, in app-private storage.
//!
//! ```text
//! <root>/<scope>/            scope: SHA-256 of the server address and user name (hex, 32)
//!     index.json             the offline index (see `index`), index.json.bak its last version
//!     plays.json             plays waiting to be sent (see `plays`)
//!     queue.json             downloads waiting (see `download`)
//!     tracks/<key>.<ext>     finished downloads, named by stable key
//!     partial/<key>.<fmt>    downloads in progress (resumed with HTTP Range)
//!     art/<album>.img        album covers, for offline browsing
//! ```
//!
//! Nothing here names a user, server, token or library path: directories are hashes, files
//! are stable keys and fixed extensions. The host passes an app-private root (Android:
//! `noBackupFilesDir`, which is never backed up or transferred).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::art::ArtImage;
use crate::error::ApiError;

use super::index::{self, DownloadFormat, DownloadOrigin, OfflineTrack, Parsed};
use super::plays::{self, PlayRecord};

/// Largest cached cover (a large cover is well under this).
pub const MAX_ART_BYTES: usize = 4 * 1024 * 1024;

/// An album on the device (its tracks grouped by [`OfflineTrack::album_ref`]).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflineAlbum {
    pub album_ref: String,
    pub album_id: Option<i64>,
    pub title: String,
    /// The album's artist; empty when its tracks have different artists.
    pub artist: String,
    pub track_count: u32,
    pub size: u64,
    pub has_art: bool,
    /// The latest download among its tracks (Unix seconds).
    pub downloaded_at: u64,
    /// Manual if the user chose any of it; otherwise the smart download that brought it.
    pub origin: DownloadOrigin,
}

struct State {
    tracks: Vec<OfflineTrack>,
    plays: Vec<PlayRecord>,
    /// Ratings made offline, waiting to be sent (see `crate::routine`).
    ratings: Vec<crate::routine::QueuedRating>,
}

/// The offline library of one user on one server.
#[derive(uniffi::Object)]
pub struct OfflineLibrary {
    dir: PathBuf,
    state: Mutex<State>,
}

/// The folder name for a user on a server: a hash, so the folder names neither.
pub fn scope_name(server_url: &str, username: &str) -> String {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    ctx.update(b"lyrion-anywhere offline v1\n");
    ctx.update(server_url.as_bytes());
    ctx.update(b"\n");
    ctx.update(username.as_bytes());
    ctx.finish().as_ref()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn scope_dir(root: &str, server_url: &str, username: &str) -> Result<PathBuf, ApiError> {
    let root = Path::new(root);
    if !root.is_absolute() || root.components().any(|c| c.as_os_str() == "..") {
        return Err(ApiError::invalid(
            "the download folder must be an absolute path",
        ));
    }
    if server_url.is_empty() || username.is_empty() || username.len() > 200 {
        return Err(ApiError::invalid("no user for the downloads"));
    }
    Ok(root.join(scope_name(server_url, username)))
}

fn io_error(what: &str, e: std::io::Error) -> ApiError {
    if e.raw_os_error() == Some(28) || e.kind() == std::io::ErrorKind::StorageFull {
        return ApiError::invalid("not enough storage space on this device");
    }
    ApiError::invalid(format!("{what}: {}", e.kind()))
}

/// Write `bytes` to `path` atomically: a temporary file, flushed, renamed over it. With
/// `backup`, the previous version is kept as `<path>.bak`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], backup: bool) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if backup && path.exists() {
        let _ = fs::rename(path, path.with_extension("json.bak"));
    }
    fs::rename(&tmp, path)
}

/// Read at most `limit` bytes of a file (`None` if missing, unreadable or larger).
pub(crate) fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    fs::read(path).ok()
}

/// The image type of cached artwork, from its first bytes (never trusted from elsewhere).
pub(crate) fn sniff_image(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

fn valid_album_ref(r: &str) -> bool {
    let b = r.as_bytes();
    match b.first() {
        Some(b'a') => (2..=13).contains(&b.len()) && b[1..].iter().all(u8::is_ascii_digit),
        Some(b't') => {
            b.len() == 17
                && b[1..]
                    .iter()
                    .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        }
        _ => false,
    }
}

impl OfflineLibrary {
    fn tracks_dir(&self) -> PathBuf {
        self.dir.join("tracks")
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn plays_path(&self) -> PathBuf {
        self.dir.join("plays.json")
    }

    fn ratings_path(&self) -> PathBuf {
        self.dir.join("ratings.json")
    }

    fn save_ratings(&self, q: &[crate::routine::QueuedRating]) {
        // Best effort, as for plays.
        let _ = write_atomic(
            &self.ratings_path(),
            &crate::routine::serialise_queue(q),
            false,
        );
    }

    pub(crate) fn queued_ratings(&self) -> Vec<crate::routine::QueuedRating> {
        self.state.lock().unwrap().ratings.clone()
    }

    /// Drop ratings the server took (only if not rated again meanwhile).
    pub(crate) fn drop_ratings(&self, sent: &[crate::routine::QueuedRating]) {
        let mut st = self.state.lock().unwrap();
        st.ratings.retain(|r| !sent.contains(r));
        self.save_ratings(&st.ratings);
    }

    pub(crate) fn queue_path(&self) -> PathBuf {
        self.dir.join("queue.json")
    }

    pub(crate) fn partial_dir(&self) -> PathBuf {
        self.dir.join("partial")
    }

    fn art_dir(&self) -> PathBuf {
        self.dir.join("art")
    }

    fn file_of(&self, t: &OfflineTrack) -> PathBuf {
        self.tracks_dir().join(format!("{}.{}", t.key, t.ext))
    }

    /// Where a download in progress is kept (`key` must be valid).
    pub(crate) fn partial_path(&self, key: &str, format: DownloadFormat) -> PathBuf {
        debug_assert!(index::valid_key(key));
        self.partial_dir()
            .join(format!("{key}.{}", format.as_str()))
    }

    fn art_path(&self, album_ref: &str) -> Option<PathBuf> {
        valid_album_ref(album_ref).then(|| self.art_dir().join(format!("{album_ref}.img")))
    }

    fn create_dirs(&self) -> Result<(), ApiError> {
        for d in [self.tracks_dir(), self.partial_dir(), self.art_dir()] {
            fs::create_dir_all(&d)
                .map_err(|e| io_error("could not create the download folder", e))?;
        }
        Ok(())
    }

    /// A copy of an index that couldn't be read, kept until it can be (a passing read
    /// error) or the user removes all downloads.
    fn unreadable_index_path(&self) -> PathBuf {
        self.index_path().with_extension("json.unreadable")
    }

    /// The index, and whether it can be trusted to name every download (so files it
    /// doesn't name may be deleted). An index that exists but can't be read (a read error,
    /// damage, a newer app's format) is never grounds for deleting the files.
    /// Also says whether a copy kept aside was read back (so the index should be saved).
    fn load_index(&self) -> (Vec<OfflineTrack>, bool, bool) {
        let read = |p: &Path| read_limited(p, index::MAX_INDEX_BYTES).map(|b| index::parse(&b));
        let (main, bak, aside) = (
            self.index_path(),
            self.index_path().with_extension("json.bak"),
            self.unreadable_index_path(),
        );
        let (mut tracks, mut trusted) = match read(&main) {
            Some(Parsed::Tracks(t, _)) => (t, true),
            _ if !main.exists() && !bak.exists() => (Vec::new(), true),
            _ => {
                // Keep what couldn't be read (the first copy only), then try the last good
                // version. Its files are kept: the list may be missing some.
                if main.exists() && !aside.exists() {
                    let _ = fs::copy(&main, &aside);
                }
                match read(&bak) {
                    Some(Parsed::Tracks(t, _)) => (t, false),
                    _ => (Vec::new(), false),
                }
            }
        };
        let mut recovered = false;
        if aside.exists() {
            match read(&aside) {
                // Readable after all: take back what it lists (reconcile drops entries
                // whose file has gone), and it's no longer needed.
                Some(Parsed::Tracks(t, _)) if trusted => {
                    for x in t {
                        if !tracks.iter().any(|k| k.key == x.key)
                            && tracks.len() < index::MAX_TRACKS
                        {
                            tracks.push(x);
                        }
                    }
                    let _ = fs::remove_file(&aside);
                    recovered = true;
                }
                _ => trusted = false,
            }
        }
        (tracks, trusted, recovered)
    }

    fn save_index(&self, tracks: &[OfflineTrack]) -> Result<(), ApiError> {
        write_atomic(&self.index_path(), &index::serialise(tracks), true)
            .map_err(|e| io_error("could not save the download list", e))
    }

    fn save_plays(&self, plays: &[PlayRecord]) {
        // Best effort: a play that can't be saved is lost, playback carries on.
        let _ = write_atomic(&self.plays_path(), &plays::serialise(plays), false);
    }

    /// Drop index entries whose file is missing or the wrong size, and (when the index is
    /// `trusted`) delete files no entry names, left by a crash.
    fn reconcile(&self, tracks: Vec<OfflineTrack>, trusted: bool) -> (Vec<OfflineTrack>, bool) {
        let before = tracks.len();
        let kept: Vec<OfflineTrack> = tracks
            .into_iter()
            .filter(|t| {
                fs::metadata(self.file_of(t)).is_ok_and(|m| m.is_file() && m.len() == t.size)
            })
            .collect();
        let names: std::collections::HashSet<String> = kept
            .iter()
            .map(|t| format!("{}.{}", t.key, t.ext))
            .collect();
        if let Ok(rd) = fs::read_dir(self.tracks_dir()).map(|rd| rd.filter(|_| trusted)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !names.contains(&name) {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        let changed = kept.len() != before;
        (kept, changed)
    }

    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Add (or replace) a finished download. The file must already be in place.
    pub(crate) fn add(&self, t: OfflineTrack) -> Result<(), ApiError> {
        let mut st = self.state.lock().unwrap();
        st.tracks.retain(|x| x.key != t.key);
        if st.tracks.len() >= index::MAX_TRACKS {
            return Err(ApiError::invalid("too many downloads"));
        }
        st.tracks.push(t);
        self.save_index(&st.tracks)
    }

    /// The finished file for `key`, if the device has it.
    pub(crate) fn file_for(&self, key: &str) -> Option<(OfflineTrack, PathBuf)> {
        let st = self.state.lock().unwrap();
        let t = st.tracks.iter().find(|t| t.key == key)?.clone();
        let p = self.file_of(&t);
        Some((t, p))
    }

    /// The downloaded track with this LMS id, if any.
    pub(crate) fn track_by_id(&self, id: i64) -> Option<OfflineTrack> {
        let st = self.state.lock().unwrap();
        st.tracks.iter().find(|t| t.track_id == id).cloned()
    }

    /// Where a finished download goes (it is moved here from its partial file).
    pub(crate) fn final_path(&self, key: &str, ext: &str) -> PathBuf {
        self.tracks_dir().join(format!("{key}.{ext}"))
    }

    pub(crate) fn has_art(&self, album_ref: &str) -> bool {
        self.art_path(album_ref).is_some_and(|p| p.is_file())
    }

    /// Cache an album cover (checked to be an image of a sane size).
    pub(crate) fn save_art(&self, album_ref: &str, bytes: &[u8]) -> bool {
        let Some(p) = self.art_path(album_ref) else {
            return false;
        };
        if bytes.len() > MAX_ART_BYTES || sniff_image(bytes).is_none() {
            return false;
        }
        write_atomic(&p, bytes, false).is_ok()
    }

    /// Record a play heard offline (`None` if too short to count). The offline player
    /// calls this; it is public for hosts' tests.
    pub fn record_play(
        &self,
        track_id: i64,
        track_key: &str,
        played_at: u64,
        seconds: u32,
    ) -> Option<PlayRecord> {
        let mut st = self.state.lock().unwrap();
        let p = plays::record(&mut st.plays, track_id, track_key, played_at, seconds)?;
        self.save_plays(&st.plays);
        Some(p)
    }

    pub(crate) fn plays(&self) -> Vec<PlayRecord> {
        self.state.lock().unwrap().plays.clone()
    }

    pub(crate) fn drop_plays(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        let mut st = self.state.lock().unwrap();
        st.plays.retain(|p| !keys.contains(&p.key));
        self.save_plays(&st.plays);
    }

    /// Smart downloads that may be removed to make room, oldest first.
    pub(crate) fn evictable(&self) -> Vec<OfflineTrack> {
        let mut v: Vec<OfflineTrack> = self
            .state
            .lock()
            .unwrap()
            .tracks
            .iter()
            .filter(|t| t.origin.evictable())
            .cloned()
            .collect();
        v.sort_by_key(|t| t.downloaded_at);
        v
    }

    /// Change a track's origin (a smart download the user then chose keeps it for good).
    pub(crate) fn set_origin(&self, key: &str, origin: DownloadOrigin) {
        let mut st = self.state.lock().unwrap();
        let mut changed = false;
        for t in st
            .tracks
            .iter_mut()
            .filter(|t| t.key == key && t.origin != origin)
        {
            t.origin = origin;
            changed = true;
        }
        if changed {
            let _ = self.save_index(&st.tracks);
        }
    }

    /// Bytes in partial downloads.
    pub(crate) fn partial_bytes(&self) -> u64 {
        fs::read_dir(self.partial_dir())
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    fn remove_where(&self, pred: impl Fn(&OfflineTrack) -> bool) -> Result<u32, ApiError> {
        let mut st = self.state.lock().unwrap();
        let (gone, kept): (Vec<_>, Vec<_>) = st.tracks.drain(..).partition(|t| pred(t));
        st.tracks = kept;
        for t in &gone {
            let _ = fs::remove_file(self.file_of(t));
        }
        // Covers no remaining track uses.
        let refs: std::collections::HashSet<String> =
            st.tracks.iter().map(OfflineTrack::album_ref).collect();
        for t in &gone {
            let r = t.album_ref();
            if !refs.contains(&r)
                && let Some(p) = self.art_path(&r)
            {
                let _ = fs::remove_file(p);
            }
        }
        self.save_index(&st.tracks)?;
        Ok(gone.len() as u32)
    }
}

#[uniffi::export]
impl OfflineLibrary {
    /// Open (creating if needed) the downloads of `username` on `server_url`, under the
    /// app-private folder `root`. Cleans up after any crash: entries without their file,
    /// and files without an entry, are removed.
    #[uniffi::constructor]
    pub fn open(root: String, server_url: String, username: String) -> Result<Arc<Self>, ApiError> {
        let dir = scope_dir(&root, &server_url, &username)?;
        let lib = OfflineLibrary {
            dir,
            state: Mutex::new(State {
                tracks: Vec::new(),
                plays: Vec::new(),
                ratings: Vec::new(),
            }),
        };
        lib.create_dirs()?;
        let (loaded, trusted, recovered) = lib.load_index();
        let (tracks, changed) = lib.reconcile(loaded, trusted);
        if changed || recovered {
            lib.save_index(&tracks)?;
        }
        let plays = read_limited(&lib.plays_path(), 8 * 1024 * 1024)
            .map(|b| plays::parse(&b))
            .unwrap_or_default();
        let ratings = read_limited(&lib.ratings_path(), 4 * 1024 * 1024)
            .map(|b| crate::routine::parse_queue(&b))
            .unwrap_or_default();
        *lib.state.lock().unwrap() = State {
            tracks,
            plays,
            ratings,
        };
        Ok(Arc::new(lib))
    }

    /// Rate a downloaded track while offline (0 clears): kept until [`crate::sync_ratings`]
    /// sends it. The newest rating of a track replaces an older one. False if the values
    /// are out of range.
    pub fn rate_offline(&self, track_id: i64, track_key: String, rating: u32, at: u64) -> bool {
        let mut st = self.state.lock().unwrap();
        let ok = crate::routine::queue(
            &mut st.ratings,
            crate::routine::QueuedRating {
                track_id,
                track_key,
                rating,
                at,
            },
        );
        if ok {
            self.save_ratings(&st.ratings);
        }
        ok
    }

    /// Ratings made offline and not yet sent.
    pub fn pending_ratings(&self) -> u32 {
        self.state.lock().unwrap().ratings.len() as u32
    }

    /// Every downloaded track.
    pub fn tracks(&self) -> Vec<OfflineTrack> {
        self.state.lock().unwrap().tracks.clone()
    }

    /// Downloaded albums, most recently downloaded first.
    pub fn albums(&self) -> Vec<OfflineAlbum> {
        let tracks = self.tracks();
        let mut by: HashMap<String, Vec<&OfflineTrack>> = HashMap::new();
        for t in &tracks {
            by.entry(t.album_ref()).or_default().push(t);
        }
        let mut out: Vec<OfflineAlbum> = by
            .into_iter()
            .map(|(r, ts)| {
                let first = ts[0];
                let same_artist = ts.iter().all(|t| t.artist == first.artist);
                OfflineAlbum {
                    has_art: self.has_art(&r),
                    album_ref: r,
                    album_id: first.album_id,
                    title: first.album.clone(),
                    artist: if same_artist {
                        first.artist.clone()
                    } else {
                        String::new()
                    },
                    track_count: ts.len() as u32,
                    size: ts.iter().map(|t| t.size).sum(),
                    downloaded_at: ts.iter().map(|t| t.downloaded_at).max().unwrap_or(0),
                    origin: if ts.iter().any(|t| t.origin == DownloadOrigin::Manual) {
                        DownloadOrigin::Manual
                    } else {
                        first.origin
                    },
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.downloaded_at
                .cmp(&a.downloaded_at)
                .then_with(|| a.title.cmp(&b.title))
        });
        out
    }

    /// An album's tracks, in track order.
    pub fn album_tracks(&self, album_ref: String) -> Vec<OfflineTrack> {
        let mut v: Vec<OfflineTrack> = self
            .tracks()
            .into_iter()
            .filter(|t| t.album_ref() == album_ref)
            .collect();
        v.sort_by(|a, b| {
            a.track_number
                .unwrap_or(u32::MAX)
                .cmp(&b.track_number.unwrap_or(u32::MAX))
                .then_with(|| a.title.cmp(&b.title))
        });
        v
    }

    /// Which of these LMS track ids are on the device (for "downloaded" marks).
    pub fn downloaded_ids(&self, ids: Vec<i64>) -> Vec<i64> {
        let st = self.state.lock().unwrap();
        ids.into_iter()
            .filter(|id| st.tracks.iter().any(|t| t.track_id == *id))
            .collect()
    }

    /// Bytes used: finished downloads and those in progress.
    pub fn used_bytes(&self) -> u64 {
        let done: u64 = self
            .state
            .lock()
            .unwrap()
            .tracks
            .iter()
            .map(|t| t.size)
            .sum();
        done + self.partial_bytes()
    }

    /// An album's cached cover, if any.
    pub fn artwork(&self, album_ref: String) -> Option<ArtImage> {
        let p = self.art_path(&album_ref)?;
        let bytes = read_limited(&p, MAX_ART_BYTES as u64)?;
        let mime = sniff_image(&bytes)?;
        Some(ArtImage {
            bytes,
            mime_type: mime.to_string(),
        })
    }

    /// Remove downloaded tracks by stable key. Returns how many went.
    pub fn remove(&self, keys: Vec<String>) -> Result<u32, ApiError> {
        self.remove_where(|t| keys.contains(&t.key))
    }

    /// Remove a downloaded album.
    pub fn remove_album(&self, album_ref: String) -> Result<u32, ApiError> {
        self.remove_where(|t| t.album_ref() == album_ref)
    }

    /// Remove every download (finished, in progress and queued) and cached cover of this
    /// user on this server. Plays not yet sent are kept.
    pub fn remove_all(&self) -> Result<(), ApiError> {
        let mut st = self.state.lock().unwrap();
        st.tracks.clear();
        for d in [self.tracks_dir(), self.partial_dir(), self.art_dir()] {
            let _ = fs::remove_dir_all(&d);
        }
        let _ = fs::remove_file(self.queue_path());
        let _ = fs::remove_file(self.index_path().with_extension("json.bak"));
        let _ = fs::remove_file(self.unreadable_index_path());
        self.save_index(&st.tracks)?;
        drop(st);
        self.create_dirs()
    }

    /// Plays recorded offline and not yet sent.
    pub fn pending_plays(&self) -> u32 {
        self.state.lock().unwrap().plays.len() as u32
    }
}

/// Delete everything kept for `username` on `server_url` (downloads, covers and unsent
/// plays): a revoked device, or a user who chose to remove their downloads at sign-out
/// without a library open.
#[uniffi::export]
pub fn remove_offline_data(
    root: String,
    server_url: String,
    username: String,
) -> Result<(), ApiError> {
    let dir = scope_dir(&root, &server_url, &username)?;
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error("could not remove the downloads", e)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::offline::index::tests::sample;

    pub(crate) fn temp_root() -> String {
        let d = std::env::temp_dir().join(format!("anywhere-offline-{}", super::plays::new_key()));
        fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    fn put(lib: &OfflineLibrary, t: &OfflineTrack) {
        fs::write(lib.final_path(&t.key, &t.ext), vec![7u8; t.size as usize]).unwrap();
        lib.add(t.clone()).unwrap();
    }

    #[test]
    fn scopes_are_hashed_per_user_and_server() {
        let a = scope_name("https://music.example.com", "alice");
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, scope_name("https://music.example.com", "bob"));
        assert_ne!(a, scope_name("https://lms.example.lan", "alice"));
        assert!(!a.contains("alice"));
        assert!(scope_dir("relative/dir", "https://x", "a").is_err());
        assert!(scope_dir("/tmp/../etc", "https://x", "a").is_err());
        assert!(scope_dir("/tmp", "https://x", "").is_err());
    }

    #[test]
    fn tracks_persist_and_group_into_albums() {
        let root = temp_root();
        let lib = OfflineLibrary::open(
            root.clone(),
            "https://music.example.com".into(),
            "alice".into(),
        )
        .unwrap();
        let (a, mut b, mut c) = (sample(1), sample(2), sample(3));
        b.artist = "Other".into();
        c.album_id = Some(9);
        c.album = "Second".into();
        for t in [&a, &b, &c] {
            put(&lib, t);
        }
        assert_eq!(lib.used_bytes(), a.size + b.size + c.size);
        let again = OfflineLibrary::open(
            root.clone(),
            "https://music.example.com".into(),
            "alice".into(),
        )
        .unwrap();
        assert_eq!(again.tracks().len(), 3);
        let albums = again.albums();
        assert_eq!(albums.len(), 2);
        let first = albums.iter().find(|x| x.album_ref == "a7").unwrap();
        assert_eq!(first.track_count, 2);
        assert_eq!(first.artist, "", "mixed artists");
        assert_eq!(
            again
                .album_tracks("a7".into())
                .iter()
                .map(|t| t.track_number)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2)]
        );
        assert_eq!(again.downloaded_ids(vec![101, 102, 555]), vec![101, 102]);
        // Another user on the same server sees nothing.
        let bob =
            OfflineLibrary::open(root, "https://music.example.com".into(), "bob".into()).unwrap();
        assert!(bob.tracks().is_empty());
    }

    #[test]
    fn crash_leftovers_are_cleaned_up() {
        let root = temp_root();
        let lib = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        let (a, b) = (sample(1), sample(2));
        put(&lib, &a);
        put(&lib, &b);
        // b's file is damaged (wrong size); a stray file appears.
        fs::write(lib.final_path(&b.key, &b.ext), b"short").unwrap();
        let stray = lib.tracks_dir().join("stray.flac");
        fs::write(&stray, b"x").unwrap();
        let again = OfflineLibrary::open(root, "https://s".into(), "u".into()).unwrap();
        assert_eq!(again.tracks(), vec![a]);
        assert!(!stray.exists());
        assert!(!again.final_path(&b.key, &b.ext).exists());
    }

    #[test]
    fn a_corrupt_index_falls_back_to_its_backup() {
        let root = temp_root();
        let lib = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        let (a, b) = (sample(1), sample(2));
        put(&lib, &a);
        put(&lib, &b); // the backup now lists a only
        fs::write(lib.index_path(), b"{\"v\":1,\"tracks\":[{\"key\":").unwrap();
        let again = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        assert_eq!(again.tracks(), vec![a.clone()]);
        assert!(
            again.final_path(&b.key, &b.ext).exists(),
            "an index that can't be read never deletes files"
        );
    }

    #[test]
    fn an_unreadable_index_never_deletes_the_downloads() {
        // Regression: a damaged (or newer-format, or briefly unreadable) index and backup
        // used to make every downloaded file an "orphan", deleted on opening.
        let root = temp_root();
        let lib = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        let (a, b) = (sample(1), sample(2));
        put(&lib, &a);
        put(&lib, &b);
        let good = fs::read(lib.index_path()).unwrap();
        for bad in [&b"garbage"[..], br#"{"v":99,"tracks":[]}"#] {
            fs::write(lib.index_path(), bad).unwrap();
            fs::write(lib.index_path().with_extension("json.bak"), b"garbage").unwrap();
            let again = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
            assert!(again.tracks().is_empty());
            assert!(again.final_path(&a.key, &a.ext).exists());
            assert!(again.final_path(&b.key, &b.ext).exists());
            // Downloading more, then opening again, still keeps them while the copy kept
            // aside can't be read.
            let c = sample(3);
            put(&again, &c);
            let later = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
            assert_eq!(later.tracks(), vec![c.clone()]);
            assert!(later.final_path(&a.key, &a.ext).exists());
            later.remove(vec![c.key.clone()]).unwrap();
            let _ = fs::remove_file(lib.unreadable_index_path());
            fs::write(lib.index_path(), &good).unwrap();
        }
        // A passing read error: the copy kept aside is read back later, nothing lost.
        fs::write(lib.unreadable_index_path(), &good).unwrap();
        fs::write(lib.index_path(), br#"{"v":1,"tracks":[]}"#).unwrap();
        let back = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        let mut keys: Vec<String> = back.tracks().into_iter().map(|t| t.key).collect();
        keys.sort();
        assert_eq!(keys, vec![a.key.clone(), b.key.clone()]);
        assert!(!lib.unreadable_index_path().exists());
        // Removing everything clears the kept copy too.
        fs::write(lib.unreadable_index_path(), b"garbage").unwrap();
        back.remove_all().unwrap();
        assert!(!lib.unreadable_index_path().exists());
    }

    #[test]
    fn removing_tracks_albums_and_everything() {
        let root = temp_root();
        let lib = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        let (a, b, mut c) = (sample(1), sample(2), sample(3));
        c.album_id = Some(9);
        for t in [&a, &b, &c] {
            put(&lib, t);
        }
        assert!(lib.save_art("a7", b"\xff\xd8\xff\xe0 jpeg"));
        assert!(lib.save_art("a9", b"\x89PNG\r\n\x1a\n...."));
        assert!(!lib.save_art("a9", b"<svg onload=x>"), "only real images");
        assert!(!lib.save_art("../x", b"\xff\xd8\xff"));
        assert_eq!(lib.artwork("a7".into()).unwrap().mime_type, "image/jpeg");
        assert_eq!(lib.remove(vec![a.key.clone()]).unwrap(), 1);
        assert!(lib.has_art("a7"), "b still uses the cover");
        assert_eq!(lib.remove_album("a7".into()).unwrap(), 1);
        assert!(!lib.has_art("a7"));
        assert!(!lib.final_path(&b.key, &b.ext).exists());
        lib.record_play(103, "", 1, 60).unwrap();
        lib.remove_all().unwrap();
        assert!(lib.tracks().is_empty());
        assert!(lib.artwork("a9".into()).is_none());
        assert_eq!(
            lib.pending_plays(),
            1,
            "unsent plays survive removing downloads"
        );
        // Revocation removes everything, plays included.
        remove_offline_data(root.clone(), "https://s".into(), "u".into()).unwrap();
        assert!(!lib.dir().exists());
        remove_offline_data(root, "https://s".into(), "u".into()).unwrap();
    }

    #[test]
    fn plays_are_durable() {
        let root = temp_root();
        let lib = OfflineLibrary::open(root.clone(), "https://s".into(), "u".into()).unwrap();
        assert!(lib.record_play(5, "", 1, 2).is_none(), "too short");
        let p = lib.record_play(5, &"cd".repeat(32), 1000, 90).unwrap();
        let again = OfflineLibrary::open(root, "https://s".into(), "u".into()).unwrap();
        assert_eq!(again.plays(), vec![p.clone()]);
        again.drop_plays(&[p.key]);
        assert_eq!(again.pending_plays(), 0);
    }

    #[test]
    fn album_refs_are_validated() {
        for ok in ["a1", "a123456789012", "t0123456789abcdef"] {
            assert!(valid_album_ref(ok), "{ok}");
        }
        for bad in [
            "",
            "a",
            "a1x",
            "a1234567890123",
            "t0123",
            "../a",
            "a-1",
            "é1",
            "tABCDEF0123456789",
        ] {
            assert!(!valid_album_ref(bad), "{bad}");
        }
    }
}
