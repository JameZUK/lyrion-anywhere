//! Incremental decoding of a network audio stream with no blocking I/O.
//!
//! Symphonia pulls bytes through `std::io::Read` and treats a zero-length read as end of
//! stream, so it must never be asked to read data that hasn't arrived yet. To keep this
//! usable from WebAssembly (no threads, no blocking), [`StreamDecoder`] only demuxes while
//! at least [`DECODE_MARGIN`] unread bytes are buffered, or once the stream has ended.
//! Probing is retried from the start of the stream if the headers (which can carry several
//! MiB of embedded artwork) haven't fully arrived yet.

use std::collections::VecDeque;
use std::io::{self, Read, Seek};
use std::sync::{Arc, Mutex};

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known as ids;
use symphonia::core::codecs::audio::{
    AudioCodecId, AudioCodecParameters, AudioDecoder, AudioDecoderOptions,
};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions, ReadOnlySource};
use symphonia::core::meta::{MetadataOptions, StandardTag};

/// Unread bytes required before demuxing a packet. Covers Symphonia's 32 KiB read-ahead
/// block plus the largest realistic compressed frame, with headroom.
pub const DECODE_MARGIN: usize = 128 * 1024;

/// Most bytes buffered while probing a stream's headers. Embedded artwork can make the
/// headers several MiB long, so the player keeps reading past its usual input limit while
/// probing; a stream whose headers still haven't been parsed at this size fails cleanly
/// instead of waiting for ever.
pub const PROBE_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
struct BufInner {
    data: VecDeque<u8>,
    /// Absolute stream offset of `data[0]`.
    base: u64,
    /// Absolute read position.
    pos: u64,
    eof: bool,
    /// While probing, keep everything so the probe can be retried from byte 0.
    keep_all: bool,
}

/// Byte buffer shared between the network side (writer) and the demuxer (reader).
#[derive(Clone, Default)]
pub struct InputBuffer(Arc<Mutex<BufInner>>);

impl InputBuffer {
    pub fn new() -> Self {
        let b = InputBuffer::default();
        b.0.lock().unwrap().keep_all = true;
        b
    }

    pub fn push(&self, d: &[u8]) {
        self.0.lock().unwrap().data.extend(d);
    }

    pub fn set_eof(&self) {
        self.0.lock().unwrap().eof = true;
    }

    pub fn eof(&self) -> bool {
        self.0.lock().unwrap().eof
    }

    /// Bytes received but not yet read by the demuxer.
    pub fn unread(&self) -> usize {
        let b = self.0.lock().unwrap();
        (b.base + b.data.len() as u64 - b.pos) as usize
    }

    fn rewind(&self) {
        let mut b = self.0.lock().unwrap();
        b.pos = b.base;
    }

    /// Run `f` over the bytes buffered from the current base (all of them while probing).
    fn with_buffered<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let mut b = self.0.lock().unwrap();
        f(b.data.make_contiguous())
    }

    fn release(&self) {
        let mut b = self.0.lock().unwrap();
        b.keep_all = false;
        let consumed = (b.pos - b.base) as usize;
        b.data.drain(..consumed);
        b.base = b.pos;
    }
}

impl Read for InputBuffer {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut b = self.0.lock().unwrap();
        let off = (b.pos - b.base) as usize;
        let avail = b.data.len() - off;
        let n = avail.min(out.len());
        let (a, c) = b.data.as_slices();
        // Copy across the deque's two slices.
        for (i, slot) in out[..n].iter_mut().enumerate() {
            let k = off + i;
            *slot = if k < a.len() { a[k] } else { c[k - a.len()] };
        }
        b.pos += n as u64;
        if !b.keep_all {
            b.data.drain(..off + n);
            b.base = b.pos;
        }
        Ok(n)
    }
}

/// What a call to [`StreamDecoder::work`] achieved.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeStatus {
    /// Decoded at least one packet; call again.
    Progress,
    /// Waiting for more network data.
    NeedData,
    /// The whole stream has been decoded.
    Finished,
    /// Unrecoverable (unsupported or corrupt stream).
    Failed(String),
}

enum State {
    Probing {
        min_bytes: usize,
    },
    Decoding {
        reader: Box<dyn FormatReader>,
        decoder: Box<dyn AudioDecoder>,
        track_id: u32,
    },
    Done,
    Failed(String),
}

/// ReplayGain values read from a stream's tags (Vorbis comments, ID3v2, MP4), if present
/// and sane. Gains are in dB; peaks are linear sample peaks (1.0 = full scale).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReplayGainTags {
    pub track_gain_db: Option<f64>,
    pub track_peak: Option<f64>,
    pub album_gain_db: Option<f64>,
    pub album_peak: Option<f64>,
}

/// Longest tag value [`parse_gain_db`] and [`parse_peak`] look at.
const MAX_TAG_VALUE: usize = 64;

/// A ReplayGain gain tag such as `-6.48 dB` or `+1.2dB`. Tag text is untrusted: anything
/// malformed, non-finite or outside ±60 dB is refused.
pub fn parse_gain_db(s: &str) -> Option<f64> {
    if s.len() > MAX_TAG_VALUE {
        return None;
    }
    let t = s.trim();
    let t = t
        .strip_suffix("dB")
        .or_else(|| t.strip_suffix("db"))
        .or_else(|| t.strip_suffix("DB"))
        .unwrap_or(t)
        .trim_end();
    let v = parse_plain_number(t)?;
    (v.abs() <= 60.0).then_some(v)
}

/// A ReplayGain peak tag such as `0.988525`. Refused unless it is a plain number within
/// 0..=16 (a peak above +24 dBFS is nonsense).
pub fn parse_peak(s: &str) -> Option<f64> {
    if s.len() > MAX_TAG_VALUE {
        return None;
    }
    let v = parse_plain_number(s.trim())?;
    (0.0..=16.0).contains(&v).then_some(v)
}

