//! This phone as an LMS player ("Play on this phone").
//!
//! Like the web app's player, the phone speaks Slimproto to LMS through the gateway's
//! `/v1/player` tunnel and fetches audio from `/v1/stream/…`; LMS owns the queue (SPEC
//! D14). The core decodes; this module hosts it: network, reconnect and resume, the
//! processing pipeline, a lock-free ring buffer and the audio output.
//!
//! - [`LocalPlayer`]: start/stop, DSP settings, a local hold (pause at once, for "becoming
//!   noisy"), ducking, and a [`SignalPathInfo`] snapshot.
//! - Output: AAudio from Rust on Android ([`OutputKind::AAudio`]); elsewhere, or as a
//!   fallback, the host pulls PCM with [`LocalPlayer::read_pcm`] ([`OutputKind::Pull`]).
//!
//! Playback state that LMS owns (play, pause, skip, volume) is changed through LMS, with
//! the ordinary player commands for this player's id; LMS then drives the engine.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod clock;
pub mod negotiate;
pub mod output;
pub mod pipeline;
pub mod protocol;
pub mod ring;
pub mod transitions;

mod audio;
mod engine;
mod local;
mod offline;
mod queue;
mod signal;
mod spool;
mod visual;

use audio::OutputShared;
use engine::{Cmd, Engine, Shared};
use output::{AudioOutput, PullOutput, PullSlot};
use ring::Ring;
use spool::SpoolDir;

pub use negotiate::{
    MixerOffer, NotBitPerfect, OfferBlocker, OutputNegotiator, OutputOffers, PcmEncoding,
};
pub use offline::{OfflinePlayer, OfflinePlayerObserver, OfflinePlayerStatus};
pub use pipeline::{
    CrossfeedSetting, DitherSetting, DspSettings, EqProfileSummary, Levelling, check_eq_profile,
};
pub use queue::LocalRepeat;
pub use transitions::TransitionMode;
pub use visual::{VISUAL_FRAME_BYTES, VISUAL_FRAME_FLOATS};

use crate::error::ApiError;
use crate::session::Session;

/// Ring capacity: about 1.4 s at 192 kHz stereo (the engine keeps far less queued).
pub(crate) const RING_SAMPLES: usize = 1 << 19;

/// Where the audio goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OutputKind {
    /// AAudio from Rust (Android only).
    AAudio,
    /// The host pulls PCM with [`LocalPlayer::read_pcm`].
    Pull,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LocalPlayerConfig {
    pub output: OutputKind,
    /// Highest sample rate offered to LMS (it transcodes anything higher).
    pub max_sample_rate: u32,
    pub dsp: DspSettings,
    /// An app-private folder for spooling live streams to disk (Android: a folder under
    /// `noBackupFilesDir`); `None` keeps streams in memory, as before. Absolute, without
    /// `..`, and not a symlink; spools go in a per-server, per-user folder inside it and
    /// are deleted as soon as they are done with.
    #[uniffi(default = None)]
    pub cache_dir: Option<String>,
    /// Read the next track ahead: tell LMS a track is ready once it is fully received, so
    /// the next one arrives long before the current one ends (needs `cache_dir`).
    #[uniffi(default = true)]
    pub read_ahead: bool,
}

/// The player's link to LMS.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PlayerConnection {
    Connecting,
    Connected,
    /// Lost; trying again (resuming the same LMS player) after `delay_ms`.
    Reconnecting {
        delay_ms: u64,
    },
    /// Ended for good: stopped here, refused, signed out, or replaced by another
    /// connection of this device.
    Stopped {
        reason: String,
    },
}

