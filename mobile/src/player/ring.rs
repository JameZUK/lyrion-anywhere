//! A lock-free single-producer, single-consumer ring of interleaved stereo `f32` samples:
//! the hand-over between the engine (producer) and the audio output's real-time callback
//! (consumer).
//!
//! - **Real-time safe:** reading never allocates, locks or blocks. Samples are stored as
//!   `AtomicU32` bit patterns, so the whole ring is safe Rust (no `unsafe`); on the targets
//!   we care about a relaxed atomic load or store is an ordinary memory access.
//! - **Bit-exact:** a sample comes out with exactly the bits it went in with.
//! - **Positions** are absolute sample counts that only grow (`u64`, which can't wrap in
//!   practice), so "how far has the output got" is a single number the engine can compare
//!   with what it wrote.
//! - **Discard:** the producer can drop everything written so far (a flush) without touching
//!   the consumer's index: it publishes a position, and the consumer jumps to it on its next
//!   read.
//! - **Discard from a point:** the producer can also drop what it wrote from a given
//!   position on, keeping what comes before (a partial flush: the next track was dropped).
//!   The consumer plays up to that position, then jumps past the dropped audio.
//! - **Hold:** while held, the consumer outputs silence and consumes nothing (pause).
//! - **Gain:** an optional factor (ducking) applied on the way out; exactly 1.0 is a true
//!   bypass, so the samples stay bit-identical.
//! - **Device clock:** the consumer counts every frame it hands the output (audio or
//!   silence). It records, for each read, where the audio landed on that count and which ring
//!   position it came from. Given the device frame being heard now (from the output's
//!   timestamp), [`Ring::presented_pos`] then says how far into the ring is audible. That is
//!   what playback position reports, rather than what has merely been handed over.
//! - **Sync corrections** are applied by the consumer, at the output, to the sample:
//!   - a timed start ([`Ring::start_at`]): silence while held until a given device frame,
//!     then audio from exactly that frame;
//!   - a pause ([`Ring::insert_silence`]): N frames of silence, consuming nothing;
//!   - a skip ([`Ring::skip`]): N frames of queued audio dropped.
//!
//! One producer and one consumer at a time are enforced at run time: each handle can be
//! taken once and is given back when dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anywhere_core::dsp::pcm;

/// Channels per frame (the core's output is always stereo).
pub const CHANNELS: usize = 2;

/// Reads remembered for mapping device frames back to ring positions. At 10 ms a read
/// this covers five seconds, far more than any output's latency.
const HISTORY: usize = 512;
/// No timed start armed.
const NO_START: u64 = u64::MAX;
/// No partial discard pending.
const NO_CUT: u64 = u64::MAX;
/// A late timed start drops at most this many frames to catch up (about a second).
const MAX_CATCH_UP_FRAMES: u64 = 48_000;

/// One read by the consumer: `frames` frames of audio from ring position `pos` (samples),
/// placed from device frame `dev`. (Silence before or after the audio is not recorded.)
struct Read {
    dev: AtomicU64,
    pos: AtomicU64,
    frames: AtomicU64,
}

pub struct Ring {
    buf: Box<[AtomicU32]>,
    mask: u64,
    /// Samples written (producer-owned).
    write: AtomicU64,
    /// Samples read or discarded (consumer-owned).
    read: AtomicU64,
    /// The consumer skips forward to here (producer-owned).
    discard_to: AtomicU64,
    /// A partial discard: the consumer plays up to `cut_from`, then jumps to `cut_to`
    /// (both producer-set; [`NO_CUT`] when none is pending; the consumer clears it).
    cut_from: AtomicU64,
    cut_to: AtomicU64,
    hold: AtomicBool,
    /// `f32` bits of the output gain.
    gain: AtomicU32,
    /// The engine expects audio to be flowing: an empty ring now is an underrun.
    expect_data: AtomicBool,
    underruns: AtomicU64,
    /// Frames of silence played in underruns.
    underrun_frames: AtomicU64,
    producer_taken: AtomicBool,
    consumer_taken: AtomicBool,
    /// Frames handed to the output device (audio and silence) since the device clock was
    /// last reset (consumer-owned).
    dev_frames: AtomicU64,
    /// Held audio starts at this device frame ([`NO_START`]: no timed start).
    start_at: AtomicU64,
    /// Frames of silence still to insert (a pause correction).
    silence: AtomicU64,
    /// Samples of audio still to drop (a skip correction).
    skip: AtomicU64,
    /// Frames dropped because a timed start came too late to make (diagnostics).
    late_frames: AtomicU64,
    /// The consumer's recent reads, and how many have been recorded (seqlock-style: an
    /// entry is valid while the count has not moved [`HISTORY`] past it).
    history: Box<[Read]>,
    reads: AtomicU64,
}

impl Ring {
    /// A ring holding at least `min_samples` samples (rounded up to a power of two, and
    /// to whole frames).
    pub fn new(min_samples: usize) -> Arc<Ring> {
        let cap = min_samples.max(CHANNELS * 64).next_power_of_two();
        Arc::new(Ring {
            buf: (0..cap).map(|_| AtomicU32::new(0)).collect(),
            mask: cap as u64 - 1,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            discard_to: AtomicU64::new(0),
            cut_from: AtomicU64::new(NO_CUT),
            cut_to: AtomicU64::new(0),
            hold: AtomicBool::new(true),
            gain: AtomicU32::new(1.0f32.to_bits()),
            expect_data: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            producer_taken: AtomicBool::new(false),
            consumer_taken: AtomicBool::new(false),
            dev_frames: AtomicU64::new(0),
            start_at: AtomicU64::new(NO_START),
            silence: AtomicU64::new(0),
            skip: AtomicU64::new(0),
            late_frames: AtomicU64::new(0),
            history: (0..HISTORY)
                .map(|_| Read {
                    dev: AtomicU64::new(0),
                    pos: AtomicU64::new(0),
                    frames: AtomicU64::new(0),
                })
                .collect(),
            reads: AtomicU64::new(0),
        })
    }

    /// Frames handed to the output device since its clock was reset.
    pub fn device_frames(&self) -> u64 {
        self.dev_frames.load(Ordering::Acquire)
    }

