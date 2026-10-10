//! Slimproto player state machine.
//!
//! Behaves towards LMS like a squeezelite-class software player:
//! - `strm s` opens a stream, and the next track's stream is appended for gapless playback.
//! - Status events go back to LMS as STAT: `STMc` connect, `STMs` track started,
//!   `STMd` decoder done (LMS then sends the next track), `STMu` played out,
//!   `STMp`/`STMr` paused/resumed, `STMf` flushed, `STMt` heartbeat,
//!   `STMl` loaded (autostart off), `STMn` unsupported.
//!
//! Sans-I/O: the host feeds control bytes, stream bytes and a millisecond clock, executes
//! the [`Action`]s, and pulls PCM with [`Player::pull_audio`].

use std::collections::VecDeque;

use crate::decode::{DecodeStatus, InputBuffer, StreamDecoder};
use crate::proto::{ClientMsg, DEVICE_ID_SOFTWARE, Helo, ServerFrameReader, ServerMsg, Stat, Strm};

/// Stop reading the network stream once this many compressed bytes are waiting.
pub const INPUT_HIGH_WATER: usize = 8 * 1024 * 1024;
/// Decode ahead until this many seconds of PCM are queued.
pub const OUTPUT_TARGET_SECS: u32 = 10;
/// Packets decoded per `tick`, to bound the time spent per call.
const PACKETS_PER_TICK: usize = 48;
const HEARTBEAT_MS: u64 = 1000;

#[derive(Debug, Clone)]
pub struct PlayerConfig {
    pub mac: [u8; 6],
    pub uuid: [u8; 16],
    /// Name shown in LMS.
    pub name: String,
    /// Codec capability tokens advertised to LMS, e.g. `["flc", "mp3", "ogg"]`.
    pub codecs: Vec<String>,
    pub max_sample_rate: u32,
    pub firmware: String,
}