/// What the output is doing (LMS decides; this reflects it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LocalPlayback {
    Stopped,
    Buffering,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LocalPlayerStatus {
    pub connection: PlayerConnection,
    pub playback: LocalPlayback,
    /// This player's id in LMS (its MAC), once connected.
    pub player_id: String,
    pub name: String,
    pub elapsed_ms: u64,
    /// Decoded audio queued ahead of the output.
    pub buffered_ms: u64,
    /// The output stream's sample rate (0 while closed).
    pub output_rate: u32,
    pub underruns: u64,
    pub underrun_frames: u64,
    /// Silenced locally (see [`LocalPlayer::set_hold`]).
    pub held: bool,
    pub last_error: Option<String>,
    /// About how much listening is on the phone ahead of what is heard (ms): the decoded
    /// audio plus what is spooled, at the stream's byte rate. What a gap in the signal can
    /// last without the music stopping.
    #[uniffi(default = 0)]
    pub cushion_ms: u64,
    /// How long after its connection drops the player keeps resuming the same LMS session
    /// (ms), from the grace the gateway granted.
    #[uniffi(default = 0)]
    pub resume_window_ms: u64,
    /// Read-ahead is in force (the setting is on and there is somewhere to spool).
    #[uniffi(default = false)]
    pub read_ahead: bool,
}

impl Default for LocalPlayerStatus {
    fn default() -> Self {
        LocalPlayerStatus {
            connection: PlayerConnection::Stopped {
                reason: "not started".into(),
            },
            playback: LocalPlayback::Stopped,
            player_id: String::new(),
            name: String::new(),
            elapsed_ms: 0,
            buffered_ms: 0,
            output_rate: 0,
            underruns: 0,
            underrun_frames: 0,
            held: false,
            last_error: None,
            cushion_ms: 0,
            resume_window_ms: 0,
            read_ahead: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SignalStage {
    pub name: String,
    /// Leaves samples bit-identical.
    pub bit_transparent: bool,
    /// Changes the sample rate.
    pub resamples: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SignalVerdict {
    BitPerfect,
    LosslessProcessed,
    Resampled,
    LossySource,
}

/// The path from file to output for what is audible now.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SignalPathInfo {
    pub source_codec: String,
    pub source_lossless: bool,
    pub source_bits: Option<u32>,
    pub source_rate: u32,
    pub source_channels: u32,
    /// Every stage after the source, in order (decode first).
    pub stages: Vec<SignalStage>,
    /// Our DSP or resampler changes samples.
    pub dsp_active: bool,
    /// The output device: the USB DAC's name on a direct route, else the route.
    pub output_device: String,
    pub output_rate: u32,
    /// e.g. "32-bit float" (Android's mixer) or "24-bit integer" (a bit-perfect DAC).
    pub output_format: String,
    /// The integer depth the DAC accepted (`None`: float to Android's mixer).
    pub output_bits: Option<u32>,
    /// Bit-perfect mixer attributes are in force: Android passes the stream to the USB
    /// DAC without mixing, resampling or volume.
    pub direct: bool,
    /// LMS's volume is ignored (100 %); the DAC's own volume controls the level.
    pub fixed_volume: bool,
    pub resampled: bool,
    /// Our engine delivers the source's samples untouched, at its rate.
    pub engine_bit_perfect: bool,
    /// ... and the platform passes them to the device untouched.
    pub bit_perfect: bool,
    /// Why it isn't bit-perfect end to end (empty when it is).
    pub not_bit_perfect: Vec<NotBitPerfect>,
    pub verdict: SignalVerdict,
    /// One line, e.g. "FLAC 96 kHz/24-bit → Decode (lossless) → AAudio 96 kHz/float".
    pub summary: String,
}

/// What the app knows about the output route (from `AudioManager`). Whether the route is
/// bit-perfect is not the app's to say: it comes from negotiation
/// ([`LocalPlayer::set_output_negotiator`]).
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct OutputRoute {
    /// e.g. "Headphones", "USB DAC", "Bluetooth"; empty = unknown.
    pub device: String,
    /// The platform mixer's sample rate (0 = unknown).
    pub mixer_rate: u32,
}

/// PCM pulled by the host (interleaved stereo float).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PcmBlock {
    /// 0 when no output is open (play nothing).
    pub sample_rate: u32,
    /// Interleaved stereo; silence where nothing was ready.
    pub samples: Vec<f32>,
    /// How many of the samples are real audio (the rest is silence).
    pub audio_samples: u32,
}

/// PCM pulled by the host as integers, in the format the output was opened with.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PcmIntBlock {
    /// 0 when no output is open (play nothing).
    pub sample_rate: u32,
    /// The output's format; for float, samples are 24-bit.
    pub encoding: PcmEncoding,
    /// Interleaved stereo, right-aligned at the format's depth; silence where nothing was
    /// ready.
    pub samples: Vec<i32>,
    /// How many of the samples are real audio.
    pub audio_samples: u32,
}

/// Receives updates, on the engine's thread (hand them to the UI thread yourself).
#[uniffi::export(with_foreign)]
pub trait LocalPlayerObserver: Send + Sync {
    fn on_status(&self, status: LocalPlayerStatus);
    fn on_signal_path(&self, path: Option<SignalPathInfo>);
}

struct Run {
    shared: Arc<Shared>,
    cmds: tokio::sync::mpsc::UnboundedSender<Cmd>,
    pull: Arc<Mutex<PullSlot>>,
    thread: std::thread::JoinHandle<()>,
}

/// The level of the live stream (docs/plans/streaming-quality.md): as LMS sends it, or FLAC
/// re-encoded to Opus by the gateway for mobile data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum StreamQuality {
    /// As LMS sends it (lossless stays lossless).
    #[default]
    Original,
    /// Opus 160 kb/s.
    High,
    /// Opus 96 kb/s ("Data saver").
    Low,
}

