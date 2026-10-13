//! Playing downloads (SPEC D14: offline, a local queue the app owns), through the same
//! audio path as streaming: decode → DSP and levelling → ring → AAudio (or a bit-perfect USB
//! DAC), with the same signal path.
//!
//! [`OfflinePlayer`] runs an engine on its own thread, like [`super::LocalPlayer`]. The app
//! sets a queue of downloaded tracks (by stable key) and controls it: play, pause, next,
//! previous, seek, jump, shuffle and repeat. Each play heard is recorded in the
//! [`OfflineLibrary`]'s durable plays queue, to be sent to the server later (SPEC D6).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::audio::{AudioPath, AudioSource, OutputShared};
use super::engine::{STATUS_EVERY, TICK};
use super::local::{LocalSource, LocalTrack, SourceEvent, Transport, open_file};
use super::output::PullSlot;
use super::pipeline::DspSettings;
use super::queue::LocalRepeat;
use super::ring::{CHANNELS, Ring};
use super::{
    DUCK_GAIN, LocalPlayback, LocalPlayerConfig, OutputNegotiator, OutputRoute, PcmBlock,
    PcmIntBlock, RING_SAMPLES, SignalPathInfo, make_output, protocol, read_float, read_int,
    sanitise_config,
};
use crate::error::ApiError;
use crate::offline::OfflineLibrary;
use crate::offline::plays::now_secs;

/// Most tracks in a local queue.
pub const MAX_QUEUE: usize = 10_000;

/// What the offline player is doing.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflinePlayerStatus {
    pub playback: LocalPlayback,
    /// The track playing (or paused), as an index into the list given to
    /// [`OfflinePlayer::play_tracks`]; `None` with no queue.
    pub index: Option<u32>,
    /// The queue's stable keys, in the list's order.
    pub keys: Vec<String>,
    /// The play order (list indices); the list order unless shuffled.
    pub order: Vec<u32>,
    pub elapsed_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    pub repeat: LocalRepeat,
    /// Decoded audio queued ahead of the output.
    pub buffered_ms: u64,
    pub output_rate: u32,
    pub underruns: u64,
    pub held: bool,
    pub last_error: Option<String>,
}

impl Default for OfflinePlayerStatus {
    fn default() -> Self {
        OfflinePlayerStatus {
            playback: LocalPlayback::Stopped,
            index: None,
            keys: Vec::new(),
            order: Vec::new(),
            elapsed_ms: 0,
            duration_ms: 0,
            shuffle: false,
            repeat: LocalRepeat::Off,
            buffered_ms: 0,
            output_rate: 0,
            underruns: 0,
            held: false,
            last_error: None,
        }
    }
}

/// Receives updates, on the engine's thread.
#[uniffi::export(with_foreign)]
pub trait OfflinePlayerObserver: Send + Sync {
    fn on_status(&self, status: OfflinePlayerStatus);
    fn on_signal_path(&self, path: Option<SignalPathInfo>);
}

enum Cmd {
    Queue {
        tracks: Vec<LocalTrack>,
        start: usize,
        shuffle: bool,
        play: bool,
    },
    Play,
    Pause,
    Next,
    Previous,
    Jump(usize),
    Seek(u64),
    Shuffle(bool),
    Repeat(LocalRepeat),
    Stop,
    SetDsp(DspSettings),
    Renegotiate,
    Shutdown,
}

struct Shared {
    status: Mutex<OfflinePlayerStatus>,
    signal: Mutex<Option<SignalPathInfo>>,
    observer: Arc<dyn OfflinePlayerObserver>,
    out: Arc<OutputShared>,
}

struct Engine {
    audio: AudioPath,
    source: LocalSource,
    shared: Arc<Shared>,
    library: Arc<OfflineLibrary>,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    last_flush: u64,
    error: Option<String>,
    last_status_at: Option<Instant>,
    last_status_key: Option<(LocalPlayback, Option<u32>, bool, LocalRepeat, bool, u32)>,
    last_signal: Option<SignalPathInfo>,
    was_playing: bool,
    stopped: bool,
}

