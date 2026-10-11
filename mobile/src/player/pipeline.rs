//! The engine's processing between the core's decoded PCM and the output ring:
//!
//! ```text
//! core pull (per track) → [DSP chain] → [resampler] → pending → ring → output
//! ```
//!
//! - **Bit-perfect by default.** With DSP off and the output at the source rate, blocks are
//!   copied straight through: no conversion, no arithmetic. Volume (`audg`) is applied by
//!   the core, and at 100 % it is exactly 1.0, which leaves samples untouched.
//! - **DSP (optional):** preamp, ReplayGain levelling from the track's tags (track or album
//!   mode) and the true-peak limiter, from `core::dsp`, in `f64`. Levelling is decided per
//!   track: the core's [`Player::pull_track`] never mixes two tracks in one block.
//! - **Resampling** happens only when the output could not be opened at the source rate.
//! - **Played accounting:** the core must learn which frames have actually been played (for
//!   elapsed time, `STMs` and `STMu`). Every block written to the ring is recorded with the
//!   source frames it carries; as the output's read position passes it, those frames are
//!   reported. This also works when resampling changes the number of frames.
//!
//! [`Player::pull_track`]: anywhere_core::player::Player::pull_track

use std::collections::VecDeque;

use anywhere_core::decode::ReplayGainTags;
use anywhere_core::dsp::chain::{Chain, ChainConfig};
use anywhere_core::dsp::levelling::{
    self, LevellingConfig, LevellingMode, LoudnessInfo, Measurement,
};
use anywhere_core::dsp::limiter::LimiterConfig;
use anywhere_core::dsp::pcm;
use anywhere_core::dsp::resample::Resampler;
use anywhere_core::dsp::signal_path::Stage;

use super::ring::{CHANNELS, Producer};

/// Loudness levelling from ReplayGain tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Levelling {
    Off,
    Track,
    /// Album gain where tagged, otherwise track gain.
    Album,
}

/// The engine's optional processing. The default does nothing (bit-perfect).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DspSettings {
    /// Master switch: when off, every stage is bypassed.
    pub enabled: bool,
    /// Preamp, dB (−20..=+12).
    pub preamp_db: f64,
    pub levelling: Levelling,
    /// Levelling target, LUFS (−30..=−5); ReplayGain's reference is −18.
    pub target_lufs: f64,
    /// True-peak limiter at −1 dBTP (recommended with any boost).
    pub limiter: bool,
    /// Level from the gateway's R128 analysis of each track (integrated loudness and true
    /// peak), falling back to ReplayGain tags where there is none. Needs `levelling`.
    #[uniffi(default = false)]
    pub analysis_levelling: bool,
}

impl Default for DspSettings {
    fn default() -> Self {
        DspSettings {
            enabled: false,
            preamp_db: 0.0,
            levelling: Levelling::Off,
            target_lufs: -18.0,
            limiter: true,
            analysis_levelling: false,
        }
    }
}

/// Where a track's levelling gain came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelSource {
    /// No levelling (off, or nothing known about the track).
    None,
    /// ReplayGain tags in the file.
    Tags,
    /// The gateway's R128 analysis.
    Analysis,
}

impl DspSettings {
    /// The same settings with every value forced into its supported range (they come from
    /// the app's settings, which may be stale or hand-edited).
    pub fn sanitised(&self) -> DspSettings {
        let f = |x: f64, d: f64, lo: f64, hi: f64| if x.is_finite() { x.clamp(lo, hi) } else { d };
        DspSettings {
            enabled: self.enabled,
            preamp_db: f(self.preamp_db, 0.0, -20.0, 12.0),
            levelling: self.levelling,
            target_lufs: f(self.target_lufs, -18.0, -30.0, -5.0),
            limiter: self.limiter,
            analysis_levelling: self.analysis_levelling,
        }
    }

    /// Does any stage change samples?
    pub fn active(&self) -> bool {
        self.enabled && (self.preamp_db != 0.0 || self.levelling != Levelling::Off || self.limiter)
    }

