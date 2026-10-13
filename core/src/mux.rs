//! Lossless segments for a browser `<audio>` element (Media Source Extensions).
//!
//! Some browsers (iOS Safari) keep a page running in the background only while a media
//! element plays. This turns the player's decoded PCM into fragmented MP4 with FLAC audio
//! (`audio/mp4; codecs="flac"`), or ALAC (`codecs="alac"`) for browsers that refuse our FLAC:
//! an init segment per sample rate, then media segments of whole frames. Both are written
//! uncompressed (FLAC VERBATIM subframes, ALAC escape frames), so the encoding is exact and
//! cheap; the data never leaves the device.
//!
//! No I/O: the host appends the bytes to a `SourceBuffer`.

/// Frames (samples per channel) per FLAC frame / MP4 sample.
pub const BLOCK: usize = 4096;
const BITS: u32 = 24;
const CHANNELS: u32 = 2;
const FULL_SCALE: f32 = 8_388_607.0; // 2^23 - 1

/// How the init segment is written. Browsers' MP4 parsers differ (notably for sample rates
/// above 65,535 Hz, which don't fit the classic sample entry), so the host can pick the
/// variant a browser's self-test accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Variant {
    pub brand: Brand,
    pub high_rate: HighRate,
    /// FLAC's fixed block-size strategy (every frame `BLOCK` long, the last padded), which
    /// some decoders need; otherwise the variable strategy.
    pub fixed_blocks: bool,
    /// 16-bit samples instead of 24-bit.
    pub bits16: bool,
    /// Apple Lossless (ALAC) instead of FLAC.
    pub alac: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Brand {
    #[default]
    Iso6,
    Iso5,
    Mp42,
}

/// How a sample rate above 65,535 Hz is written in the sample entry (the FLAC
/// configuration always carries the true rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HighRate {
    /// The rate field is 0.
    #[default]
    Zero,
    /// A version-1 sample entry with an `srat` box holding the true rate (ISO/IEC 14496-12).
    Srat,
    /// The largest integer divisor of the rate that fits (e.g. 96 kHz → 48 kHz).
    Divisor,
}

impl Variant {
    fn bits(self) -> u32 {
        if self.bits16 { 16 } else { BITS }
    }

    /// Compact code for hosts: brand in bits 0–3, high-rate style in bits 4–7, fixed block
    /// size in bit 8 (FLAC only), 16-bit samples in bit 9, ALAC in bit 10.
    pub fn from_code(code: u32) -> Variant {
        Variant {
            brand: match code & 0xf {
                1 => Brand::Iso5,
                2 => Brand::Mp42,
                _ => Brand::Iso6,
            },
            high_rate: match (code >> 4) & 0xf {
                1 => HighRate::Srat,
                2 => HighRate::Divisor,
                _ => HighRate::Zero,
            },
            fixed_blocks: code & 0x100 != 0,
            bits16: code & 0x200 != 0,
            alac: code & 0x400 != 0,
        }
    }

    pub fn code(self) -> u32 {
        let b = match self.brand {
            Brand::Iso6 => 0,
            Brand::Iso5 => 1,
            Brand::Mp42 => 2,
        };
        let h = match self.high_rate {
            HighRate::Zero => 0,
            HighRate::Srat => 1,
            HighRate::Divisor => 2,
        };
        b | (h << 4)
            | (u32::from(self.fixed_blocks) << 8)
            | (u32::from(self.bits16) << 9)
            | (u32::from(self.alac) << 10)
    }
}

/// Builds the init and media segments for one continuous stream of stereo PCM.
#[derive(Debug, Clone)]
pub struct Segmenter {
    rate: u32,
    variant: Variant,
    /// Interleaved stereo samples waiting to fill a whole FLAC frame.
    pending: Vec<f32>,
    /// Media timeline position, in frames at `rate`, of the next segment.
    decode_time: u64,
    sequence: u32,
    /// Sample number (variable strategy) or frame number (fixed) of the next FLAC frame.
    flac_sample: u64,
}

