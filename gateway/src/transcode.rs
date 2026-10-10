//! Transcoded downloads (spec D10): the system `ffmpeg`, and a size-capped LRU cache of its
//! results on disk.
//!
//! ffmpeg reads untrusted bytes (a library file can contain anything), and ffmpeg can open
//! other files and network URLs on its own: playlist demuxers (HLS, concat, ffconcat) follow
//! references, `-i` accepts any protocol, and MOV reference atoms point elsewhere. So it is
//! run defensively:
//! - with a fixed argument list, never through a shell, and with an empty environment;
//! - the input demuxer is forced from a fixed table keyed by LMS's file type, so playlist
//!   and concat demuxers can never be chosen by probing the content;
//! - input may use only the `file` protocol (`-protocol_whitelist file`), and the input and
//!   output are files the gateway itself named inside its cache directory;
//! - MOV external references stay off (`-enable_drefs 0`);
//! - the output size is capped (`-fs`), the run time is limited, and only a few jobs run at
//!   once, so `ffmpeg` can't starve LMS.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Bumped whenever the encoder settings change, so old cache entries are not reused.
pub const PROFILE: &str = "v1";

/// A download format a client can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// The library file as it is.
    Original,
    /// MP3, 320 kb/s constant bit rate (plays everywhere).
    Mp3,
    /// Opus, 160 kb/s, in an Ogg container (smaller, transparent for most listeners).
    Opus,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "original" => Some(Format::Original),
            "mp3" => Some(Format::Mp3),
            "opus" => Some(Format::Opus),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Format::Original => "original",
            Format::Mp3 => "mp3",
            Format::Opus => "opus",
        }
    }

    /// File extension and content type of a transcode (not meaningful for the original).
    pub fn ext(self) -> &'static str {
        match self {
            Format::Original => "bin",
            Format::Mp3 => "mp3",
            Format::Opus => "opus",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Format::Original => "application/octet-stream",
            Format::Mp3 => "audio/mpeg",
            Format::Opus => "audio/ogg",
        }
    }
}

/// The system ffmpeg and the encoders it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    pub path: PathBuf,
    pub mp3: bool,
    pub opus: bool,
}

impl Ffmpeg {
    pub fn supports(&self, f: Format) -> bool {
        match f {
            Format::Original => false,
            Format::Mp3 => self.mp3,
            Format::Opus => self.opus,
        }
    }
}

/// A regular file with an execute bit.
pub fn is_executable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else {
        return false;
    };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `ffmpeg` in an absolute directory on `PATH`, if there is one.
pub fn find_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::split_paths(&path)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

/// Which of the encoders we use an `ffmpeg -encoders` listing includes: (mp3, opus).
pub fn encoders(listing: &str) -> (bool, bool) {
    let mut found = (false, false);
    for line in listing.lines() {
        let mut words = line.split_whitespace();
        let (Some(flags), Some(name)) = (words.next(), words.next()) else {
            continue;
        };
        if flags.len() != 6 || !flags.starts_with('A') {
            continue;
        }
        match name {
            "libmp3lame" => found.0 = true,
            "libopus" => found.1 = true,
            _ => {}
        }
    }
    found
}

fn command(path: &Path) -> std::process::Command {
    let mut c = std::process::Command::new(path);
    c.env_clear();
    // Windows programs need SystemRoot to start at all.
    #[cfg(windows)]
    if let Some(v) = std::env::var_os("SystemRoot") {
        c.env("SystemRoot", v);
    }
    c
}

/// Run `ffmpeg -encoders` to see what it can do. `None` if it can't be run in time or has
/// neither encoder.
pub fn probe(path: &Path, limit: Duration) -> Option<Ffmpeg> {
    use std::io::Read;
    if !path.is_absolute() || !is_executable(path) {
        return None;
    }
    let mut child = command(path)
        .args(["-hide_banner", "-nostdin", "-encoders"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.take(1 << 20).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let listing = rx.recv_timeout(limit).ok();
    let _ = child.kill();
    let _ = child.wait();
    let (mp3, opus) = encoders(&String::from_utf8_lossy(&listing?));
    (mp3 || opus).then(|| Ffmpeg {
        path: path.to_path_buf(),
        mp3,
        opus,
    })
}

/// The ffmpeg demuxer for an LMS file type, from a fixed table. Types not listed are never
/// transcoded (only the original is offered).
pub fn demuxer(lms_type: &str) -> Option<&'static str> {
    Some(match lms_type {
        "flc" => "flac",
        "mp3" => "mp3",
        "mp4" | "alc" | "m4a" => "mov",
        "aac" => "aac",
        "ogg" | "ops" | "ogf" => "ogg",
        "wav" => "wav",
        "aif" => "aiff",
        "wma" => "asf",
        "wvp" => "wv",
        "ape" => "ape",
        "dsf" => "dsf",
        _ => return None,
    })
}

/// Tags written into a transcode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track: Option<i64>,
}