/// A plain decimal number: optional sign, digits, optional fraction. No exponents, `inf`
/// or `nan` (which `f64::from_str` would accept).
fn parse_plain_number(t: &str) -> Option<f64> {
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    let ok = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && digits.bytes().filter(|b| *b == b'.').count() <= 1
        && digits.bytes().any(|b| b.is_ascii_digit());
    if !ok {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// What a stream turned out to be, once its headers have been probed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceFormat {
    /// Display name, e.g. "FLAC".
    pub codec: &'static str,
    pub lossless: bool,
    /// Integer bit depth of a lossless source (`None` for lossy or float sources).
    pub bits: Option<u32>,
    /// Channels in the source (the decoder's output is always stereo).
    pub channels: u32,
    pub replay_gain: ReplayGainTags,
}

/// Integer PCM codecs: lossless, with a meaningful bit depth.
const INT_PCM: [AudioCodecId; 28] = [
    ids::CODEC_ID_PCM_S32LE,
    ids::CODEC_ID_PCM_S32LE_PLANAR,
    ids::CODEC_ID_PCM_S32BE,
    ids::CODEC_ID_PCM_S32BE_PLANAR,
    ids::CODEC_ID_PCM_S24LE,
    ids::CODEC_ID_PCM_S24LE_PLANAR,
    ids::CODEC_ID_PCM_S24BE,
    ids::CODEC_ID_PCM_S24BE_PLANAR,
    ids::CODEC_ID_PCM_S16LE,
    ids::CODEC_ID_PCM_S16LE_PLANAR,
    ids::CODEC_ID_PCM_S16BE,
    ids::CODEC_ID_PCM_S16BE_PLANAR,
    ids::CODEC_ID_PCM_S8,
    ids::CODEC_ID_PCM_S8_PLANAR,
    ids::CODEC_ID_PCM_U32LE,
    ids::CODEC_ID_PCM_U32LE_PLANAR,
    ids::CODEC_ID_PCM_U32BE,
    ids::CODEC_ID_PCM_U32BE_PLANAR,
    ids::CODEC_ID_PCM_U24LE,
    ids::CODEC_ID_PCM_U24LE_PLANAR,
    ids::CODEC_ID_PCM_U24BE,
    ids::CODEC_ID_PCM_U24BE_PLANAR,
    ids::CODEC_ID_PCM_U16LE,
    ids::CODEC_ID_PCM_U16LE_PLANAR,
    ids::CODEC_ID_PCM_U16BE,
    ids::CODEC_ID_PCM_U16BE_PLANAR,
    ids::CODEC_ID_PCM_U8,
    ids::CODEC_ID_PCM_U8_PLANAR,
];

/// Float PCM codecs: lossless, but with no integer depth.
const FLOAT_PCM: [AudioCodecId; 8] = [
    ids::CODEC_ID_PCM_F32LE,
    ids::CODEC_ID_PCM_F32LE_PLANAR,
    ids::CODEC_ID_PCM_F32BE,
    ids::CODEC_ID_PCM_F32BE_PLANAR,
    ids::CODEC_ID_PCM_F64LE,
    ids::CODEC_ID_PCM_F64LE_PLANAR,
    ids::CODEC_ID_PCM_F64BE,
    ids::CODEC_ID_PCM_F64BE_PLANAR,
];

fn source_format(params: &AudioCodecParameters, reader: &mut dyn FormatReader) -> SourceFormat {
    let c = params.codec;
    let (codec, lossless, has_depth) = match c {
        _ if c == ids::CODEC_ID_FLAC => ("FLAC", true, true),
        _ if c == ids::CODEC_ID_ALAC => ("ALAC", true, true),
        _ if INT_PCM.contains(&c) => ("PCM", true, true),
        _ if FLOAT_PCM.contains(&c) => ("PCM (float)", true, false),
        _ if c == ids::CODEC_ID_MP3 => ("MP3", false, false),
        _ if c == ids::CODEC_ID_AAC => ("AAC", false, false),
        _ if c == ids::CODEC_ID_VORBIS => ("Vorbis", false, false),
        _ if c == ids::CODEC_ID_OPUS => ("Opus", false, false),
        _ => ("Other", false, false),
    };
    let bits = if has_depth {
        params.bits_per_sample.filter(|b| (1..=32).contains(b))
    } else {
        None
    };
    let mut rg = ReplayGainTags::default();
    let mut meta = reader.metadata();
    if let Some(rev) = meta.skip_to_latest() {
        let per_track = rev.per_track.iter().flat_map(|t| t.metadata.tags.iter());
        for tag in rev.media.tags.iter().chain(per_track) {
            match &tag.std {
                Some(StandardTag::ReplayGainTrackGain(v)) => rg.track_gain_db = parse_gain_db(v),
                Some(StandardTag::ReplayGainTrackPeak(v)) => rg.track_peak = parse_peak(v),
                Some(StandardTag::ReplayGainAlbumGain(v)) => rg.album_gain_db = parse_gain_db(v),
                Some(StandardTag::ReplayGainAlbumPeak(v)) => rg.album_peak = parse_peak(v),
                _ => {}
            }
        }
    }
    SourceFormat {
        codec,
        lossless,
        bits,
        channels: params
            .channels
            .as_ref()
            .map_or(2, |ch| ch.count().clamp(1, 64) as u32),
        replay_gain: rg,
    }
}

/// Decodes one track's byte stream into interleaved stereo `f32` PCM.
pub struct StreamDecoder {
    input: InputBuffer,
    hint: Hint,
    state: State,
    tmp: Vec<f32>,
    stereo: Vec<f32>,
    source: Option<SourceFormat>,
}

/// The error [`StreamDecoder::work`] reports when it caught a panic in a third-party demuxer
/// or decoder (native hosts only: WebAssembly builds abort instead). Tests assert that no
/// hostile input ever produces it, because the same input would abort the web player.
pub const STREAM_DECODER_PANICKED: &str = "decode: the decoder failed on this stream";

/// The error [`decode_seekable`], [`verify_flac`] and the file decoder's open report when
/// they caught a panic in a third-party demuxer or decoder (see [`STREAM_DECODER_PANICKED`]).
pub const FILE_DECODER_PANICKED: &str = "decoder failed on this file";

impl StreamDecoder {
    /// `format` is the Slimproto format code from `strm` (used only as a probe hint).
    pub fn new(input: InputBuffer, format: u8) -> Self {
        let mut hint = Hint::new();
        if let Some(ext) = format_extension(format) {
            hint.with_extension(ext);
        }
        StreamDecoder {
            input,
            hint,
            state: State::Probing {
                min_bytes: DECODE_MARGIN,
            },
            tmp: Vec::new(),
            stereo: Vec::new(),
            source: None,
        }
    }

    /// The stream's format, once its headers have been probed.
    pub fn source_format(&self) -> Option<&SourceFormat> {
        self.source.as_ref()
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Still looking for the stream's headers (input may grow up to [`PROBE_MAX_BYTES`]).
    pub fn is_probing(&self) -> bool {
        matches!(self.state, State::Probing { .. })
    }

    /// Decode up to `max_packets`, passing each block of PCM to `sink(sample_rate, samples)`.
    ///
    /// Stream bytes are untrusted. Headers known to trip arithmetic overflow checks in the
    /// third-party demuxers are refused before they are parsed ([`check_wav_headers`]), which
    /// also protects WebAssembly builds (`panic = "abort"`, where a panic can't be caught).
    /// Where panics unwind (native hosts), any other panic inside the demuxer or decoder is
    /// caught as well and fails the stream cleanly.
    pub fn work(&mut self, max_packets: usize, sink: &mut dyn FnMut(u32, &[f32])) -> DecodeStatus {
        #[cfg(panic = "unwind")]
        {
            let run = std::panic::AssertUnwindSafe(|| self.work_inner(max_packets, sink));
            match std::panic::catch_unwind(run) {
                Ok(status) => status,
                Err(_) => {
                    let msg = STREAM_DECODER_PANICKED.to_string();
                    self.state = State::Failed(msg.clone());
                    DecodeStatus::Failed(msg)
                }
            }
        }
        #[cfg(not(panic = "unwind"))]
        {
            self.work_inner(max_packets, sink)
        }
    }

    fn work_inner(
        &mut self,
        max_packets: usize,
        sink: &mut dyn FnMut(u32, &[f32]),
    ) -> DecodeStatus {
        // Once probed, fall through to decoding.
        if let State::Probing { min_bytes } = self.state
            && let Some(status) = self.probe(min_bytes)
        {
            return status;
        }

        let mut decoded = 0;
        loop {
            let State::Decoding {
                reader,
                decoder,
                track_id,
            } = &mut self.state
            else {
                return match &self.state {
                    State::Done => DecodeStatus::Finished,
                    State::Failed(e) => DecodeStatus::Failed(e.clone()),
                    _ => unreachable!(),
                };
            };
            if decoded >= max_packets {
                return DecodeStatus::Progress;
            }
            if !self.input.eof() && self.input.unread() < DECODE_MARGIN {
                return if decoded > 0 {
                    DecodeStatus::Progress
                } else {
                    DecodeStatus::NeedData
                };
            }

            let packet = match reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => {
                    self.state = State::Done;
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Only reachable once the network stream has ended.
                    self.state = State::Done;
                    continue;
                }
                Err(SymError::ResetRequired) => {
                    // Chained stream with new parameters: rebuild the decoder.
                    match make_decoder(reader.as_ref()) {
                        Ok((d, id)) => {
                            *decoder = d;
                            *track_id = id;
                            continue;
                        }
                        Err(e) => {
                            self.state = State::Failed(e);
                            continue;
                        }
                    }
                }
                Err(e) => {
                    self.state = State::Failed(format!("demux: {e}"));
                    continue;
                }
            };
            if packet.track_id != *track_id {
                continue;
            }
            decoded += 1;

            match decoder.decode(&packet) {
                Ok(buf) => {
                    let rate = buf.spec().rate();
                    let channels = buf.spec().channels().count().max(1);
                    self.tmp.clear();
                    buf.copy_to_vec_interleaved(&mut self.tmp);
                    to_stereo(&self.tmp, channels, &mut self.stereo);
                    sink(rate, &self.stereo);
                }
                // A corrupt packet: skip it, as players do, rather than stopping playback.
                Err(SymError::DecodeError(_)) => {}
                Err(e) => self.state = State::Failed(format!("decode: {e}")),
            }
        }
    }

    /// Returns `None` once probed successfully, otherwise the status to report.
    fn probe(&mut self, min_bytes: usize) -> Option<DecodeStatus> {
        let eof = self.input.eof();
        if !eof && self.input.unread() < min_bytes {
            return Some(DecodeStatus::NeedData);
        }
        if let Err(e) = self.input.with_buffered(check_wav_headers) {
            let msg = format!("probe: {e}");
            self.state = State::Failed(msg.clone());
            return Some(DecodeStatus::Failed(msg));
        }
        let mss = MediaSourceStream::new(
            Box::new(ReadOnlySource::new(self.input.clone())),
            MediaSourceStreamOptions::default(),
        );
        let fmt_opts = FormatOptions::default();
        match symphonia::default::get_probe().probe(
            &self.hint,
            mss,
            fmt_opts,
            MetadataOptions::default(),
        ) {
            Ok(mut reader) => match make_decoder(reader.as_ref()) {
                Ok((decoder, track_id)) => {
                    let params = reader
                        .default_track(TrackType::Audio)
                        .and_then(|t| t.codec_params.clone());
                    if let Some(CodecParameters::Audio(params)) = params {
                        self.source = Some(source_format(&params, reader.as_mut()));
                    }
                    self.input.release();
                    self.state = State::Decoding {
                        reader,
                        decoder,
                        track_id,
                    };
                    None
                }
                Err(e) => {
                    self.state = State::Failed(e.clone());
                    Some(DecodeStatus::Failed(e))
                }
            },
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof && !eof => {
                // Headers not all here yet (e.g. large embedded artwork): retry with more data,
                // up to a hard cap so an unparseable stream can't hold the track for ever.
                self.input.rewind();
                let have = self.input.unread();
                if have >= PROBE_MAX_BYTES {
                    let msg = format!("probe: no playable headers in the first {have} bytes");
                    self.state = State::Failed(msg.clone());
                    return Some(DecodeStatus::Failed(msg));
                }
                self.state = State::Probing {
                    min_bytes: (have * 2).max(min_bytes).min(PROBE_MAX_BYTES),
                };
                Some(DecodeStatus::NeedData)
            }
            Err(e) => {
                let msg = format!("probe: {e}");
                self.state = State::Failed(msg.clone());
                Some(DecodeStatus::Failed(msg))
            }
        }
    }
}

fn make_decoder(reader: &dyn FormatReader) -> Result<(Box<dyn AudioDecoder>, u32), String> {
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err("audio track has no codec parameters".into());
    };
    #[cfg(feature = "opus")]
    let codecs = crate::opus::codecs();
    #[cfg(not(feature = "opus"))]
    let codecs = symphonia::default::get_codecs();
    let decoder = codecs
        .make_audio_decoder(params, &AudioDecoderOptions::default().gapless(true))
        .map_err(|e| format!("codec: {e}"))?;
    Ok((decoder, track.id))
}

