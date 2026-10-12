//! Sync measurement harness: two players against a fake LMS that does what LMS does to
//! keep a group in step.
//!
//! The fake LMS:
//! - loads every player (`strm s`, autostart off) and waits for each one's `STMl`;
//! - sends `strm u` with a start time 200 ms ahead (its default sync start delay), in each
//!   player's jiffies, using a jiffies epoch learnt from STAT arrival times (earliest
//!   arrival, with its drift adjustment);
//! - about once a second, compares the players' play points (from each one's latest STAT
//!   `(jiffies, elapsed)`) and skips any player behind forward (`strm a`), as LMS does when
//!   every player can skip.
//!
//! Each player is the real core [`Player`] with the real output [`Ring`] and presentation
//! maths ([`clock`]), driven the way the Android engine drives them, in front of a simulated
//! output device with:
//! - a presentation latency (speaker, Bluetooth);
//! - a sample clock that drifts against true time;
//! - a monotonic clock that drifts against the server's;
//! - timestamp jitter, and a host tick with scheduling jitter.
//!
//! The network has jitter, with occasional spikes. Ground truth: every audio frame carries
//! its own frame number, so what each device presents at each moment is known exactly. The
//! harness measures the difference in audible position between the players.
//!
//! "Old" players report frames as handed to the output (no latency) and apply corrections
//! in the core, as before this change. They show the offset this change removes.

use std::sync::Arc;

use anywhere_core::player::{OutputCorrection, OutputState, Player, PlayerConfig};
use anywhere_core::proto::{ClientFrameReader, Stat, Strm, encode_server_frame};
use anywhere_mobile::player::clock::{self, Timestamp};
use anywhere_mobile::player::ring::{Consumer, Producer, Ring};

const RATE: u32 = 44_100;
const SECONDS: u32 = 40;
const TICK_MS: f64 = 20.0;
const RING_AHEAD_FRAMES: usize = RATE as usize * 400 / 1000;
const CALLBACK_FRAMES: usize = 441; // 10 ms
const SYNC_START_DELAY: f64 = 0.200;
const CHECK_SYNC_INTERVAL: f64 = 0.950;
const MIN_SYNC_ADJUST: f64 = 0.010;
const PACKET_LATENCY: f64 = 0.002;
/// Seconds the players are connected before the playlist starts.
const IDLE: f64 = 20.0;
/// LMS asks each player for its status this often.
const STATUS_QUERY_EVERY: f64 = 5.0;

/// xorshift: deterministic, so every run measures the same thing.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A 16-bit stereo WAV whose frame `f` carries `f`: left `f % 32768 - 16384`, right
/// `f / 32768 + 1` (never 0, so silence is told apart).
fn numbered_wav(frames: u32) -> Vec<u8> {
    let data_len = frames as usize * 4;
    let mut v = Vec::with_capacity(44 + data_len);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 4).to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data_len as u32).to_le_bytes());
    for f in 0..frames {
        let l = (f % 32768) as i32 - 16384;
        let r = (f / 32768) as i32 + 1;
        v.extend_from_slice(&(l as i16).to_le_bytes());
        v.extend_from_slice(&(r as i16).to_le_bytes());
    }
    v
}

/// The frame number a pair of output samples carries (`None`: silence).
fn frame_of(l: f32, r: f32) -> Option<u32> {
    if r == 0.0 {
        return None;
    }
    let l = (l * 32768.0).round() as i32 + 16384;
    let r = (r * 32768.0).round() as i32 - 1;
    Some(r as u32 * 32768 + l as u32)
}

#[derive(Clone, Copy, Debug)]
struct DeviceModel {
    /// Seconds from a frame being handed over to it being heard.
    latency: f64,
    /// The device's sample clock against true time (parts per million).
    device_ppm: f64,
    /// The player's monotonic clock against the server's (ppm), and its arbitrary origin.
    mono_ppm: f64,
    mono_origin_s: f64,
    /// Timestamp jitter (± seconds, uniform) on the time a frame is said to be heard.
    ts_jitter: f64,
    /// The host's tick scheduling jitter (up to this many ms late).
    tick_jitter_ms: f64,
    /// Report what is heard and apply corrections at the output (this change), or report
    /// what is handed over and correct in the core (before it).
    presented: bool,
}

