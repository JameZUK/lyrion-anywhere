//! The visualiser's tap (docs/plans/wave2-design.md §4.1): the decoded PCM the engine
//! already has, analysed by the core's [`Visualiser`] only while the app has asked for
//! frames, and handed over when that audio is heard.
//!
//! - **Off by default, and free when off:** the engine thread checks one flag per block; no
//!   analyser exists until the app turns it on, and it is dropped when turned off.
//! - **Never on the real-time path:** the tap runs on the engine thread, where blocks go
//!   into the ring. The output callback never sees it.
//! - **In step with what is heard:** each frame is tagged with the source position it
//!   ends at, and released once the output has played that far (the ring keeps about
//!   400 ms ahead, so frames would otherwise lead the sound).
//! - **A bounded handoff:** a single slot holds the newest frame; the engine thread only
//!   ever `try_lock`s it (a frame is skipped rather than waited for), and the app takes it
//!   (at most [`MAX_FPS`] frames a second of audio arrive).
//! - **Reset** on a seek or flush, a new track, and a new sample rate.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anywhere_core::visual::{BANDS, MAX_FPS, SCOPE_POINTS, VisualFrame, Visualiser};

use super::ring::CHANNELS;

/// Floats in a packed frame: bands, scope, RMS (L, R), peak (L, R), bass.
pub const VISUAL_FRAME_FLOATS: usize = BANDS + SCOPE_POINTS + 2 + 2 + 1;
/// Bytes in a packed frame (little-endian `f32`s).
pub const VISUAL_FRAME_BYTES: usize = VISUAL_FRAME_FLOATS * 4;

/// Frames waiting to be heard (a little over 2 s at 30 a second; the ring holds ~0.4 s).
const QUEUE_MAX: usize = 64;
/// A frame further ahead of what is heard than this means the counts are out of step:
/// release it rather than hold it for ever.
const MAX_LEAD_SECS: u64 = 4;

/// What the app-facing player shares with the engine about the visualiser.
#[derive(Debug, Default)]
pub(crate) struct VisualShared {
    enabled: AtomicBool,
    latest: Mutex<Option<VisualFrame>>,
}

