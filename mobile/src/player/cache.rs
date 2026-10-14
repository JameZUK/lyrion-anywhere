//! The replay cache (rolling cache Phase B, B1 and B2 in docs/plans/wave3-design.md).
//!
//! Live streams that were received whole are kept, so a replay, a skip back or LMS sending
//! a track again after a skip costs only the first bytes of the stream instead of all of it:
//! once those first bytes have arrived, their fingerprint names a cached copy, which is
//! checked against its own hash and then played instead of the rest of the download.
//!
//! ```text
//! <cache_dir>/<scope>/stream-cache/<64 hex>.bin   a whole stream, named by its fingerprint
//! <cache_dir>/<scope>/stream-cache/index.json     what is kept, and how recently it was used
//! ```
//!
//! - **Only whole, clean streams:** a spool is moved in only when its stream ended cleanly
//!   and every byte of it is in the file; anything partial is deleted as before.
//! - **Bounded:** least recently used entries go first once the total passes the cap (set by
//!   the app: off, 500 MB, 1 GB, 2 GB or 5 GB). Entries in use (being played from, or being
//!   checked) are never evicted. The space is counted apart from downloads.
//! - **Needs the download permission** (the owner's decision 1): the engine keeps and uses
//!   the cache only when `/v1/account` says downloads are allowed, and deletes it when they
//!   are not.
//! - **Private and anonymous:** names are fingerprints (a hash of the stream's level, format
//!   and first bytes) with a fixed extension. The index holds no titles, paths, addresses or
//!   tokens; the optional track id says which LMS track the stream was. The root must be absolute without
//!   `..`; symlinks and anything that isn't a plain file are refused and removed.
//! - **Untrusted on reading:** the index is bounded and every entry is checked (anything
//!   malformed, or whose file is missing, the wrong size or not a plain file, is dropped);
//!   a hit is used only after the whole file has been hashed and matches the entry, and
//!   otherwise the entry is deleted and the stream comes from the network.
//! - **Deleted** on sign-out, revocation and a change of server or user, when the cap is
//!   set to off, and when the download permission goes.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use serde_json::{Value, json};

use super::spool::{self, MAX_SPOOL_BYTES};

/// Bytes of audio after the stream's header that make up its fingerprint.
pub(crate) const FINGERPRINT_BYTES: usize = 64 * 1024;
/// A header (FLAC metadata, ID3 tag, Ogg header pages) longer than this: never matched.
pub(crate) const MAX_HEADER: usize = 1024 * 1024;
/// Most entries the index holds.
pub(crate) const MAX_ENTRIES: usize = 20_000;
/// Largest index file read.
pub(crate) const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
/// Largest cap taken from the app.
pub(crate) const MAX_CAP: u64 = 64 * 1024 * 1024 * 1024;
/// The index format written by this version.
const VERSION: u64 = 1;
const DIR: &str = "stream-cache";
const EXT: &str = "bin";
const INDEX: &str = "index.json";
const INDEX_TMP: &str = "index.json.tmp";
/// Read size when hashing or playing a cached file.
pub(crate) const READ_CHUNK: usize = 64 * 1024;

/// A SHA-256 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Digest(pub [u8; 32]);

impl Digest {
    pub(crate) fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 64 lower-case hex digits, nothing else.
    pub(crate) fn parse(s: &str) -> Option<Digest> {
        if s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Digest(out))
    }

    fn of(ctx: ring::digest::Context) -> Digest {
        let mut out = [0u8; 32];
        out.copy_from_slice(ctx.finish().as_ref());
        Digest(out)
    }
}

/// A running SHA-256 over a whole stream.
pub(crate) struct StreamHash(ring::digest::Context);

impl StreamHash {
    pub(crate) fn new() -> StreamHash {
        StreamHash(ring::digest::Context::new(&ring::digest::SHA256))
    }
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    pub(crate) fn finish(self) -> Digest {
        Digest::of(self.0)
    }
}

/// The fingerprint of a stream: its level, its `strm` format byte and its first bytes (the
/// header and [`FINGERPRINT_BYTES`] after it, or the whole stream if shorter).
pub(crate) fn fingerprint(level: u8, format: u8, first: &[u8]) -> Digest {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    ctx.update(b"lyrion-anywhere stream-cache v1\n");
    ctx.update(&[level, format]);
    ctx.update(first);
    Digest::of(ctx)
}

/// How many first bytes make up the fingerprint, from what has arrived so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    /// Not yet known: wait for more.
    More,
    /// The first this many bytes.
    At(usize),
    /// This stream is never matched (a header too long to be worth it).
    Never,
}

/// Where the stream's header ends, if that can be told yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    More,
    Ends(usize),
    TooLong,
}

/// The header of a FLAC stream (`fLaC` and its metadata blocks), an MP3 stream's ID3v2
/// tag, or an Ogg stream's header pages (those before the first page that carries audio).
/// The fingerprint covers audio beyond it, so a stream that starts elsewhere in the same
/// track (a seek) has a different fingerprint even when its header is long (embedded
/// artwork). Anything else has no header. Bounded, and never panics on any input.
fn header_len(b: &[u8]) -> Header {
    let need = |n: usize| {
        if n > MAX_HEADER {
            Header::TooLong
        } else {
            Header::More
        }
    };
    if b.len() < 4 {
        return Header::More;
    }
    if &b[..4] == b"fLaC" {
        let mut pos = 4usize;
        loop {
            if pos + 4 > b.len() {
                return need(pos + 4);
            }
            let len = u32::from_be_bytes([0, b[pos + 1], b[pos + 2], b[pos + 3]]) as usize;
            let last = b[pos] & 0x80 != 0;
            pos += 4 + len;
            if pos > MAX_HEADER {
                return Header::TooLong;
            }
            if last {
                return Header::Ends(pos);
            }
        }
    }
    if &b[..3] == b"ID3" {
        if b.len() < 10 {
            return Header::More;
        }
        let size = b[6..10]
            .iter()
            .fold(0usize, |acc, x| (acc << 7) | usize::from(x & 0x7f));
        let footer = if b[5] & 0x10 != 0 { 10 } else { 0 };
        let end = 10 + size + footer;
        return if end > MAX_HEADER {
            Header::TooLong
        } else {
            Header::Ends(end)
        };
    }
    if &b[..4] == b"OggS" {
        let mut pos = 0usize;
        loop {
            if pos + 27 > b.len() {
                return need(pos + 27);
            }
            if &b[pos..pos + 4] != b"OggS" {
                // Not a page where one should be: take what came before as the header.
                return Header::Ends(pos);
            }
            let granule = i64::from_le_bytes(b[pos + 6..pos + 14].try_into().unwrap_or([0; 8]));
            if granule != 0 && granule != -1 {
                return Header::Ends(pos); // the first page with audio
            }
            let segments = usize::from(b[pos + 26]);
            if pos + 27 + segments > b.len() {
                return need(pos + 27 + segments);
            }
            let body: usize = b[pos + 27..pos + 27 + segments]
                .iter()
                .map(|x| usize::from(*x))
                .sum();
            pos += 27 + segments + body;
            if pos > MAX_HEADER {
                return Header::TooLong;
            }
        }
    }
    Header::Ends(0)
}

