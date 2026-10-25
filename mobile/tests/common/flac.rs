//! A minimal FLAC writer for tests: STREAMINFO (with the audio's MD5) plus fixed-size frames
//! of VERBATIM subframes (uncompressed, so exact by construction). Written from the FLAC
//! format specification; 16- and 24-bit stereo only.

const BLOCK: usize = 4096;

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// FLAC's UTF-8-like coding of a frame number.
fn utf8_number(mut v: u64, out: &mut Vec<u8>) {
    if v < 0x80 {
        out.push(v as u8);
        return;
    }
    let mut tail = Vec::new();
    let mut cap = 0x3f_u64;
    while v > cap {
        tail.push(0x80 | (v & 0x3f) as u8);
        v >>= 6;
        cap >>= 1;
    }
    let n = tail.len() + 1;
    out.push(!(0xffu8 >> n) | v as u8);
    out.extend(tail.iter().rev());
}

/// A FLAC file of interleaved stereo integer `samples` (right-aligned, `bits` deep).
pub fn encode(rate: u32, bits: u32, samples: &[i32]) -> Vec<u8> {
    encode_with(rate, bits, samples, false)
}

/// [`encode`], with every frame header carrying its own rate and depth (as real encoders
/// write them), so the frames alone, without `fLaC` and STREAMINFO (the stream LMS sends
/// from a seek point), still say what they are.
pub fn encode_self_describing(rate: u32, bits: u32, samples: &[i32]) -> Vec<u8> {
    encode_with(rate, bits, samples, true)
}

fn encode_with(rate: u32, bits: u32, samples: &[i32], self_describing: bool) -> Vec<u8> {
    assert!(bits == 16 || bits == 24);
    // The frame header's codes (FLAC format, "FRAME_HEADER"); 0: from STREAMINFO.
    let (rate_code, depth_code) = if self_describing {
        let r = match rate {
            88_200 => 0b0001,
            176_400 => 0b0010,
            192_000 => 0b0011,
            44_100 => 0b1001,
            48_000 => 0b1010,
            96_000 => 0b1011,
            _ => panic!("no frame-header code for {rate} Hz"),
        };
        (r, if bits == 16 { 0b100 } else { 0b110 })
    } else {
        (0, 0)
    };
    let frames = samples.len() / 2;
    let mut out = b"fLaC".to_vec();
    // STREAMINFO, the last metadata block.
    out.push(0x80);
    out.extend_from_slice(&[0, 0, 34]);
    let mut si = [0u8; 34];
    si[0..2].copy_from_slice(&(BLOCK as u16).to_be_bytes());
    si[2..4].copy_from_slice(&(BLOCK as u16).to_be_bytes());
    let packed: u64 = ((rate as u64) << 44)
        | (1u64 << 41) // two channels
        | (((bits - 1) as u64) << 36)
        | frames as u64;
    si[10..18].copy_from_slice(&packed.to_be_bytes());
    si[18..34].copy_from_slice(&anywhere_core::decode::flac_pcm_md5(samples, bits));
    out.extend_from_slice(&si);

    let width = (bits / 8) as usize;
    for (number, chunk) in samples.chunks(BLOCK * 2).enumerate() {
        let n = chunk.len() / 2;
        let mut f = vec![0xff, 0xf8]; // sync, fixed block size
        // Block size: 16-bit (n-1) at the end of the header; rate and depth from
        // STREAMINFO, or given here.
        f.push(0b0111_0000 | rate_code);
        f.push(0b0001_0000 | (depth_code << 1)); // left/right independent
        utf8_number(number as u64, &mut f);
        f.extend(((n - 1) as u16).to_be_bytes());
        f.push(crc8(&f));
        for ch in 0..2 {
            f.push(0b0000_0010); // VERBATIM, no wasted bits
            for i in 0..n {
                let s = chunk[i * 2 + ch];
                f.extend(&s.to_be_bytes()[4 - width..]);
            }
        }
        let crc = crc16(&f);
        f.extend(crc.to_be_bytes());
        out.extend(f);
    }
    out
}

/// `file` (from [`encode`]) as a library file usually is: tags and embedded artwork after
/// its STREAMINFO (the gateway strips the artwork from a stream).
pub fn with_tags_and_art(file: &[u8], title: &str, art_bytes: usize) -> Vec<u8> {
    assert_eq!(file[4], 0x80, "STREAMINFO alone");
    let mut out = file[..42].to_vec();
    out[4] = 0; // no longer the last block
    let mut tags = Vec::new();
    tags.extend(4u32.to_le_bytes());
    tags.extend(b"test");
    let tag = format!("TITLE={title}");
    tags.extend(1u32.to_le_bytes());
    tags.extend((tag.len() as u32).to_le_bytes());
    tags.extend(tag.as_bytes());
    out.push(4);
    out.extend(&(tags.len() as u32).to_be_bytes()[1..]);
    out.extend(tags);
    // PICTURE: front cover, a JPEG of `art_bytes`.
    let mut pic = Vec::new();
    pic.extend(3u32.to_be_bytes());
    pic.extend(10u32.to_be_bytes());
    pic.extend(b"image/jpeg");
    pic.extend(0u32.to_be_bytes());
    pic.extend([0u8; 16]);
    pic.extend((art_bytes as u32).to_be_bytes());
    pic.extend((0..art_bytes).map(|i| (i % 251) as u8));
    out.push(0x80 | 6);
    out.extend(&(pic.len() as u32).to_be_bytes()[1..]);
    out.extend(pic);
    out.extend(&file[42..]);
    out
}

/// Pseudo-random full-scale samples (with the extremes) for bit-exactness checks.
pub fn noise(frames: usize, bits: u32, seed: u64) -> Vec<i32> {
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..frames * 2)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            match i % 1000 {
                0 => max as i32,
                1 => min as i32,
                _ => ((x as i64).rem_euclid(max - min + 1) + min) as i32,
            }
        })
        .collect()
}

/// Decode with the core, as the player does: the reference PCM.
pub fn decode(file: &[u8]) -> (u32, Vec<f32>) {
    use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
    let input = InputBuffer::new();
    input.push(file);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, b'f');
    let mut rate = 0;
    let mut pcm = Vec::new();
    loop {
        match dec.work(64, &mut |r, s| {
            rate = r;
            pcm.extend_from_slice(s);
        }) {
            DecodeStatus::Progress => {}
            DecodeStatus::Finished => return (rate, pcm),
            other => panic!("reference decode failed: {other:?}"),
        }
    }
}
