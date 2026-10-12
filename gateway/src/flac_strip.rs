//! Drops embedded artwork (PICTURE) and PADDING metadata blocks from the start of a FLAC
//! stream on its way through the relay, so a player can start decoding sooner.
//!
//! A FLAC stream is `fLaC`, a chain of metadata blocks (4-byte header: last-block flag, type,
//! 24-bit length), then audio frames. Library files often carry a cover of 50–500 KB (and
//! sometimes far more) in a PICTURE block, plus PADDING; a player must read all of it before
//! the first frame. The artwork is served separately (`/v1/art`), and padding is only room
//! for editing tags in place, so neither is needed to play the track.
//!
//! # What is changed, and what never is
//!
//! - Only the metadata chain is rewritten: PICTURE (type 6) and PADDING (type 1) blocks are
//!   removed and the last-block flag is moved to the last block kept. Every other block
//!   (STREAMINFO, SEEKTABLE, VORBIS_COMMENT, CUESHEET, APPLICATION) and every audio byte is
//!   forwarded unchanged, so the decoded audio is bit-identical.
//! - The whole metadata chain is held until its end has been seen and checked, and anything
//!   unexpected sends the stream on **exactly as it came**:
//!   - no `fLaC` marker (an ID3 tag first, Ogg, a stream that starts mid-file after a
//!     seek, anything else);
//!   - a first block that isn't a 34-byte STREAMINFO, or a second STREAMINFO;
//!   - a reserved (7–126) or invalid (127) block type, a SEEKTABLE whose length isn't a
//!     whole number of points;
//!   - more than [`MAX_BLOCKS`] blocks, or metadata longer than [`MAX_METADATA`];
//!   - after the last block, no FLAC frame sync (`0xFFF8`/`0xFFF9`): this catches a lying
//!     last-block flag;
//!   - the stream ending (cleanly or not) before all of that was seen.
//! - Memory is bounded: at most [`MAX_METADATA`] plus one network chunk is held.
//!
//! # Offsets
//!
//! The relay applies this to its one upstream request before anything is counted, so every
//! offset the client sees (`?offset=`, acknowledgements, the replay window) is in the
//! stripped stream's coordinates, consistently: resuming needs no mapping. LMS, though,
//! counts the bytes it sent, and hears from the player how many it received (STAT). The
//! gateway converts those reports back ([`StripInfo::upstream_bytes`]) so LMS sees the
//! count it expects.

use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

/// Most metadata (marker and blocks) held before giving up and passing the stream on as it
/// is. Large enough for big scans of artwork; the replay window is 4 MiB per stream too.
pub const MAX_METADATA: usize = 16 * 1024 * 1024;
/// Most metadata blocks accepted.
pub const MAX_BLOCKS: usize = 1024;

const MARKER: &[u8; 4] = b"fLaC";
const STREAMINFO: u8 = 0;
const PADDING: u8 = 1;
const SEEKTABLE: u8 = 3;
const PICTURE: u8 = 6;
/// The highest block type defined (PICTURE).
const LAST_KNOWN_TYPE: u8 = 6;

/// What stripping did to a stream, once its metadata has been dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripInfo {
    /// Length of the (rewritten) metadata as the client receives it.
    pub header_out: u64,
    /// Bytes removed (all of them inside the metadata).
    pub removed: u64,
}

impl StripInfo {
    /// The byte count LMS sent for `client` bytes received. Everything removed lies before
    /// the first audio frame, so past the client's header the two differ by `removed`;
    /// inside it the count is left alone (monotonic, never ahead of the truth by more than
    /// what was removed).
    pub fn upstream_bytes(&self, client: u64) -> u64 {
        if client >= self.header_out {
            client.saturating_add(self.removed)
        } else {
            client
        }
    }
}

/// Outcome of looking at the metadata held so far.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    NeedMore,
    /// Not something to touch: send the stream on unchanged.
    PassThrough,
    /// The metadata ends at `consumed` (where the first frame starts); `header` replaces it.
    Strip {
        header: Vec<u8>,
        consumed: usize,
    },
}

