//! The disk spool for live streams (rolling cache Phase A, A1 in
//! docs/plans/wave3-design.md).
//!
//! Each stream LMS starts is fetched as fast as the network allows and written to an
//! app-private file; the engine's feeder hands bytes from it to the core only as fast as
//! the decoder wants them. The network is then never held back by the decoder, so a whole
//! track (and the next one, in read-ahead mode) is on the phone long before it is heard,
//! and a gap in the signal is covered by what is spooled.
//!
//! ```text
//! <cache_dir>/<scope>/spool/<id>.spool     scope: as for downloads (a hash of server and user)
//! ```
//!
//! - **Bounded:** at most [`MAX_SPOOL_BYTES`] per stream and [`MAX_TOTAL_BYTES`] in all.
//!   Over a cap, on a write error (a full disk), or with no cache folder, the rest of the
//!   stream is kept in memory with the old back-pressure: the fetch waits while more than
//!   [`MEMORY_HIGH`] bytes are waiting for the decoder.
//! - **Private and anonymous:** files are named by a counter with a fixed extension; nothing
//!   names a track, user or server. The root must be absolute, without `..`; symlinks and
//!   anything that isn't a plain file or folder are refused.
//! - **Short-lived:** a spool is deleted once its track has been fed to the decoder (or is
//!   dropped by a flush), and every spool goes when the player stops, signs out, is refused
//!   or starts a new session. Leftovers from a crash are removed when the player starts.
//! - **Untrusted on reading:** a file that is shorter than what was written (truncated or
//!   replaced) ends the track with what was fed, never anything else.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::error::ApiError;

/// Largest stream spooled to disk (an hour of 24/96 FLAC is about 2 GB, so very long
/// tracks spool their first 256 MiB and carry on in memory).
pub(crate) const MAX_SPOOL_BYTES: u64 = 256 * 1024 * 1024;
/// Largest total on disk across the spools of one player.
pub(crate) const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// Bytes waiting in memory (fallback) before the fetch pauses.
pub(crate) const MEMORY_HIGH: usize = 256 * 1024;
/// Largest piece handed to the core at once.
pub(crate) const FEED_CHUNK: usize = 64 * 1024;
/// The folder for spools under a scope.
const SPOOL_DIR: &str = "spool";
const EXT: &str = "spool";

/// Check a cache root from the app: absolute, no `..`, and not a symlink or a file.
pub(crate) fn validate_root(root: &str) -> Result<PathBuf, ApiError> {
    let path = Path::new(root);
    if root.is_empty()
        || !path.is_absolute()
        || path.components().any(|c| c.as_os_str() == "..")
        || root.len() > 4096
        || root.contains('\0')
    {
        return Err(ApiError::invalid(
            "the cache folder must be an absolute path",
        ));
    }
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            Err(ApiError::invalid("the cache folder is not a folder"))
        }
        _ => Ok(path.to_path_buf()),
    }
}

/// A folder that is ours: create it if missing, and refuse a symlink or a non-folder.
fn own_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "not a folder"))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            own_dir(path)
        }
        Err(e) => Err(e),
    }
}

/// Remove the spool folder of every scope under `root` (a player starting, signing out,
/// a revoked device or a change of server). Downloads and anything else are left alone.
pub(crate) fn remove_all(root: &Path) -> io::Result<()> {
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
        let Ok(t) = entry.file_type() else { continue };
        if t.is_symlink() || !t.is_dir() {
            continue;
        }
        let dir = entry.path().join(SPOOL_DIR);
        match fs::symlink_metadata(&dir) {
            // A planted symlink: remove the link, never what it points at.
            Ok(m) if m.file_type().is_symlink() || !m.is_dir() => fs::remove_file(&dir)?,
            Ok(_) => fs::remove_dir_all(&dir)?,
            Err(_) => {}
        }
    }
    Ok(())
}

/// Where one player's spools live: `<root>/<scope>/spool`.
#[derive(Debug, Clone)]
pub(crate) struct SpoolDir {
    dir: PathBuf,
}

