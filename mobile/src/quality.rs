//! The quality badge and the signal-path diagram over UniFFI (docs/plans/signal-path-redesign.md).
//!
//! The rules live in the core ([`anywhere_core::quality::describe`]), shared with the web app
//! and held to the same vectors. This module only mirrors its inputs and its diagram as UniFFI
//! records, and builds the local player's inputs from its signal-path snapshot:
//! - [`describe_quality`]: any diagram from inputs the app puts together (a remote player's,
//!   from the file alone; the offline player's, from the download's format);
//! - [`local_quality_diagram`] and [`crate::LocalPlayer::quality_diagram`]: this phone's,
//!   from the snapshot (what the engine decodes, the output route, and the gateway's stream
//!   message) plus what only the app knows (the file's format from LMS's status, why the
//!   streaming level in force is in force, and whether the track can be sought).

use anywhere_core::quality as q;

use crate::player::{SignalPathInfo, StreamQuality};

/// The library file as LMS reports it in `status` (tags `type`, `samplerate`, `samplesize`,
/// `bitrate`, `channels`).
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct QualityFile {
    /// LMS's content type: "flc", "mp3", "ops", … (any case).
    pub kind: Option<String>,
    /// Hz.
    pub rate: Option<u32>,
    pub bits: Option<u32>,
    /// kb/s.
    pub kbps: Option<u32>,
    pub channels: Option<u32>,
}

/// LMS's stream as it reached the gateway (the gateway's stream message).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StreamServed {
    /// "flac", "pcm", "mp3", "ogg", "aac", "alac" or "unknown".
    pub codec: String,
    pub rate: Option<u32>,
    pub bits: Option<u32>,
    pub channels: Option<u32>,
    /// Server ReplayGain applied, dB (`None`: none).
    pub gain_db: Option<f64>,
}

/// What the gateway did with LMS's stream (the gateway's stream message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StreamSent {
    /// Passed through unchanged.
    Passthrough,
    /// Re-encoded to Opus at `kbps`, for `level` (High or Low).
    Opus { kbps: u32, level: StreamQuality },
}

/// What the engine decodes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QualityDecoded {
    /// The engine's name: "FLAC", "Opus", "MP3", "PCM", …
    pub codec: String,
    pub rate: Option<u32>,
    pub bits: Option<u32>,
    pub channels: Option<u32>,
    pub lossless: bool,
}

/// The output device and the route's verdicts.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct QualityDevice {
    pub name: Option<String>,
    pub rate: Option<u32>,
    /// Every sample reaches the device unchanged.
    pub bit_perfect: bool,
    /// The engine itself leaves every sample unchanged.
    pub engine_bit_perfect: bool,
    pub resampled: bool,
    /// DSP the listener turned on changes the samples (the DSP chain; not the resampler).
    pub dsp_active: bool,
}

/// The network a per-network setting is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum QualityNetwork {
    Wifi,
    Mobile,
}

/// Why the streaming level in force is the one in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum QualityOrigin {
    /// The listener's setting: for this network, or the only one.
    Setting {
        network: Option<QualityNetwork>,
        level: StreamQuality,
    },
    /// Chosen in the sheet for this session.
    Override { level: StreamQuality },
    /// Chosen by the app, with no setting in force.
    Automatic { level: StreamQuality },
}

/// Who plays the track.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum QualityPlayer {
    /// This phone, streamed through the gateway.
    Local,
    /// Another player (Squeezebox, UPnP): only the file is known.
    Remote { name: String },
    /// A download played on this phone with no server; `format` is the download's label
    /// ("FLAC 24/96", "Opus 160").
    Offline { format: String },
}

/// Everything known about the playing track's path. Every part may be missing.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QualityInputs {
    pub player: QualityPlayer,
    pub file: Option<QualityFile>,
    pub served: Option<StreamServed>,
    pub sent: Option<StreamSent>,
    pub decoded: Option<QualityDecoded>,
    pub device: Option<QualityDevice>,
    pub origin: Option<QualityOrigin>,
    /// The stream can be sought, so a change of level takes effect now.
    pub seekable: bool,
}

/// The badge's verdict: the weakest link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiagramVerdict {
    HiRes,
    Lossless,
    Transcoded,
    Lossy,
}

/// The quality badge: "Transcoded · Opus 160".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiagramBadge {
    pub verdict: DiagramVerdict,
    /// What is heard: "Opus 160".
    pub stream: String,
    /// The badge's text: "Transcoded · Opus 160".
    pub text: String,
    /// Transcoded or Lossy: drawn in the amber tint (the word carries the meaning).
    pub lowered: bool,
}

