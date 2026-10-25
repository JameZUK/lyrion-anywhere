//! The quality badge and the signal-path block diagram, from the facts about a playing track
//! (docs/plans/signal-path-redesign.md, decisions 1, 3, 4, 5 and 7).
//!
//! One pure function, [`describe`], turns [`QualityInputs`] (the library file as LMS reports
//! it, what LMS served, what the gateway did, what the engine decodes, the output route and
//! why the streaming level is in force) into a [`Diagram`]: the badge ("Transcoded · Opus
//! 160"), the blocks of the sheet and the quality switch. Both clients draw from it, and both
//! are held to the same vectors (`core/tests/fixtures/quality-vectors.json`) byte for byte.
//!
//! The verdict is decided by codecs and re-encoding only: a lossy file is *Lossy*; a lossless
//! file heard through a lossy stream is *Transcoded*; a lossless stream arriving as stored is
//! *Hi-Res* (more than 16 bits or more than 48 kHz) or *Lossless*. Device resampling, DSP,
//! ReplayGain and volume never change it.
//!
//! Everything here is total: any input, however odd, gives a diagram whose strings are short
//! and free of control characters. Numbers out of range are treated as unknown.

use serde::{Deserialize, Serialize};

use crate::dsp::signal_path::{SignalPath, fmt_db};

/// The longest text taken from an input string (names, labels), in characters.
const MAX_NAME: usize = 40;

// ---------------------------------------------------------------------------------------
// The codec table and labels (shared with the mobile crate's and the web's format badges).
// ---------------------------------------------------------------------------------------

/// LMS's content type (`type` tag: "flc", "mp3", "ops", …) or a stream token ("flac",
/// "pcm", …) to the codec's display name and whether it is lossless. Case and surrounding
/// spaces are ignored. Unknown types are `None` (see [`codec_name`] for the fallback).
pub fn codec(kind: &str) -> Option<(&'static str, bool)> {
    let k = kind.trim();
    if k.len() > 12 {
        return None;
    }
    Some(match k.to_ascii_lowercase().as_str() {
        "flc" | "flac" => ("FLAC", true),
        "alc" | "alac" => ("ALAC", true),
        "wav" => ("WAV", true),
        "aif" | "aiff" => ("AIFF", true),
        "pcm" => ("PCM", true),
        "ape" => ("APE", true),
        "wvp" => ("WavPack", true),
        "wmal" => ("WMA Lossless", true),
        "dsf" | "dff" => ("DSD", true),
        "mp3" => ("MP3", false),
        "aac" | "mp4" | "m4a" => ("AAC", false),
        "ogg" => ("Ogg Vorbis", false),
        "ops" | "opus" => ("Opus", false),
        "wma" | "wmap" => ("WMA", false),
        "mpc" => ("Musepack", false),
        _ => return None,
    })
}

/// [`codec`], with LMS's fallback for types not in the table: a short alphanumeric type is
/// shown upper-cased and taken to be lossy ("xyz" → "XYZ"). Anything else is `None`.
pub fn codec_name(kind: &str) -> Option<(String, bool)> {
    if let Some((name, lossless)) = codec(kind) {
        return Some((name.to_string(), lossless));
    }
    let k = kind.trim();
    ((1..=6).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| (k.to_ascii_uppercase(), false))
}

/// A rate in kHz for labels: whole kilohertz as an integer, otherwise rounded (half up) to
/// one decimal place with a trailing ".0" dropped. 44100 → "44.1", 48000 → "48", 88200 →
/// "88.2", 96000 → "96", 192000 → "192", 22050 → "22.1", 11025 → "11".
pub fn khz(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        return (hz / 1000).to_string();
    }
    let tenths = (u64::from(hz) + 50) / 100;
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// A format's label: "FLAC 24/96", "FLAC 44.1 kHz" (rate only), "MP3 320" (lossy, with its
/// bit rate in kb/s), "DSD64" (a DSD rate as a multiple of 44.1 kHz), or the codec's name.
pub fn format_label(
    name: &str,
    lossless: bool,
    bits: Option<u32>,
    rate: Option<u32>,
    kbps: Option<u32>,
) -> String {
    if name == "DSD" {
        return match rate.map(|r| (f64::from(r) / 44_100.0).round() as u32) {
            Some(n) if n >= 32 => format!("DSD{n}"),
            _ => "DSD".into(),
        };
    }
    match (lossless, bits, rate, kbps) {
        (true, Some(b), Some(r), _) => format!("{name} {b}/{}", khz(r)),
        (true, None, Some(r), _) => format!("{name} {} kHz", khz(r)),
        (false, _, _, Some(k)) => format!("{name} {k}"),
        _ => name.to_string(),
    }
}

/// Hi-Res: lossless, and more than 16 bits or more than 48 kHz.
pub fn is_hi_res(lossless: bool, bits: Option<u32>, rate: Option<u32>) -> bool {
    lossless && (bits.is_some_and(|b| b > 16) || rate.is_some_and(|r| r > 48_000))
}

// ---------------------------------------------------------------------------------------
// Inputs.
// ---------------------------------------------------------------------------------------