impl VisualShared {
    #[cfg(test)]
    pub fn new(on: bool) -> VisualShared {
        VisualShared {
            enabled: AtomicBool::new(on),
            latest: Mutex::new(None),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Release);
        if !on && let Ok(mut l) = self.latest.lock() {
            *l = None;
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// The newest frame since the last call, packed ([`pack`]); `None` if none (or off).
    pub fn take(&self) -> Option<Vec<u8>> {
        if !self.enabled() {
            return None;
        }
        let f = self.latest.lock().ok()?.take()?;
        Some(pack(&f))
    }

    /// Offer a frame from the engine thread: never waits (a busy slot skips this one).
    fn offer(&self, f: VisualFrame) {
        if let Ok(mut l) = self.latest.try_lock() {
            *l = Some(f);
        }
    }
}

/// Pack a frame as [`VISUAL_FRAME_FLOATS`] little-endian `f32`s: [`BANDS`] bands,
/// [`SCOPE_POINTS`] scope points, RMS left and right, peak left and right (dBFS), bass.
/// Always exactly [`VISUAL_FRAME_BYTES`] long; non-finite values become 0 (levels: the
/// floor).
pub fn pack(f: &VisualFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(VISUAL_FRAME_BYTES);
    let mut put = |x: f32, lo: f32, hi: f32, fallback: f32| {
        let v = if x.is_finite() {
            x.clamp(lo, hi)
        } else {
            fallback
        };
        out.extend_from_slice(&v.to_le_bytes());
    };
    for i in 0..BANDS {
        put(f.bands.get(i).copied().unwrap_or(0.0), 0.0, 1.0, 0.0);
    }
    for i in 0..SCOPE_POINTS {
        put(f.scope.get(i).copied().unwrap_or(0.0), -1.0, 1.0, 0.0);
    }
    let floor = anywhere_core::visual::FLOOR_DB;
    for x in f.rms_db.iter().chain(f.peak_db.iter()) {
        put(*x, floor, 0.0, floor);
    }
    put(f.bass, 0.0, 1.0, 0.0);
    out
}

/// The engine thread's side: analyses what goes into the ring and releases each frame
/// when its audio is heard.
#[derive(Default)]
pub(crate) struct VisualTap {
    vis: Option<Visualiser>,
    rate: u32,
    seq: Option<u32>,
    /// Frames and the source position (in frames since the tap began) each ends at.
    queue: VecDeque<(u64, VisualFrame)>,
    /// Source frames handed to the pipeline, and heard, since the tap began (counted even
    /// while off, so turning it on mid-track is in step at once).
    produced: u64,
    heard: u64,
}

impl VisualTap {
    /// A block of interleaved stereo at `rate` (track `seq`) went into the pipeline,
    /// standing for `src_frames` source frames.
    pub fn pushed(
        &mut self,
        shared: &VisualShared,
        block: &[f32],
        seq: u32,
        rate: u32,
        src_frames: u64,
    ) {
        let start = self.produced;
        self.produced = self.produced.saturating_add(src_frames);
        if !shared.enabled() {
            if self.vis.is_some() {
                self.off();
            }
            return;
        }
        let frames = block.len() / CHANNELS;
        if frames == 0 || rate == 0 {
            return;
        }
        match &mut self.vis {
            Some(v) if self.rate == rate => {
                if self.seq != Some(seq) {
                    v.reset();
                }
            }
            _ => {
                self.vis = Some(Visualiser::new(rate, CHANNELS as u16));
                self.rate = rate;
                self.queue.clear();
            }
        }
        self.seq = Some(seq);
        let Some(v) = self.vis.as_mut() else { return };
        // Push a frame's worth of audio at a time, so every frame is caught (the analyser
        // keeps only its newest) and tagged with where it ends.
        let hop = (rate / MAX_FPS).max(1) as usize;
        let mut done = 0;
        while done < frames {
            let n = hop.min(frames - done);
            v.push(&block[done * CHANNELS..(done + n) * CHANNELS]);
            done += n;
            if let Some(f) = v.take() {
                let at = start + src_frames.saturating_mul(done as u64) / frames as u64;
                if self.queue.len() >= QUEUE_MAX {
                    self.queue.pop_front();
                }
                self.queue.push_back((at, f));
            }
        }
    }

    /// `frames` more source frames have been heard: hand over the newest frame now due.
    pub fn heard(&mut self, shared: &VisualShared, frames: u64) {
        self.heard = self.heard.saturating_add(frames);
        if self.vis.is_none() {
            return;
        }
        let lead = u64::from(self.rate.max(1)) * MAX_LEAD_SECS;
        if let Some((at, _)) = self.queue.front()
            && *at > self.heard.saturating_add(lead)
        {
            self.heard = *at;
        }
        let mut due = None;
        while self.queue.front().is_some_and(|(at, _)| *at <= self.heard) {
            due = self.queue.pop_front().map(|(_, f)| f);
        }
        if let Some(f) = due {
            shared.offer(f);
        }
    }

    /// Queued audio was dropped (a flush, a skip, a seek): forget it and start afresh.
    pub fn flush(&mut self) {
        self.produced = self.heard;
        self.queue.clear();
        if let Some(v) = &mut self.vis {
            v.reset();
        }
    }

    fn off(&mut self) {
        self.vis = None;
        self.seq = None;
        self.queue.clear();
    }

    #[cfg(test)]
    fn queued(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f32, hz: f32, amp: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .flat_map(|i| {
                let x = amp * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin();
                [x, x]
            })
            .collect()
    }

    fn unpack(b: &[u8]) -> Vec<f32> {
        b.as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect()
    }

    #[test]
    fn off_by_default_costs_nothing_and_gives_nothing() {
        let shared = VisualShared::default();
        let mut tap = VisualTap::default();
        let pcm = sine(44_100, 1.0, 440.0, 0.5);
        tap.pushed(&shared, &pcm, 0, 44_100, 44_100);
        tap.heard(&shared, 44_100);
        assert!(tap.vis.is_none(), "no analyser while off");
        assert_eq!(tap.queued(), 0);
        assert_eq!(shared.take(), None);
    }

    #[test]
    fn frames_wait_until_their_audio_is_heard() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        let pcm = sine(48_000, 0.5, 1000.0, 0.5);
        tap.pushed(&shared, &pcm, 0, 48_000, 24_000);
        assert!(tap.queued() >= 14, "a frame every 1/30 s: {}", tap.queued());
        // Nothing heard yet: nothing handed over.
        tap.heard(&shared, 0);
        assert_eq!(shared.take(), None);
        // A tenth of a second heard: the frames up to then are due; only the newest is kept.
        tap.heard(&shared, 4_800);
        let f = shared.take().expect("a frame");
        assert_eq!(f.len(), VISUAL_FRAME_BYTES);
        assert_eq!(shared.take(), None, "taken once");
        let left = tap.queued();
        tap.heard(&shared, 20_000);
        assert!(tap.queued() < left);
        assert!(shared.take().is_some());
    }

    #[test]
    fn at_most_thirty_frames_a_second_of_audio() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        let pcm = sine(44_100, 2.0, 220.0, 0.3);
        // In blocks of the engine's size.
        let mut count = 0;
        for (i, block) in pcm.chunks(4096 * 2).enumerate() {
            let frames = (block.len() / 2) as u64;
            tap.pushed(&shared, block, 0, 44_100, frames);
            let _ = i;
            count += tap.queued();
            tap.queue.clear();
        }
        assert!((55..=61).contains(&count), "{count} frames in 2 s");
    }