impl StreamQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            StreamQuality::Original => "original",
            StreamQuality::High => "high",
            StreamQuality::Low => "low",
        }
    }
}

/// The level to stream at: [`StreamQuality`] for Wi-Fi or for mobile data, by whether the
/// network is metered.
#[uniffi::export]
pub fn stream_quality_for(
    metered: bool,
    on_wifi: StreamQuality,
    on_mobile: StreamQuality,
) -> StreamQuality {
    if metered { on_mobile } else { on_wifi }
}

/// This phone as an LMS player.
#[derive(uniffi::Object)]
pub struct LocalPlayer {
    session: Arc<Session>,
    config: Mutex<LocalPlayerConfig>,
    run: Mutex<Option<Run>>,
    route: Mutex<OutputRoute>,
    hold: Mutex<bool>,
    duck: Mutex<bool>,
    /// The sleep timer's fade-out: a factor on the output (`f32` bits), 1 when none runs.
    fade: AtomicU32,
    /// Bumped by every change to the fade, so an older fade's thread stops.
    fade_gen: AtomicU64,
    negotiator: Mutex<Option<Arc<dyn OutputNegotiator>>>,
    /// The stream level wanted (kept across restarts of the engine).
    quality: Mutex<StreamQuality>,
    /// The visualiser (kept across restarts of the engine; off by default).
    visual: Arc<visual::VisualShared>,
    /// The phone's media volume for loudness compensation (kept across restarts).
    loudness_reference: Mutex<Option<f64>>,
    /// Whether the phone has a network, as the app last said (kept across restarts).
    network: Mutex<bool>,
    /// Tests: how much of a track read-ahead waits to be heard (kept across restarts).
    ready_after_ms: Mutex<Option<u64>>,
}

pub(crate) fn make_output(
    kind: OutputKind,
    pull: &Arc<Mutex<PullSlot>>,
) -> Result<Box<dyn AudioOutput>, ApiError> {
    match kind {
        OutputKind::Pull => Ok(Box::new(PullOutput::new(pull.clone()))),
        #[cfg(target_os = "android")]
        OutputKind::AAudio => Ok(Box::new(output::AAudioOutput::default())),
        #[cfg(not(target_os = "android"))]
        OutputKind::AAudio => Err(ApiError::invalid("AAudio is only available on Android")),
    }
}

