//! Robustness of the decoder against hostile packets (a Lyrion Anywhere addition, see
//! `PATCHES.md`).
//!
//! The decoder runs on untrusted bytes (internet radio, a compromised server), and in the
//! browser build a panic aborts the whole WebAssembly instance, so no packet may panic it.
//! Every case decodes under `catch_unwind` and the test fails, listing the packets, if any
//! panicked. The PRNG is seeded, so every run is the same.
//!
//! Fixtures `fixtures/*.pkts` are raw Opus packets (each a big-endian `u16` length, then the
//! packet) taken from Ogg files we made with FFmpeg's libopus encoder from a synthetic
//! chirp plus noise (so no third-party content):
//!
//! ```text
//! SRC="aevalsrc=0.2*sin(2*PI*(200+400*t)*t)+0.05*(random(0)-0.5):s=48000:d=0.4"
//! silk:   -ac 1 -b:a 8k  -application voip                       (SILK NB, config 1)
//! hybrid: -ac 1 -b:a 24k -application voip                       (Hybrid FB, config 15)
//! celt5:  -ac 2 -b:a 64k -frame_duration 5 -application lowdelay (CELT 5 ms, config 29)
//! long60: -ac 2 -b:a 48k -frame_duration 60                       (code 3, configs 15 and 31)
//! ```

use std::panic::{AssertUnwindSafe, catch_unwind};

use opus_decoder::{OpusDecoder, OpusMultistreamDecoder};

/// Small deterministic PRNG (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

const RATES: [u32; 5] = [48_000, 24_000, 16_000, 12_000, 8_000];
const MAX_OUT: usize = OpusDecoder::MAX_FRAME_SIZE_48K * 2;

/// Decode `packet` with `dec` (both the i16 and the float entry points), returning whether
/// it panicked. Errors are fine; a panic is not.
fn panics(dec: &mut OpusDecoder, packet: &[u8]) -> bool {
    let mut pcm = vec![0i16; MAX_OUT];
    let mut pcmf = vec![0f32; MAX_OUT];
    catch_unwind(AssertUnwindSafe(|| {
        let _ = dec.decode(packet, &mut pcm, false);
        let _ = dec.decode_float(packet, &mut pcmf, false);
        // Packet loss concealment after whatever state the packet left behind.
        let _ = dec.decode(&[], &mut pcm, false);
        assert!(pcmf.iter().all(|s| s.is_finite()), "non-finite output");
    }))
    .is_err()
}

fn check(failures: &[Vec<u8>]) {
    assert!(
        failures.is_empty(),
        "{} packets panicked the decoder; first few: {:02x?}",
        failures.len(),
        &failures[..failures.len().min(4)]
    );
}

fn fixture(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        let n = usize::from(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        out.push(bytes[i + 2..i + 2 + n].to_vec());
        i += 2 + n;
    }
    out
}

fn fixtures() -> Vec<(&'static str, Vec<Vec<u8>>)> {
    vec![
        ("silk", fixture(include_bytes!("fixtures/silk.pkts"))),
        ("hybrid", fixture(include_bytes!("fixtures/hybrid.pkts"))),
        ("celt5", fixture(include_bytes!("fixtures/celt5.pkts"))),
        ("long60", fixture(include_bytes!("fixtures/long60.pkts"))),
    ]
}

/// Regression (H2): short Hybrid frames whose range-coded redundancy flag is set and whose
/// redundancy byte count (2 to 257) exceeds what is left of the frame. The decoder sliced
/// `frame[frame.len() - redundancy_bytes..]` unchecked, so these packets panicked it (an
/// overflow in debug builds, an out-of-range slice in release builds). It must now drop the
/// redundancy, as libopus does, and decode.
///
/// Both found by brute force on the unpatched decoder: TOC config 12 (Hybrid SWB 10 ms),
/// code 0, then range-coded bits that set the redundancy flag with a large count. The first
/// takes the SILK-to-CELT branch (redundancy decoded after the main frame), the second the
/// CELT-to-SILK branch (redundancy decoded before it).
#[test]
fn hybrid_redundancy_longer_than_the_frame_does_not_panic() {
    for packet in [&SILK_TO_CELT[..], &CELT_TO_SILK[..]] {
        for &rate in &RATES {
            for channels in 1..=2 {
                let mut dec = OpusDecoder::new(rate, channels).unwrap();
                assert!(!panics(&mut dec, packet), "{rate} Hz, {channels} ch");
                // And the decoder still works afterwards.
                let mut pcm = vec![0i16; MAX_OUT];
                let n = dec.decode(packet, &mut pcm, false).unwrap();
                assert_eq!(n, rate as usize / 100);
            }
        }
    }
}

const SILK_TO_CELT: [u8; 45] = [
    0x60, 0x2f, 0x6b, 0xc3, 0xd5, 0x01, 0xd8, 0x95, 0x2e, 0xc0, 0x56, 0x4e, 0x3e, 0x7a, 0xa0,
    0x37, 0x89, 0x50, 0x65, 0x96, 0x98, 0xff, 0xed, 0xeb, 0x7c, 0x85, 0x23, 0x0b, 0x8c, 0xb7,
    0x0b, 0x96, 0x23, 0x34, 0x9a, 0x73, 0xc0, 0x51, 0x66, 0x02, 0x70, 0x1e, 0xb6, 0xd6, 0x65,
];

