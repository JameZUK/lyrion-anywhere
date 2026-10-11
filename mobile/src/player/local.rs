//! The local source: downloaded files, played from the app's own queue (SPEC D14).
//!
//! It feeds the same [`super::audio::AudioPath`] as the LMS stream, so DSP, levelling, the
//! bit-perfect route and the signal path are the same. Like the core's Slimproto player it
//! hands over PCM in per-track segments; the next track is opened and decoded before the
//! current one ends, so tracks of the same format play gaplessly (a change of rate or
//! output format reopens the output at the boundary, as when streaming).
//!
//! It also keeps account of *plays*: one per track started, ended by the next track, a
//! skip or a stop, with the seconds actually heard (seeking within a track doesn't start a
//! new play).

use std::collections::VecDeque;
use std::path::PathBuf;

use anywhere_core::decode::{DecodeStatus, FileDecoder, SourceFormat};
use anywhere_core::player::TrackAudio;

use super::audio::AudioSource;
use super::queue::{LocalRepeat, PlayQueue};

/// Decoded audio kept ahead of the output (beyond the engine's ring).
const AHEAD_SECS: u64 = 3;
/// Packets decoded per call (a slice of the engine's tick).
const PACKETS_PER_STEP: usize = 8;
/// Steps per tick at most (bounds the time a tick spends decoding).
const STEPS_PER_TICK: usize = 32;
/// Tracks opened ahead of the one playing.
const MAX_SEGMENTS: usize = 3;
/// "Previous" within this many seconds of a track's start goes to the track before.
pub(crate) const RESTART_SECS: f64 = 3.0;

/// One track of the queue, as the engine plays it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalTrack {
    pub key: String,
    pub track_id: i64,
    pub path: PathBuf,
    pub ext: String,
    pub duration_secs: f64,
    /// The album it belongs to (LMS's album id), for transitions; `None` if unknown.
    pub album: Option<String>,
}

/// Opens a track's file for decoding (a seam for tests).
pub(crate) type Opener = Box<dyn Fn(&LocalTrack) -> Result<FileDecoder, String> + Send>;

pub(crate) fn open_file(t: &LocalTrack) -> Result<FileDecoder, String> {
    let f = std::fs::File::open(&t.path)
        .map_err(|e| format!("the download is missing ({})", e.kind()))?;
    FileDecoder::open(Box::new(f), Some(&t.ext))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    Stopped,
    Playing,
    Paused,
}

/// Something the engine reports or records.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SourceEvent {
    /// A track became audible (list index).
    Started(usize),
    /// A play ended: the track (list index), when it started (Unix seconds) and the
    /// seconds heard.
    PlayEnded {
        index: usize,
        started_at: u64,
        seconds: f64,
    },
    /// A track couldn't be played (it is skipped).
    Failed { index: usize, error: String },
    /// The queue played to its end.
    Finished,
}

#[derive(Debug)]
struct Segment {
    seq: u32,
    /// Play-order position and list index of its track.
    pos: usize,
    index: usize,
    rate: u32,
    frames: u64,
    pulled: u64,
    played: u64,
    complete: bool,
    source: Option<SourceFormat>,
    /// Where in its track the segment starts (after a seek), in frames.
    start_frame: u64,
    /// Starting it starts a new play (false for a seek within the same play).
    new_play: bool,
}

struct Play {
    index: usize,
    started_at: u64,
    heard_frames: u64,
    rate: u32,
}

pub(crate) struct LocalSource {
    tracks: Vec<LocalTrack>,
    queue: Option<PlayQueue>,
    opener: Opener,
    pcm: VecDeque<f32>,
    segments: VecDeque<Segment>,
    /// The segment being decoded and its decoder.
    decoding: Option<(u32, FileDecoder)>,
    next_seq: u32,
    /// Play-order position of the newest segment opened (look-ahead continues after it).
    last_opened: Option<usize>,
    /// Nothing more to open (end of the queue, or every track failed).
    open_done: bool,
    playing: Option<u32>,
    transport: Transport,
    fixed: bool,
    flushes: u64,
    events: Vec<SourceEvent>,
    play: Option<Play>,
    /// Where a paused or stopped queue would start (ms into the current track).
    held_position_ms: u64,
    failures_in_row: usize,
    clock: Box<dyn Fn() -> u64 + Send>,
    seed: u64,
}

