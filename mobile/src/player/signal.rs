//! The signal-path snapshot for the app's panel: what is playing, every stage that touches
//! it, and an honest verdict.
//!
//! Two verdicts are kept apart:
//! - `engine_bit_perfect`: our own path (decode → volume → DSP → output stream) leaves
//!   every sample untouched, at the source's rate;
//! - `bit_perfect`: that, and the platform delivers the stream to the device untouched.
//!   On Android's normal (shared, mixed) output that can't be promised, so it is only true
//!   when the app reports a direct route (bit-perfect USB mixer attributes, milestone A2).

use anywhere_core::dsp::signal_path::{OutputInfo, SignalPath, SourceInfo, Stage, Verdict, fmt_db};
use anywhere_core::player::TrackAudio;

use super::{SignalPathInfo, SignalStage, SignalVerdict};

pub(crate) struct Inputs {
    pub audio: TrackAudio,
    /// LMS volume (left, right); (1, 1) is unity.
    pub volume: (f32, f32),
    pub pipeline_stages: Vec<Stage>,
    /// The pipeline changes samples (DSP or resampling).
    pub dsp_active: bool,
    /// Output gain for ducking (1.0 = none).
    pub duck: f32,
    /// The output stream's rate.
    pub out_rate: u32,
    pub device: String,
    /// The platform mixer's rate, if known (0 = unknown).
    pub mixer_rate: u32,
    /// The platform passes our stream to the device untouched.
    pub direct: bool,
}

fn gain_db(g: f32) -> f64 {
    20.0 * f64::from(g).log10()
}

