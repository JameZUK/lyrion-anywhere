//! Streaming track analysis for the gateway's analysis service (plan §4, milestone A3):
//! loudness, peaks, waveform, silence and fades, in one pass and constant memory however
//! long the track is.
//!
//! - **Loudness** per ITU-R BS.1770-4 / EBU R128: K-weighting at any sample rate
//!   ([`kweight`]), 400 ms blocks with 75 % overlap, absolute (-70 LUFS) and relative
//!   (-10 LU) gates → integrated loudness; maximum momentary and short-term loudness.
//! - **Loudness range** per EBU Tech 3342: 3 s short-term blocks every 100 ms, -70 LUFS
//!   absolute and -20 LU relative gates, the 10th to 95th percentile spread.
//! - **True peak** (4× oversampling, BS.1770-4 Annex 2 method, shared with the limiter) and
//!   sample peak.
//! - **Waveform**: peak and RMS per bucket, see [`waveform`] for the streaming method.
//! - **Silence and fades**: leading and trailing silence below a threshold, and the
//!   loudness slope over the first and last seconds of audio, flagged as a fade when it
//!   is steep and deep enough. Smart transitions use these.
//!
//! Channel weights follow BS.1770: 1.0 for front channels, 1.41 for surrounds, 0 for the
//! LFE (six channels are taken as L R C LFE Ls Rs, five as L R C Ls Rs). A mono signal is
//! measured as one channel, so it reads 3 LU lower than the same signal on two channels.
//!
//! Non-finite samples are counted in [`TrackAnalysis::invalid_samples`] and treated as
//! silence. A change of sample rate mid-stream is refused with
//! [`AnalysisError::RateChanged`]; the caller should restart.

pub mod encode;
pub mod kweight;
pub mod loudness;
pub mod waveform;

use serde::{Deserialize, Serialize};

use crate::dsp::biquad::Coeffs;
use crate::dsp::truepeak::TruePeakDetector;
use loudness::{GatingHistogram, lufs_to_power, power_to_lufs};
use waveform::WaveformBuilder;

/// Version of [`TrackAnalysis`]'s meaning and encodings. Bump when either changes.
pub const ANALYSIS_VERSION: u16 = 1;

/// Supported sample rates and channel counts.
pub const MIN_RATE: u32 = 8_000;
pub const MAX_RATE: u32 = 768_000;
pub const MAX_CHANNELS: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisConfig {
    /// Waveform buckets (1..=10000).
    pub buckets: usize,
    /// Level below which a frame counts as silent, dBFS.
    pub silence_threshold_dbfs: f64,
    /// Shortest leading or trailing silence reported, s (shorter counts as none).
    pub min_silence_s: f64,
    /// Seconds at each end examined for fades.
    pub fade_window_s: f64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        AnalysisConfig {
            buckets: waveform::DEFAULT_BUCKETS,
            silence_threshold_dbfs: -60.0,
            min_silence_s: 0.2,
            fade_window_s: 8.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisError {
    UnsupportedRate(u32),
    UnsupportedChannels(usize),
    /// The stream changed sample rate; analysis must restart.
    RateChanged {
        expected: u32,
        got: u32,
    },
    /// A buffer ended part-way through a frame.
    PartialFrame,
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalysisError::UnsupportedRate(r) => write!(f, "unsupported sample rate {r}"),
            AnalysisError::UnsupportedChannels(c) => write!(f, "unsupported channel count {c}"),
            AnalysisError::RateChanged { expected, got } => {
                write!(f, "sample rate changed from {expected} to {got}")
            }
            AnalysisError::PartialFrame => write!(f, "buffer ends mid-frame"),
        }
    }
}

impl std::error::Error for AnalysisError {}

/// Loudness behaviour at one end of a track.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Edge {
    /// Least-squares slope of momentary loudness, dB per second (positive = rising).
    pub slope_db_per_s: f64,
    /// Loudness over the first and last half second of the window, LUFS.
    pub start_lufs: Option<f64>,
    pub end_lufs: Option<f64>,
    /// Estimated fade length, s (0 when not a fade).
    pub length_s: f64,
    /// Steep and deep enough to be a deliberate fade.
    pub fade: bool,
}

/// Waveform buckets, quantised linearly (0 = silence, 255 = full scale).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Waveform {
    #[serde(with = "encode::b64")]
    pub peak: Vec<u8>,
    #[serde(with = "encode::b64")]
    pub rms: Vec<u8>,
}

