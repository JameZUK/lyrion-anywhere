//! How fast the stream decoder runs on one core, for docs/PERFORMANCE.md.
//!
//! `cargo run --release -p anywhere-core --example decode_speed -- FILE FORMAT`
//! where FORMAT is the Slimproto format code (`f` FLAC, `m` MP3, `o` Ogg Vorbis, `u` Opus).
//! The whole file is pushed first, so this measures decoding, not the network. With a third
//! argument, the decoded audio is also written there (raw little-endian f32, interleaved).

use anywhere_core::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (path, format, dump) = match &args[..] {
        [_, path, format] => (path, format, None),
        [_, path, format, dump] => (path, format, Some(dump)),
        _ => {
            eprintln!("usage: decode_speed FILE FORMAT [RAW_F32_OUT]");
            std::process::exit(2);
        }
    };
    let bytes = std::fs::read(path).expect("read the file");
    let format = format.as_bytes()[0];
    let mut best = f64::MAX;
    let mut audio_secs = 0.0;
    let mut pcm: Vec<u8> = Vec::new();
    for run in 0..5 {
        let input = InputBuffer::new();
        input.push(&bytes);
        input.set_eof();
        let mut dec = StreamDecoder::new(input, format);
        let (mut samples, mut rate) = (0u64, 0u32);
        let start = Instant::now();
        loop {
            match dec.work(64, &mut |r, s| {
                rate = r;
                samples += s.len() as u64;
                if run == 0 && dump.is_some() {
                    pcm.extend(s.iter().flat_map(|v| v.to_le_bytes()));
                }
            }) {
                DecodeStatus::Progress | DecodeStatus::NeedData => {}
                DecodeStatus::Finished => break,
                DecodeStatus::Failed(e) => panic!("{e}"),
            }
        }
        best = best.min(start.elapsed().as_secs_f64());
        // The sink gets interleaved stereo.
        audio_secs = samples as f64 / 2.0 / f64::from(rate);
    }
    if let Some(dump) = dump {
        std::fs::write(dump, &pcm).expect("write the decoded audio");
    }
    println!(
        "{path}: {audio_secs:.1} s of audio decoded in {:.3} s (best of 5): {:.0}x real time",
        best,
        audio_secs / best
    );
}