/// One transcode.
pub struct Job<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub demuxer: &'static str,
    pub format: Format,
    pub tags: &'a Tags,
    /// Output size limit in bytes.
    pub max_bytes: u64,
}

/// A tag value as one safe argument: no control characters, bounded length.
fn tag_value(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(500).collect()
}

fn file_url(p: &Path) -> OsString {
    let mut s = OsString::from("file:");
    s.push(p.as_os_str());
    s
}

/// ffmpeg's argument list for a job (no shell is ever involved).
pub fn args(job: &Job) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    fn words(a: &mut Vec<OsString>, items: &[&str]) {
        a.extend(items.iter().map(OsString::from));
    }
    words(
        &mut a,
        &["-hide_banner", "-nostdin", "-loglevel", "error", "-y"],
    );
    words(&mut a, &["-protocol_whitelist", "file", "-f", job.demuxer]);
    if job.demuxer == "mov" {
        words(&mut a, &["-enable_drefs", "0"]);
    }
    a.push("-i".into());
    a.push(file_url(job.input));
    words(&mut a, &["-map", "0:a:0"]);
    words(&mut a, &["-map_metadata", "-1", "-map_chapters", "-1"]);
    words(&mut a, &["-af", "aformat=channel_layouts=mono|stereo"]);
    match job.format {
        Format::Mp3 => {
            // Embedded cover art, when the file has any, is kept as the ID3 picture.
            words(
                &mut a,
                &[
                    "-map",
                    "0:v:0?",
                    "-c:v",
                    "copy",
                    "-disposition:v",
                    "attached_pic",
                ],
            );
            words(
                &mut a,
                &["-c:a", "libmp3lame", "-b:a", "320k", "-id3v2_version", "3"],
            );
            words(&mut a, &["-f", "mp3"]);
        }
        Format::Opus | Format::Original => {
            words(
                &mut a,
                &["-vn", "-c:a", "libopus", "-b:a", "160k", "-f", "ogg"],
            );
        }
    }
    let t = job.tags;
    for (k, v) in [
        ("title", &t.title),
        ("artist", &t.artist),
        ("album", &t.album),
    ] {
        if !v.is_empty() {
            a.push("-metadata".into());
            a.push(format!("{k}={}", tag_value(v)).into());
        }
    }
    if let Some(n) = t.track.filter(|n| *n > 0) {
        a.push("-metadata".into());
        a.push(format!("track={n}").into());
    }
    a.push("-threads".into());
    a.push("1".into());
    a.push("-fs".into());
    a.push(job.max_bytes.to_string().into());
    a.push(file_url(job.output));
    a
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    TimedOut,
    Failed,
}

/// Run ffmpeg with a time limit; it is killed if it overruns (or if this future is dropped).
pub async fn run(
    ffmpeg: &Path,
    args: Vec<OsString>,
    cwd: &Path,
    limit: Duration,
) -> Result<(), RunError> {
    let mut cmd = tokio::process::Command::from(command(ffmpeg));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|_| RunError::Failed)?;
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(st)) if st.success() => Ok(()),
        Ok(_) => Err(RunError::Failed),
        Err(_) => {
            let _ = child.kill().await;
            Err(RunError::TimedOut)
        }
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// Cache file names: a hex SHA-256 and a transcode extension. Nothing else in the cache
/// directory is ever served.
pub fn valid_cache_name(name: &str) -> bool {
    match name.split_once('.') {
        Some((hash, ext)) => {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && matches!(ext, "mp3" | "opus")
        }
        None => false,
    }
}

const TEMP_PREFIX: &str = ".tmp-";

#[derive(Default)]
struct Index {
    /// name → (size, last use)
    entries: HashMap<String, (u64, u64)>,
    total: u64,
    clock: u64,
}

/// Transcodes on disk, at most `cap` bytes in all; the least recently used go first.
pub struct Cache {
    dir: PathBuf,
    cap: u64,
    index: Mutex<Index>,
}