/// The result of analysing one track. Values are rounded to 0.01.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackAnalysis {
    pub version: u16,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Integrated loudness, LUFS (`None` if nothing passes the gates, e.g. silence).
    pub integrated_lufs: Option<f64>,
    /// Loudness range, LU (`None` for tracks too short or quiet to measure).
    pub loudness_range_lu: Option<f64>,
    pub max_momentary_lufs: Option<f64>,
    pub max_short_term_lufs: Option<f64>,
    /// True peak, dBTP (`None` for digital silence).
    pub true_peak_dbtp: Option<f64>,
    pub sample_peak_dbfs: Option<f64>,
    pub leading_silence_s: f64,
    pub trailing_silence_s: f64,
    pub fade_in: Edge,
    pub fade_out: Edge,
    /// Non-finite input samples (treated as silence).
    pub invalid_samples: u64,
    pub waveform: Waveform,
}

impl TrackAnalysis {
    pub fn duration_s(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames as f64 / f64::from(self.sample_rate)
        }
    }
}

/// Album loudness and true peak from its tracks' analyses.
///
/// The loudness is the duration-weighted power mean of the tracks' integrated loudness.
/// That equals the gated loudness of the whole album whenever the relative gate removes
/// the same proportion of each track, and is within a few tenths of an LU for typical
/// albums. The peak is the loudest track's.
pub fn album_loudness(tracks: &[TrackAnalysis]) -> Option<(f64, Option<f64>)> {
    let mut power = 0.0;
    let mut dur = 0.0;
    let mut peak: Option<f64> = None;
    for t in tracks {
        if let Some(l) = t.integrated_lufs {
            let d = t.duration_s();
            power += lufs_to_power(l) * d;
            dur += d;
        }
        if let Some(p) = t.true_peak_dbtp {
            peak = Some(peak.map_or(p, |q| q.max(p)));
        }
    }
    (dur > 0.0).then(|| (round2(power_to_lufs(power / dur)), peak))
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn opt_db(x: f64) -> Option<f64> {
    x.is_finite().then(|| round2(x))
}

/// Samples the analyser accepts.
pub trait Sample: Copy {
    fn to_f64(self) -> f64;
}

impl Sample for f32 {
    #[inline]
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
}

impl Sample for f64 {
    #[inline]
    fn to_f64(self) -> f64 {
        self
    }
}

/// Streaming analyser: create, [`push`](Analyser::push) interleaved samples, then
/// [`finish`](Analyser::finish).
pub struct Analyser {
    config: AnalysisConfig,
    rate: u32,
    channels: usize,
    weights: Vec<f64>,
    k: [Coeffs; 2],
    /// Per channel K-filter state: [s1, s2] for each of the two stages.
    kstate: Vec<[f64; 4]>,
    tp: TruePeakDetector,
    frame: Vec<f64>,
    frames: u64,
    invalid: u64,
    sample_peak: f64,
    true_peak: f64,
    // Sub-blocks.
    hop: usize,
    in_hop: usize,
    sub_sum: Vec<f64>,
    sub_count: u64,
    ring: [f64; 30],
    ring_pos: usize,
    gating: GatingHistogram,
    short_term: GatingHistogram,
    max_momentary: f64,
    max_short_term: f64,
    // Silence.
    silence_threshold: f64,
    first_loud: Option<u64>,
    last_loud: u64,
    // Fades: momentary loudness per sub-block at each end.
    fade_len: usize,
    head: Vec<f64>,
    head_start: Option<u64>,
    tail: Vec<f64>,
    tail_pos: usize,
    tail_count: u64,
    waveform: WaveformBuilder,
}

impl Analyser {
    pub fn new(rate: u32, channels: usize, config: AnalysisConfig) -> Result<Self, AnalysisError> {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) {
            return Err(AnalysisError::UnsupportedRate(rate));
        }
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(AnalysisError::UnsupportedChannels(channels));
        }
        let weights = (0..channels)
            .map(|c| match (channels, c) {
                (6, 3) => 0.0,
                (6, 4 | 5) | (5, 3 | 4) => 1.41,
                _ => 1.0,
            })
            .collect();
        let fade_window = if config.fade_window_s.is_finite() {
            config.fade_window_s.clamp(1.0, 60.0)
        } else {
            8.0
        };
        let fade_len = (fade_window * 10.0).round() as usize;
        let thr = if config.silence_threshold_dbfs.is_finite() {
            config.silence_threshold_dbfs.clamp(-150.0, 0.0)
        } else {
            -60.0
        };
        Ok(Analyser {
            waveform: WaveformBuilder::new(config.buckets),
            config,
            rate,
            channels,
            weights,
            k: kweight::k_weighting(rate),
            kstate: vec![[0.0; 4]; channels],
            tp: TruePeakDetector::new(channels),
            frame: vec![0.0; channels],
            frames: 0,
            invalid: 0,
            sample_peak: 0.0,
            true_peak: 0.0,
            hop: ((f64::from(rate) / 10.0).round() as usize).max(1),
            in_hop: 0,
            sub_sum: vec![0.0; channels],
            sub_count: 0,
            ring: [0.0; 30],
            ring_pos: 0,
            gating: GatingHistogram::default(),
            short_term: GatingHistogram::default(),
            max_momentary: f64::NEG_INFINITY,
            max_short_term: f64::NEG_INFINITY,
            silence_threshold: crate::dsp::db_to_gain(thr),
            first_loud: None,
            last_loud: 0,
            fade_len,
            head: Vec::with_capacity(fade_len),
            head_start: None,
            tail: vec![0.0; fade_len],
            tail_pos: 0,
            tail_count: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Push samples, checking they are still at the rate the analysis started with.
    pub fn push_with_rate<S: Sample>(
        &mut self,
        rate: u32,
        samples: &[S],
    ) -> Result<(), AnalysisError> {
        if rate != self.rate {
            return Err(AnalysisError::RateChanged {
                expected: self.rate,
                got: rate,
            });
        }
        self.push(samples)
    }

    /// Push interleaved samples (whole frames only).
    pub fn push<S: Sample>(&mut self, samples: &[S]) -> Result<(), AnalysisError> {
        let ch = self.channels;
        if !samples.len().is_multiple_of(ch) {
            return Err(AnalysisError::PartialFrame);
        }
        let [pre, rlb] = self.k;
        for f in samples.chunks_exact(ch) {
            let mut fpeak: f64 = 0.0;
            let mut fsq = 0.0;
            for (c, s) in f.iter().enumerate() {
                let mut x = s.to_f64();
                if !x.is_finite() {
                    self.invalid += 1;
                    x = 0.0;
                }
                self.frame[c] = x;
                let a = x.abs();
                if a > fpeak {
                    fpeak = a;
                }
                fsq += x * x;
                // K-weighting: two TDF-II biquads.
                let s = &mut self.kstate[c];
                let y1 = pre.b0 * x + s[0];
                s[0] = pre.b1 * x - pre.a1 * y1 + s[1];
                s[1] = pre.b2 * x - pre.a2 * y1;
                let y2 = rlb.b0 * y1 + s[2];
                s[2] = rlb.b1 * y1 - rlb.a1 * y2 + s[3];
                s[3] = rlb.b2 * y1 - rlb.a2 * y2;
                self.sub_sum[c] += y2 * y2;
            }
            if fpeak > self.sample_peak {
                self.sample_peak = fpeak;
            }
            let tp = self.tp.push_frame_above(&self.frame, self.true_peak);
            if tp > self.true_peak {
                self.true_peak = tp;
            }
            if fpeak > self.silence_threshold {
                if self.first_loud.is_none() {
                    self.first_loud = Some(self.frames);
                }
                self.last_loud = self.frames;
            }
            self.waveform.push(fpeak, fsq / ch as f64);
            self.frames += 1;
            self.in_hop += 1;
            if self.in_hop == self.hop {
                self.end_sub_block();
            }
        }
        Ok(())
    }

    fn end_sub_block(&mut self) {
        let hop = self.hop as f64;
        let mut p = 0.0;
        for c in 0..self.channels {
            p += self.weights[c] * self.sub_sum[c] / hop;
            self.sub_sum[c] = 0.0;
            for s in &mut self.kstate[c] {
                *s = crate::dsp::flush_denormal(*s);
            }
        }
        self.in_hop = 0;
        self.ring[self.ring_pos] = p;
        self.ring_pos = (self.ring_pos + 1) % 30;
        self.sub_count += 1;
        if self.sub_count >= 4 {
            let block: f64 = (0..4)
                .map(|k| self.ring[(self.ring_pos + 30 - 1 - k) % 30])
                .sum::<f64>()
                / 4.0;
            self.gating.add(block);
            let l = power_to_lufs(block);
            if l > self.max_momentary {
                self.max_momentary = l;
            }
            // Fade tracking, once audio has started.
            let l = if l.is_finite() { l.max(-70.0) } else { -70.0 };
            if self.first_loud.is_some() {
                if self.head_start.is_none() {
                    self.head_start = Some(self.sub_count);
                }
                if self.head.len() < self.fade_len {
                    self.head.push(l);
                }
            }
            self.tail[self.tail_pos] = l;
            self.tail_pos = (self.tail_pos + 1) % self.fade_len;
            self.tail_count += 1;
        }
        if self.sub_count >= 30 {
            let st = self.ring.iter().sum::<f64>() / 30.0;
            self.short_term.add(st);
            let l = power_to_lufs(st);
            if l > self.max_short_term {
                self.max_short_term = l;
            }
        }
    }

    /// Complete the analysis.
    pub fn finish(mut self) -> TrackAnalysis {
        // Flush the true-peak interpolator as if followed by silence.
        let zero = vec![0.0; self.channels];
        for _ in 0..crate::dsp::truepeak::HALF_TAPS {
            self.true_peak = self.true_peak.max(self.tp.push_frame(&zero));
        }
        let rate = f64::from(self.rate);
        let min_silence = if self.config.min_silence_s.is_finite() {
            self.config.min_silence_s.max(0.0)
        } else {
            0.0
        };
        let (lead, trail) = match self.first_loud {
            Some(first) => (first, self.frames - 1 - self.last_loud),
            None => (self.frames, self.frames),
        };
        let secs = |frames: u64| {
            let s = frames as f64 / rate;
            if s >= min_silence { round2(s) } else { 0.0 }
        };
        let leading_silence_s = secs(lead);
        let trailing_silence_s = secs(trail);

        // Fades: the tail ring in time order, minus the trailing silence.
        let fade_in = edge(&self.head, true);
        let n = (self.tail_count as usize).min(self.fade_len);
        let mut tail: Vec<f64> = (0..n)
            .map(|k| self.tail[(self.tail_pos + self.fade_len - n + k) % self.fade_len])
            .collect();
        let drop = ((trail as f64 / rate) * 10.0).floor() as usize;
        tail.truncate(tail.len().saturating_sub(drop));
        let fade_out = edge(&tail, false);

        let (peak, rms) = self.waveform.finish();
        TrackAnalysis {
            version: ANALYSIS_VERSION,
            sample_rate: self.rate,
            channels: self.channels as u16,
            frames: self.frames,
            integrated_lufs: self.gating.integrated().map(round2),
            loudness_range_lu: self.short_term.loudness_range().map(round2),
            max_momentary_lufs: opt_db(self.max_momentary).filter(|l| *l > -70.0),
            max_short_term_lufs: opt_db(self.max_short_term).filter(|l| *l > -70.0),
            true_peak_dbtp: (self.true_peak > 0.0)
                .then(|| round2(crate::dsp::gain_to_db(self.true_peak))),
            sample_peak_dbfs: (self.sample_peak > 0.0)
                .then(|| round2(crate::dsp::gain_to_db(self.sample_peak))),
            leading_silence_s,
            trailing_silence_s,
            fade_in,
            fade_out,
            invalid_samples: self.invalid,
            waveform: Waveform { peak, rms },
        }
    }
}

/// Slope and fade classification of momentary loudness values 100 ms apart.
fn edge(v: &[f64], rising: bool) -> Edge {
    if v.len() < 10 {
        return Edge::default();
    }
    let n = v.len() as f64;
    let mean_t = (n - 1.0) / 2.0 * 0.1;
    let mean_l = v.iter().sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, l) in v.iter().enumerate() {
        let t = i as f64 * 0.1 - mean_t;
        num += t * (l - mean_l);
        den += t * t;
    }
    let slope = if den > 0.0 { num / den } else { 0.0 };
    let avg = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let start = avg(&v[..5]);
    let end = avg(&v[v.len() - 5..]);
    let max = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let fade = if rising {
        slope >= 1.0 && end - start >= 6.0
    } else {
        slope <= -1.0 && start - end >= 6.0
    };
    let length_s = if !fade {
        0.0
    } else if rising {
        v.iter().position(|l| *l >= max - 1.0).unwrap_or(0) as f64 * 0.1
    } else {
        let last = v
            .iter()
            .rposition(|l| *l >= max - 1.0)
            .unwrap_or(v.len() - 1);
        (v.len() - 1 - last) as f64 * 0.1
    };
    Edge {
        slope_db_per_s: round2(slope),
        start_lufs: Some(round2(start)),
        end_lufs: Some(round2(end)),
        length_s: round2(length_s),
        fade,
    }
}

/// Why [`analyse_source`] gave up.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalyseError {
    /// Not decodable: unsupported, corrupt or truncated.
    Decode(String),
    /// Longer than the limit.
    TooLong,
    /// `should_stop` said so (e.g. a deadline passed).
    Stopped,
    /// Decoded, but held no audio.
    NoAudio,
    /// The audio's shape is outside what the analyser supports (rate, rate changes).
    Unsupported(AnalysisError),
}