/// Everything a client knows about the playing track's path. Every part may be missing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QualityInputs {
    /// Who plays it.
    pub player: Player,
    /// The library file, as LMS reports it in `status`.
    pub file: Option<FileFormat>,
    /// LMS's stream as it reached the gateway (the gateway's stream message).
    pub served: Option<Served>,
    /// What the gateway did with it (the gateway's stream message).
    pub sent: Option<Sent>,
    /// What the engine on this device decodes.
    pub decoded: Option<Decoded>,
    /// The output device and route.
    pub device: Option<Device>,
    /// Why the streaming level in force is the one in force.
    pub origin: Option<Origin>,
    /// The stream can be sought, so a change of level can take effect now.
    pub seekable: bool,
}

/// Who plays the track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Player {
    /// This device plays, streamed through the gateway.
    #[default]
    Local,
    /// A Squeezebox or UPnP player: only the file is known.
    Remote { name: String },
    /// A download played on this device with no server. `format` is the download's label as
    /// the client shows it ("FLAC 24/96", "Opus 160").
    Offline { format: String },
}

/// The library file, from LMS's `status` tags (`type`, `samplerate`, `samplesize`,
/// `bitrate`), as numbers. Values out of range are ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileFormat {
    /// LMS's content type, e.g. "flc", "mp3", "ops" (any case).
    pub kind: Option<String>,
    /// Sample rate, Hz.
    pub rate: Option<i64>,
    /// Sample size, bits.
    pub bits: Option<i64>,
    /// Bit rate, kb/s.
    pub kbps: Option<i64>,
    pub channels: Option<i64>,
}

/// LMS's stream as it reached the gateway.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Served {
    /// "flac", "pcm", "mp3", "aac", "ogg", "alac" or "opus".
    pub codec: Option<String>,
    pub rate: Option<i64>,
    pub bits: Option<i64>,
    pub channels: Option<i64>,
    /// Server ReplayGain applied to the stream, dB (absent: none).
    pub gain_db: Option<f64>,
}

/// What the gateway did with LMS's stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Sent {
    /// Passed through unchanged.
    Passthrough,
    /// Re-encoded to Opus at `kbps`, for the level asked.
    Opus {
        #[serde(default)]
        kbps: Option<i64>,
        #[serde(default)]
        level: Option<OpusLevel>,
    },
}

/// The level an Opus re-encode was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpusLevel {
    High,
    Low,
}

/// What the engine decodes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Decoded {
    /// The engine's display name: "FLAC", "Opus", "MP3", "PCM", …
    pub codec: Option<String>,
    pub rate: Option<i64>,
    pub bits: Option<i64>,
    pub channels: Option<i64>,
    pub lossless: bool,
}

#[cfg(feature = "player")]
impl Decoded {
    /// From the engine's facts about a track (`None` until its headers are probed).
    pub fn from_track(audio: &crate::player::TrackAudio) -> Option<Decoded> {
        let s = audio.source?;
        Some(Decoded {
            codec: Some(s.codec.to_string()),
            rate: Some(i64::from(audio.sample_rate)),
            bits: s.bits.map(i64::from),
            channels: Some(i64::from(s.channels)),
            lossless: s.lossless,
        })
    }
}

/// The output device and the route's verdicts (as the engine's signal path computes them).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    /// "Phone speaker", "USB DAC", …
    pub name: Option<String>,
    /// The output's rate, Hz.
    pub rate: Option<i64>,
    /// Every sample reaches the device unchanged.
    pub bit_perfect: bool,
    /// The engine itself leaves every sample unchanged (the platform may not).
    pub engine_bit_perfect: bool,
    /// The rate is converted on the way.
    pub resampled: bool,
    /// DSP the listener turned on (EQ, levelling, crossfeed, …) changes the samples.
    pub dsp_active: bool,
}

impl Device {
    /// From the engine's signal path. `bit_perfect` is the end-to-end verdict (the platform
    /// delivers the stream untouched) and `dsp_active` whether the listener's DSP changes
    /// samples; neither can be read from the path alone.
    pub fn from_path(path: &SignalPath, bit_perfect: bool, dsp_active: bool) -> Device {
        use crate::dsp::signal_path::Verdict as V;
        Device {
            name: Some(path.output.device.clone()),
            rate: Some(i64::from(path.output.sample_rate)),
            bit_perfect,
            engine_bit_perfect: path.verdict == V::BitPerfect,
            resampled: path.verdict == V::Resampled || path.stages.iter().any(|s| s.resamples),
            dsp_active,
        }
    }
}

/// The streaming levels: Original (passed through), High and Low ("Data saver") Opus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Original,
    High,
    Low,
}

impl Level {
    /// The level's name as the switch and the reasons show it.
    pub fn name(self) -> &'static str {
        match self {
            Level::Original => "Original",
            Level::High => "High",
            Level::Low => "Data saver",
        }
    }
}

/// The network type a per-network setting is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Wifi,
    Mobile,
}

/// Why the level in force is the one in force.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// The listener's setting: the per-network one (`network` given) or the only one this
    /// client has (no network type known, e.g. a browser without it).
    Setting {
        #[serde(default)]
        network: Option<Network>,
        level: Level,
    },
    /// Chosen in the sheet for this session.
    Override { level: Level },
    /// Chosen by the app, with no setting in force.
    Automatic { level: Level },
}

impl Origin {
    pub fn level(&self) -> Level {
        match self {
            Origin::Setting { level, .. }
            | Origin::Override { level }
            | Origin::Automatic { level } => *level,
        }
    }

