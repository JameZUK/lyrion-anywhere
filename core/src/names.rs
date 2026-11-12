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

/// Characters that draw nothing, any of which could make a name read as something else:
/// controls (`Cc`), the line and paragraph separators, every format character (`Cf`,
/// Unicode 16: the soft hyphen, zero-width and bidirectional controls, the Arabic and Syriac
/// marks, the tag characters, …) and the other characters that draw nothing (the combining
/// grapheme joiner, the Hangul fillers, the variation selectors). The one definition, for
/// the names checked here and the text the signal-path panel shows
/// ([`crate::quality`]).
pub fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{034f}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{115f}'..='\u{1160}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{3164}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0000}'..='\u{e007f}'
                | '\u{e0100}'..='\u{e01ef}'
        )
}

/// Longest player name kept from the server, in characters.
pub const PLAYER_NAME_MAX: usize = 64;

/// A player name the server sent (a `setd` rename), made safe to show: the [`invisible`]
/// characters removed, whitespace trimmed, at most [`PLAYER_NAME_MAX`]
/// characters. `None` if nothing is left.
pub fn player_name(raw: &str) -> Option<String> {
    let clean: String = raw
        .chars()
        .filter(|c| !invisible(*c))
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
    if name.chars().any(invisible) {
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

    /// One definition of the characters that draw nothing, for names as for the
    /// signal-path panel (review low: this check was narrower than the panel's, so a soft
    /// hyphen, a variation selector or a tag character got through it).
    #[test]
    fn every_character_that_draws_nothing_is_refused_or_dropped() {
        for c in [
            '\u{00ad}', '\u{034f}', '\u{061c}', '\u{115f}', '\u{180e}', '\u{3164}', '\u{fe0f}',
            '\u{ffa0}', '\u{1d173}', '\u{e0001}', '\u{e0041}', '\u{e0100}', '\u{0085}',
        ] {
            assert!(invisible(c), "{:04x}", c as u32);
            let disguised = format!("Kit{c}chen");
            assert!(playlist_name(&disguised).is_err(), "{:04x}", c as u32);
            assert_eq!(
                player_name(&disguised).as_deref(),
                Some("Kitchen"),
                "{:04x}",
                c as u32
            );
        }
        for c in ['a', 'é', '\u{301}', '日', '\u{1f3b5}', '\u{fffd}', '-', ' '] {
            assert!(!invisible(c), "{:04x}", c as u32);
        }
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
                assert!(!n.chars().any(invisible), "{n:?}");
                assert_eq!(n, n.trim());
            }
        }
    }
}