/// A complete, seekable input for [`decode_seekable`] (a file the host saved, or bytes in
/// memory). Re-exported so hosts need not depend on Symphonia.
pub use symphonia::core::io::MediaSource;

/// Consecutive undecodable packets after which a stream counts as corrupt.
pub const MAX_BAD_PACKETS: u32 = 64;

/// Decode a whole, seekable source into interleaved stereo `f32` PCM, passing each block to
/// `sink(sample_rate, samples)`; the sink returns `false` to stop early.
///
/// Unlike [`StreamDecoder`], the demuxer may seek, so MP4 files with their index at the end
/// decode too. It is for hosts that already hold the whole file (e.g. the gateway's track
/// analysis). The host does the I/O; this only reads through `source`. A corrupt packet is
/// skipped, as players do, but [`MAX_BAD_PACKETS`] in a row fail the stream.
///
/// A panic inside the third-party demuxers and decoders (hostile headers have been seen to
/// trip arithmetic overflow checks in debug builds) is caught and reported as an error, so
/// one bad file can't take its host down.
pub fn decode_seekable(
    mut source: Box<dyn MediaSource>,
    ext_hint: Option<&str>,
    sink: &mut dyn FnMut(u32, &[f32]) -> bool,
) -> Result<Option<SourceFormat>, String> {
    check_file_headers(&mut source)?;
    let run = std::panic::AssertUnwindSafe(|| decode_seekable_inner(source, ext_hint, sink));
    std::panic::catch_unwind(run).unwrap_or_else(|_| Err(FILE_DECODER_PANICKED.into()))
}

