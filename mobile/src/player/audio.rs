//! The audio half of the engine, shared by both playback sources (plan §3.2, SPEC D14):
//! the LMS stream (the core's Slimproto player) and the local queue of downloaded files.
//!
//! ```text
//!   AudioSource (LMS stream | local files) ─► pipeline (DSP, levelling, resampler)
//!                                              ─► ring ─► output (AAudio callback)
//!                                    negotiation (bit-perfect USB) ┘   signal path ┘
//! ```
//!
//! Everything from decoded PCM to the device lives here once, so DSP, levelling, the
//! bit-perfect route, gapless playback across tracks of the same format and the signal path
//! behave identically whichever source feeds it.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anywhere_core::decode::SourceFormat;
use anywhere_core::dsp::levelling::Measurement;
use anywhere_core::player::{Player, TrackAudio};

use super::OutputRoute;
use super::SignalPathInfo;
use super::negotiate::{
    self, MixerOffer, NotBitPerfect, OutputNegotiator, OutputOffers, PcmEncoding, Plan, RouteState,
    SourceSpec,
};
use super::output::{AudioOutput, OutputEvent};
use super::pipeline::{DspSettings, Pipeline};
use super::protocol;
use super::ring::{CHANNELS, Producer, Ring};
use super::signal;
use super::transitions::{TrackFacts, TrackFactsSource, Transitions};
use crate::error::ApiError;

/// Audio queued in the ring ahead of the output. Enough to ride out a slow tick, small
/// enough that pause and skip feel immediate (pause also holds the ring at once).
pub(crate) const RING_AHEAD: Duration = Duration::from_millis(400);
/// Frames pulled from the source per block.
const BLOCK_FRAMES: usize = 4096;
/// Retry opening a failed output this often.
const OUTPUT_RETRY: Duration = Duration::from_secs(2);
/// Rates tried when the output can't take the source's (the engine then resamples).
const FALLBACK_RATES: [u32; 2] = [48_000, 44_100];

/// Where decoded PCM comes from. Both sources hand over interleaved stereo `f32` in
/// segments, one per track, numbered by a sequence (`seq`), never crossing a track boundary
/// in one pull, so per-track processing (levelling) and the output format can follow.
pub(crate) trait AudioSource {
    /// The audio the next pull returns (the first segment not yet fully pulled).
    fn next_audio(&self) -> Option<TrackAudio>;
    /// Fill `out` with up to `out.len() / 2` frames of one track; `None` if nothing.
    fn pull_track(&mut self, out: &mut [f32]) -> Option<(u32, usize)>;
    /// Format details of a track still queued.
    fn track_audio(&self, seq: u32) -> Option<TrackAudio>;
    /// The track audible now.
    fn playing_track(&self) -> Option<u32>;
    /// Nothing more to pull until another track starts.
    fn audio_exhausted(&self) -> bool;
    /// The volume applied to pulled samples (unity: untouched).
    fn applied_volume(&self) -> (f32, f32);
    /// Ignore any volume (a bit-perfect route: the DAC's volume rules).
    fn set_fixed_volume(&mut self, fixed: bool);
}

impl AudioSource for Player {
    fn next_audio(&self) -> Option<TrackAudio> {
        Player::next_audio(self)
    }

    fn pull_track(&mut self, out: &mut [f32]) -> Option<(u32, usize)> {
        Player::pull_track(self, out)
    }

    fn track_audio(&self, seq: u32) -> Option<TrackAudio> {
        Player::track_audio(self, seq)
    }

    fn playing_track(&self) -> Option<u32> {
        Player::playing_track(self)
    }

    fn audio_exhausted(&self) -> bool {
        Player::audio_exhausted(self)
    }

    fn applied_volume(&self) -> (f32, f32) {
        Player::applied_volume(self)
    }

    fn set_fixed_volume(&mut self, fixed: bool) {
        Player::set_fixed_volume(self, fixed);
    }
}

/// What the app-facing player object shares with the engine thread about the output.
pub(crate) struct OutputShared {
    pub ring: Arc<Ring>,
    /// The app asked for silence now (becoming noisy, focus loss).
    pub hold: AtomicBool,
    pub route: Mutex<OutputRoute>,
    /// Bit-perfect output negotiation (the host's `AudioManager`), if any.
    pub negotiator: Mutex<Option<Arc<dyn OutputNegotiator>>>,
}

