//! Slimproto player state machine.
//!
//! Behaves towards LMS like a squeezelite-class software player:
//! - `strm s` opens a stream, and the next track's stream is appended for gapless playback.
//! - Status events go back to LMS as STAT: `STMc` connect, `STMs` track started,
//!   `STMd` decoder done (LMS then sends the next track), `STMu` played out,
//!   `STMp`/`STMr` paused/resumed, `STMf` flushed, `STMt` heartbeat,
//!   `STMl` loaded (autostart off), `STMn` unsupported, `STMo` output underrun (reported
//!   by the host through [`Player::output_underrun`]).
//! - `strm q` flushes everything. `strm f` (sent when the next track, already streamed, is
//!   deleted or moved in LMS's queue) keeps the audible track playing and drops only what
//!   came after it; hosts follow it through [`Player::trim_count`] and [`QueueTrim`].
//!
//! Sans-I/O: the host feeds control bytes, stream bytes and a millisecond clock, executes
//! the [`Action`]s, and pulls PCM with [`Player::pull_audio`].
//!
//! ## Synchronisation
//!
//! LMS keeps synced players together using each STAT's `(jiffies, elapsed)` pair, a timed
//! start (`strm u` with a start time) and small corrections (`strm p`: pause for N ms;
//! `strm a`: skip N ms). We advertise `AccuratePlayPoints`, so LMS uses every pair as it
//! is. For sync to work, the pair must describe what is *audible*:
//! - The clock (`now_ms`, which is also the jiffies) must be monotonic. It must also be the
//!   clock the host uses to time its output.
//! - The host reports frames as they are presented (heard), not as they are handed to its
//!   output, using [`Player::presented`]. It passes the frames newly presented and the host
//!   time at which that count was exact. Each STAT then extrapolates elapsed from that
//!   moment to its own jiffies, so the pair stays consistent to within about a millisecond
//!   whenever it is sent. ([`Player::played`] without a time still works, as before.)
//! - With [`Player::set_output_corrections`] on, the host applies the timed start, pauses
//!   and skips at its output stage (see [`OutputCorrection`]). They are then sample-accurate
//!   and don't wait for the audio already queued. The host may also pull audio ahead while
//!   a start is pending.

use std::collections::VecDeque;

use crate::decode::{DecodeStatus, InputBuffer, PROBE_MAX_BYTES, SourceFormat, StreamDecoder};
use crate::proto::{ClientMsg, DEVICE_ID_SOFTWARE, Helo, ServerFrameReader, ServerMsg, Stat, Strm};

/// Stop reading the network stream once this many compressed bytes are waiting.
pub const INPUT_HIGH_WATER: usize = 8 * 1024 * 1024;
/// Decode ahead until this many seconds of PCM are queued.
pub const OUTPUT_TARGET_SECS: u32 = 10;
/// Packets decoded per `tick`, to bound the time spent per call.
const PACKETS_PER_TICK: usize = 48;
const HEARTBEAT_MS: u64 = 1000;
/// Never extrapolate elapsed time more than this far past the host's last position report.
/// Beyond that the host has stopped reporting (or its output has stalled), so report only
/// what is known. A second: a browser in the background may only report once a second.
pub const MAX_EXTRAPOLATE_MS: u64 = 1000;
/// The longest pause or skip correction accepted from LMS (it sends at most 10 s).
const MAX_CORRECTION_MS: u32 = 10_000;
/// A timed start further ahead than this is not a real one (a corrupt or hostile frame), so
/// the player starts at once instead. LMS sends starts a few hundred milliseconds ahead.
const MAX_START_AHEAD_MS: u64 = 30_000;
/// With quick start on, the stream must be arriving at least this many times faster than it
/// plays. A live stream (radio) arrives at about real time, so it keeps waiting for LMS's
/// threshold and the cushion that gives it.
const QUICK_START_SPEEDUP: u64 = 2;
/// In read-ahead mode, `STMd` waits until this much of the track has been heard (see
/// [`Player::set_ready_after_ms`]).
pub const DEFAULT_READY_AFTER_MS: u64 = 20_000;
/// After the link to LMS comes back, how long a deferred `STMu` waits for LMS to send the
/// next track (it answers the `STMd` that waited in the host's outbox within milliseconds).
pub const LINK_SETTLE_MS: u64 = 5_000;
/// A player in a sync group drops a stall longer than this on resume (see
/// [`Player::resume_after_stall`]).
pub const SYNC_STALL_SKIP_MS: u64 = 50;

#[derive(Debug, Clone)]
pub struct PlayerConfig {
    pub mac: [u8; 6],
    pub uuid: [u8; 16],
    /// Name shown in LMS.
    pub name: String,
    /// Codec capability tokens advertised to LMS, e.g. `["flc", "mp3", "ogg"]`.
    pub codecs: Vec<String>,
    pub max_sample_rate: u32,
    pub firmware: String,
}

impl PlayerConfig {
    pub fn capabilities(&self) -> String {
        // `Model=squeezelite` gets the full software-player treatment (gapless, codec
        // negotiation) from LMS; `ModelName` is what users see.
        let mut caps = format!(
            "Model=squeezelite,ModelName=Lyrion Anywhere,AccuratePlayPoints=1,HasDigitalOut=1,\
             Firmware={},MaxSampleRate={}",
            self.firmware, self.max_sample_rate
        );
        for c in &self.codecs {
            caps.push(',');
            caps.push_str(c);
        }
        caps
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamHost {
    /// The slimproto server's own address (`server_ip` = 0).
    Server,
    Ip([u8; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes to write to the slimproto (control) connection.
    Send(Vec<u8>),
    /// Open an HTTP stream: connect to `host:port` and send `request` verbatim.
    /// Through the gateway, fetch `/v1/stream/<session>/<stream_number>` instead.
    OpenStream {
        host: StreamHost,
        port: u16,
        request: Vec<u8>,
        /// 1-based count of every `strm s` received on this connection, including ones
        /// refused locally, so it matches the gateway's numbering exactly.
        stream_number: u64,
        /// The `strm` format byte (`f` FLAC, `u` Ogg Opus, `m` MP3, …).
        format: u8,
    },
    /// Abandon the current HTTP stream.
    CloseStream,
}

/// A synchronisation correction from LMS, for a host that applies them at its output stage
/// (see [`Player::set_output_corrections`]). Applied there, they take effect at once and to
/// the sample, instead of after the audio the host has queued ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputCorrection {
    /// Play `ms` of silence now, consuming no audio, then carry on. Sent as `strm p` with an
    /// interval: this player is ahead of the group.
    Pause { ms: u32 },
    /// Drop the next `ms` of audio now. Sent as `strm a`: this player is behind. Report the
    /// skipped frames through [`Player::presented`] or [`Player::played`], like played ones.
    Skip { ms: u32 },
    /// Make the first audible frame of the pulled audio play at host time `at_ms`. Sent as
    /// `strm u` with a start time. Audio may be pulled ahead until then. If the output can no
    /// longer make that time, it should start at once and drop the audio that would already
    /// have played, so the player is still in step.
    StartAt { at_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputState {
    Stopped,
    /// Buffering before autostart.
    Waiting,
    /// Threshold reached with autostart off; waiting for `strm u`.
    Loaded,
    Playing,
    /// Paused; resumes automatically at `until` (host time, in ms) if set.
    Paused {
        until: Option<u64>,
    },
}

/// A run of PCM in the output queue belonging to one track.
#[derive(Debug)]
struct Segment {
    seq: u32,
    rate: u32,
    /// Frames appended to the queue so far.
    frames: u64,
    pulled: u64,
    played: u64,
    /// Decoder finished: no more frames will be appended.
    complete: bool,
    /// What the track's stream turned out to be (known once its headers are probed).
    source: Option<SourceFormat>,
    /// The server's ReplayGain factor from `strm` (1.0 = none), already applied.
    server_gain: f32,
}

/// What a host needs to know about one track's audio: for the signal-path display and for
/// gain decisions made outside the core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackAudio {
    /// The track's sequence number in this connection (as returned by
    /// [`Player::pull_track`]).
    pub seq: u32,
    pub sample_rate: u32,
    pub source: Option<SourceFormat>,
    /// The server's ReplayGain factor from `strm` (1.0 = none), already applied to the PCM.
    pub server_gain: f32,
}

struct Track {
    seq: u32,
    input: InputBuffer,
    decoder: StreamDecoder,
    autostart: u8,
    threshold: usize,
    /// Bytes received from the network: the bytes fed, or more if the host spools the
    /// stream ahead of the decoder and says so ([`Player::stream_bytes_received`]).
    bytes_received: u64,
    /// Bytes handed to the decoder ([`Player::on_stream_data`]).
    bytes_fed: u64,
    /// Frames the decoder has produced from them, and their rate: with the bytes it has
    /// consumed, the track's byte rate ([`Player::decoded_rate`]).
    decoded_frames: u64,
    decoded_rate: u32,
    replay_gain: f32,
    /// The network stream has ended (`DSCO` sent), or was never opened: nothing to fetch.
    stream_ended: bool,
    /// No more bytes will be fed to the decoder.
    input_ended: bool,
    decode_done: bool,
    /// `STMd` has been sent for this track.
    ready_sent: bool,
    /// When the stream was requested (host clock, ms).
    opened_at: u64,
}

/// What a partial flush (`strm f`) kept, for hosts that queue pulled audio ahead of their
/// output (see [`Player::trim`]).
///
/// LMS sends `strm f` when the next track, already streamed, is deleted or moved in its
/// queue. The audible track plays on; everything after it is dropped. A host must then drop
/// the audio it holds from any track after `keep_track`, and keep the rest:
/// - a host that pulls with [`Player::pull_track`] drops what it holds of tracks with a
///   sequence number greater than `keep_track`;
/// - a host that pulls with [`Player::pull_audio`] keeps the first `keep_frames` frames it
///   holds (pulled but not yet reported as played) and drops the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueTrim {
    /// The audible track, which was kept.
    pub keep_track: u32,
    /// Frames of `keep_track` (at its own rate) pulled but not yet reported as played when
    /// the trim happened: the audio the host holds that it must keep.
    pub keep_frames: u64,
}

/// Something LMS told the player, or the player told LMS, kept for a host's diagnostics log
/// (see [`Player::set_diagnostics`]). Only the protocol's small fixed fields: never a stream
/// request, an address or a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagEvent {
    /// A `strm` from LMS (every command but `t`, the status poll every few seconds).
    Strm {
        command: u8,
        autostart: u8,
        format: u8,
        /// Input buffer threshold in KiB (meaningful for `s`).
        threshold: u8,
    },
    /// A STAT the player sent (every event but `STMt`, the reply to that poll).
    Stat([u8; 4]),
    /// A stream announced as FLAC had no header (LMS serves a seek as bare frames): one was
    /// made from its first frame, with these parameters.
    FlacHeader { rate: u32, channels: u32, bits: u32 },
}

/// At most this many [`DiagEvent`]s wait for the host; the oldest go first.
pub const MAX_DIAG_EVENTS: usize = 64;

/// Snapshot for UIs and logs.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub output: OutputState,
    pub elapsed_ms: u64,
    pub buffered_ms: u64,
    /// Stream bytes handed to the player and not yet decoded: the track being decoded and,
    /// in read-ahead mode, the queued next track (which the host feeds whole into the
    /// player once the current track's input has ended). A host's "buffered ahead" figure
    /// counts them with what it still spools (see [`Player::decoded_rate`]).
    pub input_bytes: usize,
    pub sample_rate: u32,
    pub volume: (f32, f32),
    pub name: String,
    pub last_error: Option<String>,
}

pub struct Player {
    cfg: PlayerConfig,
    reader: ServerFrameReader,
    actions: Vec<Action>,
    now: u64,
    /// The track being decoded (or, once decoded, the last one).
    track: Option<Track>,
    /// Read-ahead mode only: a track whose `strm s` came while `track` was still decoding.
    /// It is fetched now and decoded once `track` finishes. LMS never streams more than one
    /// track ahead (it waits for the next track's `STMs`), so one slot is enough.
    next: Option<Track>,
    next_seq: u32,
    /// Interleaved stereo PCM, in segment order.
    pcm: VecDeque<f32>,
    segments: VecDeque<Segment>,
    output: OutputState,
    gain: (f32, f32),
    /// Segment currently audible (for elapsed time and STMs).
    playing_seq: Option<u32>,
    last_heartbeat: u64,
    stmu_sent: bool,
    last_error: Option<String>,
    /// Flushes so far (`strm q`, and `strm f` with nothing audible), so hosts can drop audio
    /// they had queued.
    flushes: u64,
    /// Partial flushes so far (`strm f` keeping the audible track), and the latest one.
    trims: u64,
    last_trim: Option<QueueTrim>,
    /// Bytes of pulled audio the host holds ahead of its output (e.g. a ring buffer), and
    /// that buffer's capacity, both counted in STAT's output buffer figures.
    host_buffered: (u64, u64),
    /// An underrun has been reported (STMo) and audio has not flowed since.
    underrun_reported: bool,
    /// The host never delivered that report (see [`Player::withdraw_underrun_report`]), so
    /// the audio flowing again is not reported either.
    underrun_withdrawn: bool,
    /// Ignore `audg`: samples leave at 100 % (see [`Player::set_fixed_volume`]).
    fixed_volume: bool,
    /// The host applies the volume at its output (see [`Player::set_host_volume`]).
    host_volume: bool,
    /// Start once this much audio is decoded, if it is arriving quickly (see
    /// [`Player::set_quick_start`]).
    quick_start_ms: Option<u32>,
    /// The host time at which the count of played frames was exact (see
    /// [`Player::presented`]); `None` for hosts that report without a time.
    position_at: Option<u64>,
    /// The host applies sync corrections at its output (see
    /// [`Player::set_output_corrections`]).
    output_corrections: bool,
    corrections: Vec<OutputCorrection>,
    /// A pause correction runs until this host time (ms): no extrapolation until then.
    corrected_until: u64,
    /// Read-ahead mode (see [`Player::set_ready_when_received`]).
    ready_when_received: bool,
    /// In read-ahead mode, how much of a track must be heard before `STMd` is sent early.
    ready_after_ms: u64,
    /// The host's link to LMS is up (see [`Player::set_link_up`]), and since when.
    link_up: bool,
    link_up_at: u64,
    /// The audio ran out with the link down, so `STMo` was sent in place of `STMu`; `STMu`
    /// waits for the link to have been back for [`LINK_SETTLE_MS`].
    stmu_deferred: bool,
    /// The last track `STMs` was sent for: never twice for one track.
    stms_seq: Option<u32>,
    /// The output has been stalled (an underrun) since this host time.
    stall_since: Option<u64>,
    /// Diagnostics for the host (see [`Player::set_diagnostics`]): `None` when off.
    diag: Option<VecDeque<DiagEvent>>,
    /// Tests: every `STMs` and `STMd` sent, with the track it was for.
    #[cfg(test)]
    sent_log: Vec<(&'static str, u32)>,
}

/// What [`Player::resume_after_stall`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallResume {
    /// The output was not stalled.
    NotStalled,
    /// Carry on from where the audio stopped: the player is alone, or the stall was short.
    CarryOn { stalled_ms: u64 },
    /// `stalled_ms` of audio were dropped, so the player is back in step with its group.
    Skipped { stalled_ms: u64 },
    /// In a group, but less audio is decoded than the stall: the host should flush and let
    /// LMS restart the player at the group's position.
    NotEnoughBuffered { stalled_ms: u64, buffered_ms: u64 },
}

impl Player {
    pub fn new(cfg: PlayerConfig) -> Self {
        Player {
            cfg,
            reader: ServerFrameReader::default(),
            actions: Vec::new(),
            now: 0,
            track: None,
            next_seq: 0,
            pcm: VecDeque::new(),
            segments: VecDeque::new(),
            output: OutputState::Stopped,
            gain: (1.0, 1.0),
            playing_seq: None,
            last_heartbeat: 0,
            stmu_sent: true,
            last_error: None,
            flushes: 0,
            trims: 0,
            last_trim: None,
            host_buffered: (0, 0),
            underrun_reported: false,
            underrun_withdrawn: false,
            fixed_volume: false,
            host_volume: false,
            quick_start_ms: None,
            position_at: None,
            output_corrections: false,
            corrections: Vec::new(),
            corrected_until: 0,
            next: None,
            ready_when_received: false,
            ready_after_ms: DEFAULT_READY_AFTER_MS,
            link_up: true,
            link_up_at: 0,
            stmu_deferred: false,
            stms_seq: None,
            stall_since: None,
            diag: None,
            #[cfg(test)]
            sent_log: Vec::new(),
        }
    }

    /// Keep each `strm` received and STAT sent for the host's diagnostics log (off by
    /// default). At most [`MAX_DIAG_EVENTS`] are kept until [`Player::take_diagnostics`].
    pub fn set_diagnostics(&mut self, on: bool) {
        self.diag = on.then(VecDeque::new);
    }

    /// The diagnostics events since the last call, oldest first.
    pub fn take_diagnostics(&mut self) -> Vec<DiagEvent> {
        self.diag
            .as_mut()
            .map_or_else(Vec::new, |d| d.drain(..).collect())
    }

