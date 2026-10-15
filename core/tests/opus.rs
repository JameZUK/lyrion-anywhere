//! Ogg Opus decoding (the `opus` feature).
//!
//! Fixture `fixtures/sine-997hz-stereo.opus`: one second of a synthetic 997 Hz stereo sine
//! at -20 dBFS peak, made by us with FFmpeg's libopus encoder (so no third-party content):
//!
//! ```text
//! ffmpeg -f lavfi -i "aevalsrc=0.1*sin(2*PI*997*t)|0.1*sin(2*PI*997*t):s=48000:d=1" \
//!   -c:a libopus -b:a 32k -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
//!   sine-997hz-stereo.opus
//! ```
#![cfg(feature = "opus")]

use anywhere_core::analysis::{AnalysisConfig, analyse_encoded};
use anywhere_core::decode::{
    DecodeStatus, InputBuffer, STREAM_DECODER_PANICKED, StreamDecoder, format_extension,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/sine-997hz-stereo.opus");

fn decode(bytes: &[u8], format: u8) -> (DecodeStatus, u32, Vec<f32>) {
    let input = InputBuffer::new();
    input.push(bytes);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, format);
    let mut rate = 0;
    let mut pcm = Vec::new();
    loop {
        let st = dec.work(16, &mut |r, s| {
            rate = r;
            pcm.extend_from_slice(s);
        });
        if st != DecodeStatus::Progress {
            return (st, rate, pcm);
        }
    }
}

#[test]
fn decodes_ogg_opus_sample_accurately() {
    assert_eq!(format_extension(b'u'), Some("opus"));
    for format in *b"uo" {
        let (st, rate, pcm) = decode(FIXTURE, format);
        assert_eq!(st, DecodeStatus::Finished);
        assert_eq!(rate, 48_000);
        // Pre-skip and end trimming give back exactly the encoded second.
        assert_eq!(pcm.len(), 48_000 * 2, "frames: {}", pcm.len() / 2);
        let peak = pcm.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.1).abs() < 0.01, "peak {peak}");
        // Aligned with the source: fit the sine's phase over the middle of the second.
        let w = 2.0 * std::f64::consts::PI * 997.0 / 48_000.0;
        let (mut s, mut c) = (0.0, 0.0);
        for i in 8_000..40_000 {
            let x = f64::from(pcm[i * 2]);
            s += x * (w * i as f64).sin();
            c += x * (w * i as f64).cos();
        }
        let lag = -c.atan2(s) / w;
        assert!(lag.abs() < 1.0, "decoded audio is {lag} samples off");
    }
}

#[test]
fn opus_loudness_matches_the_source() {
    let a = analyse_encoded(FIXTURE, b'u', AnalysisConfig::default()).unwrap();
    assert_eq!(a.sample_rate, 48_000);
    assert_eq!(a.frames, 48_000);
    // A -20 dBFS 997 Hz stereo sine is -20 LUFS; Opus at 32 kb/s keeps it within 0.2 LU.
    let i = a.integrated_lufs.unwrap();
    assert!((i + 20.0).abs() < 0.2, "{i}");
}