    /// The ring position (in samples) audible when the output presents device frame `dev`:
    /// everything before it has been heard. Device frames beyond what was handed over count
    /// as the last frame handed over. Positions in a gap (silence) are the end of the audio
    /// before it. A frame older than the history remembers counts as the oldest remembered.
    pub fn presented_pos(&self, dev: u64) -> u64 {
        let count = self.reads.load(Ordering::Acquire);
        let oldest = count.saturating_sub(HISTORY as u64 - 1);
        let mut best = None;
        let mut i = count;
        while i > oldest {
            i -= 1;
            let e = &self.history[(i % HISTORY as u64) as usize];
            let (d, p, n) = (
                e.dev.load(Ordering::Acquire),
                e.pos.load(Ordering::Acquire),
                e.frames.load(Ordering::Acquire),
            );
            // Overwritten while we read it: everything older is gone too.
            if self.reads.load(Ordering::Acquire) >= i + HISTORY as u64 {
                break;
            }
            best = Some(p + (dev.saturating_sub(d)).min(n) * CHANNELS as u64);
            if d <= dev {
                return best.unwrap_or(p);
            }
        }
        best.unwrap_or_else(|| self.read_pos())
    }

    /// Restart the device clock at 0 (a new output stream, whose frame count starts at 0).
    /// Only while no output is reading (between taking the consumer and starting the stream).
    pub fn reset_device_clock(&self) {
        self.dev_frames.store(0, Ordering::Release);
        self.start_at.store(NO_START, Ordering::Release);
        let pos = self.read.load(Ordering::Acquire);
        let count = self.reads.load(Ordering::Acquire);
        let e = &self.history[(count % HISTORY as u64) as usize];
        e.dev.store(0, Ordering::Release);
        e.pos.store(pos, Ordering::Release);
        e.frames.store(0, Ordering::Release);
        self.reads.store(count + 1, Ordering::Release);
    }

    /// Hold, then start the audio exactly at device frame `frame` (a timed start). The
    /// consumer releases the hold itself at that frame. If the frame has already been handed
    /// over when it is next read, the audio that should have played since is dropped (up to
    /// about a second) and the rest starts at once, still in step.
    pub fn start_at(&self, frame: u64) {
        // The hold goes first. The consumer only looks at the start while held, and clears
        // the hold once it has acted on the start; stored the other way round, a read that
        // fell between the two stores could act on the start and then be held again with no
        // start pending (silence for good).
        self.hold.store(true, Ordering::Release);
        self.start_at
            .store(frame.min(NO_START - 1), Ordering::Release);
    }

    /// Forget a timed start that has not happened yet (the hold stays as it is).
    pub fn cancel_start(&self) {
        self.start_at.store(NO_START, Ordering::Release);
    }

    /// A timed start is armed and has not happened yet.
    pub fn start_pending(&self) -> bool {
        self.start_at.load(Ordering::Acquire) != NO_START
    }

    /// Insert `frames` frames of silence before the next audio (a pause correction).
    pub fn insert_silence(&self, frames: u64) {
        self.silence.fetch_add(frames, Ordering::AcqRel);
    }

    /// Drop the next `frames` frames of audio as they become available (a skip correction).
    pub fn skip(&self, frames: u64) {
        self.skip
            .fetch_add(frames * CHANNELS as u64, Ordering::AcqRel);
    }

    /// Corrections not yet applied: (frames of silence, frames to skip).
    pub fn pending_corrections(&self) -> (u64, u64) {
        (
            self.silence.load(Ordering::Acquire),
            self.skip.load(Ordering::Acquire) / CHANNELS as u64,
        )
    }

    /// Frames dropped so far because a timed start came too late.
    pub fn late_frames(&self) -> u64 {
        self.late_frames.load(Ordering::Acquire)
    }

    /// Capacity in samples.
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// The producer handle, unless one is already out.
    pub fn producer(self: &Arc<Self>) -> Option<Producer> {
        (!self.producer_taken.swap(true, Ordering::AcqRel)).then(|| Producer { ring: self.clone() })
    }

    /// The consumer handle, unless one is already out.
    pub fn consumer(self: &Arc<Self>) -> Option<Consumer> {
        (!self.consumer_taken.swap(true, Ordering::AcqRel)).then(|| Consumer { ring: self.clone() })
    }

    /// Samples read (or discarded) so far.
    pub fn read_pos(&self) -> u64 {
        self.read.load(Ordering::Acquire)
    }

    /// Samples written so far.
    pub fn write_pos(&self) -> u64 {
        self.write.load(Ordering::Acquire)
    }

    /// Samples waiting to be played (not counting any about to be discarded).
    pub fn len(&self) -> usize {
        let w = self.write.load(Ordering::Acquire);
        let r = self
            .read
            .load(Ordering::Acquire)
            .max(self.discard_to.load(Ordering::Acquire));
        let cut = self.cut_from.load(Ordering::Acquire);
        if cut != NO_CUT && r < cut {
            let to = self.cut_to.load(Ordering::Acquire);
            return ((cut - r) + w.saturating_sub(to)) as usize;
        }
        w.saturating_sub(r) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn set_hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::Release);
    }

    pub fn is_held(&self) -> bool {
        self.hold.load(Ordering::Acquire)
    }

    /// Output gain (ducking). Non-finite or negative values become silence; 1.0 is a true
    /// bypass.
    pub fn set_gain(&self, gain: f32) {
        let g = if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.gain.store(g.to_bits(), Ordering::Release);
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Acquire))
    }

    pub fn set_expect_data(&self, expect: bool) {
        self.expect_data.store(expect, Ordering::Release);
    }

    /// Underruns so far: (events, frames of silence).
    pub fn underruns(&self) -> (u64, u64) {
        (
            self.underruns.load(Ordering::Acquire),
            self.underrun_frames.load(Ordering::Acquire),
        )
    }
}

/// Writes samples (the engine).
pub struct Producer {
    ring: Arc<Ring>,
}

impl Producer {
    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Free space, in samples. Space freed by a discard counts once the consumer has
    /// skipped past it (on its next read), so a slot is never rewritten while it may still
    /// be read.
    pub fn free(&self) -> usize {
        let w = self.ring.write.load(Ordering::Relaxed);
        let r = self.ring.read.load(Ordering::Acquire);
        self.ring.capacity() - (w - r) as usize
    }