    #[test]
    fn turning_off_drops_the_analyser_and_the_waiting_frame() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        let pcm = sine(44_100, 0.2, 440.0, 0.5);
        tap.pushed(&shared, &pcm, 0, 44_100, 8_820);
        tap.heard(&shared, 8_820);
        shared.set_enabled(false);
        assert_eq!(shared.take(), None);
        tap.pushed(&shared, &pcm, 0, 44_100, 8_820);
        assert!(tap.vis.is_none());
        assert_eq!(tap.queued(), 0);
    }

    #[test]
    fn turned_on_mid_track_it_is_in_step_with_what_is_heard() {
        let shared = VisualShared::default();
        let mut tap = VisualTap::default();
        let pcm = sine(44_100, 0.4, 440.0, 0.5);
        // 0.4 s queued ahead while off.
        tap.pushed(&shared, &pcm, 0, 44_100, 17_640);
        shared.set_enabled(true);
        tap.pushed(&shared, &pcm, 0, 44_100, 17_640);
        // The first 0.4 s is heard: none of the new frames are due yet.
        tap.heard(&shared, 17_640);
        assert_eq!(shared.take(), None);
        tap.heard(&shared, 4_410);
        assert!(shared.take().is_some());
    }

    #[test]
    fn a_flush_forgets_queued_frames_and_stays_in_step() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        let pcm = sine(44_100, 0.4, 440.0, 0.5);
        tap.pushed(&shared, &pcm, 0, 44_100, 17_640);
        tap.heard(&shared, 1_000);
        tap.flush();
        assert_eq!(tap.queued(), 0);
        let _ = shared.take();
        // After the seek, new audio is heard straight away.
        tap.pushed(&shared, &pcm, 0, 44_100, 17_640);
        tap.heard(&shared, 2_000);
        assert!(shared.take().is_some());
    }

    #[test]
    fn a_new_rate_makes_a_new_analyser_and_a_new_track_resets_it() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        tap.pushed(&shared, &sine(44_100, 0.1, 440.0, 0.5), 0, 44_100, 4_410);
        assert_eq!(tap.vis.as_ref().unwrap().sample_rate(), 44_100);
        tap.pushed(&shared, &sine(96_000, 0.1, 440.0, 0.5), 1, 96_000, 9_600);
        assert_eq!(tap.vis.as_ref().unwrap().sample_rate(), 96_000);
        assert_eq!(tap.seq, Some(1));
        tap.pushed(&shared, &sine(96_000, 0.1, 440.0, 0.5), 2, 96_000, 9_600);
        assert_eq!(tap.seq, Some(2));
    }

    #[test]
    fn counts_out_of_step_never_hold_frames_for_ever() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        // Claims 100 seconds of source for a short block: frames far ahead of what is heard.
        tap.pushed(
            &shared,
            &sine(44_100, 0.2, 440.0, 0.5),
            0,
            44_100,
            4_410_000,
        );
        tap.heard(&shared, 1);
        assert!(shared.take().is_some());
    }

    #[test]
    fn packing_is_fixed_size_and_sanitised() {
        let f = VisualFrame {
            bands: vec![f32::NAN, 2.0, -1.0],
            scope: vec![f32::INFINITY; 500],
            rms_db: [f32::NEG_INFINITY, -3.0],
            peak_db: [5.0, f32::NAN],
            bass: 7.0,
        };
        let b = pack(&f);
        assert_eq!(b.len(), VISUAL_FRAME_BYTES);
        let v = unpack(&b);
        assert!(v.iter().all(|x| x.is_finite()));
        assert_eq!(&v[..3], &[0.0, 1.0, 0.0]);
        assert_eq!(v[BANDS], 0.0, "a non-finite scope point is 0");
        let lv = BANDS + SCOPE_POINTS;
        assert_eq!(v[lv], anywhere_core::visual::FLOOR_DB);
        assert_eq!(v[lv + 1], -3.0);
        assert_eq!(v[lv + 2], 0.0);
        assert_eq!(v[lv + 3], anywhere_core::visual::FLOOR_DB);
        assert_eq!(v[lv + 4], 1.0);
    }

    #[test]
    fn hostile_blocks_never_panic() {
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        let junk = [
            f32::NAN,
            f32::INFINITY,
            -f32::INFINITY,
            1e30,
            -1e30,
            0.0,
            1.0,
        ];
        let mut seed = 0x1234_5678u64;
        for round in 0..200u32 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed % 9000) as usize;
            let block: Vec<f32> = (0..len)
                .map(|i| junk[(seed as usize + i) % junk.len()])
                .collect();
            let rate = [0u32, 1, 8_000, 44_100, 384_000, u32::MAX][(seed % 6) as usize];
            tap.pushed(&shared, &block, round % 3, rate, seed % 100_000);
            tap.heard(&shared, seed % 50_000);
            if let Some(b) = shared.take() {
                assert_eq!(b.len(), VISUAL_FRAME_BYTES);
                assert!(unpack(&b).iter().all(|x| x.is_finite()));
            }
            if round % 50 == 0 {
                tap.flush();
            }
        }
    }

    #[test]
    fn the_tap_stays_cheap() {
        // Ten seconds of 96 kHz stereo analysed well within real time (a debug build).
        let shared = VisualShared::new(true);
        let mut tap = VisualTap::default();
        let pcm = sine(96_000, 1.0, 440.0, 0.5);
        let t = std::time::Instant::now();
        for _ in 0..10 {
            for block in pcm.chunks(4096 * 2) {
                tap.pushed(&shared, block, 0, 96_000, (block.len() / 2) as u64);
                tap.heard(&shared, (block.len() / 2) as u64);
            }
        }
        assert!(t.elapsed().as_secs_f64() < 5.0, "{:?}", t.elapsed());
    }
}
