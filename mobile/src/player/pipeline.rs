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
//!   mode), EQ, loudness-compensated volume, crossfeed and the true-peak limiter, from
//!   `core::dsp`, in `f64`. Levelling is decided per track: the core's
//!   [`Player::pull_track`] never mixes two tracks in one block.
//! - **Loudness compensation** follows the volume the core applied to the samples
//!   ([`Pipeline::set_volume`]) and, when the host knows it, the phone's own media volume
//!   ([`Pipeline::set_loudness_reference_db`]): the two attenuations add up. On a
//!   bit-perfect route ([`Pipeline::set_direct`]) the DAC's own volume rules, so it is
//!   bypassed, and the signal path says so.
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
use anywhere_core::dsp::crossfeed::CrossfeedPreset;
use anywhere_core::dsp::dither::{Dither, DitherMode, needs_dither};
use anywhere_core::dsp::eq::{self, EqProfile};
use anywhere_core::dsp::levelling::{
    self, LevellingConfig, LevellingMode, LoudnessInfo, Measurement,
};
use anywhere_core::dsp::limiter::LimiterConfig;
use anywhere_core::dsp::pcm;
use anywhere_core::dsp::resample::Resampler;
use anywhere_core::dsp::signal_path::Stage;

use super::ring::{CHANNELS, Producer};
use super::transitions::TransitionMode;

/// Loudness levelling from ReplayGain tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Levelling {
    Off,
    Track,
    /// Album gain where tagged, otherwise track gain.
    Album,
}

/// Headphone crossfeed (`core::dsp::crossfeed` presets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CrossfeedSetting {
    Off,
    Light,
    Moderate,
    Natural,
}

/// How samples are requantised when the bit depth is reduced (only then: a processed
/// stream into an integer output, or a source deeper than the DAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DitherSetting {
    /// TPDF dither (recommended).
    Tpdf,
    /// TPDF with first-order noise shaping.
    TpdfShaped,
    /// No dither: round to nearest.
    Off,
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
    /// True-peak limiter (recommended with any boost).
    pub limiter: bool,
    /// Level from the gateway's R128 analysis of each track (integrated loudness and true
    /// peak), falling back to ReplayGain tags where there is none. Needs `levelling`.
    #[uniffi(default = false)]
    pub analysis_levelling: bool,
    /// The limiter's ceiling, dBTP (−20..=0).
    #[uniffi(default = -1.0)]
    pub limiter_ceiling_db: f64,
    /// A parametric EQ as AutoEq `ParametricEQ.txt` text; `None` (or a flat, invalid or
    /// oversized profile) is no EQ. Check it first with [`check_eq_profile`].
    #[uniffi(default = None)]
    pub eq_profile: Option<String>,
    pub crossfeed: CrossfeedSetting,
    pub dither: DitherSetting,
    /// Crossfades between tracks (independent of `enabled`; off keeps playback gapless and
    /// bit-perfect). See [`super::transitions`].
    pub transitions: TransitionMode,
    /// Crossfade length, seconds (1..=10; the most a smart transition uses).
    #[uniffi(default = 6.0)]
    pub crossfade_s: f64,
    /// Loudness-compensated volume: bass and treble shelves that grow as the player's
    /// volume is turned down (see `core::dsp::loudness`). Bypassed on a bit-perfect route.
    #[uniffi(default = false)]
    pub loudness: bool,
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
            limiter_ceiling_db: -1.0,
            eq_profile: None,
            crossfeed: CrossfeedSetting::Off,
            dither: DitherSetting::Tpdf,
            transitions: TransitionMode::Off,
            crossfade_s: super::transitions::DEFAULT_FADE_S,
            loudness: false,
        }
    }
}

/// An EQ profile that parsed: what the app shows after an import.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EqProfileSummary {
    pub preamp_db: f64,
    /// Filters that are on.
    pub filters: u32,
    /// Every filter is 0 dB and the preamp is 0 dB: the EQ would change nothing.
    pub flat: bool,
}

/// Parse an AutoEq `ParametricEQ.txt` profile (pasted or from a file): its summary, or why
/// it was refused (with the line). Bounded: at most 64 KiB, 512-byte lines, 32 filters.
#[uniffi::export]
pub fn check_eq_profile(text: String) -> Result<EqProfileSummary, crate::ApiError> {
    let p = parse_eq(&text).map_err(crate::ApiError::invalid)?;
    Ok(EqProfileSummary {
        preamp_db: p.preamp_db,
        filters: p.filters.len() as u32,
        flat: p.is_flat(),
    })
}