#[uniffi::export]
impl LocalPlayer {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>, config: LocalPlayerConfig) -> Arc<Self> {
        Arc::new(LocalPlayer {
            session,
            config: Mutex::new(sanitise_config(config)),
            run: Mutex::new(None),
            route: Mutex::new(OutputRoute::default()),
            hold: Mutex::new(false),
            duck: Mutex::new(false),
            fade: AtomicU32::new(1.0f32.to_bits()),
            fade_gen: AtomicU64::new(0),
            negotiator: Mutex::new(None),
            quality: Mutex::new(StreamQuality::Original),
            visual: Arc::new(visual::VisualShared::default()),
            loudness_reference: Mutex::new(None),
            network: Mutex::new(true),
            ready_after_ms: Mutex::new(None),
        })
    }

    /// Read the next track ahead (see [`LocalPlayerConfig::read_ahead`]); takes effect
    /// from the next track. Android turns it off on mobile data if the user asks.
    pub fn set_read_ahead(&self, on: bool) {
        self.config.lock().unwrap().read_ahead = on;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::SetReadAhead(on));
        }
    }

    /// The read-ahead setting (in force only with a cache folder: see
    /// [`LocalPlayerStatus::read_ahead`]).
    pub fn read_ahead(&self) -> bool {
        self.config.lock().unwrap().read_ahead
    }

    /// Whether the phone has a network (from Android's `ConnectivityManager`). Without one
    /// the player stops trying to reconnect (saving battery) and keeps playing what it
    /// has; when it returns, it reconnects at once.
    pub fn set_network_available(&self, available: bool) {
        *self.network.lock().unwrap() = available;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::Network(available));
        }
    }

    /// About how much listening is on the phone ahead of what is heard (ms; see
    /// [`LocalPlayerStatus::cushion_ms`]), for "4 min buffered ahead".
    pub fn cushion_ms(&self) -> u64 {
        self.status().cushion_ms
    }

    /// The phone's media volume as an attenuation in dB (0 = full volume), which loudness
    /// compensation follows together with LMS's own level for this player; `None` when it
    /// isn't known (a Bluetooth headset that keeps its own volume), so LMS's level alone
    /// counts. Cleaned up by `pipeline::sanitise_reference_db`; kept across restarts.
    pub fn set_loudness_reference_db(&self, db: Option<f64>) {
        let db = pipeline::sanitise_reference_db(db);
        *self.loudness_reference.lock().unwrap() = db;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::SetLoudnessReference(db));
        }
    }

    /// What [`LocalPlayer::set_loudness_reference_db`] last kept.
    pub fn loudness_reference_db(&self) -> Option<f64> {
        *self.loudness_reference.lock().unwrap()
    }

    /// Analyse the audio for a visualiser (docs/plans/wave2-design.md §4.1), or stop. Off
    /// by default; while off nothing is analysed. Turn it on only while a visualiser is on
    /// screen.
    pub fn set_visualiser(&self, on: bool) {
        self.visual.set_enabled(on);
    }

    /// The newest visualiser frame since the last call, as [`VISUAL_FRAME_BYTES`] bytes
    /// (see `visual::pack` for the layout); `None` when there is none, or it is off. Frames
    /// arrive as their audio is heard, at most 30 a second; poll at the display's pace.
    pub fn visual_frame(&self) -> Option<Vec<u8>> {
        self.visual.take()
    }

    /// The level for the streams that follow (the next track, or a seek): asked for when
    /// the player connects, and sent at once to a gateway that supports changing it.
    pub fn set_stream_quality(&self, quality: StreamQuality) {
        *self.quality.lock().unwrap() = quality;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::SetQuality(quality));
        }
    }

    pub fn stream_quality(&self) -> StreamQuality {
        *self.quality.lock().unwrap()
    }

    /// Become a player in LMS. Does nothing if already running.
    pub fn start(&self, observer: Arc<dyn LocalPlayerObserver>) -> Result<(), ApiError> {
        let mut run = self.run.lock().unwrap();
        if run.as_ref().is_some_and(|r| !r.thread.is_finished()) {
            return Ok(());
        }
        if !self.session.has_sign_in() {
            return Err(ApiError::Unauthorised);
        }
        let cfg = self.config.lock().unwrap().clone();
        let ring = Ring::new(RING_SAMPLES);
        ring.set_gain(output_gain(*self.duck.lock().unwrap(), self.fade_factor()));
        let shared = Arc::new(Shared {
            status: Mutex::new(LocalPlayerStatus::default()),
            signal: Mutex::new(None),
            observer,
            out: Arc::new(OutputShared {
                ring,
                hold: std::sync::atomic::AtomicBool::new(*self.hold.lock().unwrap()),
                route: Mutex::new(self.route.lock().unwrap().clone()),
                negotiator: Mutex::new(self.negotiator.lock().unwrap().clone()),
                visual: self.visual.clone(),
            }),
        });
        let pull = Arc::new(Mutex::new(PullSlot::default()));
        let output = make_output(cfg.output, &pull)?;
        let (cmds, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let spool_dir = cfg.cache_dir.as_deref().and_then(|root| {
            let who = self.session.current()?;
            let scope =
                crate::offline::scope_name(&self.session.server().url(), &who.user.username);
            // Leftovers of a crash, or of another server or user, go first.
            if let Ok(r) = spool::validate_root(root) {
                let _ = spool::remove_all(&r);
            }
            SpoolDir::open(root, &scope).ok()
        });
        let engine = Engine::new(
            self.session.core().clone(),
            shared.clone(),
            output,
            cfg.dsp,
            cfg.max_sample_rate,
            *self.quality.lock().unwrap(),
            spool_dir,
            cfg.read_ahead,
            cmd_rx,
        )?;
        let thread = std::thread::Builder::new()
            .name("anywhere-player".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("player runtime");
                rt.block_on(engine.run());
            })
            .map_err(|_| ApiError::invalid("could not start the player"))?;
        // The phone's volume as last known, before any audio.
        let _ = cmds.send(Cmd::SetLoudnessReference(
            *self.loudness_reference.lock().unwrap(),
        ));
        if !*self.network.lock().unwrap() {
            let _ = cmds.send(Cmd::Network(false));
        }
        if let Some(ms) = *self.ready_after_ms.lock().unwrap() {
            let _ = cmds.send(Cmd::SetReadyAfter(ms));
        }
        *run = Some(Run {
            shared,
            cmds,
            pull,
            thread,
        });
        Ok(())
    }

    /// Leave LMS and stop playing. Returns at once; the engine finishes in the background.
    pub fn stop(&self) {
        if let Some(r) = self.run.lock().unwrap().take() {
            r.shared.out.ring.set_hold(true);
            let _ = r.cmds.send(Cmd::Stop);
        }
    }

    pub fn is_running(&self) -> bool {
        self.run
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| !r.thread.is_finished())
    }

    pub fn status(&self) -> LocalPlayerStatus {
        self.run
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.shared.status.lock().unwrap().clone())
            .unwrap_or_default()
    }

    pub fn signal_path(&self) -> Option<SignalPathInfo> {
        self.run
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.shared.signal.lock().unwrap().clone())
    }

    /// This player's id in LMS (its MAC), once connected.
    pub fn player_id(&self) -> Option<String> {
        Some(self.status().player_id).filter(|id| !id.is_empty())
    }

    /// Change the DSP settings; they apply to audio processed from now on.
    pub fn set_dsp(&self, dsp: DspSettings) {
        let dsp = dsp.sanitised();
        self.config.lock().unwrap().dsp = dsp.clone();
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::SetDsp(dsp));
        }
    }

    /// Silence the output at once (headphones unplugged, audio focus lost), keeping the
    /// queued audio. The app also pauses the player through LMS, so LMS and the output
    /// agree. The hold lasts until released here, or until LMS next starts or resumes
    /// playback (from any controller).
    pub fn set_hold(&self, hold: bool) {
        *self.hold.lock().unwrap() = hold;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            r.shared.out.hold.store(hold, Ordering::Release);
            if hold {
                r.shared.out.ring.set_hold(true);
            }
        }
    }

    /// Lower the volume while another app speaks (audio focus "may duck").
    pub fn set_duck(&self, duck: bool) {
        *self.duck.lock().unwrap() = duck;
        self.apply_gain();
    }

    /// Fade the output out before the sleep timer ends, `ends_in_secs` from now (as LMS
    /// reports it), over `fade_secs` (0–120; 0 is no fade). `None` cancels and restores
    /// full volume at once. The fade runs here, in the engine, so it keeps going with the
    /// app in the background; it lifts by itself shortly after the end, once LMS has
    /// stopped the player. Calling it again replaces the fade (the timer was re-armed).
    pub fn set_sleep_fade(self: Arc<Self>, ends_in_secs: Option<f64>, fade_secs: u32) {
        let generation = self.fade_gen.fetch_add(1, Ordering::AcqRel) + 1;
        let fade = f64::from(crate::sleep::sleep_fade_sanitise(i64::from(fade_secs)));
        let ends = ends_in_secs.filter(|s| {
            s.is_finite() && *s > 0.0 && *s <= f64::from(crate::commands::MAX_SLEEP_SECS)
        });
        let Some(left) = ends.filter(|_| fade > 0.0) else {
            self.set_fade_factor(1.0);
            return;
        };
        let end = Instant::now() + Duration::from_secs_f64(left);
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("sleep-fade".into())
            .spawn(move || {
                loop {
                    if me.fade_gen.load(Ordering::Acquire) != generation {
                        return;
                    }
                    let now = Instant::now();
                    let left = if now <= end {
                        (end - now).as_secs_f64()
                    } else {
                        -(now - end).as_secs_f64()
                    };
                    let (gain, done) = fade_step(left, fade);
                    me.set_fade_factor(gain);
                    if done {
                        return;
                    }
                    std::thread::sleep(FADE_STEP);
                }
            });
        if spawned.is_err() {
            self.set_fade_factor(1.0); // no thread: no fade, rather than silence
        }
    }

    /// Tell the engine about the output route, for the signal path.
    pub fn set_output_route(&self, route: OutputRoute) {
        let route = OutputRoute {
            device: protocol::clean_name(&route.device),
            mixer_rate: if (8_000..=768_000).contains(&route.mixer_rate) {
                route.mixer_rate
            } else {
                0
            },
        };
        *self.route.lock().unwrap() = route.clone();
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            *r.shared.out.route.lock().unwrap() = route;
        }
    }

    /// The host's side of bit-perfect output negotiation (Android: `AudioManager`'s mixer
    /// attributes for a USB DAC); `None` for normal mixed output only. The engine asks it
    /// before opening its output for each new format, and tells it to clear its mixer
    /// attributes when the output closes. Renegotiates now if playing.
    pub fn set_output_negotiator(&self, negotiator: Option<Arc<dyn OutputNegotiator>>) {
        *self.negotiator.lock().unwrap() = negotiator.clone();
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            *r.shared.out.negotiator.lock().unwrap() = negotiator;
            let _ = r.cmds.send(Cmd::Renegotiate);
        }
    }

    /// Something changed what the output can do (a USB DAC plugged in or out, the
    /// bit-perfect setting): negotiate again and reopen the output, at the next moment it
    /// can be done without losing audio (between tracks, or at once if nothing is queued).
    pub fn renegotiate_output(&self) {
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::Renegotiate);
        }
    }

    /// [`OutputKind::Pull`] only: up to `max_frames` frames (at most 16,384) of audio at
    /// the returned rate. When the rate changes, reopen the host's output at the new rate
    /// before playing the block.
    pub fn read_pcm(&self, max_frames: u32) -> PcmBlock {
        let pull = self.run.lock().unwrap().as_ref().map(|r| r.pull.clone());
        read_float(pull, max_frames)
    }

    /// [`OutputKind::Pull`] only: like [`LocalPlayer::read_pcm`], as integers in the format
    /// the output was opened with (what an integer `AudioTrack`, or a DAC, receives).
    pub fn read_pcm_int(&self, max_frames: u32) -> PcmIntBlock {
        let pull = self.run.lock().unwrap().as_ref().map(|r| r.pull.clone());
        read_int(pull, max_frames)
    }
}

