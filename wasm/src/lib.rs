//! WebAssembly bindings for the player core, used by the web app's player worker.
//!
//! A thin wrapper: all behaviour lives in `anywhere_core::player`. The worker owns the
//! network (WebSocket + fetch) and feeds bytes in; an AudioWorklet plays the PCM pulled
//! out and reports back how many frames it has played.

use anywhere_core::player::{Action, Player, PlayerConfig};
use anywhere_core::proto::parse_mac;
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmPlayer {
    inner: Player,
}

fn set(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

#[wasm_bindgen]
impl WasmPlayer {
    /// `mac` is the identity the gateway assigned (from its session greeting).
    /// `codecs` is a comma-separated capability list, e.g. `"flc,mp3,ogg"`.
    #[wasm_bindgen(constructor)]
    pub fn new(mac: &str, name: &str, codecs: &str) -> Result<WasmPlayer, JsError> {
        let mac = parse_mac(mac).ok_or_else(|| JsError::new("invalid MAC from gateway"))?;
        let codecs = codecs
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(String::from)
            .collect();
        Ok(WasmPlayer {
            inner: Player::new(PlayerConfig {
                mac,
                uuid: [0; 16],
                name: name.to_string(),
                codecs,
                max_sample_rate: 192_000,
                firmware: concat!("web-", env!("CARGO_PKG_VERSION")).to_string(),
            }),
        })
    }

    pub fn connected(&mut self, now_ms: f64) {
        self.inner.connected(now_ms as u64);
    }

    /// One gateway message (one or more complete Slimproto server frames).
    pub fn on_control(&mut self, data: &[u8], now_ms: f64) -> Result<(), JsError> {
        self.inner
            .on_control_data(data, now_ms as u64)
            .map_err(|e| JsError::new(&e))
    }

    pub fn on_stream_headers(&mut self, headers: &[u8], now_ms: f64) {
        self.inner.on_stream_headers(headers, now_ms as u64);
    }

    pub fn on_stream_data(&mut self, data: &[u8], now_ms: f64) {
        self.inner.on_stream_data(data, now_ms as u64);
    }

    pub fn on_stream_end(&mut self, now_ms: f64) {
        self.inner.on_stream_end(now_ms as u64);
    }

    pub fn on_stream_error(&mut self, now_ms: f64) {
        self.inner.on_stream_error(now_ms as u64);
    }

    /// The stream being downloaded as `[stream_number, bytes_received]`, or `undefined`.
    /// After a broken download, refetch with `?offset=<bytes_received>` and do not pass the
    /// response headers in again.
    pub fn open_stream(&self) -> Option<Vec<f64>> {
        self.inner
            .open_stream()
            .map(|(n, bytes)| vec![n as f64, bytes as f64])
    }

    pub fn wants_stream_data(&self) -> bool {
        self.inner.wants_stream_data()
    }

    pub fn tick(&mut self, now_ms: f64) {
        self.inner.tick(now_ms as u64);
    }

    /// Pending actions as plain objects:
    /// `{type:"send", bytes}`, `{type:"open", stream}` (gateway stream number), `{type:"close"}`.
    pub fn take_actions(&mut self) -> Array {
        let out = Array::new();
        for a in self.inner.take_actions() {
            let o = Object::new();
            match a {
                Action::Send(bytes) => {
                    set(&o, "type", &"send".into());
                    set(&o, "bytes", &Uint8Array::from(bytes.as_slice()).into());
                }
                Action::OpenStream { stream_number, .. } => {
                    set(&o, "type", &"open".into());
                    set(&o, "stream", &JsValue::from_f64(stream_number as f64));
                }
                Action::CloseStream => set(&o, "type", &"close".into()),
            }
            out.push(&o);
        }
        out
    }

    /// Sample rate of the next PCM to pull (0 if none).
    pub fn output_rate(&self) -> u32 {
        self.inner.output_rate().unwrap_or(0)
    }

    /// Fill `out` (interleaved stereo f32) and return the number of frames written.
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        self.inner.pull_audio(out)
    }

    /// Frames the AudioWorklet has actually played.
    pub fn played(&mut self, frames: f64) {
        self.inner.played(frames.max(0.0) as u64);
    }

    pub fn status_json(&self) -> String {
        let s = self.inner.status();
        serde_json::json!({
            "output": format!("{:?}", s.output),
            "elapsed_ms": s.elapsed_ms,
            "buffered_ms": s.buffered_ms,
            "input_bytes": s.input_bytes,
            "sample_rate": s.sample_rate,
            "volume": [s.volume.0, s.volume.1],
            "name": s.name,
            "last_error": s.last_error,
        })
        .to_string()
    }
}