/// The pipeline, ring producer, output and its negotiated route.
pub(crate) struct AudioPath {
    pub out: Arc<OutputShared>,
    pub producer: Producer,
    output: Box<dyn AudioOutput>,
    output_failed_at: Option<Instant>,
    device: String,
    pub pipeline: Pipeline,
    block: Vec<f32>,
    /// How the open output was set up.
    pub route_state: RouteState,
    /// The stream format of the open output.
    out_encoding: PcmEncoding,
    /// The mixer attributes the host applied for us (cleared when the output closes).
    applied: Option<(MixerOffer, Arc<dyn OutputNegotiator>)>,
    /// What the device offered at the last negotiation (to tell whether a new track
    /// needs another format).
    last_offers: Option<OutputOffers>,
    /// The source of the audio the output was opened for.
    current_source: Option<SourceFormat>,
    /// The track whose output format was last checked.
    planned_seq: Option<u32>,
    /// Negotiate again at the next poll.
    pub renegotiate: bool,
    /// The signal path may have changed: publish it.
    signal_dirty: bool,
    /// Why the output last failed to open (taken by the engine for its status).
    error: Option<String>,
    /// Crossfades between tracks (off by default: then nothing passes through it).
    pub transitions: Transitions,
}

/// What the transitions ask about tracks, from the engine's lookups and the pipeline.
struct FactsFrom<'a> {
    dsp: &'a DspSettings,
    analysis: &'a dyn Fn(u32) -> Option<Measurement>,
    facts: &'a dyn Fn(u32) -> Option<TrackFacts>,
}

impl TrackFactsSource for FactsFrom<'_> {
    fn facts(&self, seq: u32) -> Option<TrackFacts> {
        (self.facts)(seq)
    }

    fn level_db(&self, seq: u32, audio: Option<&TrackAudio>) -> f64 {
        if !self.dsp.active() {
            return 0.0;
        }
        let tags = audio.and_then(|a| a.source).map(|s| s.replay_gain);
        super::pipeline::levelling_decision(self.dsp, tags.as_ref(), (self.analysis)(seq)).0
    }
}

impl AudioPath {
    pub(crate) fn new(
        out: Arc<OutputShared>,
        output: Box<dyn AudioOutput>,
        dsp: DspSettings,
    ) -> Result<AudioPath, ApiError> {
        let transitions = dsp_transitions(&dsp);
        let producer = out
            .ring
            .producer()
            .ok_or_else(|| ApiError::invalid("player already running"))?;
        Ok(AudioPath {
            out,
            producer,
            output,
            output_failed_at: None,
            device: String::new(),
            pipeline: Pipeline::new(dsp),
            block: vec![0.0; BLOCK_FRAMES * CHANNELS],
            route_state: RouteState::default(),
            out_encoding: PcmEncoding::Float,
            applied: None,
            last_offers: None,
            current_source: None,
            planned_seq: None,
            renegotiate: false,
            signal_dirty: false,
            error: None,
            transitions: Transitions::new(transitions),
        })
    }

    /// The signal path may have changed since the last call.
    pub(crate) fn take_signal_dirty(&mut self) -> bool {
        std::mem::take(&mut self.signal_dirty)
    }

    /// Why the output last failed to open, once.
    pub(crate) fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// A new source (a new LMS session): track numbers start again.
    pub(crate) fn forget_plan(&mut self) {
        self.planned_seq = None;
    }

    /// Frames the output has consumed since the last call.
    pub(crate) fn played(&mut self) -> u64 {
        let pos = self.out.ring.read_pos();
        self.pipeline.played(pos)
    }

    /// Drop audio queued between the source and the speaker (a flush, a skip, a seek).
    pub(crate) fn drop_queued_audio(&mut self) {
        self.producer.discard_all();
        self.pipeline.reset();
        self.transitions.reset();
    }

    /// Nothing waits between the source and the ring (no pending block, nothing held).
    pub(crate) fn idle(&self) -> bool {
        !self.pipeline.has_pending() && self.transitions.is_empty()
    }

    /// Close the output for good (stopping): Android's normal mixing is restored.
    pub(crate) fn close_output(&mut self, src: Option<&mut dyn AudioSource>) {
        self.output.close();
        self.clear_mixer_attributes();
        self.route_state = RouteState::default();
        self.out_encoding = PcmEncoding::Float;
        if let Some(p) = src {
            p.set_fixed_volume(false);
        }
    }