impl LocalSource {
    pub(crate) fn new(opener: Opener, clock: Box<dyn Fn() -> u64 + Send>, seed: u64) -> Self {
        LocalSource {
            tracks: Vec::new(),
            queue: None,
            opener,
            pcm: VecDeque::new(),
            segments: VecDeque::new(),
            decoding: None,
            next_seq: 0,
            last_opened: None,
            open_done: true,
            playing: None,
            transport: Transport::Stopped,
            fixed: false,
            flushes: 0,
            events: Vec::new(),
            play: None,
            held_position_ms: 0,
            failures_in_row: 0,
            clock,
            seed,
        }
    }

    pub(crate) fn take_events(&mut self) -> Vec<SourceEvent> {
        std::mem::take(&mut self.events)
    }

    /// Bumped whenever queued audio must be dropped (skip, seek, stop).
    pub(crate) fn flush_count(&self) -> u64 {
        self.flushes
    }

    pub(crate) fn transport(&self) -> Transport {
        self.transport
    }

    pub(crate) fn tracks(&self) -> &[LocalTrack] {
        &self.tracks
    }

    pub(crate) fn queue(&self) -> Option<&PlayQueue> {
        self.queue.as_ref()
    }

    /// The list index audible now (or about to be).
    pub(crate) fn current_index(&self) -> Option<usize> {
        if let Some(seq) = self.playing
            && let Some(s) = self.segments.iter().find(|s| s.seq == seq)
        {
            return Some(s.index);
        }
        self.queue.as_ref().map(PlayQueue::current)
    }

    /// Position in the current track.
    pub(crate) fn elapsed_ms(&self) -> u64 {
        if let Some(seq) = self.playing
            && let Some(s) = self.segments.iter().find(|s| s.seq == seq)
            && s.rate > 0
        {
            return (s.start_frame + s.played) * 1000 / u64::from(s.rate);
        }
        self.held_position_ms
    }

    pub(crate) fn duration_ms(&self) -> u64 {
        self.current_index()
            .and_then(|i| self.tracks.get(i))
            .map_or(0, |t| (t.duration_secs * 1000.0) as u64)
    }

    /// Decoded audio waiting, in ms.
    pub(crate) fn buffered_ms(&self) -> u64 {
        match self.segments.iter().find(|s| s.rate > 0) {
            Some(s) => (self.pcm.len() / 2) as u64 * 1000 / u64::from(s.rate),
            None => 0,
        }
    }

    // --- The queue ----------------------------------------------------------------------

    /// Play `tracks` from list index `start`.
    pub(crate) fn set_queue(
        &mut self,
        tracks: Vec<LocalTrack>,
        start: usize,
        shuffle: bool,
        play: bool,
    ) {
        self.end_play();
        let repeat = self
            .queue
            .as_ref()
            .map_or(LocalRepeat::Off, PlayQueue::repeat);
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.queue = PlayQueue::new(tracks.len(), start, shuffle, repeat, self.seed);
        self.tracks = tracks;
        self.failures_in_row = 0;
        if self.queue.is_none() {
            self.stop();
            return;
        }
        self.transport = if play {
            Transport::Playing
        } else {
            Transport::Paused
        };
        let pos = self.queue.as_ref().map_or(0, PlayQueue::position);
        self.start_at(pos, 0.0, true);
    }

    /// Drop all queued audio and decoders.
    fn flush(&mut self) {
        self.pcm.clear();
        self.segments.clear();
        self.decoding = None;
        self.playing = None;
        self.last_opened = None;
        self.open_done = true;
        self.flushes += 1;
    }

    /// Start the track at play-order position `pos`, `secs` in.
    fn start_at(&mut self, pos: usize, secs: f64, new_play: bool) {
        if new_play {
            self.end_play();
        }
        self.flush();
        let Some(q) = self.queue.as_mut() else { return };
        q.set_position(pos);
        self.open_done = false;
        self.held_position_ms = (secs.max(0.0) * 1000.0) as u64;
        self.open_pos(pos, secs, new_play);
    }

