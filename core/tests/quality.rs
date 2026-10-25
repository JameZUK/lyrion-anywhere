//! The quality badge and block diagram: the shared vectors, and robustness on any input.
//!
//! `core/tests/fixtures/quality-vectors.json` is the contract the web app's TypeScript port
//! and the Android UI tests are held to, byte for byte.

use anywhere_core::quality::{
    Decoded, Device, Diagram, FileFormat, Level, Network, OpusLevel, Origin, Player, QualityInputs,
    Sent, Served, describe,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    name: String,
    input: QualityInputs,
    expect: Diagram,
}

fn vectors() -> Vec<Vector> {
    serde_json::from_str(include_str!("fixtures/quality-vectors.json"))
        .expect("the vectors parse as QualityInputs and Diagram")
}

#[test]
fn vectors_describe_as_expected() {
    let vs = vectors();
    assert!(vs.len() >= 16, "only {} vectors", vs.len());
    let mut names = std::collections::HashSet::new();
    for v in &vs {
        assert!(
            names.insert(v.name.clone()),
            "duplicate vector {:?}",
            v.name
        );
        assert_eq!(describe(&v.input), v.expect, "{}", v.name);
    }
}

#[test]
fn vectors_match_as_json_too() {
    // What a client compares: the serialised diagram, not the Rust values.
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/quality-vectors.json")).unwrap();
    for v in raw.as_array().unwrap() {
        let input: QualityInputs = serde_json::from_value(v["input"].clone()).unwrap();
        let got = serde_json::to_value(describe(&input)).unwrap();
        assert_eq!(got, v["expect"], "{}", v["name"]);
    }
}

#[test]
fn junk_json_is_refused_or_described_without_panicking() {
    let junk = [
        "{}",
        "null",
        "[]",
        r#"{"player":{"kind":"martian"}}"#,
        r#"{"player":{"kind":"remote"}}"#,
        r#"{"file":{"kind":7,"rate":"96000"}}"#,
        r#"{"file":{"rate":1e300}}"#,
        r#"{"served":{"gain_db":"loud"}}"#,
        r#"{"sent":{"mode":"opus","kbps":-1,"level":"medium"}}"#,
        r#"{"sent":{"mode":"flac"}}"#,
        r#"{"origin":{"kind":"setting","level":"ultra"}}"#,
        r#"{"decoded":{"codec":"","lossless":"yes"}}"#,
        r#"{"file":{"kind":"flc","rate":-96000,"bits":99999999999}}"#,
    ];
    for j in junk {
        if let Ok(i) = serde_json::from_str::<QualityInputs>(j) {
            check(&describe(&i), j);
        }
    }
}

