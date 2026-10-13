//! Partial flushes (`strm f`) for the web player's outputs.
//!
//! LMS sends `strm f` when the next track, already streamed, is deleted or moved in its
//! queue. The core keeps the audible track and drops what came after it (see
//! `anywhere_core::player::QueueTrim`). The worker has already pulled some of that audio
//! into its output (the AudioWorklet's queue, or segments in the media element), so it must
//! drop it there too, or the deleted track plays anyway.
//!
//! [`PullLog`] remembers which track each pulled frame came from, so the worker can tell
//! how many of the frames it pulled last belong to dropped tracks. [`TrackSegmenter`] does
//! the same for the media element's FLAC segments, cutting the timeline exactly where the
//! kept track ends.

use std::collections::VecDeque;

use anywhere_core::mux::{BLOCK, Segmenter};

/// Runs of pulled frames remembered (one per track; older runs are long played).
const MAX_RUNS: usize = 16;
/// Track boundaries remembered by the segmenter (each holds under one FLAC frame of PCM).
const MAX_BOUNDARIES: usize = 8;

/// The tracks of the audio pulled since the last full flush, in pull order.
#[derive(Debug, Default, Clone)]
pub struct PullLog {
    flushes: u64,
    trims: u64,
    /// `(track, frames)`, oldest first; consecutive pulls of one track are merged.
    runs: VecDeque<(u32, u64)>,
}

impl PullLog {
    /// Follow the core's flush count: a full flush drops everything the host held.
    pub fn sync(&mut self, flush_count: u64) {
        if flush_count != self.flushes {
            self.flushes = flush_count;
            self.runs.clear();
        }
    }

    /// `frames` of `track` were pulled.
    pub fn pulled(&mut self, track: u32, frames: u64) {
        if frames == 0 {
            return;
        }
        match self.runs.back_mut() {
            Some((t, n)) if *t == track => *n += frames,
            _ => {
                self.runs.push_back((track, frames));
                if self.runs.len() > MAX_RUNS {
                    self.runs.pop_front();
                }
            }
        }
    }

    /// Once per partial flush (when `trim_count` changes): how many of the frames pulled
    /// last belong to tracks after `keep_track`, which the host must drop from the end of
    /// what it holds. They are forgotten here too.
    pub fn take_trim(&mut self, flush_count: u64, trim_count: u64, keep_track: u32) -> Option<u64> {
        self.sync(flush_count);
        if trim_count == self.trims {
            return None;
        }
        self.trims = trim_count;
        let mut drop = 0;
        while let Some(&(track, frames)) = self.runs.back() {
            if track <= keep_track {
                break;
            }
            drop += frames;
            self.runs.pop_back();
        }
        Some(drop)
    }
}

/// Where one track ends and the next begins in the segmenter's output.
#[derive(Debug, Clone)]
struct Boundary {
    /// The track before the boundary.
    before: u32,
    /// Sample rate then.
    rate: u32,
    /// Timeline position (frames at `rate`) of the FLAC frame the boundary falls in.
    block_start: u64,
    /// That frame's PCM before the boundary (under one frame).
    head: Vec<f32>,
    /// [`TrackSegmenter::epoch`] then.
    epoch: u64,
}

/// The core's [`Segmenter`], also told which track each piece of PCM comes from, so a
/// partial flush can cut the media timeline where the kept track ends (see
/// [`TrackSegmenter::trim_after`]).
#[derive(Debug, Clone)]
pub struct TrackSegmenter {
    inner: Segmenter,
    track: Option<u32>,
    boundaries: VecDeque<Boundary>,
    /// The rate and timeline end before the last rate change, until the next push.
    rate_switch: Option<(u32, u64)>,
    /// Bumped whenever everything queued was emitted or dropped (a rate change, a cut):
    /// a boundary from an earlier epoch is no longer in the queue.
    epoch: u64,
}

/// What a trim did to the media timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimelineCut {
    /// Nothing held of a later track.
    None,
    /// The later tracks' audio was still queued here, and is gone.
    Pending,
    /// Segments from timeline second `from` on (already handed to the element) must be
    /// removed; the timeline carries on there, starting with the kept part of that frame.
    From(f64),
}

impl TrackSegmenter {
    pub fn new(rate: u32) -> Self {
        TrackSegmenter {
            inner: Segmenter::new(rate),
            track: None,
            boundaries: VecDeque::new(),
            rate_switch: None,
            epoch: 0,
        }
    }