impl std::fmt::Display for AnalyseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyseError::Decode(e) => write!(f, "{e}"),
            AnalyseError::TooLong => write!(f, "too long"),
            AnalyseError::Stopped => write!(f, "stopped"),
            AnalyseError::NoAudio => write!(f, "no audio"),
            AnalyseError::Unsupported(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AnalyseError {}

/// Decode a whole, seekable file (see [`crate::decode::decode_seekable`]) and analyse it in
/// one pass, in constant memory.
///
/// - `max_seconds`: audio beyond this is refused with [`AnalyseError::TooLong`].
/// - `should_stop`: polled between packets; `true` abandons the work with
///   [`AnalyseError::Stopped`] (the host's deadline or shutdown).
///
/// The decoder delivers stereo, so mono files are measured as dual mono, as they are played.
#[cfg(feature = "player")]
pub fn analyse_source(
    source: Box<dyn crate::decode::MediaSource>,
    ext_hint: Option<&str>,
    config: AnalysisConfig,
    max_seconds: f64,
    should_stop: &dyn Fn() -> bool,
) -> Result<TrackAnalysis, AnalyseError> {
    let mut an: Option<Analyser> = None;
    let mut err: Option<AnalyseError> = None;
    let max_seconds = if max_seconds.is_finite() {
        max_seconds.max(0.0)
    } else {
        f64::MAX
    };
    let decoded = crate::decode::decode_seekable(source, ext_hint, &mut |rate, pcm| {
        if should_stop() {
            err = Some(AnalyseError::Stopped);
            return false;
        }
        if an.is_none() {
            match Analyser::new(rate, 2, config.clone()) {
                Ok(a) => an = Some(a),
                Err(e) => {
                    err = Some(AnalyseError::Unsupported(e));
                    return false;
                }
            }
        }
        let Some(a) = &mut an else { return false };
        if let Err(e) = a.push_with_rate(rate, pcm) {
            err = Some(AnalyseError::Unsupported(e));
            return false;
        }
        if a.frames as f64 > max_seconds * f64::from(a.rate) {
            err = Some(AnalyseError::TooLong);
            return false;
        }
        true
    });
    if let Some(e) = err {
        return Err(e);
    }
    decoded.map_err(AnalyseError::Decode)?;
    match an {
        Some(a) if a.frames > 0 => Ok(a.finish()),
        _ => Err(AnalyseError::NoAudio),
    }
}

/// Decode an encoded file (any format the player decodes) and analyse it. The decoder
/// delivers stereo, so mono files are measured as dual mono, as they are played.
#[cfg(feature = "player")]
pub fn analyse_encoded(
    data: &[u8],
    format_hint: u8,
    config: AnalysisConfig,
) -> Result<TrackAnalysis, String> {
    use crate::decode::{DecodeStatus, InputBuffer, StreamDecoder};
    let input = InputBuffer::new();
    input.push(data);
    input.set_eof();
    let mut dec = StreamDecoder::new(input, format_hint);
    let mut an: Option<Analyser> = None;
    let mut err: Option<String> = None;
    loop {
        let st = dec.work(64, &mut |rate, pcm| {
            if err.is_some() {
                return;
            }
            if an.is_none() {
                match Analyser::new(rate, 2, config.clone()) {
                    Ok(a) => an = Some(a),
                    Err(e) => {
                        err = Some(e.to_string());
                        return;
                    }
                }
            }
            if let Some(a) = &mut an
                && let Err(e) = a.push_with_rate(rate, pcm)
            {
                err = Some(e.to_string());
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        match st {
            DecodeStatus::Progress => {}
            DecodeStatus::Finished => break,
            DecodeStatus::NeedData => return Err("truncated stream".into()),
            DecodeStatus::Failed(e) => return Err(e),
        }
    }
    an.map(Analyser::finish).ok_or_else(|| "no audio".into())
}