struct SimPlayer {
    dev: DeviceModel,
    core: Player,
    ring: Arc<Ring>,
    producer: Producer,
    consumer: Consumer,
    wav: Vec<u8>,
    fed: usize,
    /// True time (s) the output stream started, and the next callback.
    stream_start: f64,
    next_callback: f64,
    next_tick: f64,
    /// What each device frame carried (`None`: silence).
    output: Vec<Option<u32>>,
    heard_pos: u64,
    read_pos_seen: u64,
    timed_start: bool,
    to_server: Vec<Vec<u8>>,
    buf: Vec<f32>,
}

impl SimPlayer {
    fn new(dev: DeviceModel, mac: u8, wav: Vec<u8>, stream_start: f64) -> SimPlayer {
        let mut core = Player::new(PlayerConfig {
            mac: [2, 0, 0, 0, 0, mac],
            uuid: [0; 16],
            name: format!("Sim {mac}"),
            codecs: vec!["pcm".into()],
            max_sample_rate: 192_000,
            firmware: "sim".into(),
        });
        core.set_output_corrections(dev.presented);
        let ring = Ring::new(1 << 17);
        let producer = ring.producer().unwrap();
        let consumer = ring.consumer().unwrap();
        ring.reset_device_clock();
        SimPlayer {
            dev,
            core,
            ring,
            producer,
            consumer,
            wav,
            fed: 0,
            stream_start,
            next_callback: stream_start,
            next_tick: 0.0,
            output: Vec::new(),
            heard_pos: 0,
            read_pos_seen: 0,
            timed_start: false,
            to_server: Vec::new(),
            buf: vec![0.0; 4096 * 2],
        }
    }

    fn mono_s(&self, t: f64) -> f64 {
        self.dev.mono_origin_s + t * (1.0 + self.dev.mono_ppm * 1e-6)
    }

    fn now_ms(&self, t: f64) -> u64 {
        (self.mono_s(t) * 1000.0) as u64
    }

    fn device_rate(&self) -> f64 {
        f64::from(RATE) * (1.0 + self.dev.device_ppm * 1e-6)
    }

    /// The device frame heard at true time `t` (frame 0 is heard at `stream_start`, and
    /// each frame is handed over `latency` before it is heard).
    fn heard_frame(&self, t: f64) -> Option<usize> {
        let f = (t - self.stream_start) * self.device_rate();
        (f >= 0.0).then_some(f as usize)
    }

    /// The content frame heard at true time `t` (`None`: silence).
    fn heard(&self, t: f64) -> Option<u32> {
        self.output.get(self.heard_frame(t)?).copied().flatten()
    }

    /// The output's timestamp, as AAudio would give it: a recent frame and when (on the
    /// player's monotonic clock) it was heard, with a little jitter.
    fn timestamp(&self, t: f64, rng: &mut Rng) -> Option<Timestamp> {
        let ts_t = t - 0.005;
        let frame = self.heard_frame(ts_t)?;
        let jitter = (rng.unit() * 2.0 - 1.0) * self.dev.ts_jitter;
        Some(Timestamp {
            frame: frame as i64,
            at_ns: ((self.mono_s(ts_t) + jitter) * 1e9) as i64,
        })
    }

    fn handle_control(&mut self, bytes: &[u8], t: f64) {
        let now = self.now_ms(t);
        self.core.on_control_data(bytes, now).unwrap();
        self.collect();
        self.apply_corrections(t, &mut Rng(1));
    }

    fn collect(&mut self) {
        for a in self.core.take_actions() {
            match a {
                anywhere_core::player::Action::Send(b) => self.to_server.push(b),
                anywhere_core::player::Action::OpenStream { .. } => {
                    self.core.on_stream_headers(b"HTTP/1.0 200 OK\r\n\r\n", 0);
                }
                anywhere_core::player::Action::CloseStream => {}
            }
        }
    }

