//! Waveform peaks in one streaming pass, without knowing the track's length.
//!
//! # Method: doubling buckets
//!
//! Frames are summarised into equal-width internal buckets (peak = max |sample| over all
//! channels; energy = sum of squares, for RMS). Bucket width starts at one frame. When
//! there are `4N` full buckets (N = the requested output count), adjacent pairs merge
//! into `2N` buckets of twice the width. Memory is therefore at most `4N` buckets however
//! long the track is, and when it ends there are between `2N` and `4N` (fewer only for
//! tracks shorter than `2N` frames).
//!
//! The output's `N` buckets each cover an equal share of the track and combine the
//! internal buckets whose midpoints fall in that share (max of peaks; RMS over all their
//! frames). Since each output bucket spans at least two internal ones, a peak can shift
//! by at most half an output bucket, which is invisible at display size, and is never
//! shown twice.
//!
//! Values are quantised linearly to `u8` (0 = silence, 255 = full scale or above).

/// Default output buckets.
pub const DEFAULT_BUCKETS: usize = 1000;
/// Most output buckets accepted.
pub const MAX_BUCKETS: usize = 10_000;

#[derive(Debug, Clone, Copy, Default)]
struct Bucket {
    peak: f64,
    energy: f64,
    frames: u64,
}

impl Bucket {
    fn merge(&mut self, o: &Bucket) {
        self.peak = self.peak.max(o.peak);
        self.energy += o.energy;
        self.frames += o.frames;
    }
}

/// Streaming waveform builder.
#[derive(Debug, Clone)]
pub struct WaveformBuilder {
    out: usize,
    width: u64,
    full: Vec<Bucket>,
    current: Bucket,
}

impl WaveformBuilder {
    pub fn new(buckets: usize) -> Self {
        let out = buckets.clamp(1, MAX_BUCKETS);
        WaveformBuilder {
            out,
            width: 1,
            full: Vec::with_capacity(4 * out),
            current: Bucket::default(),
        }
    }

    /// Add one frame, summarised as its largest |sample| and its mean square over channels.
    #[inline]
    pub fn push(&mut self, peak: f64, mean_square: f64) {
        let c = &mut self.current;
        if peak > c.peak {
            c.peak = peak;
        }
        c.energy += mean_square;
        c.frames += 1;
        if c.frames == self.width {
            self.full.push(*c);
            self.current = Bucket::default();
            if self.full.len() == 4 * self.out {
                self.halve();
            }
        }
    }

    fn halve(&mut self) {
        let n = self.full.len() / 2;
        for i in 0..n {
            let mut b = self.full[2 * i];
            b.merge(&self.full[2 * i + 1]);
            self.full[i] = b;
        }
        self.full.truncate(n);
        self.width *= 2;
    }

    /// Internal buckets in use (bounded by `4 × buckets`).
    pub fn internal_len(&self) -> usize {
        self.full.len() + usize::from(self.current.frames > 0)
    }

    /// The final `(peak, rms)` arrays of `buckets` values each (empty for no frames).
    pub fn finish(&self) -> (Vec<u8>, Vec<u8>) {
        let mut src = self.full.clone();
        if self.current.frames > 0 {
            src.push(self.current);
        }
        let total: u64 = src.iter().map(|b| b.frames).sum();
        if total == 0 {
            return (Vec::new(), Vec::new());
        }
        let mut peaks = Vec::with_capacity(self.out);
        let mut rms = Vec::with_capacity(self.out);
        // Walk source buckets by frame position.
        let starts: Vec<u64> = src
            .iter()
            .scan(0u64, |acc, b| {
                let s = *acc;
                *acc += b.frames;
                Some(s)
            })
            .collect();
        // Each internal bucket belongs to the output bucket holding its midpoint (so
        // nothing is counted twice); an output bucket holding no midpoint (tracks shorter
        // than the output) shows the internal bucket under its own midpoint.
        let mut j = 0;
        for i in 0..self.out {
            let lo = (total as u128 * i as u128 / self.out as u128) as u64;
            let hi = (total as u128 * (i as u128 + 1) / self.out as u128) as u64;
            let mid2 = |k: usize| 2 * starts[k] + src[k].frames;
            while j < src.len() && mid2(j) < 2 * lo {
                j += 1;
            }
            let mut b = Bucket::default();
            let mut k = j;
            while k < src.len() && mid2(k) < 2 * hi {
                b.merge(&src[k]);
                k += 1;
            }
            if b.frames == 0 {
                let centre = (lo + hi) / 2;
                let at = starts.partition_point(|s| *s <= centre).saturating_sub(1);
                b = src[at];
            }
            peaks.push(quantise(b.peak));
            let r = if b.frames > 0 {
                (b.energy / b.frames as f64).sqrt()
            } else {
                0.0
            };
            rms.push(quantise(r));
        }
        (peaks, rms)
    }
}

#[inline]
fn quantise(v: f64) -> u8 {
    if v.is_finite() {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_tiny() {
        let w = WaveformBuilder::new(10);
        assert_eq!(w.finish(), (vec![], vec![]));
        let mut w = WaveformBuilder::new(10);
        w.push(1.0, 1.0);
        let (p, r) = w.finish();
        assert_eq!(p, vec![255; 10]);
        assert_eq!(r, vec![255; 10]);
    }

    #[test]
    fn shape_follows_the_signal_for_any_length() {
        for frames in [7u64, 1000, 1999, 2000, 2001, 123_457, 1_000_000] {
            let mut w = WaveformBuilder::new(100);
            for i in 0..frames {
                // A ramp up from silence to full scale.
                let v = i as f64 / frames as f64;
                w.push(v, v * v);
                assert!(w.internal_len() <= 400);
            }
            let (p, r) = w.finish();
            assert_eq!(p.len(), 100);
            assert!(p.windows(2).all(|x| x[0] <= x[1]), "{frames}: {p:?}");
            if frames >= 1000 {
                assert!(*p.last().unwrap() >= 252);
                // Bucket i peaks near (i + 1) / 100 of full scale.
                for (i, v) in p.iter().enumerate() {
                    let want = (i + 1) as f64 / 100.0 * 255.0;
                    assert!(
                        (f64::from(*v) - want).abs() <= 2.6 + 255.0 / 100.0,
                        "{frames} {i} {v}"
                    );
                }
                assert!(r.iter().zip(&p).all(|(r, p)| r <= p));
            }
        }
    }

    #[test]
    fn a_click_is_never_lost() {
        let frames = 5_000_000u64;
        let mut w = WaveformBuilder::new(1000);
        for i in 0..frames {
            let v = if i == 3_210_987 { 0.9 } else { 0.01 };
            w.push(v, v * v);
        }
        let (p, _) = w.finish();
        let pos = p.iter().position(|v| *v > 200).unwrap();
        assert!((pos as i64 - 642).abs() <= 1, "{pos}");
        assert_eq!(p.iter().filter(|v| **v > 200).count(), 1);
    }

    #[test]
    fn quantisation() {
        assert_eq!(quantise(f64::NAN), 0);
        assert_eq!(quantise(2.0), 255);
        assert_eq!(quantise(-1.0), 0);
        assert_eq!(quantise(0.5), 128);
    }
}
