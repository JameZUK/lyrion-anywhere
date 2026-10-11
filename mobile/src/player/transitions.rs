//! Smart transitions: crossfades between unrelated tracks, gapless inside albums.
//!
//! Off by default: the engine then hands every sample through untouched, gapless and
//! bit-perfect. When on, this sits between a source ([`AudioSource`]) and the pipeline:
//!
//! ```text
//!   source ─► hold (the last few seconds of the current track) ─► pipeline ─► ring
//!                    └── at a track boundary: trim silence, mix the overlap ──┘
//! ```
//!
//! - **Look-ahead without knowing lengths.** A streamed track's length isn't known until it
//!   ends, so the last `keep` frames of the current track are held back. When the next
//!   track's audio appears, the end of the current one is in the hold, ready to mix.
//! - **Planning is pure** ([`plan`]): the two tracks' facts (album, duration, the server's
//!   analysis of silence and fades) and the setting decide between gapless and a crossfade,
//!   and its trims and length. Consecutive tracks of the same album always stay gapless,
//!   and so does a boundary we can't judge (an unknown album, a change of sample rate).
//! - **Never at the cost of a drop-out.** While the hold waits for the next track (or for
//!   its facts), the ring keeps playing; if it runs low, the hold is released and the
//!   boundary is played gapless.
//! - **Played accounting stays exact.** Every source frame is reported as played exactly
//!   once: a mixed block carries the frames of both tracks, and trimmed silence rides along
//!   with the next block (see [`Pulled::src_frames`]).
//!
//! The mixing itself is the core's sample-accurate [`TransitionMixer`] (equal power).

use std::collections::VecDeque;

use anywhere_core::dsp::transition::{FadeCurve, Transition, TransitionMixer};
use anywhere_core::player::TrackAudio;

use super::audio::AudioSource;
use super::ring::CHANNELS;

/// Shortest and longest crossfade the setting allows, seconds.
pub const MIN_FADE_S: f64 = 1.0;
pub const MAX_FADE_S: f64 = 10.0;
/// The default crossfade, seconds.
pub const DEFAULT_FADE_S: f64 = 6.0;
/// Most silence trimmed at either end of a track, seconds.
pub const MAX_TRIM_S: f64 = 5.0;
/// A crossfade shorter than this isn't worth it: gapless instead.
const MIN_OVERLAP_S: f64 = 0.1;
/// The shortest natural crossfade between two abrupt ends (smart mode).
const ABRUPT_FADE_S: f64 = 2.0;
/// Blocks pulled from the source into the hold per call (bounds the work per tick).
const TOP_UP_BLOCKS: usize = 16;

/// How tracks are joined on this phone's player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransitionMode {
    /// Gapless, bit-perfect (the default).
    Off,
    /// A fixed crossfade between tracks of different albums.
    Crossfade,
    /// Like `Crossfade`, but shaped by the server's analysis: silence at the ends is
    /// trimmed and the fade follows each track's own fade-out or fade-in.
    Smart,
}

/// The transition settings, as the engine applies them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionConfig {
    pub mode: TransitionMode,
    /// Crossfade length (the most a smart transition uses), seconds.
    pub fade_s: f64,
}

impl Default for TransitionConfig {
    fn default() -> Self {
        TransitionConfig {
            mode: TransitionMode::Off,
            fade_s: DEFAULT_FADE_S,
        }
    }
}

impl TransitionConfig {
    pub fn sanitised(self) -> Self {
        TransitionConfig {
            mode: self.mode,
            fade_s: if self.fade_s.is_finite() {
                self.fade_s.clamp(MIN_FADE_S, MAX_FADE_S)
            } else {
                DEFAULT_FADE_S
            },
        }
    }

    pub fn enabled(&self) -> bool {
        self.mode != TransitionMode::Off
    }

