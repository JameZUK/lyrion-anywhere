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

/// Which Opus encoder an ffmpeg has. libopus is preferred; ffmpeg's own `opus` encoder is
/// used when libopus is missing, as on piCorePlayer's minimal ffmpeg (issue #29). The built-in
/// one is marked experimental (so it needs `-strict -2`), is CELT-only and sits a little
/// behind libopus at the same rate, which hardly shows at 96 kb/s and above for music.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusEncoder {
    Libopus,
    /// ffmpeg's built-in `opus` encoder.
    Native,
}

impl OpusEncoder {
    /// The encoder's name for `-c:a`.
    pub fn codec(self) -> &'static str {
        match self {
            OpusEncoder::Libopus => "libopus",
            OpusEncoder::Native => "opus",
        }
    }

    /// The name the settings page and the log know it by.
    pub fn label(self) -> &'static str {
        match self {
            OpusEncoder::Libopus => "libopus",
            OpusEncoder::Native => "ffmpeg",
        }
    }

    /// Output options the encoder needs: the built-in one refuses to run without `-strict -2`.
    pub fn options(self) -> &'static [&'static str] {
        match self {
            OpusEncoder::Libopus => &[],
            OpusEncoder::Native => &["-strict", "-2"],
        }
    }
}

/// What a transcode is encoded with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// libmp3lame (ffmpeg has no MP3 encoder of its own).
    Mp3,
    Opus(OpusEncoder),
}

/// The system ffmpeg and the encoders it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    pub path: PathBuf,
    /// It has libmp3lame.
    pub mp3: bool,
    /// The Opus encoder it has, if any.
    pub opus: Option<OpusEncoder>,
}

impl Ffmpeg {
    /// The codec for a format, if this ffmpeg can make it.
    pub fn codec(&self, f: Format) -> Option<Codec> {
        match f {
            Format::Original => None,
            Format::Mp3 => self.mp3.then_some(Codec::Mp3),
            Format::Opus => self.opus.map(Codec::Opus),
        }
    }

    pub fn supports(&self, f: Format) -> bool {
        self.codec(f).is_some()
    }
}

/// Why an ffmpeg can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// Not an absolute path to an executable regular file.
    NotExecutable,
    /// It couldn't be started, or didn't list its encoders within the time limit.
    NotRunnable,
    /// It ran, but has neither an MP3 encoder (libmp3lame) nor an Opus encoder.
    NoEncoders,
}

impl ProbeError {
    /// The reason as the settings page receives it.
    pub fn code(self) -> &'static str {
        match self {
            ProbeError::NotExecutable => "not_executable",
            ProbeError::NotRunnable => "not_runnable",
            ProbeError::NoEncoders => "no_encoders",
        }
    }

    /// The reason in words, for the log.
    pub fn describe(self) -> &'static str {
        match self {
            ProbeError::NotExecutable => "is not an executable file",
            ProbeError::NotRunnable => "could not be run (or did not answer in time)",
            ProbeError::NoEncoders => {
                "has neither an MP3 encoder (libmp3lame) nor an Opus encoder (libopus or opus)"
            }
        }
    }
}

/// What the gateway found when it looked for ffmpeg at start-up: what it transcodes with,
/// and otherwise why it doesn't, for the log and the plugin's settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    /// `--no-transcode`.
    Off,
    /// No `--ffmpeg` given, and none on `PATH`.
    NotFound,
    /// An ffmpeg was named or found, but can't be used.
    Unusable {
        path: PathBuf,
        why: ProbeError,
    },
    Ready(Ffmpeg),
}

impl Setup {
    /// The ffmpeg to transcode with, if there is one.
    pub fn ready(&self) -> Option<&Ffmpeg> {
        match self {
            Setup::Ready(f) => Some(f),
            _ => None,
        }
    }

