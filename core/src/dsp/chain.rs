//! The processing chain: gain (preamp + levelling) → EQ → crossfeed → true-peak limiter,
//! then output quantisation with dither only where the bit depth is reduced.

use serde::{Deserialize, Serialize};

use super::crossfeed::{Crossfeed, CrossfeedConfig};
use super::dither::{Dither, DitherMode, needs_dither};
use super::eq::{EqProfile, ParametricEq};
use super::gain::Gain;
use super::limiter::{LimiterConfig, TruePeakLimiter};
use super::signal_path::{Stage, fmt_db};

/// What the chain should do. The default does nothing (bit-perfect).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChainConfig {
    /// User preamp, dB.
    pub preamp_db: f64,
    /// Loudness levelling gain for the current track, dB (from [`super::levelling::decide`]).
    pub levelling_db: f64,
    pub eq: Option<EqProfile>,
    pub crossfeed: Option<CrossfeedConfig>,
    pub limiter: Option<LimiterConfig>,
    pub dither: DitherMode,
}

/// Largest sample magnitude the processing path accepts (+60 dBFS); anything beyond is
/// garbage, clamped so that no stage can overflow.
pub const MAX_INPUT: f64 = 1e3;

fn all_finite(buf: &[f64]) -> bool {
    buf.iter().all(|x| x.is_finite())
}

#[derive(Debug, Clone)]
pub struct Chain {
    config: ChainConfig,
    sample_rate: u32,
    channels: usize,
    gain: Gain,
    eq: Option<ParametricEq>,
    crossfeed: Option<Crossfeed>,
    limiter: Option<TruePeakLimiter>,
    dither: Dither,
}

impl Chain {
    /// Build a chain. Refuses an invalid EQ profile.
    pub fn new(config: ChainConfig, sample_rate: u32, channels: usize) -> Result<Self, String> {
        let channels = channels.max(1);
        let eq = match &config.eq {
            Some(p) => Some(ParametricEq::new(p.clone(), sample_rate, channels)?),
            None => None,
        };
        Ok(Chain {
            gain: Gain::new(config.preamp_db + config.levelling_db),
            eq,
            crossfeed: config
                .crossfeed
                .filter(|_| channels == 2)
                .map(|c| Crossfeed::new(c, sample_rate)),
            limiter: config
                .limiter
                .map(|l| TruePeakLimiter::new(sample_rate, channels, l)),
            dither: Dither::new(config.dither, channels),
            config,
            sample_rate,
            channels,
        })
    }

    pub fn config(&self) -> &ChainConfig {
        &self.config
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Rebuild rate-dependent stages for a new sample rate (state is cleared).
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        if let Some(eq) = &mut self.eq {
            eq.set_sample_rate(sample_rate);
        }
        if let Some(cf) = &mut self.crossfeed {
            cf.set_sample_rate(sample_rate);
        }
        if let Some(l) = &mut self.limiter {
            *l = TruePeakLimiter::new(sample_rate, self.channels, l.config());
        }
    }

    /// Change the levelling gain (e.g. at a track boundary) without rebuilding anything.
    pub fn set_levelling_db(&mut self, db: f64) {
        self.config.levelling_db = db;
        self.gain = Gain::new(self.config.preamp_db + db);
    }

    /// No stage changes any sample: output is bit-identical to input.
    pub fn is_bit_transparent(&self) -> bool {
        self.gain.is_bypassed()
            && self.eq.as_ref().is_none_or(|e| e.is_bypassed())
            && self.crossfeed.is_none()
            && self.limiter.is_none()
    }

    /// Frames of delay through the chain.
    pub fn latency_frames(&self) -> usize {
        self.limiter.as_ref().map_or(0, |l| l.latency_frames())
    }

    /// The limiter, for gain-reduction metering.
    pub fn limiter(&self) -> Option<&TruePeakLimiter> {
        self.limiter.as_ref()
    }

    /// Clear filter and delay state (after a seek or flush).
    pub fn reset(&mut self) {
        if let Some(eq) = &mut self.eq {
            eq.reset();
        }
        if let Some(cf) = &mut self.crossfeed {
            cf.reset();
        }
        if let Some(l) = &mut self.limiter {
            l.reset();
        }
        self.dither.reset();
    }

    /// Process interleaved frames in place. Allocates nothing. A trailing partial frame
    /// is left untouched.
    ///
    /// Input is sanitised on the processing path: non-finite samples become silence and
    /// magnitudes are clamped to [`MAX_INPUT`], so no stage can overflow. If a recursive
    /// stage still produced a non-finite value, its state is reset and the block muted
    /// rather than letting it poison every later block.
    pub fn process(&mut self, buf: &mut [f64]) {
        if self.is_bit_transparent() {
            return;
        }
        let whole = buf.len() - buf.len() % self.channels;
        let buf = &mut buf[..whole];
        for s in buf.iter_mut() {
            *s = super::sanitise(*s).clamp(-MAX_INPUT, MAX_INPUT);
        }
        self.gain.process(buf);
        if let Some(eq) = &mut self.eq {
            eq.process(buf);
            if !all_finite(buf) {
                eq.reset();
                buf.fill(0.0);
            }
        }
        if let Some(cf) = &mut self.crossfeed {
            cf.process(buf, self.channels);
            if !all_finite(buf) {
                cf.reset();
                buf.fill(0.0);
            }
        }
        if let Some(l) = &mut self.limiter {
            l.process(buf);
        }
    }