    /// As the engine's `AudioPath::apply_correction`.
    fn apply_corrections(&mut self, t: f64, rng: &mut Rng) {
        let now_ns = (self.mono_s(t) * 1e9) as i64;
        for c in self.core.take_corrections() {
            let frames = |ms: u32| u64::from(ms) * u64::from(RATE) / 1000;
            match c {
                OutputCorrection::Pause { ms } => self.ring.insert_silence(frames(ms)),
                OutputCorrection::Skip { ms } => self.ring.skip(frames(ms)),
                OutputCorrection::StartAt { at_ms } => {
                    let target_ns = at_ms as i64 * 1_000_000;
                    let target = match self.timestamp(t, rng) {
                        Some(ts) if ts.usable(now_ns) => clock::frame_for(ts, RATE, target_ns),
                        _ => {
                            self.ring.device_frames() as i64
                                + (target_ns - now_ns) * i64::from(RATE) / 1_000_000_000
                        }
                    };
                    self.ring.start_at(target.max(0) as u64);
                    self.timed_start = true;
                }
            }
        }
    }

    /// One engine tick, as `Engine::on_tick`.
    fn tick(&mut self, t: f64, rng: &mut Rng) {
        let now = self.now_ms(t);
        let now_ns = (self.mono_s(t) * 1e9) as i64;
        // Feed the stream as the core takes it.
        while self.fed < self.wav.len() && self.core.wants_stream_data() {
            let end = (self.fed + 256 * 1024).min(self.wav.len());
            self.core.on_stream_data(&self.wav[self.fed..end], now);
            self.fed = end;
            if self.fed == self.wav.len() {
                self.core.on_stream_end(now);
            }
        }
        if self.dev.presented {
            let written = self.ring.device_frames();
            let dev = match self.timestamp(t, rng) {
                Some(ts) if ts.usable(now_ns) => clock::frame_at(ts, RATE, now_ns, written),
                _ => written,
            };
            let pos = self.ring.presented_pos(dev).max(self.heard_pos);
            let frames = (pos - self.heard_pos) / 2;
            self.heard_pos = pos;
            self.core.presented(frames, now);
        } else {
            let pos = self.ring.read_pos();
            self.core.played((pos - self.read_pos_seen) / 2);
            self.read_pos_seen = pos;
        }
        self.core.tick(now);
        self.collect();
        self.apply_corrections(t, rng);
        let state = self.core.status().output;
        let playing = state == OutputState::Playing;
        let timed = self.timed_start && matches!(state, OutputState::Paused { until: Some(_) });
        if self.timed_start && !timed {
            self.timed_start = false;
            self.ring.cancel_start();
        }
        if !timed {
            self.ring.set_hold(!playing);
        }
        // Fill the ring up to 400 ms ahead.
        while self.ring.len() / 2 < RING_AHEAD_FRAMES {
            let n = self.core.pull_audio(&mut self.buf);
            if n == 0 {
                break;
            }
            let w = self.producer.write(&self.buf[..n * 2]);
            assert_eq!(w, n * 2, "ring sized for the test");
        }
        self.core.tick(now);
        self.collect();
    }

    /// The output device asks for the next block.
    fn callback(&mut self) {
        let mut out = vec![0f32; CALLBACK_FRAMES * 2];
        self.consumer.read(&mut out);
        for f in out.chunks(2) {
            self.output.push(frame_of(f[0], f[1]));
        }
        self.next_callback += CALLBACK_FRAMES as f64 / self.device_rate();
    }
}

/// What LMS keeps per player for sync.
#[derive(Default)]
struct LmsPlayer {
    epoch: Option<f64>,
    diffs: Vec<f64>,
    last: Option<(u32, u32, f64)>, // jiffies, elapsed ms, received at
    loaded: bool,
}

