//! WebAssembly bindings for the player core, used by the web app's player worker.
//!
//! A thin wrapper: all behaviour lives in `anywhere_core::player`. The worker owns the
//! network (WebSocket + fetch) and feeds bytes in; an AudioWorklet plays the PCM pulled
//! out and reports back how many frames it has played.

use anywhere_core::decode::SourceFormat;
use anywhere_core::player::{
    Action, DiagEvent, OutputCorrection, OutputState, Player, PlayerConfig,
};
use anywhere_core::proto::parse_mac;
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

pub mod trim;
use trim::{PullLog, TimelineCut, TrackSegmenter};

#[wasm_bindgen]
pub struct WasmPlayer {
    inner: Player,
    /// Which track each pulled frame came from, for partial flushes (see [`trim`]).
    log: PullLog,
    /// The track of the last frames pulled.
    last_track: Option<u32>,
    /// The format last seen on the audible (or next) track, kept for the moment between
    /// one track's segments being retired and the next being decoded.
    last_source: Option<SourceFormat>,
}

fn set(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

/// A protocol byte as one printable character for the log: `?` for anything else.
pub fn letter(b: u8) -> String {
    if b.is_ascii_graphic() {
        char::from(b).to_string()
    } else {
        "?".to_string()
    }
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
            log: PullLog::default(),
            last_track: None,
            last_source: None,
        })
    }

    /// The worker applies LMS's volume at the output (see `Player::set_host_volume`).
    pub fn set_host_volume(&mut self, on: bool) {
        self.inner.set_host_volume(on);
    }

    /// Start a fast stream once `ms` of audio is decoded (0: off; see
    /// `Player::set_quick_start`).
    pub fn set_quick_start(&mut self, ms: u32) {
        self.inner.set_quick_start(Some(ms));
    }

    /// LMS's volume as linear `[left, right]` gains (1 is full scale).
    pub fn volume(&self) -> Vec<f32> {
        let (l, r) = self.inner.volume();
        vec![l, r]
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

    /// Fill `out` (interleaved stereo f32) and return the number of frames written. It may
    /// run on from one track into the next.
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        let want = out.len() / 2;
        let mut written = 0;
        while written < want {
            let n = self.pull_one(&mut out[written * 2..want * 2]);
            if n == 0 {
                break;
            }
            written += n;
        }
        written
    }

    /// Like [`WasmPlayer::pull`], but never runs into the next track: the frames are all of
    /// [`WasmPlayer::pulled_track`].
    pub fn pull_one(&mut self, out: &mut [f32]) -> usize {
        self.log.sync(self.inner.flush_count());
        match self.inner.pull_track(out) {
            Some((track, n)) => {
                self.log.pulled(track, n as u64);
                self.last_track = Some(track);
                n
            }
            None => 0,
        }
    }

    /// The track the last frames pulled belong to (`undefined` before any).
    pub fn pulled_track(&self) -> Option<u32> {
        self.last_track
    }

    /// How many partial flushes there have been (`strm f` while a track is audible; see
    /// `Player::trim_count`).
    pub fn trim_count(&self) -> f64 {
        self.inner.trim_count() as f64
    }

    /// The latest partial flush as `{keep_track, keep_frames}`, or `undefined` (see the
    /// core's `QueueTrim`).
    pub fn trim(&self) -> Option<Object> {
        self.inner.trim().map(|t| {
            let o = Object::new();
            set(
                &o,
                "keep_track",
                &JsValue::from_f64(f64::from(t.keep_track)),
            );
            set(&o, "keep_frames", &JsValue::from_f64(t.keep_frames as f64));
            o
        })
    }

    /// Once per partial flush: `{keep_track, keep_frames, drop_frames}`, else `undefined`.
    /// The audible track (`keep_track`) plays on; the output must drop the last
    /// `drop_frames` frames pulled, which belong to the tracks LMS dropped after it. Check
    /// after each control message, after following [`WasmPlayer::flush_count`].
    pub fn take_trim(&mut self) -> Option<Object> {
        self.take_trim_frames().map(|(keep, frames, drop)| {
            let o = Object::new();
            set(&o, "keep_track", &JsValue::from_f64(f64::from(keep)));
            set(&o, "keep_frames", &JsValue::from_f64(frames as f64));
            set(&o, "drop_frames", &JsValue::from_f64(drop as f64));
            o
        })
    }

    /// True once the current track is fully decoded and pulled (see the core's docs).
    pub fn audio_exhausted(&self) -> bool {
        self.inner.audio_exhausted()
    }

    /// How many times LMS has flushed the player (`strm q`, e.g. for a seek; `strm f` only
    /// when nothing is audible). When it changes, audio queued ahead of the output belongs to
    /// the old position. A `strm f` while a track is audible keeps that track playing and
    /// is not counted here.
    pub fn flush_count(&self) -> f64 {
        self.inner.flush_count() as f64
    }

    /// Frames the AudioWorklet has actually played.
    pub fn played(&mut self, frames: f64) {
        self.inner.played(frames.max(0.0) as u64);
    }

    /// `frames` more have become audible, and the total was exact at `at_ms` (the clock
    /// passed as `now_ms` elsewhere). Elapsed time in STATs is extrapolated from there (see
    /// `Player::presented`).
    pub fn presented(&mut self, frames: f64, at_ms: f64) {
        let frames = if frames.is_finite() {
            frames.max(0.0)
        } else {
            0.0
        };
        let at = if at_ms.is_finite() {
            at_ms.max(0.0)
        } else {
            0.0
        };
        self.inner.presented(frames as u64, at as u64);
    }

    /// The worker applies LMS's sync corrections at its output (see
    /// `Player::set_output_corrections`).
    pub fn set_output_corrections(&mut self, on: bool) {
        self.inner.set_output_corrections(on);
    }

    /// Corrections to apply at the output, as plain objects: `{type:"pause", ms}`,
    /// `{type:"skip", ms}`, `{type:"start", at}` (`at` on the `now_ms` clock).
    pub fn take_corrections(&mut self) -> Array {
        let out = Array::new();
        for c in self.inner.take_corrections() {
            let o = Object::new();
            match c {
                OutputCorrection::Pause { ms } => {
                    set(&o, "type", &"pause".into());
                    set(&o, "ms", &JsValue::from_f64(f64::from(ms)));
                }
                OutputCorrection::Skip { ms } => {
                    set(&o, "type", &"skip".into());
                    set(&o, "ms", &JsValue::from_f64(f64::from(ms)));
                }
                OutputCorrection::StartAt { at_ms } => {
                    set(&o, "type", &"start".into());
                    set(&o, "at", &JsValue::from_f64(at_ms as f64));
                }
            }
            out.push(&o);
        }
        out
    }

    /// Keep each `strm` from LMS and STAT sent, for the connection log (see
    /// `Player::set_diagnostics`).
    pub fn set_diagnostics(&mut self, on: bool) {
        self.inner.set_diagnostics(on);
    }

    /// The diagnostics events since the last call, as plain objects, oldest first:
    /// `{type:"strm", command, autostart, format, threshold}` (single characters, and KiB)
    /// or `{type:"stat", event}` (e.g. `"STMl"`). Nothing else from the command is exposed.
    pub fn take_diagnostics(&mut self) -> Array {
        let out = Array::new();
        for e in self.inner.take_diagnostics() {
            let o = Object::new();
            match e {
                DiagEvent::Strm {
                    command,
                    autostart,
                    format,
                    threshold,
                } => {
                    set(&o, "type", &"strm".into());
                    set(&o, "command", &letter(command).into());
                    set(&o, "autostart", &letter(autostart).into());
                    set(&o, "format", &letter(format).into());
                    set(&o, "threshold", &JsValue::from_f64(f64::from(threshold)));
                }
                DiagEvent::Stat(event) => {
                    set(&o, "type", &"stat".into());
                    let name: String = event.iter().map(|b| letter(*b)).collect();
                    set(&o, "event", &name.into());
                }
            }
            out.push(&o);
        }
        out
    }

    /// When a pending timed start is due (on the `now_ms` clock), or `undefined`.
    pub fn start_due(&self) -> Option<f64> {
        self.inner.start_due().map(|t| t as f64)
    }

    /// The status, with `source`: what the stream being heard actually is (`codec`,
    /// `lossless`, `bits`, `channels`), or `null` when nothing is known. The page uses it
    /// to say what is really streaming, which differs from the library track's format when
    /// the gateway re-encodes to Opus for a lower-data level.
    pub fn status_json(&mut self) -> String {
        let s = self.inner.status();
        let source = if matches!(s.output, OutputState::Stopped) {
            self.last_source = None;
            None
        } else {
            self.current_source()
        };
        serde_json::json!({
            "output": format!("{:?}", s.output),
            "elapsed_ms": s.elapsed_ms,
            "buffered_ms": s.buffered_ms,
            "input_bytes": s.input_bytes,
            "sample_rate": s.sample_rate,
            "volume": [s.volume.0, s.volume.1],
            "name": s.name,
            "last_error": s.last_error,
            "source": source.map(|f| serde_json::json!({
                "codec": f.codec,
                "lossless": f.lossless,
                "bits": f.bits,
                "channels": f.channels,
            })),
        })
        .to_string()
    }
}

