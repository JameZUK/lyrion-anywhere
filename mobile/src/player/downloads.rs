//! Downloaded originals as a source for live streams (rolling cache Phase C in
//! docs/plans/wave3-design.md).
//!
//! When the phone plays a FLAC stream at the Original level (pass-through, or with its
//! artwork and padding stripped by the gateway), its audio frames are byte for byte those of
//! the library file. If that file is downloaded, the rest of the stream can come from it:
//!
//! - **Matching:** the stream's STREAMINFO (the audio's MD5, total samples, sample rate,
//!   channels and bit depth) must equal the download's, all of it. An MD5 of all zeros
//!   (unset) never matches, nor does an unknown length.
//! - **Splicing:** at the first audio frame. With `Ms` the stream's metadata length, `Mf`
//!   the file's and `N` the bytes received, the rest of the stream is the file from
//!   `Mf + N − Ms`. The audio received so far must equal the file's at the same place.
//! - **Checked before use:** the download must be in the offline index as an Original FLAC,
//!   a plain file (never a symlink) of the size the index recorded (the offline store's own
//!   check), and decode whole to the MD5 in its STREAMINFO with every frame intact
//!   (`anywhere_core::decode::verify_flac`). Any doubt, and the stream carries on from the
//!   network.
//! - **Never in the way:** the check runs on the blocking pool beside the stream, which
//!   carries on from the network meanwhile; the download takes over from wherever the
//!   stream has got to. Once the stream's first bytes (`?limit=`) are in, it waits at most
//!   3 s for a check still running before asking for the rest (2026-10-18).
//! - **Read only:** the offline index and files are read, never written: the download
//!   manager owns them. Nothing here is exposed to the app but a "from downloads" flag.
//! - **Scope:** the signed-in user's downloads on this server only (the same hashed folder
//!   as the offline library); signing out or revocation deletes them as before, and a file
//!   that has gone is simply not used.
//!
//! Opus and MP3 downloads are never used: they are not the bytes LMS streams.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::offline::index::{self, DownloadFormat, OfflineTrack, Parsed};
use crate::offline::store;

use super::cache::open_plain;

/// Most FLAC metadata read (as the gateway's stripping allows; big scans of artwork).
pub(crate) const MAX_METADATA: usize = 16 * 1024 * 1024;
/// Most metadata blocks accepted.
pub(crate) const MAX_BLOCKS: usize = 1024;
const STREAMINFO_LEN: usize = 34;
/// Read size for a file's header.
const READ_CHUNK: usize = 64 * 1024;

/// What a FLAC STREAMINFO block says about the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamInfo {
    pub md5: [u8; 16],
    pub total_samples: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub bits: u8,
}

impl StreamInfo {
    /// The 34-byte block body.
    fn parse(b: &[u8]) -> Option<StreamInfo> {
        if b.len() != STREAMINFO_LEN {
            return None;
        }
        let packed = u64::from_be_bytes(b[10..18].try_into().ok()?);
        let mut md5 = [0u8; 16];
        md5.copy_from_slice(&b[18..34]);
        Some(StreamInfo {
            md5,
            total_samples: packed & 0xf_ffff_ffff,
            sample_rate: (packed >> 44) as u32,
            channels: ((packed >> 41) & 0x7) as u8 + 1,
            bits: ((packed >> 36) & 0x1f) as u8 + 1,
        })
    }

    /// Enough to identify the audio: an MD5 that is set, a known length, a valid rate and
    /// depth.
    pub(crate) fn usable(&self) -> bool {
        self.md5 != [0; 16]
            && self.total_samples > 0
            && (1..=655_350).contains(&self.sample_rate)
            && self.bits >= 4
    }

    /// The same audio: every field equal, and identifying.
    pub(crate) fn same_audio(&self, other: &StreamInfo) -> bool {
        self.usable() && self == other
    }
}

/// How far the start of a stream or file has shown its FLAC header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlacHead {
    /// Not yet known: more bytes are needed.
    More,
    /// A FLAC header of `len` bytes (marker and metadata blocks), followed by a frame sync.
    Done { info: StreamInfo, len: usize },
    /// Not FLAC, or not a header to trust (malformed, too long, too many blocks).
    Not,
}