    /// Open the track at `pos` as a new segment; on failure, report it and move on.
    fn open_pos(&mut self, pos: usize, secs: f64, new_play: bool) -> bool {
        let Some(index) = self.queue.as_ref().and_then(|q| q.index_at(pos)) else {
            self.open_done = true;
            return false;
        };
        self.last_opened = Some(pos);
        let Some(track) = self.tracks.get(index).cloned() else {
            self.open_done = true;
            return false;
        };
        let opened = (self.opener)(&track).and_then(|mut d| {
            let start = if secs > 0.0 { d.seek(secs)? } else { 0 };
            Ok((d, start))
        });
        match opened {
            Ok((dec, start_frame)) => {
                let seq = self.next_seq;
                self.next_seq = self.next_seq.wrapping_add(1);
                self.segments.push_back(Segment {
                    seq,
                    pos,
                    index,
                    rate: dec.sample_rate().unwrap_or(0),
                    frames: 0,
                    pulled: 0,
                    played: 0,
                    complete: false,
                    source: dec.source_format().copied(),
                    start_frame,
                    new_play,
                });
                self.decoding = Some((seq, dec));
                true
            }
            Err(error) => {
                self.events.push(SourceEvent::Failed { index, error });
                self.failures_in_row += 1;
                if self.failures_in_row >= self.tracks.len() {
                    self.open_done = true;
                }
                false
            }
        }
    }

    /// Open what plays after the newest segment (gapless look-ahead).
    fn open_next(&mut self) -> bool {
        if self.open_done || self.segments.len() >= MAX_SEGMENTS {
            return false;
        }
        let Some(last) = self.last_opened else {
            self.open_done = true;
            return false;
        };
        match self.queue.as_ref().and_then(|q| q.after(last)) {
            Some(pos) => {
                self.open_pos(pos, 0.0, true);
                true
            }
            None => {
                self.open_done = true;
                false
            }
        }
    }

    /// Decode ahead, opening following tracks as each one finishes. Call every tick.
    pub(crate) fn decode_ahead(&mut self) {
        if self.queue.is_none() || self.transport == Transport::Stopped {
            return;
        }
        for _ in 0..STEPS_PER_TICK {
            let rate = self.segments.back().map_or(48_000, |s| s.rate.max(8_000));
            if (self.pcm.len() / 2) as u64 >= u64::from(rate) * AHEAD_SECS {
                return;
            }
            let Some((seq, dec)) = self.decoding.as_mut() else {
                if !self.open_next() {
                    return;
                }
                continue;
            };
            let seq = *seq;
            let mut blocks: Vec<(u32, Vec<f32>)> = Vec::new();
            let status = dec.work(PACKETS_PER_STEP, &mut |r, s| blocks.push((r, s.to_vec())));
            for (r, samples) in blocks {
                self.append(seq, r, &samples);
            }
            match status {
                DecodeStatus::Progress | DecodeStatus::NeedData => {}
                DecodeStatus::Finished => {
                    self.finish_segment(seq);
                    self.failures_in_row = 0;
                }
                DecodeStatus::Failed(error) => {
                    let index = self
                        .segments
                        .iter()
                        .find(|s| s.seq == seq)
                        .map_or(0, |s| s.index);
                    self.events.push(SourceEvent::Failed { index, error });
                    self.finish_segment(seq);
                    self.failures_in_row += 1;
                    if self.failures_in_row >= self.tracks.len() {
                        self.open_done = true;
                    }
                }
            }
        }
    }

    fn finish_segment(&mut self, seq: u32) {
        if let Some(s) = self.segments.iter_mut().find(|s| s.seq == seq) {
            s.complete = true;
        }
        if self.decoding.as_ref().is_some_and(|(d, _)| *d == seq) {
            self.decoding = None;
        }
    }

    fn append(&mut self, seq: u32, rate: u32, samples: &[f32]) {
        let Some(i) = self.segments.iter().position(|s| s.seq == seq) else {
            return;
        };
        if self.segments[i].rate != rate && self.segments[i].frames > 0 {
            // The rate changed inside a file: carry on in a new segment of the same play.
            let (pos, index) = (self.segments[i].pos, self.segments[i].index);
            let start = self.segments[i].start_frame + self.segments[i].frames;
            self.segments[i].complete = true;
            let new_seq = self.next_seq;
            self.next_seq = self.next_seq.wrapping_add(1);
            let source = self.segments[i].source;
            self.segments.push_back(Segment {
                seq: new_seq,
                pos,
                index,
                rate,
                frames: 0,
                pulled: 0,
                played: 0,
                complete: false,
                source,
                start_frame: start,
                new_play: false,
            });
            if let Some((d, _)) = self.decoding.as_mut() {
                *d = new_seq;
            }
            return self.append(new_seq, rate, samples);
        }
        let s = &mut self.segments[i];
        s.rate = rate;
        s.frames += (samples.len() / 2) as u64;
        self.pcm.extend(samples.iter().copied());
    }