    /// Restore Android's normal mixing, through the negotiator that set the attributes
    /// (even if the host has since replaced or removed it).
    fn clear_mixer_attributes(&mut self) {
        if let Some((_, n)) = self.applied.take() {
            n.apply(None);
        }
    }

    fn negotiator(&self) -> Option<Arc<dyn OutputNegotiator>> {
        self.out.negotiator.lock().ok().and_then(|n| n.clone())
    }

    /// React to the output's events and to renegotiation requests.
    pub(crate) fn poll_output(&mut self, src: Option<&mut dyn AudioSource>) {
        if let Some(OutputEvent::Disconnected) = self.output.poll_event() {
            // Headphones unplugged, Bluetooth gone: reopen on the new default device.
            self.reopen_output(src);
        } else if self.renegotiate {
            // A USB DAC came or went, or the bit-perfect setting changed: set the output
            // up again now (queued audio carries on through the new stream).
            self.renegotiate = false;
            self.last_offers = None;
            self.reopen_output(src);
        }
    }

    /// Move audio from the source through the pipeline into the ring, reopening the output
    /// when the source's rate or format changes. `gate`: hold back new audio (a fresh start
    /// waiting for its loudness analysis). `analysis`: a track's R128 measurement, if known.
    /// `facts`: what transitions need to know about a track (album, analysis), if known.
    pub(crate) fn fill(
        &mut self,
        src: &mut dyn AudioSource,
        gate: bool,
        analysis: &dyn Fn(u32) -> Option<Measurement>,
        facts: &dyn Fn(u32) -> Option<TrackFacts>,
    ) {
        let ring = self.out.ring.clone();
        let wanted = dsp_transitions(self.pipeline.dsp());
        if wanted != self.transitions.config() {
            self.transitions.set_config(wanted);
            self.signal_dirty = true;
        }
        let through = self.transitions.active();
        loop {
            if !self.pipeline.write_pending(&mut self.producer) {
                return; // the ring is full
            }
            if through && self.output_failed_at.is_none() {
                // Keep the end of the current track in hand, for the next boundary.
                self.transitions.top_up(src, &mut self.block);
            }
            let next = if through {
                self.transitions.next_audio(src)
            } else {
                src.next_audio()
            };
            let Some(next) = next else {
                let exhausted = if through {
                    self.transitions.exhausted(src)
                } else {
                    src.audio_exhausted()
                };
                if exhausted && self.pipeline.is_resampling() {
                    // The end of the audio: let the resampler's last frames out.
                    self.pipeline.drain_resampler();
                    continue;
                }
                return;
            };
            let rate = next.sample_rate;
            // A new track at the same rate may still need another output format (e.g. a
            // 24-bit track after a 16-bit one on a bit-perfect DAC).
            let new_format = self.planned_seq != Some(next.seq)
                && needs_new_format(&self.last_offers, self.out_encoding, &next);
            if self.planned_seq != Some(next.seq) && !new_format {
                self.planned_seq = Some(next.seq);
            }
            if self.pipeline.src_rate() != Some(rate) || new_format {
                if self.pipeline.src_rate().is_some() && self.pipeline.is_resampling() {
                    self.pipeline.drain_resampler();
                    if !self.pipeline.write_pending(&mut self.producer) {
                        return;
                    }
                }
                // Play out everything at the old rate first (a rate change is a track
                // boundary), then reopen the output at the new one.
                if !ring.is_empty() {
                    return;
                }
                if !self.open_output(rate, next.source, src) {
                    return;
                }
                self.planned_seq = Some(next.seq);
                continue;
            }
            if self.output_failed_at.is_some() {
                return;
            }
            let ahead = (u64::from(self.pipeline.out_rate().unwrap_or(rate))
                * RING_AHEAD.as_millis() as u64
                / 1000) as usize
                * CHANNELS;
            let ahead = ahead.min(ring.capacity() / 2);
            if ring.len() >= ahead {
                return;
            }
            if gate {
                return;
            }
            let (seq, frames, src_frames) = if through {
                let info = FactsFrom {
                    dsp: self.pipeline.dsp(),
                    analysis,
                    facts,
                };
                let low = ring.len() < ahead / 2;
                let Some(p) = self.transitions.pull(src, &mut self.block, &info, low) else {
                    return;
                };
                (p.seq, p.frames, p.src_frames)
            } else {
                let Some((seq, frames)) = src.pull_track(&mut self.block) else {
                    return;
                };
                (seq, frames, frames as u64)
            };
            let source = src.track_audio(seq).and_then(|t| t.source);
            let tags = source.map(|s| s.replay_gain);
            self.pipeline.push_counted(
                &self.block[..frames * CHANNELS],
                seq,
                tags.as_ref(),
                analysis(seq),
                source_bits(source),
                src_frames,
            );
        }
    }