/// Pull float PCM from a pull output's slot (shared by both players).
pub(crate) fn read_float(pull: Option<Arc<Mutex<PullSlot>>>, max_frames: u32) -> PcmBlock {
    let frames = max_frames.clamp(1, 16_384) as usize;
    let empty = PcmBlock {
        sample_rate: 0,
        samples: Vec::new(),
        audio_samples: 0,
    };
    let Some(pull) = pull else { return empty };
    let mut slot = pull.lock().unwrap();
    let rate = slot.rate;
    match slot.consumer.as_mut() {
        Some(c) => {
            let mut samples = vec![0f32; frames * ring::CHANNELS];
            let n = c.read(&mut samples);
            PcmBlock {
                sample_rate: rate,
                samples,
                audio_samples: n as u32,
            }
        }
        None => empty,
    }
}

/// Pull integer PCM from a pull output's slot, in the format the output was opened with.
pub(crate) fn read_int(pull: Option<Arc<Mutex<PullSlot>>>, max_frames: u32) -> PcmIntBlock {
    let frames = max_frames.clamp(1, 16_384) as usize;
    let empty = PcmIntBlock {
        sample_rate: 0,
        encoding: PcmEncoding::Float,
        samples: Vec::new(),
        audio_samples: 0,
    };
    let Some(pull) = pull else { return empty };
    let mut slot = pull.lock().unwrap();
    let (rate, encoding) = (slot.rate, slot.encoding);
    let Some(c) = slot.consumer.as_mut() else {
        return empty;
    };
    let mut samples = vec![0i32; frames * ring::CHANNELS];
    let n = match encoding {
        PcmEncoding::I16 => {
            let mut s16 = vec![0i16; samples.len()];
            let n = c.read_i16(&mut s16);
            for (d, s) in samples.iter_mut().zip(s16) {
                *d = i32::from(s);
            }
            n
        }
        PcmEncoding::I32 => c.read_i32(&mut samples, 32),
        PcmEncoding::I24Packed | PcmEncoding::Float => c.read_i32(&mut samples, 24),
    };
    PcmIntBlock {
        sample_rate: rate,
        encoding,
        samples,
        audio_samples: n as u32,
    }
}