/// Examine the start of a stream (`buf` holds everything so far).
fn examine(buf: &[u8]) -> Verdict {
    let n = buf.len().min(MARKER.len());
    if buf[..n] != MARKER[..n] {
        return Verdict::PassThrough;
    }
    if buf.len() < MARKER.len() {
        return Verdict::NeedMore;
    }
    let mut pos = MARKER.len();
    let mut kept: Vec<(u8, usize, usize)> = Vec::new(); // (type, body start, body end)
    let mut blocks = 0usize;
    loop {
        let Some(head) = buf.get(pos..pos + 4) else {
            return Verdict::NeedMore;
        };
        let last = head[0] & 0x80 != 0;
        let kind = head[0] & 0x7f;
        let len = u32::from_be_bytes([0, head[1], head[2], head[3]]) as usize;
        blocks += 1;
        if blocks > MAX_BLOCKS {
            return Verdict::PassThrough;
        }
        let first = blocks == 1;
        if first != (kind == STREAMINFO) || (first && len != 34) {
            return Verdict::PassThrough;
        }
        if kind > LAST_KNOWN_TYPE || (kind == SEEKTABLE && !len.is_multiple_of(18)) {
            return Verdict::PassThrough;
        }
        let start = pos + 4;
        let end = start + len; // at most MAX_METADATA + 2^24: no overflow
        if end > MAX_METADATA {
            return Verdict::PassThrough;
        }
        if kind != PICTURE && kind != PADDING {
            kept.push((kind, start, end));
        }
        pos = end;
        if last {
            break;
        }
    }
    // The first frame must follow at once: a frame header starts with the sync code.
    let Some(sync) = buf.get(pos..pos + 2) else {
        return Verdict::NeedMore;
    };
    if sync[0] != 0xFF || sync[1] & 0xFE != 0xF8 {
        return Verdict::PassThrough;
    }
    let mut header = Vec::with_capacity(pos);
    header.extend_from_slice(MARKER);
    let count = kept.len();
    for (i, (kind, start, end)) in kept.into_iter().enumerate() {
        let flag = if i + 1 == count { 0x80 } else { 0 };
        header.push(flag | kind);
        header.extend_from_slice(&((end - start) as u32).to_be_bytes()[1..]);
        header.extend_from_slice(&buf[start..end]);
    }
    Verdict::Strip {
        header,
        consumed: pos,
    }
}

/// Byte-level stripper: feed it the stream in any chunking, get the output back.
pub struct Stripper {
    /// Everything received while the metadata is still being examined.
    held: Vec<u8>,
    /// Metadata dealt with: everything from now on is forwarded as it comes.
    forwarding: bool,
    info: Arc<OnceLock<StripInfo>>,
}

impl Stripper {
    pub fn new(info: Arc<OnceLock<StripInfo>>) -> Self {
        Stripper {
            held: Vec::new(),
            forwarding: false,
            info,
        }
    }

    /// The next piece of the stream; returns what to send on (possibly nothing yet).
    pub fn push(&mut self, chunk: Bytes) -> Bytes {
        if self.forwarding {
            return chunk;
        }
        self.held.extend_from_slice(&chunk);
        match examine(&self.held) {
            // `examine` never needs more than the limit and one block header.
            Verdict::NeedMore if self.held.len() <= MAX_METADATA + 4 => Bytes::new(),
            Verdict::NeedMore | Verdict::PassThrough => self.pass_through(),
            Verdict::Strip { header, consumed } => {
                let info = StripInfo {
                    header_out: header.len() as u64,
                    removed: (consumed - header.len()) as u64,
                };
                let mut out = header;
                out.extend_from_slice(&self.held[consumed..]);
                self.finish_with(info);
                Bytes::from(out)
            }
        }
    }

    /// The stream ended (cleanly or not): whatever is held goes on unchanged.
    pub fn finish(&mut self) -> Bytes {
        if self.forwarding {
            return Bytes::new();
        }
        self.pass_through()
    }

    fn pass_through(&mut self) -> Bytes {
        let out = std::mem::take(&mut self.held);
        self.finish_with(StripInfo {
            header_out: 0,
            removed: 0,
        });
        Bytes::from(out)
    }

    fn finish_with(&mut self, info: StripInfo) {
        self.forwarding = true;
        self.held = Vec::new();
        let _ = self.info.set(info);
    }
}

/// A byte stream with [`Stripper`] applied.
pub struct StripStream<S, E> {
    inner: S,
    strip: Stripper,
    /// An upstream error, reported after the bytes held before it.
    error: Option<E>,
    ended: bool,
}