    pub fn inner(&self) -> &Segmenter {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut Segmenter {
        &mut self.inner
    }

    /// Forget the tracks (after a flush, or a timeline that moved).
    pub fn forget_tracks(&mut self) {
        self.track = None;
        self.boundaries.clear();
        self.rate_switch = None;
    }

    /// Switch sample rate (see [`Segmenter::set_rate`]; flush first). A track that starts
    /// at the new rate begins exactly where the old rate's audio ended.
    pub fn set_rate(&mut self, rate: u32) {
        let old = self.inner.rate();
        let end = self.inner.decode_time();
        self.inner.set_rate(rate);
        if self.inner.rate() != old {
            self.rate_switch = Some((old, end));
            self.epoch += 1;
        }
    }

    /// Queue interleaved stereo PCM of `track`.
    pub fn push_track(&mut self, pcm: &[f32], track: u32) {
        if let Some(before) = self.track
            && before != track
        {
            let pending = self.inner.pending_frames();
            let b = match self.rate_switch {
                // The old rate's audio was all emitted before the switch.
                Some((rate, end)) if pending == 0 => Boundary {
                    before,
                    rate,
                    block_start: end,
                    head: Vec::new(),
                    epoch: self.epoch - 1,
                },
                _ => {
                    let whole = pending / BLOCK * BLOCK;
                    Boundary {
                        before,
                        rate: self.inner.rate(),
                        block_start: self.inner.decode_time() + whole as u64,
                        head: self.inner.pending_pcm()[whole * 2..pending * 2].to_vec(),
                        epoch: self.epoch,
                    }
                }
            };
            self.boundaries.push_back(b);
            if self.boundaries.len() > MAX_BOUNDARIES {
                self.boundaries.pop_front();
            }
        }
        self.rate_switch = None;
        self.track = Some(track);
        self.inner.push(pcm);
    }

    /// Drop everything after `keep_track`. The kept track's audio stays, and so does the
    /// timeline up to its end.
    pub fn trim_after(&mut self, keep_track: u32) -> TimelineCut {
        if self.track.is_none_or(|t| t <= keep_track) {
            return TimelineCut::None;
        }
        // Tracks only move on, so the cut is the last boundary out of the kept track (or
        // an earlier one).
        let Some(i) = self.boundaries.iter().rposition(|b| b.before <= keep_track) else {
            // Not remembered (only after many tracks in the queue at once): leave it.
            return TimelineCut::None;
        };
        let b = self.boundaries[i].clone();
        self.boundaries.truncate(i);
        self.track = Some(b.before);
        let now = self.inner.decode_time();
        if b.epoch == self.epoch && now <= b.block_start {
            // Not emitted yet: the frame starts in the queue (whole frames only since).
            let keep = (b.block_start - now) as usize + b.head.len() / 2;
            self.inner.truncate_pending(keep);
            return TimelineCut::Pending;
        }
        self.inner.set_rate(b.rate);
        self.inner.set_decode_time(b.block_start);
        self.epoch += 1;
        self.inner.push(&b.head);
        // Just before the frame's start, so float rounding never keeps it.
        TimelineCut::From((b.block_start as f64 - 0.5).max(0.0) / f64::from(b.rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_log_drops_only_the_later_tracks() {
        let mut log = PullLog::default();
        log.pulled(1, 1000);
        log.pulled(1, 500);
        log.pulled(2, 300);
        log.pulled(3, 200);
        assert_eq!(log.take_trim(0, 1, 1), Some(500));
        assert_eq!(log.take_trim(0, 1, 1), None, "once per trim");
        log.pulled(4, 100);
        assert_eq!(log.take_trim(0, 2, 1), Some(100));
        assert_eq!(log.take_trim(0, 3, 1), Some(0), "nothing later left");
    }

    #[test]
    fn pull_log_forgets_at_a_full_flush() {
        let mut log = PullLog::default();
        log.pulled(1, 1000);
        log.pulled(2, 300);
        assert_eq!(
            log.take_trim(1, 1, 1),
            Some(0),
            "the flush already dropped it"
        );
        log.sync(1);
        log.pulled(5, 10);
        assert_eq!(log.take_trim(1, 2, 4), Some(10));
    }

    #[test]
    fn pull_log_stays_bounded() {
        let mut log = PullLog::default();
        for t in 0..1000 {
            log.pulled(t, 1);
        }
        log.pulled(1000, 0);
        assert!(log.runs.len() <= MAX_RUNS);
        assert_eq!(log.take_trim(0, 1, 990), Some(9));
        assert_eq!(log.take_trim(0, 2, 0), Some(MAX_RUNS as u64 - 9));
    }

    fn pcm(frames: usize, value: f32) -> Vec<f32> {
        vec![value; frames * 2]
    }

    #[test]
    fn trim_of_audio_still_queued_keeps_the_timeline() {
        let mut s = TrackSegmenter::new(44_100);
        s.push_track(&pcm(BLOCK + 100, 0.1), 1);
        s.push_track(&pcm(500, 0.2), 2);
        assert_eq!(s.trim_after(1), TimelineCut::Pending);
        assert_eq!(s.inner().pending_frames(), BLOCK + 100);
        assert!(s.inner().pending_pcm().iter().all(|&x| x == 0.1));
        assert!(s.inner_mut().segment().is_some());
        assert_eq!(s.inner().emitted_seconds(), BLOCK as f64 / 44_100.0);
        // The same track carries on, and a later one can be trimmed again.
        s.push_track(&pcm(10, 0.1), 1);
        s.push_track(&pcm(10, 0.3), 3);
        assert_eq!(s.trim_after(1), TimelineCut::Pending);
        assert_eq!(s.inner().pending_frames(), 110);
    }

    #[test]
    fn trim_of_emitted_audio_cuts_at_the_frame_and_requeues_its_head() {
        let mut s = TrackSegmenter::new(44_100);
        s.push_track(&pcm(BLOCK * 2 + 100, 0.1), 1);
        s.push_track(&pcm(BLOCK * 3, 0.2), 2);
        while s.inner_mut().segment().is_some() {}
        assert_eq!(s.inner().decode_time(), (BLOCK * 5) as u64);
        let cut = s.trim_after(1);
        let at = (BLOCK * 2) as f64 / 44_100.0;
        match cut {
            TimelineCut::From(t) => assert!(t < at && t > at - 0.5 / 44_100.0 - 1e-9, "{t}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.inner().decode_time(), (BLOCK * 2) as u64);
        assert_eq!(s.inner().pending_frames(), 100);
        assert!(s.inner().pending_pcm().iter().all(|&x| x == 0.1));
        assert_eq!(s.trim_after(1), TimelineCut::None, "nothing later now");
    }

    #[test]
    fn trim_restores_the_kept_tracks_rate() {
        let mut s = TrackSegmenter::new(44_100);
        s.push_track(&pcm(BLOCK + 7, 0.1), 1);
        // The next track is at another rate: the worker flushes, then changes rate.
        let _ = s.inner_mut().flush();
        s.set_rate(96_000);
        s.push_track(&pcm(BLOCK, 0.2), 2);
        let _ = s.inner_mut().segment();
        let cut = s.trim_after(1);
        assert!(matches!(cut, TimelineCut::From(_)), "{cut:?}");
        assert_eq!(s.inner().rate(), 44_100);
        assert_eq!(
            s.inner().pending_frames(),
            0,
            "the boundary fell on a frame"
        );
        assert_eq!(
            s.inner().decode_time(),
            BLOCK as u64 + 16,
            "after the padded frame"
        );
    }

    #[test]
    fn trim_keeps_the_audible_track_when_it_is_the_later_one() {
        let mut s = TrackSegmenter::new(44_100);
        s.push_track(&pcm(100, 0.1), 1);
        s.push_track(&pcm(100, 0.2), 2);
        assert_eq!(s.trim_after(2), TimelineCut::None);
        assert_eq!(s.inner().pending_frames(), 200);
        s.push_track(&pcm(100, 0.3), 3);
        s.push_track(&pcm(100, 0.4), 4);
        assert_eq!(s.trim_after(2), TimelineCut::Pending);
        assert_eq!(s.inner().pending_frames(), 200);
    }

    /// Robustness: any mix of pushes, segments, rate changes and trims never panics, and a
    /// trim never leaves audio of a later track queued.
    #[test]
    fn arbitrary_sequences_never_panic() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..40 {
            let mut s = TrackSegmenter::new(44_100);
            let mut track = 1u32;
            let mut log = PullLog::default();
            let mut trims = 0;
            let mut first = None;
            for _ in 0..60 {
                match next(8) {
                    0..=2 => {
                        let frames = next(3 * BLOCK as u64) as usize;
                        s.push_track(&pcm(frames, track as f32), track);
                        first.get_or_insert(track);
                        log.pulled(track, frames as u64);
                    }
                    3 => track += 1 + next(2) as u32,
                    4 => while s.inner_mut().segment().is_some() {},
                    5 => {
                        let _ = s.inner_mut().flush();
                        s.set_rate([44_100, 48_000, 96_000][next(3) as usize]);
                    }
                    _ => {
                        // The core keeps a track that was pulled (the audible one).
                        let Some(first) = first else { continue };
                        let keep = track.saturating_sub(next(3) as u32).max(first);
                        trims += 1;
                        let _ = log.take_trim(0, trims, keep);
                        let _ = s.trim_after(keep);
                        assert!(
                            s.inner().pending_pcm().iter().all(|&x| x <= keep as f32),
                            "later audio left queued"
                        );
                        track = keep;
                    }
                }
            }
        }
    }

    #[test]
    fn forgotten_tracks_are_never_cut() {
        let mut s = TrackSegmenter::new(44_100);
        s.push_track(&pcm(100, 0.1), 1);
        s.push_track(&pcm(100, 0.2), 2);
        s.forget_tracks();
        s.inner_mut().reset();
        assert_eq!(s.trim_after(1), TimelineCut::None);
    }
}