fn parse_eq(text: &str) -> Result<EqProfile, String> {
    if text.len() > eq::MAX_PROFILE_BYTES {
        return Err(format!(
            "the profile is larger than {} KiB",
            eq::MAX_PROFILE_BYTES / 1024
        ));
    }
    eq::parse_autoeq(text).map_err(|e| e.to_string())
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
    ///
    /// Out-of-range numbers are clamped (non-finite ones reset to their defaults), and an
    /// EQ profile that doesn't parse, or is flat, is dropped: the EQ is then off rather
    /// than half-applied.
    pub fn sanitised(&self) -> DspSettings {
        let f = |x: f64, d: f64, lo: f64, hi: f64| if x.is_finite() { x.clamp(lo, hi) } else { d };
        DspSettings {
            enabled: self.enabled,
            preamp_db: f(self.preamp_db, 0.0, -20.0, 12.0),
            levelling: self.levelling,
            target_lufs: f(self.target_lufs, -18.0, -30.0, -5.0),
            limiter: self.limiter,
            analysis_levelling: self.analysis_levelling,
            limiter_ceiling_db: f(self.limiter_ceiling_db, -1.0, -20.0, 0.0),
            eq_profile: self
                .eq_profile
                .as_ref()
                .filter(|t| parse_eq(t).is_ok_and(|p| !p.is_flat()))
                .cloned(),
            crossfeed: self.crossfeed,
            dither: self.dither,
            transitions: self.transitions,
            crossfade_s: f(
                self.crossfade_s,
                super::transitions::DEFAULT_FADE_S,
                super::transitions::MIN_FADE_S,
                super::transitions::MAX_FADE_S,
            ),
            loudness: self.loudness,
        }
    }

    /// Does any stage change samples? (Dither alone doesn't: it only applies where the
    /// depth is reduced. The limiter is a safety net for the other stages, applied only
    /// with one of them.)
    pub fn active(&self) -> bool {
        self.active_on(false)
    }

    /// [`DspSettings::active`] on a route that is (`direct`) or isn't bit-perfect: there,
    /// loudness compensation alone doesn't switch the chain on.
    pub fn active_on(&self, direct: bool) -> bool {
        self.enabled
            && (self.preamp_db != 0.0
                || self.levelling != Levelling::Off
                || self.eq_profile.is_some()
                || self.crossfeed != CrossfeedSetting::Off
                || self.loudness_on(direct))
    }

    /// Loudness compensation applies: asked for, and not on a bit-perfect route.
    pub fn loudness_on(&self, direct: bool) -> bool {
        self.enabled && self.loudness && !direct
    }

    fn chain_config(&self, direct: bool) -> ChainConfig {
        let crossfeed = match self.crossfeed {
            CrossfeedSetting::Off => None,
            CrossfeedSetting::Light => Some(CrossfeedPreset::Light.config()),
            CrossfeedSetting::Moderate => Some(CrossfeedPreset::Moderate.config()),
            CrossfeedSetting::Natural => Some(CrossfeedPreset::Natural.config()),
        };
        ChainConfig {
            preamp_db: self.preamp_db,
            levelling_db: 0.0,
            eq: self.eq_profile.as_deref().and_then(|t| parse_eq(t).ok()),
            crossfeed,
            limiter: self.limiter.then(|| {
                LimiterConfig {
                    ceiling_dbtp: self.limiter_ceiling_db,
                    ..LimiterConfig::default()
                }
                .sanitised()
            }),
            dither: dither_mode(self.dither),
            loudness: self.loudness_on(direct),
        }
    }
}