/// Delete every stream spool kept under `cache_dir` (the folder passed as
/// [`LocalPlayerConfig::cache_dir`]), for every server and user: at sign-out, when the
/// device was revoked, or on a change of server. Downloads are left alone. A running player
/// deletes its own spools as it goes; this is for when none is running.
#[uniffi::export]
pub fn remove_stream_spools(cache_dir: String) -> Result<(), ApiError> {
    let root = spool::validate_root(&cache_dir)?;
    spool::remove_all(&root)
        .map_err(|e| ApiError::invalid(format!("could not clear the stream cache: {}", e.kind())))
}

impl LocalPlayer {
    /// Tests: how much of a track must be heard before read-ahead reports it ready (the
    /// core's default is 20 s). Not part of the app's API.
    #[doc(hidden)]
    pub fn debug_set_ready_after_ms(&self, ms: u64) {
        *self.ready_after_ms.lock().unwrap() = Some(ms);
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(Cmd::SetReadyAfter(ms));
        }
    }

    fn fade_factor(&self) -> f32 {
        f32::from_bits(self.fade.load(Ordering::Acquire))
    }

    fn set_fade_factor(&self, f: f32) {
        let f = if f.is_finite() {
            f.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if self.fade.swap(f.to_bits(), Ordering::AcqRel) != f.to_bits() {
            self.apply_gain();
        }
    }

    /// Ducking and the sleep fade, on the ring of the running player (if any).
    fn apply_gain(&self) {
        let g = output_gain(*self.duck.lock().unwrap(), self.fade_factor());
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            r.shared.out.ring.set_gain(g);
        }
    }
}