/// The fingerprint window for a stream whose first bytes are `head` (`ended`: that is the
/// whole stream).
pub(crate) fn window(head: &[u8], ended: bool) -> Window {
    let header = match header_len(head) {
        Header::TooLong => return Window::Never,
        Header::Ends(n) => n,
        Header::More if ended => return Window::At(head.len()),
        Header::More => return Window::More,
    };
    let want = header + FINGERPRINT_BYTES;
    if head.len() >= want {
        Window::At(want)
    } else if ended {
        Window::At(head.len())
    } else {
        Window::More
    }
}

/// The stream level as a byte, for the fingerprint and the index.
pub(crate) fn level_byte(q: super::StreamQuality) -> u8 {
    match q {
        super::StreamQuality::Original => 0,
        super::StreamQuality::High => 1,
        super::StreamQuality::Low => 2,
    }
}

/// One cached stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub fp: Digest,
    pub level: u8,
    pub format: u8,
    pub len: u64,
    pub sha256: Digest,
    /// When it was last used (a counter: higher is more recent).
    pub used: u64,
    /// The LMS track id, for labelling only unless `whole` says more.
    pub track: Option<i64>,
    /// The stream was the whole of `track`, from its start, as LMS said when it began (a
    /// seek's stream never is). Only such an entry is played with no server (offline
    /// controls); entries written before it existed never are.
    pub whole: bool,
}

fn entry_json(e: &Entry) -> Value {
    let mut v = json!({
        "fp": e.fp.hex(),
        "level": e.level,
        "format": (e.format as char).to_string(),
        "len": e.len,
        "sha256": e.sha256.hex(),
        "used": e.used,
    });
    if let Some(t) = e.track {
        v["track"] = json!(t);
        if e.whole {
            v["whole"] = json!(true);
        }
    }
    v
}

fn entry_from(v: &Value) -> Option<Entry> {
    let o = v.as_object()?;
    let fp = Digest::parse(o.get("fp")?.as_str()?)?;
    let level = o.get("level")?.as_u64().filter(|l| *l <= 2)? as u8;
    let format = match o.get("format")?.as_str()?.as_bytes() {
        [b] if b.is_ascii_graphic() => *b,
        _ => return None,
    };
    let len = o
        .get("len")?
        .as_u64()
        .filter(|l| (1..=MAX_SPOOL_BYTES).contains(l))?;
    let sha256 = Digest::parse(o.get("sha256")?.as_str()?)?;
    let used = o.get("used")?.as_u64()?;
    let track = match o.get("track") {
        None | Some(Value::Null) => None,
        Some(t) => Some(t.as_i64().filter(|t| *t >= 0)?),
    };
    let whole = match o.get("whole") {
        None | Some(Value::Null) => false,
        Some(w) => w.as_bool()?,
    };
    Some(Entry {
        fp,
        level,
        format,
        len,
        sha256,
        used,
        // A whole stream only of a real track.
        whole: whole && track.is_some_and(|t| t > 0),
        track,
    })
}

/// Read an index: bounded, every entry checked, anything malformed dropped (never fatal),
/// one entry per fingerprint.
pub(crate) fn parse_index(bytes: &[u8]) -> Vec<Entry> {
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    if v.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Vec::new();
    }
    let Some(list) = v.get("entries").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in list {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        if let Some(e) = entry_from(item)
            && seen.insert(e.fp)
        {
            out.push(e);
        }
    }
    out
}

pub(crate) fn serialise_index(entries: &[Entry]) -> Vec<u8> {
    let list: Vec<Value> = entries.iter().take(MAX_ENTRIES).map(entry_json).collect();
    serde_json::to_vec(&json!({ "version": VERSION, "entries": list })).unwrap_or_default()
}

/// Open a file for reading without following a symlink, and only if it is a plain file.
pub(crate) fn open_plain(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(spool::libc_nofollow());
    }
    let f = o.open(path)?;
    if !f.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
    }
    Ok(f)
}

/// What a new entry is made of (from a spool that ended cleanly).
#[derive(Debug, Clone)]
pub(crate) struct NewEntry {
    pub fp: Digest,
    pub level: u8,
    pub format: u8,
    pub len: u64,
    pub sha256: Digest,
    pub track: Option<i64>,
    /// See [`Entry::whole`].
    pub whole: bool,
}

/// One user's replay cache on one server.
pub(crate) struct StreamCache {
    dir: PathBuf,
    cap: u64,
    entries: Vec<Entry>,
    /// Entries in use, and by how many.
    pins: HashMap<Digest, u32>,
    clock: u64,
}

impl std::fmt::Debug for StreamCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamCache")
            .field("entries", &self.entries.len())
            .field("cap", &self.cap)
            .finish()
    }
}

pub(crate) type SharedCache = Arc<Mutex<StreamCache>>;

/// Totals for the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Totals {
    pub bytes: u64,
    pub count: u64,
}