    /// The set-up for `GET /admin/v1/settings`: `state` is `off`, `not_found`, `unusable`
    /// (with `ffmpeg`, the path, and `why`) or `ready` (with `ffmpeg`, `mp3`, `opus`, the
    /// encoder's name or null, and `live_streams`, whether lower-data streams are possible
    /// given `live_jobs`, the `--live-transcode-jobs` limit).
    pub fn describe(&self, live_jobs: usize) -> serde_json::Value {
        use serde_json::json;
        match self {
            Setup::Off => json!({"state": "off"}),
            Setup::NotFound => json!({"state": "not_found"}),
            Setup::Unusable { path, why } => json!({
                "state": "unusable",
                "ffmpeg": path.to_string_lossy(),
                "why": why.code(),
            }),
            Setup::Ready(f) => json!({
                "state": "ready",
                "ffmpeg": f.path.to_string_lossy(),
                "mp3": f.mp3,
                "opus": f.opus.map(OpusEncoder::label),
                "live_streams": f.opus.is_some() && live_jobs > 0,
            }),
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

/// The encoders we use that an `ffmpeg -encoders` listing includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Encoders {
    pub libmp3lame: bool,
    pub libopus: bool,
    /// ffmpeg's built-in Opus encoder.
    pub opus: bool,
}

impl Encoders {
    /// The Opus encoder to use: libopus when there is one, else ffmpeg's own.
    pub fn opus_encoder(self) -> Option<OpusEncoder> {
        if self.libopus {
            Some(OpusEncoder::Libopus)
        } else if self.opus {
            Some(OpusEncoder::Native)
        } else {
            None
        }
    }
}

/// Which of the encoders we use an `ffmpeg -encoders` listing includes. A line is six flag
/// characters (the first `A` for audio) and the encoder's name; nothing else is read.
pub fn encoders(listing: &str) -> Encoders {
    let mut found = Encoders::default();
    for line in listing.lines() {
        let mut words = line.split_whitespace();
        let (Some(flags), Some(name)) = (words.next(), words.next()) else {
            continue;
        };
        if flags.len() != 6 || !flags.starts_with('A') {
            continue;
        }
        match name {
            "libmp3lame" => found.libmp3lame = true,
            "libopus" => found.libopus = true,
            "opus" => found.opus = true,
            _ => {}
        }
    }
    found
}

/// `ffmpeg` with an empty environment (shared with [`crate::live`] and the tests).
pub fn command(path: &Path) -> std::process::Command {
    let mut c = std::process::Command::new(path);
    c.env_clear();
    // Windows programs need SystemRoot to start at all.
    #[cfg(windows)]
    if let Some(v) = std::env::var_os("SystemRoot") {
        c.env("SystemRoot", v);
    }
    c
}

/// Run `ffmpeg -encoders` to see what it can do.
pub fn probe(path: &Path, limit: Duration) -> Result<Ffmpeg, ProbeError> {
    use std::io::Read;
    if !path.is_absolute() || !is_executable(path) {
        return Err(ProbeError::NotExecutable);
    }
    let mut child = command(path)
        .args(["-hide_banner", "-nostdin", "-encoders"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ProbeError::NotRunnable)?;
    let out = child.stdout.take().ok_or(ProbeError::NotRunnable)?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.take(1 << 20).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let listing = rx.recv_timeout(limit).ok();
    let _ = child.kill();
    let _ = child.wait();
    let listing = listing.ok_or(ProbeError::NotRunnable)?;
    let found = encoders(&String::from_utf8_lossy(&listing));
    let opus = found.opus_encoder();
    if !found.libmp3lame && opus.is_none() {
        return Err(ProbeError::NoEncoders);
    }
    Ok(Ffmpeg {
        path: path.to_path_buf(),
        mp3: found.libmp3lame,
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
    /// What to encode with (from [`Ffmpeg::codec`]).
    pub codec: Codec,
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
    match job.codec {
        Codec::Mp3 => {
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
        Codec::Opus(enc) => {
            words(&mut a, &["-vn", "-c:a", enc.codec()]);
            words(&mut a, enc.options());
            words(&mut a, &["-b:a", "160k", "-f", "ogg"]);
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
    valid_name_with(name, TRANSCODE_EXTS)
}

/// The extensions of transcodes.
pub const TRANSCODE_EXTS: &[&str] = &["mp3", "opus"];

/// A hex SHA-256, a dot, and one of `exts`.
pub fn valid_name_with(name: &str, exts: &[&str]) -> bool {
    match name.split_once('.') {
        Some((hash, ext)) => {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && exts.contains(&ext)
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
    /// Bytes held for work in progress (originals being transcoded, see
    /// [`Cache::reserve`]): they count against the cap as well.
    reserved: u64,
}

/// Transcodes (or other derived files, see [`Cache::open_with`]) on disk, at most `cap`
/// bytes in all; the least recently used go first.
pub struct Cache {
    dir: PathBuf,
    cap: u64,
    exts: &'static [&'static str],
    index: Mutex<Index>,
    /// The cap in units of `unit` bytes: work in progress waits here for room.
    room: tokio::sync::Semaphore,
    unit: u64,
}

/// Room held in a [`Cache`] for work in progress; given back when dropped.
pub struct Reservation<'a> {
    cache: &'a Cache,
    bytes: u64,
    _room: tokio::sync::SemaphorePermit<'a>,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut ix = self.cache.index.lock().unwrap();
        ix.reserved = ix.reserved.saturating_sub(self.bytes);
    }
}

impl Cache {
    /// Open (creating it, owner-only) a transcode cache directory, forgetting unfinished
    /// work and trimming it to the cap.
    pub fn open(dir: &Path, cap: u64) -> std::io::Result<Cache> {
        Self::open_with(dir, cap, TRANSCODE_EXTS)
    }

    /// Like [`Cache::open`], for files named `<sha256>.<ext>` with `ext` one of `exts`.
    pub fn open_with(
        dir: &Path,
        cap: u64,
        exts: &'static [&'static str],
    ) -> std::io::Result<Cache> {
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
            } else if valid_name_with(&name, exts) && meta.is_file() {
                let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((used, name, meta.len()));
            }
        }
        found.sort();
        // Units small enough to be exact for small caps, few enough to count in a u32.
        let unit = (cap / u32::MAX as u64 + 1).max(1);
        let cache = Cache {
            dir: dir.to_path_buf(),
            cap,
            exts,
            index: Mutex::new(Index::default()),
            room: tokio::sync::Semaphore::new(cap.div_ceil(unit) as usize),
            unit,
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

    /// Bytes held for work in progress.
    pub fn reserved(&self) -> u64 {
        self.index.lock().unwrap().reserved
    }

    /// Hold `bytes` of the cap for work in progress (an original being transcoded), so the
    /// directory never holds more than the cap: waits while other work holds the room, then
    /// evicts the least recently used files to make it. `None` if `bytes` is more than the
    /// whole cap.
    pub async fn reserve(&self, bytes: u64) -> Option<Reservation<'_>> {
        if bytes > self.cap {
            return None;
        }
        let units = u32::try_from(bytes.div_ceil(self.unit)).ok()?;
        let room = self.room.acquire_many(units).await.ok()?;
        self.index.lock().unwrap().reserved += bytes;
        self.evict(None);
        Some(Reservation {
            cache: self,
            bytes,
            _room: room,
        })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.lock().unwrap().entries.contains_key(name)
    }

    /// A cached transcode, marked as just used.
    pub fn lookup(&self, name: &str) -> Option<PathBuf> {
        if !valid_name_with(name, self.exts) {
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
        if !valid_name_with(name, self.exts) || size > self.cap {
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

    /// Forget an entry and delete its file (e.g. found unreadable).
    pub fn remove(&self, name: &str) {
        if !valid_name_with(name, self.exts) {
            return;
        }
        let mut ix = self.index.lock().unwrap();
        if let Some((size, _)) = ix.entries.remove(name) {
            ix.total -= size;
        }
        drop(ix);
        let _ = std::fs::remove_file(self.dir.join(name));
    }

    /// Remove least recently used entries (never `keep`) until the cache fits.
    fn evict(&self, keep: Option<&str>) {
        let mut ix = self.index.lock().unwrap();
        while ix.total + ix.reserved > self.cap {
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

    /// An original being transcoded counts against the cap (review low: it was written
    /// outside it): room is made by evicting, a second original waits for room, and the
    /// room comes back when the work is done.
    #[tokio::test]
    async fn work_in_progress_counts_against_the_cap() {
        let dir = tmpdir("reserve");
        let c = Cache::open(&dir, 100).unwrap();
        assert!(put(&c, &name(1), 40));
        assert!(put(&c, &name(2), 40));
        let r = c.reserve(50).await.unwrap();
        assert_eq!(c.reserved(), 50);
        assert!(!c.contains(&name(1)), "the least recently used made room");
        assert!(c.contains(&name(2)));
        assert!(c.total() + c.reserved() <= 100);
        // A finished transcode while the room is held: still within the cap.
        assert!(put(&c, &name(3), 30));
        assert!(c.total() + c.reserved() <= 100, "{}", c.total());
        // More than the whole cap: refused at once.
        assert!(c.reserve(101).await.is_none());
        // A second original waits for room.
        let waiting = tokio::time::timeout(Duration::from_millis(100), c.reserve(60)).await;
        assert!(waiting.is_err(), "no room while 50 of 100 are held");
        drop(r);
        assert_eq!(c.reserved(), 0);
        let r = tokio::time::timeout(Duration::from_secs(2), c.reserve(60))
            .await
            .unwrap()
            .unwrap();
        assert!(c.total() + c.reserved() <= 100);
        drop(r);
        std::fs::remove_dir_all(dir).unwrap();
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
        let both = Encoders {
            libmp3lame: true,
            libopus: true,
            opus: false,
        };
        assert_eq!(encoders(listing), both);
        assert_eq!(both.opus_encoder(), Some(OpusEncoder::Libopus));
        assert_eq!(encoders(" A....D aac AAC\n"), Encoders::default());
        assert_eq!(Encoders::default().opus_encoder(), None);
        assert_eq!(
            encoders(" V....D libmp3lame x\n"),
            Encoders::default(),
            "not an audio encoder"
        );
        assert_eq!(encoders("libopus\n"), Encoders::default());
        assert_eq!(encoders(""), Encoders::default());

        // piCorePlayer's minimal ffmpeg: ffmpeg's own Opus encoder (experimental, the X) and
        // no MP3 encoder. Its name must match exactly: `opus` is also a decoder's name, and
        // `libopus` is read separately.
        let pcp = " A..X.D opus                 Opus\n A....D pcm_s16le            PCM\n";
        let found = encoders(pcp);
        assert_eq!(
            found,
            Encoders {
                libmp3lame: false,
                libopus: false,
                opus: true,
            }
        );
        assert_eq!(found.opus_encoder(), Some(OpusEncoder::Native));
        // libopus wins when both are listed (every full ffmpeg lists both).
        let full = format!("{listing} A..X.D opus Opus\n");
        assert_eq!(encoders(&full).opus_encoder(), Some(OpusEncoder::Libopus));
        for not_it in [
            " A....D opusenc x\n",
            " A....D libopusenc x\n",
            " D..... opus x\n",
        ] {
            assert_eq!(encoders(not_it).opus_encoder(), None, "{not_it:?}");
        }
    }

    #[test]
    fn opus_encoders_name_themselves() {
        assert_eq!(OpusEncoder::Libopus.codec(), "libopus");
        assert_eq!(OpusEncoder::Native.codec(), "opus");
        assert_eq!(OpusEncoder::Libopus.options(), &[] as &[&str]);
        assert_eq!(OpusEncoder::Native.options(), &["-strict", "-2"]);
        let f = Ffmpeg {
            path: "/usr/bin/ffmpeg".into(),
            mp3: false,
            opus: Some(OpusEncoder::Native),
        };
        assert_eq!(f.codec(Format::Mp3), None);
        assert_eq!(
            f.codec(Format::Opus),
            Some(Codec::Opus(OpusEncoder::Native))
        );
        assert_eq!(f.codec(Format::Original), None);
        assert!(f.supports(Format::Opus) && !f.supports(Format::Mp3));
    }

    #[test]
    fn the_set_up_is_described_for_the_settings_page() {
        use serde_json::json;
        assert_eq!(Setup::Off.describe(4), json!({"state": "off"}));
        assert_eq!(Setup::NotFound.describe(4), json!({"state": "not_found"}));
        assert_eq!(
            Setup::Unusable {
                path: "/opt/ffmpeg".into(),
                why: ProbeError::NoEncoders
            }
            .describe(4),
            json!({"state": "unusable", "ffmpeg": "/opt/ffmpeg", "why": "no_encoders"})
        );
        assert_eq!(ProbeError::NotExecutable.code(), "not_executable");
        assert_eq!(ProbeError::NotRunnable.code(), "not_runnable");
        let pcp = Setup::Ready(Ffmpeg {
            path: "/usr/local/bin/ffmpeg".into(),
            mp3: false,
            opus: Some(OpusEncoder::Native),
        });
        assert_eq!(
            pcp.describe(4),
            json!({"state": "ready", "ffmpeg": "/usr/local/bin/ffmpeg", "mp3": false, "opus": "ffmpeg", "live_streams": true})
        );
        assert_eq!(
            pcp.describe(0)["live_streams"],
            json!(false),
            "no live jobs"
        );
        let mp3_only = Setup::Ready(Ffmpeg {
            path: "/usr/bin/ffmpeg".into(),
            mp3: true,
            opus: None,
        });
        let d = mp3_only.describe(4);
        assert_eq!(d["opus"], json!(null));
        assert_eq!(d["live_streams"], json!(false));
        assert!(Setup::Off.ready().is_none() && pcp.ready().is_some());
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

    fn job_args(codec: Codec, demux: &'static str, tags: &Tags) -> Vec<String> {
        let job = Job {
            input: Path::new("/srv/cache/.tmp-in.src"),
            output: Path::new("/srv/cache/.tmp-out.mp3"),
            demuxer: demux,
            codec,
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
        let a = job_args(Codec::Mp3, "flac", &tags);
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

        let o = job_args(Codec::Opus(OpusEncoder::Libopus), "mov", &Tags::default());
        assert!(o.windows(2).any(|w| w[0] == "-enable_drefs" && w[1] == "0"));
        assert!(o.windows(2).any(|w| w[0] == "-c:a" && w[1] == "libopus"));
        assert!(o.windows(2).any(|w| w[0] == "-b:a" && w[1] == "160k"));
        assert!(o.contains(&"-vn".to_string()));
        assert!(!o.contains(&"-strict".to_string()));
        assert!(!a.contains(&"-enable_drefs".to_string()));

        // ffmpeg's built-in Opus encoder: `-strict -2` as an output option, right after it.
        let n = job_args(Codec::Opus(OpusEncoder::Native), "flac", &Tags::default());
        let pos = |w: &str| n.iter().position(|x| x == w).unwrap();
        assert_eq!(n[pos("-c:a") + 1], "opus");
        assert_eq!(&n[pos("-c:a") + 2..pos("-c:a") + 4], ["-strict", "-2"]);
        assert!(pos("-i") < pos("-strict") && pos("-strict") < n.len() - 1);
        assert_eq!(n[pos("-b:a") + 1], "160k");
        assert!(!n.contains(&"libopus".to_string()));
        assert_eq!(n.iter().filter(|x| *x == "-i").count(), 1);
    }

    #[test]
    fn probing_needs_an_absolute_executable() {
        assert_eq!(
            probe(Path::new("ffmpeg"), Duration::from_secs(1)),
            Err(ProbeError::NotExecutable)
        );
        assert_eq!(
            probe(Path::new("/nonexistent/ffmpeg"), Duration::from_secs(1)),
            Err(ProbeError::NotExecutable)
        );
        let dir = tmpdir("probe");
        let plain = dir.join("ffmpeg");
        std::fs::write(&plain, "#!/bin/sh\necho ' A..... libopus x'\n").unwrap();
        assert_eq!(
            probe(&plain, Duration::from_secs(1)),
            Err(ProbeError::NotExecutable),
            "not executable"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// An executable script standing in for ffmpeg.
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn probing_reports_the_encoders_found_or_why_not() {
        let dir = tmpdir("probe-why");
        let limit = Duration::from_secs(5);
        // piCorePlayer's: ffmpeg's own Opus encoder only.
        let pcp = script(&dir, "pcp", "echo ' A..X.D opus                 Opus'");
        assert_eq!(
            probe(&pcp, limit),
            Ok(Ffmpeg {
                path: pcp.clone(),
                mp3: false,
                opus: Some(OpusEncoder::Native),
            })
        );
        // A full one: libopus preferred, libmp3lame seen.
        let full = script(
            &dir,
            "full",
            "echo ' A....D libmp3lame x'; echo ' A..X.D opus x'; echo ' A....D libopus x'",
        );
        assert_eq!(
            probe(&full, limit),
            Ok(Ffmpeg {
                path: full.clone(),
                mp3: true,
                opus: Some(OpusEncoder::Libopus),
            })
        );
        // Encoders we can't use, or none at all: unusable, and the reason says so.
        let aac = script(&dir, "aac", "echo ' A....D aac x'");
        assert_eq!(probe(&aac, limit), Err(ProbeError::NoEncoders));
        let silent = script(&dir, "silent", "exit 1");
        assert_eq!(probe(&silent, limit), Err(ProbeError::NoEncoders));
        // One that never answers is given up on (and killed) after the limit.
        let stuck = script(&dir, "stuck", "sleep 20");
        let started = std::time::Instant::now();
        assert_eq!(
            probe(&stuck, Duration::from_millis(300)),
            Err(ProbeError::NotRunnable)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