impl SpoolDir {
    /// Validate `root`, then create (if needed) and check `<root>/<scope>/spool`. `scope`
    /// is a downloads scope name (32 hex characters).
    pub(crate) fn open(root: &str, scope: &str) -> Result<SpoolDir, ApiError> {
        let root = validate_root(root)?;
        if scope.len() != 32 || !scope.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::invalid("bad cache scope"));
        }
        own_dir(&root).map_err(|_| ApiError::invalid("the cache folder can't be used"))?;
        let scope_dir = root.join(scope);
        own_dir(&scope_dir).map_err(|_| ApiError::invalid("the cache folder can't be used"))?;
        let dir = scope_dir.join(SPOOL_DIR);
        own_dir(&dir).map_err(|_| ApiError::invalid("the cache folder can't be used"))?;
        Ok(SpoolDir { dir })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.dir
    }

    /// Delete every spool file here (best effort).
    pub(crate) fn clear(&self) {
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let p = e.path();
                // Spools, and anything planted as a symlink (removing a link never
                // touches what it points at).
                if p.extension().is_some_and(|x| x == EXT)
                    || e.file_type().is_ok_and(|t| t.is_symlink())
                {
                    let _ = fs::remove_file(p);
                }
            }
        }
    }

    /// A new spool file for stream id `id`: a fresh plain file (never an existing one or a
    /// symlink), opened for writing and, separately, for reading.
    fn create(&self, id: u64) -> io::Result<(File, File, PathBuf)> {
        let path = self.dir.join(format!("{id}.{EXT}"));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600).custom_flags(libc_nofollow());
        }
        let writer = opts.open(&path)?;
        let mut ropts = OpenOptions::new();
        ropts.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            ropts.custom_flags(libc_nofollow());
        }
        let reader = match ropts.open(&path) {
            Ok(r) if r.metadata().is_ok_and(|m| m.is_file()) => r,
            Ok(_) => {
                let _ = fs::remove_file(&path);
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
            }
            Err(e) => {
                let _ = fs::remove_file(&path);
                return Err(e);
            }
        };
        Ok((writer, reader, path))
    }
}

/// `O_NOFOLLOW`: never open a file through a symlink.
#[cfg(unix)]
fn libc_nofollow() -> i32 {
    libc::O_NOFOLLOW
}

/// The on-disk part of a spool.
struct DiskPart {
    path: PathBuf,
    writer: Box<dyn Write + Send>,
    reader: File,
    /// Bytes written to the file (all of them read back from it, in order).
    len: u64,
    /// Bytes read back so far.
    read: u64,
    /// New bytes still go to the file (false once over a cap or after a write error).
    open: bool,
}

/// Why a spool can't give the decoder its next bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpoolReadError {
    /// The file is shorter than what was written, or can't be read: the track ends here.
    Broken,
}

/// One stream's spool: its bytes on disk, then (after a fallback) in memory.
pub(crate) struct Spool {
    /// The stream number (the core's track `n - 1`).
    pub n: u64,
    disk: Option<DiskPart>,
    mem: VecDeque<u8>,
    /// Bytes received from the network.
    pub received: u64,
    /// Bytes handed to the decoder.
    pub fed: u64,
    /// The network stream has ended normally.
    pub complete: bool,
    /// The network stream failed for good.
    pub failed: bool,
    /// Whether this spool ever went to disk (for the cushion and tests).
    spooled: bool,
}

impl std::fmt::Debug for Spool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spool")
            .field("n", &self.n)
            .field("received", &self.received)
            .field("fed", &self.fed)
            .field("on_disk", &self.disk_bytes())
            .field("complete", &self.complete)
            .field("failed", &self.failed)
            .finish()
    }
}

impl Spool {
    /// A spool kept in memory only (no cache folder): today's back-pressure.
    pub(crate) fn memory(n: u64) -> Spool {
        Spool {
            n,
            disk: None,
            mem: VecDeque::new(),
            received: 0,
            fed: 0,
            complete: false,
            failed: false,
            spooled: false,
        }
    }

    /// A spool on disk in `dir` (file `<id>.spool`), or in memory if the file can't be made.
    pub(crate) fn create(n: u64, dir: Option<&SpoolDir>, id: u64) -> Spool {
        let mut s = Spool::memory(n);
        if let Some(Ok((w, r, path))) = dir.map(|d| d.create(id)) {
            s.attach(Box::new(w), r, path);
        }
        s
    }

    fn attach(&mut self, writer: Box<dyn Write + Send>, reader: File, path: PathBuf) {
        self.disk = Some(DiskPart {
            path,
            writer,
            reader,
            len: 0,
            read: 0,
            open: true,
        });
        self.spooled = true;
    }

