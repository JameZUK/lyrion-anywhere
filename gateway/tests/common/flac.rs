//! A small FLAC writer (16-bit stereo, verbatim subframes) for stream tests.

pub const BLOCK: usize = 1152;

pub fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut crc, b| {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

pub fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |mut crc, b| {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
        crc
    })
}

fn block(last: bool, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![(u8::from(last) << 7) | kind];
    v.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    v.extend_from_slice(body);
    v
}

/// A FLAC file laid out as library files usually are: STREAMINFO, SEEKTABLE,
/// VORBIS_COMMENT, PICTURE (`picture` bytes of image), PADDING, then `frames` frames.
pub fn library_flac(picture: usize, frames: usize) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    let mut si = Vec::new();
    si.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    si.extend_from_slice(&(BLOCK as u16).to_be_bytes());
    si.extend_from_slice(&[0; 6]);
    let packed: u64 = (44_100u64 << 44) | (1 << 41) | (15 << 36) | (frames * BLOCK) as u64;
    si.extend_from_slice(&packed.to_be_bytes());
    si.extend_from_slice(&[0; 16]);
    v.extend(block(false, 0, &si));
    v.extend(block(false, 3, &[0; 36]));
    let mut vc = 4u32.to_le_bytes().to_vec();
    vc.extend_from_slice(b"test");
    vc.extend_from_slice(&1u32.to_le_bytes());
    vc.extend_from_slice(&10u32.to_le_bytes());
    vc.extend_from_slice(b"TITLE=Test");
    v.extend(block(false, 4, &vc));
    let mut pic = 3u32.to_be_bytes().to_vec();
    pic.extend_from_slice(&10u32.to_be_bytes());
    pic.extend_from_slice(b"image/jpeg");
    pic.extend_from_slice(&0u32.to_be_bytes());
    for x in [600u32, 600, 24, 0, picture as u32] {
        pic.extend_from_slice(&x.to_be_bytes());
    }
    pic.extend((0..picture).map(|i| (i * 31 % 253) as u8));
    v.extend(block(false, 6, &pic));
    v.extend(block(true, 1, &[0; 8192]));
    let mut seed = 0x1234_5678_9abc_def1u64;
    for n in 0..frames {
        let mut f = vec![0xFF, 0xF8, 0x39, 0x18];
        if n < 0x80 {
            f.push(n as u8);
        } else {
            f.extend_from_slice(&[0xC0 | (n >> 6) as u8, 0x80 | (n & 0x3F) as u8]);
        }
        f.push(crc8(&f));
        for _channel in 0..2 {
            f.push(0x02); // a verbatim subframe
            for _ in 0..BLOCK {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                f.extend_from_slice(&((seed >> 40) as i16 / 8).to_be_bytes());
            }
        }
        let c = crc16(&f);
        f.extend_from_slice(&c.to_be_bytes());
        v.extend(f);
    }
    v
}

/// Where the first audio frame starts.
pub fn audio_offset(flac: &[u8]) -> usize {
    let mut pos = 4;
    loop {
        let len = u32::from_be_bytes([0, flac[pos + 1], flac[pos + 2], flac[pos + 3]]) as usize;
        let last = flac[pos] & 0x80 != 0;
        pos += 4 + len;
        if last {
            return pos;
        }
    }
}