impl WasmPlayer {
    /// [`WasmPlayer::take_trim`] as `(keep_track, keep_frames, drop_frames)`.
    pub fn take_trim_frames(&mut self) -> Option<(u32, u64, u64)> {
        let t = self.inner.trim()?;
        let drop = self.log.take_trim(
            self.inner.flush_count(),
            self.inner.trim_count(),
            t.keep_track,
        )?;
        Some((t.keep_track, t.keep_frames, drop))
    }

    /// The format of the track being heard, else of the one about to be, else the last
    /// known (a new track's format is known only once its decoder has read the headers).
    fn current_source(&mut self) -> Option<SourceFormat> {
        let now = self
            .inner
            .playing_track()
            .and_then(|seq| self.inner.track_audio(seq))
            .or_else(|| self.inner.next_audio())
            .and_then(|a| a.source);
        if now.is_some() {
            self.last_source = now;
        }
        self.last_source
    }
}

/// Lossless fMP4/FLAC segments for the media-element output path (see `anywhere_core::mux`).
#[wasm_bindgen]
pub struct WasmSegmenter {
    inner: TrackSegmenter,
}

#[wasm_bindgen]
impl WasmSegmenter {
    #[wasm_bindgen(constructor)]
    pub fn new(rate: u32) -> WasmSegmenter {
        WasmSegmenter {
            inner: TrackSegmenter::new(rate),
        }
    }