    fn diag(&mut self, e: DiagEvent) {
        if let Some(d) = &mut self.diag {
            if d.len() >= MAX_DIAG_EVENTS {
                d.pop_front();
            }
            d.push_back(e);
        }
    }

    /// Tests: record a track event (`STMs`, `STMd`) and the track it was for.
    fn note_sent(&mut self, _event: &'static str, _seq: u32) {
        #[cfg(test)]
        self.sent_log.push((_event, _seq));
    }

    /// Read-ahead mode, for a host that fetches whole tracks ahead (a disk spool). Off by
    /// default, and then nothing changes. When on:
    /// - `STMd` (which makes LMS send the next track) is sent once per track, as soon as its
    ///   whole stream is received ([`Player::stream_received`] or [`Player::on_stream_end`])
    ///   and at least [`Player::set_ready_after_ms`] of it has been heard; or, at the latest,
    ///   when it finishes decoding, as before.
    /// - A `strm s` that arrives while the current track is still decoding (its stream fully
    ///   received) is queued as the next track instead of replacing it. Its stream is fetched
    ///   at once and it is decoded when the current one finishes, gaplessly.
    /// - [`Player::open_stream`], STAT `bytes_received` and `HELO` refer to the track being
    ///   fetched (the newest); [`Player::on_stream_data`] and [`Player::wants_stream_data`]
    ///   to the track being fed (the oldest whose input hasn't ended). Elapsed time, `STMs`,
    ///   segments and gapless playback follow the audible track, as always.
    /// - `STMu` is sent only when every track has played out and nothing is queued.
    /// - After an `STMo`, the first audio heard again sends `STMr` (resumed), unless it is a
    ///   new track, which sends `STMs`. Never a second `STMs` for one track: in a sync group
    ///   that moves the whole group on a track.
    pub fn set_ready_when_received(&mut self, on: bool) {
        self.ready_when_received = on;
    }

    /// In read-ahead mode, how much of a track must have been heard before `STMd` is sent
    /// early (default [`DEFAULT_READY_AFTER_MS`]). This saves fetching the next track for
    /// someone skipping quickly through tracks.
    pub fn set_ready_after_ms(&mut self, ms: u64) {
        self.ready_after_ms = ms;
    }

    /// Whether the host's link to LMS is up (default: up). If the audio runs out while the
    /// link is down, `STMo` is sent instead of `STMu`, so LMS doesn't treat the queue as
    /// finished (`STMu` with the next track already received made LMS stop); `STMu` follows
    /// only once the link has been back [`LINK_SETTLE_MS`] and LMS still sent nothing new.
    pub fn set_link_up(&mut self, up: bool, now_ms: u64) {
        if up && !self.link_up {
            self.link_up_at = now_ms;
        }
        self.link_up = up;
    }

    /// How long the output has been stalled (an underrun reported by
    /// [`Player::output_underrun`], or the audio running out with the link down), if it is
    /// stalled now. A stall ends when audio is heard again or [`Player::resume_after_stall`]
    /// is called.
    pub fn stall_ms(&self, now_ms: u64) -> Option<u64> {
        self.stall_since.map(|at| now_ms.saturating_sub(at))
    }

    /// Call when audio is available again after a stall, before pulling it. The policy is
    /// the host's: `in_sync_group` says whether the player is synced with others (LMS
    /// `sync ?`). Alone, the player carries on where it stopped. In a group, a stall over
    /// [`SYNC_STALL_SKIP_MS`] drops that much audio, so the player is back in step with the
    /// others (LMS's small corrections do the rest); if less than that is decoded, nothing
    /// is dropped and [`StallResume::NotEnoughBuffered`] says the host should flush and let
    /// LMS restart the player.
    pub fn resume_after_stall(&mut self, in_sync_group: bool, now_ms: u64) -> StallResume {
        self.now = now_ms;
        let Some(since) = self.stall_since.take() else {
            return StallResume::NotStalled;
        };
        let stalled_ms = now_ms.saturating_sub(since);
        if !in_sync_group || stalled_ms <= SYNC_STALL_SKIP_MS {
            return StallResume::CarryOn { stalled_ms };
        }
        let buffered_ms = self.buffered_ms();
        if buffered_ms < stalled_ms {
            return StallResume::NotEnoughBuffered {
                stalled_ms,
                buffered_ms,
            };
        }
        self.skip_ahead(stalled_ms);
        StallResume::Skipped { stalled_ms }
    }

    /// Drop the next `ms` of decoded audio, counting it as played (elapsed time moves on).
    /// Returns the milliseconds dropped, which is less if less is decoded.
    pub fn skip_audio(&mut self, ms: u64) -> u64 {
        self.skip_ahead(ms)
    }

    /// Let the host apply LMS's sync corrections at its output stage. The timed start, the
    /// pause for an interval and the skip ahead are then handed over as
    /// [`OutputCorrection`]s (see [`Player::take_corrections`]) instead of being applied to
    /// the core's queue. Audio may also be pulled ahead while loaded or waiting for a timed
    /// start; the host keeps it silent until the start. Off by default: the core applies
    /// the corrections itself, as before.
    pub fn set_output_corrections(&mut self, on: bool) {
        self.output_corrections = on;
        if !on {
            self.corrections.clear();
        }
    }

    /// Corrections for the host to apply at its output, oldest first. Always empty unless
    /// [`Player::set_output_corrections`] is on.
    pub fn take_corrections(&mut self) -> Vec<OutputCorrection> {
        std::mem::take(&mut self.corrections)
    }

    /// Report that `frames` more of the pulled audio have been presented (are audible), and
    /// that the total was exact at host time `at_ms` (the same clock as `now_ms`). Call it
    /// regularly while playing, even with 0 frames. Elapsed time in each STAT is then
    /// extrapolated from that moment to the STAT's own jiffies.
    pub fn presented(&mut self, frames: u64, at_ms: u64) {
        self.played(frames);
        self.position_at = Some(at_ms);
    }

    /// Leave LMS's volume (`audg`) to the host. Pulled audio is then at full scale, and the
    /// host applies [`Player::volume`] at its output, where a change is heard at once rather
    /// than after the audio it has queued ahead (seconds, for a media element).
    pub fn set_host_volume(&mut self, on: bool) {
        self.host_volume = on;
    }

    /// Start playing a stream (after a seek, or from stopped) as soon as `ms` of audio is
    /// decoded, instead of waiting for all of LMS's buffer threshold, provided the stream
    /// is arriving at least twice as fast as real time: a library file, not live radio.
    /// The rest of the buffer then fills while playing. `None` (the default) keeps the
    /// threshold alone, as squeezelite does.
    pub fn set_quick_start(&mut self, ms: Option<u32>) {
        self.quick_start_ms = ms.filter(|m| *m > 0);
    }

    /// Tell the player how much pulled audio the host holds ahead of its output, and how
    /// much it can hold (bytes, at 8 bytes per stereo frame), so STAT's output buffer
    /// figures describe the whole path to the speaker.
    pub fn set_host_buffer(&mut self, buffered_bytes: u64, capacity_bytes: u64) {
        self.host_buffered = (buffered_bytes, capacity_bytes.max(buffered_bytes));
    }

    /// The host's output ran dry while playing. If the current track is still arriving
    /// (an underrun, not the end of the audio), LMS is told once with `STMo`, as hardware
    /// players do; reported again only after audio has flowed in between.
    ///
    /// The stall is timed from the first report (see [`Player::stall_ms`]).
    pub fn output_underrun(&mut self) {
        if self.output == OutputState::Playing && self.stall_since.is_none() {
            self.stall_since = Some(self.now);
        }
        let streaming = self.decoding();
        if self.output == OutputState::Playing && streaming && !self.underrun_reported {
            self.underrun_reported = true;
            self.send_stat(*b"STMo", 0);
        }
    }

    /// The host dropped the `STMo` it was holding for LMS (a sync group member that rejoins
    /// after a gap handles the stall itself, and LMS must not pause the whole group for a
    /// report that is stale by then). The stall goes on as before: it is still timed, and no
    /// second `STMo` is sent for it; but since LMS never heard of it, the audio flowing
    /// again sends no `STMr`. Does nothing unless an underrun is reported and unrecovered.
    pub fn withdraw_underrun_report(&mut self) {
        if self.underrun_reported {
            self.underrun_withdrawn = true;
        }
    }

    /// How many times the server has flushed the player. When this changes, a host that
    /// queues pulled audio ahead of the output must drop that queue.
    ///
    /// `strm q` always counts. `strm f` counts only when nothing is audible; otherwise it is
    /// a partial flush that keeps the audible track (see [`Player::trim_count`]).
    pub fn flush_count(&self) -> u64 {
        self.flushes
    }

    /// How many partial flushes (`strm f` while a track is audible) there have been. When
    /// this changes, a host that queues pulled audio ahead of its output drops what it holds
    /// beyond the audible track, as [`Player::trim`] describes. A host that ignores it plays
    /// at most its own queue of the dropped track (it holds nothing else of it).
    pub fn trim_count(&self) -> u64 {
        self.trims
    }

    /// The latest partial flush, if any (see [`QueueTrim`]).
    pub fn trim(&self) -> Option<QueueTrim> {
        self.last_trim
    }

    /// The player's MAC address (as sent in HELO).
    pub fn mac(&self) -> [u8; 6] {
        self.cfg.mac
    }

    /// Call when the control connection is (re)established. Sends HELO.
    pub fn connected(&mut self, now_ms: u64) {
        self.now = now_ms;
        self.reader = ServerFrameReader::default();
        let helo = Helo {
            device_id: DEVICE_ID_SOFTWARE,
            revision: 0,
            mac: self.cfg.mac,
            uuid: self.cfg.uuid,
            wlan_channels: 0,
            bytes_received: self.fetching().map_or(0, |t| t.bytes_received),
            lang: *b"en",
            capabilities: self.cfg.capabilities(),
        };
        self.send(ClientMsg::Helo(helo));
    }