/// Small deterministic PRNG (xorshift64*), so failures are reproducible from the seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn maybe<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
        if self.below(4) == 0 {
            None
        } else {
            Some(f(self))
        }
    }
    fn num(&mut self) -> i64 {
        const EDGES: [i64; 16] = [
            0,
            1,
            -1,
            16,
            24,
            32,
            64,
            65,
            999,
            1000,
            44_100,
            48_000,
            96_000,
            2_822_400,
            i64::MAX,
            i64::MIN,
        ];
        match self.below(3) {
            0 => EDGES[self.below(EDGES.len() as u64) as usize],
            1 => self.below(200_000) as i64,
            _ => self.next() as i64,
        }
    }
    fn float(&mut self) -> f64 {
        match self.below(6) {
            0 => f64::NAN,
            1 => f64::INFINITY,
            2 => f64::NEG_INFINITY,
            3 => f64::from_bits(self.next()),
            _ => (self.below(4000) as f64 - 2000.0) / 10.0,
        }
    }
    fn text(&mut self) -> String {
        const WORDS: [&str; 18] = [
            "flc",
            "FLAC",
            "mp3",
            "ops",
            "opus",
            "ogg",
            "pcm",
            "alc",
            "alac",
            "dsf",
            "wav",
            "aac",
            "xyz",
            "Vorbis",
            "PCM (float)",
            "",
            " ",
            "Opus",
        ];
        if self.coin() {
            return WORDS[self.below(WORDS.len() as u64) as usize].to_string();
        }
        let len = self.below(1025) as usize;
        (0..len)
            .map(|_| match self.below(5) {
                0 => char::from_u32(self.below(0x20) as u32).unwrap(),
                1 => char::from_u32(0x7f + self.below(0x40) as u32).unwrap_or('?'),
                2 => char::from_u32(self.below(0x11_0000) as u32).unwrap_or('\u{fffd}'),
                _ => char::from(b' ' + self.below(95) as u8),
            })
            .collect()
    }
    fn level(&mut self) -> Level {
        [Level::Original, Level::High, Level::Low][self.below(3) as usize]
    }
    fn inputs(&mut self) -> QualityInputs {
        QualityInputs {
            player: match self.below(3) {
                0 => Player::Local,
                1 => Player::Remote { name: self.text() },
                _ => Player::Offline {
                    format: self.text(),
                },
            },
            file: self.maybe(|r| FileFormat {
                kind: r.maybe(Rng::text),
                rate: r.maybe(Rng::num),
                bits: r.maybe(Rng::num),
                kbps: r.maybe(Rng::num),
                channels: r.maybe(Rng::num),
            }),
            served: self.maybe(|r| Served {
                codec: r.maybe(Rng::text),
                rate: r.maybe(Rng::num),
                bits: r.maybe(Rng::num),
                channels: r.maybe(Rng::num),
                gain_db: r.maybe(Rng::float),
            }),
            sent: self.maybe(|r| {
                if r.coin() {
                    Sent::Passthrough
                } else {
                    Sent::Opus {
                        kbps: r.maybe(Rng::num),
                        level: r.maybe(|r| {
                            if r.coin() {
                                OpusLevel::High
                            } else {
                                OpusLevel::Low
                            }
                        }),
                    }
                }
            }),
            decoded: self.maybe(|r| Decoded {
                codec: r.maybe(Rng::text),
                rate: r.maybe(Rng::num),
                bits: r.maybe(Rng::num),
                channels: r.maybe(Rng::num),
                lossless: r.coin(),
            }),
            device: self.maybe(|r| Device {
                name: r.maybe(Rng::text),
                rate: r.maybe(Rng::num),
                bit_perfect: r.coin(),
                engine_bit_perfect: r.coin(),
                resampled: r.coin(),
                dsp_active: r.coin(),
            }),
            origin: self.maybe(|r| match r.below(3) {
                0 => Origin::Setting {
                    network: r.maybe(|r| {
                        if r.coin() {
                            Network::Wifi
                        } else {
                            Network::Mobile
                        }
                    }),
                    level: r.level(),
                },
                1 => Origin::Override { level: r.level() },
                _ => Origin::Automatic { level: r.level() },
            }),
            seekable: self.coin(),
        }
    }
}

fn check_text(s: &str, what: &str, ctx: &str) {
    assert!(
        s.chars().count() <= 120,
        "{what} is {} characters: {s:?} ({ctx})",
        s.chars().count()
    );
    assert!(
        !s.chars().any(char::is_control),
        "{what} has a control character: {s:?} ({ctx})"
    );
}

fn check(d: &Diagram, ctx: &str) {
    if let Some(b) = &d.badge {
        check_text(&b.stream, "badge stream", ctx);
        check_text(&b.text, "badge text", ctx);
        assert!(b.text.starts_with(b.verdict.word()), "{ctx}");
        assert!(b.text.ends_with(&b.stream), "{ctx}");
    }
    for b in &d.blocks {
        check_text(&b.title, "title", ctx);
        check_text(&b.format, "format", ctx);
        check_text(&b.note, "note", ctx);
    }
    // At most one block lowers the verdict, never the device, and only when it is lowered
    // or an LMS conversion lowered it from Hi-Res.
    let lowering: Vec<_> = d.blocks.iter().filter(|b| b.lowers).collect();
    assert!(lowering.len() <= 1, "{ctx}");
    assert!(
        lowering
            .iter()
            .all(|b| b.kind != anywhere_core::quality::BlockKind::Device),
        "{ctx}"
    );
    if d.badge.is_none() {
        assert!(lowering.is_empty(), "{ctx}");
    }
    if let Some(s) = &d.switch {
        check_text(&s.origin_text, "origin text", ctx);
        assert_eq!(s.levels, [Level::Original, Level::High, Level::Low]);
    }
}

#[test]
fn describe_never_panics_and_stays_short_on_random_input() {
    let mut rng = Rng(0x5eed_0f9a_117e_5700);
    for n in 0..5000 {
        let i = rng.inputs();
        let d = describe(&i);
        check(&d, &format!("case {n}"));
        // Only this device's own stream has a switch.
        assert_eq!(d.switch.is_some(), i.player == Player::Local, "case {n}");
        // The diagram round-trips through JSON, as the clients receive it.
        let j = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<Diagram>(&j).unwrap(), d, "case {n}");
    }
}