    /// Seconds kept back at the end of each track: the longest overlap plus, for smart
    /// transitions, the most silence that may be trimmed.
    fn keep_s(&self) -> f64 {
        let c = self.sanitised();
        match c.mode {
            TransitionMode::Off => 0.0,
            TransitionMode::Crossfade => c.fade_s,
            TransitionMode::Smart => c.fade_s + MAX_TRIM_S,
        }
    }

    /// The signal path's description, e.g. "Crossfade 6 s between albums".
    pub fn describe(&self) -> Option<String> {
        let c = self.sanitised();
        let secs = format_secs(c.fade_s);
        match c.mode {
            TransitionMode::Off => None,
            TransitionMode::Crossfade => Some(format!("Crossfade {secs} (not within albums)")),
            TransitionMode::Smart => Some(format!(
                "Smart transitions, up to {secs} (not within albums)"
            )),
        }
    }
}

fn format_secs(s: f64) -> String {
    if (s - s.round()).abs() < 0.05 {
        format!("{} s", s.round() as i64)
    } else {
        format!("{s:.1} s")
    }
}

/// The server's analysis of a track's ends (`/v1/analysis`), as transitions use it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeFacts {
    pub leading_silence_s: f64,
    pub trailing_silence_s: f64,
    /// The length of a deliberate fade-in, if the track has one.
    pub fade_in_s: Option<f64>,
    /// The length of a deliberate fade-out, if the track has one.
    pub fade_out_s: Option<f64>,
}

/// What the planner knows about one track.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackFacts {
    /// The album it belongs to (LMS's album id, or the download's album reference).
    pub album: Option<String>,
    pub duration_s: Option<f64>,
    /// From the server's analysis; `None` without one.
    pub edges: Option<EdgeFacts>,
}

/// Why a boundary stays gapless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaplessWhy {
    /// Transitions are off.
    Off,
    /// Both tracks are from the same album (gapless by design).
    SameAlbum,
    /// We can't tell whether they share an album (so we don't risk it).
    Unknown,
    /// The sample rate changes (the output is reopened at the boundary).
    RateChange,
    /// The tracks are too short to overlap.
    TooShort,
}

/// How to join two tracks, in frames at their (common) rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    Gapless(GaplessWhy),
    Crossfade {
        /// Silence dropped from the end of the outgoing track.
        trim_out: u64,
        /// Silence dropped from the start of the incoming track.
        trim_in: u64,
        /// Frames the two tracks overlap.
        overlap: u64,
    },
}

/// A finite, non-negative number of seconds, at most `cap` (anything else is 0).
fn secs(v: f64, cap: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v.min(cap)
    } else {
        0.0
    }
}

fn frames(s: f64, rate: u32) -> u64 {
    (s * f64::from(rate)).round().max(0.0) as u64
}