    /// Bytes received on the control connection.
    pub fn on_control_data(&mut self, data: &[u8], now_ms: u64) -> Result<(), String> {
        self.now = now_ms;
        self.reader.push(data);
        loop {
            match self.reader.next_msg() {
                Ok(Some(msg)) => self.handle(msg),
                Ok(None) => return Ok(()),
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    pub fn on_stream_headers(&mut self, headers: &[u8], now_ms: u64) {
        self.now = now_ms;
        self.send(ClientMsg::Resp(headers.to_vec()));
    }

    /// Stream bytes for the decoder. They go to the track being fed: the current track, or,
    /// in read-ahead mode once the current track's input has ended, the queued next track.
    pub fn on_stream_data(&mut self, data: &[u8], now_ms: u64) {
        self.now = now_ms;
        if let Some(t) = self.feeding_mut() {
            t.input.push(data);
            t.bytes_fed += data.len() as u64;
            t.bytes_received = t.bytes_received.max(t.bytes_fed);
        }
    }

    /// The HTTP stream finished normally, and every byte of it has been fed: the decoder's
    /// input ends. Sends `DSCO` unless [`Player::stream_received`] already did.
    pub fn on_stream_end(&mut self, now_ms: u64) {
        self.end_stream(0, now_ms);
    }

    /// The HTTP stream failed (reset or connect failure): the decoder's input ends with
    /// what was fed. A host that spools ahead feeds what it has first.
    pub fn on_stream_error(&mut self, now_ms: u64) {
        self.end_stream(1, now_ms);
    }

    fn end_stream(&mut self, reason: u8, now_ms: u64) {
        self.now = now_ms;
        let Some(t) = self.feeding_mut() else { return };
        if t.stream_ended && t.input_ended {
            return;
        }
        if !t.input_ended {
            t.input_ended = true;
            t.input.set_eof();
        }
        if !t.stream_ended {
            t.stream_ended = true;
            self.send(ClientMsg::Dsco(reason));
        }
    }

    /// For a host that spools the network stream ahead of the decoder: the stream being
    /// fetched has been received in full, though not all of it has been fed yet. Sends
    /// `DSCO`, and in read-ahead mode lets `STMd` go early (see
    /// [`Player::set_ready_when_received`]). The host still calls [`Player::on_stream_end`]
    /// once it has fed the last byte. A host without a spool just calls `on_stream_end`.
    pub fn stream_received(&mut self, now_ms: u64) {
        self.now = now_ms;
        let Some(t) = self.fetching_mut().filter(|t| !t.stream_ended) else {
            return;
        };
        t.stream_ended = true;
        self.send(ClientMsg::Dsco(0));
    }

    /// For a host that spools the network stream ahead of the decoder: the total bytes
    /// received so far for the stream being fetched. It counts in STAT, `HELO` and
    /// [`Player::open_stream`]; it never goes backwards, nor below the bytes fed.
    pub fn stream_bytes_received(&mut self, total: u64) {
        if let Some(t) = self.fetching_mut().filter(|t| !t.stream_ended) {
            t.bytes_received = t.bytes_received.max(total);
        }
    }

    /// The stream currently being downloaded, as `(stream_number, bytes_received)`, or
    /// `None` if none is open. After a reconnect, the host resumes the download at this
    /// byte offset (without passing the response headers in again). In read-ahead mode it
    /// is the newest track's stream, which may be the queued next track's.
    pub fn open_stream(&self) -> Option<(u64, u64)> {
        self.fetching()
            .filter(|t| !t.stream_ended)
            .map(|t| (t.seq as u64 + 1, t.bytes_received))
    }

    /// The streams the decoder still takes bytes for (their input hasn't ended), oldest
    /// first, as stream numbers: at most the current track and the queued next one. A host
    /// that spools streams keeps a spool for exactly these. Any other spool is for a track
    /// the player has dropped (a flush, a partial flush, or a stream replaced), and can go.
    pub fn fed_streams(&self) -> Vec<u64> {
        [self.track.as_ref(), self.next.as_ref()]
            .into_iter()
            .flatten()
            .filter(|t| !t.input_ended)
            .map(|t| t.seq as u64 + 1)
            .collect()
    }

    /// The track whose stream is fetched (the newest).
    fn fetching(&self) -> Option<&Track> {
        self.next.as_ref().or(self.track.as_ref())
    }

    fn fetching_mut(&mut self) -> Option<&mut Track> {
        self.next.as_mut().or(self.track.as_mut())
    }

    /// The track the decoder's input is fed to: the current track until its input ends,
    /// then the queued next one.
    fn feeding(&self) -> Option<&Track> {
        if self.feeds_next() {
            self.next.as_ref()
        } else {
            self.track.as_ref()
        }
    }

    fn feeding_mut(&mut self) -> Option<&mut Track> {
        if self.feeds_next() {
            self.next.as_mut()
        } else {
            self.track.as_mut()
        }
    }

    /// The queued next track is the one being fed: the current track's input has ended.
    fn feeds_next(&self) -> bool {
        self.next.is_some() && self.track.as_ref().is_none_or(|t| t.input_ended)
    }

    /// A track is still being decoded, or one is queued behind it.
    fn decoding(&self) -> bool {
        self.next.is_some() || self.track.as_ref().is_some_and(|t| !t.decode_done)
    }

    /// Whether the host should keep reading the network stream (back-pressure).
    ///
    /// While the decoder is still probing the stream's headers (which can carry several MiB
    /// of artwork) it keeps reading up to [`PROBE_MAX_BYTES`], so a probe that needs more
    /// than [`INPUT_HIGH_WATER`] can't stall; beyond that the track fails cleanly.
    ///
    /// In read-ahead mode it refers to the track being fed (see
    /// [`Player::on_stream_data`]). A queued next track isn't decoding yet, so it is held to
    /// [`INPUT_HIGH_WATER`] until it is.
    pub fn wants_stream_data(&self) -> bool {
        let queued = self.feeds_next();
        self.feeding().is_some_and(|t| {
            // Nothing more is fed to an ended input (e.g. a refused codec's track).
            if t.input_ended {
                return false;
            }
            let limit = if t.decoder.is_probing() && !t.decode_done && !queued {
                PROBE_MAX_BYTES
            } else {
                INPUT_HIGH_WATER
            };
            t.input.unread() < limit
        })
    }

    /// Do decode work, autostart, timed resume, heartbeats and underrun detection.
    pub fn tick(&mut self, now_ms: u64) {
        self.now = now_ms;

        if let OutputState::Paused { until: Some(at) } = self.output
            && self.now >= at
        {
            self.output = OutputState::Playing;
        }

        self.decode_some();
        self.check_autostart();
        self.check_ready();

        if self.output == OutputState::Playing
            && now_ms.saturating_sub(self.last_heartbeat) >= HEARTBEAT_MS
        {
            self.last_heartbeat = now_ms;
            self.send_stat(*b"STMt", 0);
        }

        // Everything decoded and played out: tell LMS the track (and playlist) finished.
        let drained = self
            .segments
            .iter()
            .all(|s| s.complete && s.played == s.frames);
        if self.output == OutputState::Playing && drained && !self.decoding() && !self.stmu_sent {
            let settled =
                !self.stmu_deferred || now_ms.saturating_sub(self.link_up_at) >= LINK_SETTLE_MS;
            if self.link_up && settled {
                self.stmu_sent = true;
                self.output = OutputState::Stopped;
                self.send_stat(*b"STMu", 0);
            } else if !self.link_up && !self.stmu_deferred {
                // Out of audio with LMS out of reach: an underrun, not the end. `STMu` now
                // would make LMS stop even if it has the next track ready to send.
                self.stmu_deferred = true;
                self.underrun_reported = true;
                self.stall_since.get_or_insert(now_ms);
                self.send_stat(*b"STMo", 0);
            }
        }
    }

    /// Read-ahead mode: send `STMd` for the newest track once its stream is in and enough of
    /// it has been heard.
    fn check_ready(&mut self) {
        if !self.ready_when_received {
            return;
        }
        let heard = self.elapsed_ms();
        let playing = self.playing_seq;
        let after = self.ready_after_ms;
        let Some(t) = self.fetching_mut() else { return };
        if t.ready_sent || t.decode_done || !t.stream_ended {
            return;
        }
        if playing == Some(t.seq) && heard >= after {
            t.ready_sent = true;
            let seq = t.seq;
            self.note_sent("STMd", seq);
            self.send_stat(*b"STMd", 0);
        }
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Sample rate of the next PCM to be pulled. The host should (re)open its audio output at
    /// this rate when it changes; `pull_audio` never crosses a rate change.
    pub fn output_rate(&self) -> Option<u32> {
        self.segments
            .iter()
            .find(|s| s.pulled < s.frames || !s.complete)
            .map(|s| s.rate)
    }

    /// Fill `out` (interleaved stereo) with up to `out.len() / 2` frames at `output_rate()`.
    /// Returns frames written; the host plays silence for the rest. Call [`Player::played`]
    /// once the frames have actually been played (immediately, for low-latency outputs).
    pub fn pull_audio(&mut self, out: &mut [f32]) -> usize {
        self.pull(out, false).0
    }

    /// Like [`Player::pull_audio`], but never crosses into the next track, so a host can
    /// apply per-track processing (e.g. loudness levelling). Returns the frames written and
    /// the track they belong to (see [`Player::track_audio`]); `None` if nothing was pulled.
    pub fn pull_track(&mut self, out: &mut [f32]) -> Option<(u32, usize)> {
        match self.pull(out, true) {
            (n, Some(seq)) if n > 0 => Some((seq, n)),
            _ => None,
        }
    }

    fn pull(&mut self, out: &mut [f32], one_track: bool) -> (usize, Option<u32>) {
        if !self.may_pull() {
            return (0, None);
        }
        let Some(rate) = self.output_rate() else {
            return (0, None);
        };
        let want = out.len() / 2;
        let gain = self.applied_volume();
        let mut written = 0;
        let mut first_seq = None;
        for seg in self.segments.iter_mut() {
            if written == want {
                break;
            }
            if seg.pulled == seg.frames {
                if one_track && first_seq.is_some_and(|s| s != seg.seq) {
                    break;
                }
                continue;
            }
            if seg.rate != rate {
                break;
            }
            match first_seq {
                None => first_seq = Some(seg.seq),
                Some(s) if one_track && s != seg.seq => break,
                Some(_) => {}
            }
            let n = ((seg.frames - seg.pulled) as usize).min(want - written);
            for i in 0..n {
                let l = self.pcm.pop_front().unwrap_or(0.0);
                let r = self.pcm.pop_front().unwrap_or(0.0);
                out[(written + i) * 2] = l * gain.0;
                out[(written + i) * 2 + 1] = r * gain.1;
            }
            seg.pulled += n as u64;
            written += n;
        }
        (written, first_seq)
    }

    /// Whether audio may be pulled. It may while playing. A host that applies corrections at
    /// its output may also pull while waiting to start (loaded, or a timed start pending), so
    /// its output can start on time.
    fn may_pull(&self) -> bool {
        match self.output {
            OutputState::Playing => true,
            OutputState::Loaded | OutputState::Paused { until: Some(_) } => self.output_corrections,
            _ => false,
        }
    }

    /// The host time a pending timed start is due (`strm u` with a start time), if any.
    pub fn start_due(&self) -> Option<u64> {
        match self.output {
            OutputState::Paused { until } => until,
            _ => None,
        }
    }

    /// The track that is audible now (the last one reported as played), if any.
    pub fn playing_track(&self) -> Option<u32> {
        self.playing_seq
    }

    /// Format details of a track still in the output queue (`seq` from
    /// [`Player::pull_track`] or [`Player::playing_track`]).
    pub fn track_audio(&self, seq: u32) -> Option<TrackAudio> {
        let seg = self.segments.iter().find(|s| s.seq == seq)?;
        Some(TrackAudio {
            seq,
            sample_rate: seg.rate,
            source: seg.source,
            server_gain: seg.server_gain,
        })
    }

    /// The volume LMS has set (`audg`), as linear (left, right) factors; (1.0, 1.0) is
    /// unity, which leaves samples untouched.
    pub fn volume(&self) -> (f32, f32) {
        self.gain
    }

    /// Fixed volume: ignore LMS's volume (`audg`) and pass samples at 100 %. For outputs
    /// whose volume is the device's own (a bit-perfect USB DAC), so that turning LMS's
    /// volume down can't change a sample. LMS's volume is still remembered, and applies
    /// again once this is turned off.
    pub fn set_fixed_volume(&mut self, fixed: bool) {
        self.fixed_volume = fixed;
    }

    pub fn fixed_volume(&self) -> bool {
        self.fixed_volume
    }

    /// The volume applied to pulled audio: LMS's, or (1.0, 1.0) at fixed volume.
    pub fn applied_volume(&self) -> (f32, f32) {
        if self.fixed_volume || self.host_volume {
            (1.0, 1.0)
        } else {
            self.gain
        }
    }

    /// Format details of the audio the next pull returns (the first segment not yet fully
    /// pulled), so a host can prepare its output before pulling it.
    pub fn next_audio(&self) -> Option<TrackAudio> {
        let seg = self
            .segments
            .iter()
            .find(|s| s.pulled < s.frames || !s.complete)?;
        Some(TrackAudio {
            seq: seg.seq,
            sample_rate: seg.rate,
            source: seg.source,
            server_gain: seg.server_gain,
        })
    }

    /// Nothing more to pull until LMS starts another track: the current track is fully
    /// decoded and every frame has been pulled. (A pull returning nothing on its own does not
    /// mean this: decoding runs a slice at a time.)
    pub fn audio_exhausted(&self) -> bool {
        !self.decoding() && self.pcm.is_empty()
    }

    /// Report that `frames` previously pulled have now been played.
    pub fn played(&mut self, mut frames: u64) {
        // Audio flowing again after an `STMo` (read-ahead mode tells LMS with `STMr`).
        let resumed = frames > 0 && self.underrun_reported && !self.underrun_withdrawn;
        if frames > 0 {
            self.underrun_reported = false;
            self.underrun_withdrawn = false;
            self.stall_since = None;
        }
        let mut started = false;
        while frames > 0 {
            let Some(idx) = self.segments.iter().position(|s| s.played < s.pulled) else {
                break;
            };
            let seq = self.segments[idx].seq;
            if self.playing_seq != Some(seq) {
                self.playing_seq = Some(seq);
                // A track starts once: never a second `STMs` for it (in a sync group that
                // moves the whole group on a track).
                if self.stms_seq.is_none_or(|s| seq > s) {
                    self.stms_seq = Some(seq);
                    started = true;
                    self.note_sent("STMs", seq);
                    self.send_stat(*b"STMs", 0);
                }
            }
            let seg = &mut self.segments[idx];
            let n = frames.min(seg.pulled - seg.played);
            seg.played += n;
            frames -= n;
        }
        if resumed && !started && self.ready_when_received {
            self.send_stat(*b"STMr", 0);
        }
        // Drop fully played segments, keeping the one that is audible for elapsed time.
        while self.segments.len() > 1 {
            let s = &self.segments[0];
            if s.complete && s.played == s.frames {
                self.segments.pop_front();
            } else {
                break;
            }
        }
    }

    /// The byte rate of the track being decoded, in bytes per millisecond of audio: the
    /// stream bytes its decoder has consumed past the headers (tags and embedded artwork
    /// left out) over the audio they gave, once a second of it has been decoded (`None`
    /// before that, and between tracks). With the bytes still undecoded
    /// ([`Status::input_bytes`], and whatever the host spools), it says how much listening
    /// is held ahead. Per track, so a skipped track's bytes or an earlier track at another
    /// bitrate never skew it.
    pub fn decoded_rate(&self) -> Option<f64> {
        let t = self.track.as_ref()?;
        if t.decoded_rate == 0 {
            return None;
        }
        let decoded_ms = t.decoded_frames as f64 * 1000.0 / f64::from(t.decoded_rate);
        let consumed = t
            .bytes_fed
            .saturating_sub(t.input.unread() as u64)
            .saturating_sub(t.decoder.audio_from());
        (decoded_ms >= 1000.0 && consumed > 0).then(|| consumed as f64 / decoded_ms)
    }

    pub fn status(&self) -> Status {
        Status {
            output: self.output,
            elapsed_ms: self.elapsed_ms(),
            buffered_ms: self.buffered_ms(),
            input_bytes: [self.track.as_ref(), self.next.as_ref()]
                .into_iter()
                .flatten()
                .map(|t| t.input.unread())
                .sum(),
            sample_rate: self.output_rate().unwrap_or(0),
            volume: self.gain,
            name: self.cfg.name.clone(),
            last_error: self.last_error.clone(),
        }
    }

    // -----------------------------------------------------------------------

    fn handle(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Strm(s) => {
                if s.command != b't' {
                    self.diag(DiagEvent::Strm {
                        command: s.command,
                        autostart: s.autostart,
                        format: s.format,
                        threshold: s.threshold,
                    });
                }
                self.handle_strm(s)
            }
            ServerMsg::Audg(a) => self.gain = a.gains(),
            ServerMsg::Setd { id: 0, data } => {
                if data.is_empty() {
                    let mut name = self.cfg.name.clone().into_bytes();
                    name.push(0);
                    self.send(ClientMsg::Setd { id: 0, data: name });
                } else {
                    // LMS renamed the player: kept bounded and printable, since it is shown
                    // (`Status::name`) and a server can send anything up to a frame's size.
                    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
                    if let Some(name) =
                        crate::names::player_name(&String::from_utf8_lossy(&data[..end]))
                    {
                        self.cfg.name = name;
                    }
                }
            }
            // Never follow a redirect to another server.
            ServerMsg::Serv { .. } => {}
            ServerMsg::Aude { .. } | ServerMsg::Vers(_) | ServerMsg::Setd { .. } => {}
            ServerMsg::Other { .. } => {}
        }
    }

    fn handle_strm(&mut self, s: Strm) {
        match s.command {
            b's' => self.start_stream(s),
            b'p' => {
                if s.replay_gain == 0 {
                    self.output = OutputState::Paused { until: None };
                    self.send_stat(*b"STMp", 0);
                } else if self.output_corrections && self.output == OutputState::Playing {
                    // Sync adjustment at the host's output: silence now, then carry on.
                    let ms = s.replay_gain.min(MAX_CORRECTION_MS);
                    self.corrections.push(OutputCorrection::Pause { ms });
                    self.corrected_until = self.now + u64::from(ms);
                } else {
                    // Sync adjustment: pause briefly, then carry on.
                    let until = self.now + u64::from(s.replay_gain.min(MAX_CORRECTION_MS));
                    self.output = OutputState::Paused { until: Some(until) };
                }
            }
            b'u' => {
                // The start time is in our jiffies. Work out how far ahead it is, allowing
                // for the 32-bit counter wrapping. A time in the past (or 0) means now.
                let ahead = i64::from(s.replay_gain.wrapping_sub(self.jiffies()) as i32);
                if s.replay_gain != 0 && ahead > 0 && ahead as u64 <= MAX_START_AHEAD_MS {
                    let at = self.now + ahead as u64;
                    self.output = OutputState::Paused { until: Some(at) };
                    if self.output_corrections {
                        self.corrections
                            .push(OutputCorrection::StartAt { at_ms: at });
                    }
                } else {
                    self.output = OutputState::Playing;
                }
                self.send_stat(*b"STMr", 0);
            }
            b'q' => {
                self.flush();
                self.send_stat(*b"STMf", 0);
            }
            b'f' => {
                // LMS flushes what it streamed beyond the audible track (the next track was
                // deleted or moved) and keeps showing the audible one as playing.
                match self.audible_track() {
                    Some(keep) => self.trim_after(keep),
                    None => self.flush(),
                }
                self.send_stat(*b"STMf", 0);
            }
            b't' => self.send_stat(*b"STMt", s.replay_gain),
            b'a' => {
                let ms = s.replay_gain.min(MAX_CORRECTION_MS);
                if self.output_corrections {
                    self.corrections.push(OutputCorrection::Skip { ms });
                } else {
                    self.skip_ahead(u64::from(ms));
                }
            }
            _ => {}
        }
    }

    fn start_stream(&mut self, s: Strm) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let codec_ok = crate::decode::format_extension(s.format).is_some();
        // Read-ahead mode: the current track's stream is in but it is still decoding, so
        // this is the next track. Queue it rather than cut the current one off.
        let queue = self.ready_when_received
            && self
                .track
                .as_ref()
                .is_some_and(|t| t.stream_ended && !t.decode_done);
        if queue && !codec_ok {
            // Refused: say so, and leave the current track alone.
            self.send_stat(*b"STMn", 0);
            return;
        }
        if self.fetching().is_some_and(|t| !t.stream_ended) {
            self.actions.push(Action::CloseStream);
        }

        let input = InputBuffer::new();
        // A seek's stream shares the track's format: what this stream's frames don't say
        // about the rate or depth, the one before it does (see decode::flac_header_fix).
        let fallback = self.track.as_ref().and_then(|t| t.decoder.format_hint());
        let track = Track {
            seq,
            decoder: StreamDecoder::new(input.clone(), s.format).with_fallback(fallback),
            input,
            autostart: s.autostart,
            threshold: s.threshold as usize * 1024,
            bytes_received: 0,
            bytes_fed: 0,
            decoded_frames: 0,
            decoded_rate: 0,
            replay_gain: if s.replay_gain == 0 {
                1.0
            } else {
                s.replay_gain as f32 / 65536.0
            },
            stream_ended: false,
            input_ended: false,
            decode_done: false,
            ready_sent: false,
            opened_at: self.now,
        };
        if queue {
            self.next = Some(track);
        } else {
            self.next = None;
            if let Some(old) = self.track.replace(track) {
                // The replaced track decodes no more: its audio so far still plays, but its
                // segments must end, or the queue never drains past them (M12).
                self.complete_segments(old.seq);
            }
        }
        self.stmu_sent = false;
        self.stmu_deferred = false;
        if !codec_ok {
            if let Some(t) = &mut self.track {
                // Nothing will be fetched, fed or decoded: end the decoder's input too, and
                // count the track as done, so the player isn't left decoding for ever.
                t.stream_ended = true;
                t.input_ended = true;
                t.input.set_eof();
                t.decode_done = true;
                t.ready_sent = true;
            }
            self.send_stat(*b"STMn", 0);
            return;
        }
        let host = if s.server_ip == 0 {
            StreamHost::Server
        } else {
            StreamHost::Ip(s.server_ip.to_be_bytes())
        };
        self.actions.push(Action::OpenStream {
            host,
            port: s.server_port,
            request: s.request,
            stream_number: seq as u64 + 1,
            format: s.format,
        });
        self.send_stat(*b"STMc", 0);
        if matches!(self.output, OutputState::Stopped) {
            self.output = OutputState::Waiting;
        }
    }

    /// Mark every segment of track `seq` complete: nothing more will be decoded for it.
    fn complete_segments(&mut self, seq: u32) {
        for s in self.segments.iter_mut().filter(|s| s.seq == seq) {
            s.complete = true;
        }
    }

    fn flush(&mut self) {
        if self.fetching().is_some_and(|t| !t.stream_ended) {
            self.actions.push(Action::CloseStream);
        }
        self.track = None;
        self.next = None;
        self.stmu_deferred = false;
        self.stall_since = None;
        self.pcm.clear();
        self.segments.clear();
        self.playing_seq = None;
        self.output = OutputState::Stopped;
        self.stmu_sent = true;
        self.flushes += 1;
        self.corrections.clear();
        self.corrected_until = 0;
        self.position_at = None;
    }

    /// The track being heard, if playback is under way (playing or paused) and that track
    /// still has audio to come. A track that has fully played out doesn't count: its segment
    /// is only kept for the elapsed time.
    fn audible_track(&self) -> Option<u32> {
        if !matches!(
            self.output,
            OutputState::Playing | OutputState::Paused { .. }
        ) {
            return None;
        }
        let seq = self.playing_seq?;
        let decoding = self
            .track
            .as_ref()
            .is_some_and(|t| t.seq == seq && !t.decode_done);
        let unplayed = self
            .segments
            .iter()
            .any(|s| s.seq == seq && (s.played < s.frames || !s.complete));
        (decoding || unplayed).then_some(seq)
    }

    /// Drop every track after `keep` (fetched, queued or decoded), keeping `keep`'s audio and
    /// the playback state. The track being fetched, if it is after `keep`, is closed.
    fn trim_after(&mut self, keep: u32) {
        // The queued next track is never the audible one.
        let mut close = self.next.take().is_some_and(|n| !n.stream_ended);
        if let Some(t) = &self.track
            && t.seq > keep
        {
            // Its segments go below; `keep`'s, decoded before it, are already complete.
            close |= !t.stream_ended;
            self.track = None;
        }
        if close {
            self.actions.push(Action::CloseStream);
        }
        let kept = self.segments.iter().filter(|s| s.seq <= keep);
        let unpulled: u64 = kept.clone().map(|s| s.frames - s.pulled).sum();
        let host_holds: u64 = kept
            .filter(|s| s.seq == keep)
            .map(|s| s.pulled - s.played)
            .sum();
        self.segments.retain(|s| s.seq <= keep);
        // The queue holds the unpulled frames of each segment, in segment order.
        self.pcm.truncate(unpulled as usize * 2);
        self.trims += 1;
        self.last_trim = Some(QueueTrim {
            keep_track: keep,
            keep_frames: host_holds,
        });
    }

    /// Drop up to `ms` of decoded audio as if played; returns the milliseconds dropped.
    fn skip_ahead(&mut self, ms: u64) -> u64 {
        let Some(rate) = self.output_rate().filter(|r| *r > 0) else {
            return 0;
        };
        let want = ms.saturating_mul(u64::from(rate)) / 1000;
        let mut frames = want;
        for seg in self.segments.iter_mut() {
            if frames == 0 {
                break;
            }
            let n = (seg.frames - seg.pulled).min(frames);
            self.pcm.drain(..(n as usize * 2).min(self.pcm.len()));
            seg.pulled += n;
            seg.played += n;
            frames -= n;
        }
        (want - frames) * 1000 / u64::from(rate)
    }

    fn decode_some(&mut self) {
        let target_frames =
            OUTPUT_TARGET_SECS as usize * self.output_rate().unwrap_or(48_000) as usize;
        if self.pcm.len() / 2 >= target_frames {
            return;
        }
        // A header made for bare FLAC frames (a seek): noted once, for the Connection log.
        if let Some((rate, channels, bits)) = self
            .track
            .as_mut()
            .and_then(|t| t.decoder.take_synthesised())
        {
            self.diag(DiagEvent::FlacHeader {
                rate,
                channels,
                bits,
            });
        }
        let Some(t) = &mut self.track else { return };
        if t.decode_done {
            return;
        }
        let (pcm, segments, seq, rg) = (&mut self.pcm, &mut self.segments, t.seq, t.replay_gain);
        let source = t.decoder.source_format().copied();
        let mut produced = (0u64, 0u32);
        let status = t.decoder.work(PACKETS_PER_TICK, &mut |rate, samples| {
            produced = (produced.0 + (samples.len() / 2) as u64, rate);
            let need_new = match segments.back() {
                Some(s) => s.seq != seq || s.rate != rate,
                None => true,
            };
            if need_new {
                if let Some(prev) = segments.back_mut()
                    && prev.seq == seq
                {
                    prev.complete = true; // rate change mid-track
                }
                segments.push_back(Segment {
                    seq,
                    rate,
                    frames: 0,
                    pulled: 0,
                    played: 0,
                    complete: false,
                    source,
                    server_gain: rg,
                });
            }
            let seg = segments.back_mut().unwrap();
            if rg == 1.0 {
                pcm.extend(samples);
            } else {
                pcm.extend(samples.iter().map(|s| s * rg));
            }
            seg.frames += (samples.len() / 2) as u64;
        });
        if produced.0 > 0 {
            t.decoded_frames += produced.0;
            t.decoded_rate = produced.1;
        }
        // The headers are probed inside `work`: record the format on this track's segments.
        if let Some(f) = t.decoder.source_format().copied() {
            for s in self.segments.iter_mut().filter(|s| s.seq == seq) {
                s.source.get_or_insert(f);
            }
        }
        match status {
            DecodeStatus::Finished => {
                t.decode_done = true;
                // In read-ahead mode `STMd` may already have gone, once the stream was in.
                let ready_sent = std::mem::replace(&mut t.ready_sent, true);
                if let Some(s) = self.segments.iter_mut().rev().find(|s| s.seq == seq) {
                    s.complete = true;
                }
                if !ready_sent {
                    self.note_sent("STMd", seq);
                    self.send_stat(*b"STMd", 0);
                }
            }
            DecodeStatus::Failed(e) => {
                t.decode_done = true;
                t.ready_sent = true;
                self.last_error = Some(e);
                if let Some(s) = self.segments.iter_mut().rev().find(|s| s.seq == seq) {
                    s.complete = true;
                }
                self.send_stat(*b"STMn", 0);
            }
            DecodeStatus::Progress | DecodeStatus::NeedData => return,
        }
        // The current track is fully decoded: the queued next one follows it, gaplessly.
        if let Some(next) = self.next.take() {
            self.track = Some(next);
        }
    }

    fn check_autostart(&mut self) {
        if !matches!(self.output, OutputState::Waiting) {
            return;
        }
        let Some(t) = &self.track else { return };
        let quick = self
            .quick_start_ms
            .is_some_and(|min| self.buffered_ms() >= min as u64 && self.arriving_fast(t));
        let ready =
            (t.bytes_received as usize >= t.threshold || t.stream_ended || t.decode_done || quick)
                && !self.pcm.is_empty();
        if !ready {
            return;
        }
        if t.autostart == b'1' || t.autostart == b'3' {
            self.output = OutputState::Playing;
        } else {
            self.output = OutputState::Loaded;
            self.send_stat(*b"STMl", 0);
        }
    }

    /// Is the track's stream arriving at least [`QUICK_START_SPEEDUP`] times faster than it
    /// plays? Compares bytes received per millisecond waited against the bytes the decoder
    /// has used per millisecond of audio. (Headers such as artwork count as used bytes, which
    /// only makes the stream look slower: the safe direction.)
    fn arriving_fast(&self, t: &Track) -> bool {
        let decoded_ms = self.buffered_ms() as u128;
        let used = (t.bytes_fed as u128).saturating_sub(t.input.unread() as u128);
        let waited_ms = self.now.saturating_sub(t.opened_at) as u128;
        if decoded_ms == 0 || used == 0 {
            return false;
        }
        // received / waited >= SPEEDUP * used / decoded, without dividing.
        t.bytes_received as u128 * decoded_ms >= QUICK_START_SPEEDUP as u128 * used * waited_ms
    }

    fn jiffies(&self) -> u32 {
        self.now as u32
    }

    /// Elapsed time in the audible track. If the host reports positions with
    /// [`Player::presented`], it is extrapolated to now: the frames presented since the last
    /// report are added, but at most [`MAX_EXTRAPOLATE_MS`] worth and never more than the
    /// host has been given. Nothing is added while stopped, paused or in a pause correction.
    fn elapsed_ms(&self) -> u64 {
        let Some(seq) = self.playing_seq else {
            return 0;
        };
        let mut frames = 0;
        let mut queued = 0;
        let mut rate = 0;
        for s in self.segments.iter().filter(|s| s.seq == seq) {
            frames += s.played;
            queued += s.pulled - s.played;
            rate = s.rate;
        }
        if rate == 0 {
            return 0;
        }
        let mut ms = frames as f64 * 1000.0 / f64::from(rate);
        if let Some(at) = self.position_at
            && self.output == OutputState::Playing
            && self.now > at
            && self.now >= self.corrected_until
        {
            let dt = (self.now - at).min(MAX_EXTRAPOLATE_MS) as f64;
            ms += dt.min(queued as f64 * 1000.0 / f64::from(rate));
        }
        ms as u64
    }

    fn buffered_ms(&self) -> u64 {
        match self.output_rate() {
            Some(r) if r > 0 => (self.pcm.len() / 2) as u64 * 1000 / r as u64,
            _ => 0,
        }
    }

    fn send(&mut self, msg: ClientMsg) {
        self.actions.push(Action::Send(msg.encode()));
    }

    fn send_stat(&mut self, event: [u8; 4], server_timestamp: u32) {
        if &event != b"STMt" {
            self.diag(DiagEvent::Stat(event));
        }
        let elapsed = self.elapsed_ms();
        // The stream figures describe the newest stream, the one LMS sent last.
        let (input_unread, bytes) = self
            .fetching()
            .map_or((0, 0), |t| (t.input.unread() as u32, t.bytes_received));
        let (host, host_cap) = self.host_buffered;
        let out_bytes = (self.pcm.len() as u64 * 4)
            .saturating_add(host)
            .min(u32::MAX as u64) as u32;
        let out_cap = (OUTPUT_TARGET_SECS as u64 * 48_000 * 8)
            .saturating_add(host_cap)
            .min(u32::MAX as u64) as u32;
        let stat = Stat {
            event,
            stream_buffer_size: INPUT_HIGH_WATER as u32,
            stream_buffer_fullness: input_unread,
            bytes_received: bytes,
            signal_strength: 0xffff,
            jiffies: self.jiffies(),
            output_buffer_size: out_cap.max(out_bytes),
            output_buffer_fullness: out_bytes,
            elapsed_seconds: (elapsed / 1000) as u32,
            elapsed_milliseconds: elapsed as u32,
            server_timestamp,
            ..Default::default()
        };
        self.send(ClientMsg::Stat(stat));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ClientFrameReader, encode_server_frame};

    fn cfg() -> PlayerConfig {
        PlayerConfig {
            mac: [2, 0, 0, 0, 0, 1],
            uuid: [0; 16],
            name: "Test".into(),
            codecs: vec!["flc".into(), "pcm".into()],
            max_sample_rate: 192_000,
            firmware: "test".into(),
        }
    }

    fn strm(cmd: u8, autostart: u8, format: u8, arg: u32) -> Vec<u8> {
        strm_with_threshold(cmd, autostart, format, arg, 1)
    }

    /// `strm` with LMS's buffer threshold, in KiB.
    fn strm_with_threshold(cmd: u8, autostart: u8, format: u8, arg: u32, kib: u8) -> Vec<u8> {
        let s = Strm {
            command: cmd,
            autostart,
            format,
            pcm_sample_size: b'?',
            pcm_sample_rate: b'?',
            pcm_channels: b'?',
            pcm_endianness: b'?',
            threshold: kib,
            spdif_enable: 0,
            transition_period: 0,
            transition_type: b'0',
            flags: 0,
            output_threshold: 0,
            slaves: 0,
            replay_gain: arg,
            server_port: 9000,
            server_ip: 0,
            request: b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n".to_vec(),
        };
        encode_server_frame(*b"strm", &s.encode_payload())
    }

    /// STAT events sent, in order.
    fn events(actions: &[Action]) -> Vec<String> {
        let mut r = ClientFrameReader::default();
        for a in actions {
            if let Action::Send(b) = a {
                r.push(b);
            }
        }
        let mut out = vec![];
        while let Some((op, p)) = r.next_frame().unwrap() {
            if &op == b"STAT" {
                out.push(String::from_utf8_lossy(&p[0..4]).into_owned());
            } else {
                out.push(String::from_utf8_lossy(&op).into_owned());
            }
        }
        out
    }

    fn wav(rate: u32, frames: usize) -> Vec<u8> {
        let data_len = frames * 4;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 4).to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data_len as u32).to_le_bytes());
        v.resize(44 + data_len, 0x10);
        v
    }