/// Parse the FLAC header at the start of `b`: `fLaC`, a 34-byte STREAMINFO first (and only
/// there), further blocks of any defined type, the last flagged, then a frame sync
/// (`0xFFF8`/`0xFFF9`), which catches a lying last-block flag. Bounded by
/// [`MAX_METADATA`] and [`MAX_BLOCKS`]; never panics on any input.
pub(crate) fn flac_head(b: &[u8]) -> FlacHead {
    const MARKER: &[u8; 4] = b"fLaC";
    let n = b.len().min(4);
    if b[..n] != MARKER[..n] {
        return FlacHead::Not;
    }
    if b.len() < 4 {
        return FlacHead::More;
    }
    let mut pos = 4usize;
    let mut info = None;
    let mut blocks = 0usize;
    loop {
        if pos + 4 > b.len() {
            return if pos + 4 > MAX_METADATA {
                FlacHead::Not
            } else {
                FlacHead::More
            };
        }
        let kind = b[pos] & 0x7f;
        let last = b[pos] & 0x80 != 0;
        let len = u32::from_be_bytes([0, b[pos + 1], b[pos + 2], b[pos + 3]]) as usize;
        blocks += 1;
        if blocks > MAX_BLOCKS {
            return FlacHead::Not;
        }
        let first = blocks == 1;
        if first && (kind != 0 || len != STREAMINFO_LEN) {
            return FlacHead::Not;
        }
        if !first && (kind == 0 || kind == 127) {
            return FlacHead::Not;
        }
        let end = pos + 4 + len;
        if end > MAX_METADATA {
            return FlacHead::Not;
        }
        if first {
            if end > b.len() {
                return FlacHead::More;
            }
            info = StreamInfo::parse(&b[pos + 4..end]);
        }
        pos = end;
        if last {
            if pos + 2 > b.len() {
                return FlacHead::More;
            }
            return match info {
                Some(info) if b[pos] == 0xff && b[pos + 1] & 0xfe == 0xf8 => {
                    FlacHead::Done { info, len: pos }
                }
                _ => FlacHead::Not,
            };
        }
    }
}

/// Read a file's FLAC header from its start: its STREAMINFO, the header's length and the
/// STREAMINFO block's bytes.
fn read_head(f: &mut File) -> Option<(StreamInfo, usize, Vec<u8>)> {
    f.seek(SeekFrom::Start(0)).ok()?;
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK];
    loop {
        match flac_head(&buf) {
            FlacHead::Done { info, len } => return Some((info, len, buf[8..42].to_vec())),
            FlacHead::Not => return None,
            FlacHead::More if buf.len() > MAX_METADATA + 2 => return None,
            FlacHead::More => {}
        }
        let k = match f.read(&mut chunk) {
            Ok(0) => return None,
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        buf.extend_from_slice(&chunk[..k]);
    }
}

/// Read at most `limit` bytes of a plain file, never through a symlink.
fn read_plain(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let f = open_plain(path).ok()?;
    let mut out = Vec::new();
    f.take(limit + 1).read_to_end(&mut out).ok()?;
    (out.len() as u64 <= limit).then_some(out)
}

/// A download's audio behind a bare header (`fLaC` and its STREAMINFO alone), for the
/// decode check: its other metadata (tags, artwork) plays no part in the audio, and a tag
/// block the decoder can't parse shouldn't rule the file out.
struct BareAudio {
    head: Vec<u8>,
    file: File,
    /// Where the audio starts in the file, and how long it is.
    audio_at: u64,
    audio_len: u64,
    pos: u64,
}

impl BareAudio {
    fn new(file: File, info_block: &[u8], audio_at: u64, file_len: u64) -> BareAudio {
        let mut head = b"fLaC\x80\x00\x00\x22".to_vec();
        head.extend_from_slice(info_block);
        BareAudio {
            head,
            file,
            audio_at,
            audio_len: file_len.saturating_sub(audio_at),
            pos: 0,
        }
    }

    fn len(&self) -> u64 {
        self.head.len() as u64 + self.audio_len
    }
}

impl Read for BareAudio {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let head = self.head.len() as u64;
        if self.pos < head {
            let from = self.pos as usize;
            let k = buf.len().min(self.head.len() - from);
            buf[..k].copy_from_slice(&self.head[from..from + k]);
            self.pos += k as u64;
            return Ok(k);
        }
        let left = self.len().saturating_sub(self.pos);
        if left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        self.file
            .seek(SeekFrom::Start(self.audio_at + (self.pos - head)))?;
        let k = self.file.read(&mut buf[..want])?;
        self.pos += k as u64;
        Ok(k)
    }
}

impl Seek for BareAudio {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(d) => i128::from(self.len()) + i128::from(d),
            SeekFrom::Current(d) => i128::from(self.pos) + i128::from(d),
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "before the start",
            ));
        }
        self.pos = u64::try_from(target).unwrap_or(u64::MAX);
        Ok(self.pos)
    }
}

impl anywhere_core::decode::MediaSource for BareAudio {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}

/// What is known about one download's file (until its size or time changes).
#[derive(Debug, Clone)]
struct Head {
    size: u64,
    modified: Option<SystemTime>,
    /// Its STREAMINFO and header length (`None`: not a FLAC file to trust).
    found: Option<(StreamInfo, usize)>,
    /// Decoded whole to its MD5.
    verified: bool,
}