/// What a block of the diagram stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiagramBlockKind {
    File,
    Lms,
    Gateway,
    Device,
    Download,
    Player,
}

/// One card of the diagram.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiagramBlock {
    pub kind: DiagramBlockKind,
    pub title: String,
    /// The format at this point; empty when unknown.
    pub format: String,
    /// One line of what happened here; may be empty.
    pub note: String,
    /// This block lowers the verdict (amber).
    pub lowers: bool,
}

/// The quality switch under the gateway block.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiagramSwitch {
    /// Original, High, Low, in that order.
    pub levels: Vec<StreamQuality>,
    pub in_force: StreamQuality,
    /// "High, on mobile data"; empty when unknown.
    pub origin_text: String,
    /// Chosen in the sheet: offer "Back to automatic".
    pub overridden: bool,
    /// A change takes effect now (the stream is sought); otherwise from the next track.
    pub seekable: bool,
}

/// The badge, the blocks in order, and the switch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QualityDiagram {
    pub badge: Option<DiagramBadge>,
    pub blocks: Vec<DiagramBlock>,
    pub switch: Option<DiagramSwitch>,
}

// --- To the core -------------------------------------------------------------------------

fn level(l: StreamQuality) -> q::Level {
    match l {
        StreamQuality::Original => q::Level::Original,
        StreamQuality::High => q::Level::High,
        StreamQuality::Low => q::Level::Low,
    }
}

fn level_back(l: q::Level) -> StreamQuality {
    match l {
        q::Level::Original => StreamQuality::Original,
        q::Level::High => StreamQuality::High,
        q::Level::Low => StreamQuality::Low,
    }
}

fn num(v: Option<u32>) -> Option<i64> {
    v.map(i64::from)
}

fn num_back(v: Option<i64>) -> Option<u32> {
    v.and_then(|n| u32::try_from(n).ok())
}

impl From<&QualityFile> for q::FileFormat {
    fn from(f: &QualityFile) -> Self {
        q::FileFormat {
            kind: f.kind.clone(),
            rate: num(f.rate),
            bits: num(f.bits),
            kbps: num(f.kbps),
            channels: num(f.channels),
        }
    }
}

impl From<&StreamServed> for q::Served {
    fn from(s: &StreamServed) -> Self {
        q::Served {
            codec: Some(s.codec.clone()),
            rate: num(s.rate),
            bits: num(s.bits),
            channels: num(s.channels),
            gain_db: s.gain_db,
        }
    }
}

impl From<&q::Served> for StreamServed {
    fn from(s: &q::Served) -> Self {
        StreamServed {
            codec: s.codec.clone().unwrap_or_else(|| "unknown".into()),
            rate: num_back(s.rate),
            bits: num_back(s.bits),
            channels: num_back(s.channels),
            gain_db: s.gain_db,
        }
    }
}

impl From<StreamSent> for q::Sent {
    fn from(s: StreamSent) -> Self {
        match s {
            StreamSent::Passthrough => q::Sent::Passthrough,
            StreamSent::Opus { kbps, level } => q::Sent::Opus {
                kbps: Some(i64::from(kbps)),
                level: match level {
                    StreamQuality::Low => Some(q::OpusLevel::Low),
                    StreamQuality::High => Some(q::OpusLevel::High),
                    StreamQuality::Original => None,
                },
            },
        }
    }
}

impl From<&q::Sent> for StreamSent {
    fn from(s: &q::Sent) -> Self {
        match s {
            q::Sent::Passthrough => StreamSent::Passthrough,
            q::Sent::Opus { kbps, level } => StreamSent::Opus {
                kbps: num_back(*kbps).unwrap_or(0),
                level: match level {
                    Some(q::OpusLevel::Low) => StreamQuality::Low,
                    _ => StreamQuality::High,
                },
            },
        }
    }
}

impl From<&QualityDecoded> for q::Decoded {
    fn from(d: &QualityDecoded) -> Self {
        q::Decoded {
            codec: Some(d.codec.clone()),
            rate: num(d.rate),
            bits: num(d.bits),
            channels: num(d.channels),
            lossless: d.lossless,
        }
    }
}

impl From<&q::Decoded> for QualityDecoded {
    fn from(d: &q::Decoded) -> Self {
        QualityDecoded {
            codec: d.codec.clone().unwrap_or_default(),
            rate: num_back(d.rate),
            bits: num_back(d.bits),
            channels: num_back(d.channels),
            lossless: d.lossless,
        }
    }
}

