//! Opus decoding (behind the `opus` cargo feature).
//!
//! Symphonia 0.6 demuxes Ogg Opus (RFC 7845) but has no Opus decoder. This module plugs
//! one in: the pure-Rust, `unsafe`-free `opus-decoder` crate (MIT/Apache-2.0, RFC 6716 and
//! RFC 8251 conformant per its test vectors), wrapped as a Symphonia [`AudioDecoder`] and
//! registered alongside the default codecs (see [`codecs`]).
//!
//! Why this option:
//! - libopus through the `opus`/`audiopus` crates needs a C toolchain per target and can't
//!   build for `wasm32-unknown-unknown`;
//! - a pure-Rust decoder keeps every host's build simple (Android NDK, iOS, desktop).
//!
//! The feature is still off for the WebAssembly build: browsers decode Opus natively, and
//! the web player doesn't need the extra code.
//!
//! Scope: channel mapping family 0, and family 1 with one or two channels (Ogg Opus as
//! LMS and the download transcoder produce it). Output is always 48 kHz, as RFC 7845
//! recommends. The header's output gain is applied, the pre-skip is dropped, and the end
//! trim Symphonia derives from the final granule position is honoured, so decoding is
//! sample-accurate.

use std::sync::OnceLock;

use opus_decoder::OpusMultistreamDecoder;
use symphonia::core::audio::{
    AsGenericAudioBufferRef, AudioBuffer, AudioMut, AudioSpec, Channels, GenericAudioBufferRef,
    Position,
};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{
    AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::codecs::registry::{
    CodecRegistry, RegisterableAudioDecoder, SupportedAudioCodec,
};
use symphonia::core::errors::{Result, decode_error, unsupported_error};
use symphonia::core::packet::PacketRef;

/// Opus always decodes at 48 kHz.
pub const OPUS_RATE: u32 = 48_000;
/// Longest Opus packet: 120 ms at 48 kHz.
const MAX_FRAMES: usize = 5760;

/// The parsed `OpusHead` identification header (RFC 7845 §5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct OpusHead {
    pub channels: u8,
    pub pre_skip: u16,
    /// Output gain, dB (Q7.8 in the header).
    pub output_gain_db: f64,
    pub mapping_family: u8,
    pub streams: u8,
    pub coupled: u8,
    pub mapping: Vec<u8>,
}

impl OpusHead {
    /// Parse and check a header; refuses anything this decoder doesn't support.
    pub fn parse(b: &[u8]) -> std::result::Result<Self, &'static str> {
        if b.len() < 19 || &b[..8] != b"OpusHead" {
            return Err("not an OpusHead");
        }
        if b[8] >> 4 != 0 {
            return Err("unsupported OpusHead version");
        }
        let channels = b[9];
        let pre_skip = u16::from_le_bytes([b[10], b[11]]);
        let gain = i16::from_le_bytes([b[16], b[17]]);
        let family = b[18];
        let (streams, coupled, mapping) = match family {
            0 => match channels {
                1 => (1, 0, vec![0]),
                2 => (1, 1, vec![0, 1]),
                _ => return Err("family 0 allows one or two channels"),
            },
            1 => {
                if b.len() < 21 + channels as usize {
                    return Err("truncated channel mapping");
                }
                (b[19], b[20], b[21..21 + channels as usize].to_vec())
            }
            _ => return Err("unsupported channel mapping family"),
        };
        if channels == 0 || channels > 2 {
            return Err("only mono and stereo Opus are supported");
        }
        if streams == 0 || coupled > streams {
            return Err("bad stream counts");
        }
        let decoded = 2 * usize::from(coupled) + usize::from(streams - coupled);
        if mapping
            .iter()
            .any(|m| *m != 255 && usize::from(*m) >= decoded)
        {
            return Err("bad channel mapping");
        }
        Ok(OpusHead {
            channels,
            pre_skip,
            output_gain_db: f64::from(gain) / 256.0,
            mapping_family: family,
            streams,
            coupled,
            mapping,
        })
    }
}

pub struct OpusDecoder {
    params: AudioCodecParameters,
    opts: AudioDecoderOptions,
    head: OpusHead,
    inner: OpusMultistreamDecoder,
    gain: f32,
    /// Pre-skip frames still to drop (RFC 7845 §4.2).
    skip: usize,
    pcm: Vec<f32>,
    buf: AudioBuffer<f32>,
}

impl OpusDecoder {
    pub fn try_new(params: &AudioCodecParameters, opts: &AudioDecoderOptions) -> Result<Self> {
        if params.codec != CODEC_ID_OPUS {
            return unsupported_error("opus: invalid codec");
        }
        let Some(extra) = params.extra_data.as_deref() else {
            return unsupported_error("opus: missing OpusHead");
        };
        let head = match OpusHead::parse(extra) {
            Ok(h) => h,
            Err(e) => return unsupported_error(e),
        };
        let inner = match OpusMultistreamDecoder::new(
            OPUS_RATE,
            usize::from(head.channels),
            usize::from(head.streams),
            usize::from(head.coupled),
            &head.mapping,
        ) {
            Ok(d) => d,
            Err(_) => return unsupported_error("opus: decoder setup failed"),
        };
        let positions = if head.channels == 1 {
            Position::FRONT_LEFT
        } else {
            Position::FRONT_LEFT | Position::FRONT_RIGHT
        };
        let spec = AudioSpec::new(OPUS_RATE, Channels::Positioned(positions));
        Ok(OpusDecoder {
            params: params.clone(),
            opts: *opts,
            gain: crate::dsp::db_to_gain(head.output_gain_db) as f32,
            skip: usize::from(head.pre_skip),
            pcm: vec![0.0; MAX_FRAMES * usize::from(head.channels)],
            head,
            inner,
            buf: AudioBuffer::new(spec, MAX_FRAMES),
        })
    }