    /// "High, on mobile data", "Data saver, chosen in Settings", "Original, overridden here",
    /// "High, chosen automatically".
    pub fn text(&self) -> String {
        let why = match self {
            Origin::Setting {
                network: Some(Network::Wifi),
                ..
            } => "on Wi-Fi",
            Origin::Setting {
                network: Some(Network::Mobile),
                ..
            } => "on mobile data",
            Origin::Setting { network: None, .. } => "chosen in Settings",
            Origin::Override { .. } => "overridden here",
            Origin::Automatic { .. } => "chosen automatically",
        };
        format!("{}, {why}", self.level().name())
    }
}

// ---------------------------------------------------------------------------------------
// Output.
// ---------------------------------------------------------------------------------------

/// The badge's verdict: the weakest link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    HiRes,
    Lossless,
    Transcoded,
    Lossy,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Verdict::HiRes => "Hi-Res",
            Verdict::Lossless => "Lossless",
            Verdict::Transcoded => "Transcoded",
            Verdict::Lossy => "Lossy",
        }
    }

    /// Transcoded and Lossy are drawn in the amber tint.
    pub fn is_lowered(self) -> bool {
        matches!(self, Verdict::Transcoded | Verdict::Lossy)
    }
}

/// The quality badge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Badge {
    pub verdict: Verdict,
    /// What is heard: "FLAC 24/96", "Opus 160", "MP3 320".
    pub stream: String,
    /// The full text: "Transcoded · Opus 160".
    pub text: String,
}

/// What a block of the diagram stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    File,
    Lms,
    Gateway,
    Device,
    Download,
    Player,
}

/// One card of the diagram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub title: String,
    /// The format at this point ("FLAC 24/96", "Opus 48 kHz"); empty when unknown.
    pub format: String,
    /// One line of what happened here; may be empty.
    pub note: String,
    /// This block lowers the verdict (drawn in the amber tint).
    pub lowers: bool,
}

/// The quality switch under the gateway block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Switch {
    /// Always Original, High, Low, in that order.
    pub levels: [Level; 3],
    pub in_force: Level,
    /// Why the level in force is in force ("High, on mobile data"); empty when unknown.
    pub origin_text: String,
    /// The level was chosen in the sheet: offer "Back to automatic".
    pub overridden: bool,
    /// A change takes effect now (the stream is sought); otherwise from the next stream.
    pub seekable: bool,
}

/// The badge, the blocks of the sheet in order, and the switch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagram {
    pub badge: Option<Badge>,
    pub blocks: Vec<Block>,
    pub switch: Option<Switch>,
}

// ---------------------------------------------------------------------------------------
// The rules.
// ---------------------------------------------------------------------------------------

/// A format at one point of the path, with its numbers checked.
#[derive(Debug, Clone, PartialEq)]
struct Fmt {
    name: String,
    lossless: bool,
    bits: Option<u32>,
    rate: Option<u32>,
    kbps: Option<u32>,
    channels: Option<u32>,
}

impl Fmt {
    fn label(&self) -> String {
        format_label(&self.name, self.lossless, self.bits, self.rate, self.kbps)
    }

    /// The label with the channel layout when it is not stereo: "FLAC 16/44.1, mono".
    fn label_with_channels(&self) -> String {
        with_channels(self.label(), self.channels)
    }

    fn hi_res(&self) -> bool {
        is_hi_res(self.lossless, self.bits, self.rate)
    }

    fn verdict(&self) -> Verdict {
        if !self.lossless {
            Verdict::Lossy
        } else if self.hi_res() {
            Verdict::HiRes
        } else {
            Verdict::Lossless
        }
    }

    /// The same codec under any of its names: WAV and AIFF files reach a player as PCM, and
    /// the engine calls Ogg Vorbis "Vorbis".
    fn same_codec(&self, other: &Fmt) -> bool {
        fn canon(n: &str) -> &str {
            match n {
                "WAV" | "AIFF" | "PCM (float)" => "PCM",
                "Vorbis" => "Ogg Vorbis",
                n => n,
            }
        }
        canon(&self.name) == canon(&other.name)
    }
}

fn with_channels(label: String, channels: Option<u32>) -> String {
    match channels {
        Some(1) => format!("{label}, mono"),
        Some(n) if n > 2 => format!("{label}, {n} channels"),
        _ => label,
    }
}

fn in_range(v: Option<i64>, lo: i64, hi: i64) -> Option<u32> {
    v.filter(|n| (lo..=hi).contains(n)).map(|n| n as u32)
}

fn rate_of(v: Option<i64>) -> Option<u32> {
    in_range(v, 1000, 50_000_000)
}

fn bits_of(v: Option<i64>) -> Option<u32> {
    in_range(v, 1, 64)
}

fn kbps_of(v: Option<i64>) -> Option<u32> {
    in_range(v, 8, 100_000)
}

fn channels_of(v: Option<i64>) -> Option<u32> {
    in_range(v, 1, 64)
}

/// Text from an input: control characters, line and paragraph separators and the invisible
/// formatting characters (zero-width and bidirectional controls, which could make a name
/// read as something else) dropped, trimmed, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    let invisible = |c: char| {
        c.is_control()
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
    };
    let t: String = s.chars().filter(|&c| !invisible(c)).collect();
    t.trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn file_fmt(f: &FileFormat) -> Option<Fmt> {
    let (name, lossless) = codec_name(f.kind.as_deref()?)?;
    Some(Fmt {
        name,
        lossless,
        bits: bits_of(f.bits),
        rate: rate_of(f.rate),
        kbps: kbps_of(f.kbps),
        channels: channels_of(f.channels),
    })
}