    // --- Transport ----------------------------------------------------------------------

    pub(crate) fn play(&mut self) {
        let Some(q) = self.queue.as_ref() else { return };
        match self.transport {
            Transport::Playing => {}
            Transport::Paused => self.transport = Transport::Playing,
            Transport::Stopped => {
                // After the end, or a stop: start the current track again.
                let pos = q.position();
                self.transport = Transport::Playing;
                let secs = self.held_position_ms as f64 / 1000.0;
                self.start_at(pos, secs, true);
            }
        }
    }

    pub(crate) fn pause(&mut self) {
        if self.transport == Transport::Playing {
            self.held_position_ms = self.elapsed_ms();
            self.transport = Transport::Paused;
        }
    }

    /// Stop and forget the queue.
    pub(crate) fn stop(&mut self) {
        self.end_play();
        self.flush();
        self.queue = None;
        self.tracks.clear();
        self.transport = Transport::Stopped;
        self.held_position_ms = 0;
    }

    pub(crate) fn next(&mut self) -> bool {
        let Some(pos) = self.sync_queue().and_then(|q| q.next_pos()) else {
            return false;
        };
        self.start_at(pos, 0.0, true);
        true
    }

    /// Back to the start of the track, or (near its start) the track before.
    pub(crate) fn previous(&mut self) {
        let near_start = (self.elapsed_ms() as f64) < RESTART_SECS * 1000.0;
        let before = self.sync_queue().and_then(|q| q.previous_pos());
        let current = self.queue.as_ref().map_or(0, PlayQueue::position);
        match before {
            Some(pos) if near_start => self.start_at(pos, 0.0, true),
            _ => self.start_at(current, 0.0, false),
        }
    }

    /// Play list index `index` now.
    pub(crate) fn jump(&mut self, index: usize) -> bool {
        let Some(q) = self.queue.as_mut() else {
            return false;
        };
        if !q.jump_to_index(index) {
            return false;
        }
        let pos = q.position();
        self.start_at(pos, 0.0, true);
        true
    }

    /// Go to `ms` in the current track (the same play carries on).
    pub(crate) fn seek(&mut self, ms: u64) {
        let Some(pos) = self.sync_queue().map(PlayQueue::position) else {
            return;
        };
        let max = self.duration_ms();
        let ms = if max > 0 { ms.min(max) } else { ms };
        if self.transport == Transport::Stopped {
            self.held_position_ms = ms;
            return;
        }
        self.start_at(pos, ms as f64 / 1000.0, false);
        self.held_position_ms = ms;
    }

    pub(crate) fn set_shuffle(&mut self, on: bool) {
        let Some(q) = self.sync_queue() else { return };
        if q.shuffle() == on {
            return;
        }
        let cur = q.current();
        if let Some(q) = self.queue.as_mut() {
            q.set_shuffle(on);
            q.jump_to_index(cur);
        }
        self.retarget_lookahead();
    }

    pub(crate) fn set_repeat(&mut self, repeat: LocalRepeat) {
        if let Some(q) = self.queue.as_mut() {
            q.set_repeat(repeat);
        }
        self.retarget_lookahead();
    }

    /// Point the queue at the track audible now (it moves on as tracks play).
    fn sync_queue(&mut self) -> Option<&PlayQueue> {
        if let Some(seq) = self.playing
            && let Some(s) = self.segments.iter().find(|s| s.seq == seq)
            && let Some(q) = self.queue.as_mut()
        {
            q.set_position(s.pos);
        }
        self.queue.as_ref()
    }

