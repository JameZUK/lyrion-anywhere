//! Reference QR symbols for the web app's in-house encoder (`web/app/src/lib/qr/qr.js`,
//! also served by the LMS plugin), made with the `qrcode` crate: byte mode, level M, the
//! smallest version that holds the text, and each of the eight masks. The JavaScript tests
//! compare their output module by module against `web/app/tests/fixtures/qr-vectors.json`;
//! this test checks that file still matches the crate.
//!
//! To rewrite the file: `UPDATE_QR_VECTORS=1 cargo test -p anywhere-gateway --test qr_vectors`.

use qrcode::bits::Bits;
use qrcode::canvas::{Canvas, MaskPattern};
use qrcode::ec::construct_codewords;
use qrcode::{Color, EcLevel, Version};
use serde_json::{Value, json};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../web/app/tests/fixtures/qr-vectors.json"
);

const MASKS: [MaskPattern; 8] = [
    MaskPattern::Checkerboard,
    MaskPattern::HorizontalLines,
    MaskPattern::VerticalLines,
    MaskPattern::DiagonalLines,
    MaskPattern::LargeCheckerboard,
    MaskPattern::Fields,
    MaskPattern::Diamonds,
    MaskPattern::Meadow,
];

/// Texts covering versions 1 to 10 (7 and up carry version information), padding of
/// both kinds, two block sizes, multi-byte UTF-8 and a full version 10.
fn texts() -> Vec<String> {
    vec![
        "A".into(),
        "https://music.example.com/".into(),
        "lyrion-anywhere://pair?code=ABCDE".into(),
        "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK".into(),
        format!(
            "lyrion-anywhere://pair?server={}&code=ABCDEFGHJKLMNPQRSTUV",
            "https%3A%2F%2Fa-rather-long-host-name.lms.example.lan%3A8443"
        ),
        "x".repeat(140),
        format!("Caf\u{e9} \u{2615} {}", "0123456789".repeat(18)),
        "z".repeat(213),
    ]
}

fn version_for(text: &[u8]) -> i16 {
    (1..=10)
        .find(|v| {
            let mut bits = Bits::new(Version::Normal(*v));
            bits.push_byte_data(text).is_ok() && bits.push_terminator(EcLevel::M).is_ok()
        })
        .expect("fits version 10")
}

/// The symbol's rows, each as hex digits (bits left to right, 1 dark, zero-padded).
fn symbol(text: &[u8], version: i16, mask: MaskPattern) -> Vec<String> {
    let v = Version::Normal(version);
    let mut bits = Bits::new(v);
    bits.push_byte_data(text).unwrap();
    bits.push_terminator(EcLevel::M).unwrap();
    let (data, ec) = construct_codewords(&bits.into_bytes(), v, EcLevel::M).unwrap();
    let mut canvas = Canvas::new(v, EcLevel::M);
    canvas.draw_all_functional_patterns();
    canvas.draw_data(&data, &ec);
    canvas.apply_mask(mask);
    let colours = canvas.into_colors();
    let size = (version * 4 + 17) as usize;
    colours
        .chunks(size)
        .map(|row| {
            let mut bits: Vec<u8> = row.iter().map(|c| u8::from(*c == Color::Dark)).collect();
            bits.resize(size.div_ceil(4) * 4, 0);
            bits.chunks(4)
                .map(|n| format!("{:x}", n.iter().fold(0, |a, b| (a << 1) | b)))
                .collect()
        })
        .collect()
}

fn vectors() -> Value {
    let list: Vec<Value> = texts()
        .into_iter()
        .map(|t| {
            let version = version_for(t.as_bytes());
            let masks: Vec<Vec<String>> = MASKS
                .iter()
                .map(|m| symbol(t.as_bytes(), version, *m))
                .collect();
            json!({ "text": t, "version": version, "masks": masks })
        })
        .collect();
    json!({
        "about": "QR symbols from the qrcode crate (byte mode, level M), one per mask; rows as hex, 1 = dark. Made by gateway/tests/qr_vectors.rs.",
        "vectors": list,
    })
}

#[test]
fn the_js_encoders_reference_symbols_match_the_qrcode_crate() {
    let want = vectors();
    let text = serde_json::to_string_pretty(&want).unwrap() + "\n";
    if std::env::var_os("UPDATE_QR_VECTORS").is_some() {
        std::fs::write(FIXTURE, &text).unwrap();
    }
    let have: Value = serde_json::from_str(
        &std::fs::read_to_string(FIXTURE).expect("the fixture exists (see the module notes)"),
    )
    .unwrap();
    assert_eq!(
        have, want,
        "{FIXTURE} is out of date; see the module notes to rewrite it"
    );
    // The set covers what the encoder supports.
    let versions: Vec<i64> = want["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_i64().unwrap())
        .collect();
    assert_eq!(versions.first(), Some(&1));
    assert_eq!(versions.last(), Some(&10));
    assert!(versions.iter().any(|v| (7..10).contains(v)));
}