    /// Convert processed samples to `output_bits`-deep integers, dithering only where the
    /// depth is reduced (see [`needs_dither`]). `source_bits` is the source's integer
    /// depth (`None` for float or lossy sources).
    pub fn quantise(
        &mut self,
        src: &[f64],
        source_bits: Option<u32>,
        output_bits: u32,
        dst: &mut [i32],
    ) {
        if needs_dither(Some(output_bits), source_bits, !self.is_bit_transparent()) {
            self.dither.quantise(src, output_bits, dst);
        } else {
            super::pcm::from_f64(src, output_bits, dst);
        }
    }

    /// The chain's stages for the signal-path panel, in processing order.
    pub fn stages(&self) -> Vec<Stage> {
        let mut v = Vec::new();
        let c = &self.config;
        if c.preamp_db != 0.0 {
            v.push(Stage::processing(format!("Preamp {}", fmt_db(c.preamp_db))));
        }
        if c.levelling_db != 0.0 {
            v.push(Stage::processing(format!(
                "Levelling {}",
                fmt_db(c.levelling_db)
            )));
        }
        if let Some(eq) = &self.eq
            && !eq.is_bypassed()
        {
            v.push(Stage::processing(format!(
                "EQ ({} filters)",
                eq.profile().filters.len()
            )));
        }
        if let Some(cf) = &self.crossfeed {
            let cfg = cf.config();
            v.push(Stage::processing(format!(
                "Crossfeed {:.0} Hz, {}",
                cfg.cutoff_hz,
                fmt_db(-cfg.feed_db)
            )));
        }
        if let Some(l) = &self.limiter {
            v.push(Stage::processing(format!(
                "True-peak limiter {} dBTP",
                fmt_db(l.config().ceiling_dbtp).trim_end_matches(" dB")
            )));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::crossfeed::CrossfeedPreset;

    #[test]
    fn default_chain_is_transparent() {
        let mut c = Chain::new(ChainConfig::default(), 44_100, 2).unwrap();
        assert!(c.is_bit_transparent());
        assert_eq!(c.latency_frames(), 0);
        let mut b = [0.25, -0.5, f64::MIN_POSITIVE, 0.0];
        let o = b;
        c.process(&mut b);
        assert_eq!(b, o);
        assert!(c.stages().is_empty());
    }

    #[test]
    fn stages_describe_the_config() {
        let cfg = ChainConfig {
            preamp_db: -2.0,
            levelling_db: -3.2,
            crossfeed: Some(CrossfeedPreset::Natural.config()),
            limiter: Some(LimiterConfig::default()),
            ..Default::default()
        };
        let c = Chain::new(cfg, 48_000, 2).unwrap();
        let names: Vec<String> = c.stages().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "Preamp −2.0 dB",
                "Levelling −3.2 dB",
                "Crossfeed 700 Hz, −4.5 dB",
                "True-peak limiter −1.0 dBTP"
            ]
        );
        assert!(!c.is_bit_transparent());
        assert!(c.latency_frames() > 0);
    }

    #[test]
    fn crossfeed_is_dropped_for_non_stereo() {
        let cfg = ChainConfig {
            crossfeed: Some(CrossfeedPreset::Natural.config()),
            ..Default::default()
        };
        assert!(Chain::new(cfg, 48_000, 1).unwrap().is_bit_transparent());
    }

    #[test]
    fn levelling_changes_without_rebuild() {
        let mut c = Chain::new(ChainConfig::default(), 44_100, 2).unwrap();
        c.set_levelling_db(-6.020_599_913_279_624);
        let mut b = [0.5, 0.5];
        c.process(&mut b);
        assert!((b[0] - 0.25).abs() < 1e-12);
        c.set_levelling_db(0.0);
        assert!(c.is_bit_transparent());
    }

    #[test]
    fn rate_change_keeps_working() {
        let cfg = ChainConfig {
            limiter: Some(LimiterConfig::default()),
            eq: Some(
                crate::dsp::eq::parse_autoeq("Filter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1").unwrap(),
            ),
            ..Default::default()
        };
        let mut c = Chain::new(cfg, 44_100, 2).unwrap();
        let l1 = c.latency_frames();
        c.set_sample_rate(192_000);
        assert!(c.latency_frames() > l1);
        let mut b = vec![0.9; 4096];
        c.process(&mut b);
        assert!(b.iter().all(|x| x.is_finite()));
    }
}
