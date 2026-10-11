//! Headphone crossfeed after Benjamin Bauer's stereophonic-to-binaural idea (1961): on
//! loudspeakers each ear also hears the opposite speaker, later and with its high
//! frequencies shadowed by the head. Headphones lose that, so hard-panned sounds feel
//! unnaturally wide and tiring.
//!
//! Each output channel is the direct channel plus the *opposite* channel, delayed,
//! low-passed (first order) and attenuated:
//!
//! ```text
//! L' = gd·L + gc·LP(delay(R))      R' = gd·R + gc·LP(delay(L))
//! ```
//!
//! `gc / gd` is the feed level at low frequencies and `gd + gc = 1`, so a centred (mono)
//! low-frequency sound keeps unity gain; above the cut-off the crossfeed fades out, as the
//! head's shadow does. The one-pole low-pass adds its own group delay (about
//! `1 / (2π·fc)`, 0.23 ms at 700 Hz) to the explicit delay, together approximating the
//! interaural time difference of speakers at ±30°. Implemented from these published
//! descriptions only; stereo signals only (other channel counts pass through untouched).

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CrossfeedConfig {
    /// Low-pass corner of the crossfed signal, Hz (300..=2000).
    pub cutoff_hz: f64,
    /// How far the crossfed signal sits below the direct one at low frequencies, dB
    /// (1..=20).
    pub feed_db: f64,
    /// Extra delay of the crossfed signal, µs (0..=1000).
    pub delay_us: f64,
}

/// Presets, in order of strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CrossfeedPreset {
    /// Subtle: 650 Hz, -9.5 dB.
    Light,
    /// 700 Hz, -6 dB.
    Moderate,
    /// Closest to loudspeakers: 700 Hz, -4.5 dB.
    Natural,
}

impl CrossfeedPreset {
    pub fn config(self) -> CrossfeedConfig {
        let (cutoff_hz, feed_db) = match self {
            CrossfeedPreset::Light => (650.0, 9.5),
            CrossfeedPreset::Moderate => (700.0, 6.0),
            CrossfeedPreset::Natural => (700.0, 4.5),
        };
        CrossfeedConfig {
            cutoff_hz,
            feed_db,
            delay_us: 90.0,
        }
    }
}

impl CrossfeedConfig {
    pub fn sanitised(&self) -> Self {
        let f = |x: f64, d: f64, lo: f64, hi: f64| if x.is_finite() { x.clamp(lo, hi) } else { d };
        CrossfeedConfig {
            cutoff_hz: f(self.cutoff_hz, 700.0, 300.0, 2000.0),
            feed_db: f(self.feed_db, 6.0, 1.0, 20.0),
            delay_us: f(self.delay_us, 90.0, 0.0, 1000.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Crossfeed {
    config: CrossfeedConfig,
    direct: f64,
    cross: f64,
    lp_coef: f64,
    lp: [f64; 2],
    /// Delay line of (L, R) pairs.
    delay: Vec<[f64; 2]>,
    pos: usize,
}

impl Crossfeed {
    pub fn new(config: CrossfeedConfig, sample_rate: u32) -> Self {
        let mut c = Crossfeed {
            config: config.sanitised(),
            direct: 1.0,
            cross: 0.0,
            lp_coef: 0.0,
            lp: [0.0; 2],
            delay: Vec::new(),
            pos: 0,
        };
        c.set_sample_rate(sample_rate);
        c
    }

    pub fn config(&self) -> CrossfeedConfig {
        self.config
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let fs = f64::from(sample_rate.clamp(8_000, 768_000));
        let r = 10f64.powf(-self.config.feed_db / 20.0);
        self.direct = 1.0 / (1.0 + r);
        self.cross = r / (1.0 + r);
        self.lp_coef = 1.0 - (-2.0 * PI * self.config.cutoff_hz / fs).exp();
        let frames = (self.config.delay_us * 1e-6 * fs).round() as usize;
        self.delay = vec![[0.0; 2]; frames + 1];
        self.reset();
    }

    pub fn reset(&mut self) {
        self.lp = [0.0; 2];
        self.delay.fill([0.0; 2]);
        self.pos = 0;
    }

    /// Process interleaved stereo in place; any other channel count is left untouched.
    pub fn process(&mut self, buf: &mut [f64], channels: usize) {
        if channels != 2 {
            return;
        }
        let len = self.delay.len();
        for f in buf.as_chunks_mut::<2>().0 {
            let (l, r) = (f[0], f[1]);
            // Oldest entry of the ring is the delayed pair (or the current one if no delay).
            self.delay[self.pos] = [l, r];
            let d = self.delay[(self.pos + 1) % len];
            self.pos = (self.pos + 1) % len;
            // Each ear's crossfeed comes from the opposite channel.
            self.lp[0] += self.lp_coef * (d[1] - self.lp[0]);
            self.lp[1] += self.lp_coef * (d[0] - self.lp[1]);
            f[0] = self.direct * l + self.cross * self.lp[0];
            f[1] = self.direct * r + self.cross * self.lp[1];
        }
        self.lp = self.lp.map(super::flush_denormal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cf: &mut Crossfeed, l: f64, r: f64, n: usize) -> (f64, f64) {
        let mut buf: Vec<f64> = (0..n).flat_map(|_| [l, r]).collect();
        cf.process(&mut buf, 2);
        (buf[buf.len() - 2], buf[buf.len() - 1])
    }

    #[test]
    fn mono_dc_has_unity_gain() {
        let mut cf = Crossfeed::new(CrossfeedPreset::Natural.config(), 44_100);
        let (l, r) = run(&mut cf, 0.5, 0.5, 44_100);
        assert!((l - 0.5).abs() < 1e-9 && (r - 0.5).abs() < 1e-9);
    }

    #[test]
    fn hard_panned_low_frequencies_reach_the_other_ear() {
        let cfg = CrossfeedPreset::Moderate.config();
        let mut cf = Crossfeed::new(cfg, 48_000);
        let (l, r) = run(&mut cf, 1.0, 0.0, 48_000);
        let ratio_db = 20.0 * (r / l).log10();
        assert!((ratio_db + cfg.feed_db).abs() < 1e-6, "{ratio_db}");
    }

    #[test]
    fn delay_is_applied() {
        let cfg = CrossfeedConfig {
            cutoff_hz: 2000.0,
            feed_db: 3.0,
            delay_us: 500.0,
        };
        let mut cf = Crossfeed::new(cfg, 48_000);
        let mut buf = vec![0.0; 200];
        buf[0] = 1.0; // impulse on the left
        cf.process(&mut buf, 2);
        let first_right = buf.chunks(2).position(|f| f[1] != 0.0).unwrap();
        assert_eq!(first_right, 24, "500 µs at 48 kHz");
    }

    #[test]
    fn other_channel_counts_pass_through() {
        let mut cf = Crossfeed::new(CrossfeedPreset::Light.config(), 44_100);
        let mut b = [0.1, 0.2, 0.3];
        cf.process(&mut b, 3);
        assert_eq!(b, [0.1, 0.2, 0.3]);
    }
}