impl<S, E> StripStream<S, E> {
    pub fn new(inner: S, info: Arc<OnceLock<StripInfo>>) -> Self {
        StripStream {
            inner,
            strip: Stripper::new(info),
            error: None,
            ended: false,
        }
    }
}

impl<S, E> Stream for StripStream<S, E>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Unpin,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(e) = this.error.take() {
                this.ended = true;
                return Poll::Ready(Some(Err(e)));
            }
            if this.ended {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(chunk))) => {
                    let out = this.strip.push(chunk);
                    if !out.is_empty() {
                        return Poll::Ready(Some(Ok(out)));
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    let out = this.strip.finish();
                    this.error = Some(e);
                    if !out.is_empty() {
                        return Poll::Ready(Some(Ok(out)));
                    }
                }
                Poll::Ready(None) => {
                    let out = this.strip.finish();
                    this.ended = true;
                    if !out.is_empty() {
                        return Poll::Ready(Some(Ok(out)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use futures_util::StreamExt;

    /// Small deterministic PRNG (xorshift64*), so failures reproduce.
    pub struct Rng(u64);
    impl Rng {
        pub fn new(seed: u64) -> Self {
            Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        pub fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    fn crc8(data: &[u8]) -> u8 {
        let mut crc = 0u8;
        for b in data {
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
        for b in data {
            crc ^= (*b as u16) << 8;
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

    pub fn block(last: bool, kind: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![(u8::from(last) << 7) | kind];
        v.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        v.extend_from_slice(body);
        v
    }

    pub const BLOCK: usize = 1152;

    pub fn streaminfo(frames: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(BLOCK as u16).to_be_bytes());
        v.extend_from_slice(&(BLOCK as u16).to_be_bytes());
        v.extend_from_slice(&[0; 6]);
        let packed: u64 =
            (44_100u64 << 44) | (1 << 41) | (15 << 36) | (frames as u64 * BLOCK as u64);
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0; 16]);
        v
    }

    pub fn picture(size: usize, seed: u64) -> Vec<u8> {
        let mut rng = Rng::new(seed);
        let mime = b"image/jpeg";
        let mut pic = Vec::with_capacity(size + 64);
        pic.extend_from_slice(&3u32.to_be_bytes());
        pic.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        pic.extend_from_slice(mime);
        pic.extend_from_slice(&0u32.to_be_bytes());
        for x in [500u32, 500, 24, 0] {
            pic.extend_from_slice(&x.to_be_bytes());
        }
        pic.extend_from_slice(&(size as u32).to_be_bytes());
        // Artwork may contain anything, including bytes that look like FLAC structures.
        pic.extend((0..size).map(|i| match i % 97 {
            0 => 0xFF,
            1 => 0xF8,
            2 => b'f',
            _ => rng.next() as u8,
        }));
        pic
    }

    pub fn vorbis_comment() -> Vec<u8> {
        let vendor = b"test encoder";
        let mut v = (vendor.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(vendor);
        let tags = ["TITLE=Test", "REPLAYGAIN_TRACK_GAIN=-3.50 dB"];
        v.extend_from_slice(&(tags.len() as u32).to_le_bytes());
        for t in tags {
            v.extend_from_slice(&(t.len() as u32).to_le_bytes());
            v.extend_from_slice(t.as_bytes());
        }
        v
    }

    /// Audio frames (16-bit stereo, verbatim subframes) of pseudo-random samples, and the
    /// samples themselves (interleaved).
    pub fn frames(count: usize, seed: u64) -> (Vec<u8>, Vec<i16>) {
        let mut rng = Rng::new(seed);
        let mut out = Vec::new();
        let mut pcm = vec![0i16; count * BLOCK * 2];
        for n in 0..count {
            assert!(n < 0x800, "frame numbers up to two UTF-8 bytes");
            // Sync, blocking fixed; block size 1152 (code 3); 44.1 kHz (9);
            // left/right independent stereo (1), 16-bit (4).
            let mut f = vec![0xFF, 0xF8, 0x39, 0x18];
            if n < 0x80 {
                f.push(n as u8);
            } else {
                f.extend_from_slice(&[0xC0 | (n >> 6) as u8, 0x80 | (n & 0x3F) as u8]);
            }
            f.push(crc8(&f));
            for ch in 0..2 {
                f.push(0x02); // verbatim
                for i in 0..BLOCK {
                    let s = rng.next() as i16 / 4;
                    pcm[(n * BLOCK + i) * 2 + ch] = s;
                    f.extend_from_slice(&s.to_be_bytes());
                }
            }
            let c = crc16(&f);
            f.extend_from_slice(&c.to_be_bytes());
            out.extend(f);
        }
        (out, pcm)
    }

    /// A whole FLAC file: STREAMINFO, then `extra` blocks in order (the last flag is set on
    /// the final one), then `count` frames.
    pub fn flac(extra: &[(u8, Vec<u8>)], count: usize, seed: u64) -> (Vec<u8>, Vec<i16>) {
        let mut v = MARKER.to_vec();
        v.extend(block(extra.is_empty(), STREAMINFO, &streaminfo(count)));
        for (i, (kind, body)) in extra.iter().enumerate() {
            v.extend(block(i + 1 == extra.len(), *kind, body));
        }
        let (audio, pcm) = frames(count, seed);
        v.extend(audio);
        (v, pcm)
    }

    pub fn typical(picture_size: usize, count: usize, seed: u64) -> (Vec<u8>, Vec<i16>) {
        flac(
            &[
                (SEEKTABLE, vec![0; 36]),
                (4, vorbis_comment()),
                (PICTURE, picture(picture_size, seed)),
                (PADDING, vec![0; 8192]),
            ],
            count,
            seed,
        )
    }

    /// Run `input` through a stripper in chunks of the sizes `rng` picks.
    fn run(input: &[u8], rng: &mut Rng, max_chunk: u64) -> (Vec<u8>, StripInfo) {
        let info = Arc::new(OnceLock::new());
        let mut s = Stripper::new(info.clone());
        let mut out = Vec::new();
        let mut rest = input;
        while !rest.is_empty() {
            let n = (1 + rng.below(max_chunk) as usize).min(rest.len());
            out.extend_from_slice(&s.push(Bytes::copy_from_slice(&rest[..n])));
            rest = &rest[n..];
        }
        out.extend_from_slice(&s.finish());
        (out, *info.get().expect("decided by the end of the stream"))
    }

    fn strip_all(input: &[u8]) -> (Vec<u8>, StripInfo) {
        run(input, &mut Rng::new(1), u64::MAX / 2)
    }

    /// Decode a stream with the players' own decoder, as interleaved stereo `f32`.
    pub fn decode(bytes: &[u8]) -> Vec<f32> {
        use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
        let input = InputBuffer::new();
        input.push(bytes);
        input.set_eof();
        let mut dec = StreamDecoder::new(input, b'f');
        let mut pcm = Vec::new();
        loop {
            match dec.work(64, &mut |_, s| pcm.extend_from_slice(s)) {
                DecodeStatus::Finished => return pcm,
                DecodeStatus::Failed(e) => panic!("decode failed: {e}"),
                _ => {}
            }
        }
    }

    fn blocks_of(stream: &[u8]) -> Vec<(bool, u8, usize)> {
        let mut pos = 4;
        let mut v = Vec::new();
        loop {
            let h = &stream[pos..pos + 4];
            let len = u32::from_be_bytes([0, h[1], h[2], h[3]]) as usize;
            v.push((h[0] & 0x80 != 0, h[0] & 0x7f, len));
            pos += 4 + len;
            if h[0] & 0x80 != 0 {
                return v;
            }
        }
    }

    #[test]
    fn pictures_and_padding_are_dropped_and_the_rest_is_untouched() {
        let (input, _) = typical(300_000, 20, 7);
        let (out, info) = strip_all(&input);
        assert_eq!(
            blocks_of(&out),
            vec![
                (false, 0, 34),
                (false, 3, 36),
                (true, 4, vorbis_comment().len())
            ]
        );
        let header_in =
            4 + 38 + 40 + 4 + vorbis_comment().len() + 4 + picture(300_000, 7).len() + 4 + 8192;
        assert_eq!(
            info.header_out as usize,
            out.len() - (input.len() - header_in)
        );
        assert_eq!(info.removed as usize, input.len() - out.len());
        // Audio bytes identical, and the kept blocks' contents identical.
        assert_eq!(&out[info.header_out as usize..], &input[header_in..]);
        assert_eq!(
            &out[4..info.header_out as usize - 4 - vorbis_comment().len()],
            &{
                let mut v = block(false, 0, &streaminfo(20));
                v.extend(block(false, 3, &[0; 36]));
                v
            }[..]
        );
    }

    #[test]
    fn last_block_flag_moves_when_the_last_block_is_dropped_or_kept() {
        // Picture last: the flag moves to the comment.
        let (input, _) = flac(&[(4, vorbis_comment()), (PICTURE, picture(100, 1))], 2, 1);
        let (out, _) = strip_all(&input);
        assert_eq!(
            blocks_of(&out),
            vec![(false, 0, 34), (true, 4, vorbis_comment().len())]
        );
        // Picture in the middle: the comment keeps its flag.
        let (input, _) = flac(&[(PICTURE, picture(100, 1)), (4, vorbis_comment())], 2, 1);
        let (out, _) = strip_all(&input);
        assert_eq!(
            blocks_of(&out),
            vec![(false, 0, 34), (true, 4, vorbis_comment().len())]
        );
        // Only STREAMINFO left.
        let (input, _) = flac(&[(PICTURE, picture(100, 1)), (PADDING, vec![0; 10])], 2, 1);
        let (out, _) = strip_all(&input);
        assert_eq!(blocks_of(&out), vec![(true, 0, 34)]);
        // Nothing to drop: identical.
        let (input, _) = flac(&[(4, vorbis_comment())], 2, 1);
        let (out, info) = strip_all(&input);
        assert_eq!((out, info.removed), (input, 0));
    }

    #[test]
    fn stripped_stream_decodes_to_the_same_pcm_bit_for_bit() {
        let mut rng = Rng::new(42);
        for round in 0..12u64 {
            let mut extra: Vec<(u8, Vec<u8>)> = vec![];
            for _ in 0..rng.below(6) {
                let pick = rng.below(5);
                // At most one seek table and one comment block, as the format requires.
                if (pick == 2 && extra.iter().any(|b| b.0 == 4))
                    || (pick == 3 && extra.iter().any(|b| b.0 == SEEKTABLE))
                {
                    continue;
                }
                extra.push(match pick {
                    0 => (PICTURE, picture(rng.below(200_000) as usize, round)),
                    1 => (PADDING, vec![0; rng.below(10_000) as usize]),
                    2 => (4, vorbis_comment()),
                    3 => (SEEKTABLE, vec![0; 18 * rng.below(4) as usize]),
                    _ => (2, b"TEST appdata".to_vec()), // APPLICATION
                });
            }
            let (input, pcm) = flac(&extra, 1 + rng.below(12) as usize, round);
            let (out, info) = run(&input, &mut rng, 70_000);
            assert_eq!(input.len() - out.len(), info.removed as usize);
            let want: Vec<f32> = pcm.iter().map(|s| *s as f32 / 32768.0).collect();
            let original = decode(&input);
            let stripped = decode(&out);
            assert_eq!(original.len(), want.len(), "round {round}");
            assert!(
                original
                    .iter()
                    .zip(&want)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "the original decodes to the samples written (round {round})"
            );
            assert!(
                stripped
                    .iter()
                    .zip(&original)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
                    && stripped.len() == original.len(),
                "stripped audio must be bit-identical (round {round})"
            );
        }
    }

    #[test]
    fn chunking_never_changes_the_output() {
        let (input, _) = typical(50_000, 4, 3);
        let (whole, info) = strip_all(&input);
        let mut rng = Rng::new(9);
        for max in [1, 2, 3, 5, 17, 4096, 65_536] {
            let (out, i) = run(&input, &mut rng, max);
            assert_eq!((&out, i), (&whole, info), "chunks up to {max}");
        }
    }

    /// Everything that must pass the stream through byte for byte.
    #[test]
    fn unexpected_streams_pass_through_unchanged() {
        let (good, _) = typical(1000, 2, 5);
        let (vc, pic) = (vorbis_comment(), picture(1000, 5));
        let audio = frames(2, 5).0;
        let with = |blocks: Vec<Vec<u8>>, tail: &[u8]| {
            let mut v = MARKER.to_vec();
            for b in blocks {
                v.extend(b);
            }
            v.extend_from_slice(tail);
            v
        };
        let si = || block(false, STREAMINFO, &streaminfo(2));
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("short marker", b"fLa".to_vec()),
            ("not FLAC", b"ID3\x04\0\0\0\0\0\0fLaC".to_vec()),
            ("mid-file after a seek", audio.clone()),
            ("Ogg", b"OggS\0\x02".to_vec()),
            ("marker only", MARKER.to_vec()),
            ("truncated block header", good[..6].to_vec()),
            ("truncated in the picture", good[..good.len() / 2].to_vec()),
            (
                "first block not STREAMINFO",
                with(vec![block(false, 4, &vc), si()], &audio),
            ),
            (
                "STREAMINFO of the wrong size",
                with(vec![block(true, STREAMINFO, &[0; 33])], &audio),
            ),
            (
                "second STREAMINFO",
                with(
                    vec![
                        si(),
                        block(false, PICTURE, &pic),
                        block(true, 0, &streaminfo(2)),
                    ],
                    &audio,
                ),
            ),
            (
                "reserved type",
                with(
                    vec![si(), block(false, PICTURE, &pic), block(true, 9, b"x")],
                    &audio,
                ),
            ),
            (
                "invalid type 127",
                with(vec![si(), block(true, 127, b"x")], &audio),
            ),
            (
                "bad seek table",
                with(vec![si(), block(true, SEEKTABLE, &[0; 20])], &audio),
            ),
            (
                "lying last flag: never set (frames read as blocks)",
                with(
                    vec![si(), block(false, PICTURE, &pic), block(false, 4, &vc)],
                    &audio,
                ),
            ),
            (
                "lying last flag: set too early (no frame sync follows)",
                with(
                    vec![si(), block(true, PICTURE, &pic), block(true, 4, &vc)],
                    &audio,
                ),
            ),
            (
                "no audio after the metadata",
                with(vec![si(), block(true, PICTURE, &pic)], &[]),
            ),
            (
                "garbage after the metadata",
                with(vec![si(), block(true, PICTURE, &pic)], b"\x00\x01garbage"),
            ),
            (
                "a block header claiming 16 MiB at the end",
                with(
                    vec![si(), block(false, PICTURE, &pic)],
                    &[0x06, 0xFF, 0xFF, 0xFF],
                ),
            ),
        ];
        for (what, input) in cases {
            let mut rng = Rng::new(11);
            for max in [1, 7, 1 << 20] {
                let (out, info) = run(&input, &mut rng, max);
                assert_eq!(out, input, "{what} (chunks up to {max})");
                assert_eq!(info.removed, 0, "{what}");
            }
        }
    }

    #[test]
    fn metadata_past_the_limit_passes_through_without_holding_it_all() {
        // A 15 MiB picture, then blocks of 0x06 bytes (each a ~386 KB "picture"): the chain
        // crosses the limit, and from then on everything goes on unchanged. Nothing beyond
        // the limit and one chunk is ever held.
        let info = Arc::new(OnceLock::new());
        let mut s = Stripper::new(info.clone());
        let mut head = MARKER.to_vec();
        head.extend(block(false, STREAMINFO, &streaminfo(1)));
        head.push(PICTURE);
        head.extend_from_slice(&(15u32 << 20).to_be_bytes()[1..]);
        let mut pushed = head.len();
        let mut sent = s.push(Bytes::from(head)).len();
        assert_eq!(sent, 0, "held while the metadata may still fit");
        let chunk = Bytes::from(vec![0x06u8; 1 << 20]);
        for _ in 0..20 {
            pushed += chunk.len();
            sent += s.push(chunk.clone()).len();
            assert!(s.held.len() <= MAX_METADATA + (1 << 20));
        }
        assert_eq!(sent, pushed, "passed through once past the limit");
        assert_eq!(info.get().unwrap().removed, 0);
    }

    #[test]
    fn too_many_blocks_pass_through() {
        let mut extra: Vec<(u8, Vec<u8>)> = (0..MAX_BLOCKS).map(|_| (PADDING, vec![])).collect();
        extra.push((PICTURE, picture(10, 1)));
        let (input, _) = flac(&extra, 1, 1);
        let (out, info) = strip_all(&input);
        assert_eq!((out == input, info.removed), (true, 0));
        // One fewer is fine.
        extra.drain(..2);
        let (input, _) = flac(&extra, 1, 1);
        let (out, info) = strip_all(&input);
        assert!(out.len() < input.len() && info.removed > 0);
    }

    /// Fuzz: random, mutated, truncated and spliced streams never panic, the output is
    /// either the input unchanged or a valid strip of it, and chunking never matters.
    #[test]
    fn fuzz_never_panics_and_output_is_input_or_a_clean_strip() {
        let mut rng = Rng::new(2026);
        let (base, _) = typical(3000, 3, 1);
        for i in 0..4000u64 {
            let mut v = match rng.below(4) {
                0 => (0..rng.below(300)).map(|_| rng.next() as u8).collect(),
                1 => base.clone(),
                2 => {
                    // Marker and random block headers ("nested garbage").
                    let mut v = MARKER.to_vec();
                    for _ in 0..rng.below(8) {
                        let len = rng.below(64) as usize;
                        let kind = rng.below(130) as u8;
                        v.extend(block(rng.below(3) == 0, kind & 0x7f, &vec![0xFF; len]));
                    }
                    v.extend((0..rng.below(40)).map(|_| rng.next() as u8));
                    v
                }
                _ => base[..rng.below(base.len() as u64) as usize].to_vec(),
            };
            for _ in 0..rng.below(4) {
                if v.is_empty() {
                    break;
                }
                let at = rng.below(v.len() as u64) as usize;
                match rng.below(3) {
                    0 => v[at] = rng.next() as u8,
                    1 => v[at] ^= 0x80, // flip a last-block flag (or anything)
                    _ => v.truncate(at),
                }
            }
            let (whole, info) = run(&v, &mut Rng::new(i), u64::MAX / 2);
            let (pieces, info2) = run(&v, &mut Rng::new(i), 9);
            assert_eq!((&whole, info), (&pieces, info2), "case {i}");
            if info.removed == 0 {
                assert_eq!(whole, v, "case {i}: unchanged when nothing was removed");
            } else {
                assert_eq!(v.len() - whole.len(), info.removed as usize);
                let h = info.header_out as usize;
                let tail = &whole[h..];
                assert_eq!(
                    tail,
                    &v[v.len() - tail.len()..],
                    "case {i}: audio untouched"
                );
                assert_eq!(&whole[..4], MARKER);
                assert_eq!(blocks_of(&whole).iter().filter(|b| b.0).count(), 1);
            }
        }
    }

    #[test]
    fn upstream_bytes_maps_client_counts_back() {
        let i = StripInfo {
            header_out: 100,
            removed: 5000,
        };
        assert_eq!(i.upstream_bytes(0), 0);
        assert_eq!(i.upstream_bytes(99), 99);
        assert_eq!(i.upstream_bytes(100), 5100);
        assert_eq!(i.upstream_bytes(u64::MAX), u64::MAX);
        let none = StripInfo {
            header_out: 0,
            removed: 0,
        };
        assert_eq!(none.upstream_bytes(1234), 1234);
    }

    #[tokio::test]
    async fn stream_adapter_holds_then_forwards_and_reports_errors_after_held_bytes() {
        let (input, _) = typical(20_000, 3, 2);
        let chunks: Vec<Result<Bytes, &'static str>> = input
            .chunks(1000)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        let info = Arc::new(OnceLock::new());
        let s = StripStream::new(futures_util::stream::iter(chunks), info.clone());
        let got: Vec<Bytes> = s.map(|r| r.unwrap()).collect().await;
        assert!(got.iter().all(|b| !b.is_empty()), "no empty chunks");
        let out: Vec<u8> = got.concat();
        assert_eq!(out, strip_all(&input).0);
        assert!(info.get().unwrap().removed > 20_000);

        // LMS fails inside the metadata: what was held goes on unchanged, then the error.
        let chunks: Vec<Result<Bytes, &'static str>> = vec![
            Ok(Bytes::copy_from_slice(&input[..5000])),
            Err("broken"),
            Ok(Bytes::from_static(b"never")),
        ];
        let info = Arc::new(OnceLock::new());
        let mut s = StripStream::new(futures_util::stream::iter(chunks), info.clone());
        assert_eq!(s.next().await.unwrap().unwrap(), &input[..5000]);
        assert_eq!(s.next().await.unwrap(), Err("broken"));
        assert!(s.next().await.is_none());
        assert_eq!(info.get().unwrap().removed, 0);
    }
}
