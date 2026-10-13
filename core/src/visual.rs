//! The visualiser's analysis (docs/plans/wave2-design.md §4.1): from the PCM a player already
//! has, at most [`MAX_FPS`] frames per second of audio, each with a log-spaced spectrum, an
//! oscilloscope trace, left/right RMS and peak, and a bass level.
//!
//! Sans-IO and allocation-free once built. Any input is accepted: NaN or infinite samples
//! count as silence, and odd channel counts or sample rates are clamped, never a panic.

use serde::Serialize;

/// FFT length (a power of two).
pub const FFT_SIZE: usize = 2048;
/// Spectrum bands per frame.
pub const BANDS: usize = 32;
/// Oscilloscope points per frame.
pub const SCOPE_POINTS: usize = 128;
/// Most frames per second of audio.
pub const MAX_FPS: u32 = 30;
/// The spectrum's range: 0 is this many dB below full scale, 1 is full scale.
pub const RANGE_DB: f32 = 70.0;
/// The quietest level reported (dBFS), for silence.
pub const FLOOR_DB: f32 = -90.0;

const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 16_000.0;
const BASS_HZ: f32 = 150.0;
const MAX_CHANNELS: usize = 8;

/// One frame for the visualiser.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualFrame {
    /// Log-spaced bands from 40 Hz to 16 kHz, each 0–1 over [`RANGE_DB`].
    pub bands: Vec<f32>,
    /// The latest samples (mono), −1–1, oldest first.
    pub scope: Vec<f32>,
    /// RMS and peak per side over the frame, in dBFS (≥ [`FLOOR_DB`]). Mono: both the same.
    pub rms_db: [f32; 2],
    pub peak_db: [f32; 2],
    /// Energy below 150 Hz, 0–1 (for the glow).
    pub bass: f32,
}

/// Feeds PCM in, takes frames out.
#[derive(Debug, Clone)]
pub struct Visualiser {
    channels: usize,
    rate: f32,
    hop: usize,
    /// Mono history, a ring of [`FFT_SIZE`].
    ring: Vec<f32>,
    pos: usize,
    filled: usize,
    since: usize,
    sum_sq: [f64; 2],
    peak: [f32; 2],
    counted: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// FFT bin ranges for each band.
    edges: Vec<(usize, usize)>,
    bass_bins: usize,
    pending: Option<VisualFrame>,
}

