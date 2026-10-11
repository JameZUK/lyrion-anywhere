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

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub mod output;
pub mod pipeline;
pub mod protocol;
pub mod ring;

mod engine;
mod signal;

use engine::{Cmd, Engine, Shared};
use output::{AudioOutput, PullOutput, PullSlot};
use ring::Ring;

pub use pipeline::{DspSettings, Levelling};

use crate::error::ApiError;
use crate::session::Session;

/// Ring capacity: about 1.4 s at 192 kHz stereo (the engine keeps far less queued).
const RING_SAMPLES: usize = 1 << 19;

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
    pub output_device: String,
    pub output_rate: u32,
    pub output_format: String,
    pub resampled: bool,
    /// Our engine delivers the source's samples untouched, at its rate.
    pub engine_bit_perfect: bool,
    /// ... and the platform passes them to the device untouched.
    pub bit_perfect: bool,
    pub verdict: SignalVerdict,
    /// One line, e.g. "FLAC 96 kHz/24-bit → Decode (lossless) → AAudio 96 kHz/float".
    pub summary: String,
}

/// What the app knows about the output route (from `AudioManager`).
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct OutputRoute {
    /// e.g. "Headphones", "USB DAC", "Bluetooth (LDAC)"; empty = unknown.
    pub device: String,
    /// The platform mixer's sample rate (0 = unknown).
    pub mixer_rate: u32,
    /// The platform passes the stream to the device untouched (bit-perfect mixer mode).
    pub direct: bool,
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

/// This phone as an LMS player.
#[derive(uniffi::Object)]
pub struct LocalPlayer {
    session: Arc<Session>,
    config: Mutex<LocalPlayerConfig>,
    run: Mutex<Option<Run>>,
    route: Mutex<OutputRoute>,
    hold: Mutex<bool>,
    duck: Mutex<bool>,
}

fn make_output(
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
        })
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
        ring.set_gain(if *self.duck.lock().unwrap() {
            DUCK_GAIN
        } else {
            1.0
        });
        let shared = Arc::new(Shared {
            status: Mutex::new(LocalPlayerStatus::default()),
            signal: Mutex::new(None),
            observer,
            ring,
            hold: std::sync::atomic::AtomicBool::new(*self.hold.lock().unwrap()),
            route: Mutex::new(self.route.lock().unwrap().clone()),
        });
        let pull = Arc::new(Mutex::new(PullSlot::default()));
        let output = make_output(cfg.output, &pull)?;
        let (cmds, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let engine = Engine::new(
            self.session.core().clone(),
            shared.clone(),
            output,
            cfg.dsp,
            cfg.max_sample_rate,
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
            r.shared.ring.set_hold(true);
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
            r.shared.hold.store(hold, Ordering::Release);
            if hold {
                r.shared.ring.set_hold(true);
            }
        }
    }

    /// Lower the volume while another app speaks (audio focus "may duck").
    pub fn set_duck(&self, duck: bool) {
        *self.duck.lock().unwrap() = duck;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            r.shared.ring.set_gain(if duck { DUCK_GAIN } else { 1.0 });
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
            direct: route.direct,
        };
        *self.route.lock().unwrap() = route.clone();
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            *r.shared.route.lock().unwrap() = route;
        }
    }

    /// [`OutputKind::Pull`] only: up to `max_frames` frames (at most 16,384) of audio at
    /// the returned rate. When the rate changes, reopen the host's output at the new rate
    /// before playing the block.
    pub fn read_pcm(&self, max_frames: u32) -> PcmBlock {
        let frames = max_frames.clamp(1, 16_384) as usize;
        let pull = self.run.lock().unwrap().as_ref().map(|r| r.pull.clone());
        let Some(pull) = pull else {
            return PcmBlock {
                sample_rate: 0,
                samples: Vec::new(),
                audio_samples: 0,
            };
        };
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
            None => PcmBlock {
                sample_rate: 0,
                samples: Vec::new(),
                audio_samples: 0,
            },
        }
    }
}

impl Drop for LocalPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Output gain while ducked (about −14 dB).
const DUCK_GAIN: f32 = 0.2;

fn sanitise_config(c: LocalPlayerConfig) -> LocalPlayerConfig {
    LocalPlayerConfig {
        output: c.output,
        max_sample_rate: c.max_sample_rate.clamp(44_100, 384_000),
        dsp: c.dsp.sanitised(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_sanitised() {
        let c = sanitise_config(LocalPlayerConfig {
            output: OutputKind::Pull,
            max_sample_rate: 10,
            dsp: DspSettings {
                preamp_db: 500.0,
                ..DspSettings::default()
            },
        });
        assert_eq!(c.max_sample_rate, 44_100);
        assert_eq!(c.dsp.preamp_db, 12.0);
    }
}