    /// Pull everything currently playable, reporting it as played immediately.
    fn drain(p: &mut Player) -> usize {
        let mut buf = vec![0f32; 4096];
        let mut total = 0;
        loop {
            let n = p.pull_audio(&mut buf);
            if n == 0 {
                return total;
            }
            p.played(n as u64);
            total += n;
        }
    }

    #[test]
    fn helo_on_connect_and_name_on_request() {
        let mut p = Player::new(cfg());
        p.connected(0);
        p.on_control_data(&encode_server_frame(*b"setd", &[0]), 1)
            .unwrap();
        assert_eq!(events(&p.take_actions()), vec!["HELO", "SETD"]);
    }

    /// A server rename is kept bounded and printable (review low: about 32 KiB with
    /// control characters reached `Status::name`); one with nothing printable is ignored.
    #[test]
    fn a_server_rename_is_bounded_and_printable() {
        let mut p = Player::new(cfg());
        p.connected(0);
        let rename = |p: &mut Player, name: &[u8]| {
            let mut data = vec![0u8];
            data.extend_from_slice(name);
            data.push(0);
            p.on_control_data(&encode_server_frame(*b"setd", &data), 1)
                .unwrap();
        };
        rename(&mut p, b"Kitchen");
        assert_eq!(p.status().name, "Kitchen");
        let mut hostile = b"\x1b[2J\x07Den\r\n".to_vec();
        hostile.extend(std::iter::repeat_n(b'x', 30 * 1024));
        rename(&mut p, &hostile);
        let name = p.status().name;
        assert!(name.starts_with("[2JDen"), "{name:?}");
        assert_eq!(name.chars().count(), crate::names::PLAYER_NAME_MAX);
        assert!(!name.chars().any(char::is_control));
        rename(&mut p, b"\x01\x02 \t");
        assert_eq!(p.status().name, name, "nothing printable: unchanged");
    }

    /// A pull that returns nothing while decoding is still under way is not the end
    /// (decoding runs a slice per tick): hosts must not treat it as one.
    #[test]
    fn audio_is_not_exhausted_while_a_track_is_still_decoding() {
        let mut p = Player::new(cfg());
        p.connected(0);
        assert!(p.audio_exhausted(), "idle: nothing to pull");
        p.on_control_data(&strm(b's', b'1', b'p', 0), 10).unwrap();
        p.on_stream_headers(b"HTTP/1.0 200 OK\r\n\r\n", 11);
        p.on_stream_data(&wav(44100, 44100 * 5), 12); // 5 s, stream still open
        let mut buf = vec![0f32; 4096 * 2];
        while p.pull_audio(&mut buf) > 0 {}
        assert!(!p.audio_exhausted(), "track not fully decoded yet");
    }

    #[test]
    fn plays_two_tracks_gaplessly_with_correct_events() {
        let mut p = Player::new(cfg());
        p.connected(0);
        p.take_actions();

        // Track 1.
        p.on_control_data(&strm(b's', b'1', b'p', 0), 10).unwrap();
        let a = p.take_actions();
        assert!(matches!(
            a[0],
            Action::OpenStream {
                host: StreamHost::Server,
                port: 9000,
                ..
            }
        ));
        assert_eq!(events(&a), vec!["STMc"]);

        let t1 = wav(44100, 44100); // 1 s
        p.on_stream_headers(b"HTTP/1.0 200 OK\r\n\r\n", 11);
        p.on_stream_data(&t1, 12);
        p.on_stream_end(13);
        for i in 0..20 {
            p.tick(14 + i);
        }
        assert_eq!(p.status().output, OutputState::Playing);
        assert_eq!(events(&p.take_actions()), vec!["RESP", "DSCO", "STMd"]);

        // Play half of track 1, then LMS sends track 2 (as it does after STMd).
        let mut buf = vec![0f32; 22050 * 2];
        assert_eq!(p.pull_audio(&mut buf), 22050);
        p.played(22050);
        assert_eq!(events(&p.take_actions()), vec!["STMs"]);
        assert_eq!(p.status().elapsed_ms, 500);

        p.on_control_data(&strm(b's', b'1', b'p', 0), 600).unwrap();
        p.on_stream_data(&wav(44100, 44100), 601);
        p.on_stream_end(602);
        for i in 0..20 {
            p.tick(603 + i);
        }
        p.take_actions();

        // Audio is still coming (track 2 decoded, not yet pulled): not exhausted.
        assert!(!p.audio_exhausted());

        // The rest of track 1 and all of track 2 come out contiguously: no gap.
        let total = drain(&mut p);
        assert_eq!(total, 22050 + 44100);
        // Everything decoded and pulled: nothing more until LMS sends another track.
        assert!(p.audio_exhausted());
        let ev = events(&p.take_actions());
        assert_eq!(
            ev.iter().filter(|e| *e == "STMs").count(),
            1,
            "STMs for track 2: {ev:?}"
        );
        assert_eq!(p.status().elapsed_ms, 1000);

        p.tick(2000);
        assert!(events(&p.take_actions()).contains(&"STMu".to_string()));
        assert_eq!(p.status().output, OutputState::Stopped);
    }