/// LMS's stream; a lossy stream in the file's codec borrows the file's bit rate (LMS sent
/// the file as it is), as a lossless one does its depth and rate when the stream message
/// did not carry them.
fn served_fmt(s: &Served, file: Option<&Fmt>) -> Option<Fmt> {
    let token = s.codec.as_deref()?;
    let (name, lossless) = match file {
        // `ogg` is the container: Vorbis or Opus, as the file says.
        Some(f) if token.trim().eq_ignore_ascii_case("ogg") && is_ogg(&f.name) => {
            (f.name.clone(), false)
        }
        _ if token.trim().eq_ignore_ascii_case("ogg") => ("Ogg".to_string(), false),
        _ => codec(token).map(|(n, l)| (n.to_string(), l))?,
    };
    let mut f = Fmt {
        name,
        lossless,
        bits: bits_of(s.bits),
        rate: rate_of(s.rate),
        kbps: None,
        channels: channels_of(s.channels),
    };
    if let Some(file) = file.filter(|file| file.same_codec(&f)) {
        if lossless {
            f.bits = f.bits.or(file.bits);
            f.rate = f.rate.or(file.rate);
        } else {
            f.kbps = file.kbps;
        }
    }
    Some(f)
}

fn is_ogg(name: &str) -> bool {
    matches!(name, "Ogg Vorbis" | "Opus")
}

fn decoded_fmt(d: &Decoded) -> Option<Fmt> {
    let name = clean(d.codec.as_deref()?, 20);
    if name.is_empty() {
        return None;
    }
    Some(Fmt {
        name,
        lossless: d.lossless,
        bits: if d.lossless { bits_of(d.bits) } else { None },
        rate: rate_of(d.rate),
        kbps: None,
        channels: channels_of(d.channels),
    })
}

fn opus_fmt(kbps: Option<i64>) -> Fmt {
    Fmt {
        name: "Opus".into(),
        lossless: false,
        bits: None,
        rate: None,
        kbps: kbps_of(kbps),
        channels: None,
    }
}

/// The decoder's format as the device block shows it: "FLAC 24/96", "Opus 48 kHz".
fn decoded_label(d: &Fmt) -> String {
    let label = match (d.lossless, d.rate) {
        (false, Some(r)) => format!("{} {} kHz", d.name, khz(r)),
        _ => format_label(&d.name, d.lossless, d.bits, d.rate, None),
    };
    with_channels(label, d.channels)
}

/// The stream heard on this device, when it plays through the gateway.
fn heard_local(
    file: Option<&Fmt>,
    served: Option<&Fmt>,
    sent: Option<&Sent>,
    decoded: Option<&Fmt>,
) -> Option<Fmt> {
    let sent_kbps = match sent {
        Some(Sent::Opus { kbps, .. }) => Some(*kbps),
        _ => None,
    };
    if let Some(d) = decoded {
        let mut h = d.clone();
        if !h.lossless {
            if h.name == "Opus"
                && let Some(k) = sent_kbps
            {
                h.kbps = kbps_of(k);
            } else if let Some(src) = served
                .filter(|s| !s.lossless && s.same_codec(&h))
                .or(file.filter(|f| !f.lossless && f.same_codec(&h)))
            {
                h.kbps = src.kbps;
            }
        }
        return Some(h);
    }
    if let Some(k) = sent_kbps {
        return Some(opus_fmt(k));
    }
    served.or(file).cloned()
}

fn badge(verdict: Verdict, stream: String) -> Badge {
    Badge {
        verdict,
        text: format!("{} \u{b7} {stream}", verdict.word()),
        stream,
    }
}

/// The verdict: a lossy file is Lossy; a lossless file heard through a lossy stream is
/// Transcoded; otherwise the heard stream's own verdict.
fn verdict_of(file: Option<&Fmt>, heard: &Fmt) -> Verdict {
    match (file.map(|f| f.lossless), heard.lossless) {
        (Some(false), _) => Verdict::Lossy,
        (Some(true), false) => Verdict::Transcoded,
        _ => heard.verdict(),
    }
}

fn block(kind: BlockKind, title: &str, format: String, note: String) -> Block {
    Block {
        kind,
        title: title.to_string(),
        format,
        note,
        lowers: false,
    }
}

fn file_block(file: Option<&Fmt>) -> Block {
    match file {
        Some(f) => block(
            BlockKind::File,
            "File",
            f.label_with_channels(),
            match f.verdict() {
                Verdict::HiRes => "Hi-Res lossless",
                Verdict::Lossless => "Lossless",
                _ => "Lossy",
            }
            .into(),
        ),
        None => block(
            BlockKind::File,
            "File",
            String::new(),
            "Format not reported".into(),
        ),
    }
}

