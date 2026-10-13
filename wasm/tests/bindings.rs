//! Binding tests, run in Node with `wasm-pack test --node wasm`.
#![cfg(target_arch = "wasm32")]

use anywhere_core::proto::{Strm, encode_server_frame};
use anywhere_wasm::WasmPlayer;
use js_sys::Reflect;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::wasm_bindgen_test;

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap()
}

fn strm_s(format: u8) -> Vec<u8> {
    let s = Strm {
        command: b's',
        autostart: b'1',
        format,
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
        request: b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n".to_vec(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

#[wasm_bindgen_test]
fn rejects_invalid_mac() {
    assert!(WasmPlayer::new("not-a-mac", "x", "flc").is_err());
    assert!(WasmPlayer::new("02:00:00:00:00", "x", "flc").is_err());
}

#[wasm_bindgen_test]
fn connect_sends_helo() {
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc,mp3").unwrap();
    p.connected(0.0);
    let actions = p.take_actions();
    assert_eq!(actions.length(), 1);
    let a = actions.get(0);
    assert_eq!(get(&a, "type").as_string().unwrap(), "send");
    let bytes = js_sys::Uint8Array::new(&get(&a, "bytes")).to_vec();
    assert_eq!(&bytes[0..4], b"HELO");
    let caps = String::from_utf8_lossy(&bytes[8 + 36..]).into_owned();
    assert!(caps.contains("flc,mp3"), "{caps}");
}

#[wasm_bindgen_test]
fn strm_yields_open_with_gateway_stream_number() {
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc").unwrap();
    p.on_control(&strm_s(b'w'), 1.0).unwrap(); // unsupported codec still counts as #1
    p.on_control(&strm_s(b'f'), 2.0).unwrap();
    let actions = p.take_actions();
    let opens: Vec<f64> = (0..actions.length())
        .map(|i| actions.get(i))
        .filter(|a| get(a, "type").as_string().as_deref() == Some("open"))
        .map(|a| get(&a, "stream").as_f64().unwrap())
        .collect();
    assert_eq!(opens, vec![2.0]);
}

#[wasm_bindgen_test]
fn garbage_control_data_is_an_error_not_a_panic() {
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc").unwrap();
    assert!(p.on_control(&[0xff, 0xff, 1, 2, 3, 4], 0.0).is_err());
    assert!(
        p.on_control(&[0x00, 0x06, b's', b't', b'r', b'm', 1, 2], 0.0)
            .is_err()
    );
}

#[wasm_bindgen_test]
fn status_is_valid_json_and_pull_is_safe_when_idle() {
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc").unwrap();
    let v: serde_json::Value = serde_json::from_str(&p.status_json()).unwrap();
    assert_eq!(v["output"], "Stopped");
    let mut buf = vec![0f32; 256];
    assert_eq!(p.pull(&mut buf), 0);
    p.played(-5.0); // a confused host must not panic
    p.played(1e12);
    assert_eq!(p.output_rate(), 0);
}

#[wasm_bindgen_test]
fn open_stream_reports_number_and_offset() {
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc").unwrap();
    assert!(p.open_stream().is_none());
    p.on_control(&strm_s(b'f'), 1.0).unwrap();
    p.on_stream_data(&[0u8; 777], 2.0);
    assert_eq!(p.open_stream().unwrap(), vec![1.0, 777.0]);
    p.on_stream_end(3.0);
    assert!(p.open_stream().is_none());
}

#[wasm_bindgen_test]
fn segmenter_produces_init_and_media_segments() {
    let mut s = anywhere_wasm::WasmSegmenter::new(44_100);
    let init = s.init_segment();
    assert_eq!(&init[4..8], b"ftyp");
    assert!(s.segment().is_none(), "nothing queued yet");
    s.push(&vec![0.25f32; 4096 * 2 * 2 + 10]);
    let seg = s.segment().expect("two whole frames");
    assert_eq!(&seg[4..8], b"moof");
    assert_eq!(s.pending_frames(), 5);
    assert!(s.flush().is_some());
    assert!(s.emitted_seconds() > 0.18);
    s.set_rate(96_000);
    assert_eq!(s.rate(), 96_000);
}

fn strm_arg(command: u8, arg: u32) -> Vec<u8> {
    let s = Strm {
        command,
        autostart: b'0',
        format: b'm',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: 0,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        replay_gain: arg,
        server_port: 0,
        server_ip: 0,
        request: Vec::new(),
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

/// Sync corrections reach the worker as plain objects, and a timed start's time is on the
/// worker's own clock.
#[wasm_bindgen_test]
fn sync_corrections_are_handed_to_the_worker() {
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc").unwrap();
    p.set_output_corrections(true);
    p.on_control(&strm_arg(b'a', 40), 1000.0).unwrap();
    p.on_control(&strm_arg(b'u', 1250), 1000.0).unwrap();
    let c = p.take_corrections();
    assert_eq!(c.length(), 2);
    assert_eq!(get(&c.get(0), "type").as_string().unwrap(), "skip");
    assert_eq!(get(&c.get(0), "ms").as_f64().unwrap(), 40.0);
    assert_eq!(get(&c.get(1), "type").as_string().unwrap(), "start");
    assert_eq!(get(&c.get(1), "at").as_f64().unwrap(), 1250.0);
    assert_eq!(p.start_due(), Some(1250.0));
    assert_eq!(p.take_corrections().length(), 0);
    // Junk numbers from JavaScript never panic.
    p.presented(f64::NAN, f64::INFINITY);
    p.presented(-5.0, -1.0);
}

/// The browser player decodes Ogg Opus (what the gateway sends at High and Data saver).
#[wasm_bindgen_test]
fn plays_ogg_opus() {
    let opus = include_bytes!("../../core/tests/fixtures/sine-997hz-stereo.opus");
    let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "flc").unwrap();
    p.connected(0.0);
    p.on_control(&strm_s(b'u'), 1.0).unwrap();
    p.on_stream_headers(b"HTTP/1.0 200 OK\r\nContent-Type: audio/ogg\r\n\r\n", 2.0);
    p.on_stream_data(opus, 3.0);
    p.on_stream_end(4.0);
    let (mut frames, mut rates) = (0, Vec::new());
    let mut buf = vec![0f32; 4096];
    for t in 0..400 {
        p.tick(5.0 + t as f64 * 10.0);
        let rate = p.output_rate();
        let n = p.pull(&mut buf);
        if n > 0 && !rates.contains(&rate) {
            rates.push(rate);
        }
        frames += n;
        let _ = p.take_actions();
    }
    assert_eq!(rates, vec![48_000], "{}", p.status_json());
    // The fixture is one second long.
    assert!(
        (47_000..=49_500).contains(&frames),
        "pulled {frames} frames"
    );
}

#[wasm_bindgen_test]
fn visualiser_frames_have_the_documented_layout() {
    use anywhere_wasm::{VISUAL_FRAME_LEN, WasmVisualiser};
    let mut v = WasmVisualiser::new(44_100, 2);
    assert!(v.take().is_none());
    // 0.1 s of a full-scale 1 kHz tone, stereo.
    let pcm: Vec<f32> = (0..4410)
        .flat_map(|i| {
            let x = (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 44_100.0).sin();
            [x, x]
        })
        .collect();
    v.push(&pcm);
    let f = v.take().expect("a frame after 0.1 s");
    assert_eq!(f.len(), VISUAL_FRAME_LEN);
    assert_eq!(WasmVisualiser::frame_len(), VISUAL_FRAME_LEN);
    assert!(f.iter().all(|x| x.is_finite()));
    // RMS of a full-scale sine is about −3 dBFS.
    assert!((f[160] + 3.0).abs() < 1.0, "{}", f[160]);
    v.reset();
    assert!(v.take().is_none());
}

#[wasm_bindgen_test]
fn visualiser_takes_bad_input() {
    use anywhere_wasm::WasmVisualiser;
    let mut v = WasmVisualiser::new(0, 0);
    v.push(&[f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, 0.5]);
    v.push(&[f32::NAN; 3000]);
    if let Some(f) = v.take() {
        assert!(f.iter().all(|x| x.is_finite()));
    }
}