    fn chain_config(&self) -> ChainConfig {
        ChainConfig {
            preamp_db: self.preamp_db,
            levelling_db: 0.0,
            limiter: self.limiter.then(LimiterConfig::default),
            ..ChainConfig::default()
        }
    }
}

/// The levelling gain for a track's tags (0 when unknown or off).
pub fn levelling_gain_db(settings: &DspSettings, tags: &ReplayGainTags) -> f64 {
    levelling_decision(settings, Some(tags), None).0
}

/// The levelling gain for a track, dB, and where it came from.
///
/// - **Track mode:** the analysis's integrated loudness and true peak when
///   `analysis_levelling` is on and the track has an analysis, otherwise the track's
///   ReplayGain tags.
/// - **Album mode:** the album's ReplayGain tags where present (an album's loudness needs
///   every track, which one track's analysis can't give), otherwise as track mode.
pub fn levelling_decision(
    settings: &DspSettings,
    tags: Option<&ReplayGainTags>,
    analysis: Option<Measurement>,
) -> (f64, LevelSource) {
    let mode = match settings.levelling {
        Levelling::Off => return (0.0, LevelSource::None),
        Levelling::Track => LevellingMode::Track,
        Levelling::Album => LevellingMode::Album,
    };
    let m = |g: Option<f64>, p: Option<f64>| {
        g.and_then(|g| Measurement::from_replaygain(g, p.unwrap_or(1.0)))
    };
    let analysed = analysis.filter(|_| settings.analysis_levelling);
    let tag_track = tags.and_then(|t| m(t.track_gain_db, t.track_peak));
    let info = LoudnessInfo {
        track: analysed.or(tag_track),
        album: tags.and_then(|t| m(t.album_gain_db, t.album_peak)),
    };
    let cfg = LevellingConfig {
        target_lufs: settings.target_lufs,
        mode,
        // Without the limiter, never boost past the peak.
        limiter_headroom_db: if settings.limiter { 3.0 } else { 0.0 },
        ..LevellingConfig::default()
    };
    match levelling::decide(&cfg, &info) {
        None => (0.0, LevelSource::None),
        Some(d) => {
            let source = if d.used == LevellingMode::Track && analysed.is_some() {
                LevelSource::Analysis
            } else {
                LevelSource::Tags
            };
            (d.gain_db, source)
        }
    }
}

/// Ring samples carrying some source frames.
#[derive(Debug, Clone, Copy)]
struct Mark {
    /// Ring position (samples) where this block ends.
    end: u64,
    /// Ring samples in the block.
    len: u64,
    /// Source frames the block carries.
    src_frames: u64,
    /// Source frames already reported.
    reported: u64,
}

pub struct Pipeline {
    dsp: DspSettings,
    chain: Option<Chain>,
    resampler: Option<Resampler>,
    /// Rate of the audio entering the pipeline.
    src_rate: Option<u32>,
    /// Rate of the output stream.
    out_rate: Option<u32>,
    /// Track whose levelling is set up.
    track: Option<u32>,
    levelling_db: f64,
    level_source: LevelSource,
    pending: Vec<f32>,
    pending_off: usize,
    /// Source frames carried by `pending`, and how many of them were already written.
    pending_src: u64,
    pending_src_written: u64,
    marks: VecDeque<Mark>,
    /// The pending audio is the resampler's final tail.
    drained: bool,
    f64_in: Vec<f64>,
    f64_out: Vec<f64>,
}

impl Pipeline {
    pub fn new(dsp: DspSettings) -> Pipeline {
        Pipeline {
            dsp: dsp.sanitised(),
            chain: None,
            resampler: None,
            src_rate: None,
            out_rate: None,
            track: None,
            levelling_db: 0.0,
            level_source: LevelSource::None,
            pending: Vec::new(),
            pending_off: 0,
            pending_src: 0,
            pending_src_written: 0,
            marks: VecDeque::new(),
            drained: false,
            f64_in: Vec::new(),
            f64_out: Vec::new(),
        }
    }

    pub fn dsp(&self) -> &DspSettings {
        &self.dsp
    }

    pub fn src_rate(&self) -> Option<u32> {
        self.src_rate
    }

    pub fn out_rate(&self) -> Option<u32> {
        self.out_rate
    }

