//! The stream message (protocol 2, docs/plans/signal-path-redesign.md): what LMS served and
//! what the gateway did with it, sent as text on the player socket for every stream the
//! gateway relays:
//!
//! ```json
//! {"stream":{"seq":12,
//!   "served":{"codec":"flac","rate":96000,"bits":24,"channels":2,"gain_db":-3.2},
//!   "sent":{"mode":"opus","kbps":160,"level":"high"}}}
//! ```
//!
//! - `seq`: the stream's number, the count of `strm s` frames forwarded in the session (the
//!   `n` of `/v1/stream/<session>/<n>`).
//! - `served`: the codec from the `strm` format code; rate, bits and channels from the
//!   `strm`'s PCM fields for PCM, from the STREAMINFO block for FLAC ([`flac_streaminfo`]),
//!   `null` when unknown; `gain_db` the server's ReplayGain from the `strm`, absent when LMS
//!   set none.
//! - `sent`: `{"mode":"passthrough"}` or `{"mode":"opus","kbps":…,"level":"high"|"low"}`.
//!
//! Nothing in it comes from the client or from LMS's text: only the tokens and numbers
//! above, so it is always well under [`MAX_MESSAGE`] bytes.

use anywhere_core::proto::Strm;

use crate::live::Quality;

/// The longest stream message.
pub const MAX_MESSAGE: usize = 512;

/// The bytes of a FLAC stream's start that hold the `fLaC` marker and the STREAMINFO block.
pub const STREAMINFO_LEN: usize = 42;

/// Sane bounds for what is reported; anything outside is `null`.
const RATES: std::ops::RangeInclusive<u32> = 1_000..=1_000_000;
const BITS: std::ops::RangeInclusive<u8> = 4..=32;
const CHANNELS: std::ops::RangeInclusive<u8> = 1..=8;

/// LMS's stream as it reached the gateway.
#[derive(Debug, Clone, PartialEq)]
pub struct Served {
    pub codec: &'static str,
    pub rate: Option<u32>,
    pub bits: Option<u8>,
    pub channels: Option<u8>,
    /// The server's ReplayGain, in dB to one decimal.
    pub gain_db: Option<f64>,
}

/// What the gateway sent the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Passthrough,
    /// Re-encoded to Opus at `kbps` for the level asked for (High or Low).
    Opus {
        kbps: u32,
        level: Quality,
    },
}

/// The audio format of a FLAC stream, each field `None` when outside the sane bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlacFormat {
    pub rate: Option<u32>,
    pub bits: Option<u8>,
    pub channels: Option<u8>,
}

/// The codec name for a `strm` format code.
pub fn codec(format: u8) -> &'static str {
    match format {
        b'f' => "flac",
        b'p' => "pcm",
        b'm' => "mp3",
        b'o' => "ogg",
        b'a' => "aac",
        b'l' => "alac",
        _ => "unknown",
    }
}

/// A PCM `strm`'s sample size code, in bits.
fn pcm_bits(code: u8) -> Option<u8> {
    Some(match code {
        b'0' => 8,
        b'1' => 16,
        b'2' => 24,
        b'3' => 32,
        _ => return None,
    })
}

/// A PCM `strm`'s sample rate code, in Hz.
fn pcm_rate(code: u8) -> Option<u32> {
    Some(match code {
        b'0' => 11_025,
        b'1' => 22_050,
        b'2' => 32_000,
        b'3' => 44_100,
        b'4' => 48_000,
        b'5' => 8_000,
        b'6' => 12_000,
        b'7' => 16_000,
        b'8' => 24_000,
        b'9' => 96_000,
        b':' => 88_200,
        b';' => 176_400,
        b'<' => 192_000,
        b'=' => 352_800,
        b'>' => 384_000,
        _ => return None,
    })
}

/// A PCM `strm`'s channel count code.
fn pcm_channels(code: u8) -> Option<u8> {
    match code {
        b'1' => Some(1),
        b'2' => Some(2),
        _ => None,
    }
}