impl StreamCache {
    /// Open (creating it if needed) `<root>/<scope>/stream-cache` under a validated root,
    /// read its index, drop what doesn't check out and remove stray files, then keep to
    /// `cap`.
    pub(crate) fn open(root: &Path, scope: &str, cap: u64) -> io::Result<StreamCache> {
        if scope.len() != 32 || !scope.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad scope"));
        }
        let dir = root.join(scope).join(DIR);
        let mut c = StreamCache {
            dir,
            cap: cap.min(MAX_CAP),
            entries: Vec::new(),
            pins: HashMap::new(),
            clock: 0,
        };
        c.ensure_dir()?;
        let bytes = open_plain(&c.dir.join(INDEX))
            .and_then(|f| {
                let mut buf = Vec::new();
                f.take(MAX_INDEX_BYTES + 1).read_to_end(&mut buf)?;
                Ok(buf)
            })
            .unwrap_or_default();
        let listed = parse_index(&bytes);
        let before = listed.len();
        // Keep an entry only if its file is a plain file of the right size.
        c.entries = listed
            .into_iter()
            .filter(|e| {
                fs::symlink_metadata(c.file(&e.fp))
                    .is_ok_and(|m| m.file_type().is_file() && m.len() == e.len)
            })
            .collect();
        c.clock = c.entries.iter().map(|e| e.used).max().unwrap_or(0);
        let dropped = before != c.entries.len();
        c.remove_strays();
        c.evict();
        if dropped || bytes.is_empty() {
            c.save();
        }
        Ok(c)
    }

    fn ensure_dir(&self) -> io::Result<()> {
        if let Some(scope) = self.dir.parent() {
            spool::own_dir(scope)?;
        }
        spool::own_dir(&self.dir)
    }

    fn file(&self, fp: &Digest) -> PathBuf {
        self.dir.join(format!("{}.{EXT}", fp.hex()))
    }

    /// Remove every file here that no entry names (and anything that is a symlink or not a
    /// plain file), keeping the index.
    fn remove_strays(&self) {
        let Ok(list) = fs::read_dir(&self.dir) else {
            return;
        };
        for e in list.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let Ok(t) = e.file_type() else { continue };
            let known = name
                .strip_suffix(".bin")
                .and_then(Digest::parse)
                .is_some_and(|fp| self.entries.iter().any(|x| x.fp == fp));
            if t.is_dir() {
                let _ = fs::remove_dir_all(e.path());
            } else if t.is_symlink() || !(known || name == INDEX) {
                let _ = fs::remove_file(e.path());
            }
        }
    }

    /// Write the index atomically (a fresh temporary file, then a rename). Best effort: an
    /// index that can't be written is rebuilt as empty next time, and the files then go.
    fn save(&self) {
        if self.ensure_dir().is_err() {
            return;
        }
        let tmp = self.dir.join(INDEX_TMP);
        let _ = fs::remove_file(&tmp);
        let mut o = OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600).custom_flags(spool::libc_nofollow());
        }
        let ok = o.open(&tmp).and_then(|mut f| {
            f.write_all(&serialise_index(&self.entries))?;
            f.sync_all()
        });
        if ok.is_ok() {
            let _ = fs::rename(&tmp, self.dir.join(INDEX));
        } else {
            let _ = fs::remove_file(&tmp);
        }
    }

    /// A new cap; entries over it go (all of them, for 0).
    pub(crate) fn set_cap(&mut self, cap: u64) {
        let cap = cap.min(MAX_CAP);
        if cap == self.cap {
            return;
        }
        self.cap = cap;
        if self.evict() {
            self.save();
        }
    }

    pub(crate) fn totals(&self) -> Totals {
        Totals {
            bytes: self.entries.iter().map(|e| e.len).sum(),
            count: self.entries.len() as u64,
        }
    }

    /// Evict the least recently used entries not in use until the total is within the cap.
    /// Whether anything went.
    fn evict(&mut self) -> bool {
        let mut total: u64 = self.entries.iter().map(|e| e.len).sum();
        let mut any = false;
        while total > self.cap {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !self.pins.contains_key(&e.fp))
                .min_by_key(|(_, e)| e.used)
                .map(|(i, _)| i);
            let Some(i) = victim else { break };
            let e = self.entries.remove(i);
            let _ = fs::remove_file(self.file(&e.fp));
            total -= e.len;
            any = true;
        }
        any
    }

    /// Whether anything is kept at this level and format (a stream could then be a hit).
    pub(crate) fn holds_any(&self, level: u8, format: u8) -> bool {
        self.entries
            .iter()
            .any(|e| e.level == level && e.format == format)
    }

    /// The whole stream of each LMS track the cache holds one of (see [`Entry::whole`]),
    /// at `level` where there are several, for playing with no server. Only a format this
    /// build decodes counts.
    pub(crate) fn whole_tracks(&self, level: u8) -> HashMap<i64, Entry> {
        let mut out: HashMap<i64, Entry> = HashMap::new();
        for e in &self.entries {
            let Some(t) = e.track.filter(|_| e.whole) else {
                continue;
            };
            if anywhere_core::decode::format_extension(e.format).is_none() {
                continue;
            }
            match out.get(&t) {
                Some(have) if have.level == level || e.level != level => {}
                _ => {
                    out.insert(t, e.clone());
                }
            }
        }
        out
    }

    pub(crate) fn lookup(&self, fp: &Digest) -> Option<Entry> {
        self.entries.iter().find(|e| e.fp == *fp).cloned()
    }

    /// The file of an entry (for reading it).
    pub(crate) fn path_of(&self, fp: &Digest) -> PathBuf {
        self.file(fp)
    }

    pub(crate) fn pin(&mut self, fp: Digest) {
        *self.pins.entry(fp).or_insert(0) += 1;
    }

    pub(crate) fn unpin(&mut self, fp: &Digest) {
        if let Some(n) = self.pins.get_mut(fp) {
            *n -= 1;
            if *n == 0 {
                self.pins.remove(fp);
                // An entry held over the cap while in use may go now.
                if self.evict() {
                    self.save();
                }
            }
        }
    }

    /// Mark an entry used now (a hit).
    pub(crate) fn touch(&mut self, fp: &Digest) {
        self.clock += 1;
        let now = self.clock;
        if let Some(e) = self.entries.iter_mut().find(|e| e.fp == *fp) {
            e.used = now;
            self.save();
        }
    }

    /// Delete an entry (a hash mismatch, or a corrupt file), in use or not.
    pub(crate) fn remove(&mut self, fp: &Digest) {
        if let Some(i) = self.entries.iter().position(|e| e.fp == *fp) {
            self.entries.remove(i);
            let _ = fs::remove_file(self.file(fp));
            self.save();
        }
    }

    /// Delete everything (the user's "Clear", or the download permission gone). Files in use
    /// are unlinked too: what is reading them carries on to its end.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.save();
        self.remove_strays();
    }

    /// Move a finished spool file in as a new entry, pinned (in use) for the caller. `None`
    /// (and the file left where it was) when it isn't kept: the cache is off, it is larger
    /// than the cap, the same stream is already kept (then that entry is marked used), or the
    /// move fails.
    pub(crate) fn adopt(&mut self, from: &Path, n: NewEntry) -> Option<PathBuf> {
        if self.cap == 0 || n.len == 0 || n.len > self.cap || n.len > MAX_SPOOL_BYTES {
            return None;
        }
        if let Some(old) = self.lookup(&n.fp) {
            if old.sha256 == n.sha256 && old.len == n.len {
                self.touch(&n.fp);
                return None;
            }
            // The same first bytes but a different stream: the newer one replaces it, unless
            // the old one is in use.
            if self.pins.contains_key(&n.fp) {
                return None;
            }
            self.remove(&n.fp);
        }
        if self.entries.len() >= MAX_ENTRIES || self.ensure_dir().is_err() {
            return None;
        }
        match fs::symlink_metadata(from) {
            Ok(m) if m.file_type().is_file() && m.len() == n.len => {}
            _ => return None,
        }
        let to = self.file(&n.fp);
        let _ = fs::remove_file(&to); // a stray (never an entry's file: checked above)
        fs::rename(from, &to).ok()?;
        self.clock += 1;
        self.entries.push(Entry {
            fp: n.fp,
            level: n.level,
            format: n.format,
            len: n.len,
            sha256: n.sha256,
            used: self.clock,
            whole: n.whole && n.track.is_some_and(|t| t > 0),
            track: n.track,
        });
        self.pin(n.fp);
        self.evict();
        self.save();
        Some(to)
    }
}

