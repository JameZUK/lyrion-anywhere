//! Lyrics: plain text or LRC (synced) from a track's tags or the Music & Artist Information
//! plugin, parsed for the lyrics view (docs/plans/wave1-design.md §9).
//!
//! The text is untrusted (tags are written by anyone), so parsing is bounded and never
//! panics:
//! - at most [`MAX_INPUT`] bytes are read (cut at a character boundary), at most
//!   [`MAX_LINES`] lines are kept, and each line is cut to [`MAX_LINE_CHARS`] characters;
//! - times are checked (`mm:ss`, `mm:ss.f`, `mm:ss.ff`, `mm:ss.fff` or `mm:ss:ff`, minutes
//!   up to 999, seconds under 60), and anything else in brackets is text;
//! - `[offset:±ms]` is applied, saturating at zero and [`u32::MAX`].
//!
//! Lyrics are **synced** when at least one line carries a time. Then untimed lines (such as
//! a stray `[Chorus]`) are dropped, a line with several times appears once for each, and the
//! lines are sorted by time (stable, so equal times keep their order). Otherwise every line is
//! kept as it is, in order. Metadata lines (`[ar:…]`, `[ti:…]`, `[length:…]` and so on) are
//! dropped in both cases. Enhanced LRC's word times (`<mm:ss.ff>`) are removed from the text.

/// Most input read, bytes. The largest tag lyrics seen in a real library were under 10 KiB.
pub const MAX_INPUT: usize = 256 * 1024;
/// Most lines kept.
pub const MAX_LINES: usize = 4000;
/// Longest line kept, characters.
pub const MAX_LINE_CHARS: usize = 500;

/// One line of lyrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// When it starts, ms from the start of the track (0 for plain lyrics).
    pub time_ms: u32,
    /// The words, trimmed; empty for an instrumental gap in synced lyrics.
    pub text: String,
}

/// Parsed lyrics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lyrics {
    /// Whether the lines carry times.
    pub synced: bool,
    pub lines: Vec<Line>,
}

impl Lyrics {
    /// Parse plain or LRC lyrics.
    pub fn parse(input: &str) -> Lyrics {
        let input = cut(input, MAX_INPUT);
        let mut offset_ms: i64 = 0;
        let mut timed: Vec<Line> = Vec::new();
        let mut plain: Vec<Line> = Vec::new();
        for raw in input.split('\n') {
            if timed.len() >= MAX_LINES && plain.len() >= MAX_LINES {
                break;
            }
            let raw = raw.trim_end_matches('\r');
            if let Some((key, value)) = metadata(raw) {
                if key.eq_ignore_ascii_case("offset") {
                    offset_ms = value
                        .trim()
                        .parse::<i64>()
                        .unwrap_or(0)
                        .clamp(-86_400_000, 86_400_000);
                }
                continue;
            }
            let (times, rest) = leading_times(raw);
            let text = clean(rest);
            if times.is_empty() {
                if plain.len() < MAX_LINES {
                    plain.push(Line {
                        time_ms: 0,
                        text: clean(raw),
                    });
                }
            } else {
                for t in times {
                    if timed.len() < MAX_LINES {
                        timed.push(Line {
                            time_ms: t,
                            text: text.clone(),
                        });
                    }
                }
            }
        }
        if timed.is_empty() {
            // Plain: drop leading and trailing blank lines.
            let start = plain
                .iter()
                .position(|l| !l.text.is_empty())
                .unwrap_or(plain.len());
            let end = plain
                .iter()
                .rposition(|l| !l.text.is_empty())
                .map_or(start, |i| i + 1);
            return Lyrics {
                synced: false,
                lines: plain[start..end].to_vec(),
            };
        }
        for l in &mut timed {
            let t = i64::from(l.time_ms) - offset_ms;
            l.time_ms = t.clamp(0, i64::from(u32::MAX)) as u32;
        }
        timed.sort_by_key(|l| l.time_ms);
        Lyrics {
            synced: true,
            lines: timed,
        }
    }

    /// The line playing at `pos_ms`: the last one starting at or before it. `None` for plain
    /// lyrics, and before the first line.
    pub fn line_at(&self, pos_ms: u32) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let n = self.lines.partition_point(|l| l.time_ms <= pos_ms);
        n.checked_sub(1)
    }

    /// Whether there are no words at all.
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.text.is_empty())
    }
}

/// `s` cut to at most `max` bytes, at a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A whole-line metadata tag: `[key:value]`, key 1–10 ASCII letters (or `#`), that isn't a
/// time.
fn metadata(line: &str) -> Option<(&str, &str)> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (key, value) = inner.split_once(':')?;
    let ok = !key.is_empty()
        && key.len() <= 10
        && key.bytes().all(|b| b.is_ascii_alphabetic() || b == b'#');
    ok.then_some((key, value))
}

/// The time tags at the start of a line (whitespace allowed between them), in ms, and the
/// rest of the line.
fn leading_times(line: &str) -> (Vec<u32>, &str) {
    let mut times = Vec::new();
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start();
        let Some(after) = trimmed.strip_prefix('[') else {
            break;
        };
        let Some(close) = after.find(']') else { break };
        let Some(t) = parse_time(&after[..close]) else {
            break;
        };
        // A line can't carry more times than lines we keep.
        if times.len() < MAX_LINES {
            times.push(t);
        }
        rest = &after[close + 1..];
    }
    let rest = if times.is_empty() { line } else { rest };
    (times, rest)
}