impl From<&QualityDevice> for q::Device {
    fn from(d: &QualityDevice) -> Self {
        q::Device {
            name: d.name.clone(),
            rate: num(d.rate),
            bit_perfect: d.bit_perfect,
            engine_bit_perfect: d.engine_bit_perfect,
            resampled: d.resampled,
            dsp_active: d.dsp_active,
        }
    }
}

impl From<&q::Device> for QualityDevice {
    fn from(d: &q::Device) -> Self {
        QualityDevice {
            name: d.name.clone(),
            rate: num_back(d.rate),
            bit_perfect: d.bit_perfect,
            engine_bit_perfect: d.engine_bit_perfect,
            resampled: d.resampled,
            dsp_active: d.dsp_active,
        }
    }
}

impl From<QualityOrigin> for q::Origin {
    fn from(o: QualityOrigin) -> Self {
        match o {
            QualityOrigin::Setting { network, level: l } => q::Origin::Setting {
                network: network.map(|n| match n {
                    QualityNetwork::Wifi => q::Network::Wifi,
                    QualityNetwork::Mobile => q::Network::Mobile,
                }),
                level: level(l),
            },
            QualityOrigin::Override { level: l } => q::Origin::Override { level: level(l) },
            QualityOrigin::Automatic { level: l } => q::Origin::Automatic { level: level(l) },
        }
    }
}

impl From<&QualityInputs> for q::QualityInputs {
    fn from(i: &QualityInputs) -> Self {
        q::QualityInputs {
            player: match &i.player {
                QualityPlayer::Local => q::Player::Local,
                QualityPlayer::Remote { name } => q::Player::Remote { name: name.clone() },
                QualityPlayer::Offline { format } => q::Player::Offline {
                    format: format.clone(),
                },
            },
            file: i.file.as_ref().map(Into::into),
            served: i.served.as_ref().map(Into::into),
            sent: i.sent.map(Into::into),
            decoded: i.decoded.as_ref().map(Into::into),
            device: i.device.as_ref().map(Into::into),
            origin: i.origin.map(Into::into),
            seekable: i.seekable,
        }
    }
}

// --- From the core -----------------------------------------------------------------------

impl From<&q::Diagram> for QualityDiagram {
    fn from(d: &q::Diagram) -> Self {
        QualityDiagram {
            badge: d.badge.as_ref().map(|b| DiagramBadge {
                verdict: match b.verdict {
                    q::Verdict::HiRes => DiagramVerdict::HiRes,
                    q::Verdict::Lossless => DiagramVerdict::Lossless,
                    q::Verdict::Transcoded => DiagramVerdict::Transcoded,
                    q::Verdict::Lossy => DiagramVerdict::Lossy,
                },
                stream: b.stream.clone(),
                text: b.text.clone(),
                lowered: b.verdict.is_lowered(),
            }),
            blocks: d
                .blocks
                .iter()
                .map(|b| DiagramBlock {
                    kind: match b.kind {
                        q::BlockKind::File => DiagramBlockKind::File,
                        q::BlockKind::Lms => DiagramBlockKind::Lms,
                        q::BlockKind::Gateway => DiagramBlockKind::Gateway,
                        q::BlockKind::Device => DiagramBlockKind::Device,
                        q::BlockKind::Download => DiagramBlockKind::Download,
                        q::BlockKind::Player => DiagramBlockKind::Player,
                    },
                    title: b.title.clone(),
                    format: b.format.clone(),
                    note: b.note.clone(),
                    lowers: b.lowers,
                })
                .collect(),
            switch: d.switch.as_ref().map(|s| DiagramSwitch {
                levels: s.levels.iter().copied().map(level_back).collect(),
                in_force: level_back(s.in_force),
                origin_text: s.origin_text.clone(),
                overridden: s.overridden,
                seekable: s.seekable,
            }),
        }
    }
}

// --- The functions -----------------------------------------------------------------------

/// The badge, blocks and switch for `inputs` (the core's rules, shared with the web app).
#[uniffi::export]
pub fn describe_quality(inputs: QualityInputs) -> QualityDiagram {
    (&q::describe(&(&inputs).into())).into()
}