pub(crate) fn describe(i: &Inputs) -> SignalPathInfo {
    let src = i.audio.source;
    let source = SourceInfo {
        codec: src.map_or("Unknown", |s| s.codec).to_string(),
        lossless: src.is_some_and(|s| s.lossless),
        sample_rate: i.audio.sample_rate,
        bits: src.and_then(|s| s.bits),
        channels: src.map_or(2, |s| s.channels as usize),
    };
    let mut stages = Vec::new();
    if source.channels == 1 {
        stages.push(Stage::transparent("Mono to stereo"));
    } else if source.channels > 2 {
        stages.push(Stage::processing(format!(
            "Downmix {} channels to stereo (front pair)",
            source.channels
        )));
    }
    if i.audio.server_gain != 1.0 {
        stages.push(Stage::processing(format!(
            "ReplayGain (server) {}",
            fmt_db(gain_db(i.audio.server_gain))
        )));
    }
    let (l, r) = i.volume;
    if l != 1.0 || r != 1.0 {
        let db = gain_db(l.max(r));
        stages.push(Stage::processing(if db.is_finite() {
            format!("Volume {}", fmt_db(db))
        } else {
            "Volume (muted)".into()
        }));
    }
    stages.extend(i.pipeline_stages.iter().cloned());
    if i.duck != 1.0 {
        stages.push(Stage::processing("Ducked for another app"));
    }
    if i.mixer_rate > 0 && i.mixer_rate != i.out_rate {
        stages.push(Stage::resample(i.out_rate, i.mixer_rate, "system"));
    }
    let output = OutputInfo {
        device: i.device.clone(),
        sample_rate: if i.mixer_rate > 0 {
            i.mixer_rate
        } else {
            i.out_rate
        },
        bits: None, // 32-bit float to the platform
    };
    let path = SignalPath::new(source, stages, output);
    let resampled = path.stages.iter().any(|s| s.resamples);
    let engine_bit_perfect = path.verdict == Verdict::BitPerfect;
    let verdict = match path.verdict {
        Verdict::BitPerfect => SignalVerdict::BitPerfect,
        Verdict::LosslessProcessed => SignalVerdict::LosslessProcessed,
        Verdict::Resampled => SignalVerdict::Resampled,
        Verdict::LossySource => SignalVerdict::LossySource,
    };
    SignalPathInfo {
        source_codec: path.source.codec.clone(),
        source_lossless: path.source.lossless,
        source_bits: path.source.bits,
        source_rate: path.source.sample_rate,
        source_channels: path.source.channels as u32,
        stages: path
            .stages
            .iter()
            .map(|s| SignalStage {
                name: s.name.clone(),
                bit_transparent: s.bit_transparent,
                resamples: s.resamples,
            })
            .collect(),
        dsp_active: i.dsp_active,
        output_device: path.output.device.clone(),
        output_rate: i.out_rate,
        output_format: "32-bit float".into(),
        resampled,
        engine_bit_perfect,
        bit_perfect: engine_bit_perfect && i.direct,
        verdict,
        summary: path.summary(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anywhere_core::decode::{ReplayGainTags, SourceFormat};

    fn flac(rate: u32, bits: u32) -> TrackAudio {
        TrackAudio {
            seq: 0,
            sample_rate: rate,
            source: Some(SourceFormat {
                codec: "FLAC",
                lossless: true,
                bits: Some(bits),
                channels: 2,
                replay_gain: ReplayGainTags::default(),
            }),
            server_gain: 1.0,
        }
    }

    fn inputs(audio: TrackAudio) -> Inputs {
        Inputs {
            audio,
            volume: (1.0, 1.0),
            pipeline_stages: vec![],
            dsp_active: false,
            duck: 1.0,
            out_rate: audio.sample_rate,
            device: "AAudio".into(),
            mixer_rate: 0,
            direct: false,
        }
    }

    #[test]
    fn untouched_lossless_is_engine_bit_perfect_but_not_claimed_end_to_end() {
        let p = describe(&inputs(flac(96_000, 24)));
        assert_eq!(p.verdict, SignalVerdict::BitPerfect);
        assert!(p.engine_bit_perfect);
        assert!(!p.bit_perfect, "the Android mixer is in the way");
        assert!(!p.resampled);
        assert_eq!(
            (p.source_rate, p.source_bits, p.output_rate),
            (96_000, Some(24), 96_000)
        );
        assert!(p.summary.starts_with("FLAC 96 kHz/24-bit"));
        let mut direct = inputs(flac(96_000, 24));
        direct.direct = true;
        assert!(describe(&direct).bit_perfect);
    }

    #[test]
    fn volume_server_gain_and_ducking_are_processing() {
        let mut i = inputs(flac(44_100, 16));
        i.volume = (0.5, 0.5);
        let p = describe(&i);
        assert_eq!(p.verdict, SignalVerdict::LosslessProcessed);
        assert!(p.stages.iter().any(|s| s.name.starts_with("Volume")));
        let mut a = flac(44_100, 16);
        a.server_gain = 0.5;
        let p = describe(&inputs(a));
        assert!(p.stages.iter().any(|s| s.name.contains("ReplayGain")));
        let mut i = inputs(flac(44_100, 16));
        i.duck = 0.2;
        assert!(!describe(&i).engine_bit_perfect);
        let mut i = inputs(flac(44_100, 16));
        i.volume = (0.0, 0.0);
        assert!(
            describe(&i)
                .stages
                .iter()
                .any(|s| s.name == "Volume (muted)")
        );
    }

    #[test]
    fn system_resampling_is_shown() {
        let mut i = inputs(flac(44_100, 16));
        i.mixer_rate = 48_000;
        let p = describe(&i);
        assert!(p.resampled);
        assert_eq!(p.verdict, SignalVerdict::Resampled);
        assert!(p.stages.iter().any(|s| s.name.contains("system")));
    }

    #[test]
    fn lossy_and_unknown_sources() {
        let mut a = flac(44_100, 16);
        a.source = Some(SourceFormat {
            codec: "MP3",
            lossless: false,
            bits: None,
            channels: 1,
            replay_gain: ReplayGainTags::default(),
        });
        let p = describe(&inputs(a));
        assert_eq!(p.verdict, SignalVerdict::LossySource);
        assert!(p.stages.iter().any(|s| s.name == "Mono to stereo"));
        let mut a = flac(44_100, 16);
        a.source = None;
        assert_eq!(describe(&inputs(a)).verdict, SignalVerdict::LossySource);
    }
}
