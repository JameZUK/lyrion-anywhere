//! Transitions between consecutive tracks: gapless concatenation or a crossfade.
//!
//! A crossfade of `L` frames overlaps the last `L` frames of the outgoing track with the
//! first `L` of the incoming one, so the joined output is `len_a + len_b - L` frames long.
//! Frame `i` of the overlap uses position `t = (i + ½) / L`:
//!
//! - **linear:** `out·(1 - t) + in·t` (constant amplitude: right for correlated material);
//! - **equal power:** `out·cos(πt/2) + in·sin(πt/2)` (constant power: right for unrelated
//!   tracks, the usual case for smart transitions).
//!
//! Gapless is a crossfade of zero frames: the two streams are simply concatenated, sample
//! for sample.

use std::f64::consts::FRAC_PI_2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FadeCurve {
    Linear,
    EqualPower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Transition {
    Gapless,
    Crossfade { frames: usize, curve: FadeCurve },
}

impl Transition {
    /// A crossfade of `ms` milliseconds at `sample_rate` (0 ms means gapless).
    pub fn crossfade_ms(ms: f64, sample_rate: u32, curve: FadeCurve) -> Self {
        let ms = if ms.is_finite() {
            ms.clamp(0.0, 30_000.0)
        } else {
            0.0
        };
        let frames = (ms * 1e-3 * f64::from(sample_rate)).round() as usize;
        if frames == 0 {
            Transition::Gapless
        } else {
            Transition::Crossfade { frames, curve }
        }
    }

    /// Frames shared by the two tracks.
    pub fn overlap_frames(&self) -> usize {
        match self {
            Transition::Gapless => 0,
            Transition::Crossfade { frames, .. } => *frames,
        }
    }
}

/// The (outgoing, incoming) gains at frame `i` of an `len`-frame fade.
#[inline]
pub fn fade_gains(curve: FadeCurve, i: usize, len: usize) -> (f64, f64) {
    let t = (i as f64 + 0.5) / len as f64;
    match curve {
        FadeCurve::Linear => (1.0 - t, t),
        FadeCurve::EqualPower => ((FRAC_PI_2 * t).cos(), (FRAC_PI_2 * t).sin()),
    }
}

/// Mixes the overlap of a crossfade, block by block, in step with playback.
#[derive(Debug, Clone)]
pub struct TransitionMixer {
    channels: usize,
    transition: Transition,
    pos: usize,
}

impl TransitionMixer {
    pub fn new(transition: Transition, channels: usize) -> Self {
        TransitionMixer {
            channels: channels.max(1),
            transition,
            pos: 0,
        }
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.transition.overlap_frames()
    }

    /// Frames of overlap still to mix.
    pub fn remaining(&self) -> usize {
        self.transition.overlap_frames() - self.pos.min(self.transition.overlap_frames())
    }

    /// Mix the next frames of the overlap from interleaved `outgoing` and `incoming` into
    /// `dst`. Mixes as many frames as all three hold and the fade has left; returns that
    /// count.
    pub fn mix(&mut self, outgoing: &[f64], incoming: &[f64], dst: &mut [f64]) -> usize {
        let Transition::Crossfade { frames: len, curve } = self.transition else {
            return 0;
        };
        let ch = self.channels;
        let n = (outgoing.len() / ch)
            .min(incoming.len() / ch)
            .min(dst.len() / ch)
            .min(self.remaining());
        for f in 0..n {
            let (go, gi) = fade_gains(curve, self.pos + f, len);
            for c in 0..ch {
                let k = f * ch + c;
                dst[k] = outgoing[k] * go + incoming[k] * gi;
            }
        }
        self.pos += n;
        n
    }
}

/// Join two whole tracks with `transition` (convenience for tests and offline use).
pub fn join(a: &[f64], b: &[f64], channels: usize, transition: Transition) -> Vec<f64> {
    let ch = channels.max(1);
    let overlap = transition
        .overlap_frames()
        .min(a.len() / ch)
        .min(b.len() / ch);
    let t = match transition {
        Transition::Crossfade { curve, .. } if overlap > 0 => Transition::Crossfade {
            frames: overlap,
            curve,
        },
        _ => Transition::Gapless,
    };
    let split = a.len() - overlap * ch;
    let mut out = Vec::with_capacity(a.len() + b.len() - overlap * ch);
    out.extend_from_slice(&a[..split]);
    let mut mid = vec![0.0; overlap * ch];
    TransitionMixer::new(t, ch).mix(&a[split..], &b[..overlap * ch], &mut mid);
    out.extend_from_slice(&mid);
    out.extend_from_slice(&b[overlap * ch..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gapless_is_exact_concatenation() {
        let a = [0.1, 0.2, 0.3, 0.4];
        let b = [-0.5, -0.6];
        assert_eq!(
            join(&a, &b, 2, Transition::Gapless),
            [0.1, 0.2, 0.3, 0.4, -0.5, -0.6]
        );
        assert_eq!(
            Transition::crossfade_ms(0.0, 44_100, FadeCurve::Linear),
            Transition::Gapless
        );
    }

    #[test]
    fn crossfade_is_sample_accurate() {
        let rate = 44_100;
        let t = Transition::crossfade_ms(100.0, rate, FadeCurve::EqualPower);
        assert_eq!(t.overlap_frames(), 4410);
        let a = vec![1.0; 10_000 * 2];
        let b = vec![1.0; 8_000 * 2];
        let j = join(&a, &b, 2, t);
        assert_eq!(j.len(), (10_000 + 8_000 - 4410) * 2);
        // Untouched outside the overlap.
        assert!(j[..(10_000 - 4410) * 2].iter().all(|x| *x == 1.0));
        assert!(j[10_000 * 2..].iter().all(|x| *x == 1.0));
    }

    #[test]
    fn curves_keep_amplitude_or_power() {
        for i in 0..1000 {
            let (o, n) = fade_gains(FadeCurve::Linear, i, 1000);
            assert!((o + n - 1.0).abs() < 1e-12);
            let (o, n) = fade_gains(FadeCurve::EqualPower, i, 1000);
            assert!((o * o + n * n - 1.0).abs() < 1e-12);
        }
        let (o, n) = fade_gains(FadeCurve::EqualPower, 0, 1000);
        assert!(o > 0.999 && n < 0.002);
    }

    #[test]
    fn mixer_works_in_blocks() {
        let t = Transition::Crossfade {
            frames: 100,
            curve: FadeCurve::Linear,
        };
        let out = vec![1.0; 100];
        let inc = vec![0.0; 100];
        let mut whole = vec![0.0; 100];
        TransitionMixer::new(t, 1).mix(&out, &inc, &mut whole);
        let mut m = TransitionMixer::new(t, 1);
        let mut parts = vec![0.0; 100];
        let mut done = 0;
        for chunk in [7, 13, 50, 64] {
            let end = (done + chunk).min(100);
            done += m.mix(&out[done..end], &inc[done..end], &mut parts[done..end]);
        }
        assert!(m.is_done());
        assert_eq!(whole, parts);
    }
}
