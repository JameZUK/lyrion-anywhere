//! Robustness: untrusted input must never panic or wedge the core.
//!
//! Control bytes come from the server side of a tunnel and stream bytes from the network,
//! so every parser and state machine is driven here with arbitrary and adversarial input.
//! These are seeded, deterministic tests, not coverage-guided fuzzing (no cargo-fuzz yet).

use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use anywhere_core::player::{Player, PlayerConfig};
use anywhere_core::proto::{
    ClientFrameReader, ClientMsg, ServerFrameReader, Strm, encode_server_frame,
};

/// Small deterministic PRNG (xorshift64*), so failures are reproducible from the seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn cfg() -> PlayerConfig {
    PlayerConfig {
        mac: [2, 0, 0, 0, 0, 9],
        uuid: [0; 16],
        name: "robustness".into(),
        codecs: vec!["flc".into(), "mp3".into(), "ogg".into()],
        max_sample_rate: 192_000,
        firmware: "test".into(),
    }
}

/// A syntactically valid server frame with a random opcode drawn from the real set and a
/// random (often truncated or oversized) payload.
fn random_server_frame(r: &mut Rng) -> Vec<u8> {
    const OPS: [&[u8; 4]; 8] = [
        b"strm", b"audg", b"aude", b"setd", b"serv", b"vers", b"vfdc", b"grfe",
    ];
    let op = OPS[r.below(OPS.len() as u64) as usize];
    let len = r.below(80) as usize;
    let mut payload = r.bytes(len);
    if op == b"strm" && payload.len() >= 3 && r.below(2) == 0 {
        payload[0] = b"spuqtfa"[r.below(7) as usize];
        payload[2] = b"fmoapwlu"[r.below(8) as usize];
    }
    encode_server_frame(*op, &payload)
}

#[test]
fn server_frame_reader_survives_random_bytes() {
    for seed in 1..=500u64 {
        let mut r = Rng(seed);
        let mut reader = ServerFrameReader::default();
        for _ in 0..50 {
            let n = r.below(300) as usize;
            reader.push(&r.bytes(n));
            for _ in 0..10 {
                match reader.next_msg() {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => {
                        // Oversize frame: a host drops the connection; start afresh.
                        reader = ServerFrameReader::default();
                        break;
                    }
                }
            }
        }
    }
}

#[test]
fn client_frame_reader_survives_random_bytes() {
    for seed in 1..=500u64 {
        let mut r = Rng(seed);
        let mut reader = ClientFrameReader::default();
        for _ in 0..50 {
            let n = r.below(300) as usize;
            reader.push(&r.bytes(n));
            match reader.next_frame() {
                Ok(_) => {}
                Err(_) => reader = ClientFrameReader::default(),
            }
        }
    }
}

#[test]
fn oversize_frame_length_is_rejected_not_buffered_forever() {
    let mut reader = ServerFrameReader::default();
    reader.push(&[0xff, 0xff, b's', b't', b'r', b'm']);
    assert!(
        reader.next_msg().is_err(),
        "a 64 KiB+ frame must be refused"
    );
}

#[test]
fn player_survives_random_server_frames_and_stream_bytes() {
    for seed in 1..=300u64 {
        let mut r = Rng(seed);
        let mut p = Player::new(cfg());
        p.connected(0);
        let mut t = 0u64;
        let mut out = vec![0f32; 2048];
        for _ in 0..200 {
            t += r.below(50);
            match r.below(6) {
                0 | 1 => {
                    let f = random_server_frame(&mut r);
                    let _ = p.on_control_data(&f, t);
                }
                2 => {
                    let n = r.below(4096) as usize;
                    p.on_stream_data(&r.bytes(n), t);
                }
                3 => {
                    if r.below(4) == 0 {
                        p.on_stream_end(t)
                    } else {
                        p.on_stream_headers(&r.bytes(40), t)
                    }
                }
                4 => {
                    let n = p.pull_audio(&mut out);
                    p.played(r.below(n as u64 + 1));
                }
                _ => p.tick(t),
            }
            let _ = p.take_actions();
            let _ = p.status();
        }
    }
}