impl Engine {
    async fn run(mut self) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        self.publish_status(true);
        while !self.stopped {
            let active = self.source.queue().is_some();
            tokio::select! {
                cmd = self.cmds.recv() => match cmd {
                    Some(c) => self.command(c),
                    None => self.shutdown(),
                },
                _ = tick.tick(), if active => self.on_tick(),
                // Idle: wake now and then only to report (nothing to play).
                _ = tokio::time::sleep(Duration::from_secs(3600)), if !active => {}
            }
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Queue {
                tracks,
                start,
                shuffle,
                play,
            } => {
                self.error = None;
                self.source.set_queue(tracks, start, shuffle, play);
                if play {
                    self.shared.out.hold.store(false, Ordering::Release);
                }
            }
            Cmd::Play => {
                // An explicit play lifts a hold (headphones unplugged, focus lost).
                self.shared.out.hold.store(false, Ordering::Release);
                self.source.play();
            }
            Cmd::Pause => {
                self.source.pause();
                self.shared.out.ring.set_hold(true);
            }
            Cmd::Next => {
                self.source.next();
            }
            Cmd::Previous => self.source.previous(),
            Cmd::Jump(i) => {
                self.source.jump(i);
            }
            Cmd::Seek(ms) => self.source.seek(ms),
            Cmd::Shuffle(on) => self.source.set_shuffle(on),
            Cmd::Repeat(r) => self.source.set_repeat(r),
            Cmd::Stop => {
                self.source.stop();
                self.after_flush();
                self.close_output();
            }
            Cmd::SetDsp(d) => {
                self.audio.pipeline.set_dsp(d);
                self.publish_signal(true);
            }
            Cmd::Renegotiate => self.audio.renegotiate = true,
            Cmd::Shutdown => self.shutdown(),
        }
        if self.source.flush_count() != self.last_flush {
            self.after_flush();
        }
        self.record_events();
        self.publish_status(true);
    }

    fn after_flush(&mut self) {
        self.last_flush = self.source.flush_count();
        self.audio.drop_queued_audio();
        self.audio.forget_plan();
    }

    fn close_output(&mut self) {
        self.audio
            .close_output(Some(&mut self.source as &mut dyn AudioSource));
        self.audio.pipeline.unconfigure();
        self.publish_signal(true);
    }

    fn shutdown(&mut self) {
        self.source.stop();
        self.record_events();
        self.close_output();
        self.shared.out.ring.set_hold(true);
        self.publish_status(true);
        self.stopped = true;
    }

    fn on_tick(&mut self) {
        let ring = self.shared.out.ring.clone();
        let played = self.audio.played();
        if played > 0 {
            self.source.played(played);
        }
        self.source.decode_ahead();
        if self.source.flush_count() != self.last_flush {
            self.after_flush();
        }
        self.audio
            .poll_output(Some(&mut self.source as &mut dyn AudioSource));
        let source = &mut self.source;
        let facts: Vec<(u32, Option<super::transitions::TrackFacts>)> = source.fact_seqs();
        self.audio.fill(source, false, &|_| None, &|seq| {
            facts
                .iter()
                .find(|(s, _)| *s == seq)
                .and_then(|(_, f)| f.clone())
        });
        if let Some(e) = self.audio.take_error() {
            self.error = Some(e);
        }
        let playing = self.source.transport() == Transport::Playing;
        if playing && !self.was_playing {
            self.shared.out.hold.store(false, Ordering::Release);
        }
        self.was_playing = playing;
        // The end of the queue: once the last sample has left the ring.
        if playing && self.source.queue_exhausted() && ring.is_empty() && self.audio.idle() {
            self.source.finish();
            self.after_flush();
        }
        let user_hold = self.shared.out.hold.load(Ordering::Acquire);
        let playing = self.source.transport() == Transport::Playing;
        ring.set_hold(user_hold || !playing);
        ring.set_expect_data(
            playing && !user_hold && !(self.source.audio_exhausted() && self.audio.idle()),
        );
        self.record_events();
        self.publish_status(false);
        self.publish_signal(false);
    }

    /// Plays heard go to the durable plays queue; failures become the status's error.
    fn record_events(&mut self) {
        for e in self.source.take_events() {
            match e {
                SourceEvent::PlayEnded {
                    index,
                    started_at,
                    seconds,
                } => {
                    if let Some(t) = self.source.tracks().get(index) {
                        self.library.record_play(
                            t.track_id,
                            &t.key,
                            started_at,
                            seconds.round() as u32,
                        );
                    }
                }
                SourceEvent::Failed { error, .. } => {
                    self.error = Some(protocol::clean_name(&error));
                }
                SourceEvent::Started(_) => self.error = None,
                SourceEvent::Finished => {}
            }
        }
    }

    fn current_status(&self) -> OfflinePlayerStatus {
        let ring = &self.shared.out.ring;
        let rate = self.audio.pipeline.out_rate().unwrap_or(0);
        let ring_ms = if rate > 0 {
            (ring.len() / CHANNELS) as u64 * 1000 / u64::from(rate)
        } else {
            0
        };
        let q = self.source.queue();
        let playback = match self.source.transport() {
            Transport::Stopped => LocalPlayback::Stopped,
            Transport::Paused => LocalPlayback::Paused,
            Transport::Playing if self.source.playing_track().is_none() => LocalPlayback::Buffering,
            Transport::Playing => LocalPlayback::Playing,
        };
        OfflinePlayerStatus {
            playback,
            index: self.source.current_index().map(|i| i as u32),
            keys: self.source.tracks().iter().map(|t| t.key.clone()).collect(),
            order: q
                .map(|q| q.order().iter().map(|i| *i as u32).collect())
                .unwrap_or_default(),
            elapsed_ms: self.source.elapsed_ms(),
            duration_ms: self.source.duration_ms(),
            shuffle: q.is_some_and(|q| q.shuffle()),
            repeat: q.map_or(LocalRepeat::Off, |q| q.repeat()),
            buffered_ms: self.source.buffered_ms() + ring_ms,
            output_rate: rate,
            underruns: ring.underruns().0,
            held: self.shared.out.hold.load(Ordering::Acquire),
            last_error: self.error.clone(),
        }
    }

    fn publish_status(&mut self, force: bool) {
        let st = self.current_status();
        let key = (
            st.playback,
            st.index,
            st.shuffle,
            st.repeat,
            st.held,
            st.output_rate,
        );
        let due = self
            .last_status_at
            .is_none_or(|t| t.elapsed() >= STATUS_EVERY);
        let changed = self.last_status_key != Some(key);
        if let Ok(mut s) = self.shared.status.lock() {
            *s = st.clone();
        }
        if force || changed || due {
            self.last_status_at = Some(Instant::now());
            self.last_status_key = Some(key);
            self.shared.observer.on_status(st);
        }
    }

    fn publish_signal(&mut self, force: bool) {
        let force = force || self.audio.take_signal_dirty();
        let path = self.audio.signal_path(&self.source);
        if !force && path == self.last_signal {
            return;
        }
        self.last_signal = path.clone();
        if let Ok(mut s) = self.shared.signal.lock() {
            *s = path.clone();
        }
        self.shared.observer.on_signal_path(path);
    }
}

