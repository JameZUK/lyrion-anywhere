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