    /// Open the output for audio at `rate` from `source`; false if it failed (retried
    /// later).
    ///
    /// With a negotiator (a USB DAC on Android 14+), the bit-perfect format is negotiated
    /// first and the stream opened in it at the source's rate; if anything refuses, Android's
    /// normal mixer is restored and the output opens as float, as without one: at the
    /// source's rate, or a fallback rate with our resampler.
    fn open_output(
        &mut self,
        rate: u32,
        source: Option<SourceFormat>,
        src: &mut dyn AudioSource,
    ) -> bool {
        if let Some(t) = self.output_failed_at
            && t.elapsed() < OUTPUT_RETRY
        {
            return false;
        }
        self.output.close();
        self.current_source = source;
        let spec = SourceSpec {
            rate,
            bits: source_bits(source),
        };
        let negotiator = self.negotiator();
        let (mut plan, dac) = match &negotiator {
            Some(n) => {
                let offers = n.offers();
                let device = protocol::clean_name(&offers.device);
                let plan = negotiate::plan(spec, &offers);
                self.last_offers = Some(offers);
                (plan, device)
            }
            None => {
                self.last_offers = None;
                (Plan::mixed(NotBitPerfect::NoUsbDac), String::new())
            }
        };
        // Apply the mixer attributes (or restore normal mixing). A negotiator that was
        // replaced clears what it set first.
        if let Some((_, old)) = &self.applied
            && !negotiator.as_ref().is_some_and(|n| Arc::ptr_eq(n, old))
        {
            self.clear_mixer_attributes();
        }
        match (plan.offer, &negotiator) {
            (Some(offer), Some(n)) => {
                if n.apply(Some(offer)) {
                    self.applied = Some((offer, n.clone()));
                } else {
                    self.applied = None;
                    plan = Plan::mixed(NotBitPerfect::NotApplied);
                }
            }
            _ => self.clear_mixer_attributes(),
        }
        let ring = self.out.ring.clone();
        if plan.encoding != PcmEncoding::Float {
            match self.output.open(&ring, rate, plan.encoding) {
                Ok(o) if o.rate == rate && o.encoding == plan.encoding => {
                    if self.pipeline.configure(rate, rate).is_ok() {
                        self.output_ready(o.device, o.encoding, plan.quantise_bits);
                        self.route_state = RouteState {
                            direct: true,
                            encoding: o.encoding,
                            device: dac,
                            reasons: plan.reasons,
                        };
                        src.set_fixed_volume(true);
                        self.signal_dirty = true;
                        return true;
                    }
                    self.output.close();
                }
                Ok(_) => self.output.close(),
                Err(_) => {}
            }
            // The stream wouldn't take the negotiated format: back to normal mixing.
            self.clear_mixer_attributes();
            plan = Plan::mixed(NotBitPerfect::FormatRefused);
        }
        let mut last_err = String::new();
        let candidates =
            std::iter::once(rate).chain(FALLBACK_RATES.into_iter().filter(|r| *r != rate));
        for try_rate in candidates {
            match self.output.open(&ring, try_rate, PcmEncoding::Float) {
                Ok(o) => match self.pipeline.configure(rate, o.rate) {
                    Ok(()) => {
                        self.output_ready(o.device, o.encoding, None);
                        self.route_state = RouteState {
                            direct: false,
                            encoding: PcmEncoding::Float,
                            device: dac,
                            reasons: plan.reasons,
                        };
                        src.set_fixed_volume(false);
                        self.signal_dirty = true;
                        return true;
                    }
                    Err(e) => {
                        self.output.close();
                        last_err = e;
                    }
                },
                Err(e) => last_err = e,
            }
        }
        self.pipeline.unconfigure();
        self.output_failed_at = Some(Instant::now());
        self.error = Some(protocol::clean_name(&last_err));
        false
    }