    #[test]
    fn pull_track_stops_at_track_boundaries_and_reports_formats() {
        let mut p = Player::new(cfg());
        p.connected(0);
        p.on_control_data(&strm(b's', b'1', b'p', 0), 1).unwrap();
        p.on_stream_data(&wav(44100, 1000), 2);
        p.on_stream_end(3);
        for i in 0..10 {
            p.tick(4 + i);
        }
        // Track 2 at the same rate, with a server ReplayGain of 0.5.
        p.on_control_data(&strm(b's', b'1', b'p', 0x8000), 20)
            .unwrap();
        p.on_stream_data(&wav(44100, 500), 21);
        p.on_stream_end(22);
        for i in 0..10 {
            p.tick(23 + i);
        }
        let mut buf = vec![0f32; 8192];
        let (seq1, n1) = p.pull_track(&mut buf).unwrap();
        assert_eq!(n1, 1000, "stops at the end of track 1");
        let (seq2, n2) = p.pull_track(&mut buf).unwrap();
        assert_eq!((seq2, n2), (seq1 + 1, 500));
        assert!(p.pull_track(&mut buf).is_none());

        let a1 = p.track_audio(seq1).unwrap();
        assert_eq!(a1.sample_rate, 44100);
        assert_eq!(a1.server_gain, 1.0);
        let src = a1.source.unwrap();
        assert_eq!((src.codec, src.lossless, src.bits), ("PCM", true, Some(16)));
        assert_eq!(p.track_audio(seq2).unwrap().server_gain, 0.5);
        assert!(p.track_audio(seq2 + 7).is_none());

        assert_eq!(p.playing_track(), None);
        p.played(10);
        assert_eq!(p.playing_track(), Some(seq1));
        assert_eq!(p.volume(), (1.0, 1.0));
    }

    /// At fixed volume (a bit-perfect DAC), LMS's volume never touches a sample; it still
    /// applies once fixed volume is off again.
    #[test]
    fn fixed_volume_ignores_audg_and_next_audio_describes_the_next_pull() {
        let mut p = Player::new(cfg());
        p.connected(0);
        // Volume 50 % on both channels (16.16 fixed point), with adjustment on.
        let mut audg = Vec::new();
        audg.extend_from_slice(&[0; 8]);
        audg.extend_from_slice(&[1, 255]);
        audg.extend_from_slice(&0x8000u32.to_be_bytes());
        audg.extend_from_slice(&0x8000u32.to_be_bytes());
        p.on_control_data(&encode_server_frame(*b"audg", &audg), 1)
            .unwrap();
        assert_eq!(p.volume(), (0.5, 0.5));
        assert!(p.next_audio().is_none());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 2).unwrap();
        p.on_stream_data(&wav(48_000, 1000), 3);
        p.on_stream_end(4);
        for i in 0..10 {
            p.tick(5 + i);
        }
        let next = p.next_audio().unwrap();
        assert_eq!(next.sample_rate, 48_000);
        assert_eq!(next.source.unwrap().bits, Some(16));