/// Plan the join of `out` (ending) and `inc` (starting). Pure.
///
/// - Off, a change of rate, unknown facts or an unknown album, or the same album: gapless.
/// - Smart with both tracks analysed: trim the silence at the boundary (up to
///   [`MAX_TRIM_S`] each side), and overlap for the outgoing track's fade-out, else the
///   incoming one's fade-in, else a short blend of two abrupt ends, never longer than the
///   setting.
/// - Otherwise (a fixed crossfade, or no analysis): overlap for the setting's length.
///
/// The overlap never exceeds half of either track (after trimming).
pub fn plan(
    cfg: TransitionConfig,
    out: Option<&TrackFacts>,
    inc: Option<&TrackFacts>,
    rate_out: u32,
    rate_in: u32,
) -> Join {
    let cfg = cfg.sanitised();
    if !cfg.enabled() {
        return Join::Gapless(GaplessWhy::Off);
    }
    if rate_out != rate_in || rate_out == 0 {
        return Join::Gapless(GaplessWhy::RateChange);
    }
    let (Some(out), Some(inc)) = (out, inc) else {
        return Join::Gapless(GaplessWhy::Unknown);
    };
    match (&out.album, &inc.album) {
        (Some(a), Some(b)) if a == b => return Join::Gapless(GaplessWhy::SameAlbum),
        (Some(_), Some(_)) => {}
        _ => return Join::Gapless(GaplessWhy::Unknown),
    }
    let (mut trim_out, mut trim_in, mut overlap) = (0.0, 0.0, cfg.fade_s);
    if let (TransitionMode::Smart, Some(eo), Some(ei)) = (cfg.mode, out.edges, inc.edges) {
        trim_out = secs(eo.trailing_silence_s, MAX_TRIM_S);
        trim_in = secs(ei.leading_silence_s, MAX_TRIM_S);
        let fade_out = eo
            .fade_out_s
            .map(|f| secs(f, MAX_FADE_S))
            .filter(|f| *f > 0.0);
        let fade_in = ei
            .fade_in_s
            .map(|f| secs(f, MAX_FADE_S))
            .filter(|f| *f > 0.0);
        overlap = fade_out
            .or(fade_in)
            .unwrap_or(ABRUPT_FADE_S)
            .clamp(MIN_FADE_S, cfg.fade_s);
    }
    // Never more than half of either track (what is left of it once trimmed).
    for (d, trim) in [(out.duration_s, trim_out), (inc.duration_s, trim_in)] {
        if let Some(d) = d.filter(|d| d.is_finite() && *d >= 0.0) {
            overlap = overlap.min(((d - trim).max(0.0)) / 2.0);
        }
    }
    if overlap < MIN_OVERLAP_S {
        return Join::Gapless(GaplessWhy::TooShort);
    }
    Join::Crossfade {
        trim_out: frames(trim_out, rate_out),
        trim_in: frames(trim_in, rate_out),
        overlap: frames(overlap, rate_out),
    }
}

/// The gain (as a factor) that brings the outgoing track's tail from the incoming track's
/// levelling to its own: the pipeline applies one gain per block, and mixed blocks belong
/// to the incoming track.
pub fn tail_correction(out_db: f64, in_db: f64) -> f32 {
    let d = out_db - in_db;
    if d.is_finite() {
        10f64.powf(d.clamp(-60.0, 60.0) / 20.0) as f32
    } else {
        1.0
    }
}

/// One block handed on to the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pulled {
    /// The track the block belongs to (the incoming one, for a mixed block).
    pub seq: u32,
    /// Frames in the block.
    pub frames: usize,
    /// Source frames the block accounts for: its own, plus the other track's frames mixed
    /// into it, plus any trimmed silence before it.
    pub src_frames: u64,
}

/// An overlap being mixed.
#[derive(Debug)]
struct Mix {
    in_seq: u32,
    /// The outgoing track's last frames (interleaved), already level-corrected.
    tail: VecDeque<f32>,
    mixer: TransitionMixer,
    /// Incoming frames still to skip (leading silence).
    skip_in: u64,
}

/// What the engine needs to know to plan a boundary.
pub(crate) trait TrackFactsSource {
    /// The facts of track `seq`, once known.
    fn facts(&self, seq: u32) -> Option<TrackFacts>;
    /// The levelling gain the pipeline will apply to track `seq` (whose audio details are
    /// `audio`), dB.
    fn level_db(&self, seq: u32, audio: Option<&TrackAudio>) -> f64;
}

/// The hold and the mixing state (see the module docs).
#[derive(Debug)]
pub(crate) struct Transitions {
    cfg: TransitionConfig,
    /// The current track's held-back audio (interleaved stereo).
    hold: VecDeque<f32>,
    hold_seq: Option<u32>,
    hold_rate: u32,
    mix: Option<Mix>,
    /// Source frames consumed without output (trimmed), reported with the next block.
    carry: u64,
    /// Scratch for the incoming track's frames.
    scratch: Vec<f32>,
    /// The last boundary's plan (tests and the signal path).
    last_join: Option<Join>,
    /// The held track whose end was decided gapless: play it out without asking again.
    gapless_for: Option<u32>,
}

