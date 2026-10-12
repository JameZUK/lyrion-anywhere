//! Lyrics for Now playing (docs/plans/wave1-design.md §9): the gateway's answer to
//! `GET /v1/lyrics/track/{id}` (parsed there by the core), checked again here as untrusted
//! input before the app shows it. The request itself is [`crate::Me::lyrics`].

use serde_json::Value;

/// Most lines, and characters in a line, accepted (the gateway keeps to the same limits).
pub const MAX_LINES: usize = 4000;
pub const MAX_LINE_CHARS: usize = 500;

/// One line of lyrics.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LyricLine {
    /// When it starts, ms from the start of the track (0 for plain lyrics).
    pub time_ms: u32,
    /// The words; empty for an instrumental gap.
    pub text: String,
}

/// A track's lyrics.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrackLyrics {
    /// From the Music & Artist Information plugin, rather than the file's tags.
    pub from_plugin: bool,
    /// The lines carry times (LRC).
    pub synced: bool,
    pub lines: Vec<LyricLine>,
}

/// The gateway's answer, checked; `None` if it isn't lyrics or has no words.
pub fn parse_lyrics(v: &Value) -> Option<TrackLyrics> {
    let from_plugin = match v.get("source")?.as_str()? {
        "tags" => false,
        "plugin" => true,
        _ => return None,
    };
    let synced = v.get("synced")?.as_bool()?;
    let mut lines: Vec<LyricLine> = v
        .get("lines")?
        .as_array()?
        .iter()
        .take(MAX_LINES)
        .filter_map(|l| {
            let text = l.get("text")?.as_str()?;
            let time_ms = if synced {
                l.get("t")
                    .and_then(Value::as_u64)
                    .and_then(|t| u32::try_from(t).ok())
                    .unwrap_or(0)
            } else {
                0
            };
            let text: String = text
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_LINE_CHARS)
                .collect();
            Some(LyricLine {
                time_ms,
                text: text.trim().to_string(),
            })
        })
        .collect();
    if synced {
        lines.sort_by_key(|l| l.time_ms);
    }
    if lines.iter().all(|l| l.text.is_empty()) {
        return None;
    }
    Some(TrackLyrics {
        from_plugin,
        synced,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_the_gateway_answer() {
        let l = parse_lyrics(&json!({"source": "tags", "synced": true, "lines": [
            {"t": 12500, "text": "Second"}, {"t": 1000, "text": "First"}, {"t": 20000, "text": ""}
        ]}))
        .unwrap();
        assert!(!l.from_plugin && l.synced);
        let got: Vec<(u32, &str)> = l
            .lines
            .iter()
            .map(|x| (x.time_ms, x.text.as_str()))
            .collect();
        assert_eq!(got, vec![(1000, "First"), (12500, "Second"), (20000, "")]);
        let p = parse_lyrics(
            &json!({"source": "plugin", "synced": false, "lines": [{"t": 99, "text": "a"}]}),
        )
        .unwrap();
        assert!(p.from_plugin && !p.synced);
        assert_eq!(p.lines[0].time_ms, 0);
    }

    #[test]
    fn refuses_anything_else() {
        for v in [
            json!(null),
            json!("text"),
            json!([]),
            json!({}),
            json!({"source": "web", "synced": true, "lines": [{"t": 0, "text": "a"}]}),
            json!({"source": "tags", "synced": "yes", "lines": [{"t": 0, "text": "a"}]}),
            json!({"source": "tags", "synced": true, "lines": "a"}),
            json!({"source": "tags", "synced": true, "lines": []}),
            json!({"source": "tags", "synced": true, "lines": [{"t": 0, "text": "  \u{0}"}]}),
            json!({"source": "tags", "synced": true, "lines": [{"t": 0, "text": 5}, null, "x"]}),
        ] {
            assert_eq!(parse_lyrics(&v), None, "{v}");
        }
    }

    #[test]
    fn hostile_lines_are_bounded_and_cleaned() {
        let long = "é".repeat(MAX_LINE_CHARS * 3);
        let l = parse_lyrics(&json!({"source": "tags", "synced": true, "lines": [
            {"t": -5, "text": "negative"}, {"t": 1.5, "text": "fraction"},
            {"t": u64::MAX, "text": "huge"}, {"t": 50, "text": long},
            {"t": 10, "text": "a\u{1b}[31mb\u{0}"}
        ]}))
        .unwrap();
        let times: Vec<u32> = l.lines.iter().map(|x| x.time_ms).collect();
        assert_eq!(times, vec![0, 0, 0, 10, 50]);
        assert_eq!(l.lines[3].text, "a[31mb");
        assert_eq!(l.lines[4].text.chars().count(), MAX_LINE_CHARS);
        let many: Vec<Value> = (0..MAX_LINES * 2)
            .map(|_| json!({"t": 1, "text": "y"}))
            .collect();
        let l = parse_lyrics(&json!({"source": "tags", "synced": false, "lines": many})).unwrap();
        assert_eq!(l.lines.len(), MAX_LINES);
    }
}
