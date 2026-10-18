//! Names a client chooses for things the music server stores as files: a saved playlist
//! (LMS writes `<name>.m3u` into its playlist folder). Checked the same way by the gateway,
//! which enforces it, and by the apps, which say what's wrong before sending.

/// Longest playlist name, in characters.
pub const PLAYLIST_NAME_MAX: usize = 100;

/// Characters LMS or common filesystems won't take in a file name (and `:` would turn the
/// name into a tagged parameter of an LMS command).
pub const RESERVED: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Windows device names: a file of that name (with any extension) can't be created there.
const DEVICES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Invisible characters that could disguise a name: zero-width and joiners, bidirectional
/// overrides and isolates, line and paragraph separators, the byte-order mark.
pub fn invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}')
}

/// Longest player name kept from the server, in characters.
pub const PLAYER_NAME_MAX: usize = 64;

/// A player name the server sent (a `setd` rename), made safe to show: control and
/// invisible characters removed, whitespace trimmed, at most [`PLAYER_NAME_MAX`]
/// characters. `None` if nothing is left.
pub fn player_name(raw: &str) -> Option<String> {
    let clean: String = raw
        .chars()
        .filter(|c| !c.is_control() && !invisible(*c))
        .collect();
    let name: String = clean.trim().chars().take(PLAYER_NAME_MAX).collect();
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_string())
}

/// The name a playlist may be saved under (trimmed), or why not. Any script is fine;
/// clients send names composed (NFC).
pub fn playlist_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("a name is needed");
    }
    if name.chars().count() > PLAYLIST_NAME_MAX {
        return Err("names are at most 100 characters");
    }
    if name.chars().any(|c| c.is_control() || invisible(c)) {
        return Err("names can't contain control or invisible characters");
    }
    if name.contains(RESERVED) {
        return Err("names can't contain / \\ : * ? \" < > |");
    }
    if name.contains("..") {
        return Err("names can't contain ..");
    }
    if name.starts_with('.') {
        return Err("names can't start with a dot");
    }
    if name.ends_with('.') {
        return Err("names can't end with a dot");
    }
    let stem = name.split('.').next().unwrap_or("").trim_end();
    if DEVICES.iter().any(|d| stem.eq_ignore_ascii_case(d)) {
        return Err("that name is reserved");
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        for (raw, want) in [
            ("Road trip", "Road trip"),
            ("  Sunday morning \t", "Sunday morning"),
            ("Sigur Rós – live", "Sigur Rós – live"),
            ("日本語のプレイリスト", "日本語のプレイリスト"),
            ("Mix vol. 2", "Mix vol. 2"),
            ("AC-DC & friends (2026)", "AC-DC & friends (2026)"),
            ("Console", "Console"),
        ] {
            assert_eq!(playlist_name(raw).as_deref(), Ok(want), "{raw:?}");
        }
        assert!(
            playlist_name(&"é".repeat(100)).is_ok(),
            "characters, not bytes"
        );
        for bad in [
            "",
            "   ",
            "../etc/passwd",
            "..",
            "a..b",
            "a/b",
            "AC/DC",
            "a\\b",
            "C:\\x",
            "name:x",
            "a*b",
            "what?",
            "say \"hi\"",
            "<b>",
            "a|b",
            ".hidden",
            "trailing.",
            "line\nbreak",
            "tab\tinside",
            "nul\0",
            "esc\u{1b}[31m",
            "del\u{7f}",
            "c1\u{85}x",
            "rtl\u{202E}gpj.exe",
            "zero\u{200B}width",
            "bom\u{FEFF}",
            "CON",
            "nul.txt",
            "Lpt1",
            "com9 .m3u",
        ] {
            assert!(playlist_name(bad).is_err(), "{bad:?}");
        }
        assert!(playlist_name(&"a".repeat(101)).is_err());
    }

    #[test]
    fn player_names_are_cleaned_and_bounded() {
        assert_eq!(player_name("Kitchen").as_deref(), Some("Kitchen"));
        assert_eq!(
            player_name("  Living\u{202E}room\u{0}\n\t ").as_deref(),
            Some("Livingroom")
        );
        assert_eq!(player_name("Sigur Rós").as_deref(), Some("Sigur Rós"));
        assert_eq!(player_name(""), None);
        assert_eq!(player_name("\u{1b}[2J\u{200B}\r\n"), Some("[2J".into()));
        assert_eq!(player_name(" \u{7f}\u{feff} "), None);
        let long = "é".repeat(32 * 1024);
        assert_eq!(player_name(&long).unwrap().chars().count(), PLAYER_NAME_MAX);
        // Cut at a space: no trailing whitespace.
        let spaced = format!("{} b", "a".repeat(PLAYER_NAME_MAX - 1));
        assert_eq!(
            player_name(&spaced).unwrap(),
            "a".repeat(PLAYER_NAME_MAX - 1)
        );
    }

    /// Arbitrary text never panics the check, and whatever it accepts keeps every rule.
    #[test]
    fn the_check_never_panics() {
        let mut seed: u64 = 0x5eed_1234_abcd_ef01;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let alphabet: Vec<char> = "ab Z.é/\\:*?\"<>|\0\n\t\u{7f}\u{202E}\u{200B}日 ..-_~%CON"
            .chars()
            .chain(['\u{10FFFF}', '\u{FFFD}', '\u{0301}'])
            .collect();
        for _ in 0..20_000 {
            let len = (next() % 130) as usize;
            let s: String = (0..len)
                .map(|_| {
                    if next() % 5 == 0 {
                        char::from_u32((next() % 0x11_0000) as u32).unwrap_or('x')
                    } else {
                        alphabet[(next() as usize) % alphabet.len()]
                    }
                })
                .collect();
            if let Ok(n) = playlist_name(&s) {
                assert!(
                    !n.is_empty() && n.chars().count() <= PLAYLIST_NAME_MAX,
                    "{n:?}"
                );
                assert!(!n.contains(RESERVED) && !n.contains(".."), "{n:?}");
                assert!(!n.starts_with('.') && !n.ends_with('.'), "{n:?}");
                assert!(!n.chars().any(|c| c.is_control() || invisible(c)), "{n:?}");
                assert_eq!(n, n.trim());
            }
        }
    }
}