    /// The queue's order or repeat changed: drop look-ahead that hasn't reached the output,
    /// and carry on from the track playing now.
    fn retarget_lookahead(&mut self) {
        // Keep what has reached the output (or, before anything has, the current track).
        let keep = self
            .segments
            .iter()
            .filter(|s| s.pulled > 0 || Some(s.seq) == self.playing)
            .map(|s| s.seq)
            .max()
            .or_else(|| {
                self.playing
                    .is_none()
                    .then(|| self.segments.front().map(|s| s.seq))
                    .flatten()
            });
        let Some(keep_upto) = keep else {
            // Nothing worth keeping: carry on after the current track.
            self.pcm.clear();
            self.segments.clear();
            self.decoding = None;
            self.last_opened = self.queue.as_ref().map(PlayQueue::position);
            self.open_done = self.last_opened.is_none();
            return;
        };
        while self
            .segments
            .back()
            .is_some_and(|s| s.seq != keep_upto && seq_after(s.seq, keep_upto))
        {
            let s = self.segments.pop_back().expect("a segment");
            let unpulled = (s.frames - s.pulled) as usize * 2;
            let n = self.pcm.len().saturating_sub(unpulled);
            self.pcm.truncate(n);
            if self.decoding.as_ref().is_some_and(|(d, _)| *d == s.seq) {
                self.decoding = None;
            }
        }
        // The kept segments' positions follow their tracks in the new order.
        let Some(q) = self.queue.as_ref() else { return };
        let positions: Vec<Option<usize>> = self
            .segments
            .iter()
            .map(|s| q.order().iter().position(|i| *i == s.index))
            .collect();
        for (s, p) in self.segments.iter_mut().zip(positions) {
            if let Some(p) = p {
                s.pos = p;
            }
        }
        self.last_opened = self.segments.back().map(|s| s.pos);
        self.open_done = self.last_opened.is_none();
    }

    // --- Play accounting ----------------------------------------------------------------

    fn end_play(&mut self) {
        if let Some(p) = self.play.take() {
            let seconds = if p.rate > 0 {
                p.heard_frames as f64 / f64::from(p.rate)
            } else {
                0.0
            };
            self.events.push(SourceEvent::PlayEnded {
                index: p.index,
                started_at: p.started_at,
                seconds,
            });
        }
    }

    /// The queue played to its end: report it and stop (at the last track).
    pub(crate) fn finish(&mut self) {
        self.end_play();
        let last = self.queue.as_ref().map(PlayQueue::position);
        self.flush();
        self.transport = Transport::Stopped;
        self.held_position_ms = 0;
        if let (Some(q), Some(p)) = (self.queue.as_mut(), last) {
            q.set_position(p);
        }
        self.events.push(SourceEvent::Finished);
    }

    /// The queue has nothing left to play: everything decoded has been pulled.
    pub(crate) fn queue_exhausted(&self) -> bool {
        self.queue.is_some()
            && self.open_done
            && self.decoding.is_none()
            && self
                .segments
                .iter()
                .all(|s| s.complete && s.pulled == s.frames)
    }
}

/// `a` comes after `b` in sequence order (wrapping).
fn seq_after(a: u32, b: u32) -> bool {
    a.wrapping_sub(b) < u32::MAX / 2 && a != b
}

impl AudioSource for LocalSource {
    fn next_audio(&self) -> Option<TrackAudio> {
        let s = self
            .segments
            .iter()
            .find(|s| s.pulled < s.frames || !s.complete)?;
        (s.rate > 0 && s.frames > s.pulled).then_some(TrackAudio {
            seq: s.seq,
            sample_rate: s.rate,
            source: s.source,
            server_gain: 1.0,
        })
    }

    fn pull_track(&mut self, out: &mut [f32]) -> Option<(u32, usize)> {
        if self.transport != Transport::Playing {
            return None;
        }
        let want = out.len() / 2;
        // From the first segment with audio left, never across tracks.
        let seg = self
            .segments
            .iter_mut()
            .find(|s| s.pulled < s.frames || !s.complete)?;
        let n = ((seg.frames - seg.pulled) as usize).min(want);
        if n == 0 {
            return None;
        }
        for (i, x) in self.pcm.drain(..n * 2).enumerate() {
            out[i] = x;
        }
        seg.pulled += n as u64;
        Some((seg.seq, n))
    }

    fn track_audio(&self, seq: u32) -> Option<TrackAudio> {
        let s = self.segments.iter().find(|s| s.seq == seq)?;
        Some(TrackAudio {
            seq,
            sample_rate: s.rate,
            source: s.source,
            server_gain: 1.0,
        })
    }