    pub fn rate(&self) -> u32 {
        self.inner.inner().rate()
    }

    /// Bytes per second of the segments at this rate and variant (see
    /// `Segmenter::bytes_per_second`): the host sizes the element's lead from it.
    pub fn bytes_per_second(&self) -> u32 {
        self.inner.inner().bytes_per_second()
    }

    /// `ftyp`+`moov` for the current rate; append before media segments (and after a rate change).
    pub fn init_segment(&self) -> Vec<u8> {
        self.inner.inner().init_segment()
    }

    pub fn set_rate(&mut self, rate: u32) {
        self.inner.set_rate(rate);
    }

    /// Init segment variant (see `anywhere_core::mux::Variant::from_code`).
    pub fn set_variant(&mut self, code: u32) {
        self.inner
            .inner_mut()
            .set_variant(anywhere_core::mux::Variant::from_code(code));
    }

    pub fn reset(&mut self) {
        self.inner.forget_tracks();
        self.inner.inner_mut().reset();
    }

    /// Continue the timeline at `seconds` (after clearing the buffer ahead of playback).
    pub fn set_time(&mut self, seconds: f64) {
        self.inner.forget_tracks();
        self.inner.inner_mut().set_time(seconds);
    }

    /// Queue interleaved stereo PCM.
    pub fn push(&mut self, pcm: &[f32]) {
        self.inner.inner_mut().push(pcm);
    }