fn decode_seekable_inner(
    source: Box<dyn MediaSource>,
    ext_hint: Option<&str>,
    sink: &mut dyn FnMut(u32, &[f32]) -> bool,
) -> Result<Option<SourceFormat>, String> {
    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }
    let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    let (mut decoder, mut track_id) = make_decoder(reader.as_ref())?;
    let format = match reader
        .default_track(TrackType::Audio)
        .and_then(|t| t.codec_params.clone())
    {
        Some(CodecParameters::Audio(params)) => Some(source_format(&params, reader.as_mut())),
        _ => None,
    };
    let (mut tmp, mut stereo) = (Vec::new(), Vec::new());
    let mut bad = 0u32;
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(format),
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(format);
            }
            Err(SymError::ResetRequired) => {
                (decoder, track_id) = make_decoder(reader.as_ref())?;
                continue;
            }
            Err(e) => return Err(format!("demux: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                bad = 0;
                let rate = buf.spec().rate();
                let channels = buf.spec().channels().count().max(1);
                tmp.clear();
                buf.copy_to_vec_interleaved(&mut tmp);
                to_stereo(&tmp, channels, &mut stereo);
                if !sink(rate, &stereo) {
                    return Ok(format);
                }
            }
            Err(SymError::DecodeError(_)) => {
                bad += 1;
                if bad >= MAX_BAD_PACKETS {
                    return Err("decode: too many corrupt packets".into());
                }
            }
            Err(e) => return Err(format!("decode: {e}")),
        }
    }
}

/// What decoding a whole FLAC file found ([`verify_flac`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlacCheck {
    /// The MD5 of the decoded audio equals the one in its STREAMINFO (false when that is
    /// unset, all zeros).
    pub md5_ok: bool,
    /// Sample frames decoded.
    pub frames: u64,
}

/// Decode a whole FLAC file and check its audio against the MD5 in its STREAMINFO: proof
/// that every audio frame is intact. Strict: anything but FLAC, and any frame that fails to
/// decode (a bad CRC, a truncated frame), is an error. The host does the I/O (it supplies
/// `source`); this only reads through it. A panic in the third-party demuxer or decoder is
/// caught and reported as an error, as for [`decode_seekable`].
pub fn verify_flac(mut source: Box<dyn MediaSource>) -> Result<FlacCheck, String> {
    check_file_headers(&mut source)?;
    catch(move || verify_flac_inner(source)).unwrap_or_else(|| Err(FILE_DECODER_PANICKED.into()))
}

fn verify_flac_inner(source: Box<dyn MediaSource>) -> Result<FlacCheck, String> {
    let mut hint = Hint::new();
    hint.with_extension("flac");
    let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err("no codec parameters".into());
    };
    if params.codec != ids::CODEC_ID_FLAC {
        return Err("not FLAC".into());
    }
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default().verify(true))
        .map_err(|e| format!("codec: {e}"))?;
    let mut frames = 0u64;
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("demux: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = decoder
            .decode(&packet)
            .map_err(|e| format!("decode: {e}"))?;
        frames += buf.frames() as u64;
    }
    Ok(FlacCheck {
        md5_ok: decoder.finalize().verify_ok == Some(true),
        frames,
    })
}

/// FLAC's MD5 of interleaved integer samples `bits` deep (each sample little-endian in
/// whole bytes, as STREAMINFO's MD5 is defined), e.g. for writing test files.
pub fn flac_pcm_md5(samples: &[i32], bits: u32) -> [u8; 16] {
    use symphonia::core::checksum::Md5;
    use symphonia::core::io::Monitor;
    let width = bits.div_ceil(8).clamp(1, 4) as usize;
    let mut md5 = Md5::default();
    let mut bytes = Vec::with_capacity(samples.len() * width);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes()[..width]);
    }
    md5.process_buf_bytes(&bytes);
    md5.md5()
}