/// An entry in use: never evicted while this lives.
pub(crate) struct Pin {
    cache: SharedCache,
    fp: Digest,
}

impl Pin {
    /// Pin `fp` (call with the lock not held).
    pub(crate) fn new(cache: &SharedCache, fp: Digest) -> Pin {
        cache.lock().unwrap().pin(fp);
        Pin::adopted(cache, fp)
    }

    /// The pin [`StreamCache::adopt`] already took.
    pub(crate) fn adopted(cache: &SharedCache, fp: Digest) -> Pin {
        Pin {
            cache: cache.clone(),
            fp,
        }
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        if let Ok(mut c) = self.cache.lock() {
            c.unpin(&self.fp);
        }
    }
}

impl std::fmt::Debug for Pin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pin")
    }
}

/// Check a cached file before using it: a plain file of the entry's length, whose first
/// bytes are `received` and whose SHA-256 is the entry's. On success, the file positioned
/// just after `received`.
pub(crate) fn verify(path: &Path, e: &Entry, received: &[u8]) -> io::Result<File> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "the cached copy doesn't match");
    let mut f = open_plain(path)?;
    if f.metadata()?.len() != e.len || received.len() as u64 > e.len {
        return Err(bad());
    }
    let mut hash = StreamHash::new();
    let mut buf = vec![0u8; READ_CHUNK];
    let mut pos = 0usize;
    loop {
        let k = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let chunk = &buf[..k];
        if pos < received.len() {
            let upto = (received.len() - pos).min(k);
            if chunk[..upto] != received[pos..pos + upto] {
                return Err(bad());
            }
        }
        hash.update(chunk);
        pos += k;
    }
    if pos as u64 != e.len || hash.finish() != e.sha256 {
        return Err(bad());
    }
    f.seek(SeekFrom::Start(received.len() as u64))?;
    Ok(f)
}

// --- Sharing ---------------------------------------------------------------------------

/// The caches open in this process, by folder, so a running player and the app's calls
/// (totals, clearing) share one index.
static OPEN: Mutex<Vec<(PathBuf, Weak<Mutex<StreamCache>>)>> = Mutex::new(Vec::new());

/// The replay cache for `scope` under `root` (validated), shared with anything else that
/// has it open. `cap`: set it (`None` keeps the current one, or 0 for a cache opened here).
pub(crate) fn shared(root: &str, scope: &str, cap: Option<u64>) -> io::Result<SharedCache> {
    let root = spool::validate_root(root)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad cache folder"))?;
    spool::own_dir(&root)?;
    let dir = root.join(scope).join(DIR);
    let mut open = OPEN.lock().unwrap();
    open.retain(|(_, w)| w.strong_count() > 0);
    if let Some(c) = open
        .iter()
        .find(|(d, _)| *d == dir)
        .and_then(|(_, w)| w.upgrade())
    {
        if let Some(cap) = cap {
            c.lock().unwrap().set_cap(cap);
        }
        return Ok(c);
    }
    let c = Arc::new(Mutex::new(StreamCache::open(
        &root,
        scope,
        cap.unwrap_or(0),
    )?));
    open.push((dir, Arc::downgrade(&c)));
    Ok(c)
}