#[derive(Default)]
struct State {
    /// The index file's size and time when last read.
    stamp: Option<(u64, Option<SystemTime>)>,
    /// Original FLAC downloads, from the index.
    tracks: Vec<OfflineTrack>,
    /// Every download, from the index (for playing with no server).
    all: Vec<OfflineTrack>,
    heads: HashMap<String, Head>,
}

/// A checked download to play the rest of a stream from.
#[derive(Debug)]
pub(crate) struct Found {
    /// Positioned where the stream carries on.
    pub file: File,
    /// Bytes from there to its end.
    pub remaining: u64,
}

/// One user's downloads on one server, as a source for live streams (read only).
pub(crate) struct Downloads {
    dir: PathBuf,
    state: Mutex<State>,
}

impl Downloads {
    /// The downloads under `root` (the folder the app gives the offline library) for
    /// `scope` ([`crate::offline::scope_name`]). `None` if the folder isn't acceptable:
    /// absolute, without `..`, and not a symlink.
    pub(crate) fn open(root: &str, scope: &str) -> Option<Downloads> {
        let root = super::spool::validate_root(root).ok()?;
        if scope.len() != 32
            || !scope
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return None;
        }
        Some(Downloads {
            dir: root.join(scope),
            state: Mutex::new(State::default()),
        })
    }

    /// Read the index again if it changed. Anything doubtful (no folder, a symlink, an
    /// index that can't be read) means no downloads.
    fn refresh(&self, st: &mut State) {
        let real_dir = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
        let path = store::index_file(&self.dir);
        let meta = fs::symlink_metadata(&path).ok().filter(|m| m.is_file());
        let (true, true, Some(meta)) = (
            real_dir(&self.dir),
            real_dir(&self.dir.join("tracks")),
            meta,
        ) else {
            *st = State::default();
            return;
        };
        let stamp = (meta.len(), meta.modified().ok());
        if st.stamp == Some(stamp) {
            return;
        }
        st.all = match read_plain(&path, index::MAX_INDEX_BYTES).map(|b| index::parse(&b)) {
            Some(Parsed::Tracks(t, _)) => t,
            _ => Vec::new(),
        };
        st.tracks = st
            .all
            .iter()
            .filter(|t| t.format == DownloadFormat::Original && t.ext == "flac")
            .cloned()
            .collect();
        st.stamp = Some(stamp);
        let keys: std::collections::HashSet<&str> =
            st.tracks.iter().map(|t| t.key.as_str()).collect();
        st.heads.retain(|k, _| keys.contains(k.as_str()));
    }

    /// Whether any Original FLAC is downloaded (blocking: reads the index if it changed).
    pub(crate) fn may_have_flac(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        self.refresh(&mut st);
        !st.tracks.is_empty()
    }

    /// Every download whose file is in place (the offline store's check: a plain file of
    /// the recorded size), by LMS track id, for playing with no server (offline controls).
    /// Blocking: reads the index if it changed, and each file's metadata.
    pub(crate) fn by_track(&self) -> HashMap<i64, super::local::LocalTrack> {
        let all = {
            let mut st = self.state.lock().unwrap();
            self.refresh(&mut st);
            st.all.clone()
        };
        let mut out = HashMap::new();
        for t in all {
            if t.track_id <= 0 || out.contains_key(&t.track_id) {
                continue;
            }
            let path = store::track_file(&self.dir, &t);
            let fits = fs::symlink_metadata(&path).is_ok_and(|m| store::file_fits(&m, &t));
            if !fits {
                continue;
            }
            out.insert(
                t.track_id,
                super::local::LocalTrack {
                    key: format!("download:{}", t.key),
                    track_id: t.track_id,
                    path,
                    ext: t.ext.clone(),
                    duration_secs: t.duration_secs,
                    album: t.album_id.map(|a| a.to_string()),
                },
            );
        }
        out
    }

    /// What a download's file holds, remembered while its size and time stay the same.
    fn head_of(&self, t: &OfflineTrack, path: &Path) -> Option<Head> {
        // The offline store's check: a plain file (not a symlink) of the recorded size.
        let meta = fs::symlink_metadata(path).ok()?;
        if !store::file_fits(&meta, t) {
            return None;
        }
        let modified = meta.modified().ok();
        if let Some(h) = self.state.lock().unwrap().heads.get(&t.key)
            && h.size == meta.len()
            && h.modified == modified
        {
            return Some(h.clone());
        }
        let found = open_plain(path)
            .ok()
            .and_then(|mut f| read_head(&mut f))
            .map(|(i, l, _)| (i, l));
        let h = Head {
            size: meta.len(),
            modified,
            found,
            verified: false,
        };
        self.state
            .lock()
            .unwrap()
            .heads
            .insert(t.key.clone(), h.clone());
        Some(h)
    }

    /// The download that carries on a FLAC stream whose STREAMINFO is `info`, whose header
    /// is `stream_meta` bytes long and whose first bytes (all of them so far) are
    /// `received`, checked and positioned where the stream carries on. `None`: carry on
    /// from the network. Blocking (it reads and may decode whole files).
    pub(crate) fn take(
        &self,
        info: &StreamInfo,
        stream_meta: usize,
        received: &[u8],
    ) -> Option<Found> {
        if !info.usable() || received.len() < stream_meta + 2 {
            return None;
        }
        let audio = &received[stream_meta..];
        let tracks = {
            let mut st = self.state.lock().unwrap();
            self.refresh(&mut st);
            st.tracks.clone()
        };
        for t in &tracks {
            let path = store::track_file(&self.dir, t);
            let Some(head) = self.head_of(t, &path) else {
                continue;
            };
            if !head.found.is_some_and(|(i, _)| i.same_audio(info)) {
                continue;
            }
            if let Some(found) = self.check(t, &path, info, audio, &head) {
                return Some(found);
            }
        }
        None
    }

    /// Check one download in full and position it after `audio` (the stream's audio so
    /// far): the store's size check, the same STREAMINFO, the same audio bytes at the
    /// splice, and (once per file version) every frame decoded to the MD5.
    fn check(
        &self,
        t: &OfflineTrack,
        path: &Path,
        info: &StreamInfo,
        audio: &[u8],
        head: &Head,
    ) -> Option<Found> {
        let mut f = open_plain(path).ok()?;
        let meta = f.metadata().ok()?;
        if !store::file_fits(&meta, t) {
            return None;
        }
        let (file_info, meta_len, block) = read_head(&mut f)?;
        if !file_info.same_audio(info) {
            return None;
        }
        let start = (meta_len as u64).checked_add(audio.len() as u64)?;
        if start > meta.len() {
            return None;
        }
        f.seek(SeekFrom::Start(meta_len as u64)).ok()?;
        let mut have = vec![0u8; audio.len()];
        f.read_exact(&mut have).ok()?;
        if have != audio {
            return None;
        }
        let modified = meta.modified().ok();
        let known_good = head.verified && head.size == meta.len() && head.modified == modified;
        if !known_good {
            // Through the same open file (a duplicate shares its position, set again below),
            // so what is checked is what is played.
            let dup = f.try_clone().ok()?;
            let bare = BareAudio::new(dup, &block, meta_len as u64, meta.len());
            let check = anywhere_core::decode::verify_flac(Box::new(bare)).ok()?;
            if !check.md5_ok || check.frames != info.total_samples {
                return None;
            }
            if let Some(h) = self.state.lock().unwrap().heads.get_mut(&t.key)
                && h.size == meta.len()
                && h.modified == modified
            {
                h.verified = true;
            }
        }
        f.seek(SeekFrom::Start(start)).ok()?;
        Some(Found {
            file: f,
            remaining: meta.len() - start,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::offline::OfflineLibrary;
    use crate::offline::index::tests::sample;

    const BLOCK: usize = 1152;

    fn crc8(data: &[u8]) -> u8 {
        let mut crc = 0u8;
        for &b in data {
            crc ^= b;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 {
                    (crc << 1) ^ 0x07
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    fn crc16(data: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &b in data {
            crc ^= u16::from(b) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x8005
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    pub(crate) fn noise(frames: usize, seed: u64) -> Vec<i32> {
        let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        (0..frames * 2)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x as i64).rem_euclid(65_536) as i32 - 32_768
            })
            .collect()
    }

    pub(crate) fn streaminfo(rate: u32, frames: u64, md5: [u8; 16]) -> Vec<u8> {
        let mut si = vec![0u8; 34];
        si[0..2].copy_from_slice(&(BLOCK as u16).to_be_bytes());
        si[2..4].copy_from_slice(&(BLOCK as u16).to_be_bytes());
        let packed: u64 = (u64::from(rate) << 44) | (1u64 << 41) | (15u64 << 36) | frames;
        si[10..18].copy_from_slice(&packed.to_be_bytes());
        si[18..34].copy_from_slice(&md5);
        si
    }

    /// A 16-bit stereo FLAC of verbatim frames: STREAMINFO (with the real MD5, or `md5`),
    /// then `extra` blocks (type, body), the last flagged.
    pub(crate) fn flac(
        rate: u32,
        samples: &[i32],
        extra: &[(u8, Vec<u8>)],
        md5: Option<[u8; 16]>,
    ) -> Vec<u8> {
        let frames = samples.len() / 2;
        let md5 = md5.unwrap_or_else(|| anywhere_core::decode::flac_pcm_md5(samples, 16));
        let mut out = b"fLaC".to_vec();
        out.push(if extra.is_empty() { 0x80 } else { 0 });
        out.extend_from_slice(&[0, 0, 34]);
        out.extend(streaminfo(rate, frames as u64, md5));
        for (i, (kind, body)) in extra.iter().enumerate() {
            let last = if i + 1 == extra.len() { 0x80 } else { 0 };
            out.push(last | kind);
            out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
            out.extend_from_slice(body);
        }
        for (number, chunk) in samples.chunks(BLOCK * 2).enumerate() {
            let n = chunk.len() / 2;
            let mut f = vec![0xff, 0xf8, 0b0111_0000, 0b0001_0000];
            assert!(number < 0x80);
            f.push(number as u8);
            f.extend(((n - 1) as u16).to_be_bytes());
            f.push(crc8(&f));
            for ch in 0..2 {
                f.push(0b0000_0010);
                for i in 0..n {
                    f.extend(&(chunk[i * 2 + ch] as i16).to_be_bytes());
                }
            }
            let crc = crc16(&f);
            f.extend(crc.to_be_bytes());
            out.extend(f);
        }
        out
    }

    /// A well-formed VORBIS_COMMENT body (the same tags on every copy).
    fn tags() -> Vec<u8> {
        let mut v = Vec::new();
        let vendor = b"test";
        v.extend((vendor.len() as u32).to_le_bytes());
        v.extend(vendor);
        let tag = b"TITLE=Same title";
        v.extend(1u32.to_le_bytes());
        v.extend((tag.len() as u32).to_le_bytes());
        v.extend(tag);
        v
    }

    fn info_of(b: &[u8]) -> (StreamInfo, usize) {
        match flac_head(b) {
            FlacHead::Done { info, len } => (info, len),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn streaminfo_is_read_and_headers_measured() {
        let pcm = noise(5000, 1);
        let plain = flac(44_100, &pcm, &[], None);
        let (info, len) = info_of(&plain);
        assert_eq!(len, 42);
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!((info.channels, info.bits), (2, 16));
        assert_eq!(info.total_samples, 5000);
        assert_eq!(info.md5, anywhere_core::decode::flac_pcm_md5(&pcm, 16));
        assert!(info.usable());
        // Artwork and padding: a longer header, the same STREAMINFO.
        let art = flac(
            44_100,
            &pcm,
            &[
                (4, b"comment".to_vec()),
                (6, vec![9; 70_000]),
                (1, vec![0; 100]),
            ],
            None,
        );
        let (info2, len2) = info_of(&art);
        assert_eq!(info2, info);
        assert_eq!(len2, 42 + 4 + 7 + 4 + 70_000 + 4 + 100);
        // Every prefix short of the first frame sync is "more", never a wrong answer.
        for cut in 0..len2 + 2 {
            assert_eq!(flac_head(&art[..cut]), FlacHead::More, "{cut}");
        }
    }

    #[test]
    fn malformed_headers_are_refused() {
        let pcm = noise(2000, 2);
        let good = flac(48_000, &pcm, &[(4, vec![1; 10])], None);
        let bad = |f: &dyn Fn(&mut Vec<u8>)| {
            let mut b = good.clone();
            f(&mut b);
            flac_head(&b)
        };
        assert_eq!(bad(&|b| b[0] = b'F'), FlacHead::Not, "no marker");
        assert_eq!(flac_head(b"ID3\x04"), FlacHead::Not);
        assert_eq!(flac_head(b"OggS"), FlacHead::Not);
        assert_eq!(bad(&|b| b[4] = 4), FlacHead::Not, "STREAMINFO not first");
        assert_eq!(
            bad(&|b| b[7] = 33),
            FlacHead::Not,
            "STREAMINFO the wrong size"
        );
        assert_eq!(bad(&|b| b[42] = 0x80), FlacHead::Not, "a second STREAMINFO");
        assert_eq!(bad(&|b| b[42] = 0xff), FlacHead::Not, "type 127");
        // A lying last-block flag: no frame sync after it.
        assert_eq!(bad(&|b| b[4] |= 0x80), FlacHead::Not);
        // Bogus lengths: within the bound, more is waited for (and never arrives); past
        // it, refused at once.
        assert_eq!(
            bad(&|b| b[43..46].copy_from_slice(&[0x7f, 0xff, 0xff])),
            FlacHead::More
        );
        assert_eq!(
            bad(&|b| b[43..46].copy_from_slice(&[0xff, 0xff, 0xff])),
            FlacHead::Not
        );
        assert_eq!(
            bad(&|b| b[5..8].copy_from_slice(&[0xff, 0xff, 0xff])),
            FlacHead::Not
        );
        let mut many = b"fLaC".to_vec();
        many.extend([0, 0, 0, 34]);
        many.extend(streaminfo(44_100, 1, [1; 16]));
        for _ in 0..MAX_BLOCKS {
            many.extend([4, 0, 0, 0]);
        }
        assert_eq!(flac_head(&many), FlacHead::Not, "too many blocks");
        // Oversize metadata: blocks that add up past the bound are refused before it.
        let mut huge = b"fLaC".to_vec();
        huge.extend([0, 0, 0, 34]);
        huge.extend(streaminfo(44_100, 1, [1; 16]));
        huge.extend([1, 0x80, 0, 0]);
        huge.extend(vec![0u8; 0x80_0000]);
        huge.extend([0x81, 0x80, 0, 0]);
        assert_eq!(flac_head(&huge), FlacHead::Not);
    }

    #[test]
    fn fuzzed_headers_never_panic() {
        let pcm = noise(300, 3);
        let base = flac(44_100, &pcm, &[(4, vec![3; 40]), (6, vec![4; 300])], None);
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..20_000 {
            let mut b = base.clone();
            for _ in 0..(next() % 6) {
                let i = (next() as usize) % b.len();
                b[i] = next() as u8;
            }
            let cut = (next() as usize) % (b.len() + 1);
            let _ = flac_head(&b[..cut]);
            if let FlacHead::Done { len, .. } = flac_head(&b) {
                assert!(len + 2 <= b.len());
            }
        }
        for len in 0..64 {
            let junk: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let _ = flac_head(&junk);
            let mut marked = b"fLaC".to_vec();
            marked.extend(&junk);
            let _ = flac_head(&marked);
        }
    }

    #[test]
    fn the_match_rules() {
        let pcm = noise(3000, 4);
        let (info, _) = info_of(&flac(44_100, &pcm, &[], None));
        assert!(info.same_audio(&info));
        let mut zero = info;
        zero.md5 = [0; 16];
        assert!(!zero.same_audio(&zero), "an unset MD5 never matches");
        let mut other = info;
        other.total_samples += 1;
        assert!(!info.same_audio(&other), "different samples");
        let mut deeper = info;
        deeper.bits = 24;
        assert!(!info.same_audio(&deeper), "a different depth");
        let mut rate = info;
        rate.sample_rate = 48_000;
        assert!(!info.same_audio(&rate));
        let mut mono = info;
        mono.channels = 1;
        assert!(!info.same_audio(&mono));
        let mut unknown = info;
        unknown.total_samples = 0;
        assert!(
            !unknown.same_audio(&unknown),
            "an unknown length never matches"
        );
        let (master2, _) = info_of(&flac(44_100, &noise(3000, 5), &[], None));
        assert!(!info.same_audio(&master2), "another master");
    }

    // --- The downloads ------------------------------------------------------------------

    const SERVER: &str = "https://music.example.com";

    struct Lib {
        root: String,
        lib: std::sync::Arc<OfflineLibrary>,
    }

    impl Lib {
        fn new() -> Lib {
            let root = crate::offline::store::tests::temp_root();
            let lib = OfflineLibrary::open(root.clone(), SERVER.into(), "alice".into()).unwrap();
            Lib { root, lib }
        }

        fn put(&self, n: u8, bytes: &[u8], format: DownloadFormat, ext: &str) -> PathBuf {
            let mut t = sample(n);
            t.size = bytes.len() as u64;
            t.format = format;
            t.ext = ext.into();
            let p = self.lib.final_path(&t.key, &t.ext);
            fs::write(&p, bytes).unwrap();
            self.lib.add(t).unwrap();
            p
        }

        fn downloads(&self) -> Downloads {
            Downloads::open(&self.root, &crate::offline::scope_name(SERVER, "alice")).unwrap()
        }
    }

    /// The stream as the gateway sends it: the header without artwork, the same frames.
    fn stripped(file: &[u8]) -> Vec<u8> {
        let (_, len) = info_of(file);
        let mut s = b"fLaC".to_vec();
        s.push(0x80);
        s.extend([0, 0, 34]);
        s.extend(&file[8..42]);
        s.extend(&file[len..]);
        s
    }

    fn rest(mut f: Found) -> Vec<u8> {
        let mut out = Vec::new();
        f.file.read_to_end(&mut out).unwrap();
        assert_eq!(out.len() as u64, f.remaining);
        out
    }

    #[test]
    fn a_stripped_stream_is_spliced_onto_the_download() {
        let l = Lib::new();
        let pcm = noise(20_000, 6);
        let file = flac(44_100, &pcm, &[(4, tags()), (1, vec![0; 5000])], None);
        l.put(1, &file, DownloadFormat::Original, "flac");
        let d = l.downloads();
        assert!(d.may_have_flac());
        let stream = stripped(&file);
        let (info, ms) = info_of(&stream);
        for got in [ms + 2, ms + 1000, stream.len() - 10, stream.len()] {
            let found = d.take(&info, ms, &stream[..got]).expect("matched");
            let mut whole = stream[..got].to_vec();
            whole.extend(rest(found));
            assert_eq!(whole, stream, "received {got}");
        }
        // Too little received to compare any audio: no.
        assert!(d.take(&info, ms, &stream[..ms]).is_none());

        // Tags and artwork the decoder couldn't parse play no part in the check.
        let l = Lib::new();
        let odd = flac(
            44_100,
            &pcm,
            &[(4, b"tags".to_vec()), (6, vec![7; 3000])],
            None,
        );
        l.put(2, &odd, DownloadFormat::Original, "flac");
        let found = l.downloads().take(&info, ms, &stream[..ms + 10]).unwrap();
        let mut whole = stream[..ms + 10].to_vec();
        whole.extend(rest(found));
        assert_eq!(whole, stream);
    }

    #[test]
    fn doubtful_downloads_are_never_used() {
        let pcm = noise(20_000, 7);
        let file = flac(44_100, &pcm, &[(4, tags())], None);
        let stream = stripped(&file);
        let (info, ms) = info_of(&stream);
        let got = &stream[..ms + 4096];
        let none = |l: &Lib| l.downloads().take(&info, ms, got).is_none();

        // Opus or MP3 downloads, and an original that isn't FLAC, are never candidates.
        let l = Lib::new();
        l.put(1, &file, DownloadFormat::Opus, "opus");
        l.put(2, &file, DownloadFormat::Mp3, "mp3");
        l.put(3, &file, DownloadFormat::Original, "m4a");
        assert!(!l.downloads().may_have_flac());
        assert!(none(&l));

        // An unset MD5 on both sides.
        let l = Lib::new();
        let unset = flac(44_100, &pcm, &[(4, tags())], Some([0; 16]));
        l.put(1, &unset, DownloadFormat::Original, "flac");
        let s = stripped(&unset);
        let (i, m) = info_of(&s);
        assert!(l.downloads().take(&i, m, &s[..m + 4096]).is_none());

        // Another master with the same tags: a different MD5.
        let l = Lib::new();
        l.put(
            1,
            &flac(44_100, &noise(20_000, 8), &[(4, tags())], None),
            DownloadFormat::Original,
            "flac",
        );
        assert!(none(&l));

        // Tampered audio (same size): the decode check fails.
        let l = Lib::new();
        let mut bad = file.clone();
        let at = bad.len() - 500;
        bad[at] ^= 0x40;
        l.put(1, &bad, DownloadFormat::Original, "flac");
        assert!(none(&l));

        // Tampered and its CRC fixed up would still fail the MD5: a frame's sample
        // changed consistently is caught by the audio checksum. Here: the STREAMINFO's MD5
        // changed to the stream's while the audio differs.
        let l = Lib::new();
        let other = flac(44_100, &noise(20_000, 9), &[(4, tags())], Some(info.md5));
        l.put(1, &other, DownloadFormat::Original, "flac");
        assert!(
            none(&l),
            "the audio doesn't match its MD5, nor the stream's bytes"
        );

        // Truncated: the file is no longer the size the index recorded.
        let l = Lib::new();
        let p = l.put(1, &file, DownloadFormat::Original, "flac");
        fs::write(&p, &file[..file.len() - 100]).unwrap();
        assert!(none(&l));

        // Truncated and the index lying about it (a file cut short of its last frame).
        let l = Lib::new();
        l.put(
            1,
            &file[..file.len() - 100],
            DownloadFormat::Original,
            "flac",
        );
        assert!(none(&l));

        // A symlink in place of the file.
        #[cfg(unix)]
        {
            let l = Lib::new();
            let p = l.put(1, &file, DownloadFormat::Original, "flac");
            let elsewhere = std::path::Path::new(&l.root).join("elsewhere.flac");
            fs::write(&elsewhere, &file).unwrap();
            fs::remove_file(&p).unwrap();
            std::os::unix::fs::symlink(&elsewhere, &p).unwrap();
            assert!(none(&l));
        }

        // The audio received differs from the file's (a re-encode of the same audio).
        let l = Lib::new();
        l.put(1, &file, DownloadFormat::Original, "flac");
        let mut odd = got.to_vec();
        let last = odd.len() - 1;
        odd[last] ^= 1;
        assert!(l.downloads().take(&info, ms, &odd).is_none());
        // ... and the same download, with the right bytes, is used.
        assert!(l.downloads().take(&info, ms, got).is_some());

        // Removed (revocation or sign-out deletes the folder): nothing to use.
        crate::offline::remove_offline_data(l.root.clone(), SERVER.into(), "alice".into()).unwrap();
        assert!(none(&l));
    }

    #[test]
    fn folders_and_index_are_validated() {
        let l = Lib::new();
        let scope = crate::offline::scope_name(SERVER, "alice");
        assert!(Downloads::open("relative/offline", &scope).is_none());
        assert!(Downloads::open(&format!("{}/../x", l.root), &scope).is_none());
        assert!(Downloads::open("", &scope).is_none());
        assert!(Downloads::open(&l.root, "../../etc").is_none());
        assert!(Downloads::open(&l.root, &"A".repeat(32)).is_none());
        // Another user's downloads are another folder.
        let pcm = noise(5000, 10);
        let file = flac(44_100, &pcm, &[], None);
        l.put(1, &file, DownloadFormat::Original, "flac");
        let bob = Downloads::open(&l.root, &crate::offline::scope_name(SERVER, "bob")).unwrap();
        assert!(!bob.may_have_flac());
        // A garbage or symlinked index: nothing.
        let d = l.downloads();
        assert!(d.may_have_flac());
        let index = store::index_file(&d.dir);
        let good = fs::read(&index).unwrap();
        fs::write(&index, b"{\"v\":1,\"tracks\":[{\"key\":").unwrap();
        assert!(!d.may_have_flac());
        #[cfg(unix)]
        {
            let elsewhere = std::path::Path::new(&l.root).join("index-elsewhere.json");
            fs::write(&elsewhere, &good).unwrap();
            fs::remove_file(&index).unwrap();
            std::os::unix::fs::symlink(&elsewhere, &index).unwrap();
            assert!(!d.may_have_flac());
            fs::remove_file(&index).unwrap();
            // A symlinked tracks folder.
            fs::write(&index, &good).unwrap();
            assert!(d.may_have_flac());
            let tracks = d.dir.join("tracks");
            let moved = std::path::Path::new(&l.root).join("tracks-elsewhere");
            fs::rename(&tracks, &moved).unwrap();
            std::os::unix::fs::symlink(&moved, &tracks).unwrap();
            assert!(!d.may_have_flac());
        }
    }

    #[test]
    fn a_verified_file_is_checked_again_once_it_changes() {
        let l = Lib::new();
        let pcm = noise(10_000, 11);
        let file = flac(44_100, &pcm, &[], None);
        let p = l.put(1, &file, DownloadFormat::Original, "flac");
        let d = l.downloads();
        let (info, ms) = info_of(&file);
        assert!(d.take(&info, ms, &file[..ms + 100]).is_some());
        assert!(d.take(&info, ms, &file[..ms + 100]).is_some(), "again");
        // Changed in place (same size, a new time): decoded again, and refused.
        let mut bad = file.clone();
        let at = bad.len() - 300;
        bad[at] ^= 0x10;
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&p, &bad).unwrap();
        assert!(d.take(&info, ms, &file[..ms + 100]).is_none());
    }

    #[test]
    fn every_download_in_place_is_found_by_track_for_playing_offline() {
        let l = Lib::new();
        let flac_file = flac(44_100, &noise(2000, 3), &[], None);
        l.put(1, &flac_file, DownloadFormat::Original, "flac");
        l.put(2, b"an opus file", DownloadFormat::Opus, "opus");
        let gone = l.put(3, b"an mp3 file", DownloadFormat::Original, "mp3");
        let d = l.downloads();
        let found = d.by_track();
        let id = |n: u8| sample(n).track_id;
        assert_eq!(found.len(), 3, "any format, not only Original FLAC");
        let t = &found[&id(2)];
        assert_eq!(t.ext, "opus");
        assert!(t.key.starts_with("download:"));
        assert!(t.path.starts_with(&d.dir), "only in this user's folder");
        // A file that went, or changed size, is not offered.
        fs::remove_file(&gone).unwrap();
        assert!(!d.by_track().contains_key(&id(3)));
        fs::write(&found[&id(2)].path, b"longer than it was recorded").unwrap();
        assert!(!d.by_track().contains_key(&id(2)));
        #[cfg(unix)]
        {
            // A symlink in its place: never followed.
            let p = found[&id(1)].path.clone();
            let elsewhere = std::path::Path::new(&l.root).join("elsewhere.flac");
            fs::write(&elsewhere, &flac_file).unwrap();
            fs::remove_file(&p).unwrap();
            std::os::unix::fs::symlink(&elsewhere, &p).unwrap();
            assert!(!d.by_track().contains_key(&id(1)));
        }
        // Another user's downloads are never found.
        let bob = Downloads::open(&l.root, &crate::offline::scope_name(SERVER, "bob")).unwrap();
        assert!(bob.by_track().is_empty());
    }
}