struct Run {
    shared: Arc<Shared>,
    cmds: mpsc::UnboundedSender<Cmd>,
    pull: Arc<Mutex<PullSlot>>,
    thread: std::thread::JoinHandle<()>,
}

/// Plays downloaded tracks on this phone, from a queue the app owns.
#[derive(uniffi::Object)]
pub struct OfflinePlayer {
    library: Arc<OfflineLibrary>,
    config: Mutex<LocalPlayerConfig>,
    run: Mutex<Option<Run>>,
    route: Mutex<OutputRoute>,
    hold: Mutex<bool>,
    duck: Mutex<bool>,
    negotiator: Mutex<Option<Arc<dyn OutputNegotiator>>>,
    /// The visualiser (kept across restarts of the engine; off by default).
    visual: Arc<super::visual::VisualShared>,
}

impl OfflinePlayer {
    fn send(&self, c: Cmd) {
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            let _ = r.cmds.send(c);
        }
    }
}

#[uniffi::export]
impl OfflinePlayer {
    #[uniffi::constructor]
    pub fn new(library: Arc<OfflineLibrary>, config: LocalPlayerConfig) -> Arc<Self> {
        Arc::new(OfflinePlayer {
            library,
            config: Mutex::new(sanitise_config(config)),
            run: Mutex::new(None),
            route: Mutex::new(OutputRoute::default()),
            hold: Mutex::new(false),
            duck: Mutex::new(false),
            negotiator: Mutex::new(None),
            visual: Arc::new(super::visual::VisualShared::default()),
        })
    }