        let mut halved = vec![0f32; 200];
        p.pull_audio(&mut halved);
        p.set_fixed_volume(true);
        assert!(p.fixed_volume());
        assert_eq!(p.applied_volume(), (1.0, 1.0));
        assert_eq!(p.volume(), (0.5, 0.5), "LMS's volume is remembered");
        let mut full = vec![0f32; 200];
        p.pull_audio(&mut full);
        // The test WAV repeats every 100 frames, so the two blocks hold the same samples.
        for (h, f) in halved.iter().zip(&full) {
            assert_eq!(*h, f * 0.5);
        }
        assert!(full.iter().any(|s| *s != 0.0));
        p.set_fixed_volume(false);
        let mut again = vec![0f32; 200];
        p.pull_audio(&mut again);
        assert_eq!(again, halved);
    }

    #[test]
    fn host_buffer_counts_in_stat_and_underruns_report_once() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        // Half a track arrives; the stream is still open (decoding not done).
        p.on_stream_data(&wav(44100, 44100), 1);
        for i in 0..10 {
            p.tick(2 + i);
        }
        assert_eq!(p.status().output, OutputState::Playing);
        let mut buf = vec![0f32; 4096];
        p.pull_audio(&mut buf);
        p.played(100);
        p.take_actions();
        p.set_host_buffer(80_000, 400_000);
        p.output_underrun();
        p.output_underrun();
        let a = p.take_actions();
        assert_eq!(events(&a), vec!["STMo"], "reported once per episode");
        let Action::Send(bytes) = &a[0] else { panic!() };
        let stat = Stat::parse(&bytes[8..]).unwrap();
        let core_bytes = (p.pcm.len() * 4) as u32;
        assert_eq!(stat.output_buffer_fullness, core_bytes + 80_000);
        assert!(stat.output_buffer_size >= stat.output_buffer_fullness);
        assert!(stat.output_buffer_size >= 400_000);
        // Audio flows again, then runs dry again: a new report.
        p.played(10);
        p.output_underrun();
        assert_eq!(events(&p.take_actions()), vec!["STMo"]);
        // The end of the audio is not an underrun.
        p.on_stream_end(50);
        for i in 0..40 {
            p.tick(51 + i);
        }
        p.take_actions();
        p.played(1);
        p.output_underrun();
        assert!(!events(&p.take_actions()).contains(&"STMo".to_string()));
    }

    #[test]
    fn flushes_are_counted() {
        let mut p = Player::new(cfg());
        assert_eq!(p.flush_count(), 0);
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 1).unwrap();
        p.on_control_data(&strm(b'f', b'0', b'p', 0), 2).unwrap();
        assert_eq!(p.flush_count(), 2);
        assert_eq!(p.mac(), [2, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn pause_resume_flush() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        for i in 0..10 {
            p.tick(3 + i);
        }
        p.take_actions();

        p.on_control_data(&strm(b'p', b'0', b'p', 0), 20).unwrap();
        let mut buf = vec![0f32; 1024];
        assert_eq!(p.pull_audio(&mut buf), 0, "paused output yields nothing");
        p.on_control_data(&strm(b'u', b'0', b'p', 0), 30).unwrap();
        assert!(p.pull_audio(&mut buf) > 0);
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 40).unwrap();
        let a = p.take_actions();
        assert_eq!(events(&a), vec!["STMp", "STMr", "STMf"]);
        assert_eq!(p.status().output, OutputState::Stopped);
        assert_eq!(p.pull_audio(&mut buf), 0);
    }

    #[test]
    fn heartbeat_echoes_server_timestamp() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b't', b'0', b'p', 0xdead_beef), 5)
            .unwrap();
        let a = p.take_actions();
        let Action::Send(bytes) = &a[0] else { panic!() };
        let stat = Stat::parse(&bytes[8..]).unwrap();
        assert_eq!(&stat.event, b"STMt");
        assert_eq!(stat.server_timestamp, 0xdead_beef);
    }

    #[test]
    fn diagnostics_are_off_by_default() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 1).unwrap();
        assert!(p.take_diagnostics().is_empty());
    }

    #[test]
    fn diagnostics_keep_strm_and_stat_but_not_the_status_poll() {
        let mut p = Player::new(cfg());
        p.set_diagnostics(true);
        p.on_control_data(&strm_with_threshold(b's', b'0', b'p', 0, 255), 0)
            .unwrap();
        p.on_control_data(&strm(b't', b'0', b'p', 7), 1).unwrap();
        p.on_control_data(&strm(b'p', b'0', b'p', 0), 2).unwrap();
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 3).unwrap();
        let d = p.take_diagnostics();
        let s = DiagEvent::Strm {
            command: b's',
            autostart: b'0',
            format: b'p',
            threshold: 255,
        };
        assert_eq!(d[0], s, "{d:?}");
        assert!(d.contains(&DiagEvent::Stat(*b"STMc")), "{d:?}");
        assert!(d.contains(&DiagEvent::Stat(*b"STMp")), "{d:?}");
        assert!(d.contains(&DiagEvent::Stat(*b"STMf")), "{d:?}");
        assert!(
            !d.iter()
                .any(|e| matches!(e, DiagEvent::Strm { command: b't', .. })
                    || *e == DiagEvent::Stat(*b"STMt")),
            "the status poll is left out: {d:?}"
        );
        assert!(p.take_diagnostics().is_empty(), "taken once");
    }

    #[test]
    fn diagnostics_are_bounded_and_can_be_turned_off() {
        let mut p = Player::new(cfg());
        p.set_diagnostics(true);
        for i in 0..200 {
            p.on_control_data(&strm(b'p', b'0', b'p', 0), i).unwrap();
        }
        assert_eq!(p.take_diagnostics().len(), MAX_DIAG_EVENTS);
        p.set_diagnostics(false);
        p.on_control_data(&strm(b'u', b'0', b'p', 0), 300).unwrap();
        assert!(p.take_diagnostics().is_empty());
    }

    #[test]
    fn autostart_off_reports_loaded_and_waits_for_unpause() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'0', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        for i in 0..10 {
            p.tick(3 + i);
        }
        assert!(events(&p.take_actions()).contains(&"STMl".to_string()));
        assert_eq!(p.status().output, OutputState::Loaded);
        p.on_control_data(&strm(b'u', b'0', b'p', 0), 20).unwrap();
        assert_eq!(p.status().output, OutputState::Playing);
    }

    #[test]
    fn unsupported_format_is_refused() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'w', 0), 0).unwrap();
        let a = p.take_actions();
        assert!(!a.iter().any(|x| matches!(x, Action::OpenStream { .. })));
        assert_eq!(events(&a), vec!["STMn"]);
    }

    #[test]
    fn stream_numbers_count_every_strm_s_including_refused_ones() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'w', 0), 0).unwrap(); // unsupported: #1
        p.on_control_data(&strm(b's', b'1', b'p', 0), 1).unwrap(); // #2
        let numbers: Vec<u64> = p
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                Action::OpenStream { stream_number, .. } => Some(stream_number),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, vec![2]);
    }

    #[test]
    fn open_stream_reports_resume_offset() {
        let mut p = Player::new(cfg());
        assert_eq!(p.open_stream(), None);
        p.on_control_data(&strm(b's', b'1', b'w', 0), 0).unwrap(); // refused: #1
        assert_eq!(
            p.open_stream(),
            None,
            "a refused stream has nothing to resume"
        );
        p.on_control_data(&strm(b's', b'1', b'p', 0), 1).unwrap(); // #2
        p.on_stream_data(&[0u8; 1000], 2);
        p.on_stream_data(&[0u8; 234], 3);
        assert_eq!(p.open_stream(), Some((2, 1234)));
        p.on_stream_end(4);
        assert_eq!(p.open_stream(), None);
    }

    #[test]
    fn back_pressure_on_large_input() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        assert!(p.wants_stream_data());
        p.on_stream_data(&wav(44_100, (INPUT_HIGH_WATER + (2 << 20)) / 4), 1);
        p.tick(2); // probed: now decoding
        assert!(!p.wants_stream_data());
    }

    #[test]
    fn probing_reads_past_the_input_limit_up_to_a_cap() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&vec![0u8; INPUT_HIGH_WATER], 1);
        assert!(
            p.wants_stream_data(),
            "headers may be larger than the input limit"
        );
        p.on_stream_data(&vec![0u8; PROBE_MAX_BYTES - INPUT_HIGH_WATER], 2);
        assert!(!p.wants_stream_data());
    }

    /// `audg` as LMS sends it: 16.16 gains, `adjust` 1 for digital volume control.
    fn audg(adjust: u8, left: f32, right: f32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u32.to_be_bytes());
        p.extend_from_slice(&0u32.to_be_bytes());
        p.push(adjust);
        p.push(255);
        p.extend_from_slice(&((left * 65536.0) as u32).to_be_bytes());
        p.extend_from_slice(&((right * 65536.0) as u32).to_be_bytes());
        encode_server_frame(*b"audg", &p)
    }

    /// A player with one second of a constant-level track decoded and playing.
    fn playing_player(host_volume: bool) -> Player {
        let mut p = Player::new(cfg());
        p.set_host_volume(host_volume);
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        for i in 0..10 {
            p.tick(3 + i);
        }
        assert_eq!(p.status().output, OutputState::Playing);
        p
    }

    #[test]
    fn volume_is_applied_to_pulled_audio_by_default() {
        let mut p = playing_player(false);
        let mut q = playing_player(false);
        let mut full = vec![0f32; 64];
        q.pull_audio(&mut full);
        p.on_control_data(&audg(1, 0.25, 0.5), 20).unwrap();
        assert_eq!(p.volume(), (0.25, 0.5));
        let mut buf = vec![0f32; 64];
        p.pull_audio(&mut buf);
        assert!(full[0] != 0.0);
        assert_eq!(buf[0], full[0] * 0.25);
        assert_eq!(buf[1], full[1] * 0.5);
    }

    /// With the host applying volume, pulled audio stays at full scale (so audio queued
    /// ahead isn't stuck at the old volume) while `volume()` still reports LMS's gains.
    #[test]
    fn host_volume_leaves_pulled_audio_at_full_scale() {
        let mut p = playing_player(true);
        let mut q = playing_player(false);
        p.on_control_data(&audg(1, 0.125, 0.25), 20).unwrap();
        assert_eq!(p.volume(), (0.125, 0.25));
        assert_eq!(p.status().volume, (0.125, 0.25));
        let (mut a, mut b) = (vec![0f32; 64], vec![0f32; 64]);
        p.pull_audio(&mut a);
        q.pull_audio(&mut b);
        assert_eq!(a, b, "unscaled");
        // Fixed volume from LMS (adjust 0) reads as unity.
        p.on_control_data(&audg(0, 0.1, 0.2), 30).unwrap();
        assert_eq!(p.volume(), (1.0, 1.0));
    }

    /// 44.1 kHz 16-bit stereo: 176 400 bytes a second. LMS's usual threshold, 255 KiB, is
    /// about 1.5 s of it.
    const WAV_BYTES_PER_SEC: usize = 176_400;

    fn open_with_threshold(p: &mut Player, now: u64) {
        p.on_control_data(&strm_with_threshold(b's', b'1', b'p', 0, 255), now)
            .unwrap();
        p.on_stream_headers(b"HTTP/1.0 200 OK\r\n\r\n", now);
    }

    fn tick_n(p: &mut Player, from: u64, n: u64) {
        for i in 0..n {
            p.tick(from + i);
        }
    }

    #[test]
    fn without_quick_start_playback_waits_for_lms_threshold() {
        let mut p = Player::new(cfg());
        open_with_threshold(&mut p, 0);
        // 1.4 s of audio in 50 ms: below 255 KiB (and the decoder keeps 128 KiB unread).
        p.on_stream_data(&wav(44100, 44100 * 7 / 5), 50);
        tick_n(&mut p, 51, 200);
        assert_eq!(p.status().output, OutputState::Waiting);
        p.on_stream_data(&vec![0x10; WAV_BYTES_PER_SEC / 2], 80);
        tick_n(&mut p, 81, 5);
        assert_eq!(p.status().output, OutputState::Playing);
    }

    #[test]
    fn quick_start_plays_a_fast_stream_before_the_threshold() {
        let mut p = Player::new(cfg());
        p.set_quick_start(Some(500));
        open_with_threshold(&mut p, 0);
        p.on_stream_data(&wav(44100, 44100 * 7 / 5), 50);
        tick_n(&mut p, 51, 200);
        assert_eq!(p.status().output, OutputState::Playing);
        assert!(p.track.as_ref().unwrap().bytes_received < 255 * 1024);
    }

    #[test]
    fn quick_start_waits_for_its_minimum() {
        let mut p = Player::new(cfg());
        p.set_quick_start(Some(500));
        open_with_threshold(&mut p, 0);
        p.on_stream_data(&wav(44100, 44100), 50); // 1 s: about 0.25 s decodable
        tick_n(&mut p, 51, 200);
        assert_eq!(p.status().output, OutputState::Waiting);
    }

    /// A stream arriving at about real time (live radio through LMS) keeps the cushion
    /// LMS's threshold gives it.
    #[test]
    fn quick_start_ignores_a_stream_arriving_in_real_time() {
        let mut p = Player::new(cfg());
        p.set_quick_start(Some(500));
        open_with_threshold(&mut p, 0);
        let full = wav(44100, 44100 * 3);
        p.on_stream_data(&full[..44], 0);
        // 1.4 s of audio over 1.4 s.
        let tenth = WAV_BYTES_PER_SEC / 10;
        for step in 1..=14usize {
            let at = 44 + (step - 1) * tenth;
            p.on_stream_data(&full[at..at + tenth], step as u64 * 100);
            tick_n(&mut p, step as u64 * 100, 30);
        }
        assert!(
            p.status().buffered_ms >= 500,
            "enough decoded: only the speed holds it"
        );
        assert_eq!(p.status().output, OutputState::Waiting);
        // The threshold still starts it.
        let at = 44 + 14 * tenth;
        p.on_stream_data(&full[at..at + WAV_BYTES_PER_SEC / 2], 1500);
        tick_n(&mut p, 1501, 5);
        assert_eq!(p.status().output, OutputState::Playing);
    }

    /// After a seek (flush, then a new stream) quick start applies again; a zero minimum
    /// turns it off.
    #[test]
    fn quick_start_after_a_seek_and_off_with_zero() {
        let mut p = Player::new(cfg());
        p.set_quick_start(Some(500));
        open_with_threshold(&mut p, 0);
        p.on_stream_data(&wav(44100, 44100 * 7 / 5), 50);
        tick_n(&mut p, 51, 200);
        assert_eq!(p.status().output, OutputState::Playing);
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 5000).unwrap();
        open_with_threshold(&mut p, 5001);
        p.on_stream_data(&wav(44100, 44100 * 7 / 5), 5030);
        tick_n(&mut p, 5031, 200);
        assert_eq!(p.status().output, OutputState::Playing);

        let mut q = Player::new(cfg());
        q.set_quick_start(Some(0));
        open_with_threshold(&mut q, 0);
        q.on_stream_data(&wav(44100, 44100 * 7 / 5), 50);
        tick_n(&mut q, 51, 200);
        assert_eq!(q.status().output, OutputState::Waiting);
    }

    /// The STAT events sent, with their jiffies and elapsed milliseconds.
    fn stats(actions: &[Action]) -> Vec<(String, u32, u32)> {
        let mut r = ClientFrameReader::default();
        for a in actions {
            if let Action::Send(b) = a {
                r.push(b);
            }
        }
        let mut out = vec![];
        while let Some((op, p)) = r.next_frame().unwrap() {
            if &op == b"STAT" {
                let s = Stat::parse(&p).unwrap();
                out.push((
                    String::from_utf8_lossy(&s.event).into_owned(),
                    s.jiffies,
                    s.elapsed_milliseconds,
                ));
            }
        }
        out
    }

    /// `strm` carrying a 32-bit argument in the replay-gain slot (interval or start time).
    fn strm_arg(cmd: u8, arg: u32) -> Vec<u8> {
        strm(cmd, b'0', b'p', arg)
    }

    /// A player with ten seconds of 44.1 kHz audio decoded, loaded (autostart off, as LMS
    /// starts a synced group) and taking corrections at its output.
    fn loaded_sync_player(now: u64) -> Player {
        let mut p = Player::new(cfg());
        p.set_output_corrections(true);
        p.on_control_data(&strm(b's', b'0', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 10), now);
        p.on_stream_end(now);
        tick_n(&mut p, now, 40);
        assert_eq!(p.status().output, OutputState::Loaded);
        p.take_actions();
        p
    }

    /// The (jiffies, elapsed) pair in each STAT describes the same instant: elapsed is
    /// extrapolated from the host's last position report to the STAT's jiffies.
    #[test]
    fn stat_elapsed_is_extrapolated_to_its_own_jiffies() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 3), 1);
        p.on_stream_end(2);
        tick_n(&mut p, 3, 40);
        let mut buf = vec![0f32; 44100 * 2 * 2];
        p.pull_audio(&mut buf); // 2 s handed to the host
        p.presented(44100 / 2, 1000); // 500 ms audible, exact at t = 1000
        p.take_actions();
        p.on_control_data(&strm_arg(b't', 7), 1040).unwrap();
        let s = stats(&p.take_actions());
        assert_eq!(s[0].0, "STMt");
        assert_eq!(s[0].1, 1040);
        assert_eq!(s[0].2, 540, "500 ms plus the 40 ms since the report");
        // Never further than MAX_EXTRAPOLATE_MS past the report.
        p.on_control_data(&strm_arg(b't', 8), 4000).unwrap();
        let s = stats(&p.take_actions());
        assert_eq!(s[0].2, 500 + MAX_EXTRAPOLATE_MS as u32);
        // A fresh report moves the anchor.
        p.presented(441, 4050);
        p.on_control_data(&strm_arg(b't', 9), 4060).unwrap();
        assert_eq!(stats(&p.take_actions())[0].2, 520);
    }

    #[test]
    fn extrapolation_never_passes_what_the_host_was_given_nor_runs_while_paused() {
        let mut p = playing_player(false);
        let mut buf = vec![0f32; 441 * 2 * 2];
        p.pull_audio(&mut buf); // 20 ms handed over
        p.presented(441, 100); // 10 ms heard
        p.take_actions(); // STMs
        p.on_control_data(&strm_arg(b't', 1), 180).unwrap();
        assert_eq!(
            stats(&p.take_actions())[0].2,
            20,
            "capped at the 20 ms pulled"
        );
        p.on_control_data(&strm(b'p', b'0', b'p', 0), 200).unwrap();
        let s = stats(&p.take_actions());
        assert_eq!(s[0].0, "STMp");
        assert_eq!(s[0].2, 10, "paused: what was heard, no extrapolation");
    }

    /// Hosts that report without a time keep the old behaviour exactly.
    #[test]
    fn played_without_a_time_is_not_extrapolated() {
        let mut p = playing_player(false);
        let mut buf = vec![0f32; 44100 * 2];
        p.pull_audio(&mut buf);
        p.played(4410);
        p.take_actions();
        p.on_control_data(&strm_arg(b't', 1), 5000).unwrap();
        assert_eq!(stats(&p.take_actions())[0].2, 100);
    }

    /// A synced start: LMS loads every player, then sends `strm u` with a start time in
    /// each one's jiffies. With output corrections on, the host is told the time and may
    /// pre-roll audio into its output while it waits.
    #[test]
    fn timed_start_hands_the_host_its_start_time_and_allows_pre_roll() {
        let mut p = loaded_sync_player(1000);
        let mut buf = vec![0f32; 4410 * 2];
        assert_eq!(p.pull_audio(&mut buf), 4410, "pre-roll while loaded");
        p.on_control_data(&strm_arg(b'u', 1250), 1050).unwrap();
        assert_eq!(events(&p.take_actions()), vec!["STMr"]);
        assert_eq!(
            p.take_corrections(),
            vec![OutputCorrection::StartAt { at_ms: 1250 }]
        );
        assert_eq!(p.start_due(), Some(1250));
        assert!(
            p.pull_audio(&mut buf) > 0,
            "pre-roll while the start is due"
        );
        p.tick(1249);
        assert_eq!(p.status().output, OutputState::Paused { until: Some(1250) });
        p.tick(1250);
        assert_eq!(p.status().output, OutputState::Playing);
        assert_eq!(p.start_due(), None);
    }

    #[test]
    fn without_output_corrections_nothing_is_pulled_before_the_start() {
        let mut p = loaded_sync_player(1000);
        p.set_output_corrections(false);
        let mut buf = vec![0f32; 64];
        assert_eq!(p.pull_audio(&mut buf), 0);
        p.on_control_data(&strm_arg(b'u', 1250), 1050).unwrap();
        assert!(p.take_corrections().is_empty());
        assert_eq!(p.pull_audio(&mut buf), 0);
        p.tick(1250);
        assert!(p.pull_audio(&mut buf) > 0);
    }

    /// The start time is in the 32-bit jiffies, which wrap after 49 days.
    #[test]
    fn timed_start_across_the_jiffies_wrap() {
        let now = u64::from(u32::MAX) - 100; // jiffies just before the wrap
        let mut p = loaded_sync_player(now);
        let target = 99u32; // 200 ms ahead, after the wrap
        p.on_control_data(&strm_arg(b'u', target), now).unwrap();
        assert_eq!(
            p.take_corrections(),
            vec![OutputCorrection::StartAt { at_ms: now + 200 }]
        );
    }

    /// Security: a start time in the past, or implausibly far ahead (a corrupt or hostile
    /// frame), starts at once rather than holding the player silent.
    #[test]
    fn implausible_start_times_start_at_once() {
        for (now, arg) in [(5000u64, 4000u32), (5000, 5000 + 31_000), (5000, u32::MAX)] {
            let mut p = loaded_sync_player(now);
            p.on_control_data(&strm_arg(b'u', arg), now).unwrap();
            assert_eq!(p.status().output, OutputState::Playing, "{arg}");
            assert!(p.take_corrections().is_empty());
        }
    }

    #[test]
    fn pause_and_skip_corrections_go_to_the_host_output() {
        let mut p = playing_player(false);
        p.set_output_corrections(true);
        let mut buf = vec![0f32; 4410 * 2];
        p.pull_audio(&mut buf);
        p.presented(2205, 100);
        p.take_actions(); // STMs
        p.on_control_data(&strm_arg(b'p', 30), 100).unwrap();
        assert_eq!(p.status().output, OutputState::Playing, "no state change");
        p.on_control_data(&strm_arg(b'a', 45), 100).unwrap();
        assert_eq!(
            p.take_corrections(),
            vec![
                OutputCorrection::Pause { ms: 30 },
                OutputCorrection::Skip { ms: 45 }
            ]
        );
        assert!(events(&p.take_actions()).is_empty(), "no STAT for either");
        // No extrapolation while the pause runs: nothing new is heard.
        p.on_control_data(&strm_arg(b't', 1), 120).unwrap();
        assert_eq!(stats(&p.take_actions())[0].2, 50);
        // Hostile intervals are capped.
        p.on_control_data(&strm_arg(b'a', u32::MAX), 200).unwrap();
        assert_eq!(
            p.take_corrections(),
            vec![OutputCorrection::Skip {
                ms: MAX_CORRECTION_MS
            }]
        );
    }

    #[test]
    fn without_output_corrections_the_core_pauses_and_skips_itself() {
        let mut p = playing_player(false);
        p.on_control_data(&strm_arg(b'p', 30), 100).unwrap();
        assert_eq!(p.status().output, OutputState::Paused { until: Some(130) });
        p.tick(130);
        assert_eq!(p.status().output, OutputState::Playing);
        let before = p.status().buffered_ms;
        p.on_control_data(&strm_arg(b'a', 100), 140).unwrap();
        assert_eq!(p.status().buffered_ms, before - 100);
        assert!(p.take_corrections().is_empty());
    }

    #[test]
    fn a_flush_drops_pending_corrections() {
        let mut p = playing_player(false);
        p.set_output_corrections(true);
        p.on_control_data(&strm_arg(b'a', 45), 100).unwrap();
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 101).unwrap();
        assert!(p.take_corrections().is_empty());
    }

    /// A player playing track 1 (half heard) with track 2 fully decoded behind it, as when
    /// LMS has already streamed the next track.
    fn two_tracks_queued() -> (Player, u32) {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        tick_n(&mut p, 3, 20);
        let mut buf = vec![0f32; 22050 * 2];
        assert_eq!(p.pull_audio(&mut buf), 22050);
        p.played(22050);
        let seq1 = p.playing_track().unwrap();
        p.on_control_data(&strm(b's', b'1', b'p', 0), 600).unwrap();
        p.on_stream_data(&wav(44100, 44100), 601);
        p.on_stream_end(602);
        tick_n(&mut p, 603, 20);
        p.take_actions();
        (p, seq1)
    }

    /// Regression (spike S1): LMS sends `strm f` when the next track, already streamed, is
    /// deleted or moved in the queue, and keeps showing the current track as playing. The
    /// audible track must play on; only the next track's audio goes.
    #[test]
    fn strm_f_keeps_the_audible_track_and_drops_the_next() {
        let (mut p, seq1) = two_tracks_queued();
        let flushes = p.flush_count();
        p.on_control_data(&strm(b'f', b'0', b'p', 0), 700).unwrap();
        let a = p.take_actions();
        assert_eq!(events(&a), vec!["STMf"]);
        assert!(
            !a.contains(&Action::CloseStream),
            "track 2 was fully received"
        );
        assert_eq!(p.status().output, OutputState::Playing);
        assert_eq!(p.flush_count(), flushes, "not a full flush");
        assert_eq!(p.trim_count(), 1);
        assert_eq!(
            p.trim(),
            Some(QueueTrim {
                keep_track: seq1,
                keep_frames: 0
            })
        );
        assert_eq!(
            drain(&mut p),
            22050,
            "the rest of track 1, and nothing of track 2"
        );
        assert_eq!(p.playing_track(), Some(seq1));
        p.tick(1500);
        assert!(events(&p.take_actions()).contains(&"STMu".to_string()));
    }

    /// `strm f` while the next track is still arriving closes that stream; audio the host
    /// already holds for the audible track is counted in the trim so it can keep it.
    #[test]
    fn strm_f_closes_the_next_stream_and_reports_what_the_host_keeps() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        tick_n(&mut p, 3, 20);
        let mut buf = vec![0f32; 4410 * 2];
        p.pull_audio(&mut buf); // 4410 frames to the host
        p.played(1000); // 1000 heard, 3410 still queued in the host
        let seq1 = p.playing_track().unwrap();
        p.on_control_data(&strm(b's', b'1', b'p', 0), 100).unwrap();
        p.on_stream_data(&wav(44100, 44100)[..20_000], 101); // still arriving
        tick_n(&mut p, 102, 5);
        p.take_actions();
        p.on_control_data(&strm(b'f', b'0', b'p', 0), 200).unwrap();
        let a = p.take_actions();
        assert_eq!(a[0], Action::CloseStream);
        assert_eq!(events(&a), vec!["STMf"]);
        assert_eq!(
            p.trim(),
            Some(QueueTrim {
                keep_track: seq1,
                keep_frames: 3410
            })
        );
        assert_eq!(p.open_stream(), None);
        assert!(!p.wants_stream_data());
        p.played(3410);
        assert_eq!(drain(&mut p), 44100 - 4410);
    }

    /// With nothing audible yet, `strm f` is a full flush as before; `strm q` always is.
    #[test]
    fn strm_f_with_nothing_audible_and_strm_q_flush_everything() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        tick_n(&mut p, 2, 10);
        p.on_control_data(&strm(b'f', b'0', b'p', 0), 20).unwrap();
        assert_eq!(p.flush_count(), 1);
        assert_eq!(p.trim_count(), 0);
        assert_eq!(p.status().output, OutputState::Stopped);

        let (mut p, _) = two_tracks_queued();
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 700).unwrap();
        assert_eq!(p.flush_count(), 1);
        assert_eq!(p.trim_count(), 0);
        assert_eq!(p.status().output, OutputState::Stopped);
        assert_eq!(drain(&mut p), 0);
    }

    /// Once track 2 is audible (track 1 fully played), `strm f` keeps track 2.
    #[test]
    fn strm_f_keeps_whichever_track_is_audible() {
        let (mut p, seq1) = two_tracks_queued();
        let mut buf = vec![0f32; 30_000 * 2];
        let n = p.pull_audio(&mut buf);
        p.played(n as u64);
        assert_eq!(p.playing_track(), Some(seq1 + 1));
        p.take_actions();
        p.on_control_data(&strm(b'f', b'0', b'p', 0), 900).unwrap();
        assert_eq!(p.trim().unwrap().keep_track, seq1 + 1);
        assert_eq!(drain(&mut p), 22050 + 44100 - 30_000);
    }

    /// Robustness: arbitrary `strm` arguments and clocks never panic or overflow.
    #[test]
    fn arbitrary_sync_commands_never_panic() {
        let mut x = 0x1234_5678_9abc_def0u64;
        for corrections in [false, true] {
            let mut p = playing_player(false);
            p.set_output_corrections(corrections);
            let mut now = 0u64;
            let mut buf = vec![0f32; 512];
            for _ in 0..3000 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                now += x % 50;
                let cmd = b"puatsq"[(x >> 8) as usize % 6];
                let arg = (x >> 16) as u32;
                p.on_control_data(&strm_arg(cmd, arg), now).unwrap();
                if cmd == b's' {
                    p.on_stream_data(&wav(48000, 2000), now);
                }
                p.tick(now);
                let n = p.pull_audio(&mut buf);
                p.presented((x >> 40) % (n as u64 + 1), now.saturating_sub(x % 300));
                let _ = p.status();
                p.take_actions();
                p.take_corrections();
            }
        }
    }

    /// Regression (browser player): LMS sends a seek's flush and new stream back to back
    /// (about 2 ms apart for MP3), often in one message. A host that watched the output
    /// state on a timer never saw "Stopped" and kept the old audio queued; the flush count
    /// is what it must follow.
    #[test]
    fn a_flush_is_counted_even_when_the_new_stream_arrives_with_it() {
        let mut p = playing_player(false);
        let before = p.flush_count();
        let mut both = strm(b'q', b'0', b'p', 0);
        both.extend(strm(b's', b'1', b'p', 0));
        p.on_control_data(&both, 50).unwrap();
        assert_eq!(p.flush_count(), before + 1);
        assert_eq!(
            p.status().output,
            OutputState::Waiting,
            "never seen as Stopped"
        );
        let ev = events(&p.take_actions());
        assert!(ev.contains(&"STMf".to_string()) && ev.contains(&"STMc".to_string()));
    }

    // --- Read-ahead mode (rolling cache, Phase A) --------------------------------------

    /// A player in read-ahead mode that sends `STMd` early after 2 s heard.
    fn read_ahead() -> Player {
        let mut p = Player::new(cfg());
        p.set_ready_when_received(true);
        p.set_ready_after_ms(2000);
        p
    }

    /// Play `ms` of audio in 10 ms steps, as a host would: tick, pull, report it heard.
    fn play_ms(p: &mut Player, ms: u64, now: &mut u64) -> u64 {
        let mut buf = vec![0f32; 441 * 2];
        let mut total = 0;
        for _ in 0..ms / 10 {
            *now += 10;
            p.tick(*now);
            let n = p.pull_audio(&mut buf);
            p.played(n as u64);
            total += n as u64;
        }
        total
    }

    fn count(ev: &[String], name: &str) -> usize {
        ev.iter().filter(|e| *e == name).count()
    }

    fn stream_numbers(actions: &[Action]) -> Vec<u64> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::OpenStream { stream_number, .. } => Some(*stream_number),
                _ => None,
            })
            .collect()
    }

    /// The STAT `bytes_received` LMS would see now (from a heartbeat).
    fn stat_bytes(p: &mut Player, now: u64) -> u64 {
        p.take_actions();
        p.on_control_data(&strm_arg(b't', 1), now).unwrap();
        let a = p.take_actions();
        let Action::Send(bytes) = &a[0] else { panic!() };
        Stat::parse(&bytes[8..]).unwrap().bytes_received
    }

    #[test]
    fn read_ahead_is_off_by_default_and_a_new_stream_replaces_a_decoding_one() {
        let mut p = Player::new(cfg());
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        play_ms(&mut p, 3000, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMd"), 0, "STMd waits for the decoder: {ev:?}");
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert!(p.next.is_none());
        assert_eq!(p.track.as_ref().unwrap().seq, 1, "replaced, as before");
    }

    /// Start a 44.1 kHz track and play some of it, so it is still decoding.
    fn decoding_44k(p: &mut Player, now: &mut u64) {
        p.on_control_data(&strm(b's', b'1', b'p', 0), *now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), *now);
        p.on_stream_end(*now);
        play_ms(p, 3000, now);
        assert!(p.decoding(), "the first track is still decoding");
    }

    /// Play until nothing more comes out, collecting the output rate at each pull and the
    /// frames pulled at each rate.
    fn play_out(p: &mut Player, now: &mut u64) -> Vec<(u32, u64)> {
        let mut buf = vec![0f32; 441 * 2];
        let mut by_rate: Vec<(u32, u64)> = Vec::new();
        let mut idle = 0;
        while idle < 300 {
            *now += 10;
            p.tick(*now);
            let rate = p.output_rate();
            let n = p.pull_audio(&mut buf);
            p.played(n as u64);
            if n == 0 {
                idle += 1;
                continue;
            }
            idle = 0;
            let rate = rate.unwrap();
            match by_rate.last_mut() {
                Some((r, f)) if *r == rate => *f += n as u64,
                _ => by_rate.push((rate, n as u64)),
            }
        }
        by_rate
    }

    /// Regression (M12): a bare `strm s` replacing a track that is still decoding left that
    /// track's last segment incomplete for ever. Once its audio was pulled, `output_rate`
    /// still named it, so a new track at another rate was never pulled, `STMu` never went,
    /// and every such replacement left another orphan segment behind.
    #[test]
    fn a_new_stream_replacing_a_decoding_one_at_another_rate_plays() {
        let mut p = Player::new(cfg());
        let mut now = 0;
        decoding_44k(&mut p, &mut now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(48000, 48000 * 2), now);
        p.on_stream_end(now);
        p.take_actions();
        let by_rate = play_out(&mut p, &mut now);
        // The old track's decoded audio plays out, then all of the new track at 48 kHz.
        assert_eq!(by_rate.last(), Some(&(48000, 96000)), "{by_rate:?}");
        assert_eq!(by_rate[0].0, 44100, "{by_rate:?}");
        assert_eq!(p.output_rate(), None);
        // Both tracks are done and played out: the end is reported.
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMu"), 1, "{ev:?}");
        assert_eq!(p.status().output, OutputState::Stopped);
    }

    #[test]
    fn repeated_replacements_leave_no_segments_behind() {
        let mut p = Player::new(cfg());
        let mut now = 0;
        for _ in 0..6 {
            decoding_44k(&mut p, &mut now);
        }
        play_out(&mut p, &mut now);
        p.take_actions();
        // Only the audible track's segment is kept, for the elapsed time.
        assert!(p.segments.len() <= 1, "{} segments", p.segments.len());
        assert!(p.segments.iter().all(|s| s.complete));
    }

    /// A FLAC seek from LMS is bare frames with no header (#12). The player makes one, plays
    /// the audio from the seek, says so once to the host, and never sends `STMn`.
    #[test]
    fn a_flac_seek_stream_without_a_header_plays_and_is_reported() {
        const FLAC: &[u8] = include_bytes!("../tests/fixtures/sine-44k-1152.flac");
        const FRAME_10: usize = 8729; // as `flac --analyze` lists it; 39 frames of 1,152
        let mut p = Player::new(cfg());
        p.set_diagnostics(true);
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'f', 0), now).unwrap();
        p.on_stream_data(&FLAC[FRAME_10..], now);
        p.on_stream_end(now);
        let by_rate = play_out(&mut p, &mut now);
        // One second of audio; from frame 10 on, 32,580 frames remain.
        assert_eq!(by_rate.len(), 1, "{by_rate:?}");
        assert_eq!(by_rate[0].0, 44100, "{by_rate:?}");
        assert!((31_500..=33_500).contains(&by_rate[0].1), "{by_rate:?}");
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMn"), 0, "{ev:?}");
        assert_eq!(count(&ev, "STMs"), 1, "{ev:?}");
        assert_eq!(count(&ev, "STMu"), 1, "{ev:?}");
        let made: Vec<_> = p
            .take_diagnostics()
            .into_iter()
            .filter(|d| matches!(d, DiagEvent::FlacHeader { .. }))
            .collect();
        assert_eq!(
            made,
            vec![DiagEvent::FlacHeader {
                rate: 44100,
                channels: 2,
                bits: 16
            }]
        );
    }

    /// Regression (M12): a refused codec's track set its stream and input as ended but never
    /// ended the decoder's input, so it probed for ever: the player counted as decoding, a
    /// dry output sent `STMo` (an underrun) rather than letting `STMu` (the end) go, and it
    /// asked for stream data that would never come.
    #[test]
    fn a_refused_codec_is_not_decoding() {
        let mut p = Player::new(cfg());
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 2), now);
        p.on_stream_end(now);
        play_ms(&mut p, 200, &mut now);
        assert_eq!(p.status().output, OutputState::Playing);
        p.take_actions();
        // Refused while the first track still plays.
        p.on_control_data(&strm(b's', b'1', b'w', 0), now).unwrap();
        assert_eq!(events(&p.take_actions()), vec!["STMn"]);
        assert!(!p.decoding());
        assert!(!p.wants_stream_data());
        assert!(p.fed_streams().is_empty());
        play_out(&mut p, &mut now);
        p.output_underrun();
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMo"), 0, "{ev:?}");
        assert_eq!(count(&ev, "STMn"), 0, "refused once only: {ev:?}");
        assert_eq!(count(&ev, "STMu"), 1, "{ev:?}");
    }

    #[test]
    fn read_ahead_sends_stmd_once_the_stream_is_in_and_enough_is_heard() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        p.tick(now);
        assert_eq!(p.status().output, OutputState::Playing);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "DSCO"), 1);
        assert_eq!(count(&ev, "STMd"), 0, "nothing heard yet");
        play_ms(&mut p, 1900, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMd"), 0, "1.9 s heard");
        play_ms(&mut p, 200, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMd"), 1);
        assert!(
            !p.track.as_ref().unwrap().decode_done,
            "well before the decoder"
        );
        // The rest of the track: the decoder finishing sends no second STMd.
        play_ms(&mut p, 13_000, &mut now);
        assert!(p.track.as_ref().unwrap().decode_done);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMd"), 0);
        assert_eq!(count(&ev, "STMu"), 1);
        assert_eq!(p.sent_log, vec![("STMs", 0), ("STMd", 0)]);
    }

    #[test]
    fn read_ahead_waits_for_the_whole_stream() {
        let mut p = read_ahead();
        let mut now = 0;
        let full = wav(44100, 44100 * 15);
        let half = full.len() / 2;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&full[..half], now);
        play_ms(&mut p, 3000, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMd") + count(&ev, "DSCO"), 0, "{ev:?}");
        p.on_stream_data(&full[half..], now);
        p.on_stream_end(now);
        p.tick(now + 1);
        let ev = events(&p.take_actions());
        assert_eq!((count(&ev, "DSCO"), count(&ev, "STMd")), (1, 1));
    }

    /// A track shorter than the wait (or fully decoded early) sends `STMd` when decoded, as
    /// before, and only once.
    #[test]
    fn read_ahead_short_track_is_ready_when_decoded() {
        let mut p = read_ahead();
        p.set_ready_after_ms(DEFAULT_READY_AFTER_MS);
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 3), now);
        p.on_stream_end(now);
        tick_n(&mut p, 0, 20);
        now = 20;
        assert_eq!(count(&events(&p.take_actions()), "STMd"), 1);
        play_ms(&mut p, 3100, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMd"), 0);
    }

    /// Regression (#30): a track small enough to fit the input whole has its input end at
    /// once, so the host feeds the queued next track whole into the queued slot. The status
    /// counts that track's undecoded bytes too, and the byte rate is the decoding track's
    /// own: a "buffered ahead" figure that saw only the current track's input, and took the
    /// session's every byte as decoded, showed a third of what was really held.
    #[test]
    fn status_counts_the_queued_tracks_input_and_the_byte_rate_is_the_tracks_own() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert_eq!(p.decoded_rate(), None, "nothing decoded yet");
        let t1 = wav(44100, 44100 * 15);
        p.on_stream_data(&t1, now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        p.take_actions();
        let st = p.status();
        assert!(
            st.input_bytes > 0 && st.input_bytes < t1.len(),
            "the track's bytes beyond what is decoded ahead: {}",
            st.input_bytes
        );
        let rate = p.decoded_rate().expect("a second decoded");
        assert!(
            (rate - 176.4).abs() < 176.4 * 0.02,
            "CD-quality PCM is 176.4 bytes a millisecond: {rate}"
        );

        // The next track, queued and fed whole, counts; the rate is still track 1's.
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.take_actions();
        let before = p.status().input_bytes;
        let t2 = wav(44100, 44100 * 5);
        assert!(p.wants_stream_data(), "the queued track takes bytes");
        p.on_stream_data(&t2, now);
        p.on_stream_end(now);
        assert_eq!(p.status().input_bytes, before + t2.len());
        assert_eq!(p.decoded_rate(), Some(rate));

        // Past the hand-over, the rate is track 2's own, from its bytes and its audio.
        play_ms(&mut p, 14_000, &mut now);
        assert_eq!(p.status().input_bytes, 0, "both decoded");
        let rate2 = p.decoded_rate().expect("a second of track 2 decoded");
        assert!(
            (rate2 - 176.4).abs() < 176.4 * 0.02,
            "track 2's rate from its own bytes: {rate2}"
        );
        play_ms(&mut p, 5000, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMu"), 1, "{ev:?}");
    }

    /// The byte rate leaves out what came before the audio: a tag with a megabyte of
    /// artwork is read by the probe and gives no audio, and would otherwise make the start
    /// of a track look far denser than it is (a cushion a third short for its first minute).
    #[test]
    fn the_byte_rate_leaves_out_tags_and_artwork_ahead_of_the_audio() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        // A JUNK chunk of 1 MiB between the format and the data, as artwork would be.
        let mut t = wav(44100, 44100 * 20);
        let junk = 1usize << 20;
        let mut chunk = b"JUNK".to_vec();
        chunk.extend_from_slice(&(junk as u32).to_le_bytes());
        chunk.resize(8 + junk, 0);
        t.splice(36..36, chunk);
        let riff_len = (t.len() - 8) as u32;
        t[4..8].copy_from_slice(&riff_len.to_le_bytes());
        p.on_stream_data(&t, now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        assert_eq!(p.status().output, OutputState::Playing);
        let rate = p.decoded_rate().expect("a second decoded");
        // Without the junk left out, about 260: the reader's first read past the headers
        // is the only give.
        assert!(
            (rate - 176.4).abs() < 176.4 * 0.05,
            "the audio's own rate, not the junk's: {rate}"
        );
    }

    /// The next `strm s` comes while the current track is still decoding: it is queued and
    /// fetched at once, the current track plays on, and the two join gaplessly.
    #[test]
    fn read_ahead_queues_the_next_track_while_the_current_one_decodes() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        let mut heard = play_ms(&mut p, 2100, &mut now);
        p.take_actions();

        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        let a = p.take_actions();
        assert!(!a.contains(&Action::CloseStream));
        assert_eq!(stream_numbers(&a), vec![2]);
        assert_eq!(events(&a), vec!["STMc"]);
        assert_eq!(p.status().output, OutputState::Playing);
        assert_eq!(p.open_stream(), Some((2, 0)));
        assert!(p.wants_stream_data(), "the next track is fetched now");

        let t2 = wav(44100, 44100 * 3);
        p.on_stream_data(&t2, now);
        assert_eq!(p.open_stream(), Some((2, t2.len() as u64)));
        assert_eq!(stat_bytes(&mut p, now), t2.len() as u64);
        assert_eq!(p.track.as_ref().unwrap().seq, 0, "track 1 still decoding");
        p.on_stream_end(now);
        assert_eq!(p.open_stream(), None);
        assert_eq!(count(&events(&p.take_actions()), "DSCO"), 1);

        // Track 1 decodes to its end, then track 2 follows without a gap.
        heard += play_ms(&mut p, 16_500, &mut now);
        assert_eq!(heard, 44100 * 18);
        assert!(p.next.is_none());
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMs"), 1, "track 2 started: {ev:?}");
        assert_eq!(count(&ev, "STMu"), 1);
        assert_eq!(
            p.sent_log,
            vec![("STMs", 0), ("STMd", 0), ("STMd", 1), ("STMs", 1)],
            "track 2 is short: ready once decoded, before it starts"
        );
    }

    /// A host with a disk spool: the network stream is in before the decoder has been fed
    /// all of it. Fed bytes go to the current track until its input ends, then to the next.
    #[test]
    fn read_ahead_with_a_spool_feeds_the_current_track_while_the_next_is_fetched() {
        let mut p = read_ahead();
        let mut now = 0;
        let t1 = wav(44100, 44100 * 15);
        let fed = 44 + 5 * WAV_BYTES_PER_SEC;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&t1[..fed], now);
        p.stream_bytes_received(t1.len() as u64);
        p.stream_bytes_received(10); // never backwards
        assert_eq!(p.open_stream(), Some((1, t1.len() as u64)));
        p.stream_received(now);
        assert_eq!(p.open_stream(), None);
        assert_eq!(events(&p.take_actions()), vec!["STMc", "DSCO"]);
        play_ms(&mut p, 2100, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMd"), 1);

        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert!(p.next.is_some());
        assert_eq!(p.open_stream(), Some((2, 0)));
        p.stream_bytes_received(1234);
        assert_eq!(p.open_stream(), Some((2, 1234)));
        // Feeding carries on into track 1.
        p.on_stream_data(&t1[fed..], now);
        assert_eq!(p.track.as_ref().unwrap().bytes_fed, t1.len() as u64);
        p.on_stream_end(now);
        assert_eq!(count(&events(&p.take_actions()), "DSCO"), 0, "already sent");
        // Now track 2 is fed.
        let t2 = wav(44100, 44100 * 2);
        p.on_stream_data(&t2, now);
        assert_eq!(p.next.as_ref().unwrap().bytes_fed, t2.len() as u64);
        p.on_stream_end(now);
        assert_eq!(count(&events(&p.take_actions()), "DSCO"), 1);
        let heard = play_ms(&mut p, 15_000, &mut now);
        assert_eq!(
            heard,
            44100 * 17 - 2100 * 441 / 10,
            "the rest of 1, all of 2"
        );
        assert_eq!(p.sent_log.iter().filter(|e| e.0 == "STMs").count(), 2);
    }

    /// A queued track that isn't decoding yet is held to the input limit, not the larger
    /// probe allowance.
    #[test]
    fn read_ahead_queued_track_is_held_to_the_input_limit() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&vec![0u8; INPUT_HIGH_WATER - 1], now);
        assert!(p.wants_stream_data());
        p.on_stream_data(&[0u8; 1], now);
        assert!(!p.wants_stream_data());
    }

    /// Nothing more is queued yet, and the audio runs out: `STMu` waits until everything has
    /// played out, and an underrun meanwhile is `STMo`.
    #[test]
    fn read_ahead_sends_stmu_only_when_everything_is_drained() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 12), now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap(); // track 2: no data yet
        play_ms(&mut p, 10_500, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMu"), 0, "track 2 is still to come: {ev:?}");
        assert!(!p.audio_exhausted());
        assert_eq!(p.status().output, OutputState::Playing);
        p.output_underrun();
        assert_eq!(events(&p.take_actions()), vec!["STMo"]);
        // Track 2 arrives: it starts (STMs), with no STMr for a new track.
        p.on_stream_data(&wav(44100, 44100), now);
        p.on_stream_end(now);
        play_ms(&mut p, 1500, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMs"), 1);
        assert_eq!(count(&ev, "STMr"), 0);
        assert_eq!(count(&ev, "STMu"), 1);
    }

    #[test]
    fn read_ahead_strm_q_clears_the_queue_and_strm_f_keeps_the_current_track() {
        let setup = || {
            let mut p = read_ahead();
            let mut now = 0;
            p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
            p.on_stream_data(&wav(44100, 44100 * 15), now);
            p.on_stream_end(now);
            play_ms(&mut p, 2100, &mut now);
            p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
            p.on_stream_data(&wav(44100, 44100)[..1000], now);
            p.take_actions();
            (p, now)
        };
        let (mut p, now) = setup();
        p.on_control_data(&strm(b'q', b'0', b'p', 0), now).unwrap();
        let a = p.take_actions();
        assert_eq!(a.iter().filter(|x| **x == Action::CloseStream).count(), 1);
        assert_eq!(events(&a), vec!["STMf"]);
        assert!(p.next.is_none() && p.track.is_none());
        assert_eq!(p.status().output, OutputState::Stopped);

        let (mut p, mut now) = setup();
        p.on_control_data(&strm(b'f', b'0', b'p', 0), now).unwrap();
        let a = p.take_actions();
        assert_eq!(a[0], Action::CloseStream, "the next track's fetch");
        assert_eq!(events(&a), vec!["STMf"]);
        assert!(p.next.is_none());
        assert_eq!(p.trim().unwrap().keep_track, 0);
        assert_eq!(p.open_stream(), None);
        // Track 1 plays on to its end.
        let heard = play_ms(&mut p, 13_500, &mut now);
        assert_eq!(heard, 44100 * 15 - 2100 * 441 / 10);
        assert_eq!(count(&events(&p.take_actions()), "STMu"), 1);
    }

    /// The streams a spooling host keeps follow the tracks that still take input.
    #[test]
    fn fed_streams_follow_the_tracks_still_taking_input() {
        let mut p = read_ahead();
        let mut now = 0;
        assert!(p.fed_streams().is_empty());
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert_eq!(p.fed_streams(), vec![1]);
        let t1 = wav(44100, 44100 * 15);
        p.on_stream_data(&t1[..t1.len() / 2], now);
        p.stream_received(now);
        play_ms(&mut p, 2100, &mut now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert_eq!(p.fed_streams(), vec![1, 2], "current, then queued");
        p.on_stream_data(&t1[t1.len() / 2..], now);
        p.on_stream_end(now);
        assert_eq!(p.fed_streams(), vec![2]);
        // A partial flush drops the queued track.
        p.on_control_data(&strm(b'f', b'0', b'p', 0), now).unwrap();
        assert!(p.fed_streams().is_empty());
        // A new stream replacing one still fetched leaves only the new one.
        p.on_control_data(&strm(b'q', b'0', b'p', 0), now).unwrap();
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert_eq!(p.fed_streams(), vec![4]);
        p.on_control_data(&strm(b'q', b'0', b'p', 0), now).unwrap();
        assert!(p.fed_streams().is_empty());
    }

    /// LMS never sends two ahead; if it did, the newer one replaces the queued one.
    #[test]
    fn read_ahead_a_second_next_track_replaces_the_queued_one() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.take_actions();
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        let a = p.take_actions();
        assert_eq!(a[0], Action::CloseStream);
        assert_eq!(stream_numbers(&a), vec![3]);
        assert_eq!(p.next.as_ref().unwrap().seq, 2);
        assert_eq!(p.track.as_ref().unwrap().seq, 0);
    }

    /// A refused next track (unsupported format) leaves the current track alone.
    #[test]
    fn read_ahead_a_refused_next_track_leaves_the_current_one() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        p.take_actions();
        p.on_control_data(&strm(b's', b'1', b'w', 0), now).unwrap();
        let a = p.take_actions();
        assert_eq!(events(&a), vec!["STMn"]);
        assert!(stream_numbers(&a).is_empty());
        assert!(p.next.is_none());
        assert_eq!(p.track.as_ref().unwrap().seq, 0);
        // Numbering still counts it.
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        assert_eq!(stream_numbers(&p.take_actions()), vec![3]);
    }

    /// Security: a malformed `strm` is refused as before and doesn't disturb the queue.
    #[test]
    fn read_ahead_malformed_strm_is_refused_and_changes_nothing() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        p.on_stream_end(now);
        play_ms(&mut p, 2100, &mut now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.take_actions();
        let good = strm(b's', b'1', b'p', 0);
        let truncated = encode_server_frame(*b"strm", &good[6..20]);
        assert!(p.on_control_data(&truncated, now).is_err());
        assert!(p.take_actions().is_empty());
        assert_eq!(p.next.as_ref().unwrap().seq, 1);
        let mut q = read_ahead();
        assert!(
            q.on_control_data(&[0xff, 0xff, b's', b't', b'r', b'm'], 0)
                .is_err(),
            "oversize frame"
        );
    }

    /// Out of audio with the link down: `STMo`, not `STMu` (which would make LMS stop with
    /// the next track ready). `STMu` only once the link has been back a while and LMS has
    /// sent nothing new.
    #[test]
    fn link_down_turns_the_end_of_the_audio_into_an_underrun() {
        let mut p = playing_player(false);
        let mut now = 20;
        p.set_link_up(false, now);
        play_ms(&mut p, 1100, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMu"), 0);
        assert_eq!(count(&ev, "STMo"), 1, "{ev:?}");
        assert_eq!(p.status().output, OutputState::Playing);
        assert!(p.stall_ms(now).is_some());
        play_ms(&mut p, 500, &mut now);
        assert!(events(&p.take_actions()).iter().all(|e| e == "STMt"));
        p.set_link_up(true, now);
        let back = now;
        play_ms(&mut p, LINK_SETTLE_MS - 100, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMu"), 0);
        play_ms(&mut p, 200, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMu"), 1);
        assert!(now - back >= LINK_SETTLE_MS);
        assert_eq!(p.status().output, OutputState::Stopped);
    }

    /// When the link comes back LMS answers the delayed `STMd` with the next track: it
    /// starts with `STMs`, and no `STMu` or `STMr` is sent.
    #[test]
    fn link_back_with_the_next_track_starts_it() {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100), now);
        p.on_stream_end(now);
        p.set_link_up(false, now);
        play_ms(&mut p, 1500, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMo"), 1);
        p.set_link_up(true, now);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now + 50)
            .unwrap();
        p.on_stream_data(&wav(44100, 44100), now + 50);
        p.on_stream_end(now + 50);
        play_ms(&mut p, 2000, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMs"), 1);
        assert_eq!(count(&ev, "STMr"), 0);
        assert_eq!(count(&ev, "STMu"), 1, "after track 2 played out: {ev:?}");
        assert_eq!(p.sent_log.iter().filter(|e| e.0 == "STMs").count(), 2);
    }

    /// After an `STMo` mid-track, the audio coming back is `STMr` in read-ahead mode, never
    /// a second `STMs` (which would move a whole sync group on a track).
    #[test]
    fn read_ahead_resume_after_an_underrun_is_stmr() {
        for mode in [true, false] {
            let mut p = Player::new(cfg());
            p.set_ready_when_received(mode);
            let mut now = 0;
            let full = wav(44100, 44100 * 4);
            p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
            p.on_stream_data(&full[..44 + WAV_BYTES_PER_SEC], now);
            play_ms(&mut p, 1500, &mut now);
            p.take_actions();
            p.output_underrun();
            assert_eq!(events(&p.take_actions()), vec!["STMo"]);
            p.on_stream_data(&full[44 + WAV_BYTES_PER_SEC..], now);
            p.on_stream_end(now);
            play_ms(&mut p, 500, &mut now);
            let ev = events(&p.take_actions());
            assert_eq!(count(&ev, "STMs"), 0);
            assert_eq!(count(&ev, "STMr"), usize::from(mode), "mode {mode}: {ev:?}");
        }
    }

    /// An `STMo` the host withdrew (never delivered) is followed by no `STMr` when the audio
    /// comes back, and no second `STMo` while the same stall lasts; the next underrun, after
    /// audio has flowed, is reported as usual.
    #[test]
    fn a_withdrawn_underrun_report_sends_no_stmr_and_the_next_underrun_is_reported() {
        let mut p = read_ahead();
        let mut now = 0;
        let full = wav(44100, 44100 * 4);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&full[..44 + WAV_BYTES_PER_SEC], now);
        play_ms(&mut p, 1500, &mut now);
        p.take_actions();
        p.output_underrun();
        assert_eq!(events(&p.take_actions()), vec!["STMo"]);
        p.withdraw_underrun_report();
        p.output_underrun();
        assert!(p.take_actions().is_empty(), "still one report per stall");
        assert!(p.stall_ms(now + 10).is_some(), "the stall is still timed");
        p.on_stream_data(
            &full[44 + WAV_BYTES_PER_SEC..44 + 2 * WAV_BYTES_PER_SEC],
            now,
        );
        play_ms(&mut p, 300, &mut now);
        let ev = events(&p.take_actions());
        assert_eq!(count(&ev, "STMr"), 0, "{ev:?}");
        assert_eq!(count(&ev, "STMs"), 0, "{ev:?}");
        assert_eq!(p.stall_ms(now), None);
        // A real underrun later is reported, and its recovery too.
        play_ms(&mut p, 2000, &mut now);
        p.take_actions();
        p.output_underrun();
        assert_eq!(events(&p.take_actions()), vec!["STMo"]);
        p.on_stream_data(&full[44 + 2 * WAV_BYTES_PER_SEC..], now);
        p.on_stream_end(now);
        play_ms(&mut p, 300, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMr"), 1);
    }

    /// Withdrawing with nothing reported, or after the audio came back, changes nothing.
    #[test]
    fn withdrawing_without_an_unrecovered_underrun_does_nothing() {
        let mut p = read_ahead();
        let mut now = 0;
        let full = wav(44100, 44100 * 4);
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&full[..44 + WAV_BYTES_PER_SEC], now);
        play_ms(&mut p, 500, &mut now);
        p.take_actions();
        p.withdraw_underrun_report();
        play_ms(&mut p, 1000, &mut now);
        p.take_actions();
        p.output_underrun();
        assert_eq!(events(&p.take_actions()), vec!["STMo"]);
        p.on_stream_data(&full[44 + WAV_BYTES_PER_SEC..], now);
        p.on_stream_end(now);
        play_ms(&mut p, 300, &mut now);
        assert_eq!(count(&events(&p.take_actions()), "STMr"), 1);
    }

    /// A player with 15 s of a track decoding, 1 s heard, still streaming.
    fn stall_player() -> (Player, u64) {
        let mut p = read_ahead();
        let mut now = 0;
        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
        p.on_stream_data(&wav(44100, 44100 * 15), now);
        play_ms(&mut p, 1000, &mut now);
        p.take_actions();
        (p, now)
    }

    #[test]
    fn stalls_are_timed_and_skipped_only_in_a_sync_group() {
        let (mut p, now) = stall_player();
        assert_eq!(p.resume_after_stall(true, now), StallResume::NotStalled);
        assert_eq!(p.stall_ms(now), None);

        // Alone: carry on where it stopped, whatever the length.
        p.output_underrun();
        p.output_underrun(); // timed from the first report
        assert_eq!(p.stall_ms(now + 100), Some(100));
        let before = p.status();
        assert_eq!(
            p.resume_after_stall(false, now + 300),
            StallResume::CarryOn { stalled_ms: 300 }
        );
        assert_eq!(p.stall_ms(now + 300), None);
        assert_eq!(p.status().buffered_ms, before.buffered_ms);

        // In a group, a short stall is left to LMS's own corrections.
        let now = now + 1000;
        p.tick(now);
        p.output_underrun();
        assert_eq!(
            p.resume_after_stall(true, now + SYNC_STALL_SKIP_MS),
            StallResume::CarryOn {
                stalled_ms: SYNC_STALL_SKIP_MS
            }
        );

        // A longer one is dropped from the buffer, and elapsed time moves on with it.
        let now = now + 1000;
        p.tick(now);
        p.output_underrun();
        let before = p.status();
        assert_eq!(
            p.resume_after_stall(true, now + 500),
            StallResume::Skipped { stalled_ms: 500 }
        );
        let after = p.status();
        assert!((before.buffered_ms - after.buffered_ms).abs_diff(500) <= 1);
        assert!((after.elapsed_ms - before.elapsed_ms).abs_diff(500) <= 1);

        // Longer than the buffer: nothing dropped; the host flushes.
        let now = now + 1000;
        p.tick(now);
        p.output_underrun();
        let before = p.status().buffered_ms;
        assert_eq!(
            p.resume_after_stall(true, now + 60_000),
            StallResume::NotEnoughBuffered {
                stalled_ms: 60_000,
                buffered_ms: before
            }
        );
        assert_eq!(p.status().buffered_ms, before);
    }

    #[test]
    fn audio_flowing_again_ends_a_stall_and_a_flush_forgets_it() {
        let (mut p, now) = stall_player();
        p.output_underrun();
        let mut buf = vec![0f32; 64];
        let n = p.pull_audio(&mut buf);
        p.played(n as u64);
        assert_eq!(p.stall_ms(now + 10), None);
        p.output_underrun();
        p.on_control_data(&strm(b'q', b'0', b'p', 0), now).unwrap();
        assert_eq!(p.stall_ms(now + 10), None);
        // No stall while not playing.
        p.output_underrun();
        assert_eq!(p.stall_ms(now + 10), None);
    }

    #[test]
    fn skip_audio_drops_at_most_what_is_decoded() {
        let (mut p, _) = stall_player();
        let buffered = p.status().buffered_ms;
        assert_eq!(p.skip_audio(250), 250);
        assert!(p.skip_audio(u64::MAX) <= buffered);
        assert_eq!(p.status().buffered_ms, 0);
        assert_eq!(Player::new(cfg()).skip_audio(100), 0);
    }

    /// A skip that crosses into the next track still starts it with one `STMs`.
    #[test]
    fn a_skip_into_the_next_track_still_starts_it_once() {
        let (mut p, seq1) = two_tracks_queued();
        assert_eq!(p.skip_audio(600), 600);
        let mut now = 700;
        play_ms(&mut p, 1000, &mut now);
        assert_eq!(p.playing_track(), Some(seq1 + 1));
        assert_eq!(count(&events(&p.take_actions()), "STMs"), 1);
        let started: Vec<_> = p.sent_log.iter().filter(|e| e.0 == "STMs").collect();
        assert_eq!(started, vec![&("STMs", seq1), &("STMs", seq1 + 1)]);
    }

    /// Robustness and the per-track invariants: random sequences of LMS commands, stream
    /// data, stream ends, link changes, underruns, stalls and skips never panic, never send
    /// `STMs` twice for one track, and never send `STMd` twice for one track.
    #[test]
    fn random_read_ahead_sequences_keep_the_track_invariants() {
        let track = wav(44100, 44100 * 3);
        let (mut queued, mut started) = (0, 0);
        for seed in 1..=40u64 {
            let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let mut rnd = move || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            let mut p = Player::new(cfg());
            p.set_ready_when_received(seed % 4 != 0);
            p.set_ready_after_ms(rnd() % 3000);
            p.set_output_corrections(seed % 3 == 0);
            let mut now = 0u64;
            let mut offset = 0usize;
            let mut buf = vec![0f32; 2048];
            for _ in 0..1000 {
                let r = rnd();
                now += r % 40;
                match (r >> 8) % 18 {
                    0 => {
                        p.on_control_data(&strm(b's', b'1', b'p', 0), now).unwrap();
                        offset = 0;
                    }
                    1 => {
                        let cmd = b"qftupa"[(r >> 16) as usize % 6];
                        p.on_control_data(&strm_arg(cmd, (r >> 24) as u32 % 3000), now)
                            .unwrap();
                    }
                    2..=5 => {
                        let n = ((r >> 16) as usize % 60_000).min(track.len() - offset);
                        p.on_stream_data(&track[offset..offset + n], now);
                        offset += n;
                    }
                    6 => p.on_stream_end(now),
                    7 => p.on_stream_error(now),
                    8 => p.stream_received(now),
                    9 => p.stream_bytes_received(r >> 40),
                    10 => p.set_link_up((r >> 16) % 2 == 0, now),
                    11 => p.output_underrun(),
                    12 => {
                        let _ = p.resume_after_stall((r >> 16) % 2 == 0, now);
                    }
                    13 => {
                        p.skip_audio((r >> 16) % 2000);
                    }
                    14 => {
                        let n = p.pull_track(&mut buf).map_or(0, |(_, n)| n);
                        p.presented((r >> 20) % (n as u64 + 1), now);
                    }
                    _ => {
                        p.tick(now);
                        let n = p.pull_audio(&mut buf);
                        p.played(n as u64);
                    }
                }
                let _ = p.status();
                let _ = p.open_stream();
                let _ = p.wants_stream_data();
                let _ = p.stall_ms(now);
                p.take_actions();
                p.take_corrections();
                queued += usize::from(p.next.is_some());
            }
            started += p.sent_log.iter().filter(|e| e.0 == "STMs").count();
            for event in ["STMs", "STMd"] {
                let seqs: Vec<u32> = p
                    .sent_log
                    .iter()
                    .filter(|e| e.0 == event)
                    .map(|e| e.1)
                    .collect();
                let mut unique = seqs.clone();
                unique.sort_unstable();
                unique.dedup();
                assert_eq!(seqs.len(), unique.len(), "seed {seed}: {event} {seqs:?}");
            }
        }
        assert!(
            queued > 0 && started > 0,
            "the sequences reach the interesting states"
        );
    }
}
