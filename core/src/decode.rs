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
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

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

/// Decodes one track's byte stream into interleaved stereo `f32` PCM.
pub struct StreamDecoder {
    input: InputBuffer,
    hint: Hint,
    state: State,
    tmp: Vec<f32>,
    stereo: Vec<f32>,
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
        }
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
            Ok(reader) => match make_decoder(reader.as_ref()) {
                Ok((decoder, track_id)) => {
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
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default().gapless(true))
        .map_err(|e| format!("codec: {e}"))?;
    Ok((decoder, track.id))
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
