//! A description of the path from file to output, for the app's signal-path panel.
//!
//! Example: `FLAC 96 kHz/24-bit → Decode (lossless) → Levelling −3.2 dB → Output 96 kHz/24-bit`
//! with the verdict "Lossless, processed".

use serde::{Deserialize, Serialize};

/// What is being played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Codec name for display, e.g. "FLAC".
    pub codec: String,
    pub lossless: bool,
    pub sample_rate: u32,
    /// Integer depth, or `None` for float or lossy sources.
    pub bits: Option<u32>,
    pub channels: usize,
}

/// Where it ends up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    /// Device or route for display, e.g. "USB DAC".
    pub device: String,
    pub sample_rate: u32,
    /// Integer depth, or `None` for float output.
    pub bits: Option<u32>,
}

/// One stage of the path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    /// Display text, e.g. "Levelling −3.2 dB".
    pub name: String,
    /// The stage leaves samples bit-identical.
    pub bit_transparent: bool,
    /// The stage changes the sample rate.
    #[serde(default)]
    pub resamples: bool,
}

impl Stage {
    pub fn transparent(name: impl Into<String>) -> Self {
        Stage {
            name: name.into(),
            bit_transparent: true,
            resamples: false,
        }
    }

    pub fn processing(name: impl Into<String>) -> Self {
        Stage {
            name: name.into(),
            bit_transparent: false,
            resamples: false,
        }
    }

    /// A resampling stage, e.g. "Resample 44.1→48 kHz (sinc)".
    pub fn resample(from: u32, to: u32, method: &str) -> Self {
        Stage {
            name: format!("Resample {}→{} kHz ({method})", khz(from), khz(to)),
            bit_transparent: false,
            resamples: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// Every sample reaches the output unchanged.
    BitPerfect,
    /// Lossless source, but some stage changes samples (at the same rate).
    LosslessProcessed,
    /// The sample rate is converted somewhere on the way.
    Resampled,
    /// The source itself is lossy.
    LossySource,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::BitPerfect => "Bit-perfect",
            Verdict::LosslessProcessed => "Lossless, processed",
            Verdict::Resampled => "Resampled",
            Verdict::LossySource => "Lossy source",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalPath {
    pub source: SourceInfo,
    pub stages: Vec<Stage>,
    pub output: OutputInfo,
    pub verdict: Verdict,
}

/// "44.1", "48", "176.4".
pub fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}", rate / 1000)
    } else {
        let s = format!("{:.3}", f64::from(rate) / 1000.0);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// "−3.2 dB" (typographic minus), "+1.5 dB".
pub fn fmt_db(db: f64) -> String {
    if db < 0.0 {
        format!("\u{2212}{:.1} dB", -db)
    } else {
        format!("+{db:.1} dB")
    }
}

fn fmt_format(rate: u32, bits: Option<u32>) -> String {
    match bits {
        Some(b) => format!("{} kHz/{b}-bit", khz(rate)),
        None => format!("{} kHz/float", khz(rate)),
    }
}

impl SignalPath {
    /// Assemble a path and work out its verdict. `stages` are the stages between decoding
    /// and output; decode and output-conversion stages are added here.
    pub fn new(source: SourceInfo, mut stages: Vec<Stage>, output: OutputInfo) -> Self {
        stages.insert(
            0,
            if source.lossless {
                Stage::transparent("Decode (lossless)")
            } else {
                Stage::processing("Decode (lossy)")
            },
        );
        match (source.bits, output.bits) {
            (Some(s), Some(o)) if o < s => stages.push(Stage::processing(format!(
                "Requantise {s}→{o}-bit (dither)"
            ))),
            (None, Some(o)) => stages.push(Stage::processing(format!("Quantise to {o}-bit"))),
            _ => {}
        }
        let resampled =
            stages.iter().any(|s| s.resamples) || source.sample_rate != output.sample_rate;
        if source.sample_rate != output.sample_rate && !stages.iter().any(|s| s.resamples) {
            // The platform (not us) is converting the rate: say so.
            stages.push(Stage {
                name: format!(
                    "Resample {}→{} kHz (system)",
                    khz(source.sample_rate),
                    khz(output.sample_rate)
                ),
                bit_transparent: false,
                resamples: true,
            });
        }
        let verdict = if !source.lossless {
            Verdict::LossySource
        } else if resampled {
            Verdict::Resampled
        } else if stages.iter().all(|s| s.bit_transparent) {
            Verdict::BitPerfect
        } else {
            Verdict::LosslessProcessed
        };
        SignalPath {
            source,
            stages,
            output,
            verdict,
        }
    }

    /// One-line summary, e.g. "FLAC 96 kHz/24-bit → Decode (lossless) → USB DAC 96 kHz/24-bit".
    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "{} {}",
            self.source.codec,
            fmt_format(self.source.sample_rate, self.source.bits)
        )];
        parts.extend(self.stages.iter().map(|s| s.name.clone()));
        parts.push(format!(
            "{} {}",
            self.output.device,
            fmt_format(self.output.sample_rate, self.output.bits)
        ));
        parts.join(" → ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac(rate: u32, bits: u32) -> SourceInfo {
        SourceInfo {
            codec: "FLAC".into(),
            lossless: true,
            sample_rate: rate,
            bits: Some(bits),
            channels: 2,
        }
    }

    fn out(rate: u32, bits: Option<u32>) -> OutputInfo {
        OutputInfo {
            device: "USB DAC".into(),
            sample_rate: rate,
            bits,
        }
    }

    #[test]
    fn verdicts() {
        let p = SignalPath::new(flac(96_000, 24), vec![], out(96_000, Some(24)));
        assert_eq!(p.verdict, Verdict::BitPerfect);
        assert_eq!(
            p.summary(),
            "FLAC 96 kHz/24-bit → Decode (lossless) → USB DAC 96 kHz/24-bit"
        );
        // Float output from integer is still exact.
        let p = SignalPath::new(flac(44_100, 16), vec![], out(44_100, None));
        assert_eq!(p.verdict, Verdict::BitPerfect);

        let p = SignalPath::new(
            flac(96_000, 24),
            vec![Stage::processing(format!("Levelling {}", fmt_db(-3.2)))],
            out(96_000, Some(24)),
        );
        assert_eq!(p.verdict, Verdict::LosslessProcessed);
        assert!(p.summary().contains("Levelling −3.2 dB"));

        let p = SignalPath::new(flac(96_000, 24), vec![], out(96_000, Some(16)));
        assert_eq!(p.verdict, Verdict::LosslessProcessed);

        let p = SignalPath::new(
            flac(44_100, 16),
            vec![Stage::resample(44_100, 48_000, "sinc")],
            out(48_000, Some(16)),
        );
        assert_eq!(p.verdict, Verdict::Resampled);
        assert!(p.summary().contains("Resample 44.1→48 kHz (sinc)"));

        let p = SignalPath::new(flac(44_100, 16), vec![], out(48_000, Some(16)));
        assert_eq!(p.verdict, Verdict::Resampled);
        assert!(p.summary().contains("(system)"));

        let mut mp3 = flac(44_100, 16);
        mp3.lossless = false;
        mp3.bits = None;
        let p = SignalPath::new(mp3, vec![], out(44_100, Some(16)));
        assert_eq!(p.verdict, Verdict::LossySource);
    }

    #[test]
    fn formatting() {
        assert_eq!(khz(44_100), "44.1");
        assert_eq!(khz(176_400), "176.4");
        assert_eq!(khz(192_000), "192");
        assert_eq!(khz(11_025), "11.025");
        assert_eq!(fmt_db(1.46), "+1.5 dB");
    }
}