    fn output_ready(&mut self, device: String, encoding: PcmEncoding, bits: Option<u32>) {
        self.device = device;
        self.out_encoding = encoding;
        self.pipeline.set_output_bits(bits);
        self.output_failed_at = None;
    }

    fn reopen_output(&mut self, src: Option<&mut dyn AudioSource>) {
        let Some(src_rate) = self.pipeline.src_rate() else {
            return;
        };
        self.output.close();
        self.output_failed_at = None;
        self.signal_dirty = true;
        if self.negotiator().is_some() || self.route_state.direct {
            // The device may offer something else now (a DAC came or went): negotiate
            // afresh. Queued audio carries on through the new stream.
            match src {
                Some(s) => {
                    self.open_output(src_rate, self.current_source, s);
                }
                None => {
                    let mut none = NoSource;
                    self.open_output(src_rate, self.current_source, &mut none);
                }
            }
            return;
        }
        let ring = self.out.ring.clone();
        let out = self.pipeline.out_rate().unwrap_or(src_rate);
        match self.output.open(&ring, out, PcmEncoding::Float) {
            Ok(o) => self.device = o.device,
            Err(_) => {
                // The new device can't take that rate: start over (fallbacks, resampling).
                self.pipeline.unconfigure();
                let source = self.current_source;
                match src {
                    Some(s) => {
                        self.open_output(src_rate, source, s);
                    }
                    None => {
                        self.open_output(src_rate, source, &mut NoSource);
                    }
                }
            }
        }
    }

    /// The signal path for what `src` is playing now (`None` while nothing is audible).
    pub(crate) fn signal_path(&self, src: &dyn AudioSource) -> Option<SignalPathInfo> {
        let route = self.out.route.lock().map(|r| r.clone()).unwrap_or_default();
        let seq = src.playing_track()?;
        let audio = src.track_audio(seq)?;
        let out_rate = self.pipeline.out_rate()?;
        Some(signal::describe(&signal::Inputs {
            transitions: self.transitions.config().describe(),
            audio,
            volume: src.applied_volume(),
            pipeline_stages: self.pipeline.stages(),
            dsp_active: self.pipeline.processing(),
            dsp_chain: self.pipeline.dsp_processing(),
            duck: self.out.ring.gain(),
            out_rate,
            device: if route.device.is_empty() {
                self.device.clone()
            } else {
                route.device.clone()
            },
            mixer_rate: route.mixer_rate,
            route: self.route_state.clone(),
        }))
    }
}

/// A stand-in source when the output must be reopened with no source at hand.
struct NoSource;

impl AudioSource for NoSource {
    fn next_audio(&self) -> Option<TrackAudio> {
        None
    }

    fn pull_track(&mut self, _: &mut [f32]) -> Option<(u32, usize)> {
        None
    }

    fn track_audio(&self, _: u32) -> Option<TrackAudio> {
        None
    }

    fn playing_track(&self) -> Option<u32> {
        None
    }

    fn audio_exhausted(&self) -> bool {
        true
    }

    fn applied_volume(&self) -> (f32, f32) {
        (1.0, 1.0)
    }

    fn set_fixed_volume(&mut self, _: bool) {}
}

/// Would `next` need another stream format than the open output's (`current`)? Judged
/// from what the device offered at the last negotiation; never without a negotiator.
fn needs_new_format(
    offers: &Option<OutputOffers>,
    current: PcmEncoding,
    next: &TrackAudio,
) -> bool {
    let Some(offers) = offers else {
        return false;
    };
    let spec = SourceSpec {
        rate: next.sample_rate,
        bits: source_bits(next.source),
    };
    negotiate::plan(spec, offers).encoding != current
}

/// The integer depth of a lossless source (`None`: lossy, float or not yet known).
pub(crate) fn source_bits(source: Option<SourceFormat>) -> Option<u32> {
    source.filter(|s| s.lossless).and_then(|s| s.bits)
}

/// The transition settings carried in the DSP settings.
pub(crate) fn dsp_transitions(dsp: &DspSettings) -> super::transitions::TransitionConfig {
    super::transitions::TransitionConfig {
        mode: dsp.transitions,
        fade_s: dsp.crossfade_s,
    }
    .sanitised()
}
