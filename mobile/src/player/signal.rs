//! The signal-path snapshot for the app's panel: what is playing, every stage that touches
//! it, and an honest verdict.
//!
//! Two verdicts are kept apart:
//! - `engine_bit_perfect`: our own path (decode → volume → DSP → output stream) leaves
//!   every sample untouched, at the source's rate;
//! - `bit_perfect`: that, and the platform delivers the stream to the device untouched.
//!   On Android's normal (shared, mixed) output that can't be promised. It is only true
//!   when bit-perfect mixer attributes were applied for a USB DAC and the stream opened in
//!   their format at the source's rate (see [`super::negotiate`]).
//!
//! When it isn't bit-perfect end to end, `not_bit_perfect` lists every reason.

use anywhere_core::dsp::signal_path::{OutputInfo, SignalPath, SourceInfo, Stage, Verdict, fmt_db};
use anywhere_core::player::TrackAudio;

use super::negotiate::{NotBitPerfect, RouteState};
use super::{SignalPathInfo, SignalStage, SignalVerdict};

pub(crate) struct Inputs {
    pub audio: TrackAudio,
    /// The volume applied to samples (left, right); (1, 1) is unity. At fixed volume
    /// (bit-perfect output) it is always unity.
    pub volume: (f32, f32),
    pub pipeline_stages: Vec<Stage>,
    /// The pipeline changes samples (DSP, resampling or requantising).
    pub dsp_active: bool,
    /// Our DSP chain itself changes samples (preamp, levelling, EQ, crossfeed, limiter).
    pub dsp_chain: bool,
    /// Output gain for ducking (1.0 = none).
    pub duck: f32,
    /// The output stream's rate.
    pub out_rate: u32,
    pub device: String,
    /// The platform mixer's rate, if known (0 = unknown). Ignored on a direct route.
    pub mixer_rate: u32,
    /// How the output was set up (negotiated mixer attributes, format, reasons).
    pub route: RouteState,
    /// Crossfades between tracks, described (`None`: off, gapless).
    pub transitions: Option<String>,
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
    let other_processing = !stages.iter().all(|s| s.bit_transparent) || i.duck != 1.0;
    stages.extend(i.pipeline_stages.iter().cloned());
    if let Some(t) = &i.transitions {
        stages.push(Stage::processing(t.clone()));
    }
    if i.duck != 1.0 {
        stages.push(Stage::processing("Ducked for another app"));
    }
    let direct = i.route.direct;
    let mixer_rate = if direct { 0 } else { i.mixer_rate };
    if direct {
        stages.push(Stage::transparent("Android bit-perfect mixer"));
    } else if mixer_rate > 0 && mixer_rate != i.out_rate {
        stages.push(Stage::resample(i.out_rate, mixer_rate, "system"));
    }
    let device = if direct && !i.route.device.is_empty() {
        i.route.device.clone()
    } else {
        i.device.clone()
    };
    let output_bits = if direct {
        i.route.encoding.bits()
    } else {
        None
    };
    let output = OutputInfo {
        device,
        sample_rate: if mixer_rate > 0 {
            mixer_rate
        } else {
            i.out_rate
        },
        // Requantising is already a pipeline stage; the depth is filled in below.
        bits: None,
    };
    let mut path = SignalPath::new(source, stages, output);
    path.output.bits = output_bits;
    let resampled = path.stages.iter().any(|s| s.resamples);
    let engine_bit_perfect = path.verdict == Verdict::BitPerfect;
    let verdict = match path.verdict {
        Verdict::BitPerfect => SignalVerdict::BitPerfect,
        Verdict::LosslessProcessed => SignalVerdict::LosslessProcessed,
        Verdict::Resampled => SignalVerdict::Resampled,
        Verdict::LossySource => SignalVerdict::LossySource,
    };
    // Every reason it isn't bit-perfect end to end: the route's own, then the engine's.
    let mut reasons = i.route.reasons.clone();
    if !direct && reasons.is_empty() {
        // Not direct, and the route gave no reason: still never claim it.
        reasons.push(NotBitPerfect::NoUsbDac);
    }
    for (applies, r) in [
        (!path.source.lossless, NotBitPerfect::LossySource),
        (i.dsp_chain, NotBitPerfect::DspActive),
        (resampled, NotBitPerfect::Resampled),
        (other_processing, NotBitPerfect::OtherProcessing),
        (i.transitions.is_some(), NotBitPerfect::Crossfade),
    ] {
        if applies && !reasons.contains(&r) {
            reasons.push(r);
        }
    }
    let bit_perfect = engine_bit_perfect && direct && reasons.is_empty();
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
        dsp_active: i.dsp_active || i.transitions.is_some(),
        output_device: path.output.device.clone(),
        output_rate: i.out_rate,
        output_format: if direct {
            i.route.encoding.describe()
        } else {
            "32-bit float".into()
        },
        output_bits,
        direct,
        fixed_volume: direct,
        resampled,
        engine_bit_perfect,
        bit_perfect,
        not_bit_perfect: reasons,
        verdict,
        summary: path.summary(),
        from_cache: false,
        from_downloads: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::negotiate::PcmEncoding;
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
            dsp_chain: false,
            route: RouteState::default(),
            transitions: None,
        }
    }

    #[test]
    fn crossfades_are_shown_and_never_called_bit_perfect() {
        let mut i = inputs(flac(44_100, 16));
        i.route = usb(PcmEncoding::I16);
        assert!(describe(&i).bit_perfect, "a direct route, untouched");
        i.transitions = Some("Crossfade 6 s (not within albums)".into());
        let p = describe(&i);
        assert!(!p.bit_perfect && !p.engine_bit_perfect);
        assert!(p.dsp_active);
        assert!(p.not_bit_perfect.contains(&NotBitPerfect::Crossfade));
        assert!(
            p.stages
                .iter()
                .any(|s| s.name == "Crossfade 6 s (not within albums)" && !s.bit_transparent)
        );
        assert!(p.summary.contains("Crossfade 6 s"), "{}", p.summary);
    }

    fn usb(encoding: PcmEncoding) -> RouteState {
        RouteState {
            direct: true,
            encoding,
            device: "Example DAC".into(),
            reasons: vec![],
        }
    }

    #[test]
    fn untouched_lossless_is_engine_bit_perfect_but_not_claimed_end_to_end() {
        let p = describe(&inputs(flac(96_000, 24)));
        assert_eq!(p.verdict, SignalVerdict::BitPerfect);
        assert!(p.engine_bit_perfect);
        assert!(!p.bit_perfect, "the Android mixer is in the way");
        assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::NoUsbDac]);
        assert!(!p.resampled && !p.direct && !p.fixed_volume);
        assert_eq!(p.output_format, "32-bit float");
        assert_eq!(
            (p.source_rate, p.source_bits, p.output_rate),
            (96_000, Some(24), 96_000)
        );
        assert!(p.summary.starts_with("FLAC 96 kHz/24-bit"));
    }

    /// End to end only when the route is direct, its format holds the source and nothing
    /// in the engine touched the samples.
    #[test]
    fn a_direct_usb_route_is_bit_perfect_end_to_end() {
        let mut i = inputs(flac(96_000, 24));
        i.route = usb(PcmEncoding::I24Packed);
        i.mixer_rate = 48_000; // the mixer is bypassed: its rate doesn't matter
        let p = describe(&i);
        assert!(p.bit_perfect && p.engine_bit_perfect && p.direct && p.fixed_volume);
        assert!(p.not_bit_perfect.is_empty());
        assert!(!p.resampled);
        assert_eq!(p.output_device, "Example DAC");
        assert_eq!((p.output_bits, p.output_rate), (Some(24), 96_000));
        assert_eq!(p.output_format, "24-bit integer");
        assert!(
            p.summary
                .ends_with("Android bit-perfect mixer → Example DAC 96 kHz/24-bit"),
            "{}",
            p.summary
        );
        // A 16-bit source padded into 32-bit: still exact.
        let mut i = inputs(flac(44_100, 16));
        i.route = usb(PcmEncoding::I32);
        assert!(describe(&i).bit_perfect);
    }

    /// Each way of falling short shows up as a reason, and kills the claim.
    #[test]
    fn every_shortfall_has_a_reason() {
        let base = || {
            let mut i = inputs(flac(96_000, 24));
            i.route = usb(PcmEncoding::I24Packed);
            i
        };
        // The route's own reasons (e.g. a 16-bit-only DAC).
        let mut i = base();
        i.route.reasons = vec![NotBitPerfect::DacTooShallow];
        i.pipeline_stages = vec![Stage::processing("Dither to 16-bit (TPDF)")];
        i.dsp_active = true;
        let p = describe(&i);
        assert!(!p.bit_perfect && p.direct);
        assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::DacTooShallow]);
        // DSP.
        let mut i = base();
        i.pipeline_stages = vec![Stage::processing("Preamp −3.0 dB")];
        i.dsp_active = true;
        i.dsp_chain = true;
        let p = describe(&i);
        assert!(!p.bit_perfect);
        assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::DspActive]);
        assert_eq!(p.verdict, SignalVerdict::LosslessProcessed);
        // Resampling (float fallback).
        let mut i = inputs(flac(44_100, 16));
        i.route = RouteState {
            reasons: vec![NotBitPerfect::RateNotOffered],
            ..RouteState::default()
        };
        i.pipeline_stages = vec![Stage::resample(44_100, 48_000, "sinc")];
        i.out_rate = 48_000;
        let p = describe(&i);
        assert_eq!(
            p.not_bit_perfect,
            vec![NotBitPerfect::RateNotOffered, NotBitPerfect::Resampled]
        );
        // A lossy source through a direct route (dithered to 24-bit).
        let mut i = base();
        i.audio.source = Some(SourceFormat {
            codec: "MP3",
            lossless: false,
            bits: None,
            channels: 2,
            replay_gain: ReplayGainTags::default(),
        });
        let p = describe(&i);
        assert_eq!(p.not_bit_perfect, vec![NotBitPerfect::LossySource]);
        assert_eq!(p.verdict, SignalVerdict::LossySource);
        // LMS's ReplayGain, a downmix, ducking.
        let mut i = base();
        i.audio.server_gain = 0.5;
        assert_eq!(
            describe(&i).not_bit_perfect,
            vec![NotBitPerfect::OtherProcessing]
        );
        let mut i = base();
        i.duck = 0.2;
        assert_eq!(
            describe(&i).not_bit_perfect,
            vec![NotBitPerfect::OtherProcessing]
        );
        // Blocked routes carry their reason through.
        for r in [
            NotBitPerfect::TurnedOff,
            NotBitPerfect::AndroidTooOld,
            NotBitPerfect::NotOffered,
            NotBitPerfect::NotApplied,
            NotBitPerfect::FormatRefused,
        ] {
            let mut i = inputs(flac(48_000, 16));
            i.route = RouteState {
                reasons: vec![r],
                ..RouteState::default()
            };
            let p = describe(&i);
            assert!(p.engine_bit_perfect && !p.bit_perfect);
            assert_eq!(p.not_bit_perfect, vec![r]);
        }
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