#[test]
fn player_survives_clock_going_backwards() {
    let mut p = Player::new(cfg());
    p.connected(10_000);
    let mut s = Strm {
        command: b's',
        autostart: b'1',
        format: b'f',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: 1,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        replay_gain: 0,
        server_port: 9000,
        server_ip: 0,
        request: b"GET / HTTP/1.0\r\n\r\n".to_vec(),
    };
    p.on_control_data(&encode_server_frame(*b"strm", &s.encode_payload()), 10_000)
        .unwrap();
    s.command = b't';
    p.on_control_data(&encode_server_frame(*b"strm", &s.encode_payload()), 5)
        .unwrap();
    p.tick(3);
    p.tick(10_001);
    p.tick(0);
}

#[test]
fn decoder_survives_random_and_truncated_streams() {
    for seed in 1..=200u64 {
        let mut r = Rng(seed);
        let input = InputBuffer::new();
        let fmt = b"fmoap"[r.below(5) as usize];
        let mut dec = StreamDecoder::new(input.clone(), fmt);
        // Plausible magic numbers make the probe go deeper before failing.
        let magic: &[u8] = match r.below(4) {
            0 => b"fLaC",
            1 => b"ID3\x04\x00\x00\x00\x00\x00\x00",
            2 => b"OggS",
            _ => b"RIFF\x24\x00\x00\x00WAVEfmt ",
        };
        input.push(magic);
        let n = r.below(300_000) as usize;
        input.push(&r.bytes(n));
        input.set_eof();
        for _ in 0..10_000 {
            match dec.work(64, &mut |_, _| {}) {
                DecodeStatus::Progress => {}
                // A caught panic would have aborted the web player (M13).
                DecodeStatus::Failed(e) => {
                    assert_ne!(e, CAUGHT_PANIC, "seed {seed} panicked the decoder");
                    break;
                }
                _ => break,
            }
        }
    }
}

#[test]
fn player_input_is_bounded_by_back_pressure_signal() {
    // The host must stop reading when wants_stream_data() is false; verify the signal flips
    // before memory use grows past its bound.
    let mut p = Player::new(cfg());
    let s = Strm {
        command: b's',
        autostart: b'1',
        format: b'f',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: 1,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        replay_gain: 0,
        server_port: 9000,
        server_ip: 0,
        request: b"GET / HTTP/1.0\r\n\r\n".to_vec(),
    };
    p.on_control_data(&encode_server_frame(*b"strm", &s.encode_payload()), 0)
        .unwrap();
    let chunk = vec![0u8; 64 * 1024];
    let mut pushed = 0;
    while p.wants_stream_data() {
        p.on_stream_data(&chunk, 1);
        pushed += chunk.len();
        // Unprobed input may grow to the probe cap (headers can be large), never beyond.
        assert!(pushed <= anywhere_core::decode::PROBE_MAX_BYTES + chunk.len());
    }
}

/// A valid AutoEq profile used as the seed for mutation.
const AUTOEQ: &str = "Preamp: -6.4 dB\n\
Filter 1: ON LSC Fc 105 Hz Gain 5.8 dB Q 0.70\n\
Filter 2: ON PK Fc 152 Hz Gain -2.5 dB Q 0.81\n\
Filter 3: ON PK Fc 3800 Hz Gain 4.1 dB Q 3.2\n\
Filter 4: OFF NO Fc 6000 Hz Q 5\n\
Filter 5: ON HSC Fc 10000 Hz Gain -2.6 dB Q 0.70\n";

/// Accepted profiles must be within the documented bounds, and running them must give
/// finite audio.
fn check_profile(p: &anywhere_core::dsp::eq::EqProfile) {
    use anywhere_core::dsp::eq::{MAX_FILTERS, ParametricEq};
    assert!(p.filters.len() <= MAX_FILTERS);
    p.validate().expect("parser output must validate");
    let mut eq = ParametricEq::new(p.clone(), 44_100, 2).unwrap();
    let mut buf: Vec<f64> = (0..512)
        .map(|i| ((i * 31) % 17) as f64 / 17.0 - 0.5)
        .collect();
    eq.process(&mut buf);
    assert!(buf.iter().all(|x| x.is_finite()));
}