impl LmsPlayer {
    /// `Slim::Player::Player::trackJiffiesEpoch`, in outline: the earliest-arrival offset,
    /// raised when every recent arrival is later (the player's clock running slow).
    fn track_epoch(&mut self, jiffies: u32, now: f64) {
        let offset = now - f64::from(jiffies) / 1000.0;
        match self.epoch {
            None => self.epoch = Some(offset),
            Some(e) if offset < e || offset - e > 50.0 => {
                self.epoch = Some(offset);
                self.diffs.clear();
            }
            Some(e) => {
                self.diffs.push(offset - e);
                if self.diffs.len() >= 10 {
                    let min = self.diffs.iter().copied().fold(f64::MAX, f64::min);
                    if min > 0.001 && (min <= 0.005 || self.diffs.len() >= 50) {
                        self.epoch = Some(e + min);
                        self.diffs.clear();
                    } else if self.diffs.len() >= 50 {
                        self.diffs.clear();
                    }
                }
            }
        }
    }

    fn timestamp(&self, jiffies: u32) -> f64 {
        self.epoch.unwrap() + f64::from(jiffies) / 1000.0 - PACKET_LATENCY
    }

    /// The apparent start of the stream (server time at which elapsed was 0).
    fn apparent_start(&self, now: f64) -> Option<f64> {
        let (j, e, _) = self.last?;
        let status = self.timestamp(j);
        (e > 0 && now - status < 3.0).then(|| status - f64::from(e) / 1000.0)
    }
}

struct Link {
    queue: Vec<(f64, Vec<u8>)>,
    last: f64,
}

impl Link {
    fn new() -> Link {
        Link {
            queue: Vec::new(),
            last: 0.0,
        }
    }

    /// Send at `t` with a delay (in order, as TCP delivers).
    fn send(&mut self, t: f64, delay: f64, bytes: Vec<u8>) {
        let at = (t + delay).max(self.last);
        self.last = at;
        self.queue.push((at, bytes));
    }

    fn due(&mut self, t: f64) -> Vec<Vec<u8>> {
        let (due, rest): (Vec<_>, Vec<_>) = self.queue.drain(..).partition(|(at, _)| *at <= t);
        self.queue = rest;
        due.into_iter().map(|(_, b)| b).collect()
    }
}

fn strm(command: u8, autostart: u8, arg: u32) -> Vec<u8> {
    let s = Strm {
        command,
        autostart,
        format: b'p',
        pcm_sample_size: b'?',
        pcm_sample_rate: b'?',
        pcm_channels: b'?',
        pcm_endianness: b'?',
        threshold: 255,
        spdif_enable: 0,
        transition_period: 0,
        transition_type: b'0',
        flags: 0,
        output_threshold: 0,
        slaves: 0,
        replay_gain: arg,
        server_port: 9000,
        server_ip: 0,
        request: if command == b's' {
            b"GET /stream.pcm HTTP/1.0\r\n\r\n".to_vec()
        } else {
            Vec::new()
        },
    };
    encode_server_frame(*b"strm", &s.encode_payload())
}

#[derive(Debug)]
struct Measured {
    /// The difference in audible position as soon as both play (the timed start, before
    /// any correction), in ms.
    start_offset_ms: f64,
    /// Over the settled part (from 3 s after the start): the largest and the mean
    /// absolute difference in audible position, in ms.
    max_ms: f64,
    mean_ms: f64,
    skips: u32,
}