/// `mm:ss`, `mm:ss.f{1,3}` or `mm:ss:ff`, in ms.
fn parse_time(s: &str) -> Option<u32> {
    let (min, rest) = s.split_once(':')?;
    let (sec, frac) = match rest.find(['.', ':']) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let digits = |t: &str, lo: usize, hi: usize| {
        (lo..=hi).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit())
    };
    if !digits(min, 1, 3) || !digits(sec, 2, 2) {
        return None;
    }
    let min: u32 = min.parse().ok()?;
    let sec: u32 = sec.parse().ok()?;
    if sec >= 60 {
        return None;
    }
    let ms = match frac {
        None => 0,
        Some(f) if digits(f, 1, 3) => {
            let v: u32 = f.parse().ok()?;
            match f.len() {
                1 => v * 100,
                2 => v * 10,
                _ => v,
            }
        }
        Some(_) => return None,
    };
    Some(min * 60_000 + sec * 1000 + ms)
}

/// The words: enhanced-LRC word times removed, control characters dropped, whitespace
/// trimmed, and cut to [`MAX_LINE_CHARS`].
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_LINE_CHARS * 4));
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('>') {
            Some(j) if parse_time(&after[..j]).is_some() => rest = &after[j + 1..],
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let words: String = out
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    words
        .trim()
        .chars()
        .take(MAX_LINE_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &Lyrics) -> Vec<(u32, &str)> {
        l.lines
            .iter()
            .map(|x| (x.time_ms, x.text.as_str()))
            .collect()
    }

    #[test]
    fn plain_lyrics_keep_their_lines() {
        let l = Lyrics::parse("\r\n\nFirst line\r\n\n[Chorus]\nSecond  \n\n");
        assert!(!l.synced);
        assert_eq!(
            texts(&l),
            vec![(0, "First line"), (0, ""), (0, "[Chorus]"), (0, "Second")]
        );
        assert_eq!(l.line_at(10_000), None);
    }

    #[test]
    fn synced_lyrics_are_timed_and_sorted() {
        let l = Lyrics::parse(
            "[ar:Someone]\n[ti:Song]\n[00:12.23]Two\n[00:08.58] One\n[Chorus]\n[01:02]Three\n[00:20.5]\n",
        );
        assert!(l.synced);
        assert_eq!(
            texts(&l),
            vec![(8580, "One"), (12230, "Two"), (20500, ""), (62000, "Three")]
        );
        assert_eq!(l.line_at(0), None);
        assert_eq!(l.line_at(8579), None);
        assert_eq!(l.line_at(8580), Some(0));
        assert_eq!(l.line_at(15_000), Some(1));
        assert_eq!(l.line_at(u32::MAX), Some(3));
    }

    #[test]
    fn repeated_times_and_formats() {
        let l = Lyrics::parse("[00:01.000][00:30:50] [999:59.9]Again\n");
        assert_eq!(
            texts(&l),
            vec![
                (1000, "Again"),
                (30500, "Again"),
                (999 * 60_000 + 59_900, "Again")
            ]
        );
    }

    #[test]
    fn offset_shifts_and_saturates() {
        let l = Lyrics::parse("[offset:+500]\n[00:01.00]A\n[00:00.20]B\n");
        assert_eq!(texts(&l), vec![(0, "B"), (500, "A")]);
        let l = Lyrics::parse("[offset:-1000]\n[00:01.00]A\n");
        assert_eq!(texts(&l), vec![(2000, "A")]);
        let l = Lyrics::parse("[offset:nonsense]\n[00:01.00]A\n");
        assert_eq!(texts(&l), vec![(1000, "A")]);
    }

    #[test]
    fn enhanced_word_times_are_removed() {
        let l = Lyrics::parse("[00:01.00]<00:01.00>Hello <00:01.50>world <b>bold</b> 1<2\n");
        assert_eq!(texts(&l), vec![(1000, "Hello world <b>bold</b> 1<2")]);
    }

    #[test]
    fn bad_times_are_text() {
        for s in [
            "[00:60]x",
            "[1:2]x",
            "[0000:01]x",
            "[aa:bb]x",
            "[00:01.1234]x",
            "[:01]x",
            "[00:01.]x",
            "[-1:00]x",
        ] {
            let l = Lyrics::parse(s);
            assert!(!l.synced, "{s}");
            assert_eq!(l.lines.len(), 1, "{s}");
        }
    }

    #[test]
    fn control_characters_and_long_lines() {
        let long = "é".repeat(MAX_LINE_CHARS + 50);
        let l = Lyrics::parse(&format!("a\u{0}b\u{1b}[31mc\t d\n{long}"));
        assert_eq!(l.lines[0].text, "ab[31mc  d");
        assert_eq!(l.lines[1].text.chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn limits_hold() {
        let many = "[00:01]x\n".repeat(MAX_LINES * 3);
        let l = Lyrics::parse(&many);
        assert_eq!(l.lines.len(), MAX_LINES);
        let many_tags = format!("{}x", "[00:01]".repeat(MAX_LINES * 3));
        assert_eq!(Lyrics::parse(&many_tags).lines.len(), MAX_LINES);
        let huge = "ü".repeat(MAX_INPUT); // 2 bytes each, cut mid-way at a boundary
        let l = Lyrics::parse(&huge);
        assert_eq!(l.lines.len(), 1);
        let plain = "x\n".repeat(MAX_LINES * 3);
        assert_eq!(Lyrics::parse(&plain).lines.len(), MAX_LINES);
    }

    #[test]
    fn empty_input() {
        for s in ["", "\n\n", "[ar:x]\n[ti:y]", "   "] {
            let l = Lyrics::parse(s);
            assert!(l.is_empty(), "{s:?}");
            assert_eq!(l.line_at(0), None);
        }
    }
}