    pub fn is_resampling(&self) -> bool {
        self.resampler.is_some()
    }

    pub fn levelling_db(&self) -> f64 {
        self.levelling_db
    }

    /// Where the current track's levelling gain came from.
    pub fn level_source(&self) -> LevelSource {
        self.level_source
    }

    /// The track whose levelling is set up (the last one pushed).
    pub fn track(&self) -> Option<u32> {
        self.track
    }

    /// Change the DSP settings (applies to audio processed from now on).
    pub fn set_dsp(&mut self, dsp: DspSettings) {
        let dsp = dsp.sanitised();
        if dsp == self.dsp {
            return;
        }
        self.dsp = dsp;
        self.chain = None;
        self.track = None; // re-decide levelling for the next block
        self.build_chain();
    }

    fn build_chain(&mut self) {
        self.chain = match (self.dsp.active(), self.src_rate) {
            (true, Some(rate)) => Chain::new(self.dsp.chain_config(), rate, CHANNELS).ok(),
            _ => None,
        };
        if let Some(c) = &mut self.chain {
            c.set_levelling_db(self.levelling_db);
        }
    }

    /// The pipeline now takes audio at `src` and the output runs at `out`.
    pub fn configure(&mut self, src: u32, out: u32) -> Result<(), String> {
        self.resampler = if src == out {
            None
        } else {
            Some(Resampler::new(src, out, CHANNELS)?)
        };
        self.src_rate = Some(src);
        self.out_rate = Some(out);
        match &mut self.chain {
            Some(c) => c.set_sample_rate(src),
            None => self.build_chain(),
        }
        Ok(())
    }

    /// The output went away (closed or failed); rates are unknown until reconfigured.
    pub fn unconfigure(&mut self) {
        self.src_rate = None;
        self.out_rate = None;
        self.resampler = None;
    }

    pub fn has_pending(&self) -> bool {
        self.pending_off < self.pending.len()
    }

    /// Process one block of the core's PCM (interleaved stereo, `frames` frames of track
    /// `seq`) into the pending buffer. Call only when [`Pipeline::has_pending`] is false.
    pub fn push(&mut self, block: &[f32], seq: u32, tags: Option<&ReplayGainTags>) {
        self.push_with(block, seq, tags, None);
    }

    /// [`Pipeline::push`], with the track's R128 analysis when known (used for levelling
    /// if `analysis_levelling` is on; decided once, at the track's first block).
    pub fn push_with(
        &mut self,
        block: &[f32],
        seq: u32,
        tags: Option<&ReplayGainTags>,
        analysis: Option<Measurement>,
    ) {
        debug_assert!(!self.has_pending());
        // Source frames that produced no output yet (held inside the resampler) ride along
        // with the next output, so every frame is eventually reported as played.
        let carry = self.pending_src - self.pending_src_written;
        self.pending.clear();
        self.pending_off = 0;
        let frames = (block.len() / CHANNELS) as u64 + carry;
        self.drained = false;
        if self.track != Some(seq) {
            self.track = Some(seq);
            (self.levelling_db, self.level_source) = levelling_decision(&self.dsp, tags, analysis);
            if let Some(c) = &mut self.chain {
                c.set_levelling_db(self.levelling_db);
            }
        }
        if self.chain.is_none() && self.resampler.is_none() {
            // The bit-perfect path: a plain copy.
            self.pending.extend_from_slice(block);
        } else {
            self.f64_in.resize(block.len(), 0.0);
            pcm::f32_to_f64(block, &mut self.f64_in);
            if let Some(c) = &mut self.chain {
                c.process(&mut self.f64_in);
            }
            let out: &[f64] = match &mut self.resampler {
                Some(r) => {
                    self.f64_out.clear();
                    r.process(&self.f64_in, &mut self.f64_out);
                    &self.f64_out
                }
                None => &self.f64_in,
            };
            self.pending.resize(out.len(), 0.0);
            pcm::f64_to_f32(out, &mut self.pending);
        }
        self.pending_src = frames;
        self.pending_src_written = 0;
    }