#[test]
fn autoeq_parser_survives_random_and_mutated_profiles() {
    use anywhere_core::dsp::eq::parse_autoeq;
    let mut r = Rng(0xa5a5_1234);
    let iters = if cfg!(debug_assertions) {
        20_000
    } else {
        200_000
    };
    let alphabet = b"Filter PreampONOFFPKLSCHSCLPHPNOFcGainQHzdB:0123456789.-+ \n\r\t#eExinfNaN";
    for i in 0..iters {
        let text: Vec<u8> = match i % 3 {
            // Random bytes from a profile-like alphabet.
            0 => (0..r.below(400))
                .map(|_| alphabet[r.below(alphabet.len() as u64) as usize])
                .collect(),
            // Byte mutations of a valid profile.
            1 => {
                let mut t = AUTOEQ.as_bytes().to_vec();
                for _ in 0..1 + r.below(6) {
                    let k = r.below(t.len() as u64) as usize;
                    match r.below(3) {
                        0 => t[k] = alphabet[r.below(alphabet.len() as u64) as usize],
                        1 => {
                            t.remove(k);
                        }
                        _ => t.insert(k, r.next() as u8),
                    }
                }
                t
            }
            // Raw random bytes.
            _ => {
                let n = r.below(300) as usize;
                r.bytes(n)
            }
        };
        let s = String::from_utf8_lossy(&text);
        if let Ok(p) = parse_autoeq(&s) {
            check_profile(&p);
        }
    }
    check_profile(&parse_autoeq(AUTOEQ).unwrap());
}

#[test]
fn dsp_chain_survives_hostile_samples() {
    use anywhere_core::dsp::chain::{Chain, ChainConfig};
    use anywhere_core::dsp::crossfeed::CrossfeedPreset;
    use anywhere_core::dsp::eq::parse_autoeq;
    use anywhere_core::dsp::limiter::LimiterConfig;
    let cfg = ChainConfig {
        preamp_db: 12.0,
        eq: Some(parse_autoeq(AUTOEQ).unwrap()),
        crossfeed: Some(CrossfeedPreset::Natural.config()),
        limiter: Some(LimiterConfig::default()),
        ..Default::default()
    };
    let mut r = Rng(77);
    let specials = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        -f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
        0.0,
        1.0,
        -1.0,
    ];
    for rate in [8_000, 44_100, 192_000, 768_000] {
        let mut chain = Chain::new(cfg.clone(), rate, 2).unwrap();
        let mut q = vec![0i32; 1024];
        for _ in 0..50 {
            let n = 1 + r.below(1023) as usize;
            let mut buf: Vec<f64> = (0..n)
                .map(|_| {
                    if r.below(4) == 0 {
                        specials[r.below(specials.len() as u64) as usize]
                    } else {
                        (r.next() as i64) as f64 / i64::MAX as f64
                    }
                })
                .collect();
            chain.process(&mut buf);
            // A trailing partial frame is left untouched; every whole frame is finite.
            let whole = n - n % 2;
            assert!(
                buf[..whole].iter().all(|x| x.is_finite()),
                "non-finite output at {rate} Hz"
            );
            chain.quantise(&buf, Some(24), 16, &mut q[..n]);
        }
    }
}

/// A small, valid 16-bit stereo WAV file.
fn wav16(rate: u32, frames: usize) -> Vec<u8> {
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
    for i in 0..frames * 2 {
        v.extend_from_slice(&(((i % 200) as i16 - 100) * 100).to_le_bytes());
    }
    v
}

/// Decode `file` through the streaming decoder (as the player does) to the end.
fn stream_decode(file: &[u8], fmt: u8) -> DecodeStatus {
    let input = InputBuffer::new();
    input.push(file);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, fmt);
    let mut last = DecodeStatus::NeedData;
    for _ in 0..10_000 {
        last = dec.work(64, &mut |_, s| assert!(s.iter().all(|x| x.is_finite())));
        if last != DecodeStatus::Progress {
            break;
        }
    }
    last
}

/// The streaming decoder's message when it caught a panic (native builds only).
const CAUGHT_PANIC: &str = anywhere_core::decode::STREAM_DECODER_PANICKED;