impl Cache {
    /// Open (creating it, owner-only) a cache directory, forgetting unfinished work and
    /// trimming it to the cap.
    pub fn open(dir: &Path, cap: u64) -> std::io::Result<Cache> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut found: Vec<(SystemTime, String, u64)> = Vec::new();
        for entry in std::fs::read_dir(dir)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata() else { continue };
            if name.starts_with(TEMP_PREFIX) && meta.is_file() {
                let _ = std::fs::remove_file(entry.path());
            } else if valid_cache_name(&name) && meta.is_file() {
                let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((used, name, meta.len()));
            }
        }
        found.sort();
        let cache = Cache {
            dir: dir.to_path_buf(),
            cap,
            index: Mutex::new(Index::default()),
        };
        {
            let mut ix = cache.index.lock().unwrap();
            for (_, name, size) in found {
                ix.clock += 1;
                let tick = ix.clock;
                ix.total += size;
                ix.entries.insert(name, (size, tick));
            }
        }
        cache.evict(None);
        Ok(cache)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn cap(&self) -> u64 {
        self.cap
    }

    /// Bytes held.
    pub fn total(&self) -> u64 {
        self.index.lock().unwrap().total
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.lock().unwrap().entries.contains_key(name)
    }

    /// A cached transcode, marked as just used.
    pub fn lookup(&self, name: &str) -> Option<PathBuf> {
        if !valid_cache_name(name) {
            return None;
        }
        let path = self.dir.join(name);
        let mut ix = self.index.lock().unwrap();
        if !ix.entries.contains_key(name) {
            return None;
        }
        if !path.is_file() {
            if let Some((size, _)) = ix.entries.remove(name) {
                ix.total -= size;
            }
            return None;
        }
        ix.clock += 1;
        let tick = ix.clock;
        if let Some(e) = ix.entries.get_mut(name) {
            e.1 = tick;
        }
        drop(ix);
        // Keep the order across restarts too (best effort).
        if let Ok(f) = std::fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(path)
    }

    /// Record a finished file (already at `dir/name`), then evict others to fit. A file
    /// bigger than the whole cache is not kept. Returns whether it was kept.
    pub fn insert(&self, name: &str, size: u64) -> bool {
        let path = self.dir.join(name);
        if !valid_cache_name(name) || size > self.cap {
            let _ = std::fs::remove_file(&path);
            return false;
        }
        {
            let mut ix = self.index.lock().unwrap();
            ix.clock += 1;
            let tick = ix.clock;
            if let Some((old, _)) = ix.entries.insert(name.to_string(), (size, tick)) {
                ix.total -= old;
            }
            ix.total += size;
        }
        self.evict(Some(name));
        true
    }

    /// Remove least recently used entries (never `keep`) until the cache fits.
    fn evict(&self, keep: Option<&str>) {
        let mut ix = self.index.lock().unwrap();
        while ix.total > self.cap {
            let victim = ix
                .entries
                .iter()
                .filter(|(n, _)| Some(n.as_str()) != keep)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(n, _)| n.clone());
            let Some(victim) = victim else { break };
            if let Some((size, _)) = ix.entries.remove(&victim) {
                ix.total -= size;
            }
            let _ = std::fs::remove_file(self.dir.join(&victim));
        }
    }

    /// A fresh temporary path in the cache directory (removed at next start if left over).
    pub fn temp_path(&self, ext: &str) -> PathBuf {
        self.dir
            .join(format!("{TEMP_PREFIX}{}.{ext}", crate::random_id()))
    }
}