impl Segmenter {
    /// Start a stream at `rate` Hz. Call [`Segmenter::init_segment`] first, then
    /// [`Segmenter::push`] and [`Segmenter::segment`].
    pub fn new(rate: u32) -> Self {
        Segmenter {
            rate: rate.clamp(1, 655_350),
            variant: Variant::default(),
            pending: Vec::new(),
            decode_time: 0,
            sequence: 0,
            flac_sample: 0,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Switch sample rate (a track at a different rate). Pending audio is dropped, so call
    /// [`Segmenter::flush`] first. The timeline continues in seconds; append the new init
    /// segment before the next media segment.
    pub fn set_rate(&mut self, rate: u32) {
        let rate = rate.clamp(1, 655_350);
        if rate == self.rate {
            return;
        }
        let seconds = self.decode_time as f64 / self.rate as f64;
        self.rate = rate;
        self.decode_time = (seconds * rate as f64).round() as u64;
        self.pending.clear();
        self.flac_sample = 0;
    }

    /// Continue the timeline at `seconds` (after the host cleared its buffer ahead of the
    /// media element's current position), dropping anything queued.
    pub fn set_time(&mut self, seconds: f64) {
        let s = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        self.pending.clear();
        self.decode_time = (s * self.rate as f64).ceil() as u64;
    }

    /// Restart the timeline at zero (after a flush/seek the host resets its SourceBuffer).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.decode_time = 0;
        self.flac_sample = 0;
    }

    /// Seconds of audio already emitted in segments.
    pub fn emitted_seconds(&self) -> f64 {
        self.decode_time as f64 / self.rate as f64
    }

    /// Frames buffered but not yet in a segment.
    pub fn pending_frames(&self) -> usize {
        self.pending.len() / 2
    }

    /// The interleaved stereo PCM buffered but not yet in a segment.
    pub fn pending_pcm(&self) -> &[f32] {
        &self.pending
    }

    /// Keep only the first `frames` buffered frames (a host dropping audio it queued).
    pub fn truncate_pending(&mut self, frames: usize) {
        self.pending
            .truncate(frames.saturating_mul(2).min(self.pending.len()));
    }

    /// Media timeline position of the next segment, in frames at [`Segmenter::rate`].
    pub fn decode_time(&self) -> u64 {
        self.decode_time
    }

    /// Continue the timeline at exactly `frames` (at the current rate), dropping anything
    /// queued: for a host that removed the segments from there on.
    pub fn set_decode_time(&mut self, frames: u64) {
        self.pending.clear();
        self.decode_time = frames;
    }

    /// Queue interleaved stereo PCM (-1.0..1.0).
    pub fn push(&mut self, pcm: &[f32]) {
        self.pending.extend_from_slice(&pcm[..pcm.len() & !1]);
    }

    /// A media segment of all whole frames queued, or `None` if less than one frame.
    pub fn segment(&mut self) -> Option<Vec<u8>> {
        let whole = self.pending.len() / 2 / BLOCK * BLOCK;
        if whole == 0 {
            return None;
        }
        Some(self.emit(whole))
    }

    /// A media segment of everything queued, padding the last frame to the FLAC minimum.
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        let frames = self.pending.len() / 2;
        if frames == 0 {
            return None;
        }
        Some(self.emit(frames))
    }

    fn emit(&mut self, frames: usize) -> Vec<u8> {
        let mut samples: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut taken = 0;
        let v = self.variant;
        while taken < frames {
            let n = (frames - taken).min(BLOCK);
            let block = &self.pending[taken * 2..(taken + n) * 2];
            if v.alac {
                samples.push((n as u32, alac_frame(block, n, v.bits())));
                taken += n;
                continue;
            }
            // Fixed strategy: every frame is a full block (silence pads a short last one).
            let len = if v.fixed_blocks {
                BLOCK
            } else {
                n.max(MIN_BLOCK)
            };
            let encoded = flac_frame(block, self.rate, self.flac_sample, len, v);
            self.flac_sample += if v.fixed_blocks { 1 } else { len as u64 };
            samples.push((len as u32, encoded));
            taken += n;
        }
        self.pending.drain(..frames * 2);
        self.sequence += 1;
        let seg = media_segment(self.sequence, self.decode_time, &samples);
        self.decode_time += samples.iter().map(|(d, _)| *d as u64).sum::<u64>();
        seg
    }

    /// `ftyp` + `moov` for the current rate.
    pub fn init_segment(&self) -> Vec<u8> {
        init_segment(self.rate, self.variant)
    }

    /// Write future init segments in this variant.
    pub fn set_variant(&mut self, v: Variant) {
        self.variant = v;
    }
}

/// FLAC's minimum block size (frames); a shorter final block is padded with silence.
const MIN_BLOCK: usize = 16;

// --- FLAC ----------------------------------------------------------------------------------

fn to_i16(x: f32) -> i32 {
    let v = (x.clamp(-1.0, 1.0) * 32_767.0).round();
    if v.is_nan() { 0 } else { v as i32 }
}

fn to_i24(x: f32) -> i32 {
    let v = (x.clamp(-1.0, 1.0) * FULL_SCALE).round();
    if v.is_nan() { 0 } else { v as i32 }
}

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

/// FLAC's UTF-8-like coding of the sample number (up to 36 bits).
fn utf8_number(mut v: u64, out: &mut Vec<u8>) {
    v &= (1 << 36) - 1;
    if v < 0x80 {
        out.push(v as u8);
        return;
    }
    let mut bytes = Vec::new();
    let mut cap = 0x3f_u64; // payload bits of the first byte shrink as the length grows
    while v > cap {
        bytes.push(0x80 | (v & 0x3f) as u8);
        v >>= 6;
        cap >>= 1;
    }
    let n = bytes.len() + 1;
    let lead = !(0xffu8 >> n); // n leading ones
    out.push(lead | v as u8);
    out.extend(bytes.iter().rev());
}