impl Transitions {
    pub fn new(cfg: TransitionConfig) -> Self {
        Transitions {
            cfg: cfg.sanitised(),
            hold: VecDeque::new(),
            hold_seq: None,
            hold_rate: 0,
            mix: None,
            carry: 0,
            scratch: Vec::new(),
            last_join: None,
            gapless_for: None,
        }
    }

    pub fn config(&self) -> TransitionConfig {
        self.cfg
    }

    /// New settings (from the next boundary on; a hold already built is played out).
    pub fn set_config(&mut self, cfg: TransitionConfig) {
        self.cfg = cfg.sanitised();
    }

    /// Anything held or being mixed?
    pub fn is_empty(&self) -> bool {
        self.hold.is_empty() && self.mix.is_none() && self.carry == 0
    }

    /// Should audio go through here? (Off with nothing held: the plain path.)
    pub fn active(&self) -> bool {
        self.cfg.enabled() || !self.is_empty()
    }

    #[cfg(test)]
    pub fn last_join(&self) -> Option<Join> {
        self.last_join
    }

    /// Frames held now.
    #[cfg(test)]
    pub fn held_frames(&self) -> usize {
        self.hold.len() / CHANNELS + self.mix.as_ref().map_or(0, |m| m.tail.len() / CHANNELS)
    }

    /// Forget everything (a flush: a seek, a skip, a stop).
    pub fn reset(&mut self) {
        self.hold.clear();
        self.hold_seq = None;
        self.hold_rate = 0;
        self.mix = None;
        self.carry = 0;
        self.gapless_for = None;
    }

    fn keep_frames(&self) -> usize {
        if !self.cfg.enabled() {
            return 0;
        }
        frames(self.cfg.keep_s(), self.hold_rate.max(8_000)) as usize
    }

    /// The audio the next [`Transitions::pull`] hands on.
    pub fn next_audio(&self, src: &dyn AudioSource) -> Option<TrackAudio> {
        if let Some(m) = &self.mix {
            return src.track_audio(m.in_seq);
        }
        if let Some(seq) = self.hold_seq.filter(|_| !self.hold.is_empty()) {
            return src.track_audio(seq);
        }
        src.next_audio()
    }

    /// Nothing more to hand on until another track starts.
    pub fn exhausted(&self, src: &dyn AudioSource) -> bool {
        self.is_empty() && src.audio_exhausted()
    }

    /// Pull ahead from the source into the hold (while there is room).
    pub fn top_up(&mut self, src: &mut dyn AudioSource, block: &mut [f32]) {
        if !self.cfg.enabled() || self.mix.is_some() {
            return;
        }
        for _ in 0..TOP_UP_BLOCKS {
            let Some(next) = src.next_audio() else { return };
            match self.hold_seq {
                Some(s) if s != next.seq && !self.hold.is_empty() => return, // a boundary
                _ => {}
            }
            if self.hold.is_empty() {
                self.hold_seq = Some(next.seq);
                self.hold_rate = next.sample_rate;
            }
            if self.hold.len() / CHANNELS >= self.keep_frames() + block.len() / CHANNELS {
                return;
            }
            let Some((seq, n)) = src.pull_track(block) else {
                return;
            };
            if Some(seq) != self.hold_seq {
                // Shouldn't happen (next_audio said otherwise); keep accounting exact.
                self.hold_seq = Some(seq);
            }
            self.hold.extend(block[..n * CHANNELS].iter().copied());
        }
    }