/// Delete the replay cache of every scope under `root` except `keep` (sign-out, revocation,
/// another server or user). Caches open in this process are emptied first, so their
/// indexes agree. Downloads and anything else are left alone.
pub(crate) fn remove_all_except(root: &Path, keep: Option<&str>) -> io::Result<()> {
    {
        let open = OPEN.lock().unwrap();
        for (dir, w) in open.iter() {
            let scope = dir
                .parent()
                .and_then(Path::file_name)
                .map(|s| s.to_string_lossy().into_owned());
            if dir.starts_with(root)
                && scope.as_deref() != keep
                && let Some(c) = w.upgrade()
            {
                c.lock().unwrap().clear();
            }
        }
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let scope = name.to_string_lossy();
        if scope.len() != 32 || !scope.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if keep == Some(scope.as_ref()) {
            continue;
        }
        let Ok(t) = entry.file_type() else { continue };
        if t.is_symlink() || !t.is_dir() {
            continue;
        }
        let dir = entry.path().join(DIR);
        match fs::symlink_metadata(&dir) {
            // A planted symlink: remove the link, never what it points at.
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => fs::remove_file(&dir)?,
            Ok(_) => fs::remove_dir_all(&dir)?,
            Err(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::spool::tests::temp_root;

    const SCOPE: &str = "0123456789abcdef0123456789abcdef";

    fn d(n: u8) -> Digest {
        Digest([n; 32])
    }

    fn sha(bytes: &[u8]) -> Digest {
        let mut h = StreamHash::new();
        h.update(bytes);
        h.finish()
    }

    /// A spool-like file with `bytes` in it, outside the cache.
    fn spool_file(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let dir = root.join(SCOPE).join("spool");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    fn new_entry(fp: Digest, bytes: &[u8]) -> NewEntry {
        NewEntry {
            fp,
            level: 1,
            format: b'u',
            len: bytes.len() as u64,
            sha256: sha(bytes),
            track: Some(7),
            whole: false,
        }
    }

    fn open(root: &Path, cap: u64) -> StreamCache {
        StreamCache::open(root, SCOPE, cap).unwrap()
    }

    fn cache_dir(root: &Path) -> PathBuf {
        root.join(SCOPE).join(DIR)
    }

    #[test]
    fn digests_are_strict_hex() {
        let x = d(0xab);
        assert_eq!(Digest::parse(&x.hex()), Some(x));
        for bad in [
            "",
            "ab",
            &"A".repeat(64),
            &"g".repeat(64),
            &"a".repeat(63),
            &"a".repeat(65),
            &format!("{}/", "a".repeat(63)),
            &format!("..{}", "a".repeat(62)),
        ] {
            assert_eq!(Digest::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn index_round_trips_and_drops_malformed_entries() {
        let e = Entry {
            fp: d(1),
            level: 2,
            format: b'f',
            len: 1234,
            sha256: d(2),
            used: 9,
            track: Some(42),
            whole: true,
        };
        let mut f = e.clone();
        f.fp = d(3);
        f.track = None;
        f.whole = false;
        let bytes = serialise_index(&[e.clone(), f.clone()]);
        assert_eq!(parse_index(&bytes), vec![e.clone(), f.clone()]);

        let good = entry_json(&e);
        let mut cases: Vec<Value> = Vec::new();
        for (k, v) in [
            ("fp", json!("zz")),
            ("fp", json!(1)),
            ("fp", json!("../../etc/passwd")),
            ("level", json!(3)),
            ("level", json!(-1)),
            ("level", json!("high")),
            ("format", json!("")),
            ("format", json!("ab")),
            ("format", json!(" ")),
            ("format", json!("\u{0}")),
            ("format", json!("é")),
            ("len", json!(0)),
            ("len", json!(-5)),
            ("len", json!(MAX_SPOOL_BYTES + 1)),
            ("len", json!(1.5)),
            ("sha256", json!(null)),
            ("used", json!(-1)),
            ("used", json!("now")),
            ("track", json!(-3)),
            ("track", json!("x")),
            ("whole", json!("yes")),
            ("whole", json!(1)),
        ] {
            let mut x = good.clone();
            x[k] = v;
            cases.push(x);
        }
        for k in ["fp", "level", "format", "len", "sha256", "used"] {
            let mut x = good.clone();
            x.as_object_mut().unwrap().remove(k);
            cases.push(x);
        }
        cases.push(json!("not an object"));
        cases.push(json!(null));
        cases.push(json!([1, 2]));
        // Plus a duplicate of the good one: kept once.
        cases.push(good.clone());
        cases.push(good.clone());
        let doc = json!({"version": VERSION, "entries": cases});
        let parsed = parse_index(&serde_json::to_vec(&doc).unwrap());
        assert_eq!(parsed, vec![e.clone()]);
        // Wrong or missing version, wrong shapes, junk: nothing (never a failure).
        for junk in [
            json!({"version": 2, "entries": [good.clone()]}),
            json!({"entries": [good.clone()]}),
            json!({"version": VERSION, "entries": {}}),
            json!([good.clone()]),
            json!(null),
        ] {
            assert!(parse_index(&serde_json::to_vec(&junk).unwrap()).is_empty());
        }
        assert!(parse_index(b"").is_empty());
        assert!(parse_index(b"{not json").is_empty());
        assert!(parse_index(&vec![b' '; MAX_INDEX_BYTES as usize + 1]).is_empty());
    }

    #[test]
    fn the_index_is_bounded() {
        let many: Vec<Entry> = (0..MAX_ENTRIES + 50)
            .map(|i| {
                let mut fp = [0u8; 32];
                fp[..8].copy_from_slice(&(i as u64).to_be_bytes());
                Entry {
                    fp: Digest(fp),
                    level: 0,
                    format: b'f',
                    len: 1,
                    sha256: d(0),
                    used: i as u64,
                    track: None,
                    whole: false,
                }
            })
            .collect();
        let doc =
            json!({"version": VERSION, "entries": many.iter().map(entry_json).collect::<Vec<_>>()});
        assert_eq!(
            parse_index(&serde_json::to_vec(&doc).unwrap()).len(),
            MAX_ENTRIES
        );
        assert_eq!(parse_index(&serialise_index(&many)).len(), MAX_ENTRIES);
    }

    #[test]
    fn fuzzed_indexes_never_panic() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let base = serialise_index(&[Entry {
            fp: d(5),
            level: 1,
            format: b'u',
            len: 99,
            sha256: d(6),
            used: 1,
            track: Some(1),
            whole: true,
        }]);
        for _ in 0..5000 {
            let mut b = base.clone();
            for _ in 0..(rnd() % 8) {
                let i = (rnd() as usize) % b.len();
                match rnd() % 3 {
                    0 => b[i] = rnd() as u8,
                    1 => {
                        b.remove(i);
                    }
                    _ => b.insert(i, rnd() as u8),
                }
                if b.is_empty() {
                    break;
                }
            }
            for e in parse_index(&b) {
                assert!(e.level <= 2 && e.len >= 1 && e.len <= MAX_SPOOL_BYTES);
                assert!(e.format.is_ascii_graphic());
                assert!(
                    !e.whole || e.track.is_some_and(|t| t > 0),
                    "whole needs a track"
                );
            }
        }
        // Random bytes too.
        for _ in 0..2000 {
            let len = (rnd() % 300) as usize;
            let b: Vec<u8> = (0..len).map(|_| rnd() as u8).collect();
            let _ = parse_index(&b);
            let _ = window(&b, rnd() % 2 == 0);
        }
    }

    #[test]
    fn windows_cover_the_header_and_64_kib_of_audio() {
        // No header: 64 KiB.
        let plain = vec![0x55u8; 100_000];
        assert_eq!(window(&plain[..1000], false), Window::More);
        assert_eq!(window(&plain, false), Window::At(FINGERPRINT_BYTES));
        // Short streams: the whole stream, once it has ended.
        assert_eq!(window(&plain[..1000], true), Window::At(1000));
        assert_eq!(window(b"", true), Window::At(0));
        // FLAC: the metadata, then 64 KiB.
        let mut flac = b"fLaC".to_vec();
        flac.extend([0x00, 0, 0, 34]);
        flac.extend([0u8; 34]);
        flac.extend([0x86, 0, 0x01, 0x00]); // the last block: 256 bytes
        flac.extend([0u8; 256]);
        let header = flac.len();
        flac.extend(vec![1u8; 70_000]);
        assert_eq!(window(&flac, false), Window::At(header + FINGERPRINT_BYTES));
        assert_eq!(window(&flac[..header + 10], false), Window::More);
        assert_eq!(window(&flac[..20], false), Window::More);
        // A FLAC header too long to bother with.
        let mut big = b"fLaC".to_vec();
        big.extend([0x86, 0x20, 0, 0]); // 2 MiB
        assert_eq!(window(&big, false), Window::Never);
        // ID3v2: the tag (syncsafe size), then 64 KiB.
        let mut id3 = b"ID3\x04\x00\x00".to_vec();
        id3.extend([0, 0, 0x02, 0x00]); // 256
        id3.extend([0u8; 256]);
        id3.extend(vec![2u8; 70_000]);
        assert_eq!(
            window(&id3, false),
            Window::At(10 + 256 + FINGERPRINT_BYTES)
        );
        let mut id3big = b"ID3\x04\x00\x00".to_vec();
        id3big.extend([0x7f, 0x7f, 0x7f, 0x7f]);
        assert_eq!(window(&id3big, false), Window::Never);
        // Ogg: the header pages (granule 0), then 64 KiB from the first audio page.
        let page = |granule: i64, body: usize| {
            let mut p = b"OggS\x00\x02".to_vec();
            p.extend(granule.to_le_bytes());
            p.extend([0u8; 12]);
            let segs = body.div_ceil(255).max(1);
            p.push(segs as u8);
            let mut left = body;
            for _ in 0..segs {
                let s = left.min(255);
                p.push(s as u8);
                left -= s;
            }
            p.extend(vec![3u8; body]);
            p
        };
        let mut ogg = page(0, 19);
        ogg.extend(page(0, 600));
        ogg.extend(page(-1, 300));
        let header = ogg.len();
        for g in 1..400 {
            ogg.extend(page(g * 960, 255));
        }
        assert_eq!(window(&ogg, false), Window::At(header + FINGERPRINT_BYTES));
        assert_eq!(window(&ogg[..header - 1], false), Window::More);
    }

    #[test]
    fn fingerprints_follow_level_format_and_bytes() {
        let a = vec![7u8; FINGERPRINT_BYTES];
        let f = fingerprint(1, b'u', &a);
        assert_eq!(
            f,
            fingerprint(1, b'u', &a),
            "the same stream, the same name"
        );
        assert_ne!(f, fingerprint(0, b'u', &a), "another level");
        assert_ne!(f, fingerprint(1, b'f', &a), "another format");
        let mut b = a.clone();
        b[FINGERPRINT_BYTES - 1] ^= 1;
        assert_ne!(f, fingerprint(1, b'u', &b), "a byte differs");
        assert_ne!(f, fingerprint(1, b'u', &a[..100]), "a shorter stream");
    }

    #[test]
    fn a_seek_never_matches_the_whole_track() {
        // FLAC with a long header (artwork): a stream after a seek has the same header,
        // then other audio. The window reaches past the header, so they differ.
        let mut head = b"fLaC".to_vec();
        head.extend([0x86, 0x01, 0x80, 0x00]); // 96 KiB of metadata, the last block
        head.extend(vec![9u8; 0x18000]);
        let mut whole = head.clone();
        whole.extend((0..200_000u32).map(|i| (i % 251) as u8));
        let mut seeked = head.clone();
        seeked.extend((100_000..300_000u32).map(|i| (i % 251) as u8));
        let w1 = match window(&whole, false) {
            Window::At(n) => n,
            w => panic!("{w:?}"),
        };
        let w2 = match window(&seeked, false) {
            Window::At(n) => n,
            w => panic!("{w:?}"),
        };
        assert!(w1 > head.len());
        assert_ne!(
            fingerprint(0, b'f', &whole[..w1]),
            fingerprint(0, b'f', &seeked[..w2])
        );
    }

    #[test]
    fn adopt_keeps_whole_streams_and_survives_reopening() {
        let root = temp_root();
        let mut c = open(&root, 1 << 30);
        let bytes = vec![4u8; 5000];
        let src = spool_file(&root, "1.spool", &bytes);
        let to = c.adopt(&src, new_entry(d(1), &bytes)).unwrap();
        assert!(!src.exists() && to.exists());
        assert_eq!(fs::read(&to).unwrap(), bytes);
        assert_eq!(
            c.totals(),
            Totals {
                bytes: 5000,
                count: 1
            }
        );
        c.unpin(&d(1));
        drop(c);
        let c = open(&root, 1 << 30);
        let e = c.lookup(&d(1)).unwrap();
        assert_eq!(
            (e.len, e.level, e.format, e.track),
            (5000, 1, b'u', Some(7))
        );
        // The same stream again: not a second copy, just marked used.
        let mut c = c;
        let src2 = spool_file(&root, "2.spool", &bytes);
        assert!(c.adopt(&src2, new_entry(d(1), &bytes)).is_none());
        assert!(src2.exists(), "left to the spool to delete");
        assert_eq!(c.totals().count, 1);
    }

    #[test]
    fn off_or_too_big_is_never_kept() {
        let root = temp_root();
        let bytes = vec![4u8; 5000];
        let mut off = open(&root, 0);
        let src = spool_file(&root, "1.spool", &bytes);
        assert!(off.adopt(&src, new_entry(d(1), &bytes)).is_none());
        let mut small = open(&root, 4000);
        assert!(small.adopt(&src, new_entry(d(1), &bytes)).is_none());
        // A length that doesn't match the file: refused.
        let mut c = open(&root, 1 << 20);
        let mut lie = new_entry(d(1), &bytes);
        lie.len = 4999;
        assert!(c.adopt(&src, lie).is_none());
        assert!(src.exists());
        assert_eq!(c.totals().count, 0);
    }

    #[test]
    fn lru_eviction_spares_entries_in_use() {
        let root = temp_root();
        let mut c = open(&root, 10_000);
        let bytes = vec![1u8; 4000];
        for i in 1..=2u8 {
            let src = spool_file(&root, &format!("{i}.spool"), &bytes);
            c.adopt(&src, new_entry(d(i), &bytes)).unwrap();
            c.unpin(&d(i));
        }
        // 1 is used again: 2 is now the oldest.
        c.touch(&d(1));
        let src = spool_file(&root, "3.spool", &bytes);
        c.adopt(&src, new_entry(d(3), &bytes)).unwrap();
        c.unpin(&d(3));
        assert!(c.lookup(&d(2)).is_none(), "least recently used went");
        assert!(!c.path_of(&d(2)).exists());
        assert!(c.lookup(&d(1)).is_some() && c.lookup(&d(3)).is_some());
        // In use, 1 is never evicted, even when it is the oldest.
        c.pin(d(1));
        c.touch(&d(3));
        let src = spool_file(&root, "4.spool", &bytes);
        c.adopt(&src, new_entry(d(4), &bytes)).unwrap();
        assert!(c.lookup(&d(1)).is_some(), "in use");
        assert!(c.lookup(&d(3)).is_none());
        // Everything in use and over the cap: held until released, then evicted.
        c.set_cap(4000);
        assert_eq!(c.totals().count, 2);
        c.unpin(&d(1));
        assert_eq!(c.totals().count, 1);
        c.unpin(&d(4));
        assert!(c.lookup(&d(4)).is_some());
        // Off: everything goes.
        c.set_cap(0);
        assert_eq!(c.totals(), Totals::default());
        assert_eq!(
            fs::read_dir(cache_dir(&root)).unwrap().count(),
            1,
            "the index only"
        );
    }

    #[test]
    fn reopening_drops_entries_whose_files_are_wrong_and_strays() {
        let root = temp_root();
        let mut c = open(&root, 1 << 30);
        let bytes = vec![1u8; 3000];
        for i in 1..=4u8 {
            let src = spool_file(&root, &format!("{i}.spool"), &bytes);
            c.adopt(&src, new_entry(d(i), &bytes)).unwrap();
            c.unpin(&d(i));
        }
        drop(c);
        let dir = cache_dir(&root);
        // 1: gone. 2: the wrong size. 3: replaced by a symlink. 4: fine.
        fs::remove_file(dir.join(format!("{}.bin", d(1).hex()))).unwrap();
        fs::write(dir.join(format!("{}.bin", d(2).hex())), b"short").unwrap();
        #[cfg(unix)]
        {
            let p = dir.join(format!("{}.bin", d(3).hex()));
            fs::remove_file(&p).unwrap();
            let target = root.join("elsewhere");
            fs::write(&target, &bytes).unwrap();
            std::os::unix::fs::symlink(&target, &p).unwrap();
        }
        // Strays: an unknown file, a folder, a leftover temporary index.
        fs::write(dir.join("stray.bin"), b"x").unwrap();
        fs::write(dir.join(format!("{}.bin", d(9).hex())), b"x").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join(INDEX_TMP), b"x").unwrap();
        let c = open(&root, 1 << 30);
        let kept: Vec<Digest> = c.entries.iter().map(|e| e.fp).collect();
        #[cfg(unix)]
        assert_eq!(kept, vec![d(4)]);
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        #[cfg(unix)]
        assert_eq!(
            names,
            vec![format!("{}.bin", d(4).hex()), INDEX.to_string()]
        );
        // What the symlink pointed at is untouched.
        #[cfg(unix)]
        assert_eq!(fs::read(root.join("elsewhere")).unwrap(), bytes);
        // A damaged index: everything goes, nothing fails.
        drop(c);
        fs::write(dir.join(INDEX), b"{\"version\":1,\"entries\":[{\"fp\":").unwrap();
        let c = open(&root, 1 << 30);
        assert_eq!(c.totals(), Totals::default());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folders_and_index_are_refused() {
        let root = temp_root();
        let elsewhere = temp_root();
        // A symlinked scope or cache folder: refused.
        std::os::unix::fs::symlink(&elsewhere, root.join(SCOPE)).unwrap();
        assert!(StreamCache::open(&root, SCOPE, 1 << 20).is_err());
        fs::remove_file(root.join(SCOPE)).unwrap();
        fs::create_dir(root.join(SCOPE)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, cache_dir(&root)).unwrap();
        assert!(StreamCache::open(&root, SCOPE, 1 << 20).is_err());
        fs::remove_file(cache_dir(&root)).unwrap();
        // A symlinked index is never read through, and is removed.
        fs::create_dir(cache_dir(&root)).unwrap();
        let fake = elsewhere.join("index.json");
        let bytes = vec![1u8; 10];
        fs::write(elsewhere.join("x.bin"), &bytes).unwrap();
        fs::write(&fake, serialise_index(&[])).unwrap();
        std::os::unix::fs::symlink(&fake, cache_dir(&root).join(INDEX)).unwrap();
        let c = StreamCache::open(&root, SCOPE, 1 << 20).unwrap();
        assert_eq!(c.totals().count, 0);
        assert!(fake.exists(), "never touched through the link");
        // Bad scopes.
        for scope in ["", "..", "../x", "0123456789abcdef0123456789abcdeg"] {
            assert!(StreamCache::open(&root, scope, 1).is_err());
        }
    }

    #[test]
    fn verify_checks_length_prefix_and_hash() {
        let root = temp_root();
        let mut c = open(&root, 1 << 30);
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
        let src = spool_file(&root, "1.spool", &bytes);
        let path = c.adopt(&src, new_entry(d(1), &bytes)).unwrap();
        let e = c.lookup(&d(1)).unwrap();
        let mut f = verify(&path, &e, &bytes[..70_000]).unwrap();
        let mut rest = Vec::new();
        f.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, bytes[70_000..]);
        // Received bytes that differ from the file's start.
        let mut other = bytes[..70_000].to_vec();
        other[69_999] ^= 1;
        assert!(verify(&path, &e, &other).is_err());
        // Tampered: one byte changed, same length.
        let mut t = bytes.clone();
        t[150_000] ^= 0xff;
        fs::write(&path, &t).unwrap();
        assert!(verify(&path, &e, &bytes[..70_000]).is_err());
        // Truncated, or longer.
        fs::write(&path, &bytes[..100]).unwrap();
        assert!(verify(&path, &e, &bytes[..50]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        fs::write(&path, &longer).unwrap();
        assert!(verify(&path, &e, &bytes[..50]).is_err());
        // A folder, a missing file, a symlink to the right bytes: all refused.
        fs::remove_file(&path).unwrap();
        assert!(verify(&path, &e, &bytes[..50]).is_err());
        fs::create_dir(&path).unwrap();
        assert!(verify(&path, &e, &bytes[..50]).is_err());
        fs::remove_dir(&path).unwrap();
        #[cfg(unix)]
        {
            let target = root.join("good");
            fs::write(&target, &bytes).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            assert!(verify(&path, &e, &bytes[..50]).is_err());
        }
    }

    #[test]
    fn clear_and_remove_all_take_everything_but_downloads() {
        let root = temp_root();
        let rs = root.to_str().unwrap();
        let c = shared(rs, SCOPE, Some(1 << 30)).unwrap();
        let bytes = vec![3u8; 1000];
        let src = spool_file(&root, "1.spool", &bytes);
        assert!(
            c.lock()
                .unwrap()
                .adopt(&src, new_entry(d(1), &bytes))
                .is_some()
        );
        let _pin = Pin::adopted(&c, d(1));
        // The same folder opened again is the same cache.
        let again = shared(rs, SCOPE, None).unwrap();
        assert!(Arc::ptr_eq(&c, &again));
        assert_eq!(again.lock().unwrap().totals().count, 1);
        // Another scope's cache, and downloads.
        let other = "fedcba9876543210fedcba9876543210";
        let o = shared(rs, other, Some(1 << 30)).unwrap();
        let src = spool_file(&root, "2.spool", &bytes);
        o.lock()
            .unwrap()
            .adopt(&src, new_entry(d(2), &bytes))
            .unwrap();
        fs::create_dir_all(root.join(SCOPE).join("tracks")).unwrap();
        fs::write(root.join(SCOPE).join("tracks").join("k.flac"), b"keep").unwrap();
        // Everything but this scope's cache.
        remove_all_except(&root, Some(SCOPE)).unwrap();
        assert_eq!(o.lock().unwrap().totals().count, 0);
        assert!(!root.join(other).join(DIR).exists());
        assert_eq!(c.lock().unwrap().totals().count, 1);
        // Everything, even in use.
        remove_all_except(&root, None).unwrap();
        assert_eq!(c.lock().unwrap().totals().count, 0);
        assert!(!cache_dir(&root).exists());
        assert!(root.join(SCOPE).join("tracks").join("k.flac").exists());
        // Still usable afterwards (the folder is made again).
        let src = spool_file(&root, "3.spool", &bytes);
        assert!(
            c.lock()
                .unwrap()
                .adopt(&src, new_entry(d(3), &bytes))
                .is_some()
        );
        c.lock().unwrap().clear();
        assert_eq!(c.lock().unwrap().totals(), Totals::default());
        // Missing roots are fine.
        remove_all_except(&root.join("missing"), None).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remove_all_never_follows_a_planted_symlink() {
        let root = temp_root();
        let elsewhere = temp_root();
        fs::create_dir(root.join(SCOPE)).unwrap();
        fs::write(elsewhere.join("keep.bin"), b"x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, cache_dir(&root)).unwrap();
        remove_all_except(&root, None).unwrap();
        assert!(elsewhere.join("keep.bin").exists());
        assert!(fs::symlink_metadata(cache_dir(&root)).is_err());
    }

    #[test]
    fn only_whole_streams_of_real_tracks_count_for_playing_offline() {
        let root = temp_root();
        let mut c = open(&root, 1 << 30);
        let mut add = |n: u8, level: u8, format: u8, track: Option<i64>, whole: bool| {
            let bytes = vec![n; 1000 + usize::from(n)];
            let src = spool_file(&root, &format!("{n}.spool"), &bytes);
            let mut e = new_entry(d(n), &bytes);
            (e.level, e.format, e.track, e.whole) = (level, format, track, whole);
            c.adopt(&src, e).unwrap();
            c.unpin(&d(n));
        };
        add(1, 1, b'f', Some(10), true);
        add(2, 0, b'f', Some(10), true); // the same track at the Original level
        add(3, 0, b'f', Some(11), false); // a label only (a seek, or an older entry)
        add(4, 0, b'f', None, true); // no track: never whole
        add(5, 0, b'x', Some(12), true); // a format this build can't decode
        add(6, 2, b'm', Some(13), true);
        let at_original = c.whole_tracks(0);
        assert_eq!(at_original.len(), 2, "{at_original:?}");
        assert_eq!(at_original[&10].fp, d(2), "the level asked for first");
        assert_eq!(at_original[&13].fp, d(6), "any level otherwise");
        assert_eq!(c.whole_tracks(1)[&10].fp, d(1));
        // Kept across reopening; a label-only entry stays a label.
        drop(c);
        let c = open(&root, 1 << 30);
        assert_eq!(c.whole_tracks(0).len(), 2);
        assert!(!c.lookup(&d(3)).unwrap().whole);
        assert!(!c.lookup(&d(4)).unwrap().whole);
        // An index that says "whole" for no track, or track 0, is not believed.
        let doc = json!({"version": VERSION, "entries": [
            {"fp": d(7).hex(), "level": 0, "format": "f", "len": 5, "sha256": d(8).hex(),
             "used": 1, "whole": true},
            {"fp": d(9).hex(), "level": 0, "format": "f", "len": 5, "sha256": d(8).hex(),
             "used": 1, "track": 0, "whole": true},
        ]});
        assert!(
            parse_index(&serde_json::to_vec(&doc).unwrap())
                .iter()
                .all(|e| !e.whole)
        );
    }
}
