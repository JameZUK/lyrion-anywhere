//! Robustness: untrusted input must never panic or wedge the core.
//!
//! Control bytes come from the server side of a tunnel and stream bytes from the network,
//! so every parser and state machine is driven here with arbitrary and adversarial input.
//! (Coverage-guided fuzzing via cargo-fuzz complements these deterministic tests.)

use anywhere_core::decode::{InputBuffer, StreamDecoder};
use anywhere_core::player::{Player, PlayerConfig};
use anywhere_core::proto::{ClientFrameReader, ServerFrameReader, Strm, encode_server_frame};

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
                anywhere_core::decode::DecodeStatus::Progress => {}
                _ => break,
            }
        }
    }
}

#[test]
fn player_input_is_bounded_by_back_pressure_signal() {
    // The host must stop reading when wants_stream_data() is false; verify the signal flips
    // before memory use grows past the high-water mark.
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
        assert!(pushed <= anywhere_core::player::INPUT_HIGH_WATER + chunk.len());
    }
}