/// Bytes at the start of a file checked by [`check_wav_headers`] before a [`FileDecoder`]
/// lets Symphonia probe it.
const FILE_HEADER_CHECK: usize = 1024 * 1024;

/// Refuse a whole file whose WAV headers would trip the third-party reader's overflow
/// checks ([`check_wav_headers`]) before Symphonia's prober, which scans what it can see,
/// parses them; then rewind. Every whole-file entry point does this, so the caught panic is
/// never what saves us.
fn check_file_headers(source: &mut Box<dyn MediaSource>) -> Result<(), String> {
    let mut head = Vec::new();
    source
        .take(FILE_HEADER_CHECK as u64)
        .read_to_end(&mut head)
        .map_err(|e| format!("read: {e}"))?;
    check_wav_headers(&head).map_err(|e| format!("probe: {e}"))?;
    source
        .seek(io::SeekFrom::Start(0))
        .map_err(|e| format!("read: {e}"))?;
    Ok(())
}

/// Decodes a complete local file (a downloaded track) piece by piece, with seeking.
///
/// Unlike [`StreamDecoder`] the source is seekable and wholly present, so the demuxer may
/// seek (MP4 with its index at the end works) and the host can ask for a position. Like
/// [`decode_seekable`], a corrupt packet is skipped and [`MAX_BAD_PACKETS`] in a row fail
/// the file. The host does the I/O (it supplies `source`); this only reads through it.
///
/// File bytes are untrusted: the WAV headers known to trip Symphonia's arithmetic are
/// refused before probing, and where panics unwind any other panic in the demuxer or decoder
/// fails the file instead of the host.
pub struct FileDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    source: Option<SourceFormat>,
    /// The track's sample rate, from its parameters or the first decoded block.
    rate: Option<u32>,
    /// Frames in the track, if the container says.
    frames: Option<u64>,
    /// Frames still to drop after an accurate seek (the demuxer lands on a packet start).
    skip: u64,
    tmp: Vec<f32>,
    stereo: Vec<f32>,
    bad: u32,
    state: FileState,
}

#[derive(Debug, Clone, PartialEq)]
enum FileState {
    Decoding,
    Done,
    Failed(String),
}

impl FileDecoder {
    /// Probe `source` (`ext_hint`, e.g. "flac", helps the prober).
    pub fn open(mut source: Box<dyn MediaSource>, ext_hint: Option<&str>) -> Result<Self, String> {
        check_file_headers(&mut source)?;
        let hint = ext_hint.map(str::to_string);
        catch(move || Self::open_inner(source, hint.as_deref()))
            .unwrap_or_else(|| Err(FILE_DECODER_PANICKED.into()))
    }

    fn open_inner(source: Box<dyn MediaSource>, ext_hint: Option<&str>) -> Result<Self, String> {
        let mut hint = Hint::new();
        if let Some(ext) = ext_hint {
            hint.with_extension(ext);
        }
        let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
        let mut reader = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("probe: {e}"))?;
        let (decoder, track_id) = make_decoder(reader.as_ref())?;
        let track = reader.default_track(TrackType::Audio);
        let frames = track.and_then(|t| t.num_frames);
        let params = track.and_then(|t| t.codec_params.clone());
        let (source, rate) = match params {
            Some(CodecParameters::Audio(params)) => (
                Some(source_format(&params, reader.as_mut())),
                params.sample_rate.filter(|r| (1..=1_536_000).contains(r)),
            ),
            _ => (None, None),
        };
        Ok(FileDecoder {
            reader,
            decoder,
            track_id,
            source,
            rate,
            frames,
            skip: 0,
            tmp: Vec::new(),
            stereo: Vec::new(),
            bad: 0,
            state: FileState::Decoding,
        })
    }

    pub fn source_format(&self) -> Option<&SourceFormat> {
        self.source.as_ref()
    }

    /// The sample rate, once known (from the headers, or after the first block).
    pub fn sample_rate(&self) -> Option<u32> {
        self.rate
    }

    /// Frames in the track, if the container says.
    pub fn total_frames(&self) -> Option<u64> {
        self.frames
    }

    pub fn is_finished(&self) -> bool {
        self.state == FileState::Done
    }

    /// Decode up to `max_packets`, passing each block of interleaved stereo PCM to
    /// `sink(sample_rate, samples)`. Never blocks on anything but reading the file.
    pub fn work(&mut self, max_packets: usize, sink: &mut dyn FnMut(u32, &[f32])) -> DecodeStatus {
        match &self.state {
            FileState::Done => return DecodeStatus::Finished,
            FileState::Failed(e) => return DecodeStatus::Failed(e.clone()),
            FileState::Decoding => {}
        }
        match catch(|| self.work_inner(max_packets, sink)) {
            Some(s) => s,
            None => {
                let msg = "decode: the decoder failed on this file".to_string();
                self.state = FileState::Failed(msg.clone());
                DecodeStatus::Failed(msg)
            }
        }
    }

    fn work_inner(
        &mut self,
        max_packets: usize,
        sink: &mut dyn FnMut(u32, &[f32]),
    ) -> DecodeStatus {
        let mut decoded = 0;
        while decoded < max_packets {
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => {
                    self.state = FileState::Done;
                    return DecodeStatus::Finished;
                }
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    self.state = FileState::Done;
                    return DecodeStatus::Finished;
                }
                Err(SymError::ResetRequired) => {
                    match make_decoder(self.reader.as_ref()) {
                        Ok((d, id)) => {
                            self.decoder = d;
                            self.track_id = id;
                        }
                        Err(e) => return self.fail(e),
                    }
                    continue;
                }
                Err(e) => return self.fail(format!("demux: {e}")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            decoded += 1;
            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    self.bad = 0;
                    let rate = buf.spec().rate();
                    let channels = buf.spec().channels().count().max(1);
                    self.tmp.clear();
                    buf.copy_to_vec_interleaved(&mut self.tmp);
                    to_stereo(&self.tmp, channels, &mut self.stereo);
                    if rate == 0 {
                        return self.fail("decode: no sample rate".into());
                    }
                    self.rate = Some(rate);
                    let frames = (self.stereo.len() / 2) as u64;
                    let drop = self.skip.min(frames);
                    self.skip -= drop;
                    let rest = &self.stereo[drop as usize * 2..];
                    if !rest.is_empty() {
                        sink(rate, rest);
                    }
                }
                Err(SymError::DecodeError(_)) => {
                    self.bad += 1;
                    if self.bad >= MAX_BAD_PACKETS {
                        return self.fail("decode: too many corrupt packets".into());
                    }
                }
                Err(e) => return self.fail(format!("decode: {e}")),
            }
        }
        DecodeStatus::Progress
    }

    fn fail(&mut self, msg: String) -> DecodeStatus {
        self.state = FileState::Failed(msg.clone());
        DecodeStatus::Failed(msg)
    }

    /// Go to `seconds` from the start (clamped to the track). Returns the position decoding
    /// resumes from, in frames: sample-accurate where the format allows.
    pub fn seek(&mut self, seconds: f64) -> Result<u64, String> {
        if let FileState::Failed(e) = &self.state {
            return Err(e.clone());
        }
        let rate = self.rate.ok_or("seek: unknown sample rate")?;
        let mut secs = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        if let Some(n) = self.frames {
            secs = secs.min(n as f64 / f64::from(rate));
        }
        match catch(|| self.seek_inner(secs, rate)) {
            Some(Ok(frame)) => Ok(frame),
            Some(Err(e)) => Err(e),
            None => {
                let msg = "seek: the decoder failed on this file".to_string();
                self.state = FileState::Failed(msg.clone());
                Err(msg)
            }
        }
    }

    fn seek_inner(&mut self, secs: f64, rate: u32) -> Result<u64, String> {
        let time = symphonia::core::units::Time::try_from_secs_f64(secs).ok_or("seek: bad time")?;
        let seeked = self
            .reader
            .seek(
                symphonia::core::formats::SeekMode::Accurate,
                symphonia::core::formats::SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| format!("seek: {e}"))?;
        self.decoder.reset();
        self.state = FileState::Decoding;
        self.bad = 0;
        // Timestamps are in the track's time base; for audio that is almost always one
        // tick per frame. Convert through seconds otherwise.
        let base = self
            .reader
            .tracks()
            .iter()
            .find(|t| t.id == self.track_id)
            .and_then(|t| t.time_base);
        let to_frames = |ts: symphonia::core::units::Timestamp| -> u64 {
            let ticks = ts.get().max(0) as u64;
            match base {
                Some(b) => b
                    .calc_time(symphonia::core::units::Timestamp::new(ticks as i64))
                    .map_or(ticks, |t| {
                        (t.as_secs_f64() * f64::from(rate)).round() as u64
                    }),
                None => ticks,
            }
        };
        let required = to_frames(seeked.required_ts);
        let actual = to_frames(seeked.actual_ts).min(required);
        self.skip = required - actual;
        Ok(required)
    }
}