/// Run a group of two players and measure how far apart they sound.
fn run(a: DeviceModel, b: DeviceModel, seed: u64) -> Measured {
    let mut rng = Rng(seed);
    let wav = numbered_wav(RATE * SECONDS);
    let mut players = [
        SimPlayer::new(a, 1, wav.clone(), 0.0123),
        SimPlayer::new(b, 2, wav, 0.0371),
    ];
    let mut lms = [LmsPlayer::default(), LmsPlayer::default()];
    let mut down = [Link::new(), Link::new()];
    let mut up = [Link::new(), Link::new()];
    let mut readers = [ClientFrameReader::default(), ClientFrameReader::default()];
    let mut started_at = None;
    let mut next_check = 0.0;
    let mut skips = 0;
    let net = |rng: &mut Rng| {
        let spike = if rng.unit() < 0.05 { 0.05 } else { 0.0 };
        0.002 + rng.unit() * 0.015 + spike
    };

    for p in players.iter_mut() {
        p.core.connected(p.now_ms(0.0));
        p.collect();
    }
    // The players sit connected for a while first (as players usually have been when a
    // group starts), answering LMS's status queries: that is how LMS learns their clocks.
    let mut next_query = 1.0;
    let mut playlist_sent = false;
    let end = IDLE + f64::from(SECONDS) - 5.0;
    let step = 0.0005;
    let mut t = 0.0;
    let mut diffs = Vec::new();
    let mut start_offset_ms = f64::NAN;
    while t < end {
        t += step;
        if t >= next_query {
            next_query += STATUS_QUERY_EVERY;
            for link in &mut down {
                let d = net(&mut rng);
                link.send(t, d, strm(b't', b'0', 0));
            }
        }
        if !playlist_sent && t >= IDLE {
            playlist_sent = true;
            for link in &mut down {
                let d = net(&mut rng);
                link.send(t, d, strm(b's', b'0', 0));
            }
        }
        for i in 0..2 {
            for bytes in down[i].due(t) {
                players[i].handle_control(&bytes, t);
            }
            while players[i].next_callback <= t + players[i].dev.latency {
                // A block is handed over `latency` before it is heard.
                players[i].callback();
            }
            if t >= players[i].next_tick {
                players[i].tick(t, &mut rng);
                players[i].next_tick =
                    t + (TICK_MS + rng.unit() * players[i].dev.tick_jitter_ms) / 1000.0;
            }
            for bytes in std::mem::take(&mut players[i].to_server) {
                let d = net(&mut rng);
                up[i].send(t, d, bytes);
            }
            for bytes in up[i].due(t) {
                readers[i].push(&bytes);
                while let Some((op, payload)) = readers[i].next_frame().unwrap() {
                    if &op != b"STAT" {
                        continue;
                    }
                    let s = Stat::parse(&payload).unwrap();
                    lms[i].track_epoch(s.jiffies, t);
                    lms[i].last = Some((s.jiffies, s.elapsed_milliseconds, t));
                    if &s.event == b"STMl" {
                        lms[i].loaded = true;
                    }
                }
            }
        }
        // Start the group once every player has loaded.
        if started_at.is_none() && lms.iter().all(|l| l.loaded) {
            let at = t + SYNC_START_DELAY;
            started_at = Some(at);
            for i in 0..2 {
                let jiffies = ((at - lms[i].epoch.unwrap()) * 1000.0) as u32;
                let d = net(&mut rng);
                down[i].send(t, d, strm(b'u', b'0', jiffies));
            }
            next_check = at + 2.0;
        }
        // Maintain sync: skip whoever is behind the one furthest ahead.
        if let Some(at) = started_at
            && t >= next_check
            && t > at
        {
            next_check = t + CHECK_SYNC_INTERVAL;
            let points: Vec<_> = lms.iter().map(|l| l.apparent_start(t)).collect();
            if let [Some(p0), Some(p1)] = points[..] {
                let (behind, delta) = if p0 > p1 { (0, p0 - p1) } else { (1, p1 - p0) };
                if (MIN_SYNC_ADJUST..=10.0).contains(&delta) {
                    let ms = (delta * 1000.0) as u32;
                    let d = net(&mut rng);
                    down[behind].send(t, d, strm(b'a', b'0', ms));
                    next_check += 1.0;
                    skips += 1;
                }
            }
        }
        // Measure every 5 ms.
        if let Some(at) = started_at
            && ((t * 2000.0).round() as u64).is_multiple_of(10)
        {
            let (h0, h1) = (players[0].heard(t), players[1].heard(t));
            if start_offset_ms.is_nan()
                && let (Some(f0), Some(f1)) = (h0, h1)
            {
                // How far apart they sound once both are playing, before any correction.
                start_offset_ms = (f64::from(f0) - f64::from(f1)) * 1000.0 / f64::from(RATE);
            }
            if t > at + 3.0
                && let (Some(f0), Some(f1)) = (h0, h1)
            {
                diffs.push((f64::from(f0) - f64::from(f1)) * 1000.0 / f64::from(RATE));
            }
        }
    }
    assert!(started_at.is_some(), "the group never started");
    assert!(!diffs.is_empty(), "nothing audible to measure");
    let max_ms = diffs.iter().map(|d| d.abs()).fold(0.0, f64::max);
    let mean_ms = diffs.iter().map(|d| d.abs()).sum::<f64>() / diffs.len() as f64;
    Measured {
        start_offset_ms,
        max_ms,
        mean_ms,
        skips,
    }
}