fn rate_code(rate: u32) -> u8 {
    match rate {
        88_200 => 0b0001,
        176_400 => 0b0010,
        192_000 => 0b0011,
        8_000 => 0b0100,
        16_000 => 0b0101,
        22_050 => 0b0110,
        24_000 => 0b0111,
        32_000 => 0b1000,
        44_100 => 0b1001,
        48_000 => 0b1010,
        96_000 => 0b1011,
        _ => 0b0000, // from STREAMINFO
    }
}

/// One FLAC frame of verbatim stereo, `n` samples long (`interleaved` may be shorter: the
/// rest is silence). `number` is the first sample number (variable strategy) or the frame
/// number (fixed strategy).
fn flac_frame(interleaved: &[f32], rate: u32, number: u64, n: usize, v: Variant) -> Vec<u8> {
    let real = (interleaved.len() / 2).min(n);
    let width = if v.bits16 { 2 } else { 3 };
    let mut f = Vec::with_capacity(16 + n * 2 * width + 2);
    f.extend([0xff, if v.fixed_blocks { 0xf8 } else { 0xf9 }]); // sync + blocking strategy
    let explicit = n != 4096;
    let size_code = if explicit { 0b0111 } else { 0b1100 }; // 4096 has its own code
    f.push((size_code << 4) | rate_code(rate));
    f.push(if v.bits16 { 0b0001_1000 } else { 0b0001_1100 }); // stereo independent, depth
    utf8_number(number, &mut f);
    if explicit {
        f.extend(((n - 1) as u16).to_be_bytes());
    }
    f.push(crc8(&f));
    for ch in 0..2 {
        f.push(0b0000_0010); // VERBATIM subframe, no wasted bits
        for i in 0..n {
            let x = if i < real {
                interleaved[i * 2 + ch]
            } else {
                0.0
            };
            if v.bits16 {
                f.extend(&(to_i16(x) as i16).to_be_bytes());
            } else {
                f.extend(&to_i24(x).to_be_bytes()[1..]);
            }
        }
    }
    let crc = crc16(&f);
    f.extend(crc.to_be_bytes());
    f
}

/// STREAMINFO block body (34 bytes).
fn streaminfo(rate: u32, v: Variant) -> [u8; 34] {
    let mut s = [0u8; 34];
    let min = if v.fixed_blocks { BLOCK } else { MIN_BLOCK };
    s[0..2].copy_from_slice(&(min as u16).to_be_bytes());
    s[2..4].copy_from_slice(&(BLOCK as u16).to_be_bytes());
    // min/max frame size unknown (0). Then 20 bits rate, 3 bits channels-1, 5 bits bps-1,
    // 36 bits total samples (0 = unknown), then MD5 (zero = unknown).
    let packed: u64 = ((rate as u64 & 0xf_ffff) << 44)
        | (((CHANNELS - 1) as u64) << 41)
        | (((v.bits() - 1) as u64) << 36);
    s[10..18].copy_from_slice(&packed.to_be_bytes());
    s
}

// --- ALAC ----------------------------------------------------------------------------------

/// Writes bits most significant first.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn with_capacity(bytes: usize) -> Bits {
        Bits {
            out: Vec::with_capacity(bytes),
            acc: 0,
            n: 0,
        }
    }

    fn put(&mut self, value: u32, bits: u32) {
        self.acc = (self.acc << bits) | (u64::from(value) & ((1u64 << bits) - 1));
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
    }

    /// Pad to a whole byte with zero bits.
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put(0, pad);
        }
        self.out
    }
}

/// Largest ALAC frame we write (uncompressed, partial-frame size included), in bytes.
fn alac_max_frame(bits: u32) -> u32 {
    (3 + 4 + 12 + 4 + 32 + BLOCK as u32 * CHANNELS * bits + 3).div_ceil(8)
}

/// One ALAC frame: a channel-pair element sent uncompressed ("escape"), then the end tag.
/// `n` frames (a short final frame says its own length).
fn alac_frame(interleaved: &[f32], n: usize, bits: u32) -> Vec<u8> {
    let mut w = Bits::with_capacity(alac_max_frame(bits) as usize);
    w.put(1, 3); // element: channel pair
    w.put(0, 4); // element instance
    w.put(0, 12); // unused, must be zero
    let partial = n != BLOCK;
    w.put(u32::from(partial), 1);
    w.put(0, 2); // no bytes shifted out
    w.put(1, 1); // escape: uncompressed samples follow
    if partial {
        w.put(n as u32, 32);
    }
    for i in 0..n {
        for ch in 0..2 {
            let x = interleaved.get(i * 2 + ch).copied().unwrap_or(0.0);
            let v = if bits == 16 { to_i16(x) } else { to_i24(x) };
            w.put(v as u32, bits);
        }
    }
    w.put(7, 3); // end of frame
    w.finish()
}