    /// Write whole frames from `samples`, as many as fit. Returns the samples written.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let n = samples.len().min(self.free());
        let n = n - n % CHANNELS;
        let w = self.ring.write.load(Ordering::Relaxed);
        for (i, s) in samples[..n].iter().enumerate() {
            let slot = ((w + i as u64) & self.ring.mask) as usize;
            self.ring.buf[slot].store(s.to_bits(), Ordering::Relaxed);
        }
        self.ring.write.store(w + n as u64, Ordering::Release);
        n
    }

    /// Drop everything written so far (flush). Returns the position from which new
    /// samples count.
    pub fn discard_all(&mut self) -> u64 {
        let w = self.ring.write.load(Ordering::Relaxed);
        self.ring.discard_to.store(w, Ordering::Release);
        // Corrections were for the audio just dropped.
        self.ring.skip.store(0, Ordering::Release);
        self.ring.silence.store(0, Ordering::Release);
        w
    }

    /// Drop what was written from ring position `pos` (samples) on, keeping what comes
    /// before it: the consumer plays up to `pos`, then jumps to whatever is written next.
    /// A position already played drops the rest at once. Nothing happens if `pos` is at or
    /// past what was written.
    pub fn discard_from(&mut self, pos: u64) {
        let w = self.ring.write.load(Ordering::Relaxed);
        let pos = pos - pos % CHANNELS as u64;
        if pos >= w {
            return;
        }
        self.ring.cut_to.store(w, Ordering::Release);
        self.ring.cut_from.store(pos, Ordering::Release);
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.ring.producer_taken.store(false, Ordering::Release);
    }
}

/// Reads samples (the audio output's callback). Real-time safe.
pub struct Consumer {
    ring: Arc<Ring>,
}

impl Consumer {
    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Fill `out` (interleaved stereo): with audio while available, then silence. Returns
    /// the samples of real audio written (the rest of `out` is zeroed). While held, writes
    /// silence and consumes nothing.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let len = out.len();
        let n = self.read_map(len, |i, s| out[i] = s);
        // An odd trailing sample (never a whole frame) is silence too.
        out[len - len % CHANNELS..].fill(0.0);
        n
    }

    /// [`Consumer::read`] into 16-bit integers (exact for 16-bit sources; anything else is
    /// rounded to nearest, without dither).
    pub fn read_i16(&mut self, out: &mut [i16]) -> usize {
        self.read_map(out.len(), |i, s| out[i] = pcm::f32_to_int(s, 16) as i16)
    }

    /// [`Consumer::read`] into right-aligned `bits`-deep integers (24 or 32; exact for
    /// sources of up to `min(bits, 24)` bits).
    pub fn read_i32(&mut self, out: &mut [i32], bits: u32) -> usize {
        self.read_map(out.len(), |i, s| out[i] = pcm::f32_to_int(s, bits))
    }

    /// [`Consumer::read`] into packed little-endian 24-bit samples (3 bytes each).
    pub fn read_i24_packed(&mut self, out: &mut [u8]) -> usize {
        self.read_map(out.len() / 3, |i, s| {
            let b = pcm::f32_to_int(s, 24).to_le_bytes();
            out[i * 3..i * 3 + 3].copy_from_slice(&b[..3]);
        })
    }

    /// The one read loop: `put(index, sample)` for each of up to `len` samples (whole
    /// frames), audio first and then silence (0.0), so every slot is written. Returns the
    /// samples of real audio. Real-time safe when `put` is.
    fn read_map(&mut self, len: usize, mut put: impl FnMut(usize, f32)) -> usize {
        let ring = &*self.ring;
        let len = len - len % CHANNELS;
        let frames = (len / CHANNELS) as u64;
        let dev = ring.dev_frames.load(Ordering::Relaxed);
        // Apply a discard first (even while held), so the producer gets the space back.
        let mut r = ring.read.load(Ordering::Relaxed);
        let discard = ring.discard_to.load(Ordering::Acquire);
        if discard > r {
            r = discard;
            ring.read.store(r, Ordering::Release);
        }
        let mut w = ring.write.load(Ordering::Acquire);
        // A partial discard: play up to its start, then jump past it.
        let mut cut_at = None;
        let cut = ring.cut_from.load(Ordering::Acquire);
        if cut != NO_CUT {
            let to = ring.cut_to.load(Ordering::Acquire);
            if r >= cut {
                r = r.max(to);
                ring.read.store(r, Ordering::Release);
                let _ = ring.cut_from.compare_exchange(
                    cut,
                    NO_CUT,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            } else {
                w = w.min(cut);
                cut_at = Some((cut, to));
            }
        }
        // Frames of silence before the audio in this read.
        let mut lead = 0u64;
        if ring.hold.load(Ordering::Acquire) {
            let start = ring.start_at.load(Ordering::Acquire);
            // No start, or not yet: silence. The start acted on is cleared by compare and
            // swap, so one armed or cancelled since it was read is kept for the next read
            // rather than wiped.
            if start == NO_START
                || start >= dev + frames
                || ring
                    .start_at
                    .compare_exchange(start, NO_START, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                (0..len).for_each(|i| put(i, 0.0));
                record(ring, dev, r, 0, frames);
                return 0;
            }
            if start >= dev {
                lead = start - dev;
            } else {
                // Too late for the requested frame: drop what would already have played.
                let late = (dev - start).min(MAX_CATCH_UP_FRAMES);
                let drop = (late * CHANNELS as u64).min(w.saturating_sub(r));
                r += drop;
                ring.late_frames
                    .fetch_add(drop / CHANNELS as u64, Ordering::Relaxed);
            }
            ring.hold.store(false, Ordering::Release);
        }
        // A pause correction: silence first, consuming nothing.
        let silence = ring.silence.load(Ordering::Acquire);
        if silence > 0 {
            let take = silence.min(frames - lead);
            if ring
                .silence
                .compare_exchange(silence, silence - take, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                lead += take;
            }
        }
        // A skip correction: drop queued audio (as much as is here; the rest next time).
        let skip = ring.skip.load(Ordering::Acquire);
        if skip > 0 {
            let take = skip.min(w.saturating_sub(r));
            let take = take - take % CHANNELS as u64;
            if take > 0
                && ring
                    .skip
                    .compare_exchange(skip, skip - take, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                r += take;
            }
        }
        let lead_samples = lead as usize * CHANNELS;
        let avail = w.saturating_sub(r) as usize;
        let n = avail.min(len - lead_samples);
        (0..lead_samples).for_each(|i| put(i, 0.0));
        let gain = f32::from_bits(ring.gain.load(Ordering::Acquire));
        for i in 0..n {
            let slot = ((r + i as u64) & ring.mask) as usize;
            let s = f32::from_bits(ring.buf[slot].load(Ordering::Relaxed));
            // Exactly 1.0 leaves the bits untouched (no multiplication at all).
            put(lead_samples + i, if gain == 1.0 { s } else { s * gain });
        }
        (lead_samples + n..len).for_each(|i| put(i, 0.0));
        let mut next = r + n as u64;
        if let Some((cut, to)) = cut_at
            && next >= cut
        {
            next = next.max(to);
            let _ =
                ring.cut_from
                    .compare_exchange(cut, NO_CUT, Ordering::AcqRel, Ordering::Acquire);
        }
        ring.read.store(next, Ordering::Release);
        record(ring, dev + lead, r, (n / CHANNELS) as u64, frames);
        if lead_samples + n < len && ring.expect_data.load(Ordering::Acquire) {
            ring.underruns.fetch_add(1, Ordering::Relaxed);
            ring.underrun_frames.fetch_add(
                ((len - lead_samples - n) / CHANNELS) as u64,
                Ordering::Relaxed,
            );
        }
        n
    }
}