fn dither_mode(d: DitherSetting) -> DitherMode {
    match d {
        DitherSetting::TpdfShaped => DitherMode::TpdfShaped,
        DitherSetting::Tpdf | DitherSetting::Off => DitherMode::Tpdf,
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

/// The signal-path stage when a bit-perfect route bypasses loudness compensation.
pub const LOUDNESS_BYPASSED: &str = "Loudness bypassed (bit-perfect output)";

/// Added to the loudness stage's name while it follows the phone's media volume.
pub const FOLLOWING_PHONE: &str = " (following the phone's volume)";

/// The most attenuation the phone's volume is taken to apply, dB (muted counts as this).
pub const MAX_REFERENCE_DB: f64 = 120.0;

/// The phone's media-volume attenuation, cleaned up: `None` (not known) stays `None`, NaN
/// is not known, +∞ (muted) and anything past [`MAX_REFERENCE_DB`] is the most, and a
/// negative value (a boost) counts as none.
pub fn sanitise_reference_db(db: Option<f64>) -> Option<f64> {
    let db = db?;
    if db.is_nan() {
        return None;
    }
    Some(db.clamp(0.0, MAX_REFERENCE_DB))
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
    /// Integer output depth (see [`Pipeline::set_output_bits`]).
    out_bits: Option<u32>,
    /// The current track's integer depth.
    source_bits: Option<u32>,
    out_dither: Dither,
    ints: Vec<i32>,
    /// The volume the source applied to the samples (linear), for loudness compensation.
    volume: f64,
    /// The output is a bit-perfect route (fixed volume): loudness compensation is bypassed.
    direct: bool,
    /// The phone's media volume as an attenuation (dB), when loudness follows it.
    reference_db: Option<f64>,
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
            out_bits: None,
            source_bits: None,
            out_dither: Dither::new(dither_mode(dsp.dither), CHANNELS),
            ints: Vec::new(),
            volume: 1.0,
            direct: false,
            reference_db: None,
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
        if dsp.dither != self.dsp.dither {
            self.out_dither = Dither::new(dither_mode(dsp.dither), CHANNELS);
        }
        self.dsp = dsp;
        self.chain = None;
        self.track = None; // re-decide levelling for the next block
        self.build_chain();
    }

    fn build_chain(&mut self) {
        self.chain = match (self.dsp.active_on(self.direct), self.src_rate) {
            (true, Some(rate)) => {
                Chain::new(self.dsp.chain_config(self.direct), rate, CHANNELS).ok()
            }
            _ => None,
        };
        let volume = self.loudness_volume();
        if let Some(c) = &mut self.chain {
            c.set_levelling_db(self.levelling_db);
            c.set_volume(volume);
        }
    }

    /// The volume loudness compensation follows: the source's, times the phone's media
    /// volume when known.
    fn loudness_volume(&self) -> f64 {
        match self.reference_db {
            Some(db) => self.volume * 10f64.powf(-db / 20.0),
            None => self.volume,
        }
    }

    /// The volume the source applied to the samples it hands over (linear, 1.0 = full):
    /// LMS's `audg` as the core applies it. Drives loudness compensation, which follows
    /// smoothly; call it before each [`Pipeline::push_counted`].
    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume;
        let v = self.loudness_volume();
        if let Some(c) = &mut self.chain {
            c.set_volume(v);
        }
    }

    /// The phone's media volume as an attenuation in dB (0 = full), for loudness
    /// compensation while this phone is the player; `None` when it isn't known (a
    /// Bluetooth headset with its own volume, say), so only the source's volume counts.
    /// Cleaned up by [`sanitise_reference_db`]. The compensation follows smoothly.
    pub fn set_loudness_reference_db(&mut self, db: Option<f64>) {
        self.reference_db = sanitise_reference_db(db);
        let v = self.loudness_volume();
        if let Some(c) = &mut self.chain {
            c.set_volume(v);
        }
    }

    /// The phone's media volume loudness follows, dB of attenuation (if any).
    pub fn loudness_reference_db(&self) -> Option<f64> {
        self.reference_db
    }

    /// The output is (`true`) or isn't a bit-perfect route. On one, the DAC's volume rules
    /// and loudness compensation is bypassed (the chain is rebuilt without it).
    pub fn set_direct(&mut self, direct: bool) {
        if direct == self.direct {
            return;
        }
        self.direct = direct;
        if self.dsp.enabled && self.dsp.loudness {
            self.chain = None;
            self.build_chain();
        }
    }

    /// Loudness compensation is asked for but bypassed by a bit-perfect route.
    pub fn loudness_bypassed(&self) -> bool {
        self.dsp.enabled && self.dsp.loudness && self.direct
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
        self.push_with(block, seq, tags, None, None);
    }

    /// [`Pipeline::push`], with the track's R128 analysis when known (used for levelling
    /// if `analysis_levelling` is on; decided once, at the track's first block) and the
    /// source's integer depth (`None`: lossy or float), which decides whether an integer
    /// output needs dither.
    pub fn push_with(
        &mut self,
        block: &[f32],
        seq: u32,
        tags: Option<&ReplayGainTags>,
        analysis: Option<Measurement>,
        source_bits: Option<u32>,
    ) {
        let n = (block.len() / CHANNELS) as u64;
        self.push_counted(block, seq, tags, analysis, source_bits, n);
    }

    /// [`Pipeline::push_with`] for a block that accounts for `src_frames` source frames
    /// rather than its own length: a crossfade carries both tracks' frames, and trimmed
    /// silence rides along with the next block (see [`super::transitions`]).
    pub fn push_counted(
        &mut self,
        block: &[f32],
        seq: u32,
        tags: Option<&ReplayGainTags>,
        analysis: Option<Measurement>,
        source_bits: Option<u32>,
        src_frames: u64,
    ) {
        debug_assert!(!self.has_pending());
        // Source frames that produced no output yet (held inside the resampler) ride along
        // with the next output, so every frame is eventually reported as played.
        let carry = self.pending_src - self.pending_src_written;
        self.pending.clear();
        self.pending_off = 0;
        let frames = src_frames + carry;
        self.drained = false;
        if self.track != Some(seq) {
            self.track = Some(seq);
            (self.levelling_db, self.level_source) = levelling_decision(&self.dsp, tags, analysis);
            if let Some(c) = &mut self.chain {
                c.set_levelling_db(self.levelling_db);
            }
        }
        self.source_bits = source_bits;
        let quantise = self.quantise_bits();
        if self.chain.is_none() && self.resampler.is_none() && quantise.is_none() {
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
            match quantise {
                // An integer output, and the depth is reduced: onto its grid, dithered
                // (exact in the f32 ring, so the output's conversion is lossless).
                Some(bits) => {
                    self.ints.resize(out.len(), 0);
                    if self.dsp.dither == DitherSetting::Off {
                        pcm::from_f64(out, bits, &mut self.ints);
                    } else {
                        self.out_dither.quantise(out, bits, &mut self.ints);
                    }
                    pcm::int_to_f32(&self.ints, bits, &mut self.pending);
                }
                None => pcm::f64_to_f32(out, &mut self.pending),
            }
        }
        self.pending_src = frames;
        self.pending_src_written = 0;
    }

    /// The output is an integer stream (a bit-perfect DAC) whose samples are exact at
    /// `bits` (≤ 24); `None` for float output. Processed or deeper audio is then dithered to
    /// that depth here, so the output only ever converts exact values.
    pub fn set_output_bits(&mut self, bits: Option<u32>) {
        let bits = bits.map(|b| b.clamp(16, pcm::F32_EXACT_BITS));
        if bits != self.out_bits {
            self.out_bits = bits;
            self.out_dither.reset();
        }
    }

    pub fn output_bits(&self) -> Option<u32> {
        self.out_bits
    }

    /// The depth audio is requantised to now, if any: an integer output and a reduction
    /// (processing, or a source that is lossy or deeper than the output).
    fn quantise_bits(&self) -> Option<u32> {
        let q = self.out_bits?;
        let processed = self.chain.as_ref().is_some_and(|c| !c.is_bit_transparent())
            || self.resampler.is_some();
        needs_dither(Some(q), self.source_bits, processed).then_some(q)
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
        self.out_dither.reset();
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
        if self.loudness_bypassed() {
            v.push(Stage::transparent(LOUDNESS_BYPASSED));
        } else if self.reference_db.is_some() {
            for s in v.iter_mut().filter(|s| s.name.starts_with("Loudness")) {
                s.name.push_str(FOLLOWING_PHONE);
            }
        }
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
        if let Some(bits) = self.quantise_bits() {
            v.push(Stage::processing(match self.dsp.dither {
                DitherSetting::Tpdf => format!("Dither to {bits}-bit (TPDF)"),
                DitherSetting::TpdfShaped => format!("Dither to {bits}-bit (TPDF, noise-shaped)"),
                DitherSetting::Off => format!("Rounded to {bits}-bit (no dither)"),
            }));
        }
        v
    }

    /// Any sample-changing processing in the pipeline itself?
    pub fn processing(&self) -> bool {
        self.chain.as_ref().is_some_and(|c| !c.is_bit_transparent())
            || self.resampler.is_some()
            || self.quantise_bits().is_some()
    }

    /// Our DSP chain (not resampling or requantising) changes samples.
    pub fn dsp_processing(&self) -> bool {
        self.chain.as_ref().is_some_and(|c| !c.is_bit_transparent())
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
            limiter_ceiling_db: f64::INFINITY,
            eq_profile: Some("Filter 1: ON PK Fc 99999999 Hz Gain 3 dB Q 1".into()),
            crossfeed: CrossfeedSetting::Off,
            dither: DitherSetting::Tpdf,
            transitions: TransitionMode::Smart,
            crossfade_s: f64::NAN,
            loudness: false,
        }
        .sanitised();
        assert_eq!(weird.crossfade_s, 6.0, "nonsense becomes the default");
        assert_eq!(weird.transitions, TransitionMode::Smart);
        assert_eq!(
            DspSettings {
                crossfade_s: 99.0,
                ..DspSettings::default()
            }
            .sanitised()
            .crossfade_s,
            10.0
        );
        assert_eq!(weird.preamp_db, 0.0);
        assert_eq!(weird.target_lufs, -5.0);
        assert_eq!(weird.limiter_ceiling_db, -1.0);
        assert_eq!(weird.eq_profile, None, "an invalid profile is dropped");
        assert!(!weird.active());
    }

    const PROFILE: &str = "Preamp: -6.4 dB\n\
        Filter 1: ON LSC Fc 105 Hz Gain 5.8 dB Q 0.70\n\
        Filter 2: ON PK Fc 3800 Hz Gain 4.1 dB Q 3.2\n\
        Filter 3: OFF PK Fc 6000 Hz Gain 2 dB Q 5\n";

    /// Out-of-range values are clamped, nonsense is reset, and invalid, flat or oversized
    /// EQ profiles are refused (the EQ is then off, never half-applied).
    #[test]
    fn settings_are_clamped_or_refused() {
        let base = DspSettings {
            enabled: true,
            ..DspSettings::default()
        };
        let s = DspSettings {
            preamp_db: -99.0,
            target_lufs: -99.0,
            limiter_ceiling_db: 6.0,
            ..base.clone()
        }
        .sanitised();
        assert_eq!(
            (s.preamp_db, s.target_lufs, s.limiter_ceiling_db),
            (-20.0, -30.0, 0.0)
        );
        let s = DspSettings {
            preamp_db: 99.0,
            limiter_ceiling_db: -99.0,
            ..base.clone()
        }
        .sanitised();
        assert_eq!((s.preamp_db, s.limiter_ceiling_db), (12.0, -20.0));
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let s = DspSettings {
                preamp_db: bad,
                target_lufs: bad,
                limiter_ceiling_db: bad,
                ..base.clone()
            }
            .sanitised();
            assert_eq!(
                (s.preamp_db, s.target_lufs, s.limiter_ceiling_db),
                (0.0, -18.0, -1.0)
            );
        }
        // EQ: a valid profile is kept and activates the chain; others are dropped.
        let with = |t: &str| {
            DspSettings {
                eq_profile: Some(t.to_string()),
                ..base.clone()
            }
            .sanitised()
        };
        let ok = with(PROFILE);
        assert_eq!(ok.eq_profile.as_deref(), Some(PROFILE));
        assert!(ok.active());
        assert_eq!(
            with("Filter 1: ON PK Fc 1000 Hz Gain 0 dB Q 1").eq_profile,
            None,
            "flat"
        );
        assert_eq!(with("garbage").eq_profile, None);
        assert_eq!(
            with("Filter 1: ON PK Fc 1000 Hz Gain 300 dB Q 1").eq_profile,
            None
        );
        let huge = format!(
            "{PROFILE}{}",
            "# padding\n".repeat(eq::MAX_PROFILE_BYTES / 10)
        );
        assert_eq!(with(&huge).eq_profile, None, "oversized");
        // The import check says why.
        let summary = check_eq_profile(PROFILE.into()).unwrap();
        assert_eq!((summary.filters, summary.flat), (2, false));
        assert_eq!(summary.preamp_db, -6.4);
        assert!(check_eq_profile(huge).is_err());
        let err = check_eq_profile("Preamp: -3 dB\nFilter 1: ON ZZ Fc 1 Hz".into()).unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
        // Each stage alone makes the chain active (bit-perfect goes); dither alone doesn't.
        assert!(!base.active(), "the default has nothing to do");
        for s in [
            DspSettings {
                preamp_db: -1.0,
                ..base.clone()
            },
            DspSettings {
                levelling: Levelling::Album,
                ..base.clone()
            },
            DspSettings {
                crossfeed: CrossfeedSetting::Light,
                ..base.clone()
            },
            ok.clone(),
        ] {
            assert!(s.active(), "{s:?}");
            assert!(
                !DspSettings {
                    enabled: false,
                    ..s
                }
                .active(),
                "the master switch"
            );
        }
        assert!(
            !DspSettings {
                dither: DitherSetting::TpdfShaped,
                ..base.clone()
            }
            .active()
        );
    }

    /// Hostile profile text through the settings path: never a panic, and whatever is kept
    /// builds a working chain.
    #[test]
    fn eq_text_fuzz_never_panics_and_kept_profiles_run() {
        let mut x = 0x5eed_u64;
        let mut rnd = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        let alphabet = b"Filter PreampONOFFPKLSCHSCLPHPNOFcGainQHzdB:0123456789.-+ \n\t#e";
        for i in 0..3_000 {
            let mut t = PROFILE.as_bytes().to_vec();
            if i % 2 == 0 {
                for _ in 0..1 + rnd(5) {
                    let k = rnd(t.len() as u64) as usize;
                    t[k] = alphabet[rnd(alphabet.len() as u64) as usize];
                }
            } else {
                t = (0..rnd(300)).map(|_| rnd(256) as u8).collect();
            }
            let text = String::from_utf8_lossy(&t).into_owned();
            let _ = check_eq_profile(text.clone());
            let s = DspSettings {
                enabled: true,
                eq_profile: Some(text),
                ..DspSettings::default()
            }
            .sanitised();
            if s.eq_profile.is_some() {
                let mut p = Pipeline::new(s);
                p.configure(48_000, 48_000).unwrap();
                p.push(&[0.5, -0.5, 0.25, 0.1], 0, None);
                assert!(p.pending.iter().all(|v| v.is_finite()));
            }
        }
    }

    /// Integer output (a bit-perfect DAC): untouched audio is copied as is, and processed
    /// or lossy audio is dithered onto the output's grid, so the output's conversion to
    /// integers is always exact.
    #[test]
    fn integer_output_copies_untouched_audio_and_dithers_the_rest() {
        let ring = Ring::new(1 << 18);
        ring.set_hold(false);
        let mut prod = ring.producer().unwrap();
        let mut p = Pipeline::new(DspSettings::default());
        p.configure(96_000, 96_000).unwrap();
        p.set_output_bits(Some(24));
        let input = noise(4096, 3);
        p.push_with(&input, 0, None, None, Some(24));
        assert!(!p.processing(), "24-bit into 24-bit: untouched");
        assert!(p.stages().is_empty());
        assert_eq!(p.pending, input);
        assert!(p.write_pending(&mut prod));
        // A lossy source into the same output: dithered to 24 bits.
        p.push_with(&[0.123_456_79, -0.333_333_34], 1, None, None, None);
        assert!(p.processing());
        assert!(
            p.stages()
                .iter()
                .any(|s| s.name == "Dither to 24-bit (TPDF)")
        );
        assert!(p.pending.iter().all(|v| pcm::f32_on_grid(*v, 24)));
        p.write_pending(&mut prod);
        // A 24-bit source into a 16-bit DAC: reduced with dither.
        p.set_output_bits(Some(16));
        p.push_with(&input, 2, None, None, Some(24));
        assert!(p.pending.iter().all(|v| pcm::f32_on_grid(*v, 16)));
        assert!(
            p.stages()
                .iter()
                .any(|s| s.name.starts_with("Dither to 16-bit"))
        );
        p.write_pending(&mut prod);
        // DSP on: processed audio is dithered (or rounded, if dither is off).
        let mut p = Pipeline::new(DspSettings {
            enabled: true,
            preamp_db: -3.0,
            limiter: false,
            dither: DitherSetting::Off,
            ..DspSettings::default()
        });
        p.configure(44_100, 44_100).unwrap();
        p.set_output_bits(Some(16));
        p.push_with(&[0.5, -0.5], 0, None, None, Some(16));
        let g = 10f64.powf(-3.0 / 20.0);
        assert_eq!(
            p.pending[0],
            ((0.5 * g * 32_768.0).round() / 32_768.0) as f32
        );
        assert!(
            p.stages()
                .iter()
                .any(|s| s.name == "Rounded to 16-bit (no dither)")
        );
        // Back to float output: no quantisation.
        p.set_output_bits(None);
        p.pending.clear();
        p.pending_off = 0;
        p.push_with(&[0.5, -0.5], 0, None, None, Some(16));
        assert_eq!(f64::from(p.pending[0]), f64::from((0.5 * g) as f32));
        // Out-of-range depths are clamped to what the f32 ring carries exactly.
        p.set_output_bits(Some(32));
        assert_eq!(p.output_bits(), Some(24));
        p.set_output_bits(Some(1));
        assert_eq!(p.output_bits(), Some(16));
    }

    /// Bit-exactness through the pipeline, ring and integer output, at 16/24/32-bit for
    /// 44.1–192 kHz: what the DAC receives is the source's integers.
    #[test]
    fn integer_path_is_bit_exact_at_every_rate_and_depth() {
        for rate in [44_100, 48_000, 88_200, 96_000, 176_400, 192_000] {
            for (src_bits, out_bits) in [(16, 16), (16, 24), (16, 32), (24, 24), (24, 32)] {
                let mut x = u64::from(rate) * 31 + u64::from(src_bits);
                let ints: Vec<i32> = (0..rate as usize / 20 * CHANNELS)
                    .map(|i| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let hi = (1i64 << (src_bits - 1)) - 1;
                        match i % 50 {
                            0 => hi as i32,
                            1 => (-hi - 1) as i32,
                            _ => ((x % (2 * hi as u64 + 2)) as i64 - hi - 1) as i32,
                        }
                    })
                    .collect();
                let mut f = vec![0f32; ints.len()];
                pcm::int_to_f32(&ints, src_bits, &mut f);
                let ring = Ring::new(1 << 20);
                ring.set_hold(false);
                let mut prod = ring.producer().unwrap();
                let mut cons = ring.consumer().unwrap();
                let mut p = Pipeline::new(DspSettings::default());
                p.configure(rate, rate).unwrap();
                p.set_output_bits(Some(out_bits.min(24)));
                for chunk in f.chunks(4096) {
                    p.push_with(chunk, 0, None, None, Some(src_bits));
                    assert!(p.write_pending(&mut prod));
                }
                assert!(!p.processing());
                let mut got = vec![0i32; ints.len()];
                let n = if out_bits == 16 {
                    let mut o = vec![0i16; ints.len()];
                    let n = cons.read_i16(&mut o);
                    got = o.into_iter().map(i32::from).collect();
                    n
                } else {
                    cons.read_i32(&mut got, out_bits)
                };
                assert_eq!(n, ints.len());
                let shift = out_bits - src_bits;
                for (k, (g, i)) in got.iter().zip(&ints).enumerate() {
                    assert_eq!(
                        *g,
                        i << shift,
                        "{src_bits}→{out_bits} at {rate} Hz, sample {k}"
                    );
                }
            }
        }
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
        p.push_with(&[0.1, 0.1], 1, None, Some(m), None);
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
        p.push_with(&[0.1, 0.1], 2, Some(&tags), None, None);
        assert_eq!(p.level_source(), LevelSource::Tags);
        let names: Vec<String> = p.stages().into_iter().map(|s| s.name).collect();
        assert!(
            names.iter().any(|n| n.ends_with("(ReplayGain)")),
            "{names:?}"
        );
        // A track is decided once: a late analysis doesn't change it mid-track.
        p.pending.clear();
        p.pending_off = 0;
        p.push_with(&[0.1, 0.1], 2, Some(&tags), Some(m), None);
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

    fn loudness_settings(eq: bool) -> DspSettings {
        DspSettings {
            enabled: true,
            loudness: true,
            eq_profile: eq.then(|| PROFILE.to_string()),
            ..DspSettings::default()
        }
    }

    /// Process `block` as one push and take what came out.
    fn run(p: &mut Pipeline, block: &[f32], seq: u32) -> Vec<f32> {
        p.pending.clear();
        p.pending_off = 0;
        p.push_with(block, seq, None, None, None);
        std::mem::take(&mut p.pending)
    }

    #[test]
    fn loudness_is_off_by_default_and_survives_sanitising() {
        assert!(!DspSettings::default().loudness);
        let s = loudness_settings(false);
        assert!(s.sanitised().loudness);
        assert!(s.active(), "loudness alone switches the chain on");
        assert!(!s.active_on(true), "but not on a bit-perfect route");
        assert!(
            !DspSettings {
                enabled: false,
                ..s
            }
            .active(),
            "the master switch rules"
        );
    }

    #[test]
    fn loudness_follows_the_volume_and_shows_in_the_signal_path() {
        let mut p = Pipeline::new(loudness_settings(false));
        p.configure(44_100, 44_100).unwrap();
        let names =
            |p: &Pipeline| -> Vec<String> { p.stages().into_iter().map(|s| s.name).collect() };
        assert_eq!(
            names(&p),
            [
                "Loudness (none at full volume)",
                "True-peak limiter −1.0 dBTP"
            ]
        );
        p.set_volume(0.1);
        assert_eq!(names(&p)[0], "Loudness +5.0 dB bass, +2.0 dB treble");
        assert!(p.dsp_processing());
        // Odd volumes from a confused source never break it.
        for v in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e-300] {
            p.set_volume(v);
            let out = run(&mut p, &noise(2048, 7), 1);
            assert!(out.iter().all(|x| x.is_finite()), "{v}");
        }
    }

    #[test]
    fn a_bit_perfect_route_bypasses_loudness_and_says_so() {
        let mut p = Pipeline::new(loudness_settings(false));
        p.configure(96_000, 96_000).unwrap();
        p.set_direct(true);
        assert!(p.loudness_bypassed());
        assert!(!p.processing() && !p.dsp_processing(), "no chain at all");
        let stages = p.stages();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].name, LOUDNESS_BYPASSED);
        assert!(stages[0].bit_transparent);
        // Even told of a low volume, the samples pass untouched.
        p.set_volume(0.05);
        let input = noise(4096, 3);
        assert_eq!(run(&mut p, &input, 1), input);

        // With an EQ too, the chain runs but without loudness.
        let mut p = Pipeline::new(loudness_settings(true));
        p.configure(96_000, 96_000).unwrap();
        p.set_direct(true);
        p.set_volume(0.05);
        let names: Vec<String> = p.stages().into_iter().map(|s| s.name).collect();
        assert!(
            !names.iter().any(|n| n.starts_with("Loudness +")),
            "{names:?}"
        );
        assert!(names.iter().any(|n| n == LOUDNESS_BYPASSED), "{names:?}");

        // Back on a mixed route, it applies again.
        p.set_direct(false);
        assert!(!p.loudness_bypassed());
        let names: Vec<String> = p.stages().into_iter().map(|s| s.name).collect();
        assert!(
            names.iter().any(|n| n.starts_with("Loudness +")),
            "{names:?}"
        );
    }

    /// Full-scale audio (the worst case: as if no volume had been applied) at the most
    /// compensation, with an EQ boosting the bass too, never clips.
    #[test]
    fn loudness_at_full_scale_never_clips_through_the_pipeline() {
        let ceiling = 10f32.powf(-1.0 / 20.0) + 1e-6;
        for eq in [false, true] {
            for vol in [1.0, 0.3, 0.01, 0.0] {
                let mut p = Pipeline::new(loudness_settings(eq));
                p.configure(48_000, 48_000).unwrap();
                p.set_volume(vol);
                let block: Vec<f32> = (0..4096 * CHANNELS)
                    .map(|i| if (i / 2 / 240) % 2 == 0 { 1.0 } else { -1.0 })
                    .collect();
                for k in 0..24 {
                    let out = run(&mut p, &block, 1);
                    let peak = out.iter().fold(0f32, |m, x| m.max(x.abs()));
                    assert!(peak <= ceiling, "eq {eq} vol {vol} block {k}: {peak}");
                }
            }
        }
    }

    #[test]
    fn volume_steps_through_the_pipeline_are_smooth() {
        let mut p = Pipeline::new(loudness_settings(false));
        p.configure(44_100, 44_100).unwrap();
        let mut out = Vec::new();
        // LMS volume steps, down and back up, on a steady low tone.
        for (k, vol) in [1.0, 0.5, 0.25, 0.1, 0.05, 0.1, 0.5, 1.0]
            .into_iter()
            .enumerate()
        {
            p.set_volume(vol);
            let block: Vec<f32> = (0..2048)
                .flat_map(|i| {
                    let t = (k * 2048 + i) as f64 / 44_100.0;
                    let s = (0.05 * (2.0 * std::f64::consts::PI * 50.0 * t).sin()) as f32;
                    [s, s]
                })
                .collect();
            out.extend(run(&mut p, &block, 1).into_iter().step_by(2));
        }
        let lat = p.chain.as_ref().unwrap().latency_frames() + 1;
        // A 50 Hz tone at 0.05 moves at most ~3.6e-4 a sample; boosted, at most ~3× that.
        let worst = out[lat..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0f32, f32::max);
        assert!(worst < 2e-3, "a step of {worst}");
    }

    #[test]
    fn loudness_follows_the_phones_volume_too() {
        let mut p = Pipeline::new(loudness_settings(false));
        p.configure(44_100, 44_100).unwrap();
        let first = |p: &Pipeline| p.stages()[0].name.clone();
        // Full player level, the phone 20 dB down: 20 dB in all, as 0.1 alone gives.
        p.set_loudness_reference_db(Some(20.0));
        assert_eq!(
            first(&p),
            format!("Loudness +5.0 dB bass, +2.0 dB treble{FOLLOWING_PHONE}")
        );
        // The player level adds to it (another 20 dB: 40 dB, the most).
        p.set_volume(0.1);
        assert_eq!(
            first(&p),
            format!("Loudness +10.0 dB bass, +4.0 dB treble{FOLLOWING_PHONE}")
        );
        // At full volume on both it is none, and still says what it follows.
        p.set_volume(1.0);
        p.set_loudness_reference_db(Some(0.0));
        assert_eq!(
            first(&p),
            format!("Loudness (none at full volume){FOLLOWING_PHONE}")
        );
        // Not known any more (a Bluetooth headset's own volume): the player level only.
        p.set_loudness_reference_db(None);
        p.set_volume(0.1);
        assert_eq!(first(&p), "Loudness +5.0 dB bass, +2.0 dB treble");
        assert_eq!(p.loudness_reference_db(), None);
    }

    #[test]
    fn the_phones_volume_survives_a_rebuilt_chain_and_odd_values() {
        let mut p = Pipeline::new(loudness_settings(false));
        p.set_loudness_reference_db(Some(20.0));
        // The chain is built after the reference was given.
        p.configure(48_000, 48_000).unwrap();
        assert!(p.stages()[0].name.starts_with("Loudness +5.0 dB"));
        // Changing the DSP rebuilds the chain; the reference stays.
        p.set_dsp(loudness_settings(true));
        assert!(
            p.stages()
                .iter()
                .any(|s| s.name.starts_with("Loudness +5.0 dB")),
            "{:?}",
            p.stages()
        );
        for (db, want) in [
            (Some(f64::NAN), None),
            (Some(f64::INFINITY), Some(MAX_REFERENCE_DB)),
            (Some(f64::NEG_INFINITY), Some(0.0)),
            (Some(-6.0), Some(0.0)),
            (Some(1e300), Some(MAX_REFERENCE_DB)),
            (Some(12.5), Some(12.5)),
            (None, None),
        ] {
            p.set_loudness_reference_db(db);
            assert_eq!(p.loudness_reference_db(), want, "{db:?}");
            let out = run(&mut p, &noise(2048, 11), 1);
            assert!(out.iter().all(|x| x.is_finite()), "{db:?}");
        }
    }

    #[test]
    fn a_bit_perfect_route_ignores_the_phones_volume() {
        let mut p = Pipeline::new(loudness_settings(false));
        p.configure(96_000, 96_000).unwrap();
        p.set_direct(true);
        p.set_loudness_reference_db(Some(30.0));
        let stages = p.stages();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].name, LOUDNESS_BYPASSED);
        let input = noise(4096, 5);
        assert_eq!(run(&mut p, &input, 1), input);
    }
}