/// The server's ReplayGain from a `strm s` (16.16 fixed point, 0 for none) in dB, to one
/// decimal.
pub fn gain_db(fixed: u32) -> Option<f64> {
    if fixed == 0 {
        return None;
    }
    let db = 20.0 * (f64::from(fixed) / 65_536.0).log10();
    let rounded = (db * 10.0).round() / 10.0;
    // Never "-0.0".
    Some(if rounded == 0.0 { 0.0 } else { rounded })
}

impl Served {
    /// What a `strm s` says: the codec, the PCM parameters of a PCM stream and the gain.
    /// A FLAC stream's format comes later, from its first bytes ([`Self::with_flac`]).
    pub fn from_strm(s: &Strm) -> Served {
        let pcm = s.format == b'p';
        Served {
            codec: codec(s.format),
            rate: pcm.then(|| pcm_rate(s.pcm_sample_rate)).flatten(),
            bits: pcm.then(|| pcm_bits(s.pcm_sample_size)).flatten(),
            channels: pcm.then(|| pcm_channels(s.pcm_channels)).flatten(),
            gain_db: gain_db(s.replay_gain),
        }
    }

    /// The format read from a FLAC stream's STREAMINFO.
    pub fn with_flac(self, f: FlacFormat) -> Served {
        Served {
            rate: f.rate,
            bits: f.bits,
            channels: f.channels,
            ..self
        }
    }
}

/// The format in a FLAC stream's first [`STREAMINFO_LEN`] bytes: the `fLaC` marker, then a
/// metadata block header for a 34-byte STREAMINFO, then the block (sample rate 20 bits,
/// channels − 1 in 3 bits, bits per sample − 1 in 5 bits, from byte 18). Anything else (too
/// short, no marker, another block first: the headerless stream of a seek) is `None`; a
/// field outside the sane bounds is `None` on its own.
pub fn flac_streaminfo(head: &[u8]) -> Option<FlacFormat> {
    let h = head.get(..STREAMINFO_LEN)?;
    if &h[..4] != b"fLaC" || h[4] & 0x7f != 0 || h[5..8] != [0, 0, 34] {
        return None;
    }
    let rate = (u32::from(h[18]) << 12) | (u32::from(h[19]) << 4) | (u32::from(h[20]) >> 4);
    let channels = ((h[20] >> 1) & 0x07) + 1;
    let bits = (((h[20] & 0x01) << 4) | (h[21] >> 4)) + 1;
    Some(FlacFormat {
        rate: Some(rate).filter(|r| RATES.contains(r)),
        bits: Some(bits).filter(|b| BITS.contains(b)),
        channels: Some(channels).filter(|c| CHANNELS.contains(c)),
    })
}