/// The safety net itself (M13): a panic caught inside the streaming decoder is reported as
/// a failure with its own message, never as progress or a normal end, and the decoder stays
/// failed afterwards. (The sink runs inside the guarded call, so a panicking sink stands in
/// for a panicking demuxer.)
#[test]
fn a_caught_panic_is_reported_as_a_failure() {
    let input = InputBuffer::new();
    input.push(&wav16(8_000, 4_000));
    input.set_eof();
    let mut dec = StreamDecoder::new(input, b'p');
    let mut status = DecodeStatus::Progress;
    for _ in 0..10_000 {
        status = dec.work(64, &mut |_, _| panic!("a hostile stream"));
        if status != DecodeStatus::Progress {
            break;
        }
    }
    assert_eq!(status, DecodeStatus::Failed(CAUGHT_PANIC.to_string()));
    assert_eq!(
        dec.work(64, &mut |_, _| {}),
        DecodeStatus::Failed(CAUGHT_PANIC.to_string()),
        "still failed"
    );
}

/// Regression: hostile WAV header fields through the streaming decoder (the player's path,
/// shared with the web player). Symphonia's WAV reader overflowed on some of these in debug
/// builds; they must be refused *before* the reader sees them, because WebAssembly builds
/// abort on a panic. So the native safety net (a caught panic) must never be what saved us.
#[test]
fn stream_decoder_survives_hostile_wav_headers() {
    let base = wav16(8_000, 4_000);
    assert_eq!(stream_decode(&base, b'p'), DecodeStatus::Finished);
    let mut r = Rng(7);
    let mut refused = 0;
    for i in 0..6_000 {
        let mut b = base.clone();
        for k in [4usize, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
            if k + 4 <= b.len() && r.below(2) == 0 {
                let v = match r.below(4) {
                    0 => u32::MAX,
                    1 => 0,
                    2 => 1 << r.below(32),
                    _ => r.next() as u32,
                };
                b[k..k + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        if i % 3 == 0 {
            // An extensible format chunk with a hostile channel mask.
            b[16..20].copy_from_slice(&40u32.to_le_bytes());
            b[20..22].copy_from_slice(&0xfffeu16.to_le_bytes());
            let mut ext = vec![22u8, 0, 16, 0];
            ext.extend_from_slice(&(r.next() as u32).to_le_bytes());
            ext.extend_from_slice(&[
                1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
            ]);
            b.splice(36..36, ext);
        }
        if i % 5 == 0 {
            // Junk first: the prober scans for the marker.
            let n = 1 + r.below(64) as usize;
            b.splice(0..0, r.bytes(n));
        }
        let fmt = b"pfmo"[r.below(4) as usize];
        if let DecodeStatus::Failed(e) = stream_decode(&b, fmt) {
            assert_ne!(e, CAUGHT_PANIC, "case {i} reached the reader's overflow");
            refused += 1;
        }
    }
    assert!(
        refused > 100,
        "the mutations should hit refusals ({refused})"
    );
}

/// The exact header that overflowed: 16,384 channels × 4 bytes in a `u16`.
#[test]
fn wav_channel_count_overflow_is_refused_before_parsing() {
    let mut b = wav16(44_100, 100);
    b[22..24].copy_from_slice(&16_384u16.to_le_bytes());
    b[34..36].copy_from_slice(&32u16.to_le_bytes());
    assert_eq!(
        anywhere_core::decode::check_wav_headers(&b),
        Err("wav: unsupported channel count")
    );
    match stream_decode(&b, b'p') {
        DecodeStatus::Failed(e) => assert!(e.contains("channel count"), "{e}"),
        other => panic!("{other:?}"),
    }
    // RIFF length under 4 (the reader subtracts 4), and a mask needing a 32-bit shift.
    let mut short = wav16(44_100, 10);
    short[4..8].copy_from_slice(&3u32.to_le_bytes());
    assert!(anywhere_core::decode::check_wav_headers(&short).is_err());
    // A sane file passes, with junk before it or not.
    let ok = wav16(44_100, 10);
    assert_eq!(anywhere_core::decode::check_wav_headers(&ok), Ok(()));
    let mut junk = b"RIFFxxxxNOPE".to_vec();
    junk.extend_from_slice(&ok);
    assert_eq!(anywhere_core::decode::check_wav_headers(&junk), Ok(()));
    // Truncated anywhere: never a panic in the checker itself.
    for n in 0..ok.len().min(60) {
        let _ = anywhere_core::decode::check_wav_headers(&ok[..n]);
    }
}

/// The same with read-ahead mode on (rolling cache), adding the host calls it brings: the
/// network stream received, spooled byte counts, the link going up and down, underruns,
/// stall recovery and skips.
#[test]
fn read_ahead_player_survives_random_frames_and_host_calls() {
    for seed in 1..=200u64 {
        let mut r = Rng(seed);
        let mut p = Player::new(cfg());
        p.set_ready_when_received(true);
        p.set_ready_after_ms(r.below(30_000));
        p.connected(0);
        let mut t = 0u64;
        let mut out = vec![0f32; 2048];
        for _ in 0..200 {
            t += r.below(50);
            match r.below(12) {
                0 | 1 => {
                    let f = random_server_frame(&mut r);
                    let _ = p.on_control_data(&f, t);
                }
                2 | 3 => {
                    let n = r.below(4096) as usize;
                    p.on_stream_data(&r.bytes(n), t);
                }
                4 => match r.below(3) {
                    0 => p.on_stream_end(t),
                    1 => p.on_stream_error(t),
                    _ => p.stream_received(t),
                },
                5 => p.stream_bytes_received(r.next()),
                6 => p.set_link_up(r.below(2) == 0, t),
                7 => p.output_underrun(),
                8 => {
                    let _ = p.resume_after_stall(r.below(2) == 0, t);
                }
                9 => {
                    p.skip_audio(r.next());
                }
                10 => {
                    let n = p.pull_audio(&mut out);
                    p.played(r.below(n as u64 + 1));
                }
                _ => p.tick(t),
            }
            let _ = p.take_actions();
            let _ = p.status();
            let _ = p.open_stream();
            let _ = p.wants_stream_data();
            let _ = p.stall_ms(t);
            let _ = p.trim();
        }
    }
}

/// The FLAC check that decides whether a download may stand in for a stream: random bytes,
/// a FLAC marker over junk, and headers with bogus lengths never panic, and never pass.
#[test]
fn flac_verification_survives_random_and_hostile_files() {
    use anywhere_core::decode::verify_flac;
    use std::io::Cursor;
    let mut r = Rng(0x00c0_ffee);
    let check = |bytes: Vec<u8>| verify_flac(Box::new(Cursor::new(bytes)));
    for _ in 0..300 {
        let n = r.below(4096) as usize;
        assert!(check(r.bytes(n)).is_err());
        let mut marked = b"fLaC".to_vec();
        marked.extend(r.bytes(n));
        assert!(!check(marked).is_ok_and(|c| c.md5_ok));
    }
    // A STREAMINFO with an MD5 but no audio, one whose length runs far past the end, and
    // one followed by junk after a frame sync: never verified.
    let mut si = b"fLaC\x80\x00\x00\x22".to_vec();
    si.extend([0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0]);
    si.extend(((44_100u64 << 44) | (1 << 41) | (15 << 36) | 1000).to_be_bytes());
    si.extend([0xab; 16]);
    assert!(!check(si.clone()).is_ok_and(|c| c.md5_ok));
    let mut long = si.clone();
    long[5..8].copy_from_slice(&[0xff, 0xff, 0xff]);
    assert!(!check(long).is_ok_and(|c| c.md5_ok));
    let mut junk_frames = si;
    junk_frames.extend([0xff, 0xf8]);
    junk_frames.extend(r.bytes(20_000));
    assert!(!check(junk_frames).is_ok_and(|c| c.md5_ok));
}

/// `HELO` and `STAT` payloads come from clients (through the gateway): random and
/// truncated bytes never panic, short ones are refused, and what parses re-encodes to the
/// same fixed fields.
#[test]
fn helo_and_stat_parsers_survive_random_payloads() {
    use anywhere_core::proto::{Helo, Stat};
    let mut r = Rng(0x0e10_57a7);
    for _ in 0..20_000 {
        let n = r.below(120) as usize;
        let p = r.bytes(n);
        match Helo::parse(&p) {
            Ok(h) => {
                assert!(n >= 36);
                let again = ClientMsg::Helo(h.clone()).encode();
                assert_eq!(&again[8..8 + 36], &p[..36], "fixed fields round-trip");
            }
            Err(_) => assert!(n < 36),
        }
        match Stat::parse(&p) {
            Ok(s) => {
                assert!(n >= Stat::LEN);
                assert_eq!(s.encode_payload(), p[..Stat::LEN]);
            }
            Err(_) => assert!(n < Stat::LEN),
        }
    }
}

/// The `strm` request check the gateway enforces, on random bytes and on mutations of a
/// good request: never panics, and anything accepted is exactly this player's stream.
#[test]
fn stream_request_check_survives_random_and_mutated_requests() {
    use anywhere_core::proto::{direct_stream_target, validate_stream_request};
    let mac = [2, 0, 0, 0, 0, 9];
    let want = "/stream.mp3?player=02:00:00:00:00:09";
    let good = format!("GET {want} HTTP/1.0\r\n\r\n").into_bytes();
    let server = std::net::IpAddr::from([10, 0, 0, 10]);
    let mut r = Rng(0x5747_e4a1);
    for round in 0..20_000 {
        let req = if round % 2 == 0 {
            let n = r.below(200) as usize;
            r.bytes(n)
        } else {
            let mut b = good.clone();
            for _ in 0..1 + r.below(3) {
                let i = r.below(b.len() as u64) as usize;
                match r.below(3) {
                    0 => b[i] = r.next() as u8,
                    1 => b.truncate(i),
                    _ => b.insert(i, b" /%:?&\r\n\xff"[r.below(9) as usize]),
                }
                if b.is_empty() {
                    break;
                }
            }
            b
        };
        if let Ok(path) = validate_stream_request(&req, &mac) {
            assert_eq!(path, want);
            let line = req.split(|b| *b == b'\r').next().unwrap();
            let line = std::str::from_utf8(line).unwrap();
            assert!(line.starts_with(&format!("GET {want} HTTP/1.")), "{line:?}");
        }
        let ip = (r.next() as u32).to_be_bytes();
        let ip = if r.below(2) == 0 { [0; 4] } else { ip };
        if let Ok(addr) = direct_stream_target(ip, r.next() as u16, &req, &mac, server) {
            assert_eq!(addr.ip(), server);
        }
    }
}

/// MAC addresses are parsed from configuration and from LMS answers: arbitrary text never
/// panics, and whatever parses formats back to six well-formed pairs.
#[test]
fn mac_parser_survives_arbitrary_text() {
    use anywhere_core::proto::{format_mac, parse_mac};
    let alphabet: Vec<char> = "0123456789abcdefABCDEF:-+ xé\u{0}".chars().collect();
    let mut r = Rng(0x00ac_0ddd);
    for _ in 0..20_000 {
        let n = r.below(30) as usize;
        let s: String = if r.below(4) == 0 {
            String::from_utf8_lossy(&r.bytes(n)).into_owned()
        } else {
            (0..n)
                .map(|_| alphabet[r.below(alphabet.len() as u64) as usize])
                .collect()
        };
        if let Some(mac) = parse_mac(&s) {
            let f = format_mac(&mac);
            assert_eq!(f.len(), 17);
            assert_eq!(parse_mac(&f), Some(mac));
        }
    }
}

/// `OpusHead` comes from the stream: random headers, and mutations of a valid one, never
/// panic, and what is accepted is mono or stereo with a consistent mapping.
#[cfg(feature = "opus")]
#[test]
fn opus_head_parser_survives_random_and_mutated_headers() {
    use anywhere_core::opus::OpusHead;
    let mut good = b"OpusHead".to_vec();
    good.extend([1, 2, 0x38, 0x01, 0x80, 0xbb, 0, 0, 0, 0, 0]);
    assert!(OpusHead::parse(&good).is_ok());
    let mut r = Rng(0x0905_4ead);
    for round in 0..20_000 {
        let b = if round % 3 == 0 {
            let mut b = b"OpusHead".to_vec();
            let n = r.below(40) as usize;
            b.extend(r.bytes(n));
            b
        } else if round % 3 == 1 {
            let mut b = good.clone();
            for _ in 0..1 + r.below(3) {
                let i = 8 + r.below((b.len() - 8) as u64) as usize;
                b[i] = r.next() as u8;
            }
            if r.below(4) == 0 {
                b.truncate(r.below(b.len() as u64) as usize);
            }
            b
        } else {
            let n = r.below(40) as usize;
            r.bytes(n)
        };
        if let Ok(h) = OpusHead::parse(&b) {
            assert!(matches!(h.channels, 1 | 2));
            assert_eq!(h.mapping.len(), h.channels as usize);
            assert!(h.streams >= 1 && h.coupled <= h.streams);
            assert!(h.output_gain_db.is_finite());
        }
    }
}