    fn playing_track(&self) -> Option<u32> {
        self.playing
    }

    fn audio_exhausted(&self) -> bool {
        self.decoding.is_none() && self.pcm.is_empty()
    }

    fn applied_volume(&self) -> (f32, f32) {
        // No software volume offline: the system's (or the DAC's) volume applies.
        (1.0, 1.0)
    }

    fn set_fixed_volume(&mut self, fixed: bool) {
        self.fixed = fixed;
    }
}

impl LocalSource {
    /// `frames` pulled earlier have now been played by the output.
    pub(crate) fn played(&mut self, mut frames: u64) {
        while frames > 0 {
            let Some(idx) = self.segments.iter().position(|s| s.played < s.pulled) else {
                break;
            };
            let (seq, index, new_play, pos, rate) = {
                let s = &self.segments[idx];
                (s.seq, s.index, s.new_play, s.pos, s.rate)
            };
            if self.playing != Some(seq) {
                self.playing = Some(seq);
                if let Some(q) = self.queue.as_mut() {
                    q.set_position(pos);
                }
                if new_play || self.play.is_none() {
                    self.end_play();
                    self.play = Some(Play {
                        index,
                        started_at: (self.clock)(),
                        heard_frames: 0,
                        rate,
                    });
                    self.events.push(SourceEvent::Started(index));
                }
            }
            let s = &mut self.segments[idx];
            let n = frames.min(s.pulled - s.played);
            s.played += n;
            frames -= n;
            if let Some(p) = self.play.as_mut() {
                p.heard_frames += n;
                if p.rate == 0 {
                    p.rate = rate;
                }
            }
        }
        // Drop fully played segments before the one audible now (it stays, for elapsed
        // time and the signal path, until the next one starts).
        while self.segments.len() > 1 {
            let s = &self.segments[0];
            if s.complete
                && s.played == s.frames
                && s.pulled == s.frames
                && Some(s.seq) != self.playing
            {
                self.segments.pop_front();
            } else {
                break;
            }
        }
    }

    /// What transitions need to know about track `seq` (its album and length). Downloads
    /// carry no analysis, so smart transitions use their fixed crossfade here.
    pub(crate) fn facts(&self, seq: u32) -> Option<super::transitions::TrackFacts> {
        let s = self.segments.iter().find(|s| s.seq == seq)?;
        let t = self.tracks.get(s.index)?;
        Some(super::transitions::TrackFacts {
            album: t.album.clone(),
            duration_s: (t.duration_secs.is_finite() && t.duration_secs > 0.0)
                .then_some(t.duration_secs),
            edges: None,
        })
    }

    /// [`LocalSource::facts`] for every queued segment (a snapshot for one fill).
    pub(crate) fn fact_seqs(&self) -> Vec<(u32, Option<super::transitions::TrackFacts>)> {
        self.segments
            .iter()
            .map(|s| (s.seq, self.facts(s.seq)))
            .collect()
    }