    /// See [`super::LocalPlayer::set_visualiser`].
    pub fn set_visualiser(&self, on: bool) {
        self.visual.set_enabled(on);
    }

    /// See [`super::LocalPlayer::visual_frame`].
    pub fn visual_frame(&self) -> Option<Vec<u8>> {
        self.visual.take()
    }

    /// Start the engine (idle until a queue is set). Does nothing if already running.
    pub fn start(&self, observer: Arc<dyn OfflinePlayerObserver>) -> Result<(), ApiError> {
        let mut run = self.run.lock().unwrap();
        if run.as_ref().is_some_and(|r| !r.thread.is_finished()) {
            return Ok(());
        }
        let cfg = self.config.lock().unwrap().clone();
        let ring = Ring::new(RING_SAMPLES);
        ring.set_gain(if *self.duck.lock().unwrap() {
            DUCK_GAIN
        } else {
            1.0
        });
        ring.set_hold(true);
        let out = Arc::new(OutputShared {
            ring,
            hold: std::sync::atomic::AtomicBool::new(*self.hold.lock().unwrap()),
            route: Mutex::new(self.route.lock().unwrap().clone()),
            negotiator: Mutex::new(self.negotiator.lock().unwrap().clone()),
            visual: self.visual.clone(),
        });
        let shared = Arc::new(Shared {
            status: Mutex::new(OfflinePlayerStatus::default()),
            signal: Mutex::new(None),
            observer,
            out: out.clone(),
        });
        let pull = Arc::new(Mutex::new(PullSlot::default()));
        let output = make_output(cfg.output, &pull)?;
        let audio = AudioPath::new(out, output, cfg.dsp)?;
        let seed = u64::from(getrandom::u32().unwrap_or(0x9e37)) << 32 | now_secs();
        let source = LocalSource::new(Box::new(open_file), Box::new(now_secs), seed);
        let (cmds, cmd_rx) = mpsc::unbounded_channel();
        let engine = Engine {
            audio,
            source,
            shared: shared.clone(),
            library: self.library.clone(),
            cmds: cmd_rx,
            last_flush: 0,
            error: None,
            last_status_at: None,
            last_status_key: None,
            last_signal: None,
            was_playing: false,
            stopped: false,
        };
        let thread = std::thread::Builder::new()
            .name("anywhere-offline".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("offline player runtime");
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

    /// Play downloaded tracks (stable keys, in order) from list index `start`. Keys not on
    /// the device are left out (and `start` follows its track). Returns how many queued.
    pub fn play_tracks(
        &self,
        keys: Vec<String>,
        start: u32,
        shuffle: bool,
        play: bool,
    ) -> Result<u32, ApiError> {
        let mut tracks = Vec::new();
        let mut start_at = 0;
        for (i, k) in keys.iter().take(MAX_QUEUE).enumerate() {
            if !crate::offline::index::valid_key(k) {
                continue;
            }
            if let Some((t, path)) = self.library.file_for(k) {
                if i as u32 == start {
                    start_at = tracks.len();
                }
                tracks.push(LocalTrack {
                    key: t.key,
                    track_id: t.track_id,
                    path,
                    ext: t.ext,
                    duration_secs: t.duration_secs,
                    album: t.album_id.map(|id| id.to_string()),
                });
            }
        }
        if tracks.is_empty() {
            return Err(ApiError::invalid("none of these tracks is downloaded"));
        }
        if self.run.lock().unwrap().is_none() {
            return Err(ApiError::invalid("the player isn't started"));
        }
        let n = tracks.len() as u32;
        self.send(Cmd::Queue {
            tracks,
            start: start_at,
            shuffle,
            play,
        });
        Ok(n)
    }

    pub fn play(&self) {
        self.send(Cmd::Play);
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    pub fn next(&self) {
        self.send(Cmd::Next);
    }

    /// Back to the start of the track, or (within its first 3 s) the track before.
    pub fn previous(&self) {
        self.send(Cmd::Previous);
    }

    /// Play the queue's track at list index `index`.
    pub fn jump_to(&self, index: u32) {
        self.send(Cmd::Jump(index as usize));
    }

    pub fn seek_to(&self, ms: u64) {
        self.send(Cmd::Seek(ms));
    }

    pub fn set_shuffle(&self, on: bool) {
        self.send(Cmd::Shuffle(on));
    }

    pub fn set_repeat(&self, repeat: LocalRepeat) {
        self.send(Cmd::Repeat(repeat));
    }

    /// Stop and forget the queue (the output closes; the engine stays ready).
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }

    /// End the engine (the app is signing out, or the player is being replaced).
    pub fn shutdown(&self) {
        if let Some(r) = self.run.lock().unwrap().take() {
            r.shared.out.ring.set_hold(true);
            let _ = r.cmds.send(Cmd::Shutdown);
        }
    }

    pub fn is_running(&self) -> bool {
        self.run
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| !r.thread.is_finished())
    }

    pub fn status(&self) -> OfflinePlayerStatus {
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

    /// The Sound settings; they apply to audio processed from now on.
    pub fn set_dsp(&self, dsp: DspSettings) {
        let dsp = dsp.sanitised();
        self.config.lock().unwrap().dsp = dsp.clone();
        self.send(Cmd::SetDsp(dsp));
    }

    /// Silence the output at once (headphones unplugged, focus lost), keeping the queued
    /// audio; lifted by the next play.
    pub fn set_hold(&self, hold: bool) {
        *self.hold.lock().unwrap() = hold;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            r.shared.out.hold.store(hold, Ordering::Release);
            if hold {
                r.shared.out.ring.set_hold(true);
            }
        }
    }

    pub fn set_duck(&self, duck: bool) {
        *self.duck.lock().unwrap() = duck;
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            r.shared
                .out
                .ring
                .set_gain(if duck { DUCK_GAIN } else { 1.0 });
        }
    }

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

    pub fn set_output_negotiator(&self, negotiator: Option<Arc<dyn OutputNegotiator>>) {
        *self.negotiator.lock().unwrap() = negotiator.clone();
        if let Some(r) = self.run.lock().unwrap().as_ref() {
            *r.shared.out.negotiator.lock().unwrap() = negotiator;
            let _ = r.cmds.send(Cmd::Renegotiate);
        }
    }

    pub fn renegotiate_output(&self) {
        self.send(Cmd::Renegotiate);
    }

    /// [`super::OutputKind::Pull`] only: see [`super::LocalPlayer::read_pcm`].
    pub fn read_pcm(&self, max_frames: u32) -> PcmBlock {
        let pull = self.run.lock().unwrap().as_ref().map(|r| r.pull.clone());
        read_float(pull, max_frames)
    }

    /// [`super::OutputKind::Pull`] only: see [`super::LocalPlayer::read_pcm_int`].
    pub fn read_pcm_int(&self, max_frames: u32) -> PcmIntBlock {
        let pull = self.run.lock().unwrap().as_ref().map(|r| r.pull.clone());
        read_int(pull, max_frames)
    }
}

impl Drop for OfflinePlayer {
    fn drop(&mut self) {
        self.shutdown();
    }
}