/// The device block's note: the output device, then what the route does to the samples.
fn device_note(d: Option<&Device>) -> String {
    let Some(d) = d else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(name) = d.name.as_deref().map(|n| clean(n, MAX_NAME))
        && !name.is_empty()
    {
        parts.push(name);
    }
    if d.bit_perfect {
        parts.push("bit-perfect".into());
    } else {
        if d.resampled {
            parts.push(match rate_of(d.rate) {
                Some(r) => format!("resampled to {} kHz", khz(r)),
                None => "resampled".into(),
            });
        }
        if d.dsp_active {
            parts.push("processed by DSP you turned on".into());
        }
    }
    capitalise(parts.join(", "))
}

fn capitalise(s: String) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) if first.is_lowercase() => first.to_uppercase().chain(c).collect(),
        _ => s,
    }
}

fn device_block(decoded: Option<&Fmt>, device: Option<&Device>) -> Block {
    block(
        BlockKind::Device,
        "This device",
        decoded.map(decoded_label).unwrap_or_default(),
        device_note(device),
    )
}

/// The LMS block's note, when LMS changed something on the way out: converted the codec,
/// lowered the rate or depth, or applied server ReplayGain. `None`: LMS sent the file as
/// it is (or nothing is known), and the diagram has no LMS block.
fn lms_changes(
    file: Option<&Fmt>,
    served: Option<&Fmt>,
    gain: Option<f64>,
) -> Option<(String, bool)> {
    let mut parts: Vec<String> = Vec::new();
    let mut lowered = false;
    if let (Some(f), Some(s)) = (file, served) {
        if !f.same_codec(s) {
            parts.push(format!("converted from {}", f.name));
        }
        // DSD and PCM rates and depths are not comparable.
        if f.name != "DSD" && s.lossless {
            let rate = matches!((f.rate, s.rate), (Some(a), Some(b)) if b < a);
            let bits = matches!((f.bits, s.bits), (Some(a), Some(b)) if b < a && f.lossless);
            let what = match (bits, rate, s.bits, s.rate) {
                (true, true, Some(b), Some(r)) => Some(format!("{b}-bit, {} kHz", khz(r))),
                (true, false, Some(b), _) => Some(format!("{b}-bit")),
                (false, true, _, Some(r)) => Some(format!("{} kHz", khz(r))),
                _ => None,
            };
            if let Some(what) = what {
                lowered = true;
                parts.push(format!("lowered to {what}"));
            }
        }
    }
    if let Some(g) = gain.filter(|g| g.is_finite() && g.abs() >= 0.05 && g.abs() <= 100.0) {
        parts.push(format!("ReplayGain {}", fmt_db(g)));
    }
    if parts.is_empty() {
        return None;
    }
    Some((capitalise(parts.join(", ")), lowered))
}

/// What the gateway block says it did.
fn gateway_block(sent: Option<&Sent>, out: Option<&Fmt>, origin: Option<&Origin>) -> (Block, bool) {
    let (format, note, reencoded) = match sent {
        Some(Sent::Passthrough) => (
            out.map(Fmt::label_with_channels).unwrap_or_default(),
            "Passed through unchanged".to_string(),
            false,
        ),
        Some(Sent::Opus { kbps, .. }) => {
            let k = kbps_of(*kbps);
            let action = match k {
                Some(k) => format!("Re-encoded to Opus {k} kb/s"),
                None => "Re-encoded to Opus".into(),
            };
            let note = match origin {
                Some(o) => format!("{action} ({})", o.text()),
                None => action,
            };
            (opus_fmt(*kbps).label(), note, true)
        }
        None => (String::new(), String::new(), false),
    };
    (
        block(BlockKind::Gateway, "Lyrion gateway", format, note),
        reencoded,
    )
}

fn switch_of(inputs: &QualityInputs) -> Switch {
    let in_force = match (&inputs.origin, &inputs.sent) {
        (Some(o), _) => o.level(),
        (None, Some(Sent::Opus { level, .. })) => match level {
            Some(OpusLevel::Low) => Level::Low,
            _ => Level::High,
        },
        _ => Level::Original,
    };
    Switch {
        levels: [Level::Original, Level::High, Level::Low],
        in_force,
        origin_text: inputs.origin.as_ref().map(Origin::text).unwrap_or_default(),
        overridden: matches!(inputs.origin, Some(Origin::Override { .. })),
        seekable: inputs.seekable,
    }
}

fn set_lowers(blocks: &mut [Block], kind: BlockKind) {
    if let Some(b) = blocks.iter_mut().find(|b| b.kind == kind) {
        b.lowers = true;
    }
}