/// The `alac` box: ALACSpecificConfig (frame length, depth, tuning defaults, rate).
fn alac_config(rate: u32, bits: u32) -> Vec<u8> {
    let mut c = Vec::with_capacity(24);
    c.extend((BLOCK as u32).to_be_bytes()); // frame length
    c.push(0); // compatible version
    c.push(bits as u8);
    c.extend([40, 10, 14]); // pb, mb, kb: the standard defaults
    c.push(CHANNELS as u8);
    c.extend(255u16.to_be_bytes()); // max run
    c.extend(alac_max_frame(bits).to_be_bytes());
    c.extend((rate.saturating_mul(CHANNELS * bits)).to_be_bytes()); // average bit rate
    c.extend(rate.to_be_bytes());
    full(b"alac", 0, 0, &c)
}

// --- MP4 -----------------------------------------------------------------------------------

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + body.len());
    b.extend(((8 + body.len()) as u32).to_be_bytes());
    b.extend(kind);
    b.extend(body);
    b
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + body.len());
    v.push(version);
    v.extend(&flags.to_be_bytes()[1..]);
    v.extend(body);
    bx(kind, &v)
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn init_segment(rate: u32, variant: Variant) -> Vec<u8> {
    let brand: &[u8; 4] = match variant.brand {
        Brand::Iso6 => b"iso6",
        Brand::Iso5 => b"iso5",
        Brand::Mp42 => b"mp42",
    };
    let ftyp = bx(b"ftyp", &cat(&[brand, &0u32.to_be_bytes(), brand, b"mp41"]));

    let mut mvhd = Vec::new();
    mvhd.extend([0u8; 8]); // creation, modification
    mvhd.extend(1000u32.to_be_bytes()); // timescale
    mvhd.extend(0u32.to_be_bytes()); // duration
    mvhd.extend(0x0001_0000u32.to_be_bytes()); // rate 1.0
    mvhd.extend(0x0100u16.to_be_bytes()); // volume 1.0
    mvhd.extend([0u8; 10]);
    for m in MATRIX {
        mvhd.extend(m.to_be_bytes());
    }
    mvhd.extend([0u8; 24]); // pre_defined
    mvhd.extend(2u32.to_be_bytes()); // next track id
    let mvhd = full(b"mvhd", 0, 0, &mvhd);

    let mut tkhd = Vec::new();
    tkhd.extend([0u8; 8]);
    tkhd.extend(1u32.to_be_bytes()); // track id
    tkhd.extend([0u8; 4]);
    tkhd.extend(0u32.to_be_bytes()); // duration
    tkhd.extend([0u8; 8]);
    tkhd.extend(0u16.to_be_bytes()); // layer
    tkhd.extend(0u16.to_be_bytes()); // alternate group
    tkhd.extend(0x0100u16.to_be_bytes()); // volume
    tkhd.extend([0u8; 2]);
    for m in MATRIX {
        tkhd.extend(m.to_be_bytes());
    }
    tkhd.extend([0u8; 8]); // width, height
    let tkhd = full(b"tkhd", 0, 0x3, &tkhd);

    let mut mdhd = Vec::new();
    mdhd.extend([0u8; 8]);
    mdhd.extend(rate.to_be_bytes()); // timescale = sample rate
    mdhd.extend(0u32.to_be_bytes());
    mdhd.extend(0x55c4u16.to_be_bytes()); // language "und"
    mdhd.extend([0u8; 2]);
    let mdhd = full(b"mdhd", 0, 0, &mdhd);

    let hdlr = full(
        b"hdlr",
        0,
        0,
        &cat(&[&[0u8; 4], b"soun", &[0u8; 12], b"SoundHandler\0"]),
    );
    let smhd = full(b"smhd", 0, 0, &[0u8; 4]);
    let dref = full(
        b"dref",
        0,
        0,
        &cat(&[&1u32.to_be_bytes(), &full(b"url ", 0, 1, &[])]),
    );
    let dinf = bx(b"dinf", &dref);

    let mut dfla_body = vec![0x80, 0, 0, 34]; // last block, STREAMINFO, length 34
    dfla_body.extend(streaminfo(rate, variant));
    let dfla = full(b"dfLa", 0, 0, &dfla_body);
    let high = rate > 65_535;
    let v1 = high && variant.high_rate == HighRate::Srat;
    let mut entry = Vec::new();
    entry.extend([0u8; 6]);
    entry.extend(1u16.to_be_bytes()); // data reference index
    entry.extend((if v1 { 1u16 } else { 0 }).to_be_bytes()); // entry version
    entry.extend([0u8; 6]);
    entry.extend((CHANNELS as u16).to_be_bytes());
    entry.extend((variant.bits() as u16).to_be_bytes()); // must match STREAMINFO (browsers check)
    entry.extend([0u8; 4]);
    let field_rate = if !high {
        rate
    } else {
        match variant.high_rate {
            HighRate::Zero => 0,
            // Largest integer divisor of the rate that fits in 16 bits.
            HighRate::Srat | HighRate::Divisor => (2..=16)
                .map(|d| rate / d)
                .find(|r| *r <= 65_535 && r * (rate / r) == rate)
                .unwrap_or(0),
        }
    };
    entry.extend((field_rate << 16).to_be_bytes());
    if v1 {
        entry.extend(full(b"srat", 0, 0, &rate.to_be_bytes()));
    }
    let audio_entry = if variant.alac {
        entry.extend(alac_config(rate, variant.bits()));
        bx(b"alac", &entry)
    } else {
        entry.extend(dfla);
        bx(b"fLaC", &entry)
    };
    let stsd = full(b"stsd", 0, 0, &cat(&[&1u32.to_be_bytes(), &audio_entry]));
    let empty = 0u32.to_be_bytes();
    let stbl = bx(
        b"stbl",
        &cat(&[
            &stsd,
            &full(b"stts", 0, 0, &empty),
            &full(b"stsc", 0, 0, &empty),
            &full(b"stsz", 0, 0, &[0u8; 8]),
            &full(b"stco", 0, 0, &empty),
        ]),
    );
    let minf = bx(b"minf", &cat(&[&smhd, &dinf, &stbl]));
    let mdia = bx(b"mdia", &cat(&[&mdhd, &hdlr, &minf]));
    let trak = bx(b"trak", &cat(&[&tkhd, &mdia]));
    let trex = full(
        b"trex",
        0,
        0,
        &cat(&[&1u32.to_be_bytes(), &1u32.to_be_bytes(), &[0u8; 12]]),
    );
    let mvex = bx(b"mvex", &trex);
    let moov = bx(b"moov", &cat(&[&mvhd, &trak, &mvex]));
    cat(&[&ftyp, &moov])
}