/// The stream message for stream `seq`.
pub fn message(seq: u64, served: &Served, sent: Sent) -> String {
    let mut s = serde_json::json!({
        "codec": served.codec,
        "rate": served.rate,
        "bits": served.bits,
        "channels": served.channels,
    });
    if let Some(g) = served.gain_db.filter(|g| g.is_finite()) {
        s["gain_db"] = serde_json::json!(g);
    }
    let sent = match sent {
        Sent::Passthrough => serde_json::json!({ "mode": "passthrough" }),
        Sent::Opus { kbps, level } => serde_json::json!({
            "mode": "opus",
            "kbps": kbps,
            "level": level.name(),
        }),
    };
    let out =
        serde_json::json!({ "stream": { "seq": seq, "served": s, "sent": sent } }).to_string();
    debug_assert!(out.len() <= MAX_MESSAGE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FLAC start with a STREAMINFO for `rate`, `channels` and `bits` (as stored: the
    /// real values, which the block holds less one).
    pub(crate) fn streaminfo(rate: u32, channels: u8, bits: u8) -> Vec<u8> {
        let mut v = b"fLaC".to_vec();
        v.extend_from_slice(&[0x80, 0, 0, 34]); // last block, STREAMINFO, 34 bytes
        v.extend_from_slice(&[0x10, 0x00, 0x10, 0x00]); // block sizes
        v.extend_from_slice(&[0; 6]); // frame sizes
        let c = (channels - 1) & 7;
        let b = (bits - 1) & 31;
        v.push((rate >> 12) as u8);
        v.push((rate >> 4) as u8);
        v.push((((rate & 0x0f) as u8) << 4) | (c << 1) | (b >> 4));
        v.push((b & 0x0f) << 4);
        v.extend_from_slice(&[0; 4]); // total samples
        v.extend_from_slice(&[0; 16]); // MD5
        assert_eq!(v.len(), STREAMINFO_LEN);
        v
    }

    #[test]
    fn reads_streaminfo() {
        let f = flac_streaminfo(&streaminfo(96_000, 2, 24)).unwrap();
        assert_eq!(
            f,
            FlacFormat {
                rate: Some(96_000),
                bits: Some(24),
                channels: Some(2)
            }
        );
        let f = flac_streaminfo(&streaminfo(44_100, 1, 16)).unwrap();
        assert_eq!(
            (f.rate, f.bits, f.channels),
            (Some(44_100), Some(16), Some(1))
        );
        // More bytes after the block change nothing; not the last block is fine too.
        let mut v = streaminfo(192_000, 2, 24);
        v[4] = 0;
        v.extend_from_slice(b"rest of the stream");
        assert_eq!(flac_streaminfo(&v).unwrap().rate, Some(192_000));
    }

    #[test]
    fn refuses_what_is_not_a_streaminfo() {
        let good = streaminfo(96_000, 2, 24);
        assert!(flac_streaminfo(&good[..41]).is_none());
        assert!(flac_streaminfo(b"").is_none());
        let mut v = good.clone();
        v[0] = b'F';
        assert!(flac_streaminfo(&v).is_none(), "no marker");
        let mut v = good.clone();
        v[4] = 0x84; // a PICTURE block first
        assert!(flac_streaminfo(&v).is_none());
        let mut v = good.clone();
        v[7] = 33; // wrong length
        assert!(flac_streaminfo(&v).is_none());
        // A headerless stream (a seek): frames straight away.
        let mut frames = vec![0xff, 0xf8, 0x69, 0x08];
        frames.resize(64, 0x55);
        assert!(flac_streaminfo(&frames).is_none());
    }

    #[test]
    fn extremes_are_null_on_their_own() {
        let f = flac_streaminfo(&streaminfo(0, 2, 16)).unwrap();
        assert_eq!((f.rate, f.bits, f.channels), (None, Some(16), Some(2)));
        let f = flac_streaminfo(&streaminfo((1 << 20) - 1, 8, 32)).unwrap();
        assert_eq!((f.rate, f.bits, f.channels), (None, Some(32), Some(8)));
        let f = flac_streaminfo(&streaminfo(999, 1, 1)).unwrap();
        assert_eq!((f.rate, f.bits, f.channels), (None, None, Some(1)));
        let f = flac_streaminfo(&streaminfo(1_000_000, 8, 4)).unwrap();
        assert_eq!(
            (f.rate, f.bits, f.channels),
            (Some(1_000_000), Some(4), Some(8))
        );
        let f = flac_streaminfo(&streaminfo(1_000, 1, 3)).unwrap();
        assert_eq!((f.rate, f.bits), (Some(1_000), None));
    }

    /// Robustness: every length from 0 to 64 bytes of random data, with and without a real
    /// marker and block header in front, never panics and only ever reports values within
    /// the bounds, or none.
    #[test]
    fn random_bytes_never_panic_and_stay_in_bounds() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let check = |b: &[u8]| {
            if let Some(f) = flac_streaminfo(b) {
                assert!(f.rate.is_none_or(|r| RATES.contains(&r)), "{f:?}");
                assert!(f.bits.is_none_or(|x| BITS.contains(&x)), "{f:?}");
                assert!(f.channels.is_none_or(|c| CHANNELS.contains(&c)), "{f:?}");
            }
        };
        let mut reported = 0;
        for len in 0..=64usize {
            for _ in 0..500 {
                let mut b: Vec<u8> = (0..len).map(|_| next() as u8).collect();
                check(&b);
                if len >= 8 {
                    b[..8].copy_from_slice(&[b'f', b'L', b'a', b'C', 0x80, 0, 0, 34]);
                    check(&b);
                    reported += usize::from(flac_streaminfo(&b).is_some());
                }
            }
        }
        assert!(reported > 0, "the prefixed cases were parsed");
        for fill in [0x00u8, 0xff] {
            let mut b = vec![fill; 64];
            b[..8].copy_from_slice(&[b'f', b'L', b'a', b'C', 0x00, 0, 0, 34]);
            check(&b);
        }
    }

    fn strm(format: u8, size: u8, rate: u8, channels: u8, gain: u32) -> Strm {
        Strm {
            command: b's',
            autostart: b'1',
            format,
            pcm_sample_size: size,
            pcm_sample_rate: rate,
            pcm_channels: channels,
            pcm_endianness: b'1',
            threshold: 0,
            spdif_enable: 0,
            transition_period: 0,
            transition_type: b'0',
            flags: 0,
            output_threshold: 0,
            slaves: 0,
            replay_gain: gain,
            server_port: 9000,
            server_ip: 0,
            request: Vec::new(),
        }
    }

    #[test]
    fn served_from_the_strm() {
        let pcm = Served::from_strm(&strm(b'p', b'1', b'3', b'2', 0));
        assert_eq!(
            pcm,
            Served {
                codec: "pcm",
                rate: Some(44_100),
                bits: Some(16),
                channels: Some(2),
                gain_db: None
            }
        );
        let pcm = Served::from_strm(&strm(b'p', b'2', b'9', b'1', 0));
        assert_eq!(
            (pcm.rate, pcm.bits, pcm.channels),
            (Some(96_000), Some(24), Some(1))
        );
        let unknown = Served::from_strm(&strm(b'p', b'?', b'?', b'?', 0));
        assert_eq!(
            (unknown.rate, unknown.bits, unknown.channels),
            (None, None, None)
        );
        // Only PCM takes its format from the strm.
        let mp3 = Served::from_strm(&strm(b'm', b'1', b'3', b'2', 0));
        assert_eq!((mp3.codec, mp3.rate, mp3.bits), ("mp3", None, None));
        for (f, name) in [
            (b'f', "flac"),
            (b'o', "ogg"),
            (b'a', "aac"),
            (b'l', "alac"),
            (b'u', "unknown"),
            (b'?', "unknown"),
            (0, "unknown"),
        ] {
            assert_eq!(Served::from_strm(&strm(f, b'?', b'?', b'?', 0)).codec, name);
        }
    }

    #[test]
    fn gain_in_db() {
        assert_eq!(gain_db(0), None);
        assert_eq!(gain_db(65_536), Some(0.0));
        // -3.2 dB is 10^(-3.2/20) = 0.6918 of unity.
        assert_eq!(gain_db((0.6918 * 65_536.0) as u32), Some(-3.2));
        assert_eq!(gain_db(2 * 65_536), Some(6.0));
        assert_eq!(gain_db(65_500), Some(0.0), "never -0.0");
        assert_eq!(gain_db(1), Some(-96.3));
        assert_eq!(gain_db(u32::MAX), Some(96.3));
    }

    #[test]
    fn the_message() {
        let served = Served {
            codec: "flac",
            rate: Some(96_000),
            bits: Some(24),
            channels: Some(2),
            gain_db: Some(-3.2),
        };
        let m = message(
            12,
            &served,
            Sent::Opus {
                kbps: 160,
                level: Quality::High,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"stream":{"seq":12,
                "served":{"codec":"flac","rate":96000,"bits":24,"channels":2,"gain_db":-3.2},
                "sent":{"mode":"opus","kbps":160,"level":"high"}}})
        );
        let m = message(
            1,
            &Served {
                codec: "mp3",
                rate: None,
                bits: None,
                channels: None,
                gain_db: None,
            },
            Sent::Passthrough,
        );
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"stream":{"seq":1,
                "served":{"codec":"mp3","rate":null,"bits":null,"channels":null},
                "sent":{"mode":"passthrough"}}})
        );
        // The largest it can be is well within the bound.
        let big = message(
            u64::MAX,
            &Served {
                codec: "unknown",
                rate: Some(1_000_000),
                bits: Some(32),
                channels: Some(8),
                gain_db: Some(-96.3),
            },
            Sent::Opus {
                kbps: u32::MAX,
                level: Quality::Low,
            },
        );
        assert!(big.len() <= MAX_MESSAGE, "{}", big.len());
        // Never mistaken for a greeting, a keepalive or a control message.
        assert!(v.get("session").is_none() && v.get("keepalive").is_none());
    }
}