    pub fn head(&self) -> &OpusHead {
        &self.head
    }

    fn decode_inner(&mut self, packet: &PacketRef<'_>) -> Result<()> {
        self.buf.clear();
        if packet.data.is_empty() {
            // An empty packet would ask for loss concealment; there is nothing to decode.
            return Ok(());
        }
        let frames = match self.inner.decode_float(packet.data, &mut self.pcm, false) {
            Ok(n) if n <= MAX_FRAMES => n,
            _ => return decode_error("opus: invalid packet"),
        };
        let ch = usize::from(self.head.channels);
        self.buf.render_uninit(Some(frames));
        for c in 0..ch {
            let Some(plane) = self.buf.plane_mut(c) else {
                return decode_error("opus: missing plane");
            };
            for (i, s) in plane.iter_mut().enumerate() {
                *s = self.pcm[i * ch + c] * self.gain;
            }
        }
        if self.opts.gapless {
            // Symphonia 0.6's Ogg Opus mapping reports the pre-skip only as the track's
            // delay, not as per-packet trims, so drop it here. If a packet does carry a
            // start trim, the demuxer is handling it and that takes precedence.
            let start = if packet.trim_start.get() > 0 {
                self.skip = 0;
                packet.trim_start.get() as usize
            } else {
                let n = self.skip.min(frames);
                self.skip -= n;
                n
            };
            self.buf.trim(start, packet.trim_end.get() as usize);
        }
        Ok(())
    }
}

impl AudioDecoder for OpusDecoder {
    fn reset(&mut self) {
        // After a seek the demuxer positions the stream; the pre-skip no longer applies.
        self.inner.reset();
        self.skip = 0;
    }

    fn codec_info(&self) -> &CodecInfo {
        &Self::supported_codecs()[0].info
    }

    fn codec_params(&self) -> &AudioCodecParameters {
        &self.params
    }

    fn decode_ref(&mut self, packet: &PacketRef<'_>) -> Result<GenericAudioBufferRef<'_>> {
        if let Err(e) = self.decode_inner(packet) {
            self.buf.clear();
            return Err(e);
        }
        Ok(self.buf.as_generic_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        Default::default()
    }

    fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
        self.buf.as_generic_audio_buffer_ref()
    }
}

impl RegisterableAudioDecoder for OpusDecoder {
    fn try_registry_new(
        params: &AudioCodecParameters,
        opts: &AudioDecoderOptions,
    ) -> Result<Box<dyn AudioDecoder>>
    where
        Self: Sized,
    {
        Ok(Box::new(OpusDecoder::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [SupportedAudioCodec] {
        &[SupportedAudioCodec {
            id: CODEC_ID_OPUS,
            info: CodecInfo {
                short_name: "opus",
                long_name: "Opus",
                profiles: &[],
            },
        }]
    }
}

/// Symphonia's default codecs plus Opus.
pub fn codecs() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        r.register_audio_decoder::<OpusDecoder>();
        r
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(channels: u8, family: u8, extra: &[u8]) -> Vec<u8> {
        let mut h = b"OpusHead".to_vec();
        h.extend_from_slice(&[1, channels]);
        h.extend_from_slice(&312u16.to_le_bytes());
        h.extend_from_slice(&48_000u32.to_le_bytes());
        h.extend_from_slice(&(-256i16).to_le_bytes());
        h.push(family);
        h.extend_from_slice(extra);
        h
    }

    #[test]
    fn parses_valid_heads() {
        let h = OpusHead::parse(&head(2, 0, &[])).unwrap();
        assert_eq!(
            (h.channels, h.pre_skip, h.streams, h.coupled),
            (2, 312, 1, 1)
        );
        assert_eq!(h.output_gain_db, -1.0);
        let h = OpusHead::parse(&head(1, 1, &[1, 0, 0])).unwrap();
        assert_eq!(h.mapping, vec![0]);
    }

    #[test]
    fn refuses_hostile_heads() {
        let bad = [
            b"OpusHea".to_vec(),
            head(0, 0, &[]),
            head(3, 0, &[]),
            head(6, 1, &[4, 2, 0, 1, 2, 3, 4, 5]),
            head(2, 1, &[1, 1]),
            head(2, 1, &[1, 1, 0, 9]),
            head(2, 1, &[0, 0, 0, 1]),
            head(2, 1, &[1, 2, 0, 1]),
            head(2, 2, &[]),
            head(2, 255, &[]),
            {
                let mut h = head(2, 0, &[]);
                h[8] = 0x10;
                h
            },
        ];
        for b in bad {
            assert!(OpusHead::parse(&b).is_err(), "{b:?}");
        }
    }
}