/// The badge, the diagram and the switch for a playing track.
pub fn describe(inputs: &QualityInputs) -> Diagram {
    let file = inputs.file.as_ref().and_then(file_fmt);
    let decoded = inputs.decoded.as_ref().and_then(decoded_fmt);
    match &inputs.player {
        Player::Remote { name } => {
            let name = clean(name, MAX_NAME);
            let mut blocks = vec![
                file_block(file.as_ref()),
                block(
                    BlockKind::Player,
                    if name.is_empty() { "Player" } else { &name },
                    String::new(),
                    "Plays the stream from LMS".into(),
                ),
            ];
            let badge = file.as_ref().map(|f| {
                let v = f.verdict();
                if v == Verdict::Lossy {
                    blocks[0].lowers = true;
                }
                badge(v, f.label())
            });
            Diagram {
                badge,
                blocks,
                switch: None,
            }
        }
        Player::Offline { format } => {
            let label = clean(format, MAX_NAME);
            let mut blocks = vec![
                block(
                    BlockKind::Download,
                    "Download",
                    label.clone(),
                    "Saved on this device".into(),
                ),
                device_block(decoded.as_ref(), inputs.device.as_ref()),
            ];
            let badge = decoded.as_ref().map(|d| {
                let v = verdict_of(file.as_ref(), d);
                if v.is_lowered() {
                    blocks[0].lowers = true;
                }
                let stream = if label.is_empty() { d.label() } else { label };
                badge(v, stream)
            });
            Diagram {
                badge,
                blocks,
                switch: None,
            }
        }
        Player::Local => {
            let served = inputs
                .served
                .as_ref()
                .and_then(|s| served_fmt(s, file.as_ref()));
            let sent = inputs.sent.as_ref();
            let mut blocks = vec![file_block(file.as_ref())];
            let lms = lms_changes(
                file.as_ref(),
                served.as_ref(),
                inputs.served.as_ref().and_then(|s| s.gain_db),
            );
            let lms_lowered = lms.as_ref().is_some_and(|(_, l)| *l);
            if let Some((note, _)) = lms {
                blocks.push(block(
                    BlockKind::Lms,
                    "LMS",
                    served
                        .as_ref()
                        .map(Fmt::label_with_channels)
                        .unwrap_or_default(),
                    note,
                ));
            }
            let (gateway, reencoded) = gateway_block(
                sent,
                served.as_ref().or(file.as_ref()),
                inputs.origin.as_ref(),
            );
            blocks.push(gateway);
            blocks.push(device_block(decoded.as_ref(), inputs.device.as_ref()));

            let heard = heard_local(file.as_ref(), served.as_ref(), sent, decoded.as_ref());
            let badge = heard.map(|h| {
                let v = verdict_of(file.as_ref(), &h);
                let file_lossy = file.as_ref().is_some_and(|f| !f.lossless);
                let served_lossy = served.as_ref().is_some_and(|s| !s.lossless);
                let lowering = match v {
                    _ if file_lossy => Some(BlockKind::File),
                    Verdict::Transcoded | Verdict::Lossy if served_lossy => Some(BlockKind::Lms),
                    Verdict::Transcoded | Verdict::Lossy if reencoded => Some(BlockKind::Gateway),
                    Verdict::Lossless if lms_lowered && file.as_ref().is_some_and(Fmt::hi_res) => {
                        Some(BlockKind::Lms)
                    }
                    _ => None,
                };
                if let Some(kind) = lowering {
                    set_lowers(&mut blocks, kind);
                }
                badge(v, h.label())
            });
            Diagram {
                badge,
                blocks,
                switch: Some(switch_of(inputs)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac(rate: i64, bits: i64) -> FileFormat {
        FileFormat {
            kind: Some("flc".into()),
            rate: Some(rate),
            bits: Some(bits),
            ..FileFormat::default()
        }
    }

    fn served(codec: &str, rate: Option<i64>, bits: Option<i64>, gain: Option<f64>) -> Served {
        Served {
            codec: Some(codec.into()),
            rate,
            bits,
            channels: Some(2),
            gain_db: gain,
        }
    }

    fn dec(codec: &str, rate: i64, bits: Option<i64>, lossless: bool) -> Decoded {
        Decoded {
            codec: Some(codec.into()),
            rate: Some(rate),
            bits,
            channels: Some(2),
            lossless,
        }
    }

    fn local(file: FileFormat, s: Served, sent: Sent, d: Decoded) -> QualityInputs {
        QualityInputs {
            file: Some(file),
            served: Some(s),
            sent: Some(sent),
            decoded: Some(d),
            ..QualityInputs::default()
        }
    }

    fn opus(kbps: i64, level: OpusLevel) -> Sent {
        Sent::Opus {
            kbps: Some(kbps),
            level: Some(level),
        }
    }

    fn kinds(d: &Diagram) -> Vec<BlockKind> {
        d.blocks.iter().map(|b| b.kind).collect()
    }

    #[test]
    fn khz_formats_rates() {
        assert_eq!(khz(44_100), "44.1");
        assert_eq!(khz(48_000), "48");
        assert_eq!(khz(88_200), "88.2");
        assert_eq!(khz(96_000), "96");
        assert_eq!(khz(176_400), "176.4");
        assert_eq!(khz(192_000), "192");
        assert_eq!(khz(22_050), "22.1");
        assert_eq!(khz(11_025), "11");
        assert_eq!(khz(8_000), "8");
        assert_eq!(khz(0), "0");
        assert_eq!(khz(u32::MAX), "4294967.3");
    }

    #[test]
    fn labels_follow_the_badge_rules() {
        assert_eq!(
            format_label("FLAC", true, Some(24), Some(96_000), None),
            "FLAC 24/96"
        );
        assert_eq!(
            format_label("FLAC", true, None, Some(44_100), None),
            "FLAC 44.1 kHz"
        );
        assert_eq!(format_label("MP3", false, None, None, Some(320)), "MP3 320");
        assert_eq!(
            format_label("Opus", false, None, Some(48_000), None),
            "Opus"
        );
        assert_eq!(
            format_label("DSD", true, Some(1), Some(2_822_400), None),
            "DSD64"
        );
        assert_eq!(
            format_label("DSD", true, Some(1), Some(5_644_800), None),
            "DSD128"
        );
        assert_eq!(format_label("DSD", true, None, Some(44_100), None), "DSD");
        assert_eq!(format_label("DSD", true, None, None, None), "DSD");
        assert!(is_hi_res(true, Some(24), Some(44_100)));
        assert!(is_hi_res(true, Some(16), Some(96_000)));
        assert!(!is_hi_res(true, Some(16), Some(48_000)));
        assert!(!is_hi_res(false, Some(24), Some(96_000)));
    }

    #[test]
    fn codec_table() {
        assert_eq!(codec("flc"), Some(("FLAC", true)));
        assert_eq!(codec(" FLAC "), Some(("FLAC", true)));
        assert_eq!(codec("ops"), Some(("Opus", false)));
        assert_eq!(codec("dsf"), Some(("DSD", true)));
        assert_eq!(codec("xyz"), None);
        assert_eq!(codec_name("xyz"), Some(("XYZ".into(), false)));
        assert_eq!(codec_name("toolongname"), None);
        assert_eq!(codec_name("a/b"), None);
        assert_eq!(codec_name(""), None);
    }

    #[test]
    fn badge_text_for_each_verdict() {
        let d = describe(&local(
            flac(96_000, 24),
            served("flac", Some(96_000), Some(24), None),
            Sent::Passthrough,
            dec("FLAC", 96_000, Some(24), true),
        ));
        assert_eq!(d.badge.unwrap().text, "Hi-Res \u{b7} FLAC 24/96");

        let d = describe(&local(
            flac(44_100, 16),
            served("flac", Some(44_100), Some(16), None),
            Sent::Passthrough,
            dec("FLAC", 44_100, Some(16), true),
        ));
        assert_eq!(d.badge.unwrap().text, "Lossless \u{b7} FLAC 16/44.1");

        let d = describe(&local(
            flac(96_000, 24),
            served("flac", Some(96_000), Some(24), None),
            opus(160, OpusLevel::High),
            dec("Opus", 48_000, None, false),
        ));
        let b = d.badge.unwrap();
        assert_eq!(b.verdict, Verdict::Transcoded);
        assert_eq!(b.stream, "Opus 160");
        assert_eq!(b.text, "Transcoded \u{b7} Opus 160");

        let mp3 = FileFormat {
            kind: Some("mp3".into()),
            rate: Some(44_100),
            kbps: Some(320),
            ..FileFormat::default()
        };
        let d = describe(&local(
            mp3,
            served("mp3", None, None, None),
            Sent::Passthrough,
            dec("MP3", 44_100, None, false),
        ));
        assert_eq!(d.badge.unwrap().text, "Lossy \u{b7} MP3 320");
    }

    #[test]
    fn route_effects_never_change_the_verdict() {
        let mut i = local(
            flac(96_000, 24),
            served("flac", Some(96_000), Some(24), Some(-6.0)),
            Sent::Passthrough,
            dec("FLAC", 96_000, Some(24), true),
        );
        i.device = Some(Device {
            name: Some("Phone speaker".into()),
            rate: Some(48_000),
            resampled: true,
            dsp_active: true,
            ..Device::default()
        });
        let d = describe(&i);
        assert_eq!(d.badge.unwrap().verdict, Verdict::HiRes);
        assert!(d.blocks.iter().all(|b| !b.lowers));
        assert_eq!(
            d.blocks.last().unwrap().note,
            "Phone speaker, resampled to 48 kHz, processed by DSP you turned on"
        );
    }

    #[test]
    fn lms_block_only_when_lms_changed_something() {
        // As stored: no block.
        let d = describe(&local(
            flac(96_000, 24),
            served("flac", Some(96_000), Some(24), None),
            Sent::Passthrough,
            dec("FLAC", 96_000, Some(24), true),
        ));
        assert!(!kinds(&d).contains(&BlockKind::Lms));
        // Stream message without depth and rate (STREAMINFO unread): nothing to report.
        let d = describe(&local(
            flac(96_000, 24),
            served("flac", None, None, None),
            Sent::Passthrough,
            dec("FLAC", 96_000, Some(24), true),
        ));
        assert!(!kinds(&d).contains(&BlockKind::Lms));
        // A WAV file reaches the player as PCM: not a conversion.
        let wav = FileFormat {
            kind: Some("wav".into()),
            ..flac(44_100, 16)
        };
        let d = describe(&local(
            wav,
            served("pcm", Some(44_100), Some(16), None),
            Sent::Passthrough,
            dec("PCM", 44_100, Some(16), true),
        ));
        assert!(!kinds(&d).contains(&BlockKind::Lms));
        // A zero gain is no gain.
        let d = describe(&local(
            flac(44_100, 16),
            served("flac", Some(44_100), Some(16), Some(0.0)),
            Sent::Passthrough,
            dec("FLAC", 44_100, Some(16), true),
        ));
        assert!(!kinds(&d).contains(&BlockKind::Lms));

        // Converted codec.
        let alac = FileFormat {
            kind: Some("alc".into()),
            ..flac(44_100, 16)
        };
        let d = describe(&local(
            alac,
            served("flac", Some(44_100), Some(16), None),
            Sent::Passthrough,
            dec("FLAC", 44_100, Some(16), true),
        ));
        let lms = &d.blocks[1];
        assert_eq!(lms.kind, BlockKind::Lms);
        assert_eq!(lms.note, "Converted from ALAC");
        assert!(!lms.lowers);

        // Lowered, and the verdict with it.
        let d = describe(&local(
            flac(96_000, 24),
            served("flac", Some(44_100), Some(16), None),
            Sent::Passthrough,
            dec("FLAC", 44_100, Some(16), true),
        ));
        assert_eq!(d.blocks[1].note, "Lowered to 16-bit, 44.1 kHz");
        assert!(d.blocks[1].lowers);
        assert_eq!(d.badge.unwrap().verdict, Verdict::Lossless);

        // Lowered the rate only, still Hi-Res: shown, not lowering the verdict.
        let d = describe(&local(
            flac(96_000, 24),
            served("flac", Some(48_000), Some(24), None),
            Sent::Passthrough,
            dec("FLAC", 48_000, Some(24), true),
        ));
        assert_eq!(d.blocks[1].note, "Lowered to 48 kHz");
        assert!(!d.blocks[1].lowers);
        assert_eq!(d.badge.unwrap().verdict, Verdict::HiRes);

        // Gain, with a conversion.
        let alac = FileFormat {
            kind: Some("alc".into()),
            ..flac(44_100, 16)
        };
        let d = describe(&local(
            alac,
            served("flac", Some(44_100), Some(16), Some(-3.24)),
            Sent::Passthrough,
            dec("FLAC", 44_100, Some(16), true),
        ));
        assert_eq!(
            d.blocks[1].note,
            "Converted from ALAC, ReplayGain \u{2212}3.2 dB"
        );
        assert!(!d.blocks[1].lowers);

        // LMS transcoding a lossless file to MP3: the LMS block lowers.
        let d = describe(&local(
            flac(44_100, 16),
            served("mp3", Some(44_100), None, None),
            Sent::Passthrough,
            dec("MP3", 44_100, None, false),
        ));
        assert_eq!(d.blocks[1].note, "Converted from FLAC");
        assert!(d.blocks[1].lowers);
        assert_eq!(d.badge.unwrap().text, "Transcoded \u{b7} MP3");
    }

    #[test]
    fn reason_text_for_each_origin() {
        let cases = [
            (
                Origin::Setting {
                    network: Some(Network::Mobile),
                    level: Level::High,
                },
                "High, on mobile data",
            ),
            (
                Origin::Setting {
                    network: Some(Network::Wifi),
                    level: Level::Original,
                },
                "Original, on Wi-Fi",
            ),
            (
                Origin::Setting {
                    network: None,
                    level: Level::Low,
                },
                "Data saver, chosen in Settings",
            ),
            (
                Origin::Override {
                    level: Level::Original,
                },
                "Original, overridden here",
            ),
            (
                Origin::Automatic { level: Level::High },
                "High, chosen automatically",
            ),
        ];
        for (o, want) in cases {
            assert_eq!(o.text(), want);
            let mut i = local(
                flac(96_000, 24),
                served("flac", Some(96_000), Some(24), None),
                opus(160, OpusLevel::High),
                dec("Opus", 48_000, None, false),
            );
            i.origin = Some(o.clone());
            let d = describe(&i);
            let gw = d
                .blocks
                .iter()
                .find(|b| b.kind == BlockKind::Gateway)
                .unwrap();
            assert_eq!(gw.note, format!("Re-encoded to Opus 160 kb/s ({want})"));
            assert!(gw.lowers);
            let sw = d.switch.unwrap();
            assert_eq!(sw.origin_text, want);
            assert_eq!(sw.in_force, o.level());
            assert_eq!(sw.overridden, matches!(o, Origin::Override { .. }));
        }
    }

    #[test]
    fn switch_without_an_origin_follows_the_stream() {
        let mut i = local(
            flac(96_000, 24),
            served("flac", None, None, None),
            opus(64, OpusLevel::Low),
            dec("Opus", 48_000, None, false),
        );
        assert_eq!(describe(&i).switch.unwrap().in_force, Level::Low);
        i.sent = Some(Sent::Passthrough);
        assert_eq!(describe(&i).switch.unwrap().in_force, Level::Original);
        i.player = Player::Remote {
            name: "Kitchen".into(),
        };
        assert!(describe(&i).switch.is_none());
    }

    #[test]
    fn nothing_known_gives_no_badge() {
        let d = describe(&QualityInputs::default());
        assert!(d.badge.is_none());
        assert_eq!(
            kinds(&d),
            [BlockKind::File, BlockKind::Gateway, BlockKind::Device]
        );
    }

    #[test]
    fn inputs_are_cleaned() {
        let i = QualityInputs {
            player: Player::Remote {
                name: format!("  Living\u{7}room\n{}", "x".repeat(500)),
            },
            ..QualityInputs::default()
        };
        let d = describe(&i);
        let t = &d.blocks[1].title;
        assert!(t.starts_with("Livingroom"));
        assert_eq!(t.chars().count(), MAX_NAME);
        let i = QualityInputs {
            player: Player::Remote { name: " \n".into() },
            ..QualityInputs::default()
        };
        assert_eq!(describe(&i).blocks[1].title, "Player");
        // Bidirectional and zero-width controls cannot disguise a name.
        let i = QualityInputs {
            player: Player::Remote {
                name: "Kit\u{202e}nehc\u{200b}\u{2066}\u{feff}".into(),
            },
            ..QualityInputs::default()
        };
        assert_eq!(describe(&i).blocks[1].title, "Kitnehc");
    }
}