    /// Queue interleaved stereo PCM of `track` (from [`WasmPlayer::pulled_track`]), so a
    /// partial flush can cut the timeline where a track ends (see `trim_after`).
    pub fn push_track(&mut self, pcm: &[f32], track: u32) {
        self.inner.push_track(pcm, track);
    }

    /// A partial flush: drop everything after `keep_track`. Returns the timeline second
    /// from which the segments already handed out must be removed, or `undefined` if none
    /// were (anything still queued here is dropped either way). The timeline carries on
    /// where the kept track ends; if the rate changed back, append a new init segment.
    pub fn trim_after(&mut self, keep_track: u32) -> Option<f64> {
        match self.inner.trim_after(keep_track) {
            TimelineCut::From(t) => Some(t),
            TimelineCut::None | TimelineCut::Pending => None,
        }
    }

    /// A media segment of the whole FLAC frames queued, or `undefined`.
    pub fn segment(&mut self) -> Option<Vec<u8>> {
        self.inner.inner_mut().segment()
    }

    /// A media segment of everything queued (end of stream), or `undefined`.
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        self.inner.inner_mut().flush()
    }

    pub fn emitted_seconds(&self) -> f64 {
        self.inner.inner().emitted_seconds()
    }

    pub fn pending_frames(&self) -> u32 {
        self.inner.inner().pending_frames() as u32
    }
}

/// Values in each visualiser frame from [`WasmVisualiser::take`]: the spectrum bands, then
/// the oscilloscope points, then left and right RMS (dBFS), left and right peak (dBFS) and
/// the bass level (0–1).
pub const VISUAL_FRAME_LEN: usize =
    anywhere_core::visual::BANDS + anywhere_core::visual::SCOPE_POINTS + 5;

/// The visualiser's analysis (see `anywhere_core::visual`), fed with the PCM the worker pulls.
#[wasm_bindgen]
pub struct WasmVisualiser {
    inner: anywhere_core::visual::Visualiser,
}