/// Small deterministic PRNG.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Ogg's page checksum (CRC-32, polynomial 0x04c11db7, unreflected, initial value 0).
fn ogg_crc(page: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in page {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The byte ranges of each Ogg page and of its body (header and segment table excluded).
fn ogg_pages(b: &[u8]) -> Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 27 <= b.len() && &b[i..i + 4] == b"OggS" {
        let nseg = usize::from(b[i + 26]);
        let body = i + 27 + nseg;
        let len: usize = b[i + 27..body].iter().map(|&x| usize::from(x)).sum();
        out.push((i..body + len, body..body + len));
        i = body + len;
    }
    out
}

/// Corrupt Ogg Opus never panics the decoder. A caught panic doesn't count as surviving:
/// it would abort the web player, where panics can't be caught (M13). Most random damage is
/// caught by the Ogg page checksum before a packet reaches the Opus decoder, so a quarter
/// of the cases damage the audio pages and then fix their checksums, which feeds the
/// decoder hostile packets.
#[test]
fn corrupt_opus_never_panics() {
    let pages = ogg_pages(FIXTURE);
    assert!(pages.len() > 3);
    let mut r = Rng(0x0b05);
    let iters = if cfg!(debug_assertions) { 300 } else { 3000 };
    for i in 0..iters {
        let mut b = FIXTURE.to_vec();
        match i % 4 {
            // Flip bits anywhere (headers, page CRCs, packets).
            0 => {
                for _ in 0..1 + r.next() % 16 {
                    let k = (r.next() % b.len() as u64) as usize;
                    b[k] ^= 1 << (r.next() % 8);
                }
            }
            // Truncate.
            1 => b.truncate((r.next() % b.len() as u64) as usize),
            // Overwrite a run with random bytes, past the headers.
            2 => {
                let start = 200 + (r.next() % (b.len() as u64 - 400)) as usize;
                for byte in &mut b[start..start + 64] {
                    *byte = r.next() as u8;
                }
            }
            // Damage the audio pages' bodies (past OpusHead and OpusTags), keeping the
            // checksums valid.
            _ => {
                for (page, body) in &pages[2..] {
                    if body.is_empty() || r.next() % 3 != 0 {
                        continue;
                    }
                    for _ in 0..1 + r.next() % 6 {
                        let k = body.start + (r.next() % body.len() as u64) as usize;
                        b[k] = if r.next() % 2 == 0 {
                            b[k] ^ (1 << (r.next() % 8))
                        } else {
                            r.next() as u8
                        };
                    }
                    b[page.start + 22..page.start + 26].fill(0);
                    let crc = ogg_crc(&b[page.clone()]);
                    b[page.start + 22..page.start + 26].copy_from_slice(&crc.to_le_bytes());
                }
            }
        }
        let (st, _, pcm) = decode(&b, b'u');
        match st {
            DecodeStatus::Finished => {}
            DecodeStatus::Failed(e) => assert_ne!(e, STREAM_DECODER_PANICKED, "case {i}"),
            other => panic!("case {i}: {other:?}"),
        }
        assert!(pcm.iter().all(|s| s.is_finite()));
    }
}

/// The checksum fix-up above is right: an untouched page keeps its checksum.
#[test]
fn ogg_crc_matches_the_fixture() {
    for (page, _) in ogg_pages(FIXTURE) {
        let mut p = FIXTURE[page].to_vec();
        let want = u32::from_le_bytes(p[22..26].try_into().unwrap());
        p[22..26].fill(0);
        assert_eq!(ogg_crc(&p), want);
    }
}

/// The packets of a page that holds only whole packets.
fn page_packets(page: &[u8]) -> Vec<Vec<u8>> {
    assert_eq!(page[5] & 1, 0, "a continued packet");
    let nseg = usize::from(page[26]);
    let lacing = &page[27..27 + nseg];
    assert!(lacing.last().is_some_and(|&l| l < 255), "an unfinished packet");
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut at = 27 + nseg;
    for &l in lacing {
        cur.extend_from_slice(&page[at..at + usize::from(l)]);
        at += usize::from(l);
        if l < 255 {
            out.push(std::mem::take(&mut cur));
        }
    }
    out
}

/// A copy of `page` (its header fields kept) holding `packets`, with a valid checksum.
fn rebuild_page(page: &[u8], packets: &[Vec<u8>]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for p in packets {
        lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
        lacing.push((p.len() % 255) as u8);
    }
    let mut out = page[..26].to_vec();
    out.push(u8::try_from(lacing.len()).unwrap());
    out.extend_from_slice(&lacing);
    for p in packets {
        out.extend_from_slice(p);
    }
    out[22..26].fill(0);
    let crc = ogg_crc(&out);
    out[22..26].copy_from_slice(&crc.to_le_bytes());
    out
}

/// Regression (H2): short Hybrid packets whose redundancy byte count exceeds the frame
/// panicked the vendored decoder, which in the browser aborts the web player. Here they
/// arrive the way a hostile stream would deliver them, inside valid Ogg pages: the stream
/// must decode on (the decoder drops the bad redundancy, as libopus does), and certainly
/// not end in a caught panic. The two packets are the vendored crate's regression packets
/// (`third_party/opus-decoder/tests/robustness.rs`), one per redundancy branch.
#[test]
fn a_hostile_hybrid_packet_in_ogg_does_not_panic() {
    const SILK_TO_CELT: [u8; 45] = [
        0x60, 0x2f, 0x6b, 0xc3, 0xd5, 0x01, 0xd8, 0x95, 0x2e, 0xc0, 0x56, 0x4e, 0x3e, 0x7a,
        0xa0, 0x37, 0x89, 0x50, 0x65, 0x96, 0x98, 0xff, 0xed, 0xeb, 0x7c, 0x85, 0x23, 0x0b,
        0x8c, 0xb7, 0x0b, 0x96, 0x23, 0x34, 0x9a, 0x73, 0xc0, 0x51, 0x66, 0x02, 0x70, 0x1e,
        0xb6, 0xd6, 0x65,
    ];
    const CELT_TO_SILK: [u8; 32] = [
        0x60, 0x1f, 0xba, 0x7b, 0x32, 0xf2, 0x93, 0x06, 0x9b, 0x12, 0xe6, 0xd7, 0x33, 0xd1,
        0x90, 0x4d, 0x24, 0x04, 0xe4, 0x5b, 0x10, 0xdc, 0x31, 0x31, 0x20, 0x5c, 0x27, 0xda,
        0x12, 0x1e, 0xbd, 0xdd,
    ];
    let pages = ogg_pages(FIXTURE);
    for hostile in [&SILK_TO_CELT[..], &CELT_TO_SILK[..]] {
        // The first audio page (after OpusHead and OpusTags), its first packet replaced.
        let (page, _) = pages[2].clone();
        let mut packets = page_packets(&FIXTURE[page.clone()]);
        assert!(packets.len() > 1);
        packets[0] = hostile.to_vec();
        let mut b = FIXTURE[..page.start].to_vec();
        b.extend(rebuild_page(&FIXTURE[page.clone()], &packets));
        b.extend_from_slice(&FIXTURE[page.end..]);
        let (st, rate, pcm) = decode(&b, b'u');
        if let DecodeStatus::Failed(e) = &st {
            assert_ne!(e, STREAM_DECODER_PANICKED, "the hostile packet panicked the decoder");
        }
        assert_eq!(st, DecodeStatus::Finished);
        assert_eq!(rate, 48_000);
        assert!(pcm.len() > 40_000 * 2, "{} samples", pcm.len());
        assert!(pcm.iter().all(|s| s.is_finite()));
    }
}

/// Regression: `opus-decoder` 0.1.1 overflowed building a collapse mask for bands with more
/// than 8 short blocks: a panic in debug builds, wrongly decoded transients in release
/// builds (fixed in `third_party/opus-decoder`, see its `PATCHES.md`).
///
/// Fixture `fixtures/noise-96k.opus`: the last second and a half of what the gateway's live
/// encoder (FFmpeg's libopus, 96 kb/s) made of its own synthetic test signal (pseudo-random
/// FLAC frames, `gateway/tests/live.rs`), ending where the signal stops. Cut without
/// re-encoding, so no third-party content:
///
/// ```text
/// ffmpeg -ss 9 -i whole.opus -c copy -map_metadata -1 -fflags +bitexact noise-96k.opus
/// ```
#[test]
fn a_transient_with_many_short_blocks_decodes() {
    let (st, rate, pcm) = decode(include_bytes!("fixtures/noise-96k.opus"), b'u');
    assert_eq!(st, DecodeStatus::Finished);
    assert_eq!(rate, 48_000);
    assert!(pcm.len() > 48_000 * 2, "{} samples", pcm.len());
    assert!(pcm.iter().all(|s| s.is_finite() && s.abs() <= 2.0));
}

/// Opus decodes far faster than real time. The vendored decoder's inverse MDCT used a direct
/// O(n²) DFT and ran at about 2× real time; its FFT plan runs at well over 100×. The floor
/// (20×) is asserted only in optimised builds and leaves room for slow CI machines.
#[test]
fn opus_decodes_far_faster_than_real_time() {
    let noise: &[u8] = include_bytes!("fixtures/noise-96k.opus");
    let start = std::time::Instant::now();
    let mut secs = 0.0;
    while start.elapsed().as_secs_f64() < 0.5 || secs < 5.0 {
        let (st, rate, pcm) = decode(noise, b'u');
        assert_eq!(st, DecodeStatus::Finished);
        secs += pcm.len() as f64 / 2.0 / f64::from(rate);
        if secs > 120.0 {
            break;
        }
    }
    let speed = secs / start.elapsed().as_secs_f64();
    println!("Opus (noise, 96 kb/s): {speed:.0}x real time");
    if !cfg!(debug_assertions) {
        assert!(speed >= 20.0, "Opus decodes at only {speed:.1}x real time");
    }
}