/// Run `f`, catching a panic where panics unwind (`None` if it panicked).
fn catch<T>(f: impl FnOnce() -> T) -> Option<T> {
    #[cfg(panic = "unwind")]
    {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
    }
    #[cfg(not(panic = "unwind"))]
    {
        Some(f())
    }
}

/// Most channels Symphonia's WAV reader maps; it refuses more, but only after arithmetic
/// that overflows on absurd counts.
const WAV_MAX_CHANNELS: u16 = 26;
/// Speaker positions defined for a WAVE_FORMAT_EXTENSIBLE channel mask (bits 0..=17).
const WAV_MASK_BITS: u32 = 18;

/// Refuse RIFF/WAVE headers that would make Symphonia's WAV reader panic.
///
/// Symphonia 0.6's WAV reader does unchecked arithmetic on header fields: a channel count
/// times the sample width (`u16`), a shift by the channel mask's bit length, and the RIFF
/// length minus 4. Hostile values overflow, which panics in builds with overflow checks.
/// Its prober scans for the `RIFF…WAVE` marker anywhere in the stream, so every occurrence
/// in `bytes` (what the prober can see) is checked, walking the chunks the way the reader
/// does until the `data` chunk. A header that is cut short is left to the reader, which
/// fails cleanly on it (or waits for more data, when this is checked again).
pub fn check_wav_headers(bytes: &[u8]) -> Result<(), &'static str> {
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut from = 0;
    while let Some(off) = bytes
        .get(from..)
        .and_then(|b| b.windows(4).position(|w| w == b"RIFF"))
    {
        let start = from + off;
        from = start + 1;
        if bytes.get(start + 8..start + 12) != Some(b"WAVE".as_slice()) {
            continue;
        }
        let riff_len = u32_at(start + 4);
        if riff_len < 4 {
            return Err("wav: RIFF length too small");
        }
        let len = (riff_len < u32::MAX).then(|| u64::from(riff_len) - 4);
        let base = start + 12;
        let mut consumed = 0u64;
        loop {
            if consumed & 1 == 1 {
                consumed += 1;
            }
            if len.is_some_and(|l| consumed + 8 > l) {
                break;
            }
            let Some(hdr) = usize::try_from(consumed)
                .ok()
                .and_then(|c| base.checked_add(c))
            else {
                break;
            };
            if hdr.checked_add(8).is_none_or(|end| end > bytes.len()) {
                break; // not here yet
            }
            let tag = &bytes[hdr..hdr + 4];
            let chunk_len = u32_at(hdr + 4);
            consumed += 8;
            if len.is_some_and(|l| l - consumed < u64::from(chunk_len)) {
                break; // the reader refuses this cleanly
            }
            let body = hdr + 8;
            consumed += u64::from(chunk_len);
            match tag {
                b"fmt " => {
                    if chunk_len >= 16 && body + 16 <= bytes.len() {
                        let channels = u16_at(body + 2);
                        if channels == 0 || channels > WAV_MAX_CHANNELS {
                            return Err("wav: unsupported channel count");
                        }
                        let extensible = u16_at(body) == 0xfffe;
                        if extensible && chunk_len >= 40 && body + 24 <= bytes.len() {
                            let mask = u32_at(body + 20);
                            if mask >> WAV_MASK_BITS != 0 {
                                return Err("wav: invalid channel mask");
                            }
                        }
                    }
                }
                b"data" => break,
                _ => {}
            }
        }
    }
    Ok(())
}