/// The local player's inputs from its signal-path snapshot (`None`: nothing playing yet)
/// and what the app knows: the file's format, why the level in force is in force, and
/// whether the track can be sought.
pub fn local_inputs(
    path: Option<&SignalPathInfo>,
    file: Option<QualityFile>,
    origin: Option<QualityOrigin>,
    seekable: bool,
) -> QualityInputs {
    QualityInputs {
        player: QualityPlayer::Local,
        file,
        served: path.and_then(|p| p.served.clone()),
        sent: path.and_then(|p| p.sent),
        decoded: path.and_then(|p| p.decoded.clone()),
        device: path.and_then(|p| p.device.clone()),
        origin,
        seekable,
    }
}

/// This phone's diagram, from a signal-path snapshot (see [`local_inputs`]).
#[uniffi::export]
pub fn local_quality_diagram(
    path: Option<SignalPathInfo>,
    file: Option<QualityFile>,
    origin: Option<QualityOrigin>,
    seekable: bool,
) -> QualityDiagram {
    describe_quality(local_inputs(path.as_ref(), file, origin, seekable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{SignalPathInfo, SignalVerdict};
    use serde_json::Value;

    fn vectors() -> Vec<Value> {
        let text = include_str!("../../core/tests/fixtures/quality-vectors.json");
        let v: Value = serde_json::from_str(text).unwrap();
        v.as_array().unwrap().clone()
    }

    fn expected(v: &Value) -> q::Diagram {
        serde_json::from_value(v["expect"].clone()).unwrap()
    }

    /// An empty snapshot, filled in from a vector's input as the engine would.
    fn snapshot(i: &q::QualityInputs) -> SignalPathInfo {
        SignalPathInfo {
            source_codec: String::new(),
            source_lossless: false,
            source_bits: None,
            source_rate: 0,
            source_channels: 2,
            stages: vec![],
            dsp_active: false,
            output_device: String::new(),
            output_rate: 0,
            output_format: String::new(),
            output_bits: None,
            direct: false,
            fixed_volume: false,
            resampled: false,
            engine_bit_perfect: false,
            bit_perfect: false,
            not_bit_perfect: vec![],
            verdict: SignalVerdict::BitPerfect,
            summary: String::new(),
            from_cache: false,
            from_downloads: false,
            dsp_chain: false,
            decoded: i.decoded.as_ref().map(Into::into),
            device: i.device.as_ref().map(Into::into),
            served: i.served.as_ref().map(Into::into),
            sent: i.sent.as_ref().map(Into::into),
        }
    }

    fn file_of(f: &q::FileFormat) -> QualityFile {
        QualityFile {
            kind: f.kind.clone(),
            rate: num_back(f.rate),
            bits: num_back(f.bits),
            kbps: num_back(f.kbps),
            channels: num_back(f.channels),
        }
    }

    fn origin_of(o: &q::Origin) -> QualityOrigin {
        match o {
            q::Origin::Setting { network, level: l } => QualityOrigin::Setting {
                network: network.map(|n| match n {
                    q::Network::Wifi => QualityNetwork::Wifi,
                    q::Network::Mobile => QualityNetwork::Mobile,
                }),
                level: level_back(*l),
            },
            q::Origin::Override { level: l } => QualityOrigin::Override {
                level: level_back(*l),
            },
            q::Origin::Automatic { level: l } => QualityOrigin::Automatic {
                level: level_back(*l),
            },
        }
    }

    /// The local player's builder, fed each local vector's inputs through a snapshot,
    /// reproduces the vector's badge, blocks and switch exactly.
    #[test]
    fn the_local_builder_reproduces_the_vectors() {
        let mut local = 0;
        for v in vectors() {
            let name = v["name"].as_str().unwrap();
            let input: q::QualityInputs = serde_json::from_value(v["input"].clone()).unwrap();
            if input.player != q::Player::Local {
                continue;
            }
            local += 1;
            let path = snapshot(&input);
            let got = local_quality_diagram(
                Some(path),
                input.file.as_ref().map(file_of),
                input.origin.as_ref().map(origin_of),
                input.seekable,
            );
            let want: QualityDiagram = (&expected(&v)).into();
            assert_eq!(got, want, "{name}");
        }
        assert!(local >= 12, "the local vectors ({local})");
    }

    /// `describe_quality` reproduces every vector (remote and offline too) through the
    /// UniFFI records, both ways.
    #[test]
    fn describe_quality_reproduces_every_vector() {
        for v in vectors() {
            let name = v["name"].as_str().unwrap();
            let input: q::QualityInputs = serde_json::from_value(v["input"].clone()).unwrap();
            let inputs = QualityInputs {
                player: match &input.player {
                    q::Player::Local => QualityPlayer::Local,
                    q::Player::Remote { name } => QualityPlayer::Remote { name: name.clone() },
                    q::Player::Offline { format } => QualityPlayer::Offline {
                        format: format.clone(),
                    },
                },
                file: input.file.as_ref().map(file_of),
                served: input.served.as_ref().map(Into::into),
                sent: input.sent.as_ref().map(Into::into),
                decoded: input.decoded.as_ref().map(Into::into),
                device: input.device.as_ref().map(Into::into),
                origin: input.origin.as_ref().map(origin_of),
                seekable: input.seekable,
            };
            let got = describe_quality(inputs);
            let want: QualityDiagram = (&expected(&v)).into();
            assert_eq!(got, want, "{name}");
        }
    }

    /// The reported case: a FLAC 24/96 file, the gateway's Opus 160 for High on mobile
    /// data, decoded as Opus: "Transcoded · Opus 160", lowered, the gateway block amber.
    #[test]
    fn the_reported_case() {
        let mut path = snapshot(&q::QualityInputs::default());
        path.decoded = Some(QualityDecoded {
            codec: "Opus".into(),
            rate: Some(48_000),
            bits: None,
            channels: Some(2),
            lossless: false,
        });
        path.served = Some(StreamServed {
            codec: "flac".into(),
            rate: Some(96_000),
            bits: Some(24),
            channels: Some(2),
            gain_db: None,
        });
        path.sent = Some(StreamSent::Opus {
            kbps: 160,
            level: StreamQuality::High,
        });
        let d = local_quality_diagram(
            Some(path),
            Some(QualityFile {
                kind: Some("flc".into()),
                rate: Some(96_000),
                bits: Some(24),
                kbps: None,
                channels: None,
            }),
            Some(QualityOrigin::Setting {
                network: Some(QualityNetwork::Mobile),
                level: StreamQuality::High,
            }),
            true,
        );
        let badge = d.badge.unwrap();
        assert_eq!(badge.text, "Transcoded \u{b7} Opus 160");
        assert!(badge.lowered);
        let gateway = d
            .blocks
            .iter()
            .find(|b| b.kind == DiagramBlockKind::Gateway)
            .unwrap();
        assert!(gateway.lowers);
        let sw = d.switch.unwrap();
        assert_eq!(sw.in_force, StreamQuality::High);
        assert_eq!(
            sw.levels,
            vec![
                StreamQuality::Original,
                StreamQuality::High,
                StreamQuality::Low
            ]
        );
    }

    /// Nothing playing yet: the file alone, and a switch (a local player always has one).
    #[test]
    fn no_snapshot_yet() {
        let d = local_quality_diagram(None, None, None, false);
        assert!(d.badge.is_none());
        assert!(d.switch.is_some());
        assert_eq!(d.blocks.first().unwrap().kind, DiagramBlockKind::File);
    }

    /// Hostile strings and numbers from the app or the gateway never panic and come out
    /// short and clean.
    #[test]
    fn odd_inputs_never_panic() {
        let long = "x\u{202e}\u{0}".repeat(500);
        for player in [
            QualityPlayer::Local,
            QualityPlayer::Remote { name: long.clone() },
            QualityPlayer::Offline {
                format: long.clone(),
            },
        ] {
            let d = describe_quality(QualityInputs {
                player,
                file: Some(QualityFile {
                    kind: Some(long.clone()),
                    rate: Some(u32::MAX),
                    bits: Some(u32::MAX),
                    kbps: Some(0),
                    channels: Some(u32::MAX),
                }),
                served: Some(StreamServed {
                    codec: long.clone(),
                    rate: Some(u32::MAX),
                    bits: None,
                    channels: None,
                    gain_db: Some(f64::NAN),
                }),
                sent: Some(StreamSent::Opus {
                    kbps: u32::MAX,
                    level: StreamQuality::Original,
                }),
                decoded: Some(QualityDecoded {
                    codec: long.clone(),
                    rate: Some(0),
                    bits: Some(u32::MAX),
                    channels: Some(0),
                    lossless: true,
                }),
                device: Some(QualityDevice {
                    name: Some(long.clone()),
                    rate: Some(u32::MAX),
                    ..QualityDevice::default()
                }),
                origin: None,
                seekable: false,
            });
            for b in &d.blocks {
                for s in [&b.title, &b.format, &b.note] {
                    assert!(s.chars().count() < 200, "{s}");
                    assert!(!s.chars().any(|c| c.is_control() || c == '\u{202e}'), "{s}");
                }
            }
        }
    }
}
