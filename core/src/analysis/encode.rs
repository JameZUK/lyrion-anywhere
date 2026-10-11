//! Compact encodings of [`TrackAnalysis`]: JSON (waveform arrays as base64 strings) and a
//! versioned little-endian binary form. Both decoders treat their input as untrusted:
//! lengths and values are checked, and they never panic.
//!
//! Binary layout (version 1), all little-endian:
//!
//! ```text
//! "LAAN"  magic
//! u16     version
//! u32     sample rate         u16 channels       u64 frames
//! f32 ×5  integrated LUFS, LRA LU, max momentary, max short-term, true peak dBTP
//! f32     sample peak dBFS    (NaN encodes "none" in all six)
//! f32 ×2  leading, trailing silence (s)
//! edge ×2 fade in, fade out:  f32 slope dB/s, f32 start LUFS, f32 end LUFS, f32 length s, u8 fade
//! u64     invalid samples
//! u16     buckets (N)         u8 ×N peak          u8 ×N RMS
//! ```

use super::waveform::MAX_BUCKETS;
use super::{ANALYSIS_VERSION, Edge, TrackAnalysis, Waveform};

const MAGIC: &[u8; 4] = b"LAAN";
/// Largest binary encoding accepted.
pub const MAX_BINARY_BYTES: usize = 64 + 22 * 4 + 2 * 17 + 2 * MAX_BUCKETS + 64;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= c.len() {
                s.push(B64[(n >> (18 - 6 * k)) as usize & 63] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Decode standard base64 with padding; `None` for anything malformed.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    let quads = b.len() / 4;
    for (qi, q) in b.chunks(4).enumerate() {
        let last = qi + 1 == quads;
        let pad = q.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (k, c) in q.iter().enumerate() {
            let v = if k >= 4 - pad { 0 } else { val(*c)? };
            n |= v << (18 - 6 * k);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

/// Serde helper: `Vec<u8>` as a base64 string.
pub mod b64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::base64_encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() > super::MAX_BUCKETS.div_ceil(3) * 4 {
            return Err(D::Error::custom("waveform too long"));
        }
        super::base64_decode(&s).ok_or_else(|| D::Error::custom("bad base64"))
    }
}

struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f64) {
        self.0.extend_from_slice(&(v as f32).to_le_bytes());
    }
    fn opt(&mut self, v: Option<f64>) {
        self.f32(v.unwrap_or(f64::NAN));
    }
    fn edge(&mut self, e: &Edge) {
        self.f32(e.slope_db_per_s);
        self.opt(e.start_lufs);
        self.opt(e.end_lufs);
        self.f32(e.length_s);
        self.u8(u8::from(e.fade));
    }
}

struct R<'a>(&'a [u8]);
impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.0.len() < n {
            return Err("truncated".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// A finite value within ±1e6 (analysis values are far smaller).
    fn f32(&mut self) -> Result<f64, String> {
        let v = f64::from(f32::from_le_bytes(self.take(4)?.try_into().unwrap()));
        if v.is_finite() && v.abs() <= 1e6 {
            Ok(v)
        } else {
            Err("value out of range".into())
        }
    }
    fn opt(&mut self) -> Result<Option<f64>, String> {
        let v = f64::from(f32::from_le_bytes(self.take(4)?.try_into().unwrap()));
        if v.is_nan() {
            Ok(None)
        } else if v.is_finite() && v.abs() <= 1e6 {
            Ok(Some(v))
        } else {
            Err("value out of range".into())
        }
    }
    fn edge(&mut self) -> Result<Edge, String> {
        Ok(Edge {
            slope_db_per_s: self.f32()?,
            start_lufs: self.opt()?,
            end_lufs: self.opt()?,
            length_s: self.f32()?,
            fade: match self.u8()? {
                0 => false,
                1 => true,
                _ => return Err("bad flag".into()),
            },
        })
    }
}

impl TrackAnalysis {
    /// The compact binary form (about 2.1 KiB for 1000 buckets).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = W(Vec::with_capacity(128 + 2 * self.waveform.peak.len()));
        w.0.extend_from_slice(MAGIC);
        w.u16(self.version);
        w.u32(self.sample_rate);
        w.u16(self.channels);
        w.u64(self.frames);
        w.opt(self.integrated_lufs);
        w.opt(self.loudness_range_lu);
        w.opt(self.max_momentary_lufs);
        w.opt(self.max_short_term_lufs);
        w.opt(self.true_peak_dbtp);
        w.opt(self.sample_peak_dbfs);
        w.f32(self.leading_silence_s);
        w.f32(self.trailing_silence_s);
        w.edge(&self.fade_in);
        w.edge(&self.fade_out);
        w.u64(self.invalid_samples);
        let n = self.waveform.peak.len().min(MAX_BUCKETS);
        w.u16(n as u16);
        w.0.extend_from_slice(&self.waveform.peak[..n]);
        let mut rms = self.waveform.rms.clone();
        rms.resize(n, 0);
        w.0.extend_from_slice(&rms);
        w.0
    }

    /// Parse the binary form, refusing anything malformed, oversized or of another version.
    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        if b.len() > MAX_BINARY_BYTES {
            return Err("too large".into());
        }
        let mut r = R(b);
        if r.take(4)? != MAGIC {
            return Err("not an analysis".into());
        }
        let version = r.u16()?;
        if version != ANALYSIS_VERSION {
            return Err(format!("unsupported version {version}"));
        }
        let sample_rate = r.u32()?;
        let channels = r.u16()?;
        let frames = r.u64()?;
        let a = TrackAnalysis {
            version,
            sample_rate,
            channels,
            frames,
            integrated_lufs: r.opt()?,
            loudness_range_lu: r.opt()?,
            max_momentary_lufs: r.opt()?,
            max_short_term_lufs: r.opt()?,
            true_peak_dbtp: r.opt()?,
            sample_peak_dbfs: r.opt()?,
            leading_silence_s: r.f32()?,
            trailing_silence_s: r.f32()?,
            fade_in: r.edge()?,
            fade_out: r.edge()?,
            invalid_samples: r.u64()?,
            waveform: {
                let n = r.u16()? as usize;
                if n > MAX_BUCKETS {
                    return Err("too many buckets".into());
                }
                Waveform {
                    peak: r.take(n)?.to_vec(),
                    rms: r.take(n)?.to_vec(),
                }
            },
        };
        if !r.0.is_empty() {
            return Err("trailing bytes".into());
        }
        Ok(a)
    }

    /// Compact JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("analysis always serialises")
    }

    /// Parse JSON, refusing oversized input, other versions, and inconsistent waveforms.
    pub fn from_json(s: &str) -> Result<Self, String> {
        if s.len() > 4 * MAX_BINARY_BYTES {
            return Err("too large".into());
        }
        let a: TrackAnalysis = serde_json::from_str(s).map_err(|e| e.to_string())?;
        if a.version != ANALYSIS_VERSION {
            return Err(format!("unsupported version {}", a.version));
        }
        if a.waveform.peak.len() != a.waveform.rms.len() {
            return Err("waveform arrays differ in length".into());
        }
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip_and_rejects() {
        for n in 0..50 {
            let v: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let e = base64_encode(&v);
            assert_eq!(base64_decode(&e).unwrap(), v);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        for bad in [
            "T", "TQ=", "TQ===", "T===", "TQ==TWFu", "TW!u", "====", "TWFu\n",
        ] {
            assert!(base64_decode(bad).is_none(), "{bad}");
        }
    }
}
