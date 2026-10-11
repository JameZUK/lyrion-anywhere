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
//! - **Hold:** while held, the consumer outputs silence and consumes nothing (pause).
//! - **Gain:** an optional factor (ducking) applied on the way out; exactly 1.0 is a true
//!   bypass, so the samples stay bit-identical.
//!
//! One producer and one consumer at a time are enforced at run time: each handle can be
//! taken once and is given back when dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anywhere_core::dsp::pcm;

/// Channels per frame (the core's output is always stereo).
pub const CHANNELS: usize = 2;

pub struct Ring {
    buf: Box<[AtomicU32]>,
    mask: u64,
    /// Samples written (producer-owned).
    write: AtomicU64,
    /// Samples read or discarded (consumer-owned).
    read: AtomicU64,
    /// The consumer skips forward to here (producer-owned).
    discard_to: AtomicU64,
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
            hold: AtomicBool::new(true),
            gain: AtomicU32::new(1.0f32.to_bits()),
            expect_data: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            producer_taken: AtomicBool::new(false),
            consumer_taken: AtomicBool::new(false),
        })
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
        w
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
        // Apply a discard first (even while held), so the producer gets the space back.
        let mut r = ring.read.load(Ordering::Relaxed);
        let discard = ring.discard_to.load(Ordering::Acquire);
        if discard > r {
            r = discard;
            ring.read.store(r, Ordering::Release);
        }
        if ring.hold.load(Ordering::Acquire) {
            (0..len).for_each(|i| put(i, 0.0));
            return 0;
        }
        let w = ring.write.load(Ordering::Acquire);
        let avail = w.saturating_sub(r) as usize;
        let n = avail.min(len);
        let gain = f32::from_bits(ring.gain.load(Ordering::Acquire));
        for i in 0..n {
            let slot = ((r + i as u64) & ring.mask) as usize;
            let s = f32::from_bits(ring.buf[slot].load(Ordering::Relaxed));
            // Exactly 1.0 leaves the bits untouched (no multiplication at all).
            put(i, if gain == 1.0 { s } else { s * gain });
        }
        (n..len).for_each(|i| put(i, 0.0));
        ring.read.store(r + n as u64, Ordering::Release);
        if n < len && ring.expect_data.load(Ordering::Acquire) {
            ring.underruns.fetch_add(1, Ordering::Relaxed);
            ring.underrun_frames
                .fetch_add(((len - n) / CHANNELS) as u64, Ordering::Relaxed);
        }
        n
    }
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