    /// Hand on the next block (into `out`), or `None` for now. `low`: the output is about
    /// to run dry, so nothing may wait any longer.
    pub fn pull(
        &mut self,
        src: &mut dyn AudioSource,
        out: &mut [f32],
        info: &dyn TrackFactsSource,
        low: bool,
    ) -> Option<Pulled> {
        let want = out.len() / CHANNELS;
        if want == 0 {
            return None;
        }
        if self.mix.is_some() {
            // The outgoing track's audio before the overlap goes first.
            if !self.hold.is_empty() {
                let n = self.hold.len() / CHANNELS;
                return Some(self.release(out, n));
            }
            return self.pull_mix(src, out, low);
        }
        if self.hold.is_empty() {
            if !self.cfg.enabled() {
                return self.plain(src, out);
            }
            self.top_up(src, out);
            if self.hold.is_empty() {
                return None;
            }
        }
        let keep = self.keep_frames();
        let held = self.hold.len() / CHANNELS;
        if held > keep {
            return Some(self.release(out, held - keep));
        }
        let seq = self.hold_seq?;
        if self.gapless_for == Some(seq) {
            return Some(self.release(out, held));
        }
        // Has the held track ended? (The source has moved on, or has nothing more.)
        let next = src.next_audio();
        let ended = match &next {
            Some(n) => n.seq != seq,
            None => src.audio_exhausted(),
        };
        if !ended {
            // Still waiting for more of this track: release only if the output needs it.
            return low.then(|| self.release(out, held));
        }
        let Some(next) = next else {
            // The end of everything: play the rest out.
            return Some(self.release(out, held));
        };
        let facts_out = info.facts(seq);
        let facts_in = info.facts(next.seq);
        if self.cfg.enabled() && facts_in.is_none() && !low {
            return None; // wait (briefly) for the next track's facts
        }
        let join = plan(
            self.cfg,
            facts_out.as_ref(),
            facts_in.as_ref(),
            self.hold_rate,
            next.sample_rate,
        );
        self.last_join = Some(join);
        match join {
            Join::Gapless(_) => {
                self.gapless_for = Some(seq);
                Some(self.release(out, held))
            }
            Join::Crossfade {
                trim_out,
                trim_in,
                overlap,
            } => {
                // Fit to what is held (a short track, or a source that never got ahead).
                let trim_out = (trim_out as usize).min(held);
                let overlap = (overlap as usize).min(held - trim_out);
                if (overlap as f64) < MIN_OVERLAP_S * f64::from(self.hold_rate) {
                    return Some(self.release(out, held));
                }
                // Drop the trailing silence (its frames still count as played).
                for _ in 0..trim_out * CHANNELS {
                    self.hold.pop_back();
                }
                self.carry += trim_out as u64;
                let before = held - trim_out - overlap;
                let gain = tail_correction(
                    info.level_db(seq, src.track_audio(seq).as_ref()),
                    info.level_db(next.seq, Some(&next)),
                );
                let tail: VecDeque<f32> = self
                    .hold
                    .drain(before * CHANNELS..)
                    .map(|x| if gain == 1.0 { x } else { x * gain })
                    .collect();
                self.mix = Some(Mix {
                    in_seq: next.seq,
                    tail,
                    mixer: TransitionMixer::new(
                        Transition::Crossfade {
                            frames: overlap,
                            curve: FadeCurve::EqualPower,
                        },
                        CHANNELS,
                    ),
                    skip_in: trim_in,
                });
                if before > 0 {
                    return Some(self.release(out, before));
                }
                self.pull_mix(src, out, low)
            }
        }
    }

    /// The plain path (transitions off, nothing held): straight from the source.
    fn plain(&mut self, src: &mut dyn AudioSource, out: &mut [f32]) -> Option<Pulled> {
        let (seq, n) = src.pull_track(out)?;
        Some(Pulled {
            seq,
            frames: n,
            src_frames: n as u64 + std::mem::take(&mut self.carry),
        })
    }