fn clean(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn db(x: f64) -> f32 {
    if x > 0.0 && x.is_finite() {
        ((20.0 * x.log10()) as f32).clamp(FLOOR_DB, 0.0)
    } else {
        FLOOR_DB
    }
}

impl Visualiser {
    /// For interleaved PCM of `channels` at `sample_rate` Hz (clamped to sane values).
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let rate = sample_rate.clamp(8_000, 768_000) as f32;
        let channels = (channels as usize).clamp(1, MAX_CHANNELS);
        let hop = (rate as usize / MAX_FPS as usize).max(1);
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        let bin_hz = rate / FFT_SIZE as f32;
        let nyquist = FFT_SIZE / 2;
        let high = HIGH_HZ.min(rate / 2.0 * 0.95);
        let mut edges = Vec::with_capacity(BANDS);
        let ratio = (high / LOW_HZ).max(1.0);
        for b in 0..BANDS {
            let lo = LOW_HZ * ratio.powf(b as f32 / BANDS as f32);
            let hi = LOW_HZ * ratio.powf((b + 1) as f32 / BANDS as f32);
            let a = ((lo / bin_hz).floor() as usize).clamp(1, nyquist - 1);
            let z = ((hi / bin_hz).ceil() as usize).clamp(a + 1, nyquist);
            edges.push((a, z));
        }
        let bass_bins = ((BASS_HZ / bin_hz).ceil() as usize).clamp(2, nyquist);
        Visualiser {
            channels,
            rate,
            hop,
            ring: vec![0.0; FFT_SIZE],
            pos: 0,
            filled: 0,
            since: 0,
            sum_sq: [0.0; 2],
            peak: [0.0; 2],
            counted: 0,
            window,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            edges,
            bass_bins,
            pending: None,
        }
    }

    /// The sample rate in use (after clamping).
    pub fn sample_rate(&self) -> u32 {
        self.rate as u32
    }

    /// Add interleaved samples (a trailing partial frame is ignored). A frame becomes
    /// available every 1/[`MAX_FPS`] s of audio; only the newest is kept.
    pub fn push(&mut self, samples: &[f32]) {
        let ch = self.channels;
        for f in samples.chunks_exact(ch) {
            let l = clean(f[0]);
            let r = if ch > 1 { clean(f[1]) } else { l };
            let mono = if ch == 1 {
                l
            } else {
                f.iter().map(|x| clean(*x)).sum::<f32>() / ch as f32
            };
            self.ring[self.pos] = mono;
            self.pos = (self.pos + 1) % FFT_SIZE;
            self.filled = (self.filled + 1).min(FFT_SIZE);
            for (i, x) in [l, r].into_iter().enumerate() {
                self.sum_sq[i] += f64::from(x) * f64::from(x);
                self.peak[i] = self.peak[i].max(x.abs());
            }
            self.counted += 1;
            self.since += 1;
            if self.since >= self.hop {
                self.since = 0;
                self.pending = Some(self.analyse());
            }
        }
    }

    /// The newest frame since the last call, if any.
    pub fn take(&mut self) -> Option<VisualFrame> {
        self.pending.take()
    }

    /// Forget the history (a seek, a new track).
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.pos = 0;
        self.filled = 0;
        self.since = 0;
        self.sum_sq = [0.0; 2];
        self.peak = [0.0; 2];
        self.counted = 0;
        self.pending = None;
    }

    fn analyse(&mut self) -> VisualFrame {
        // Oldest first: the ring from `pos`.
        for i in 0..FFT_SIZE {
            let x = self.ring[(self.pos + i) % FFT_SIZE];
            self.re[i] = x * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im);
        // Magnitude relative to a full-scale sine through the Hann window (gain 0.5).
        let norm = 2.0 / (FFT_SIZE as f32 * 0.5);
        let mag = |k: usize, re: &[f32], im: &[f32]| (re[k] * re[k] + im[k] * im[k]).sqrt() * norm;
        let bands = self
            .edges
            .iter()
            .map(|&(a, z)| {
                let m = (a..z)
                    .map(|k| mag(k, &self.re, &self.im))
                    .fold(0.0f32, f32::max);
                let d = db(f64::from(m));
                ((d + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
            })
            .collect();
        let bass_m = (1..self.bass_bins)
            .map(|k| mag(k, &self.re, &self.im))
            .fold(0.0f32, f32::max);
        let bass = ((db(f64::from(bass_m)) + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0);
        let start = (self.pos + FFT_SIZE - SCOPE_POINTS.min(self.filled.max(1))) % FFT_SIZE;
        let scope = (0..SCOPE_POINTS)
            .map(|i| self.ring[(start + i) % FFT_SIZE])
            .collect();
        let n = self.counted.max(1) as f64;
        let rms_db = [
            db((self.sum_sq[0] / n).sqrt()),
            db((self.sum_sq[1] / n).sqrt()),
        ];
        let peak_db = [db(f64::from(self.peak[0])), db(f64::from(self.peak[1]))];
        self.sum_sq = [0.0; 2];
        self.peak = [0.0; 2];
        self.counted = 0;
        VisualFrame {
            bands,
            scope,
            rms_db,
            peak_db,
            bass,
        }
    }
}

/// In-place iterative radix-2 FFT (`re.len()` a power of two, equal to `im.len()`).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] as f64 * cr - im[b] as f64 * ci;
                let ti = re[b] as f64 * ci + im[b] as f64 * cr;
                re[b] = (re[a] as f64 - tr) as f32;
                im[b] = (im[a] as f64 - ti) as f32;
                re[a] = (re[a] as f64 + tr) as f32;
                im[a] = (im[a] as f64 + ti) as f32;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f32, amp: f32, frames: usize, channels: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let x = amp * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin();
                std::iter::repeat_n(x, channels)
            })
            .collect()
    }

    fn band_of(v: &Visualiser, hz: f32) -> usize {
        let bin = (hz / (v.rate / FFT_SIZE as f32)) as usize;
        v.edges
            .iter()
            .position(|&(a, z)| (a..z).contains(&bin))
            .unwrap()
    }

    #[test]
    fn the_fft_finds_a_tone() {
        let mut re: Vec<f32> = (0..64)
            .map(|i| (2.0 * std::f32::consts::PI * 8.0 * i as f32 / 64.0).cos())
            .collect();
        let mut im = vec![0.0; 64];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..32)
            .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
            .collect();
        let top = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(top, 8);
        assert!((mags[8] - 32.0).abs() < 0.01);
    }

    #[test]
    fn a_tone_lights_its_band_and_levels_are_right() {
        let mut v = Visualiser::new(48_000, 2);
        v.push(&sine(48_000, 1_000.0, 0.5, 4_800, 2));
        let f = v.take().expect("a frame");
        assert_eq!(f.bands.len(), BANDS);
        assert_eq!(f.scope.len(), SCOPE_POINTS);
        let b = band_of(&v, 1_000.0);
        let top = f
            .bands
            .iter()
            .enumerate()
            .max_by(|a, c| a.1.total_cmp(c.1))
            .unwrap()
            .0;
        assert!(
            top.abs_diff(b) <= 1,
            "loudest band {top}, expected about {b}"
        );
        // A −6 dBFS sine: peak −6 dB, RMS −9 dB.
        assert!((f.peak_db[0] + 6.02).abs() < 0.2, "{:?}", f.peak_db);
        assert!((f.rms_db[1] + 9.03).abs() < 0.3, "{:?}", f.rms_db);
        assert!(f.bands[b] > 0.8);
        assert!(f.bass < 0.5, "no bass in a 1 kHz tone: {}", f.bass);
    }

    #[test]
    fn bass_drives_the_glow() {
        let mut v = Visualiser::new(44_100, 1);
        v.push(&sine(44_100, 60.0, 0.8, 8_820, 1));
        assert!(v.take().unwrap().bass > 0.8);
    }

    #[test]
    fn frames_come_at_most_thirty_a_second_of_audio() {
        let mut v = Visualiser::new(48_000, 2);
        let mut frames = 0;
        for _ in 0..100 {
            v.push(&sine(48_000, 440.0, 0.1, 480, 2)); // 10 ms
            frames += usize::from(v.take().is_some());
        }
        assert!((29..=31).contains(&frames), "{frames} frames in 1 s");
    }

    #[test]
    fn silence_and_garbage_are_quiet_never_a_panic() {
        let mut v = Visualiser::new(48_000, 2);
        v.push(&[0.0; 9_600]);
        let f = v.take().unwrap();
        assert!(f.bands.iter().all(|b| *b == 0.0));
        assert_eq!(f.peak_db, [FLOOR_DB; 2]);
        let junk = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e30,
            -1e30,
            f32::MIN_POSITIVE,
        ];
        let mut v = Visualiser::new(48_000, 2);
        v.push(&junk.repeat(2_000));
        let f = v.take().unwrap();
        assert!(f.bands.iter().chain(&f.scope).all(|x| x.is_finite()));
        assert!(f.bands.iter().all(|b| (0.0..=1.0).contains(b)));
        assert!(
            f.peak_db
                .iter()
                .chain(&f.rms_db)
                .all(|d| (FLOOR_DB..=0.0).contains(d))
        );
    }

    #[test]
    fn odd_formats_are_clamped() {
        for (rate, ch) in [
            (0, 0),
            (1, 1),
            (u32::MAX, u16::MAX),
            (8_000, 3),
            (768_000, 8),
            (11_025, 1),
        ] {
            let mut v = Visualiser::new(rate, ch);
            v.push(&vec![0.25; 8 * 60_000]); // over 1/30 s at any rate and width
            v.push(&[0.1; 7]); // a partial frame
            let f = v.take().unwrap();
            assert_eq!(f.bands.len(), BANDS);
            assert!(f.bands.iter().all(|b| b.is_finite()));
        }
    }

    #[test]
    fn reset_forgets_the_past() {
        let mut v = Visualiser::new(48_000, 1);
        v.push(&sine(48_000, 500.0, 0.9, 4_000, 1));
        v.reset();
        assert!(v.take().is_none());
        v.push(&[0.0; 1_600]);
        assert!(v.take().unwrap().bands.iter().all(|b| *b == 0.0));
    }

    #[test]
    fn random_input_is_bounded_and_cheap() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let rate = 8_000 + (rnd() % 760_000) as u32;
            let ch = (rnd() % 9) as u16;
            let mut v = Visualiser::new(rate, ch);
            let n = (rnd() % 20_000) as usize;
            let buf: Vec<f32> = (0..n).map(|_| f32::from_bits(rnd() as u32)).collect();
            v.push(&buf);
            if let Some(f) = v.take() {
                assert!(f.bands.iter().chain(&f.scope).all(|x| x.is_finite()));
            }
        }
        // One second of 48 kHz stereo (30 FFTs) well within a second, even unoptimised.
        let mut v = Visualiser::new(48_000, 2);
        let buf = sine(48_000, 440.0, 0.5, 48_000, 2);
        let t = std::time::Instant::now();
        v.push(&buf);
        assert!(
            t.elapsed() < std::time::Duration::from_millis(900),
            "{:?}",
            t.elapsed()
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(20));
    }
}
