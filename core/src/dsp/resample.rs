//! Sample-rate conversion, used **only** when the output can't take the source's rate.
//!
//! Wraps the `rubato` crate's asynchronous sinc resampler (pure Rust, MIT) with a long
//! Blackman-Harris-windowed sinc (256 taps, cubic interpolation between 256 oversampled
//! points), all in `f64`. The wrapper hides rubato's fixed chunk size: feed any number of
//! frames, get every output frame exactly once, aligned (the filter's delay is trimmed),
//! and [`Resampler::flush`] produces exactly `round(input · to / from)` frames in total.
//!
//! Behind the `resample` cargo feature, so the WebAssembly build doesn't carry it.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, Resampler as _, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

const CHUNK: usize = 1024;

pub struct Resampler {
    inner: Async<f64>,
    channels: usize,
    from: u32,
    to: u32,
    pending: Vec<f64>,
    scratch: Vec<f64>,
    to_skip: usize,
    frames_in: u64,
    frames_out: u64,
}

impl std::fmt::Debug for Resampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resampler")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("channels", &self.channels)
            .finish()
    }
}

impl Resampler {
    pub fn new(from: u32, to: u32, channels: usize) -> Result<Self, String> {
        let ok = |r: u32| (8_000..=768_000).contains(&r);
        if !ok(from) || !ok(to) {
            return Err("sample rate out of range".into());
        }
        if channels == 0 || channels > 32 {
            return Err("unsupported channel count".into());
        }
        let mut params = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2);
        params.oversampling_factor = 256;
        params.interpolation = SincInterpolationType::Cubic;
        let inner = Async::<f64>::new_sinc(
            f64::from(to) / f64::from(from),
            1.0,
            &params,
            CHUNK,
            channels,
            FixedAsync::Input,
        )
        .map_err(|e| format!("resampler: {e}"))?;
        let to_skip = inner.output_delay();
        let scratch = vec![0.0; inner.output_frames_max() * channels];
        Ok(Resampler {
            inner,
            channels,
            from,
            to,
            pending: Vec::with_capacity(CHUNK * channels),
            scratch,
            to_skip,
            frames_in: 0,
            frames_out: 0,
        })
    }

    pub fn rates(&self) -> (u32, u32) {
        (self.from, self.to)
    }

    fn expected_out(&self) -> u64 {
        (self.frames_in * u64::from(self.to) + u64::from(self.from) / 2) / u64::from(self.from)
    }

    fn run_chunk(&mut self, valid: usize, out: &mut Vec<f64>) {
        let ch = self.channels;
        self.pending.resize(CHUNK * ch, 0.0);
        let input = InterleavedSlice::new(&self.pending[..], ch, CHUNK).expect("sized");
        let cap = self.scratch.len() / ch;
        let mut output = InterleavedSlice::new_mut(&mut self.scratch[..], ch, cap).expect("sized");
        let idx = Indexing::new().partial_len(valid);
        let (_, n) = self
            .inner
            .process_into_buffer(&input, &mut output, Some(&idx))
            .expect("buffers are sized from the resampler");
        self.pending.clear();
        let skip = self.to_skip.min(n);
        self.to_skip -= skip;
        out.extend_from_slice(&self.scratch[skip * ch..n * ch]);
        self.frames_out += (n - skip) as u64;
    }

    /// Resample interleaved `input`, appending output frames to `out`.
    pub fn process(&mut self, input: &[f64], out: &mut Vec<f64>) {
        let ch = self.channels;
        let whole = input.len() - input.len() % ch;
        let mut rest = &input[..whole];
        self.frames_in += (whole / ch) as u64;
        while !rest.is_empty() {
            let room = CHUNK * ch - self.pending.len();
            let take = room.min(rest.len());
            self.pending
                .extend(rest[..take].iter().map(|x| super::sanitise(*x)));
            rest = &rest[take..];
            if self.pending.len() == CHUNK * ch {
                self.run_chunk(CHUNK, out);
            }
        }
    }

    /// Drain the filter at end of stream: total output becomes exactly
    /// `round(input frames · to / from)`.
    pub fn flush(&mut self, out: &mut Vec<f64>) {
        let ch = self.channels;
        let want = self.expected_out();
        let mut guard = 0;
        while self.frames_out < want && guard < 64 {
            let valid = self.pending.len() / ch;
            self.run_chunk(valid, out);
            guard += 1;
        }
        if self.frames_out > want {
            let extra = (self.frames_out - want) as usize;
            out.truncate(out.len() - extra * ch);
            self.frames_out = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn length_and_tone_are_preserved() {
        for (from, to) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (96_000, 48_000),
            (44_100, 192_000),
        ] {
            let n = from as usize / 2;
            let f = 1000.0;
            let x: Vec<f64> = (0..n)
                .flat_map(|i| {
                    let v = 0.5 * (2.0 * PI * f * i as f64 / f64::from(from)).sin();
                    [v, -v]
                })
                .collect();
            let mut r = Resampler::new(from, to, 2).unwrap();
            let mut out = Vec::new();
            for c in x.chunks(777 * 2) {
                r.process(c, &mut out);
            }
            r.flush(&mut out);
            let want = (n as u64 * u64::from(to) + u64::from(from) / 2) / u64::from(from);
            assert_eq!(out.len() as u64, want * 2, "{from}→{to}");
            // Fit the sine away from the edges: amplitude must be preserved, and the
            // output aligned with the input to within about one output sample (rubato reports
            // its delay in whole samples).
            let w = 2.0 * PI * f / f64::from(to);
            let (mut s, mut c) = (0.0, 0.0);
            let range = (want as usize / 4)..(want as usize * 3 / 4);
            for i in range.clone() {
                s += out[i * 2] * (w * i as f64).sin();
                c += out[i * 2] * (w * i as f64).cos();
                assert_eq!(out[i * 2], -out[i * 2 + 1]);
            }
            let n_fit = range.len() as f64;
            let amp = 2.0 * (s * s + c * c).sqrt() / n_fit;
            let lag = -c.atan2(s) / w;
            assert!((amp - 0.5).abs() < 1e-3, "{from}→{to}: amplitude {amp}");
            assert!(lag.abs() < 1.5, "{from}→{to}: lag {lag} samples");
        }
    }

    #[test]
    fn rejects_bad_parameters() {
        assert!(Resampler::new(0, 48_000, 2).is_err());
        assert!(Resampler::new(44_100, 10_000_000, 2).is_err());
        assert!(Resampler::new(44_100, 48_000, 0).is_err());
    }
}