    /// The rate of the track audible now (tests).
    #[cfg(test)]
    pub(crate) fn rate(&self) -> Option<u32> {
        let seq = self.playing?;
        self.segments.iter().find(|s| s.seq == seq).map(|s| s.rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-bit stereo WAV whose left channel counts frames from `base`.
    fn wav(rate: u32, frames: usize, base: i16) -> Vec<u8> {
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
        for i in 0..frames {
            let x = base.wrapping_add((i % 1000) as i16);
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    /// Tracks backed by in-memory WAVs (`files[i]`; `None`: missing).
    fn source(files: Vec<Option<Vec<u8>>>) -> (LocalSource, Vec<LocalTrack>) {
        let tracks: Vec<LocalTrack> = (0..files.len())
            .map(|i| LocalTrack {
                key: format!("{i:064x}"),
                track_id: 100 + i as i64,
                path: PathBuf::from(format!("/nowhere/{i}")),
                ext: "wav".into(),
                duration_secs: 0.0,
                album: None,
            })
            .collect();
        let opener: Opener = Box::new(move |t: &LocalTrack| {
            let i = (t.track_id - 100) as usize;
            match files.get(i).cloned().flatten() {
                Some(b) => FileDecoder::open(Box::new(std::io::Cursor::new(b)), Some("wav")),
                None => Err("missing".into()),
            }
        });
        (
            LocalSource::new(opener, Box::new(|| 1_800_000_000), 42),
            tracks,
        )
    }

    /// Pull everything playable, marking it played as it goes. Returns the left channel.
    fn drain(s: &mut LocalSource, max_frames: usize) -> Vec<(u32, f32)> {
        let mut out = Vec::new();
        let mut block = vec![0f32; 2048];
        for _ in 0..10_000 {
            s.decode_ahead();
            let want = (max_frames.saturating_sub(out.len())).min(1024);
            match s.pull_track(&mut block[..want * 2]) {
                Some((seq, n)) => {
                    let rate = s.track_audio(seq).unwrap().sample_rate;
                    for f in 0..n {
                        out.push((rate, block[f * 2]));
                    }
                    s.played(n as u64);
                    if out.len() >= max_frames {
                        break;
                    }
                }
                None if s.queue_exhausted() => break,
                None => {}
            }
        }
        out
    }

    #[test]
    fn tracks_follow_each_other_gaplessly() {
        let (mut s, tracks) = source(vec![
            Some(wav(44_100, 30_000, 0)),
            Some(wav(44_100, 20_000, 5000)),
        ]);
        s.set_queue(tracks, 0, false, true);
        let out = drain(&mut s, usize::MAX);
        assert_eq!(out.len(), 50_000, "every frame, nothing added");
        for (i, (_, x)) in out.iter().enumerate() {
            let want = if i < 30_000 {
                (i % 1000) as f32
            } else {
                (5000 + (i - 30_000) % 1000) as f32
            };
            assert_eq!(*x, want / 32768.0, "frame {i}");
        }
        assert!(s.queue_exhausted());
        s.finish();
        let ev = s.take_events();
        assert_eq!(ev[0], SourceEvent::Started(0));
        assert!(ev.contains(&SourceEvent::Started(1)));
        let plays: Vec<(usize, f64)> = ev
            .iter()
            .filter_map(|e| match e {
                SourceEvent::PlayEnded { index, seconds, .. } => Some((*index, *seconds)),
                _ => None,
            })
            .collect();
        assert_eq!(plays.len(), 2);
        assert!((plays[0].1 - 30_000.0 / 44_100.0).abs() < 1e-9);
        assert_eq!(ev.last(), Some(&SourceEvent::Finished));
        assert_eq!(s.transport(), Transport::Stopped);
    }

    #[test]
    fn a_rate_change_starts_a_new_segment_for_the_output_to_follow() {
        let (mut s, tracks) = source(vec![
            Some(wav(44_100, 5_000, 0)),
            Some(wav(96_000, 5_000, 0)),
        ]);
        s.set_queue(tracks, 0, false, true);
        let out = drain(&mut s, usize::MAX);
        assert_eq!(out.len(), 10_000);
        assert!(out[..5000].iter().all(|(r, _)| *r == 44_100));
        assert!(out[5000..].iter().all(|(r, _)| *r == 96_000));
    }

    #[test]
    fn missing_or_broken_files_are_skipped_and_reported() {
        let mut broken = wav(44_100, 1000, 0);
        broken[22..24].copy_from_slice(&u16::MAX.to_le_bytes());
        let (mut s, tracks) = source(vec![None, Some(broken), Some(wav(44_100, 3000, 0))]);
        s.set_queue(tracks, 0, false, true);
        let out = drain(&mut s, usize::MAX);
        assert_eq!(out.len(), 3000);
        let failed: Vec<usize> = s
            .take_events()
            .iter()
            .filter_map(|e| match e {
                SourceEvent::Failed { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(failed, vec![0, 1]);
        // Nothing playable at all: it gives up rather than spinning.
        let (mut s, tracks) = source(vec![None, None]);
        s.set_queue(tracks, 0, false, true);
        assert!(drain(&mut s, usize::MAX).is_empty());
        assert!(s.queue_exhausted());
    }

    #[test]
    fn seek_next_previous_and_jump() {
        let (mut s, tracks) = source(vec![
            Some(wav(10_000, 40_000, 0)),
            Some(wav(10_000, 40_000, 3000)),
            Some(wav(10_000, 40_000, 6000)),
        ]);
        s.set_queue(tracks, 0, false, true);
        drain(&mut s, 5_000);
        assert_eq!(s.elapsed_ms(), 500);
        let flushes = s.flush_count();
        s.seek(1_500);
        assert!(s.flush_count() > flushes);
        let out = drain(&mut s, 1);
        assert_eq!(
            out[0].1,
            ((15_000 % 1000) as f32) / 32768.0,
            "sample-accurate"
        );
        assert_eq!(s.elapsed_ms(), 1_500);
        assert!(s.next());
        let out = drain(&mut s, 1);
        assert_eq!(out[0].1, 3000.0 / 32768.0);
        assert_eq!(s.current_index(), Some(1));
        // Later in a track: back to its start (the same play).
        drain(&mut s, 35_000);
        assert!(s.elapsed_ms() > 3_000);
        s.previous();
        drain(&mut s, 1);
        assert_eq!(s.current_index(), Some(1));
        assert!(s.elapsed_ms() < 10);
        // Within 3 s of the start: the track before.
        s.previous();
        drain(&mut s, 1);
        assert_eq!(s.current_index(), Some(0));
        assert!(s.jump(2));
        drain(&mut s, 1);
        assert_eq!(s.current_index(), Some(2));
        assert!(!s.next(), "the end, without repeat");
        assert!(!s.jump(7));
        // One play per track started; seeking and restarting didn't start another.
        let started: Vec<usize> = s
            .take_events()
            .iter()
            .filter_map(|e| match e {
                SourceEvent::Started(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(started, vec![0, 1, 0, 2]);
    }

    #[test]
    fn repeat_one_loops_gaplessly_and_shuffle_keeps_the_current_track() {
        let (mut s, tracks) = source(vec![
            Some(wav(8_000, 1_000, 0)),
            Some(wav(8_000, 1_000, 2000)),
        ]);
        s.set_queue(tracks.clone(), 0, false, true);
        s.set_repeat(LocalRepeat::One);
        let out = drain(&mut s, 3_000);
        assert_eq!(out.len(), 3_000);
        assert!(
            out.iter().all(|(_, x)| *x < 1000.0 / 32768.0),
            "only the first track"
        );
        assert_eq!(out[1000].1, 0.0, "looped to its start");

        let (mut s, _) = source((0..10).map(|i| Some(wav(8_000, 500, i * 1000))).collect());
        s.set_queue(tracks_n(10), 4, false, true);
        drain(&mut s, 10);
        s.set_shuffle(true);
        assert_eq!(s.current_index(), Some(4));
        assert_eq!(s.queue().unwrap().order()[0], 4);
        // Play the rest: every other track exactly once.
        let _ = drain(&mut s, usize::MAX);
        let started: Vec<usize> = s
            .take_events()
            .iter()
            .filter_map(|e| match e {
                SourceEvent::Started(i) => Some(*i),
                _ => None,
            })
            .collect();
        let mut sorted = started.clone();
        sorted.sort();
        assert_eq!(sorted, (0..10).collect::<Vec<_>>(), "{started:?}");
    }

    fn tracks_n(n: usize) -> Vec<LocalTrack> {
        (0..n)
            .map(|i| LocalTrack {
                key: format!("{i:064x}"),
                track_id: 100 + i as i64,
                path: PathBuf::from(format!("/nowhere/{i}")),
                ext: "wav".into(),
                duration_secs: 0.0,
                album: None,
            })
            .collect()
    }

    #[test]
    fn pause_holds_the_audio_and_stop_forgets_the_queue() {
        let (mut s, tracks) = source(vec![Some(wav(8_000, 8_000, 0))]);
        s.set_queue(tracks, 0, false, false);
        s.decode_ahead();
        let mut block = vec![0f32; 256];
        assert!(s.pull_track(&mut block).is_none(), "paused: nothing pulled");
        s.play();
        assert!(s.pull_track(&mut block).is_some());
        s.played(128);
        s.pause();
        assert_eq!(s.transport(), Transport::Paused);
        assert!(s.pull_track(&mut block).is_none());
        s.stop();
        assert!(s.queue().is_none());
        assert!(s.current_index().is_none());
        let ev = s.take_events();
        assert!(matches!(
            ev.last(),
            Some(SourceEvent::PlayEnded { index: 0, .. })
        ));
        assert!(s.rate().is_none());
    }
}