    /// New bytes go to disk.
    pub(crate) fn on_disk(&self) -> bool {
        self.disk.as_ref().is_some_and(|d| d.open)
    }

    /// Whether any of this stream went to disk.
    #[cfg(test)]
    pub(crate) fn spooled(&self) -> bool {
        self.spooled
    }

    /// Bytes this spool holds on disk.
    pub(crate) fn disk_bytes(&self) -> u64 {
        self.disk.as_ref().map_or(0, |d| d.len)
    }

    /// Received but not yet handed to the decoder.
    pub(crate) fn unfed(&self) -> u64 {
        self.received - self.fed
    }

    /// Everything received has been handed to the decoder.
    pub(crate) fn caught_up(&self) -> bool {
        self.fed >= self.received
    }

    /// The fetch may read more now: always while spooling to disk; in memory, while little
    /// is waiting for the decoder.
    pub(crate) fn wants_network(&self) -> bool {
        self.on_disk() || self.mem.len() < MEMORY_HIGH
    }

    /// Store bytes from the network. `others_on_disk`: what the player's other spools hold
    /// on disk (for the total cap). Over a cap, or when the disk refuses (full), this and
    /// later bytes stay in memory.
    pub(crate) fn write(&mut self, bytes: &[u8], others_on_disk: u64) {
        self.received += bytes.len() as u64;
        if let Some(d) = &mut self.disk
            && d.open
        {
            let new_len = d.len + bytes.len() as u64;
            let fits = new_len <= MAX_SPOOL_BYTES && others_on_disk + new_len <= MAX_TOTAL_BYTES;
            if fits && d.writer.write_all(bytes).is_ok() {
                d.len = new_len;
                return;
            }
            // From here on, memory. A partial write left bytes past `len`: they are never
            // read (reads stop at `len`).
            d.open = false;
        }
        self.mem.extend(bytes);
    }

    /// Up to `max` bytes for the decoder, in order: what's on disk, then memory. Empty when
    /// caught up.
    pub(crate) fn read(&mut self, max: usize) -> Result<Vec<u8>, SpoolReadError> {
        if let Some(d) = &mut self.disk
            && d.read < d.len
        {
            let want = (d.len - d.read).min(max as u64) as usize;
            let mut buf = vec![0u8; want];
            let mut got = 0;
            while got < want {
                match d.reader.read(&mut buf[got..]) {
                    Ok(0) => return Err(SpoolReadError::Broken), // shorter than written
                    Ok(k) => got += k,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(SpoolReadError::Broken),
                }
            }
            d.read += got as u64;
            self.fed += got as u64;
            return Ok(buf);
        }
        let n = self.mem.len().min(max);
        let out: Vec<u8> = self.mem.drain(..n).collect();
        self.fed += n as u64;
        Ok(out)
    }