/// ffmpeg, the cache, and the limits on transcoding work.
pub struct Transcoder {
    pub ffmpeg: Ffmpeg,
    pub cache: Cache,
    pub jobs: tokio::sync::Semaphore,
    pub timeout: Duration,
    /// One transcode per cache entry at a time: later requests wait for the first.
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Transcoder {
    pub fn new(ffmpeg: Ffmpeg, cache: Cache, jobs: usize, timeout: Duration) -> Transcoder {
        Transcoder {
            ffmpeg,
            cache,
            jobs: tokio::sync::Semaphore::new(jobs.max(1)),
            timeout,
            inflight: Mutex::new(HashMap::new()),
        }
    }

    pub fn lock_for(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut m = self.inflight.lock().unwrap();
        // Forget locks nobody holds any more.
        m.retain(|_, l| Arc::strong_count(l) > 1);
        m.entry(name.to_string()).or_default().clone()
    }

    /// The largest transcode accepted (and the `-fs` limit given to ffmpeg).
    pub fn max_output(&self) -> u64 {
        self.cache.cap().min(2 << 30)
    }
}

/// The cache entry name for a track in a format: a hash of its stable key and of what
/// identifies this version of the file.
pub fn cache_name(key: &str, format: Format, size: Option<u64>, mtime: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [
        key,
        format.as_str(),
        PROFILE,
        &size.map(|s| s.to_string()).unwrap_or_default(),
        mtime,
    ] {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    format!("{}.{}", hex::encode(h.finalize()), format.ext())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anywhere-cache-{tag}-{}", crate::random_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn name(i: u8) -> String {
        format!("{}.mp3", hex::encode([i; 32]))
    }

    fn put(c: &Cache, n: &str, size: usize) -> bool {
        std::fs::write(c.dir().join(n), vec![0u8; size]).unwrap();
        c.insert(n, size as u64)
    }

    #[test]
    fn formats_parse_strictly() {
        for f in [Format::Original, Format::Mp3, Format::Opus] {
            assert_eq!(Format::parse(f.as_str()), Some(f));
        }
        for bad in ["", "MP3", "orig", "flac", "opus ", "mp3\0", "../mp3"] {
            assert_eq!(Format::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_least_recently_used_are_evicted_to_fit() {
        let dir = tmpdir("lru");
        let c = Cache::open(&dir, 100).unwrap();
        assert!(put(&c, &name(1), 40));
        assert!(put(&c, &name(2), 40));
        assert!(c.lookup(&name(1)).is_some(), "1 is now the most recent");
        assert!(put(&c, &name(3), 40));
        assert!(c.contains(&name(1)));
        assert!(!c.contains(&name(2)), "2 was least recently used");
        assert!(!dir.join(name(2)).exists(), "and its file is gone");
        assert!(c.contains(&name(3)));
        assert_eq!(c.total(), 80);
        // Bigger than the whole cache: not kept, nothing else evicted.
        assert!(!put(&c, &name(4), 101));
        assert!(!dir.join(name(4)).exists());
        assert_eq!(c.total(), 80);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reopening_keeps_entries_drops_leftovers_and_trims() {
        let dir = tmpdir("reopen");
        {
            let c = Cache::open(&dir, 1000).unwrap();
            put(&c, &name(1), 300);
            put(&c, &name(2), 300);
        }
        std::fs::write(dir.join(".tmp-abc.in"), b"partial").unwrap();
        std::fs::write(dir.join("not-a-cache-file.mp3"), b"x").unwrap();
        let c = Cache::open(&dir, 1000).unwrap();
        assert_eq!(c.total(), 600);
        assert!(!dir.join(".tmp-abc.in").exists(), "unfinished work removed");
        assert!(
            c.lookup("not-a-cache-file.mp3").is_none(),
            "foreign files never served"
        );
        let small = Cache::open(&dir, 400).unwrap();
        assert!(small.total() <= 400, "trimmed to a smaller cap");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_deleted_file_is_a_miss() {
        let dir = tmpdir("gone");
        let c = Cache::open(&dir, 1000).unwrap();
        put(&c, &name(1), 10);
        std::fs::remove_file(dir.join(name(1))).unwrap();
        assert!(c.lookup(&name(1)).is_none());
        assert_eq!(c.total(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_names_are_strict() {
        assert!(valid_cache_name(&name(1)));
        assert!(valid_cache_name(&format!("{}.opus", "a".repeat(64))));
        for bad in [
            String::new(),
            "x.mp3".into(),
            format!("{}.flac", "a".repeat(64)),
            format!("{}.mp3", "A".repeat(64)),
            format!("../{}.mp3", "a".repeat(61)),
            format!("{}.mp3.mp3", "a".repeat(64)),
            format!(".tmp-{}.mp3", "a".repeat(59)),
        ] {
            assert!(!valid_cache_name(&bad), "{bad}");
        }
    }

    #[test]
    fn cache_names_change_with_the_file_and_format() {
        let a = cache_name("k", Format::Mp3, Some(10), "t1");
        assert!(valid_cache_name(&a));
        assert_ne!(a, cache_name("k", Format::Opus, Some(10), "t1"));
        assert_ne!(a, cache_name("k", Format::Mp3, Some(11), "t1"));
        assert_ne!(a, cache_name("k", Format::Mp3, Some(10), "t2"));
        assert_ne!(a, cache_name("k2", Format::Mp3, Some(10), "t1"));
        assert_eq!(a, cache_name("k", Format::Mp3, Some(10), "t1"));
    }

    #[test]
    fn encoder_listing_is_parsed() {
        let listing = "Encoders:\n V..... = Video\n ------\n A....D libmp3lame           MP3\n A....D libopus              Opus\n A....D aac   AAC\n";
        assert_eq!(encoders(listing), (true, true));
        assert_eq!(encoders(" A....D aac AAC\n"), (false, false));
        assert_eq!(
            encoders(" V....D libmp3lame x\n"),
            (false, false),
            "not an audio encoder"
        );
        assert_eq!(encoders("libopus\n"), (false, false));
        assert_eq!(encoders(""), (false, false));
    }

    #[test]
    fn encoder_listing_parser_never_panics() {
        let mut seed = 0x9e3779b97f4a7c15u64;
        for _ in 0..2000 {
            let mut b = Vec::new();
            for _ in 0..(seed % 200) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                b.push(b" A.D\nlibopus\t\x00\xff"[(seed % 15) as usize]);
            }
            let _ = encoders(&String::from_utf8_lossy(&b));
        }
    }

    #[test]
    fn only_known_types_are_transcoded() {
        assert_eq!(demuxer("flc"), Some("flac"));
        assert_eq!(demuxer("mp4"), Some("mov"));
        for bad in [
            "", "hls", "concat", "m3u8", "ffconcat", "image2", "flc ", "FLC", "lavfi",
        ] {
            assert_eq!(demuxer(bad), None, "{bad}");
        }
    }

    fn job_args(format: Format, demux: &'static str, tags: &Tags) -> Vec<String> {
        let job = Job {
            input: Path::new("/srv/cache/.tmp-in.src"),
            output: Path::new("/srv/cache/.tmp-out.mp3"),
            demuxer: demux,
            format,
            tags,
            max_bytes: 1000,
        };
        args(&job)
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect()
    }

    #[test]
    fn ffmpeg_arguments_are_locked_down() {
        let tags = Tags {
            title: "Title\r\n-i /etc/passwd".into(),
            artist: "-f concat".into(),
            album: String::new(),
            track: Some(3),
        };
        let a = job_args(Format::Mp3, "flac", &tags);
        let pos = |w: &str| a.iter().position(|x| x == w).unwrap();
        // The input restrictions come before -i, so they apply to it.
        assert!(pos("-protocol_whitelist") < pos("-i"));
        assert_eq!(a[pos("-protocol_whitelist") + 1], "file");
        assert_eq!(a[pos("-f") + 1], "flac", "demuxer forced");
        assert_eq!(a[pos("-i") + 1], "file:/srv/cache/.tmp-in.src");
        assert_eq!(a.last().unwrap(), "file:/srv/cache/.tmp-out.mp3");
        assert!(a.contains(&"-nostdin".to_string()));
        assert_eq!(a[pos("-fs") + 1], "1000");
        assert_eq!(a[pos("-b:a") + 1], "320k");
        assert!(a.contains(&"libmp3lame".to_string()));
        // Hostile tags stay one argument each, without control characters.
        assert!(a.contains(&"title=Title-i /etc/passwd".to_string()));
        assert!(a.contains(&"artist=-f concat".to_string()));
        assert!(a.contains(&"track=3".to_string()));
        assert!(
            !a.iter().any(|x| x.starts_with("album=")),
            "empty tags left out"
        );
        assert!(!a.iter().any(|x| x.chars().any(char::is_control)));
        assert_eq!(
            a.iter().filter(|x| *x == "-i").count(),
            1,
            "exactly one input"
        );

        let o = job_args(Format::Opus, "mov", &Tags::default());
        assert!(o.windows(2).any(|w| w[0] == "-enable_drefs" && w[1] == "0"));
        assert!(o.windows(2).any(|w| w[0] == "-c:a" && w[1] == "libopus"));
        assert!(o.windows(2).any(|w| w[0] == "-b:a" && w[1] == "160k"));
        assert!(o.contains(&"-vn".to_string()));
        assert!(!a.contains(&"-enable_drefs".to_string()));
    }

    #[test]
    fn probing_needs_an_absolute_executable() {
        assert!(probe(Path::new("ffmpeg"), Duration::from_secs(1)).is_none());
        assert!(probe(Path::new("/nonexistent/ffmpeg"), Duration::from_secs(1)).is_none());
        let dir = tmpdir("probe");
        let plain = dir.join("ffmpeg");
        std::fs::write(&plain, "#!/bin/sh\necho ' A..... libopus x'\n").unwrap();
        assert!(
            probe(&plain, Duration::from_secs(1)).is_none(),
            "not executable"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