    /// The source rate is about to change (or the audio has ended): emit the resampler's
    /// tail. Call only when [`Pipeline::has_pending`] is false.
    pub fn drain_resampler(&mut self) {
        debug_assert!(!self.has_pending());
        if let Some(r) = &mut self.resampler {
            let carry = self.pending_src - self.pending_src_written;
            self.f64_out.clear();
            r.flush(&mut self.f64_out);
            self.pending.clear();
            self.pending_off = 0;
            self.pending.resize(self.f64_out.len(), 0.0);
            pcm::f64_to_f32(&self.f64_out, &mut self.pending);
            self.pending_src = carry;
            self.pending_src_written = 0;
            self.drained = true;
            // A fresh resampler for whatever comes next at this rate.
            if let (Some(s), Some(o)) = (self.src_rate, self.out_rate) {
                self.resampler = Resampler::new(s, o, CHANNELS).ok();
            }
        }
    }

    /// Write as much pending audio as fits. Returns true once nothing is pending.
    pub fn write_pending(&mut self, producer: &mut Producer) -> bool {
        let rest = &self.pending[self.pending_off..];
        if rest.is_empty() {
            if self.drained && self.pending_src > self.pending_src_written {
                // The resampler's tail was empty, but frames still need accounting for:
                // they count as played once everything before them has been.
                self.marks.push_back(Mark {
                    end: producer.ring().write_pos(),
                    len: 0,
                    src_frames: self.pending_src - self.pending_src_written,
                    reported: 0,
                });
                self.pending_src_written = self.pending_src;
            }
            return true;
        }
        let w = producer.write(rest);
        if w > 0 {
            let total = self.pending.len() as u64;
            let before = self.pending_off as u64;
            self.pending_off += w;
            let after = self.pending_off as u64;
            // Source frames carried by this piece, proportionally (exact at the end).
            let src_after = if after == total {
                self.pending_src
            } else {
                self.pending_src * after / total
            };
            let src_before = self.pending_src_written;
            debug_assert!(src_after >= src_before && before < after);
            self.pending_src_written = src_after;
            self.marks.push_back(Mark {
                end: producer.ring().write_pos(),
                len: w as u64,
                src_frames: src_after - src_before,
                reported: 0,
            });
        }
        !self.has_pending()
    }

    /// Source frames newly played, now that the output has read up to `read_pos`.
    pub fn played(&mut self, read_pos: u64) -> u64 {
        let mut total = 0;
        while let Some(m) = self.marks.front_mut() {
            if read_pos >= m.end {
                total += m.src_frames - m.reported;
                self.marks.pop_front();
                continue;
            }
            let start = m.end - m.len;
            if read_pos > start {
                let done = m.src_frames * (read_pos - start) / m.len;
                total += done - m.reported;
                m.reported = done;
            }
            break;
        }
        total
    }