    /// Delete the file now (the spool is done with).
    pub(crate) fn remove(&mut self) {
        if let Some(d) = self.disk.take() {
            drop(d.writer);
            let _ = fs::remove_file(&d.path);
        }
        self.mem.clear();
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn temp_root() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "anywhere-spool-{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            getrandom::u32().unwrap_or(0)
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    const SCOPE: &str = "0123456789abcdef0123456789abcdef";

    fn dir() -> (PathBuf, SpoolDir) {
        let root = temp_root();
        let d = SpoolDir::open(root.to_str().unwrap(), SCOPE).unwrap();
        (root, d)
    }

    fn drain(s: &mut Spool) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = s.read(FEED_CHUNK).unwrap();
            if b.is_empty() {
                return out;
            }
            out.extend(b);
        }
    }

    fn files(d: &SpoolDir) -> usize {
        fs::read_dir(d.path()).unwrap().count()
    }

    #[test]
    fn bytes_round_trip_through_the_file_and_the_file_goes_when_done() {
        let (_root, d) = dir();
        let mut s = Spool::create(1, Some(&d), 7);
        assert!(s.on_disk() && s.wants_network());
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for c in data.chunks(10_000) {
            s.write(c, 0);
        }
        assert_eq!(s.disk_bytes(), data.len() as u64);
        assert_eq!(s.unfed(), data.len() as u64);
        let half = s.read(1000).unwrap();
        assert_eq!(half, data[..1000]);
        let rest = drain(&mut s);
        assert_eq!(rest, data[1000..]);
        assert!(s.caught_up());
        assert_eq!(files(&d), 1);
        drop(s);
        assert_eq!(files(&d), 0, "deleted when dropped");
    }

    #[test]
    fn without_a_folder_it_is_memory_with_back_pressure() {
        let mut s = Spool::create(1, None, 1);
        assert!(!s.on_disk() && !s.spooled());
        s.write(&vec![1u8; MEMORY_HIGH - 1], 0);
        assert!(s.wants_network());
        s.write(&[1u8; 1], 0);
        assert!(!s.wants_network(), "the fetch waits for the decoder");
        assert_eq!(s.read(10).unwrap().len(), 10);
        assert!(s.wants_network());
    }

    #[test]
    fn over_the_stream_cap_the_rest_stays_in_memory() {
        let (_root, d) = dir();
        let mut s = Spool::create(1, Some(&d), 1);
        // Pretend the file is nearly at the cap.
        s.disk.as_mut().unwrap().len = MAX_SPOOL_BYTES - 10;
        s.disk.as_mut().unwrap().read = MAX_SPOOL_BYTES - 10;
        s.write(&[5u8; 20], 0);
        assert!(!s.on_disk(), "fell back to memory");
        assert_eq!(s.disk_bytes(), MAX_SPOOL_BYTES - 10);
        assert_eq!(s.read(100).unwrap(), vec![5u8; 20]);
    }

    #[test]
    fn over_the_total_cap_the_rest_stays_in_memory() {
        let (_root, d) = dir();
        let mut s = Spool::create(1, Some(&d), 1);
        s.write(&[1u8; 100], MAX_TOTAL_BYTES - 150);
        assert!(s.on_disk());
        s.write(&[2u8; 100], MAX_TOTAL_BYTES - 150);
        assert!(!s.on_disk());
        let all = drain(&mut s);
        assert_eq!(all[..100], [1u8; 100]);
        assert_eq!(all[100..], [2u8; 100]);
    }

    /// A writer that fills up after `room` bytes, as a full disk does.
    struct FullAfter {
        inner: File,
        room: usize,
    }

    impl Write for FullAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.room == 0 {
                return Err(io::Error::from(io::ErrorKind::StorageFull));
            }
            let n = buf.len().min(self.room);
            self.room -= n;
            self.inner.write(&buf[..n])
        }
        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn a_full_disk_falls_back_to_memory_without_losing_a_byte() {
        let (_root, d) = dir();
        let (w, r, path) = d.create(9).unwrap();
        let mut s = Spool::memory(1);
        s.attach(
            Box::new(FullAfter {
                inner: w,
                room: 150,
            }),
            r,
            path,
        );
        s.write(&[1u8; 100], 0);
        // Half of this lands in the file before the disk fills: it is never read back.
        s.write(&[2u8; 100], 0);
        s.write(&[3u8; 100], 0);
        assert!(!s.on_disk());
        assert_eq!(s.received, 300);
        let all = drain(&mut s);
        let mut want = vec![1u8; 100];
        want.extend([2u8; 100]);
        want.extend([3u8; 100]);
        assert_eq!(all, want);
    }

    #[test]
    fn a_truncated_file_ends_the_track_instead_of_feeding_anything_else() {
        let (_root, d) = dir();
        let mut s = Spool::create(1, Some(&d), 3);
        s.write(&[9u8; 50_000], 0);
        let path = s.disk.as_ref().unwrap().path.clone();
        // Something cut the file short.
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(1000)
            .unwrap();
        assert_eq!(s.read(500).unwrap(), vec![9u8; 500]);
        assert_eq!(s.read(FEED_CHUNK), Err(SpoolReadError::Broken));
    }

    #[test]
    fn a_missing_file_never_panics() {
        let (_root, d) = dir();
        let mut s = Spool::create(1, Some(&d), 4);
        s.write(&[1u8; 1000], 0);
        d.clear();
        assert_eq!(files(&d), 0);
        // The open handle still reads what was written (an unlinked file lives on); either
        // way nothing panics, and only written bytes come back.
        if let Ok(b) = s.read(FEED_CHUNK) {
            assert!(b.iter().all(|x| *x == 1));
        }
        s.write(&[2u8; 10], 0);
        drop(s);
    }

    #[test]
    fn roots_must_be_absolute_plain_folders() {
        for bad in ["", "relative/dir", "/tmp/../etc", "/a/b/../../c", "/x\0y"] {
            assert!(validate_root(bad).is_err(), "{bad:?}");
            assert!(SpoolDir::open(bad, SCOPE).is_err(), "{bad:?}");
        }
        let root = temp_root();
        // A file is not a folder.
        let file = root.join("file");
        fs::write(&file, b"x").unwrap();
        assert!(validate_root(file.to_str().unwrap()).is_err());
        // A symlinked root is refused.
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            assert!(validate_root(link.to_str().unwrap()).is_err());
        }
        // Scopes are hashes only.
        for scope in ["", "../x", "0123456789abcdef0123456789abcdeg", "a"] {
            assert!(SpoolDir::open(root.to_str().unwrap(), scope).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_the_cache_are_refused_and_never_followed() {
        let root = temp_root();
        let elsewhere = temp_root();
        // A symlinked scope folder.
        std::os::unix::fs::symlink(&elsewhere, root.join(SCOPE)).unwrap();
        assert!(SpoolDir::open(root.to_str().unwrap(), SCOPE).is_err());
        fs::remove_file(root.join(SCOPE)).unwrap();
        // A symlinked spool folder.
        fs::create_dir(root.join(SCOPE)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join(SCOPE).join(SPOOL_DIR)).unwrap();
        assert!(SpoolDir::open(root.to_str().unwrap(), SCOPE).is_err());
        // Clearing removes the link, not what it points at.
        fs::write(elsewhere.join("keep.spool"), b"x").unwrap();
        remove_all(&root).unwrap();
        assert!(elsewhere.join("keep.spool").exists());
        assert!(fs::symlink_metadata(root.join(SCOPE).join(SPOOL_DIR)).is_err());
        // A symlink planted where a spool file will go: never written through.
        let d = SpoolDir::open(root.to_str().unwrap(), SCOPE).unwrap();
        let target = elsewhere.join("target");
        fs::write(&target, b"original").unwrap();
        std::os::unix::fs::symlink(&target, d.path().join("5.spool")).unwrap();
        let mut s = Spool::create(1, Some(&d), 5);
        assert!(!s.on_disk(), "refused, so memory");
        s.write(b"evil", 0);
        drop(s);
        assert_eq!(fs::read(&target).unwrap(), b"original");
    }

    #[test]
    fn remove_all_clears_every_scope_and_nothing_else() {
        let root = temp_root();
        let a = SpoolDir::open(root.to_str().unwrap(), SCOPE).unwrap();
        let b = SpoolDir::open(root.to_str().unwrap(), "fedcba9876543210fedcba9876543210").unwrap();
        let mut s1 = Spool::create(1, Some(&a), 1);
        s1.write(b"one", 0);
        let mut s2 = Spool::create(1, Some(&b), 1);
        s2.write(b"two", 0);
        fs::create_dir_all(root.join(SCOPE).join("tracks")).unwrap();
        fs::write(root.join(SCOPE).join("tracks").join("x.flac"), b"keep").unwrap();
        fs::write(root.join("other.txt"), b"keep").unwrap();
        remove_all(&root).unwrap();
        assert!(!a.path().exists() && !b.path().exists());
        assert!(root.join(SCOPE).join("tracks").join("x.flac").exists());
        assert!(root.join("other.txt").exists());
        // Missing roots are fine.
        remove_all(&root.join("nope")).unwrap();
        // Dropping spools whose folder is gone never panics.
        drop((s1, s2));
    }

    #[test]
    fn random_writes_and_reads_never_panic_and_keep_order() {
        let (_root, d) = dir();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for id in 0..20 {
            let mut s = Spool::create(id, Some(&d), id);
            let mut want = Vec::new();
            let mut got = Vec::new();
            for _ in 0..200 {
                if rnd() % 2 == 0 {
                    let len = (rnd() % 5000) as usize;
                    let b: Vec<u8> = (0..len).map(|_| rnd() as u8).collect();
                    // Sometimes as if the total cap was reached.
                    let others = if rnd() % 50 == 0 { MAX_TOTAL_BYTES } else { 0 };
                    s.write(&b, others);
                    want.extend(b);
                } else {
                    got.extend(s.read((rnd() % 9000) as usize).unwrap());
                }
            }
            got.extend(drain(&mut s));
            assert_eq!(got, want);
        }
        d.clear();
    }
}