/// Map interleaved samples with `channels` channels onto interleaved stereo.
fn to_stereo(src: &[f32], channels: usize, dst: &mut Vec<f32>) {
    dst.clear();
    match channels {
        2 => dst.extend_from_slice(src),
        1 => dst.extend(src.iter().flat_map(|s| [*s, *s])),
        n => dst.extend(src.chunks_exact(n).flat_map(|f| [f[0], f[1]])),
    }
}

/// Slimproto `strm` format code → file extension hint.
pub fn format_extension(format: u8) -> Option<&'static str> {
    Some(match format {
        b'f' => "flac",
        b'm' => "mp3",
        b'o' => "ogg",
        b'a' => "aac",
        b'l' => "m4a",
        b'p' => "wav",
        // Ogg Opus; only accepted when this build can decode it.
        #[cfg(feature = "opus")]
        b'u' => "opus",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 16-bit PCM WAV file.
    fn wav(rate: u32, channels: u16, frames: usize) -> Vec<u8> {
        let data_len = frames * channels as usize * 2;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        v.extend_from_slice(&(channels * 2).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..frames * channels as usize {
            v.extend_from_slice(&(((i % 200) as i16 - 100) * 100).to_le_bytes());
        }
        v
    }

    fn decode_all(dec: &mut StreamDecoder) -> (u32, usize, DecodeStatus) {
        let (mut rate, mut samples) = (0, 0);
        loop {
            let st = dec.work(64, &mut |r, s| {
                rate = r;
                samples += s.len();
            });
            if st != DecodeStatus::Progress {
                return (rate, samples, st);
            }
        }
    }

    #[test]
    fn waits_for_margin_then_decodes_everything_after_eof() {
        let file = wav(44100, 2, 44100 * 3); // 3 s ≈ 529 KB
        let input = InputBuffer::new();
        let mut dec = StreamDecoder::new(input.clone(), b'p');

        input.push(&file[..1000]);
        assert_eq!(dec.work(64, &mut |_, _| {}), DecodeStatus::NeedData);

        // Feed in uneven network-sized chunks.
        for chunk in file[1000..].chunks(7919) {
            input.push(chunk);
            dec.work(8, &mut |_, _| {});
        }
        input.set_eof();
        let (rate, _, st) = decode_all(&mut dec);
        assert_eq!(st, DecodeStatus::Finished);
        assert_eq!(rate, 44100);
        assert!(dec.is_finished());
    }

    #[test]
    fn mono_becomes_stereo_and_no_frames_are_lost() {
        let frames = 8000 * 2;
        let file = wav(8000, 1, frames);
        let input = InputBuffer::new();
        input.push(&file);
        input.set_eof();
        let mut dec = StreamDecoder::new(input, b'p');
        let mut total = 0;
        loop {
            let st = dec.work(64, &mut |_, s| total += s.len());
            if st == DecodeStatus::Finished {
                break;
            }
            assert_eq!(st, DecodeStatus::Progress);
        }
        assert_eq!(
            total,
            frames * 2,
            "every mono frame should become one stereo frame"
        );
    }

    #[test]
    fn garbage_fails_cleanly() {
        let input = InputBuffer::new();
        input.push(&[0x42; 4096]);
        input.set_eof();
        let mut dec = StreamDecoder::new(input, b'f');
        assert!(matches!(
            dec.work(8, &mut |_, _| {}),
            DecodeStatus::Failed(_)
        ));
    }

    #[test]
    fn replaygain_tags_parse_strictly() {
        assert_eq!(parse_gain_db("-6.48 dB"), Some(-6.48));
        assert_eq!(parse_gain_db("+1.20dB"), Some(1.2));
        assert_eq!(parse_gain_db(" 3 db "), Some(3.0));
        assert_eq!(parse_gain_db("0.5"), Some(0.5));
        assert_eq!(parse_peak("0.988525"), Some(0.988525));
        assert_eq!(parse_peak("1"), Some(1.0));
        for bad in [
            "",
            "dB",
            "-",
            ".",
            "nan",
            "NaN dB",
            "inf",
            "-inf dB",
            "1e3",
            "1e-3",
            "0x10",
            "--1",
            "+-1",
            "1.2.3",
            "61 dB",
            "-60.01 dB",
            "1,5 dB",
            "1 dB extra",
            "١٢",
        ] {
            assert_eq!(parse_gain_db(bad), None, "{bad:?}");
        }
        for bad in ["", "-0.1", "17", "nan", "inf", "1e0", "0.5 dB"] {
            assert_eq!(parse_peak(bad), None, "{bad:?}");
        }
        let long = format!("{}1", "0".repeat(MAX_TAG_VALUE));
        assert_eq!(parse_gain_db(&long), None);
        assert_eq!(parse_peak(&long), None);
    }

    #[test]
    fn arbitrary_tag_text_never_panics_or_escapes_bounds() {
        let alphabet: Vec<char> = "0123456789.+-dBe inaxé\u{0}".chars().collect();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..50_000 {
            let len = (seed % 24) as usize;
            let mut s = String::new();
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            if let Some(g) = parse_gain_db(&s) {
                assert!(g.is_finite() && g.abs() <= 60.0, "{s:?} -> {g}");
            }
            if let Some(p) = parse_peak(&s) {
                assert!((0.0..=16.0).contains(&p), "{s:?} -> {p}");
            }
        }
    }

    #[test]
    fn wav_source_format_is_reported_after_probing() {
        let file = wav(48000, 1, 48000);
        let input = InputBuffer::new();
        input.push(&file);
        input.set_eof();
        let mut dec = StreamDecoder::new(input, b'p');
        assert!(dec.source_format().is_none());
        dec.work(1, &mut |_, _| {});
        let f = dec.source_format().copied().unwrap();
        assert_eq!(f.codec, "PCM");
        assert!(f.lossless);
        assert_eq!(f.bits, Some(16));
        assert_eq!(f.channels, 1);
        assert_eq!(f.replay_gain, ReplayGainTags::default());
    }

    #[test]
    fn reader_crosses_deque_wrap() {
        let mut b = InputBuffer::new();
        b.release(); // streaming mode
        b.push(&[1, 2, 3, 4]);
        let mut out = [0u8; 3];
        assert_eq!(b.read(&mut out).unwrap(), 3);
        b.push(&[5, 6, 7]);
        let mut out = [0u8; 8];
        assert_eq!(b.read(&mut out).unwrap(), 4);
        assert_eq!(&out[..4], &[4, 5, 6, 7]);
        assert_eq!(b.read(&mut out).unwrap(), 0);
    }

    /// A 16-bit stereo WAV whose left sample at frame `i` is `i` (wrapping) and right `-i`.
    fn counting_wav(rate: u32, frames: usize) -> Vec<u8> {
        let mut v = wav(rate, 2, 0);
        let data_len = frames * 4;
        v[4..8].copy_from_slice(&((36 + data_len) as u32).to_le_bytes());
        v[40..44].copy_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..frames {
            let x = (i % 30_000) as i16;
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&(-x).to_le_bytes());
        }
        v
    }

    fn open_bytes(b: Vec<u8>, ext: Option<&str>) -> Result<FileDecoder, String> {
        FileDecoder::open(Box::new(std::io::Cursor::new(b)), ext)
    }

    fn file_all(dec: &mut FileDecoder) -> (Vec<f32>, DecodeStatus) {
        let mut out = Vec::new();
        loop {
            let st = dec.work(16, &mut |_, s| out.extend_from_slice(s));
            if st != DecodeStatus::Progress {
                return (out, st);
            }
        }
    }

    #[test]
    fn file_decoder_decodes_every_frame_exactly() {
        let mut dec = open_bytes(counting_wav(44_100, 50_000), Some("wav")).unwrap();
        assert_eq!(dec.sample_rate(), Some(44_100));
        assert_eq!(dec.total_frames(), Some(50_000));
        assert_eq!(dec.source_format().unwrap().bits, Some(16));
        let (pcm, st) = file_all(&mut dec);
        assert_eq!(st, DecodeStatus::Finished);
        assert!(dec.is_finished());
        assert_eq!(pcm.len(), 50_000 * 2);
        for (i, f) in pcm.as_chunks::<2>().0.iter().enumerate() {
            let x = (i % 30_000) as f32 / 32768.0;
            assert_eq!((f[0], f[1]), (x, -x), "frame {i}");
        }
        // Finished stays finished.
        assert_eq!(dec.work(4, &mut |_, _| {}), DecodeStatus::Finished);
    }

    #[test]
    fn file_decoder_seeks_sample_accurately() {
        let mut dec = open_bytes(counting_wav(48_000, 96_000), None).unwrap();
        // Decode a little, then jump forward, back, and past the end.
        dec.work(2, &mut |_, _| {});
        for (secs, frame) in [(1.25, 60_000u64), (0.5, 24_000), (0.0, 0)] {
            assert_eq!(dec.seek(secs).unwrap(), frame);
            let mut first = None;
            dec.work(1, &mut |_, s| {
                if first.is_none() {
                    first = Some(s[0]);
                }
            });
            let want = (frame % 30_000) as f32 / 32768.0;
            assert_eq!(first, Some(want), "seek to {secs}");
        }
        assert_eq!(dec.seek(99.0).unwrap(), 96_000);
        let (rest, st) = file_all(&mut dec);
        assert!(rest.is_empty());
        assert_eq!(st, DecodeStatus::Finished);
        // Seeking after the end starts decoding again.
        dec.seek(1.9).unwrap();
        let (rest, _) = file_all(&mut dec);
        assert_eq!(rest.len(), (96_000 - 91_200) * 2);
        // Nonsense positions are clamped.
        assert_eq!(dec.seek(f64::NAN).unwrap(), 0);
        assert_eq!(dec.seek(-3.0).unwrap(), 0);
    }

    #[test]
    fn file_decoder_refuses_hostile_and_broken_files() {
        // The WAV header that overflows Symphonia's arithmetic.
        let mut hostile = wav(44_100, 2, 100);
        hostile[22..24].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(open_bytes(hostile, Some("wav")).is_err());
        assert!(open_bytes(Vec::new(), None).is_err());
        assert!(open_bytes(b"not audio at all".to_vec(), Some("flac")).is_err());
        // Truncated mid-data: decodes what is there, then finishes cleanly.
        let mut cut = counting_wav(44_100, 10_000);
        cut.truncate(44 + 4 * 5_000 + 3);
        if let Ok(mut dec) = open_bytes(cut, Some("wav")) {
            let (pcm, st) = file_all(&mut dec);
            assert!(pcm.len() <= 10_000 * 2);
            assert!(matches!(
                st,
                DecodeStatus::Finished | DecodeStatus::Failed(_)
            ));
        }
    }

    #[test]
    fn file_decoder_never_panics_on_arbitrary_bytes() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let base = counting_wav(8_000, 2_000);
        for round in 0..300 {
            let mut b = if round % 3 == 0 {
                (0..(next() % 4096) as usize)
                    .map(|_| next() as u8)
                    .collect()
            } else {
                base.clone()
            };
            if !b.is_empty() {
                for _ in 0..(next() % 16) {
                    let i = (next() as usize) % b.len();
                    b[i] = next() as u8;
                }
                if round % 2 == 0 {
                    let n = (next() as usize) % b.len();
                    b.truncate(n);
                }
            }
            let ext = [None, Some("wav"), Some("flac"), Some("mp3"), Some("ogg")][round % 5];
            if let Ok(mut dec) = open_bytes(b, ext) {
                let _ = dec.seek((next() % 3) as f64);
                let _ = file_all(&mut dec);
            }
        }
    }
}