/// How often the sleep fade moves the gain (small steps: no audible stairs).
const FADE_STEP: Duration = Duration::from_millis(100);
/// The fade lifts this long after the timer's end (LMS has stopped the player by then).
const FADE_LIFT_AFTER_SECS: f64 = 10.0;

/// The output gain for ducking and the sleep fade. Exactly 1.0 (a true bypass) when
/// neither applies.
pub(crate) fn output_gain(duck: bool, fade: f32) -> f32 {
    let f = if fade.is_finite() {
        fade.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if duck { DUCK_GAIN * f } else { f }
}

/// One step of the sleep fade, `left` seconds before the end (negative after it): the gain,
/// and whether the fade is over (lifted back to full, well after the end).
pub(crate) fn fade_step(left: f64, fade: f64) -> (f32, bool) {
    if left.is_nan() || left < -FADE_LIFT_AFTER_SECS {
        return (1.0, true);
    }
    (crate::sleep::sleep_fade_gain(Some(left), fade), false)
}

impl Drop for LocalPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Output gain while ducked (about −14 dB).
pub(crate) const DUCK_GAIN: f32 = 0.2;

pub(crate) fn sanitise_config(c: LocalPlayerConfig) -> LocalPlayerConfig {
    LocalPlayerConfig {
        output: c.output,
        max_sample_rate: c.max_sample_rate.clamp(44_100, 384_000),
        dsp: c.dsp.sanitised(),
        // A folder that isn't acceptable means no spooling, never a failure to play.
        cache_dir: c.cache_dir.filter(|d| spool::validate_root(d).is_ok()),
        read_ahead: c.read_ahead,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ducking_and_the_sleep_fade_combine_and_full_is_a_true_bypass() {
        assert_eq!(output_gain(false, 1.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(output_gain(true, 1.0), DUCK_GAIN);
        assert!((output_gain(true, 0.5) - DUCK_GAIN * 0.5).abs() < 1e-7);
        assert_eq!(output_gain(false, 0.0), 0.0);
        // Junk never amplifies or goes silent by accident.
        assert_eq!(output_gain(false, f32::NAN), 1.0);
        assert_eq!(output_gain(false, 7.0), 1.0);
        assert_eq!(output_gain(false, -1.0), 0.0);
    }

    #[test]
    fn the_sleep_fade_steps_down_then_lifts_after_the_end() {
        assert_eq!(fade_step(100.0, 30.0), (1.0, false));
        let (g, done) = fade_step(15.0, 30.0);
        assert!((g - 0.25).abs() < 1e-6 && !done);
        assert_eq!(fade_step(0.0, 30.0), (0.0, false));
        assert_eq!(fade_step(-5.0, 30.0), (0.0, false)); // LMS stops the player meanwhile
        assert_eq!(fade_step(-10.5, 30.0), (1.0, true)); // back to full for next time
        assert_eq!(fade_step(f64::NAN, 30.0), (1.0, true));
    }

    #[test]
    fn config_is_sanitised() {
        let c = sanitise_config(LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 10,
            dsp: DspSettings {
                preamp_db: 500.0,
                ..DspSettings::default()
            },
            cache_dir: Some("relative/cache".into()),
            read_ahead: true,
        });
        assert_eq!(c.max_sample_rate, 44_100);
        assert_eq!(c.dsp.preamp_db, 12.0);
        assert_eq!(c.cache_dir, None, "an unacceptable folder means no spool");
        for bad in ["/tmp/../etc", "", "x"] {
            let c = sanitise_config(LocalPlayerConfig {
                output: OutputKind::Pull,
                max_sample_rate: 48_000,
                dsp: DspSettings::default(),
                cache_dir: Some(bad.into()),
                read_ahead: true,
            });
            assert_eq!(c.cache_dir, None, "{bad}");
        }
        let root = std::env::temp_dir();
        let c = sanitise_config(LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 48_000,
            dsp: DspSettings::default(),
            cache_dir: Some(root.to_string_lossy().into_owned()),
            read_ahead: false,
        });
        assert!(c.cache_dir.is_some() && !c.read_ahead);
    }

    #[test]
    fn removing_spools_refuses_bad_folders() {
        assert!(remove_stream_spools("relative".into()).is_err());
        assert!(remove_stream_spools("/a/../b".into()).is_err());
        let missing = std::env::temp_dir().join("anywhere-no-such-cache-folder-x");
        assert!(remove_stream_spools(missing.to_string_lossy().into_owned()).is_ok());
    }
}