    /// Hand on up to `max` frames from the front of the hold.
    fn release(&mut self, out: &mut [f32], max: usize) -> Pulled {
        let n = max
            .min(out.len() / CHANNELS)
            .min(self.hold.len() / CHANNELS);
        for (d, s) in out.iter_mut().zip(self.hold.drain(..n * CHANNELS)) {
            *d = s;
        }
        let seq = self.hold_seq.unwrap_or(0);
        if self.hold.is_empty() {
            self.hold_seq = None;
            self.gapless_for = None;
        }
        Pulled {
            seq,
            frames: n,
            src_frames: n as u64 + std::mem::take(&mut self.carry),
        }
    }

    /// Mix the next frames of the overlap.
    fn pull_mix(
        &mut self,
        src: &mut dyn AudioSource,
        out: &mut [f32],
        low: bool,
    ) -> Option<Pulled> {
        let mix = self.mix.as_mut()?;
        // Skip the incoming track's leading silence first.
        while mix.skip_in > 0 {
            match src.next_audio() {
                Some(a) if a.seq == mix.in_seq => {}
                _ => break,
            }
            let n = (mix.skip_in as usize).min(out.len() / CHANNELS);
            let Some((seq, got)) = src.pull_track(&mut out[..n * CHANNELS]) else {
                break;
            };
            debug_assert_eq!(seq, mix.in_seq);
            mix.skip_in -= got as u64;
            self.carry += got as u64;
        }
        let in_seq = mix.in_seq;
        let remaining = mix.tail.len() / CHANNELS;
        let incoming_here = matches!(src.next_audio(), Some(a) if a.seq == in_seq);
        let mut got = 0;
        if mix.skip_in == 0 && incoming_here {
            let want = remaining.min(out.len() / CHANNELS);
            self.scratch.resize(want * CHANNELS, 0.0);
            if let Some((seq, n)) = src.pull_track(&mut self.scratch) {
                debug_assert_eq!(seq, in_seq);
                got = n;
            }
        }
        if got == 0 {
            // The incoming track has nothing (yet). If the output is about to run dry, or
            // the incoming track ended before the overlap did, finish the fade alone.
            let gone = !incoming_here && (src.next_audio().is_some() || src.audio_exhausted());
            if !(low || gone) {
                return None;
            }
            let n = remaining.min(out.len() / CHANNELS);
            self.scratch.clear();
            self.scratch.resize(n * CHANNELS, 0.0);
            got = n;
            // Silence stands in for the missing incoming audio: its frames aren't source
            // frames, so only the outgoing ones are counted below.
            let tail: Vec<f32> = mix.tail.drain(..n * CHANNELS).collect();
            let mut f64_out = vec![0.0f64; n * CHANNELS];
            let a: Vec<f64> = tail.iter().map(|x| f64::from(*x)).collect();
            let b = vec![0.0f64; n * CHANNELS];
            mix.mixer.mix(&a, &b, &mut f64_out);
            for (d, s) in out.iter_mut().zip(&f64_out) {
                *d = *s as f32;
            }
            let src_frames = n as u64 + std::mem::take(&mut self.carry);
            if mix.tail.is_empty() {
                self.mix = None;
            }
            return Some(Pulled {
                seq: in_seq,
                frames: got,
                src_frames,
            });
        }
        let tail: Vec<f64> = mix.tail.drain(..got * CHANNELS).map(f64::from).collect();
        let inc: Vec<f64> = self.scratch[..got * CHANNELS]
            .iter()
            .map(|x| f64::from(*x))
            .collect();
        let mut mixed = vec![0.0f64; got * CHANNELS];
        mix.mixer.mix(&tail, &inc, &mut mixed);
        for (d, s) in out.iter_mut().zip(&mixed) {
            *d = *s as f32;
        }
        if mix.tail.is_empty() {
            self.mix = None;
        }
        Some(Pulled {
            seq: in_seq,
            frames: got,
            // Both tracks' frames are in this block.
            src_frames: 2 * got as u64 + std::mem::take(&mut self.carry),
        })
    }
}

#[cfg(test)]
mod tests;