impl PlayerConfig {
    pub fn capabilities(&self) -> String {
        // `Model=squeezelite` gets the full software-player treatment (gapless, codec
        // negotiation) from LMS; `ModelName` is what users see.
        let mut caps = format!(
            "Model=squeezelite,ModelName=Lyrion Anywhere,AccuratePlayPoints=1,HasDigitalOut=1,\
             Firmware={},MaxSampleRate={}",
            self.firmware, self.max_sample_rate
        );
        for c in &self.codecs {
            caps.push(',');
            caps.push_str(c);
        }
        caps
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamHost {
    /// The slimproto server's own address (`server_ip` = 0).
    Server,
    Ip([u8; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bytes to write to the slimproto (control) connection.
    Send(Vec<u8>),
    /// Open an HTTP stream: connect to `host:port` and send `request` verbatim.
    /// Through the gateway, fetch `/v1/stream/<session>/<stream_number>` instead.
    OpenStream {
        host: StreamHost,
        port: u16,
        request: Vec<u8>,
        /// 1-based count of every `strm s` received on this connection, including ones
        /// refused locally, so it matches the gateway's numbering exactly.
        stream_number: u64,
    },
    /// Abandon the current HTTP stream.
    CloseStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputState {
    Stopped,
    /// Buffering before autostart.
    Waiting,
    /// Threshold reached with autostart off; waiting for `strm u`.
    Loaded,
    Playing,
    /// Paused; resumes automatically at `until` (jiffies) if set.
    Paused {
        until: Option<u64>,
    },
}

/// A run of PCM in the output queue belonging to one track.
#[derive(Debug)]
struct Segment {
    seq: u32,
    rate: u32,
    /// Frames appended to the queue so far.
    frames: u64,
    pulled: u64,
    played: u64,
    /// Decoder finished: no more frames will be appended.
    complete: bool,
}

struct Track {
    seq: u32,
    input: InputBuffer,
    decoder: StreamDecoder,
    autostart: u8,
    threshold: usize,
    bytes_received: u64,
    replay_gain: f32,
    stream_ended: bool,
    decode_done: bool,
}

/// Snapshot for UIs and logs.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub output: OutputState,
    pub elapsed_ms: u64,
    pub buffered_ms: u64,
    pub input_bytes: usize,
    pub sample_rate: u32,
    pub volume: (f32, f32),
    pub name: String,
    pub last_error: Option<String>,
}

pub struct Player {
    cfg: PlayerConfig,
    reader: ServerFrameReader,
    actions: Vec<Action>,
    now: u64,
    track: Option<Track>,
    next_seq: u32,
    /// Interleaved stereo PCM, in segment order.
    pcm: VecDeque<f32>,
    segments: VecDeque<Segment>,
    output: OutputState,
    gain: (f32, f32),
    /// Segment currently audible (for elapsed time and STMs).
    playing_seq: Option<u32>,
    last_heartbeat: u64,
    stmu_sent: bool,
    last_error: Option<String>,
}

impl Player {
    pub fn new(cfg: PlayerConfig) -> Self {
        Player {
            cfg,
            reader: ServerFrameReader::default(),
            actions: Vec::new(),
            now: 0,
            track: None,
            next_seq: 0,
            pcm: VecDeque::new(),
            segments: VecDeque::new(),
            output: OutputState::Stopped,
            gain: (1.0, 1.0),
            playing_seq: None,
            last_heartbeat: 0,
            stmu_sent: true,
            last_error: None,
        }
    }

    /// Call when the control connection is (re)established. Sends HELO.
    pub fn connected(&mut self, now_ms: u64) {
        self.now = now_ms;
        self.reader = ServerFrameReader::default();
        let helo = Helo {
            device_id: DEVICE_ID_SOFTWARE,
            revision: 0,
            mac: self.cfg.mac,
            uuid: self.cfg.uuid,
            wlan_channels: 0,
            bytes_received: self.track.as_ref().map_or(0, |t| t.bytes_received),
            lang: *b"en",
            capabilities: self.cfg.capabilities(),
        };
        self.send(ClientMsg::Helo(helo));
    }

    /// Bytes received on the control connection.
    pub fn on_control_data(&mut self, data: &[u8], now_ms: u64) -> Result<(), String> {
        self.now = now_ms;
        self.reader.push(data);
        loop {
            match self.reader.next_msg() {
                Ok(Some(msg)) => self.handle(msg),
                Ok(None) => return Ok(()),
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    pub fn on_stream_headers(&mut self, headers: &[u8], now_ms: u64) {
        self.now = now_ms;
        self.send(ClientMsg::Resp(headers.to_vec()));
    }

    pub fn on_stream_data(&mut self, data: &[u8], now_ms: u64) {
        self.now = now_ms;
        if let Some(t) = &mut self.track {
            t.input.push(data);
            t.bytes_received += data.len() as u64;
        }
    }

    /// The HTTP stream finished normally.
    pub fn on_stream_end(&mut self, now_ms: u64) {
        self.end_stream(0, now_ms);
    }

    /// The HTTP stream failed (reset or connect failure).
    pub fn on_stream_error(&mut self, now_ms: u64) {
        self.end_stream(1, now_ms);
    }

    fn end_stream(&mut self, reason: u8, now_ms: u64) {
        self.now = now_ms;
        if let Some(t) = &mut self.track
            && !t.stream_ended
        {
            t.stream_ended = true;
            t.input.set_eof();
            self.send(ClientMsg::Dsco(reason));
        }
    }

    /// Whether the host should keep reading the network stream (back-pressure).
    pub fn wants_stream_data(&self) -> bool {
        self.track
            .as_ref()
            .is_some_and(|t| t.input.unread() < INPUT_HIGH_WATER)
    }

    /// Do decode work, autostart, timed resume, heartbeats and underrun detection.
    pub fn tick(&mut self, now_ms: u64) {
        self.now = now_ms;

        if let OutputState::Paused { until: Some(at) } = self.output
            && self.jiffies() as u64 >= at
        {
            self.output = OutputState::Playing;
        }

        self.decode_some();
        self.check_autostart();

        if self.output == OutputState::Playing
            && now_ms.saturating_sub(self.last_heartbeat) >= HEARTBEAT_MS
        {
            self.last_heartbeat = now_ms;
            self.send_stat(*b"STMt", 0);
        }

        // Everything decoded and played out: tell LMS the track (and playlist) finished.
        let drained = self
            .segments
            .iter()
            .all(|s| s.complete && s.played == s.frames);
        let decoding = self.track.as_ref().is_some_and(|t| !t.decode_done);
        if self.output == OutputState::Playing && drained && !decoding && !self.stmu_sent {
            self.stmu_sent = true;
            self.output = OutputState::Stopped;
            self.send_stat(*b"STMu", 0);
        }
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Sample rate of the next PCM to be pulled. The host should (re)open its audio output at
    /// this rate when it changes; `pull_audio` never crosses a rate change.
    pub fn output_rate(&self) -> Option<u32> {
        self.segments
            .iter()
            .find(|s| s.pulled < s.frames || !s.complete)
            .map(|s| s.rate)
    }

    /// Fill `out` (interleaved stereo) with up to `out.len() / 2` frames at `output_rate()`.
    /// Returns frames written; the host plays silence for the rest. Call [`Player::played`]
    /// once the frames have actually been played (immediately, for low-latency outputs).
    pub fn pull_audio(&mut self, out: &mut [f32]) -> usize {
        if self.output != OutputState::Playing {
            return 0;
        }
        let Some(rate) = self.output_rate() else {
            return 0;
        };
        let want = out.len() / 2;
        let mut written = 0;
        for seg in self.segments.iter_mut() {
            if written == want {
                break;
            }
            if seg.pulled == seg.frames {
                continue;
            }
            if seg.rate != rate {
                break;
            }
            let n = ((seg.frames - seg.pulled) as usize).min(want - written);
            for i in 0..n {
                let l = self.pcm.pop_front().unwrap_or(0.0);
                let r = self.pcm.pop_front().unwrap_or(0.0);
                out[(written + i) * 2] = l * self.gain.0;
                out[(written + i) * 2 + 1] = r * self.gain.1;
            }
            seg.pulled += n as u64;
            written += n;
        }
        written
    }

    /// Report that `frames` previously pulled have now been played.
    pub fn played(&mut self, mut frames: u64) {
        while frames > 0 {
            let Some(idx) = self.segments.iter().position(|s| s.played < s.pulled) else {
                break;
            };
            let seg = &mut self.segments[idx];
            if seg.played == 0 && self.playing_seq != Some(seg.seq) {
                let seq = seg.seq;
                self.playing_seq = Some(seq);
                self.send_stat(*b"STMs", 0);
            }
            let seg = &mut self.segments[idx];
            let n = frames.min(seg.pulled - seg.played);
            seg.played += n;
            frames -= n;
        }
        // Drop fully played segments, keeping the one that is audible for elapsed time.
        while self.segments.len() > 1 {
            let s = &self.segments[0];
            if s.complete && s.played == s.frames {
                self.segments.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn status(&self) -> Status {
        Status {
            output: self.output,
            elapsed_ms: self.elapsed_ms(),
            buffered_ms: self.buffered_ms(),
            input_bytes: self.track.as_ref().map_or(0, |t| t.input.unread()),
            sample_rate: self.output_rate().unwrap_or(0),
            volume: self.gain,
            name: self.cfg.name.clone(),
            last_error: self.last_error.clone(),
        }
    }

    // -----------------------------------------------------------------------

    fn handle(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Strm(s) => self.handle_strm(s),
            ServerMsg::Audg(a) => self.gain = a.gains(),
            ServerMsg::Setd { id: 0, data } => {
                if data.is_empty() {
                    let mut name = self.cfg.name.clone().into_bytes();
                    name.push(0);
                    self.send(ClientMsg::Setd { id: 0, data: name });
                } else {
                    // LMS renamed the player.
                    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
                    self.cfg.name = String::from_utf8_lossy(&data[..end]).into_owned();
                }
            }
            // Never follow a redirect to another server.
            ServerMsg::Serv { .. } => {}
            ServerMsg::Aude { .. } | ServerMsg::Vers(_) | ServerMsg::Setd { .. } => {}
            ServerMsg::Other { .. } => {}
        }
    }

    fn handle_strm(&mut self, s: Strm) {
        match s.command {
            b's' => self.start_stream(s),
            b'p' => {
                if s.replay_gain == 0 {
                    self.output = OutputState::Paused { until: None };
                    self.send_stat(*b"STMp", 0);
                } else {
                    // Sync adjustment: pause briefly, then carry on.
                    let until = self.jiffies() as u64 + s.replay_gain as u64;
                    self.output = OutputState::Paused { until: Some(until) };
                }
            }
            b'u' => {
                if s.replay_gain != 0 && (s.replay_gain as u64) > self.jiffies() as u64 {
                    self.output = OutputState::Paused {
                        until: Some(s.replay_gain as u64),
                    };
                } else {
                    self.output = OutputState::Playing;
                }
                self.send_stat(*b"STMr", 0);
            }
            b'q' | b'f' => {
                self.flush();
                self.send_stat(*b"STMf", 0);
            }
            b't' => self.send_stat(*b"STMt", s.replay_gain),
            b'a' => self.skip_ahead(s.replay_gain as u64),
            _ => {}
        }
    }

    fn start_stream(&mut self, s: Strm) {
        if self.track.as_ref().is_some_and(|t| !t.stream_ended) {
            self.actions.push(Action::CloseStream);
        }
        let seq = self.next_seq;
        self.next_seq += 1;

        let codec_ok = crate::decode::format_extension(s.format).is_some();
        let input = InputBuffer::new();
        self.track = Some(Track {
            seq,
            decoder: StreamDecoder::new(input.clone(), s.format),
            input,
            autostart: s.autostart,
            threshold: s.threshold as usize * 1024,
            bytes_received: 0,
            replay_gain: if s.replay_gain == 0 {
                1.0
            } else {
                s.replay_gain as f32 / 65536.0
            },
            stream_ended: false,
            decode_done: false,
        });
        self.stmu_sent = false;
        if !codec_ok {
            self.send_stat(*b"STMn", 0);
            return;
        }
        let host = if s.server_ip == 0 {
            StreamHost::Server
        } else {
            StreamHost::Ip(s.server_ip.to_be_bytes())
        };
        self.actions.push(Action::OpenStream {
            host,
            port: s.server_port,
            request: s.request,
            stream_number: seq as u64 + 1,
        });
        self.send_stat(*b"STMc", 0);
        if matches!(self.output, OutputState::Stopped) {
            self.output = OutputState::Waiting;
        }
    }

    fn flush(&mut self) {
        if self.track.as_ref().is_some_and(|t| !t.stream_ended) {
            self.actions.push(Action::CloseStream);
        }
        self.track = None;
        self.pcm.clear();
        self.segments.clear();
        self.playing_seq = None;
        self.output = OutputState::Stopped;
        self.stmu_sent = true;
    }

    fn skip_ahead(&mut self, ms: u64) {
        let Some(rate) = self.output_rate() else {
            return;
        };
        let mut frames = ms * rate as u64 / 1000;
        for seg in self.segments.iter_mut() {
            let n = (seg.frames - seg.pulled).min(frames);
            for _ in 0..n * 2 {
                self.pcm.pop_front();
            }
            seg.pulled += n;
            seg.played += n;
            frames -= n;
            if frames == 0 {
                break;
            }
        }
    }

    fn decode_some(&mut self) {
        let target_frames =
            OUTPUT_TARGET_SECS as usize * self.output_rate().unwrap_or(48_000) as usize;
        if self.pcm.len() / 2 >= target_frames {
            return;
        }
        let Some(t) = &mut self.track else { return };
        if t.decode_done {
            return;
        }
        let (pcm, segments, seq, rg) = (&mut self.pcm, &mut self.segments, t.seq, t.replay_gain);
        let status = t.decoder.work(PACKETS_PER_TICK, &mut |rate, samples| {
            let need_new = match segments.back() {
                Some(s) => s.seq != seq || s.rate != rate,
                None => true,
            };
            if need_new {
                if let Some(prev) = segments.back_mut()
                    && prev.seq == seq
                {
                    prev.complete = true; // rate change mid-track
                }
                segments.push_back(Segment {
                    seq,
                    rate,
                    frames: 0,
                    pulled: 0,
                    played: 0,
                    complete: false,
                });
            }
            let seg = segments.back_mut().unwrap();
            if rg == 1.0 {
                pcm.extend(samples);
            } else {
                pcm.extend(samples.iter().map(|s| s * rg));
            }
            seg.frames += (samples.len() / 2) as u64;
        });
        match status {
            DecodeStatus::Finished => {
                t.decode_done = true;
                if let Some(s) = self.segments.iter_mut().rev().find(|s| s.seq == seq) {
                    s.complete = true;
                }
                self.send_stat(*b"STMd", 0);
            }
            DecodeStatus::Failed(e) => {
                t.decode_done = true;
                self.last_error = Some(e);
                if let Some(s) = self.segments.iter_mut().rev().find(|s| s.seq == seq) {
                    s.complete = true;
                }
                self.send_stat(*b"STMn", 0);
            }
            DecodeStatus::Progress | DecodeStatus::NeedData => {}
        }
    }

    fn check_autostart(&mut self) {
        if !matches!(self.output, OutputState::Waiting) {
            return;
        }
        let Some(t) = &self.track else { return };
        let ready = (t.bytes_received as usize >= t.threshold || t.stream_ended || t.decode_done)
            && !self.pcm.is_empty();
        if !ready {
            return;
        }
        if t.autostart == b'1' || t.autostart == b'3' {
            self.output = OutputState::Playing;
        } else {
            self.output = OutputState::Loaded;
            self.send_stat(*b"STMl", 0);
        }
    }

    fn jiffies(&self) -> u32 {
        self.now as u32
    }

    fn elapsed_ms(&self) -> u64 {
        let Some(seq) = self.playing_seq else {
            return 0;
        };
        let mut frames = 0;
        let mut rate = 0;
        for s in self.segments.iter().filter(|s| s.seq == seq) {
            frames += s.played;
            rate = s.rate;
        }
        if rate == 0 {
            0
        } else {
            frames * 1000 / rate as u64
        }
    }

    fn buffered_ms(&self) -> u64 {
        match self.output_rate() {
            Some(r) if r > 0 => (self.pcm.len() / 2) as u64 * 1000 / r as u64,
            _ => 0,
        }
    }

    fn send(&mut self, msg: ClientMsg) {
        self.actions.push(Action::Send(msg.encode()));
    }

    fn send_stat(&mut self, event: [u8; 4], server_timestamp: u32) {
        let elapsed = self.elapsed_ms();
        let (input_unread, bytes) = self
            .track
            .as_ref()
            .map_or((0, 0), |t| (t.input.unread() as u32, t.bytes_received));
        let out_bytes = (self.pcm.len() * 4) as u32;
        let stat = Stat {
            event,
            stream_buffer_size: INPUT_HIGH_WATER as u32,
            stream_buffer_fullness: input_unread,
            bytes_received: bytes,
            signal_strength: 0xffff,
            jiffies: self.jiffies(),
            output_buffer_size: (OUTPUT_TARGET_SECS * 48_000 * 8).max(out_bytes),
            output_buffer_fullness: out_bytes,
            elapsed_seconds: (elapsed / 1000) as u32,
            elapsed_milliseconds: elapsed as u32,
            server_timestamp,
            ..Default::default()
        };
        self.send(ClientMsg::Stat(stat));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{ClientFrameReader, encode_server_frame};

    fn cfg() -> PlayerConfig {
        PlayerConfig {
            mac: [2, 0, 0, 0, 0, 1],
            uuid: [0; 16],
            name: "Test".into(),
            codecs: vec!["flc".into(), "pcm".into()],
            max_sample_rate: 192_000,
            firmware: "test".into(),
        }
    }

    fn strm(cmd: u8, autostart: u8, format: u8, arg: u32) -> Vec<u8> {
        let s = Strm {
            command: cmd,
            autostart,
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
            replay_gain: arg,
            server_port: 9000,
            server_ip: 0,
            request: b"GET /stream.mp3?player=02:00:00:00:00:01 HTTP/1.0\r\n\r\n".to_vec(),
        };
        encode_server_frame(*b"strm", &s.encode_payload())
    }

    /// STAT events sent, in order.
    fn events(actions: &[Action]) -> Vec<String> {
        let mut r = ClientFrameReader::default();
        for a in actions {
            if let Action::Send(b) = a {
                r.push(b);
            }
        }
        let mut out = vec![];
        while let Some((op, p)) = r.next_frame().unwrap() {
            if &op == b"STAT" {
                out.push(String::from_utf8_lossy(&p[0..4]).into_owned());
            } else {
                out.push(String::from_utf8_lossy(&op).into_owned());
            }
        }
        out
    }

    fn wav(rate: u32, frames: usize) -> Vec<u8> {
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
        v.resize(44 + data_len, 0x10);
        v
    }

    /// Pull everything currently playable, reporting it as played immediately.
    fn drain(p: &mut Player) -> usize {
        let mut buf = vec![0f32; 4096];
        let mut total = 0;
        loop {
            let n = p.pull_audio(&mut buf);
            if n == 0 {
                return total;
            }
            p.played(n as u64);
            total += n;
        }
    }

    #[test]
    fn helo_on_connect_and_name_on_request() {
        let mut p = Player::new(cfg());
        p.connected(0);
        p.on_control_data(&encode_server_frame(*b"setd", &[0]), 1)
            .unwrap();
        assert_eq!(events(&p.take_actions()), vec!["HELO", "SETD"]);
    }

    #[test]
    fn plays_two_tracks_gaplessly_with_correct_events() {
        let mut p = Player::new(cfg());
        p.connected(0);
        p.take_actions();

        // Track 1.
        p.on_control_data(&strm(b's', b'1', b'p', 0), 10).unwrap();
        let a = p.take_actions();
        assert!(matches!(
            a[0],
            Action::OpenStream {
                host: StreamHost::Server,
                port: 9000,
                ..
            }
        ));
        assert_eq!(events(&a), vec!["STMc"]);

        let t1 = wav(44100, 44100); // 1 s
        p.on_stream_headers(b"HTTP/1.0 200 OK\r\n\r\n", 11);
        p.on_stream_data(&t1, 12);
        p.on_stream_end(13);
        for i in 0..20 {
            p.tick(14 + i);
        }
        assert_eq!(p.status().output, OutputState::Playing);
        assert_eq!(events(&p.take_actions()), vec!["RESP", "DSCO", "STMd"]);

        // Play half of track 1, then LMS sends track 2 (as it does after STMd).
        let mut buf = vec![0f32; 22050 * 2];
        assert_eq!(p.pull_audio(&mut buf), 22050);
        p.played(22050);
        assert_eq!(events(&p.take_actions()), vec!["STMs"]);
        assert_eq!(p.status().elapsed_ms, 500);

        p.on_control_data(&strm(b's', b'1', b'p', 0), 600).unwrap();
        p.on_stream_data(&wav(44100, 44100), 601);
        p.on_stream_end(602);
        for i in 0..20 {
            p.tick(603 + i);
        }
        p.take_actions();

        // The rest of track 1 and all of track 2 come out contiguously: no gap.
        let total = drain(&mut p);
        assert_eq!(total, 22050 + 44100);
        let ev = events(&p.take_actions());
        assert_eq!(
            ev.iter().filter(|e| *e == "STMs").count(),
            1,
            "STMs for track 2: {ev:?}"
        );
        assert_eq!(p.status().elapsed_ms, 1000);

        p.tick(2000);
        assert!(events(&p.take_actions()).contains(&"STMu".to_string()));
        assert_eq!(p.status().output, OutputState::Stopped);
    }

    #[test]
    fn pause_resume_flush() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        for i in 0..10 {
            p.tick(3 + i);
        }
        p.take_actions();

        p.on_control_data(&strm(b'p', b'0', b'p', 0), 20).unwrap();
        let mut buf = vec![0f32; 1024];
        assert_eq!(p.pull_audio(&mut buf), 0, "paused output yields nothing");
        p.on_control_data(&strm(b'u', b'0', b'p', 0), 30).unwrap();
        assert!(p.pull_audio(&mut buf) > 0);
        p.on_control_data(&strm(b'q', b'0', b'p', 0), 40).unwrap();
        let a = p.take_actions();
        assert_eq!(events(&a), vec!["STMp", "STMr", "STMf"]);
        assert_eq!(p.status().output, OutputState::Stopped);
        assert_eq!(p.pull_audio(&mut buf), 0);
    }

    #[test]
    fn heartbeat_echoes_server_timestamp() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b't', b'0', b'p', 0xdead_beef), 5)
            .unwrap();
        let a = p.take_actions();
        let Action::Send(bytes) = &a[0] else { panic!() };
        let stat = Stat::parse(&bytes[8..]).unwrap();
        assert_eq!(&stat.event, b"STMt");
        assert_eq!(stat.server_timestamp, 0xdead_beef);
    }

    #[test]
    fn autostart_off_reports_loaded_and_waits_for_unpause() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'0', b'p', 0), 0).unwrap();
        p.on_stream_data(&wav(44100, 44100), 1);
        p.on_stream_end(2);
        for i in 0..10 {
            p.tick(3 + i);
        }
        assert!(events(&p.take_actions()).contains(&"STMl".to_string()));
        assert_eq!(p.status().output, OutputState::Loaded);
        p.on_control_data(&strm(b'u', b'0', b'p', 0), 20).unwrap();
        assert_eq!(p.status().output, OutputState::Playing);
    }

    #[test]
    fn unsupported_format_is_refused() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'w', 0), 0).unwrap();
        let a = p.take_actions();
        assert!(!a.iter().any(|x| matches!(x, Action::OpenStream { .. })));
        assert_eq!(events(&a), vec!["STMn"]);
    }

    #[test]
    fn stream_numbers_count_every_strm_s_including_refused_ones() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'w', 0), 0).unwrap(); // unsupported: #1
        p.on_control_data(&strm(b's', b'1', b'p', 0), 1).unwrap(); // #2
        let numbers: Vec<u64> = p
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                Action::OpenStream { stream_number, .. } => Some(stream_number),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, vec![2]);
    }

    #[test]
    fn back_pressure_on_large_input() {
        let mut p = Player::new(cfg());
        p.on_control_data(&strm(b's', b'1', b'p', 0), 0).unwrap();
        assert!(p.wants_stream_data());
        p.on_stream_data(&vec![0u8; INPUT_HIGH_WATER], 1);
        assert!(!p.wants_stream_data());
    }
}