const CELT_TO_SILK: [u8; 32] = [
    0x60, 0x1f, 0xba, 0x7b, 0x32, 0xf2, 0x93, 0x06, 0x9b, 0x12, 0xe6, 0xd7, 0x33, 0xd1, 0x90,
    0x4d, 0x24, 0x04, 0xe4, 0x5b, 0x10, 0xdc, 0x31, 0x31, 0x20, 0x5c, 0x27, 0xda, 0x12, 0x1e,
    0xbd, 0xdd,
];

/// Short Hybrid frames with random payloads: about one in 4096 sets the redundancy flag,
/// and most of those then claim more redundancy bytes than the frame has.
#[test]
fn short_hybrid_frames_never_panic() {
    let mut r = Rng(0x4b1d);
    let iters = if cfg!(debug_assertions) { 8_000 } else { 100_000 };
    let mut dec = OpusDecoder::new(48_000, 2).unwrap();
    let mut failures = Vec::new();
    for i in 0..iters {
        let config = 12 + (i % 4) as u8;
        let stereo = r.below(2) as u8;
        let len = 6 + r.below(40);
        let mut p = vec![(config << 3) | (stereo << 2)];
        p.extend(r.bytes(len));
        if panics(&mut dec, &p) {
            failures.push(p);
            dec = OpusDecoder::new(48_000, 2).unwrap();
        }
    }
    check(&failures);
}

/// Every TOC configuration (SILK NB/MB/WB, Hybrid, CELT 2.5 to 20 ms), mono and stereo, with
/// every frame-count code 0 to 3, and random payloads of every length 0 to 300, through
/// decoders at every output rate and channel count.
#[test]
fn random_packets_for_every_toc_never_panic() {
    let mut r = Rng(0x0b05_5eed);
    let mut failures = Vec::new();
    for config in 0u8..=31 {
        for code in 0u8..=3 {
            let rate = RATES[usize::from(config) % RATES.len()];
            let channels = 1 + usize::from(code) % 2;
            let mut dec = OpusDecoder::new(rate, channels).unwrap();
            for len in 0..=300 {
                let stereo = r.below(2) as u8;
                let mut p = vec![(config << 3) | (stereo << 2) | code];
                p.extend(r.bytes(len));
                if code == 3 && len > 0 {
                    // Mostly plausible frame counts, so the frames are actually decoded.
                    p[1] = (p[1] & 0xc0) | (1 + r.below(6) as u8);
                }
                if panics(&mut dec, &p) {
                    failures.push(p);
                    dec = OpusDecoder::new(rate, channels).unwrap();
                }
            }
        }
    }
    check(&failures);
}

/// Real packets of each mode decode, and truncated or bit-flipped copies never panic.
#[test]
fn truncated_and_bit_flipped_real_packets_never_panic() {
    let mut r = Rng(0xf11b);
    let rounds = if cfg!(debug_assertions) { 4 } else { 16 };
    let mut failures = Vec::new();
    for (name, packets) in fixtures() {
        assert!(!packets.is_empty(), "{name}");
        // The unmodified stream decodes cleanly.
        let mut dec = OpusDecoder::new(48_000, 2).unwrap();
        let mut pcm = vec![0i16; MAX_OUT];
        for p in &packets {
            dec.decode(p, &mut pcm, false)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for round in 0..rounds {
            let rate = RATES[round % RATES.len()];
            let mut dec = OpusDecoder::new(rate, 1 + round % 2).unwrap();
            for p in &packets {
                let mut m = p.clone();
                match r.below(4) {
                    0 => m.truncate(r.below(m.len() + 1)),
                    1 => {
                        for _ in 0..1 + r.below(8) {
                            let k = r.below(m.len());
                            m[k] ^= 1 << r.below(8);
                        }
                    }
                    2 => {
                        // Keep the TOC, scramble the rest from a random point.
                        let start = 1 + r.below(m.len());
                        for b in m.iter_mut().skip(start) {
                            *b = r.next() as u8;
                        }
                    }
                    _ => {
                        // Flip a bit in the last few bytes, where CELT's raw bits and the
                        // Hybrid redundancy live.
                        let k = m.len() - 1 - r.below(m.len().min(8));
                        m[k] ^= 1 << r.below(8);
                    }
                }
                if panics(&mut dec, &m) {
                    failures.push(m);
                    dec = OpusDecoder::new(rate, 1 + round % 2).unwrap();
                }
            }
        }
    }
    check(&failures);
}

/// The multistream wrapper (what Lyrion Anywhere's core uses) with random packets: two
/// streams, one coupled, so the self-delimited framing of the first stream is exercised.
#[test]
fn random_multistream_packets_never_panic() {
    let mut r = Rng(0x3a17);
    let iters = if cfg!(debug_assertions) { 3_000 } else { 20_000 };
    let mut dec = OpusMultistreamDecoder::new(48_000, 3, 2, 1, &[0, 1, 2]).unwrap();
    let mut pcm = vec![0f32; OpusDecoder::MAX_FRAME_SIZE_48K * 3];
    let mut failures = Vec::new();
    for _ in 0..iters {
        let len = r.below(400);
        let mut p = r.bytes(len);
        if len > 2 {
            // A plausible self-delimiting length for the first stream.
            p[1] = r.below(len.min(250)) as u8;
        }
        let ok = catch_unwind(AssertUnwindSafe(|| {
            let _ = dec.decode_float(&p, &mut pcm, false);
        }))
        .is_ok();
        if !ok {
            failures.push(p);
            dec = OpusMultistreamDecoder::new(48_000, 3, 2, 1, &[0, 1, 2]).unwrap();
        }
    }
    check(&failures);
}