    /// Everything queued was dropped (flush): forget pending audio and filter state.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.pending_off = 0;
        self.pending_src = 0;
        self.pending_src_written = 0;
        self.marks.clear();
        self.drained = false;
        self.track = None;
        if let Some(c) = &mut self.chain {
            c.reset();
        }
        if let (Some(s), Some(o), true) = (self.src_rate, self.out_rate, self.resampler.is_some()) {
            self.resampler = Resampler::new(s, o, CHANNELS).ok();
        }
    }

    /// The pipeline's stages, for the signal path.
    pub fn stages(&self) -> Vec<Stage> {
        let mut v = match &self.chain {
            Some(c) => c.stages(),
            None => Vec::new(),
        };
        let source = match self.level_source {
            LevelSource::Analysis => Some("R128 analysis"),
            LevelSource::Tags => Some("ReplayGain"),
            LevelSource::None => None,
        };
        if let Some(source) = source {
            for s in v.iter_mut().filter(|s| s.name.starts_with("Levelling")) {
                s.name = format!("{} ({source})", s.name);
            }
        }
        if let Some(r) = &self.resampler {
            let (from, to) = r.rates();
            v.push(Stage::resample(from, to, "sinc"));
        }
        v
    }

    /// Any sample-changing processing in the pipeline itself?
    pub fn processing(&self) -> bool {
        self.chain.as_ref().is_some_and(|c| !c.is_bit_transparent()) || self.resampler.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::ring::Ring;

    fn noise(frames: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..frames * CHANNELS)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // 24-bit integer samples, as a lossless decoder produces them.
                ((x >> 8) as i32 - (1 << 23)) as f32 / 8_388_608.0
            })
            .collect()
    }

    fn drain(ring: &std::sync::Arc<Ring>, out: &mut Vec<f32>) {
        let mut c = ring.consumer().unwrap();
        let mut buf = vec![0f32; 1000];
        loop {
            let n = c.read(&mut buf);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    /// The bit-perfect guarantee: with DSP off and no resampling, the ring receives exactly
    /// the bits the core produced, at every common rate.
    #[test]
    fn bypassed_pipeline_is_bit_perfect() {
        for rate in [44_100, 48_000, 88_200, 96_000, 176_400, 192_000] {
            let ring = Ring::new(1 << 20);
            ring.set_hold(false);
            let mut prod = ring.producer().unwrap();
            let mut p = Pipeline::new(DspSettings::default());
            p.configure(rate, rate).unwrap();
            let input = noise(rate as usize / 10, rate);
            for chunk in input.chunks(2048) {
                p.push(chunk, 0, Some(&ReplayGainTags::default()));
                assert!(p.write_pending(&mut prod));
            }
            let mut got = Vec::new();
            drain(&ring, &mut got);
            assert_eq!(got.len(), input.len());
            assert!(
                got.iter()
                    .zip(&input)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{rate} Hz not bit-perfect"
            );
            assert!(!p.processing());
            assert!(p.stages().is_empty());
        }
    }

    #[test]
    fn dsp_off_ignores_tags_and_settings_are_sanitised() {
        let tags = ReplayGainTags {
            track_gain_db: Some(-6.0),
            ..Default::default()
        };
        let mut s = DspSettings {
            levelling: Levelling::Track,
            ..DspSettings::default()
        };
        assert!(!s.active(), "the master switch is off");
        s.enabled = true;
        assert!(s.active());
        assert_eq!(levelling_gain_db(&s, &tags), -6.0);
        let weird = DspSettings {
            enabled: true,
            preamp_db: f64::NAN,
            levelling: Levelling::Off,
            target_lufs: 1e9,
            limiter: false,
            analysis_levelling: true,
        }
        .sanitised();
        assert_eq!(weird.preamp_db, 0.0);
        assert_eq!(weird.target_lufs, -5.0);
        assert!(!weird.active());
    }

    #[test]
    fn levelling_follows_the_track_and_album_tags() {
        let s = DspSettings {
            enabled: true,
            levelling: Levelling::Album,
            limiter: true,
            ..DspSettings::default()
        };
        let both = ReplayGainTags {
            track_gain_db: Some(-3.0),
            track_peak: Some(0.5),
            album_gain_db: Some(-5.0),
            album_peak: Some(0.9),
        };
        assert_eq!(levelling_gain_db(&s, &both), -5.0);
        let track_only = ReplayGainTags {
            track_gain_db: Some(-3.0),
            ..Default::default()
        };
        assert_eq!(levelling_gain_db(&s, &track_only), -3.0);
        assert_eq!(levelling_gain_db(&s, &ReplayGainTags::default()), 0.0);
    }

    /// R128 levelling from the analysis: the gain is target − integrated loudness, capped
    /// by the true peak; tags are the fallback; the source shows in the signal path.
    #[test]
    fn analysis_levelling_gain_and_fallback() {
        let loud = Measurement::from_analysis(-9.0, -0.2).unwrap();
        let quiet = Measurement::from_analysis(-24.0, -12.0).unwrap();
        let tags = ReplayGainTags {
            track_gain_db: Some(-2.0),
            track_peak: Some(0.9),
            album_gain_db: Some(-4.0),
            album_peak: Some(0.95),
        };
        let mut s = DspSettings {
            enabled: true,
            levelling: Levelling::Track,
            target_lufs: -18.0,
            limiter: true,
            analysis_levelling: true,
            ..DspSettings::default()
        };
        // A loud master is turned down by exactly the difference.
        assert_eq!(
            levelling_decision(&s, Some(&tags), Some(loud)),
            (-9.0, LevelSource::Analysis)
        );
        // A quiet one is turned up: by 6 dB, within the peak's room plus the limiter's
        // 3 dB headroom (−12 dBTP + 6 dB stays under the −1 dBTP ceiling).
        assert_eq!(
            levelling_decision(&s, Some(&tags), Some(quiet)),
            (6.0, LevelSource::Analysis)
        );
        // Without the limiter, a boost never takes the true peak over the ceiling.
        let hot_quiet = Measurement::from_analysis(-24.0, -4.0).unwrap();
        s.limiter = false;
        let (g, _) = levelling_decision(&s, None, Some(hot_quiet));
        assert!((g - 3.0).abs() < 1e-9, "{g}");
        s.limiter = true;
        // No analysis: the tags (ReplayGain is referenced to −18 LUFS).
        assert_eq!(
            levelling_decision(&s, Some(&tags), None),
            (-2.0, LevelSource::Tags)
        );
        // Nothing at all: no levelling.
        assert_eq!(levelling_decision(&s, None, None), (0.0, LevelSource::None));
        // Analysis levelling off: the analysis is ignored.
        s.analysis_levelling = false;
        assert_eq!(
            levelling_decision(&s, Some(&tags), Some(loud)),
            (-2.0, LevelSource::Tags)
        );
        // Album mode prefers the album's tags; without them, the track's analysis.
        s.analysis_levelling = true;
        s.levelling = Levelling::Album;
        assert_eq!(
            levelling_decision(&s, Some(&tags), Some(loud)),
            (-4.0, LevelSource::Tags)
        );
        let track_tags = ReplayGainTags {
            track_gain_db: Some(-2.0),
            ..Default::default()
        };
        assert_eq!(
            levelling_decision(&s, Some(&track_tags), Some(loud)),
            (-9.0, LevelSource::Analysis)
        );
        // Levelling off: nothing, whatever is known.
        s.levelling = Levelling::Off;
        assert_eq!(
            levelling_decision(&s, Some(&tags), Some(loud)),
            (0.0, LevelSource::None)
        );
        // The target moves the gain one for one.
        s.levelling = Levelling::Track;
        s.target_lufs = -14.0;
        assert_eq!(levelling_decision(&s, None, Some(loud)).0, -5.0);
    }

    #[test]
    fn the_signal_path_names_the_levelling_source() {
        let mut p = Pipeline::new(DspSettings {
            enabled: true,
            levelling: Levelling::Track,
            limiter: false,
            analysis_levelling: true,
            ..DspSettings::default()
        });
        p.configure(44_100, 44_100).unwrap();
        let m = Measurement::from_analysis(-12.0, -1.5).unwrap();
        p.push_with(&[0.1, 0.1], 1, None, Some(m));
        assert_eq!(p.level_source(), LevelSource::Analysis);
        assert_eq!(p.levelling_db(), -6.0);
        let names: Vec<String> = p.stages().into_iter().map(|s| s.name).collect();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("Levelling") && n.ends_with("(R128 analysis)")),
            "{names:?}"
        );
        // The next track has only tags.
        p.pending.clear();
        p.pending_off = 0;
        let tags = ReplayGainTags {
            track_gain_db: Some(-3.0),
            ..Default::default()
        };
        p.push_with(&[0.1, 0.1], 2, Some(&tags), None);
        assert_eq!(p.level_source(), LevelSource::Tags);
        let names: Vec<String> = p.stages().into_iter().map(|s| s.name).collect();
        assert!(
            names.iter().any(|n| n.ends_with("(ReplayGain)")),
            "{names:?}"
        );
        // A track is decided once: a late analysis doesn't change it mid-track.
        p.pending.clear();
        p.pending_off = 0;
        p.push_with(&[0.1, 0.1], 2, Some(&tags), Some(m));
        assert_eq!(p.level_source(), LevelSource::Tags);
        assert_eq!(p.levelling_db(), -3.0);
    }

    #[test]
    fn active_dsp_processes_and_reports_its_stages() {
        let ring = Ring::new(1 << 16);
        ring.set_hold(false);
        let mut prod = ring.producer().unwrap();
        let mut p = Pipeline::new(DspSettings {
            enabled: true,
            preamp_db: -6.0,
            limiter: false,
            ..DspSettings::default()
        });
        p.configure(44_100, 44_100).unwrap();
        p.push(&[0.5, -0.5, 0.25, -0.25], 1, None);
        assert!(p.write_pending(&mut prod));
        let mut got = Vec::new();
        drain(&ring, &mut got);
        let g = 10f64.powf(-6.0 / 20.0);
        for (o, i) in got.iter().zip([0.5f64, -0.5, 0.25, -0.25]) {
            assert!((f64::from(*o) - i * g).abs() < 1e-6);
        }
        assert!(p.processing());
        assert_eq!(p.stages().len(), 1);
    }

    #[test]
    fn levelling_changes_at_the_track_boundary() {
        let ring = Ring::new(1 << 16);
        ring.set_hold(false);
        let mut prod = ring.producer().unwrap();
        let mut p = Pipeline::new(DspSettings {
            enabled: true,
            levelling: Levelling::Track,
            limiter: false,
            ..DspSettings::default()
        });
        p.configure(48_000, 48_000).unwrap();
        let quiet = ReplayGainTags {
            track_gain_db: Some(-6.0),
            ..Default::default()
        };
        p.push(&[0.5, 0.5], 1, Some(&quiet));
        p.write_pending(&mut prod);
        assert_eq!(p.levelling_db(), -6.0);
        p.push(&[0.5, 0.5], 2, Some(&ReplayGainTags::default()));
        p.write_pending(&mut prod);
        assert_eq!(p.levelling_db(), 0.0);
        let mut got = Vec::new();
        drain(&ring, &mut got);
        assert!(got[0] < 0.3);
        assert_eq!(got[2], 0.5, "no tags: untouched");
    }

    #[test]
    fn played_frames_are_reported_as_the_output_reads() {
        let ring = Ring::new(256);
        ring.set_hold(false);
        let mut prod = ring.producer().unwrap();
        let mut cons = ring.consumer().unwrap();
        let mut p = Pipeline::new(DspSettings::default());
        p.configure(44_100, 44_100).unwrap();
        p.push(&vec![0.1; 300], 0, None); // 150 frames, more than fits
        assert!(!p.write_pending(&mut prod));
        let mut buf = vec![0f32; 100];
        cons.read(&mut buf);
        assert_eq!(p.played(ring.read_pos()), 50);
        assert!(p.write_pending(&mut prod));
        let mut total = 50;
        loop {
            let n = cons.read(&mut buf);
            total += p.played(ring.read_pos());
            if n == 0 {
                break;
            }
        }
        assert_eq!(total, 150, "every source frame reported exactly once");
    }

    #[test]
    fn resampled_audio_still_reports_every_source_frame() {
        let ring = Ring::new(1 << 18);
        ring.set_hold(false);
        let mut prod = ring.producer().unwrap();
        let mut p = Pipeline::new(DspSettings::default());
        p.configure(44_100, 48_000).unwrap();
        let input = noise(44_100 / 2, 7);
        for chunk in input.chunks(4096) {
            p.push(chunk, 0, None);
            assert!(p.write_pending(&mut prod));
        }
        p.drain_resampler();
        assert!(p.write_pending(&mut prod));
        let frames_out = ring.write_pos() / 2;
        assert_eq!(frames_out, 24_000, "exact length after the tail");
        assert_eq!(p.played(ring.write_pos()), 44_100 / 2);
        assert!(p.processing());
        assert!(p.stages().iter().any(|s| s.resamples));
    }

    #[test]
    fn reset_forgets_pending_audio() {
        let ring = Ring::new(64 * CHANNELS);
        let mut prod = ring.producer().unwrap();
        let mut p = Pipeline::new(DspSettings::default());
        p.configure(44_100, 44_100).unwrap();
        p.push(&vec![0.1; 1000], 0, None);
        assert!(!p.write_pending(&mut prod));
        p.reset();
        assert!(!p.has_pending());
        assert_eq!(p.played(u64::MAX), 0);
    }
}