#[wasm_bindgen]
impl WasmVisualiser {
    /// For interleaved PCM of `channels` at `rate` Hz (the core clamps odd values).
    #[wasm_bindgen(constructor)]
    pub fn new(rate: u32, channels: u16) -> WasmVisualiser {
        WasmVisualiser {
            inner: anywhere_core::visual::Visualiser::new(rate, channels),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    /// Values per frame (see [`VISUAL_FRAME_LEN`]).
    pub fn frame_len() -> usize {
        VISUAL_FRAME_LEN
    }

    /// Add interleaved samples.
    pub fn push(&mut self, pcm: &[f32]) {
        self.inner.push(pcm);
    }

    /// The newest frame since the last call as one flat array (see [`VISUAL_FRAME_LEN`]), or
    /// `undefined`.
    pub fn take(&mut self) -> Option<Vec<f32>> {
        self.inner.take().map(|f| flatten(&f))
    }

    /// Forget the history (a flush).
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

/// One frame as the flat layout the worker sends to the page.
pub fn flatten(f: &anywhere_core::visual::VisualFrame) -> Vec<f32> {
    let mut out = Vec::with_capacity(VISUAL_FRAME_LEN);
    out.extend(f.bands.iter().copied().take(anywhere_core::visual::BANDS));
    out.resize(anywhere_core::visual::BANDS, 0.0);
    out.extend(
        f.scope
            .iter()
            .copied()
            .take(anywhere_core::visual::SCOPE_POINTS),
    );
    out.resize(
        anywhere_core::visual::BANDS + anywhere_core::visual::SCOPE_POINTS,
        0.0,
    );
    out.extend_from_slice(&f.rms_db);
    out.extend_from_slice(&f.peak_db);
    out.push(f.bass);
    out
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use anywhere_core::mux::BLOCK;
    use anywhere_core::proto::{Strm, encode_server_frame};

    fn strm(cmd: u8) -> Vec<u8> {
        let s = Strm {
            command: cmd,
            autostart: b'1',
            format: b'p',
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
            request: b"GET /stream.wav?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n".to_vec(),
        };
        encode_server_frame(*b"strm", &s.encode_payload())
    }

    /// Protocol bytes reach the log as one printable character, never control characters
    /// or anything wider.
    #[test]
    fn letters_are_one_printable_character() {
        assert_eq!(letter(b's'), "s");
        assert_eq!(letter(b'1'), "1");
        assert_eq!(letter(0), "?");
        assert_eq!(letter(b' '), "?");
        assert_eq!(letter(b'\n'), "?");
        assert_eq!(letter(0x7f), "?");
        assert_eq!(letter(0xff), "?");
        for b in 0..=255u8 {
            let l = letter(b);
            assert_eq!(l.len(), 1);
            assert!(l.as_bytes()[0].is_ascii_graphic());
        }
    }

    /// 16-bit stereo WAV of one constant sample value, so each track's audio is recognisable.
    fn wav(rate: u32, frames: usize, value: i16) -> Vec<u8> {
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
        for _ in 0..frames * 2 {
            v.extend_from_slice(&value.to_le_bytes());
        }
        v
    }

    const T1: i16 = 1000;
    const T2: i16 = 2000;

    fn track_of(sample: f32) -> i16 {
        (sample * 32768.0).round() as i16
    }

    /// Track 1 playing, with track 2 fully streamed behind it.
    fn two_tracks() -> WasmPlayer {
        let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "pcm").unwrap();
        p.set_host_volume(true);
        p.on_control(&strm(b's'), 0.0).unwrap();
        p.on_stream_data(&wav(44_100, 44_100, T1), 1.0);
        p.on_stream_end(2.0);
        for t in 3..23 {
            p.tick(f64::from(t));
        }
        p.on_control(&strm(b's'), 600.0).unwrap();
        p.on_stream_data(&wav(44_100, 44_100, T2), 601.0);
        p.on_stream_end(602.0);
        for t in 603..623 {
            p.tick(f64::from(t));
        }
        p
    }

    /// Regression: after `strm f` (the next track deleted or moved in LMS's queue), the
    /// worker must drop what it already pulled of the next track, and only that.
    #[test]
    fn strm_f_says_how_much_of_the_pulled_audio_to_drop() {
        let mut p = two_tracks();
        let mut buf = vec![0f32; 50_000 * 2];
        assert_eq!(p.pull(&mut buf), 50_000, "runs on into track 2");
        assert_eq!(track_of(buf[44_099 * 2]), T1);
        assert_eq!(track_of(buf[44_100 * 2]), T2);
        p.played(30_000.0);
        assert_eq!(p.take_trim_frames(), None);
        let flushes = p.flush_count();
        p.on_control(&strm(b'f'), 700.0).unwrap();
        assert_eq!(p.flush_count(), flushes, "a partial flush");
        let (keep, keep_frames, drop) = p.take_trim_frames().expect("a trim");
        assert_eq!(keep, p.pulled_track().unwrap() - 1, "track 1 is kept");
        assert_eq!(
            keep_frames,
            44_100 - 30_000,
            "what the output holds of track 1"
        );
        assert_eq!(drop, 50_000 - 44_100, "what it holds of track 2");
        assert_eq!(p.take_trim_frames(), None, "once");
        // Nothing of track 2 comes out any more.
        assert_eq!(p.pull(&mut buf), 0);
    }

    /// A full flush (`strm q`) is not a trim: the output drops everything anyway.
    #[test]
    fn a_full_flush_is_not_a_trim() {
        let mut p = two_tracks();
        let mut buf = vec![0f32; 50_000 * 2];
        p.pull(&mut buf);
        p.played(10_000.0);
        p.on_control(&strm(b'q'), 700.0).unwrap();
        assert_eq!(p.take_trim_frames(), None);
    }

    /// `pull_one` never crosses a track, and says whose audio it is.
    #[test]
    fn pull_one_stays_within_a_track() {
        let mut p = two_tracks();
        let mut buf = vec![0f32; 50_000 * 2];
        assert_eq!(p.pull_one(&mut buf), 44_100);
        let first = p.pulled_track().unwrap();
        assert_eq!(p.pull_one(&mut buf), 44_100);
        assert_eq!(p.pulled_track(), Some(first + 1));
        assert!(buf[..44_100 * 2].iter().all(|&s| track_of(s) == T2));
    }

    /// Regression, media-element output: segments of the dropped track already handed to
    /// the element are cut from the timeline exactly where track 1 ends.
    #[test]
    fn strm_f_cuts_the_media_timeline_where_the_kept_track_ends() {
        let mut p = two_tracks();
        let mut seg = WasmSegmenter::new(44_100);
        let mut buf = vec![0f32; 4096 * 2];
        let mut pulled = 0;
        while pulled < 60_000 {
            let n = p.pull_one(&mut buf);
            assert!(n > 0);
            seg.push_track(&buf[..n * 2], p.pulled_track().unwrap());
            while seg.segment().is_some() {}
            pulled += n;
        }
        p.played(20_000.0);
        p.on_control(&strm(b'f'), 700.0).unwrap();
        let (keep, _, drop) = p.take_trim_frames().unwrap();
        assert_eq!(drop as usize, pulled - 44_100);
        let from = seg
            .trim_after(keep)
            .expect("emitted audio of track 2 is removed");
        let block_start = 44_100 / BLOCK * BLOCK;
        assert!(from < block_start as f64 / 44_100.0);
        assert!(block_start as f64 / 44_100.0 - from < 1.0 / 44_100.0);
        // The rest of track 1 is queued again, from the start of that FLAC frame.
        assert_eq!(seg.pending_frames() as usize, 44_100 - block_start);
        assert!((seg.emitted_seconds() - block_start as f64 / 44_100.0).abs() < 1e-12);
        assert!(
            seg.inner
                .inner()
                .pending_pcm()
                .iter()
                .all(|&s| track_of(s) == T1),
            "only track 1 is queued"
        );
        assert_eq!(p.pull_one(&mut buf), 0, "nothing of track 2 is left");
    }

    /// The status says what the stream really is, for the page's signal path and quality
    /// badge: nothing before a track, the decoded format while one plays, nothing once
    /// stopped. (A Hi-Res FLAC in the library may arrive as Opus from the gateway; the page
    /// must be able to say so.)
    #[test]
    fn status_reports_the_streams_real_format() {
        let mut p = WasmPlayer::new("02:00:00:00:00:01", "Web", "pcm").unwrap();
        let v: serde_json::Value = serde_json::from_str(&p.status_json()).unwrap();
        assert!(v["source"].is_null(), "nothing streamed yet");
        let mut p = two_tracks();
        let v: serde_json::Value = serde_json::from_str(&p.status_json()).unwrap();
        assert_eq!(v["source"]["codec"], "PCM");
        assert_eq!(v["source"]["lossless"], true);
        assert_eq!(v["source"]["bits"], 16);
        assert_eq!(v["source"]["channels"], 2);
        // Still known while track 1 is heard and its segment is being retired.
        let mut buf = vec![0f32; 50_000 * 2];
        p.pull(&mut buf);
        p.played(44_100.0);
        p.tick(700.0);
        let v: serde_json::Value = serde_json::from_str(&p.status_json()).unwrap();
        assert_eq!(v["source"]["codec"], "PCM");
        p.on_control(&strm(b'q'), 800.0).unwrap();
        let v: serde_json::Value = serde_json::from_str(&p.status_json()).unwrap();
        assert!(v["source"].is_null(), "stopped: nothing is streaming");
    }
}