/// Remember a read: `frames` frames of audio from ring position `pos`, placed from device
/// frame `dev`; the device clock moves on by `total` frames. Consumer only.
fn record(ring: &Ring, dev: u64, pos: u64, frames: u64, total: u64) {
    let count = ring.reads.load(Ordering::Relaxed);
    let e = &ring.history[(count % HISTORY as u64) as usize];
    e.dev.store(dev, Ordering::Release);
    e.pos.store(pos, Ordering::Release);
    e.frames.store(frames, Ordering::Release);
    ring.reads.store(count + 1, Ordering::Release);
    let d = ring.dev_frames.load(Ordering::Relaxed);
    ring.dev_frames.store(d + total, Ordering::Release);
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.ring.consumer_taken.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(n: usize) -> (Arc<Ring>, Producer, Consumer) {
        let r = Ring::new(n);
        r.set_hold(false);
        let p = r.producer().unwrap();
        let c = r.consumer().unwrap();
        (r, p, c)
    }

    #[test]
    fn a_partial_discard_plays_up_to_the_point_then_skips_the_rest() {
        let (r, mut p, mut c) = ring(1024);
        let first: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let dropped: Vec<f32> = (0..60).map(|i| 1000.0 + i as f32).collect();
        p.write(&first);
        let cut = r.write_pos();
        p.write(&dropped);
        p.discard_from(cut);
        assert_eq!(r.len(), 100, "the dropped audio no longer counts");
        let after: Vec<f32> = (0..20).map(|i| -(i as f32)).collect();
        p.write(&after);
        assert_eq!(r.len(), 120);
        let mut out = vec![9.0f32; 80];
        assert_eq!(c.read(&mut out), 80);
        assert_eq!(out, first[..80]);
        // This read ends exactly at the cut (the rest of the block is silence).
        let mut out = vec![9.0f32; 40];
        assert_eq!(c.read(&mut out), 20);
        assert_eq!(out[..20], first[80..]);
        // Then straight to what was written after the discard.
        let mut out = vec![9.0f32; 40];
        assert_eq!(c.read(&mut out), 20);
        assert_eq!(out[..20], after[..]);
        assert!(r.is_empty());
    }

    #[test]
    fn a_partial_discard_of_audio_already_playing_drops_the_rest_at_once() {
        let (r, mut p, mut c) = ring(1024);
        p.write(&[1.0; 100]);
        let mut out = vec![0.0f32; 60];
        c.read(&mut out);
        // The cut point was already passed: everything after what was read goes.
        p.discard_from(40);
        p.write(&[2.0; 10]);
        let mut out = vec![0.0f32; 20];
        assert_eq!(c.read(&mut out), 10);
        assert_eq!(out[..10], [2.0; 10]);
        // A cut beyond what was written does nothing; a full discard overrides a cut.
        p.discard_from(r.write_pos() + 10);
        p.write(&[3.0; 10]);
        let cut = r.write_pos() - 4;
        p.discard_from(cut);
        p.discard_all();
        p.write(&[4.0; 4]);
        let mut out = vec![0.0f32; 20];
        assert_eq!(c.read(&mut out), 4);
        assert_eq!(out[..4], [4.0; 4]);
    }

    #[test]
    fn handles_are_exclusive_and_returned_on_drop() {
        let r = Ring::new(256);
        let p = r.producer().unwrap();
        assert!(r.producer().is_none());
        let c = r.consumer().unwrap();
        assert!(r.consumer().is_none());
        drop(p);
        drop(c);
        assert!(r.producer().is_some());
        assert!(r.consumer().is_some());
    }

    #[test]
    fn samples_round_trip_bit_exactly_across_the_wrap() {
        let (r, mut p, mut c) = ring(256);
        assert_eq!(r.capacity(), 256);
        // Every bit pattern class, including NaN payloads, subnormals and signed zero.
        let special = [
            0.0f32,
            -0.0,
            f32::MIN_POSITIVE,
            1e-45,
            -1.0,
            1.0,
            f32::from_bits(0x7fc0_1234),
            f32::INFINITY,
        ];
        let mut sent = Vec::new();
        let mut got = Vec::new();
        let mut x = 1u32;
        for round in 0..50 {
            let chunk: Vec<f32> = (0..(round % 7 + 1) * 34)
                .map(|i| {
                    x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    if i % 9 == 0 {
                        special[(x as usize) % special.len()]
                    } else {
                        f32::from_bits(x)
                    }
                })
                .collect();
            let n = p.write(&chunk);
            sent.extend_from_slice(&chunk[..n]);
            let mut out = vec![0f32; 64];
            let m = c.read(&mut out);
            got.extend_from_slice(&out[..m]);
        }
        let mut out = vec![0f32; 512];
        loop {
            let m = c.read(&mut out);
            if m == 0 {
                break;
            }
            got.extend_from_slice(&out[..m]);
        }
        assert_eq!(sent.len(), got.len());
        for (a, b) in sent.iter().zip(&got) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        assert_eq!(r.read_pos(), r.write_pos());
    }

    #[test]
    fn never_overfills_and_writes_whole_frames() {
        let (r, mut p, _c) = ring(128);
        assert_eq!(
            p.write(&[0.5; 127]),
            126,
            "odd sample counts round down to frames"
        );
        assert_eq!(p.write(&[0.5; 10]), 2);
        assert_eq!(p.free(), 0);
        assert_eq!(p.write(&[0.5; 10]), 0);
        assert_eq!(r.len(), 128);
    }

    #[test]
    fn hold_outputs_silence_and_consumes_nothing() {
        let (r, mut p, mut c) = ring(128);
        p.write(&[0.25; 20]);
        r.set_hold(true);
        let mut out = [9.0f32; 8];
        assert_eq!(c.read(&mut out), 0);
        assert_eq!(out, [0.0; 8]);
        assert_eq!(r.len(), 20);
        // A discard while held still frees the space.
        p.write(&[0.5; 108]);
        assert_eq!(p.free(), 0);
        p.discard_all();
        c.read(&mut out);
        assert_eq!(p.free(), 128);
        p.write(&[0.25; 20]);
        r.set_hold(false);
        assert_eq!(c.read(&mut out), 8);
        assert_eq!(out, [0.25; 8]);
    }

    #[test]
    fn discard_drops_queued_audio_only() {
        let (r, mut p, mut c) = ring(128);
        p.write(&[1.0; 40]);
        let from = p.discard_all();
        assert_eq!(from, 40);
        assert!(r.is_empty());
        p.write(&[2.0; 4]);
        let mut out = [0f32; 8];
        assert_eq!(c.read(&mut out), 4);
        assert_eq!(&out[..4], &[2.0; 4]);
        assert_eq!(r.read_pos(), 44);
    }

    #[test]
    fn gain_of_one_is_bit_transparent_and_other_gains_apply() {
        let (r, mut p, mut c) = ring(128);
        let odd = f32::from_bits(0x3f80_0001);
        p.write(&[odd, odd]);
        let mut out = [0f32; 2];
        c.read(&mut out);
        assert_eq!(out[0].to_bits(), odd.to_bits());
        r.set_gain(0.5);
        p.write(&[1.0, -1.0]);
        c.read(&mut out);
        assert_eq!(out, [0.5, -0.5]);
        for bad in [f32::NAN, f32::INFINITY, -3.0] {
            r.set_gain(bad);
            assert_eq!(r.gain(), 0.0);
        }
        r.set_gain(7.0);
        assert_eq!(r.gain(), 1.0, "never amplifies");
    }

    /// Integer reads for a bit-perfect DAC: samples on the source's grid come out as the
    /// source's integers (padded when the container is deeper), silence fills the rest,
    /// and hold, gain and underrun accounting behave as for `f32`.
    #[test]
    fn integer_reads_are_exact_on_the_grid() {
        let ints16: Vec<i32> = vec![-32768, 32767, 0, 1, -1, 12345, -23456, 7];
        let ints24: Vec<i32> = vec![-8_388_608, 8_388_607, 0, 1, -1, 4_000_001, -77, 3];
        let as_f32 = |v: &[i32], bits: u32| {
            let mut f = vec![0f32; v.len()];
            pcm::int_to_f32(v, bits, &mut f);
            f
        };
        let (r, mut p, mut c) = ring(256);
        p.write(&as_f32(&ints16, 16));
        let mut o16 = [9i16; 10];
        assert_eq!(c.read_i16(&mut o16), 8);
        let want: Vec<i16> = ints16.iter().map(|v| *v as i16).collect();
        assert_eq!(&o16[..8], &want[..]);
        assert_eq!(&o16[8..], &[0, 0], "silence after the audio");

        p.write(&as_f32(&ints24, 24));
        let mut packed = [0xaau8; 8 * 3 + 2];
        assert_eq!(c.read_i24_packed(&mut packed), 8);
        for (k, v) in ints24.iter().enumerate() {
            let b = &packed[k * 3..k * 3 + 3];
            let back =
                i32::from_le_bytes([b[0], b[1], b[2], if b[2] & 0x80 != 0 { 0xff } else { 0 }]);
            assert_eq!(back, *v);
        }
        assert_eq!(
            &packed[24..],
            &[0xaa, 0xaa],
            "a partial sample is left alone"
        );

        p.write(&as_f32(&ints24, 24));
        let mut o32 = [0i32; 8];
        assert_eq!(c.read_i32(&mut o32, 32), 8);
        let want: Vec<i32> = ints24.iter().map(|v| v << 8).collect();
        assert_eq!(&o32[..], &want[..]);

        // 16-bit audio into a 24-bit container: padded, exact.
        p.write(&as_f32(&ints16, 16));
        let mut o24 = [0i32; 8];
        assert_eq!(c.read_i32(&mut o24, 24), 8);
        let want: Vec<i32> = ints16.iter().map(|v| v << 8).collect();
        assert_eq!(&o24[..], &want[..]);

        // Held: silence, nothing consumed.
        p.write(&as_f32(&ints16, 16));
        r.set_hold(true);
        let mut held = [5i32; 4];
        assert_eq!(c.read_i32(&mut held, 24), 0);
        assert_eq!(held, [0; 4]);
        r.set_hold(false);
        // Ducked: scaled and rounded.
        r.set_gain(0.5);
        let mut ducked = [0i16; 8];
        c.read_i16(&mut ducked);
        assert_eq!(ducked[0], -16384);
        assert_eq!(ducked[1], 16384, "32767 × 0.5 rounds to nearest");
        // Underruns count for integer reads too.
        r.set_expect_data(true);
        let mut empty = [0i16; 4];
        assert_eq!(c.read_i16(&mut empty), 0);
        assert_eq!(r.underruns().0, 1);
    }

    #[test]
    fn underruns_count_only_when_audio_is_expected() {
        let (r, mut p, mut c) = ring(128);
        let mut out = [0f32; 16];
        c.read(&mut out);
        assert_eq!(r.underruns(), (0, 0), "idle is not an underrun");
        r.set_expect_data(true);
        p.write(&[0.1; 6]);
        assert_eq!(c.read(&mut out), 6);
        assert_eq!(r.underruns(), (1, 5));
    }

    /// Random operation sequences against a simple model (a queue), property-test style.
    #[test]
    fn matches_a_queue_model_under_random_operations() {
        use std::collections::VecDeque;
        for seed in 1..=300u64 {
            let mut s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let mut rnd = move |n: u64| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                s % n
            };
            let (r, mut p, mut c) = ring(64 << rnd(3));
            let mut model: VecDeque<u32> = VecDeque::new();
            let mut next = 0u32;
            for _ in 0..400 {
                match rnd(6) {
                    0 | 1 => {
                        let n = rnd(80) as usize;
                        let chunk: Vec<f32> =
                            (0..n).map(|k| f32::from_bits(next + k as u32)).collect();
                        let w = p.write(&chunk);
                        assert!(w <= n && w % 2 == 0);
                        assert!(model.len() + w <= r.capacity());
                        model.extend((0..w).map(|k| next + k as u32));
                        next += n as u32;
                    }
                    2 | 3 => {
                        let mut out = vec![1.0f32; rnd(70) as usize];
                        let got = c.read(&mut out);
                        let want = model.len().min(out.len() - out.len() % 2);
                        assert_eq!(got, want);
                        for o in &out[..got] {
                            assert_eq!(o.to_bits(), model.pop_front().unwrap());
                        }
                        assert!(out[got..].iter().all(|x| *x == 0.0));
                    }
                    4 => {
                        p.discard_all();
                        model.clear();
                    }
                    _ => {
                        let held = rnd(2) == 0;
                        r.set_hold(held);
                        if held {
                            let mut out = vec![1.0f32; 8];
                            assert_eq!(c.read(&mut out), 0);
                        }
                        r.set_hold(false);
                    }
                }
                assert_eq!(r.len(), model.len());
            }
        }
    }

    /// Frames 0, 1, 2… as samples (left = right = the frame number), for following them.
    fn frames(from: u32, n: u32) -> Vec<f32> {
        (from..from + n)
            .flat_map(|f| [f as f32, f as f32])
            .collect()
    }

    #[test]
    fn device_frames_map_back_to_ring_positions() {
        let (r, mut p, mut c) = ring(4096);
        r.reset_device_clock();
        p.write(&frames(0, 1000));
        let mut out = vec![0f32; 200]; // 100 frames a read
        for _ in 0..4 {
            c.read(&mut out);
        }
        assert_eq!(r.device_frames(), 400);
        assert_eq!(r.presented_pos(0), 0);
        assert_eq!(r.presented_pos(150), 300, "frame 150 heard: 150 frames in");
        assert_eq!(r.presented_pos(400), 800);
        assert_eq!(
            r.presented_pos(10_000),
            800,
            "never beyond what was handed over"
        );
        // An underrun: 50 frames of audio, then silence. Heard positions stop at the audio.
        p.discard_all();
        c.read(&mut out);
        p.write(&frames(0, 50));
        c.read(&mut out);
        assert_eq!(r.presented_pos(560), 2000 + 100);
        assert_eq!(
            r.presented_pos(590),
            2000 + 100,
            "in the silence after the audio"
        );
    }

    /// A timed start: silence until the device frame, then audio from exactly that frame,
    /// even in the middle of a read.
    #[test]
    fn timed_start_begins_at_the_exact_device_frame() {
        let (r, mut p, mut c) = ring(4096);
        r.reset_device_clock();
        p.write(&frames(1, 500));
        r.start_at(250);
        assert!(r.start_pending());
        let mut out = vec![9f32; 200];
        assert_eq!(c.read(&mut out), 0);
        assert_eq!(c.read(&mut out), 0);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(
            c.read(&mut out),
            100,
            "50 frames of silence, then 50 of audio"
        );
        assert!(out[..100].iter().all(|s| *s == 0.0));
        assert_eq!(out[100], 1.0, "the first frame lands on device frame 250");
        assert!(!r.is_held() && !r.start_pending());
        assert_eq!(r.presented_pos(250), 0);
        assert_eq!(r.presented_pos(260), 20);
        assert_eq!(r.late_frames(), 0);
    }

    /// Too late for the requested frame: the audio that should already have played is
    /// dropped, so what is heard is still in step.
    #[test]
    fn a_late_timed_start_catches_up_by_dropping_audio() {
        let (r, mut p, mut c) = ring(4096);
        r.reset_device_clock();
        let mut out = vec![0f32; 200];
        c.read(&mut out);
        c.read(&mut out); // device frame 200
        p.write(&frames(0, 1000));
        r.start_at(120);
        assert_eq!(c.read(&mut out), 200);
        assert_eq!(out[0], 80.0, "frame 80 plays at device frame 200");
        assert_eq!(r.late_frames(), 80);
        assert_eq!(r.presented_pos(200), 160);
    }

    #[test]
    fn inserted_silence_delays_the_audio_without_consuming_it() {
        let (r, mut p, mut c) = ring(4096);
        r.reset_device_clock();
        p.write(&frames(0, 1000));
        let mut out = vec![0f32; 200];
        c.read(&mut out);
        r.insert_silence(130);
        assert_eq!(r.pending_corrections(), (130, 0));
        assert_eq!(c.read(&mut out), 0);
        assert_eq!(c.read(&mut out), 140, "30 frames of silence, then audio");
        assert_eq!(out[60], 100.0, "carries on where it stopped");
        assert_eq!(r.pending_corrections(), (0, 0));
        assert_eq!(r.presented_pos(220), 200, "the silence holds the position");
        assert_eq!(r.presented_pos(240), 220);
        assert_eq!(r.underruns().0, 0);
    }

    #[test]
    fn skips_drop_queued_audio_and_carry_over_until_it_arrives() {
        let (r, mut p, mut c) = ring(4096);
        r.reset_device_clock();
        p.write(&frames(0, 100));
        r.skip(150);
        let mut out = vec![0f32; 200];
        assert_eq!(c.read(&mut out), 0, "all 100 queued frames skipped");
        assert_eq!(r.pending_corrections(), (0, 50));
        p.write(&frames(100, 200));
        c.read(&mut out);
        assert_eq!(out[0], 150.0, "the rest of the skip, then audio");
        // Skipped audio counts as heard once the next audio is.
        assert_eq!(r.presented_pos(100), 300);
        // A flush forgets corrections meant for the audio it drops.
        r.skip(40);
        r.insert_silence(40);
        p.discard_all();
        assert_eq!(r.pending_corrections(), (0, 0));
        r.start_at(1_000_000);
        r.cancel_start();
        assert!(!r.start_pending());
    }

    #[test]
    fn device_frames_older_than_the_history_map_to_the_oldest_remembered() {
        let (r, mut p, mut c) = ring(1 << 16);
        r.reset_device_clock();
        let mut out = vec![0f32; 2];
        for i in 0..(HISTORY as u32 * 3) {
            p.write(&frames(i, 1));
            c.read(&mut out);
        }
        let oldest = r.presented_pos(0);
        assert!(oldest > 0 && oldest < r.read_pos());
        assert_eq!(r.presented_pos(r.device_frames()), r.read_pos());
    }

    /// Positions read while the consumer runs on another thread are always ones it really
    /// passed through: never past what was read, and never going back for later frames.
    #[test]
    fn presented_positions_are_consistent_while_the_output_runs() {
        let r = Ring::new(8192);
        r.set_hold(false);
        r.reset_device_clock();
        let mut p = r.producer().unwrap();
        let mut c = r.consumer().unwrap();
        let reader = r.clone();
        let out_thread = std::thread::spawn(move || {
            let mut out = vec![0f32; 96];
            for _ in 0..200_000 {
                c.read(&mut out);
            }
        });
        let mut x = 7u32;
        while !out_thread.is_finished() {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            p.write(&vec![0.5; (x % 300) as usize * 2]);
            let dev = reader.device_frames();
            let a = reader.presented_pos(dev.saturating_sub(200));
            let b = reader.presented_pos(dev);
            assert!(a <= b, "{a} > {b}");
            assert!(b <= reader.read_pos());
        }
        out_thread.join().unwrap();
    }

    /// Frame `i` of a test stream, as bits no silence (0.0) can be taken for; `tag` tells
    /// two streams apart.
    fn tagged(tag: u32, from: u32, n: u32) -> Vec<f32> {
        (from..from + n)
            .flat_map(|f| {
                let s = f32::from_bits(tag | (f + 1));
                [s, s]
            })
            .collect()
    }

    /// (tag, frame) of a sample made by [`tagged`].
    fn untag(s: f32) -> (u32, u32) {
        let b = s.to_bits();
        (b & 0xff00_0000, (b & 0x00ff_ffff) - 1)
    }

    /// Spin until `done`, or until the output thread has stopped early (a panic there then
    /// surfaces at `join` instead of leaving the test waiting for ever).
    fn wait_until<T>(out: &std::thread::JoinHandle<T>, mut done: impl FnMut() -> bool) {
        while !done() && !out.is_finished() {
            std::thread::yield_now();
        }
    }

    /// A consumer on its own thread, reading `block` frames at a time until `stop`: every
    /// audio frame it played, with the device frame it played at.
    fn run_output(
        mut c: Consumer,
        block: usize,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<Vec<(u64, f32)>> {
        std::thread::spawn(move || {
            let mut out = vec![0f32; block * CHANNELS];
            let mut heard = Vec::new();
            let mut dev = 0u64;
            while !stop.load(Ordering::Acquire) {
                c.read(&mut out);
                for (i, f) in out.chunks(CHANNELS).enumerate() {
                    assert_eq!(f[0].to_bits(), f[1].to_bits(), "frames stay whole");
                    if f[0].to_bits() != 0 {
                        heard.push((dev + i as u64, f[0]));
                    }
                }
                dev += block as u64;
                std::thread::yield_now();
            }
            heard
        })
    }

    /// A partial discard while the output runs: what was before the cut plays, in order and
    /// whole (all of it when the output hadn't reached the cut), and nothing dropped plays
    /// after the new audio has started.
    #[test]
    fn a_partial_discard_while_the_output_runs_never_plays_dropped_audio() {
        const OLD: u32 = 0x0100_0000;
        const NEW: u32 = 0x0200_0000;
        let mut x = 11u64;
        for round in 0..200 {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let r = Ring::new(4096);
            r.set_hold(false);
            let mut p = r.producer().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            p.write(&tagged(OLD, 0, 1000));
            let out = run_output(r.consumer().unwrap(), 32, stop.clone());
            for _ in 0..(x >> 33) % 200 {
                std::thread::yield_now();
            }
            let cut = ((x >> 20) % 1000) * 2;
            p.discard_from(cut);
            let read_after = r.read_pos();
            p.write(&tagged(NEW, 0, 200));
            wait_until(&out, || r.is_empty());
            stop.store(true, Ordering::Release);
            let heard = out.join().unwrap();
            let old: Vec<u32> = heard
                .iter()
                .map(|(_, s)| untag(*s))
                .take_while(|(t, _)| *t == OLD)
                .map(|(_, f)| f)
                .collect();
            let new: Vec<(u32, u32)> = heard[old.len()..].iter().map(|(_, s)| untag(*s)).collect();
            assert!(
                old.iter().enumerate().all(|(i, f)| *f == i as u32),
                "round {round}: the audio before the cut plays in order"
            );
            assert_eq!(
                new,
                (0..200).map(|f| (NEW, f)).collect::<Vec<_>>(),
                "round {round}: then the new audio, all of it, and nothing dropped"
            );
            let played = old.len() as u64 * CHANNELS as u64;
            if read_after + 32 * CHANNELS as u64 <= cut {
                assert_eq!(played, cut, "round {round}: up to the cut exactly");
            } else {
                assert!(
                    played <= cut.max(read_after + 32 * CHANNELS as u64),
                    "round {round}: {played} played past a cut at {cut}"
                );
            }
        }
    }

    /// A skip while the output runs and the producer writes: exactly the frames asked for
    /// are dropped (in pieces if they hadn't all arrived), and nothing else is lost or
    /// repeated.
    #[test]
    fn a_skip_while_the_output_runs_drops_exactly_what_was_asked() {
        let mut x = 5u64;
        for round in 0..100 {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let r = Ring::new(2048);
            r.set_hold(false);
            let mut p = r.producer().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let out = run_output(r.consumer().unwrap(), 48, stop.clone());
            let skip = 1 + (x >> 40) % 700;
            let at = (x >> 20) % 3000;
            let total = 4000u32;
            let mut next = 0u32;
            let mut asked = false;
            while next < total {
                if !asked && u64::from(next) >= at {
                    r.skip(skip);
                    asked = true;
                }
                let chunk = tagged(0x0100_0000, next, 37.min(total - next));
                let mut off = 0;
                while off < chunk.len() {
                    off += p.write(&chunk[off..]);
                    std::thread::yield_now();
                }
                next += 37.min(total - next);
            }
            wait_until(&out, || r.is_empty() && r.pending_corrections().1 == 0);
            stop.store(true, Ordering::Release);
            let heard: Vec<u32> = out
                .join()
                .unwrap()
                .iter()
                .map(|(_, s)| untag(*s).1)
                .collect();
            let mut gaps = 0u64;
            let mut prev: Option<u32> = None;
            for f in &heard {
                match prev {
                    Some(p) => {
                        assert!(*f > p, "round {round}: {f} after {p}");
                        gaps += u64::from(f - p - 1);
                    }
                    None => gaps += u64::from(*f),
                }
                prev = Some(*f);
            }
            gaps += u64::from(total - 1 - prev.unwrap_or(0));
            assert_eq!(gaps, skip, "round {round}: dropped exactly the skip");
            assert_eq!(heard.len() as u64, u64::from(total) - skip);
            assert_eq!(r.pending_corrections(), (0, 0));
        }
    }

    /// A timed start armed while the output runs: the audio starts exactly at the frame
    /// asked for, or, if that frame had already gone by, as far into the audio as the time
    /// since (so it is still in step), counted as late.
    #[test]
    fn a_timed_start_while_the_output_runs_lands_in_step() {
        let mut x = 3u64;
        for round in 0..200 {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let r = Ring::new(8192);
            r.reset_device_clock();
            let mut p = r.producer().unwrap();
            p.write(&tagged(0x0100_0000, 0, 3000));
            let stop = Arc::new(AtomicBool::new(false));
            let out = run_output(r.consumer().unwrap(), 16, stop.clone());
            for _ in 0..(x >> 33) % 100 {
                std::thread::yield_now();
            }
            // Sometimes ahead of the output, sometimes already behind it.
            let start = (r.device_frames() + (x >> 20) % 400).saturating_sub(100);
            r.start_at(start);
            wait_until(&out, || !r.is_held() && r.is_empty());
            stop.store(true, Ordering::Release);
            let heard = out.join().unwrap();
            let (dev, first) = heard[0];
            let first = untag(first).1;
            assert_eq!(
                dev - u64::from(first),
                start,
                "round {round}: frame {first} at device frame {dev} for a start at {start}"
            );
            assert_eq!(r.late_frames(), u64::from(first), "round {round}");
            for (i, (d, s)) in heard.iter().enumerate() {
                assert_eq!(untag(*s).1, first + i as u32, "round {round}: in order");
                assert_eq!(*d, dev + i as u64, "round {round}: without a gap");
            }
            assert!(!r.start_pending());
        }
    }

    /// A timed start armed while the ring is already held and the output runs is never
    /// lost: the hold is released at the frame asked for, whatever the timing of the
    /// hand-over. (Regression: `start_at` once stored the frame before the hold, so a read
    /// that fell between the two stores acted on the start, released the hold, and was then
    /// held again with no start pending: silence for good.)
    #[test]
    fn a_start_armed_while_held_is_never_lost() {
        let r = Ring::new(256);
        r.reset_device_clock();
        let stop = Arc::new(AtomicBool::new(false));
        let out = {
            let mut c = r.consumer().unwrap();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut out = [0f32; 2 * CHANNELS];
                while !stop.load(Ordering::Acquire) {
                    c.read(&mut out);
                }
            })
        };
        for round in 0..20_000u32 {
            r.set_hold(true);
            let start = r.device_frames() + 1;
            r.start_at(start);
            while r.is_held() {
                // The output has moved well past the start (or has died) and the ring is
                // still held: the start was lost.
                if out.is_finished() || r.device_frames() > start + 4096 {
                    let state = (r.is_held(), r.start_pending(), r.device_frames());
                    stop.store(true, Ordering::Release);
                    out.join().unwrap();
                    panic!(
                        "round {round}: the start at {start} was lost; \
                         (held, start pending, device frame) = {state:?}"
                    );
                }
                std::thread::yield_now();
            }
        }
        stop.store(true, Ordering::Release);
        out.join().unwrap();
    }

    /// Producer and consumer on two threads: every sample arrives once, in order, intact.
    #[test]
    fn concurrent_transfer_is_lossless_and_ordered() {
        let r = Ring::new(1024);
        r.set_hold(false);
        let mut p = r.producer().unwrap();
        let mut c = r.consumer().unwrap();
        const TOTAL: u32 = 2_000_000;
        let writer = std::thread::spawn(move || {
            let mut next = 0u32;
            let mut chunk = Vec::with_capacity(96);
            while next < TOTAL {
                chunk.clear();
                let n = (next % 96 + 2).min(TOTAL - next) & !1;
                let n = n.max(2);
                for k in 0..n {
                    chunk.push(f32::from_bits(next + k));
                }
                let mut off = 0;
                while off < chunk.len() {
                    off += p.write(&chunk[off..]);
                    if off < chunk.len() {
                        std::thread::yield_now();
                    }
                }
                next += n;
            }
            next
        });
        let mut expect = 0u32;
        let mut out = vec![0f32; 70];
        let written_total;
        loop {
            let n = c.read(&mut out);
            for s in &out[..n] {
                assert_eq!(s.to_bits(), expect, "sample out of order or corrupted");
                expect += 1;
            }
            if n == 0 {
                if writer.is_finished() {
                    // Drain whatever was published last.
                    let n = c.read(&mut out);
                    for s in &out[..n] {
                        assert_eq!(s.to_bits(), expect);
                        expect += 1;
                    }
                    if n == 0 {
                        written_total = writer.join().unwrap();
                        break;
                    }
                } else {
                    std::thread::yield_now();
                }
            }
        }
        assert_eq!(expect, written_total);
    }
}