/// A hardware player: reports exactly what it plays, a few ms of latency.
fn hardware() -> DeviceModel {
    DeviceModel {
        latency: 0.002,
        device_ppm: 0.0,
        mono_ppm: 0.0,
        mono_origin_s: 1234.5,
        ts_jitter: 0.0,
        tick_jitter_ms: 1.0,
        presented: true,
    }
}

fn phone(latency: f64, presented: bool) -> DeviceModel {
    DeviceModel {
        latency,
        device_ppm: 60.0,
        mono_ppm: -25.0,
        mono_origin_s: 98_765.432,
        ts_jitter: 0.0005,
        tick_jitter_ms: 4.0,
        presented,
    }
}

fn report(name: &str, m: &Measured) {
    eprintln!(
        "{name}: start offset {:.1} ms, settled max {:.1} ms, mean {:.1} ms, {} skips",
        m.start_offset_ms, m.max_ms, m.mean_ms, m.skips
    );
}

#[test]
fn phone_speaker_stays_in_step_with_a_hardware_player() {
    for seed in [1, 2, 3] {
        let m = run(hardware(), phone(0.030, true), seed);
        report("speaker (30 ms)", &m);
        assert!(m.start_offset_ms.abs() <= 15.0, "{m:?}");
        assert!(m.max_ms <= 20.0 && m.mean_ms <= 8.0, "{m:?}");
    }
}

#[test]
fn bluetooth_stays_in_step_with_a_hardware_player() {
    for seed in [4, 5, 6] {
        let m = run(hardware(), phone(0.250, true), seed);
        report("Bluetooth (250 ms)", &m);
        // 250 ms of latency is more than LMS's 200 ms start delay: the start catches up by
        // dropping what it missed, so it is still in step.
        assert!(m.start_offset_ms.abs() <= 15.0, "{m:?}");
        assert!(m.max_ms <= 20.0 && m.mean_ms <= 8.0, "{m:?}");
    }
}

/// A browser through Web Audio: tens of ms of latency, a jittery timestamp and tick.
#[test]
fn web_audio_like_output_stays_in_step() {
    for seed in [7, 8] {
        let mut web = phone(0.060, true);
        web.ts_jitter = 0.003;
        web.tick_jitter_ms = 15.0;
        let m = run(hardware(), web, seed);
        report("Web Audio-like (60 ms, jittery)", &m);
        assert!(m.max_ms <= 30.0 && m.mean_ms <= 12.0, "{m:?}");
    }
}

/// Two of our players with different routes, both drifting.
#[test]
fn two_phones_on_different_routes_stay_in_step() {
    let mut fast = phone(0.020, true);
    fast.device_ppm = 120.0;
    let mut slow = phone(0.180, true);
    slow.device_ppm = -90.0;
    slow.mono_ppm = 40.0;
    slow.mono_origin_s = 5.0;
    let m = run(fast, slow, 9);
    report("speaker vs Bluetooth, drifting", &m);
    assert!(m.max_ms <= 20.0 && m.mean_ms <= 8.0, "{m:?}");
}

/// Before this change: frames counted as played when handed to the output. LMS believes the
/// players are in step, but the Bluetooth player is heard its latency late.
#[test]
fn reporting_handed_over_frames_leaves_the_latency_as_an_offset() {
    let m = run(hardware(), phone(0.250, false), 10);
    report("before: Bluetooth (250 ms), handed-over frames", &m);
    assert!(m.mean_ms >= 200.0, "{m:?}");
    let m = run(hardware(), phone(0.030, false), 11);
    report("before: speaker (30 ms), handed-over frames", &m);
    assert!(m.mean_ms >= 20.0, "{m:?}");
}
