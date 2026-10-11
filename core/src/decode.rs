//! Incremental decoding of a network audio stream with no blocking I/O.
//!
//! Symphonia pulls bytes through `std::io::Read` and treats a zero-length read as end of
//! stream, so it must never be asked to read data that hasn't arrived yet. To keep this
//! usable from WebAssembly (no threads, no blocking), [`StreamDecoder`] only demuxes while
//! at least [`DECODE_MARGIN`] unread bytes are buffered, or once the stream has ended.
//! Probing is retried from the start of the stream if the headers (which can carry several
//! MiB of embedded artwork) haven't fully arrived yet.

use std::collections::VecDeque;
use std::io::{self, Read};
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
    pub fn work(&mut self, max_packets: usize, sink: &mut dyn FnMut(u32, &[f32])) -> DecodeStatus {
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
    source: Box<dyn MediaSource>,
    ext_hint: Option<&str>,
    sink: &mut dyn FnMut(u32, &[f32]) -> bool,
) -> Result<Option<SourceFormat>, String> {
    let run = std::panic::AssertUnwindSafe(|| decode_seekable_inner(source, ext_hint, sink));
    std::panic::catch_unwind(run).unwrap_or_else(|_| Err("decoder failed on this file".into()))
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
}