fn media_segment(sequence: u32, decode_time: u64, samples: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mfhd = full(b"mfhd", 0, 0, &sequence.to_be_bytes());
    let tfhd = full(b"tfhd", 0, 0x02_0000, &1u32.to_be_bytes()); // default-base-is-moof
    let tfdt = full(b"tfdt", 1, 0, &decode_time.to_be_bytes());
    let build_trun = |data_offset: i32| {
        let mut t = Vec::new();
        t.extend((samples.len() as u32).to_be_bytes());
        t.extend(data_offset.to_be_bytes());
        for (duration, bytes) in samples {
            t.extend(duration.to_be_bytes());
            t.extend((bytes.len() as u32).to_be_bytes());
        }
        // data-offset, sample-duration and sample-size present
        full(b"trun", 0, 0x00_0301, &t)
    };
    let moof_len = |trun: &[u8]| 8 + mfhd.len() + 8 + tfhd.len() + tfdt.len() + trun.len();
    let provisional = build_trun(0);
    let offset = (moof_len(&provisional) + 8) as i32; // data starts after moof + mdat header
    let trun = build_trun(offset);
    let traf = bx(b"traf", &cat(&[&tfhd, &tfdt, &trun]));
    let moof = bx(b"moof", &cat(&[&mfhd, &traf]));
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|(_, b)| b.iter().copied())
        .collect();
    let mdat = bx(b"mdat", &data);
    cat(&[&moof, &mdat])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, rate: u32, phase: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let t = (i + phase) as f32 / rate as f32;
                let l = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                [l, -l * 0.5]
            })
            .collect()
    }

    /// Walk the top-level boxes: (type, size).
    fn boxes(b: &[u8]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= b.len() {
            let size = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as usize;
            out.push((String::from_utf8_lossy(&b[i + 4..i + 8]).into_owned(), size));
            assert!(size >= 8 && i + size <= b.len(), "box overruns buffer");
            i += size;
        }
        assert_eq!(i, b.len(), "trailing bytes");
        out
    }

    #[test]
    fn crcs_match_the_flac_reference_values() {
        assert_eq!(crc8(b"123456789"), 0xf4);
        assert_eq!(crc16(b"123456789"), 0xfee8);
    }

    #[test]
    fn sample_numbers_use_flacs_utf8_coding() {
        let enc = |v| {
            let mut o = Vec::new();
            utf8_number(v, &mut o);
            o
        };
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(0x7f), [0x7f]);
        assert_eq!(enc(0x80), [0xc2, 0x80]);
        assert_eq!(enc(0x7ff), [0xdf, 0xbf]);
        assert_eq!(enc(0x800), [0xe0, 0xa0, 0x80]);
        assert_eq!(enc(0xffff), [0xef, 0xbf, 0xbf]);
        assert_eq!(enc(0x10000), [0xf0, 0x90, 0x80, 0x80]);
        assert_eq!(enc((1 << 36) - 1).len(), 7);
    }

    #[test]
    fn segments_are_well_formed_boxes() {
        let mut s = Segmenter::new(44_100);
        let init = s.init_segment();
        assert_eq!(
            boxes(&init)
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>(),
            ["ftyp", "moov"]
        );
        s.push(&sine(BLOCK * 3 + 100, 44_100, 0));
        let seg = s.segment().unwrap();
        assert_eq!(
            boxes(&seg)
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>(),
            ["moof", "mdat"]
        );
        assert_eq!(
            s.pending_frames(),
            100,
            "partial frame kept for the next segment"
        );
        assert!(s.segment().is_none());
        let last = s.flush().unwrap();
        boxes(&last);
        assert_eq!(s.pending_frames(), 0);
    }

    /// Regression: Chromium refuses the stream if the sample entry's bit depth differs from
    /// STREAMINFO's ("FLAC AudioSampleEntry sample size mismatches ... STREAMINFO").
    #[test]
    fn sample_entry_depth_matches_streaminfo() {
        for (code, bits) in [(0x000, 24), (0x200, 16), (0x300, 16)] {
            let v = Variant::from_code(code);
            let mut s = Segmenter::new(96_000);
            s.set_variant(v);
            let init = s.init_segment();
            let at = init
                .windows(4)
                .position(|w| w == b"fLaC")
                .expect("fLaC entry")
                + 4;
            // 6 reserved, 2 data ref index, 8 reserved, 2 channels, then 2 bytes sample size.
            let depth = u16::from_be_bytes([init[at + 18], init[at + 19]]);
            let info = streaminfo(96_000, v);
            let packed = u64::from_be_bytes(info[10..18].try_into().unwrap());
            assert_eq!(depth as u64, ((packed >> 36) & 0x1f) + 1);
            assert_eq!(depth, bits);
            let min = u16::from_be_bytes([info[0], info[1]]) as usize;
            assert_eq!(min, if v.fixed_blocks { BLOCK } else { MIN_BLOCK });
        }
    }

    #[test]
    fn variants_write_the_documented_fields() {
        let entry_at = |init: &[u8]| init.windows(4).position(|w| w == b"fLaC").unwrap() + 4;
        let rate_field = |init: &[u8]| {
            let at = entry_at(init) + 24;
            u32::from_be_bytes(init[at..at + 4].try_into().unwrap()) >> 16
        };
        let mut s = Segmenter::new(96_000);
        assert_eq!(
            rate_field(&s.init_segment()),
            0,
            "default: 0 for high rates"
        );
        s.set_variant(Variant {
            brand: Brand::Iso5,
            high_rate: HighRate::Srat,
            ..Variant::default()
        });
        let init = s.init_segment();
        assert_eq!(&init[8..12], b"iso5");
        assert_eq!(
            u16::from_be_bytes([init[entry_at(&init) + 8], init[entry_at(&init) + 9]]),
            1,
            "entry version 1"
        );
        assert_eq!(rate_field(&init), 48_000);
        let srat = init
            .windows(4)
            .position(|w| w == b"srat")
            .expect("srat box");
        assert_eq!(
            u32::from_be_bytes(init[srat + 8..srat + 12].try_into().unwrap()),
            96_000
        );
        s.set_variant(Variant {
            brand: Brand::Mp42,
            high_rate: HighRate::Divisor,
            ..Variant::default()
        });
        let init = s.init_segment();
        assert_eq!(&init[8..12], b"mp42");
        assert_eq!(rate_field(&init), 48_000);
        assert!(!init.windows(4).any(|w| w == b"srat"));
        // Rates that fit are always written as they are.
        let mut s = Segmenter::new(44_100);
        s.set_variant(Variant {
            brand: Brand::Iso6,
            high_rate: HighRate::Srat,
            ..Variant::default()
        });
        assert_eq!(rate_field(&s.init_segment()), 44_100);
        assert!(!s.init_segment().windows(4).any(|w| w == b"srat"));
        for code in (0..64)
            .chain(0x100..0x140)
            .chain(0x200..0x240)
            .chain(0x300..0x340)
            .chain(0x400..0x440)
            .chain(0x600..0x640)
        {
            let v = Variant::from_code(code);
            assert_eq!(Variant::from_code(v.code()), v);
        }
        assert_eq!(Variant::from_code(0), Variant::default());
    }

    #[cfg(feature = "player")]
    #[test]
    fn every_variant_still_decodes_bit_exactly() {
        for code in [
            0x00, 0x01, 0x02, 0x10, 0x11, 0x20, 0x100, 0x200, 0x300, 0x311, 0x400, 0x600, 0x411,
            0x621,
        ] {
            let rate = 96_000;
            let mut s = Segmenter::new(rate);
            let v = Variant::from_code(code);
            s.set_variant(v);
            let pcm = sine(BLOCK * 2 + 777, rate, 0);
            let mut file = s.init_segment();
            s.push(&pcm);
            file.extend(s.flush().unwrap());
            let (got_rate, got) = decode_all(file, "mp4");
            assert_eq!(got_rate, rate, "variant {code:#x}");
            assert_eq!(
                &got[..pcm.len()],
                &expected(&pcm, v)[..],
                "variant {code:#x}"
            );
            // Fixed strategy: the short last frame is padded with silence to a full block.
            let frames = if v.fixed_blocks {
                BLOCK * 3
            } else {
                BLOCK * 2 + 777
            };
            assert_eq!(got.len(), frames * 2, "variant {code:#x}");
            assert!(got[pcm.len()..].iter().all(|&x| x == 0));
        }
    }

    #[test]
    fn alac_init_segments_describe_the_stream() {
        for (code, bits) in [(0x400, 24u8), (0x600, 16)] {
            let mut s = Segmenter::new(44_100);
            s.set_variant(Variant::from_code(code));
            let init = s.init_segment();
            assert!(!init.windows(4).any(|w| w == b"fLaC"));
            let entry = init
                .windows(4)
                .position(|w| w == b"alac")
                .expect("alac entry")
                + 4;
            assert_eq!(
                u16::from_be_bytes([init[entry + 18], init[entry + 19]]),
                u16::from(bits),
                "entry sample size"
            );
            // The nested `alac` box: version/flags, then the 24-byte config.
            let cfg = init[entry..].windows(4).position(|w| w == b"alac").unwrap() + entry + 8;
            assert_eq!(&init[cfg..cfg + 4], &(BLOCK as u32).to_be_bytes());
            assert_eq!(init[cfg + 5], bits);
            assert_eq!(init[cfg + 9], 2, "channels");
            assert_eq!(&init[cfg + 20..cfg + 24], &44_100u32.to_be_bytes());
        }
    }

    #[test]
    fn alac_frames_are_well_formed() {
        // A full frame has no length field; a short one says its length.
        let full = alac_frame(&vec![0.0; BLOCK * 2], BLOCK, 16);
        assert_eq!(full.len(), (23 + BLOCK * 2 * 16 + 3).div_ceil(8));
        assert_eq!(full[0] >> 5, 1, "channel pair element");
        let short = alac_frame(&[0.5, -0.5, 0.25, -0.25], 2, 24);
        assert_eq!(short.len(), (23usize + 32 + 2 * 2 * 24 + 3).div_ceil(8));
        // Everything fits the advertised maximum.
        assert!(
            alac_frame(&vec![1.0; BLOCK * 2], BLOCK - 1, 24).len() <= alac_max_frame(24) as usize
        );
    }

    #[test]
    fn timeline_is_continuous_across_segments_and_rate_changes() {
        let mut s = Segmenter::new(44_100);
        s.push(&sine(BLOCK * 2, 44_100, 0));
        s.segment().unwrap();
        assert_eq!(s.emitted_seconds(), (BLOCK * 2) as f64 / 44_100.0);
        let before = s.emitted_seconds();
        s.set_rate(96_000);
        assert!((s.emitted_seconds() - before).abs() < 1.0 / 44_100.0);
        s.reset();
        assert_eq!(s.emitted_seconds(), 0.0);
        s.push(&sine(100, 96_000, 0));
        s.set_time(12.5);
        assert_eq!(s.emitted_seconds(), 12.5);
        assert_eq!(s.pending_frames(), 0, "queued audio dropped");
        for bad in [f64::NAN, f64::INFINITY, -3.0] {
            s.set_time(bad);
            assert_eq!(s.emitted_seconds(), 0.0);
        }
    }

    /// A host dropping audio it queued (a partial flush) cuts the queue and the timeline
    /// exactly, and never out of range.
    #[test]
    fn pending_audio_and_timeline_can_be_cut_exactly() {
        let mut s = Segmenter::new(44_100);
        s.push(&sine(BLOCK + 10, 44_100, 0));
        let head: Vec<f32> = s.pending_pcm()[..20].to_vec();
        s.truncate_pending(10);
        assert_eq!(s.pending_frames(), 10);
        assert_eq!(s.pending_pcm(), &head[..]);
        s.truncate_pending(usize::MAX);
        assert_eq!(s.pending_frames(), 10, "never grows");
        s.push(&sine(BLOCK * 3, 44_100, 0));
        s.segment().unwrap();
        assert_eq!(s.decode_time(), (BLOCK * 3) as u64);
        s.set_decode_time(BLOCK as u64);
        assert_eq!(s.decode_time(), BLOCK as u64);
        assert_eq!(s.pending_frames(), 0, "queued audio dropped");
        s.push(&sine(BLOCK, 44_100, 0));
        let seg = s.segment().unwrap();
        assert!(!seg.is_empty());
        assert_eq!(s.decode_time(), (BLOCK * 2) as u64);
    }

    /// Decode our output with Symphonia: it must give back exactly the 24-bit samples.
    #[cfg(feature = "player")]
    fn decode_all(bytes: Vec<u8>, ext: &str) -> (u32, Vec<i32>) {
        use symphonia::core::audio::GenericAudioBufferRef;
        use symphonia::core::codecs::audio::AudioDecoderOptions;
        use symphonia::core::formats::probe::Hint;
        use symphonia::core::formats::{FormatOptions, TrackType};
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;

        let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension(ext);
        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .expect("probe");
        let track = format
            .default_track(TrackType::Audio)
            .expect("audio track")
            .clone();
        let params = track
            .codec_params
            .as_ref()
            .unwrap()
            .audio()
            .unwrap()
            .clone();
        let rate = params.sample_rate.unwrap();
        let mut dec = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .expect("decoder");
        let mut out = Vec::new();
        while let Ok(Some(p)) = format.next_packet() {
            let buf: GenericAudioBufferRef = dec.decode(&p).expect("decode");
            let mut v: Vec<i32> = Vec::new();
            buf.copy_to_vec_interleaved(&mut v);
            out.extend(v.into_iter().map(|x| x >> 8)); // s32 → 24-bit
        }
        (rate, out)
    }

    #[cfg(feature = "player")]
    fn expected_i24(pcm: &[f32]) -> Vec<i32> {
        pcm.iter().map(|&x| to_i24(x)).collect()
    }

    /// What `decode_all` (24-bit scale) gives back for this variant's sample depth.
    #[cfg(feature = "player")]
    fn expected(pcm: &[f32], v: Variant) -> Vec<i32> {
        if v.bits16 {
            pcm.iter().map(|&x| to_i16(x) << 8).collect()
        } else {
            expected_i24(pcm)
        }
    }

    #[cfg(feature = "player")]
    #[test]
    fn flac_frames_decode_bit_exactly() {
        // A plain .flac stream of our frames: "fLaC" + STREAMINFO + frames.
        for rate in [44_100, 96_000, 192_000, 22_050] {
            let pcm = sine(BLOCK * 2 + 777, rate, 3);
            let mut flac = b"fLaC".to_vec();
            flac.extend([0x80, 0, 0, 34]);
            flac.extend(streaminfo(rate, Variant::default()));
            let mut first = 0u64;
            for chunk in pcm.chunks(BLOCK * 2) {
                let n = (chunk.len() / 2).max(MIN_BLOCK);
                flac.extend(flac_frame(chunk, rate, first, n, Variant::default()));
                first += n as u64;
            }
            let (got_rate, got) = decode_all(flac, "flac");
            assert_eq!(got_rate, rate);
            assert_eq!(&got[..pcm.len()], &expected_i24(&pcm)[..], "rate {rate}");
        }
    }

    #[cfg(feature = "player")]
    #[test]
    fn fragmented_mp4_decodes_bit_exactly() {
        let rate = 96_000;
        let mut s = Segmenter::new(rate);
        let mut file = s.init_segment();
        let mut all = Vec::new();
        for i in 0..5 {
            let pcm = sine(3000 + i * 1234, rate, all.len() / 2);
            all.extend_from_slice(&pcm);
            s.push(&pcm);
            if let Some(seg) = s.segment() {
                file.extend(seg);
            }
        }
        file.extend(s.flush().unwrap());
        let (got_rate, got) = decode_all(file, "mp4");
        assert_eq!(got_rate, rate);
        assert_eq!(&got[..all.len()], &expected_i24(&all)[..]);
    }

    #[test]
    fn hostile_pcm_never_panics_and_is_clamped() {
        let mut s = Segmenter::new(48_000);
        let weird = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            2.0,
            -2.0,
            0.0,
            1e-30,
            -1.0,
        ];
        s.push(&weird.repeat(1000));
        s.push(&[0.5]); // odd length: the stray sample is ignored
        let seg = s.flush().unwrap();
        boxes(&seg);
        assert_eq!(to_i24(f32::NAN), 0);
        assert_eq!(to_i24(f32::INFINITY), 8_388_607);
        assert_eq!(to_i24(-5.0), -8_388_607);
        for rate in [0, 1, 7_350, 655_350, u32::MAX] {
            let mut s = Segmenter::new(rate);
            boxes(&s.init_segment());
            s.push(&[0.1; 64]);
            boxes(&s.flush().unwrap());
        }
    }
}
