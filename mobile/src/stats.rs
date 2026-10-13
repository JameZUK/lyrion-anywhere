//! Listening stats (docs/plans/wave3-design.md §1; `GET /v1/me/stats`, see
//! docs/PROTOCOL.md "Listening stats"), typed for the apps: [`crate::Me::stats`].
//!
//! - **Asking:** the period and offset are checked here as the gateway checks them
//!   ([`period_problem`], [`stats_path`]) before anything is sent.
//! - **Reading** ([`parse_stats`]): strictly. `by_hour` must have 24 entries and
//!   `by_weekday` 7, or the whole answer is refused as malformed. Within lists, anything
//!   malformed is left out (an entry with a bad id, a date that isn't `YYYY-MM-DD`), never
//!   guessed. Names are cleaned (control and invisible formatting characters removed, at
//!   most 200 characters) and lists hold at most 20.

use serde_json::Value;

use crate::error::ApiError;
use crate::model::clean_text;

/// The largest offset from UTC, in minutes (UTC+14).
pub const MAX_TZ_MINUTES: i32 = 840;
/// Longest list read from an answer.
pub const MAX_LIST: usize = 20;
/// Longest artist name kept, in characters.
pub const MAX_NAME: usize = 200;

/// The stretch of history asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StatsPeriod {
    /// Today and the days before it, `days` in all (7–400).
    Days { days: u32 },
    /// One calendar month (year 1970–2100, month 1–12).
    Month { year: u32, month: u32 },
    /// One calendar year (1970–2100).
    Year { year: u32 },
    /// Everything the history keeps.
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsTotals {
    pub plays: u64,
    pub seconds: u64,
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    /// Days with at least one play.
    pub days: u64,
}

/// An album or track (by library id: load names and artwork as for shelves).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsItem {
    pub id: i64,
    pub plays: u64,
    pub seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsArtist {
    pub id: i64,
    /// Empty when the server couldn't name the artist.
    pub name: String,
    pub plays: u64,
    pub seconds: u64,
}

/// An artist first played in the period.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsNewArtist {
    pub id: i64,
    pub name: String,
    /// Unix seconds.
    pub first_played: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsMonth {
    /// `YYYY-MM`.
    pub month: String,
    pub plays: u64,
    pub seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsStreak {
    pub days: u64,
    /// `YYYY-MM-DD`, local to the offset asked with.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatsDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub seconds: u64,
}

/// The user's listening over a period.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListeningStats {
    /// Is history being recorded? Off: everything is zero or empty.
    pub enabled: bool,
    pub retention_days: u32,
    /// The period covered, Unix seconds: plays with `from <= played_at < to`.
    pub from: i64,
    pub to: i64,
    pub totals: StatsTotals,
    pub top_artists: Vec<StatsArtist>,
    pub top_albums: Vec<StatsItem>,
    pub top_tracks: Vec<StatsItem>,
    pub new_artists: Vec<StatsNewArtist>,
    /// Plays per local hour, 24 entries.
    pub by_hour: Vec<u64>,
    /// Plays per weekday, 7 entries, Monday first.
    pub by_weekday: Vec<u64>,
    /// Month by month, for a year, everything, or more than 60 days; otherwise empty.
    pub by_month: Vec<StatsMonth>,
    pub longest_streak: Option<StatsStreak>,
    pub busiest_day: Option<StatsDay>,
}

/// Why this period or offset would be refused, if it would.
pub fn period_problem(period: StatsPeriod, tz_minutes: i32) -> Option<String> {
    let year_ok = |y: u32| (1970..=2100).contains(&y);
    let bad = match period {
        StatsPeriod::Days { days } => !(7..=400).contains(&days),
        StatsPeriod::Month { year, month } => !year_ok(year) || !(1..=12).contains(&month),
        StatsPeriod::Year { year } => !year_ok(year),
        StatsPeriod::All => false,
    };
    if bad {
        return Some("bad period".into());
    }
    (!(-MAX_TZ_MINUTES..=MAX_TZ_MINUTES).contains(&tz_minutes)).then(|| "bad offset".into())
}

/// The request path for this period and offset (checked first).
pub fn stats_path(period: StatsPeriod, tz_minutes: i32) -> Result<String, ApiError> {
    if let Some(p) = period_problem(period, tz_minutes) {
        return Err(ApiError::invalid(p));
    }
    let p = match period {
        StatsPeriod::Days { days } => format!("days-{days}"),
        StatsPeriod::Month { year, month } => format!("month-{year:04}-{month:02}"),
        StatsPeriod::Year { year } => format!("year-{year:04}"),
        StatsPeriod::All => "all".into(),
    };
    Ok(format!("v1/me/stats?period={p}&tz={tz_minutes}"))
}

fn pos_id(v: &Value) -> Option<i64> {
    v.as_i64().filter(|n| (1..=999_999_999_999).contains(n))
}

fn count(v: Option<&Value>) -> Option<u64> {
    v?.as_u64()
}

/// Bidirectional and invisible formatting characters.
fn is_format(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

fn name(v: Option<&Value>) -> String {
    let raw = v.and_then(Value::as_str).unwrap_or("");
    clean_text(raw)
        .chars()
        .filter(|c| !is_format(*c))
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_string()
}

/// `YYYY-MM-DD` (or `YYYY-MM` with `day: false`), digits in the right places.
fn date_ok(s: &str, day: bool) -> bool {
    let b = s.as_bytes();
    let len = if day { 10 } else { 7 };
    b.len() == len
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

fn date(v: Option<&Value>, day: bool) -> Option<String> {
    v?.as_str().filter(|s| date_ok(s, day)).map(str::to_string)
}

fn list<T>(v: &Value, key: &str, f: impl Fn(&Value) -> Option<T>) -> Vec<T> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(f).take(MAX_LIST).collect())
        .unwrap_or_default()
}

fn buckets(v: &Value, key: &str, len: usize) -> Result<Vec<u64>, ApiError> {
    let l = v
        .get(key)
        .and_then(Value::as_array)
        .filter(|l| l.len() == len)
        .ok_or_else(|| ApiError::bad_response(format!("bad {key}")))?;
    l.iter()
        .map(|n| n.as_u64())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| ApiError::bad_response(format!("bad {key}")))
}

fn item(e: &Value) -> Option<StatsItem> {
    Some(StatsItem {
        id: pos_id(e.get("id")?)?,
        plays: count(e.get("plays"))?,
        seconds: count(e.get("seconds"))?,
    })
}

/// Read the gateway's answer.
pub fn parse_stats(v: &Value) -> Result<ListeningStats, ApiError> {
    let totals = v
        .get("totals")
        .filter(|t| t.is_object())
        .ok_or_else(|| ApiError::bad_response("no totals"))?;
    let total = |k: &str| count(totals.get(k)).unwrap_or(0);
    let period = v.get("period");
    let at = |k: &str| period.and_then(|p| p.get(k)).and_then(Value::as_i64);
    let (Some(from), Some(to)) = (at("from"), at("to")) else {
        return Err(ApiError::bad_response("no period"));
    };
    Ok(ListeningStats {
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        retention_days: v
            .get("retention_days")
            .and_then(Value::as_u64)
            .and_then(|d| u32::try_from(d).ok())
            .unwrap_or(396),
        from,
        to,
        totals: StatsTotals {
            plays: total("plays"),
            seconds: total("seconds"),
            tracks: total("tracks"),
            albums: total("albums"),
            artists: total("artists"),
            days: total("days"),
        },
        top_artists: list(v, "top_artists", |e| {
            Some(StatsArtist {
                id: pos_id(e.get("id")?)?,
                name: name(e.get("name")),
                plays: count(e.get("plays"))?,
                seconds: count(e.get("seconds"))?,
            })
        }),
        top_albums: list(v, "top_albums", item),
        top_tracks: list(v, "top_tracks", item),
        new_artists: list(v, "new_artists", |e| {
            Some(StatsNewArtist {
                id: pos_id(e.get("id")?)?,
                name: name(e.get("name")),
                first_played: e.get("first_played")?.as_i64().filter(|t| *t >= 0)?,
            })
        }),
        by_hour: buckets(v, "by_hour", 24)?,
        by_weekday: buckets(v, "by_weekday", 7)?,
        // A year by month or the whole history: a few dozen at most.
        by_month: v
            .get("by_month")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|e| {
                        Some(StatsMonth {
                            month: date(e.get("month"), false)?,
                            plays: count(e.get("plays"))?,
                            seconds: count(e.get("seconds"))?,
                        })
                    })
                    .take(64)
                    .collect()
            })
            .unwrap_or_default(),
        longest_streak: v.get("longest_streak").and_then(|k| {
            Some(StatsStreak {
                days: count(k.get("days"))?,
                from: date(k.get("from"), true)?,
                to: date(k.get("to"), true)?,
            })
        }),
        busiest_day: v.get("busiest_day").and_then(|d| {
            Some(StatsDay {
                date: date(d.get("date"), true)?,
                seconds: count(d.get("seconds"))?,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer() -> Value {
        json!({
            "enabled": true,
            "retention_days": 396,
            "period": {"from": 1_790_000_000, "to": 1_792_600_000},
            "totals": {"plays": 412, "seconds": 82800, "tracks": 301, "albums": 64, "artists": 41, "days": 27},
            "top_artists": [{"id": 12, "name": "An Example Band", "plays": 40, "seconds": 9100}],
            "top_albums": [{"id": 7, "plays": 22, "seconds": 5200}],
            "top_tracks": [{"id": 9, "plays": 6, "seconds": 1300}],
            "new_artists": [{"id": 31, "name": "Another Example", "first_played": 1_791_000_000}],
            "by_hour": vec![1; 24],
            "by_weekday": vec![2; 7],
            "by_month": [{"month": "2026-10", "plays": 412, "seconds": 82800}],
            "longest_streak": {"days": 9, "from": "2026-10-02", "to": "2026-10-10"},
            "busiest_day": {"date": "2026-10-04", "seconds": 14400}
        })
    }

    #[test]
    fn the_contract_example_reads_whole() {
        let s = parse_stats(&answer()).unwrap();
        assert!(s.enabled);
        assert_eq!((s.from, s.to), (1_790_000_000, 1_792_600_000));
        assert_eq!(s.totals.plays, 412);
        assert_eq!(s.totals.days, 27);
        assert_eq!(s.top_artists[0].name, "An Example Band");
        assert_eq!(s.top_albums[0].id, 7);
        assert_eq!(s.top_tracks[0].seconds, 1300);
        assert_eq!(s.new_artists[0].first_played, 1_791_000_000);
        assert_eq!((s.by_hour.len(), s.by_weekday.len()), (24, 7));
        assert_eq!(s.by_month[0].month, "2026-10");
        assert_eq!(s.longest_streak.unwrap().days, 9);
        assert_eq!(s.busiest_day.unwrap().date, "2026-10-04");
    }

    #[test]
    fn malformed_answers_are_refused_or_trimmed_never_guessed() {
        for (k, bad) in [
            ("by_hour", json!(vec![1; 23])),
            ("by_hour", json!(vec![-1; 24])),
            ("by_weekday", json!(vec![1; 8])),
            ("by_weekday", json!("x")),
            ("totals", json!(null)),
            ("period", json!({"from": "x", "to": 1})),
        ] {
            let mut v = answer();
            v[k] = bad;
            assert!(parse_stats(&v).is_err(), "{k}");
        }
        let mut v = answer();
        v["top_artists"] = json!([
            {"id": 0, "name": "Zero", "plays": 1, "seconds": 1},
            {"id": -4, "name": "Negative", "plays": 1, "seconds": 1},
            {"id": 5, "name": "Evil\u{202e}Name\u{0007}", "plays": 1, "seconds": 1},
            {"id": 6, "plays": 1, "seconds": 1},
            {"id": 7, "name": "x", "plays": -1, "seconds": 1}
        ]);
        v["top_tracks"] = json!(
            (1..=50)
                .map(|i| json!({"id": i, "plays": 1, "seconds": 1}))
                .collect::<Vec<_>>()
        );
        v["longest_streak"] = json!({"days": 2, "from": "2026/10/02", "to": "2026-10-03"});
        v["busiest_day"] = json!(null);
        v["by_month"] = json!([{"month": "2026-1", "plays": 1, "seconds": 1}, {"month": "2026-11", "plays": 1, "seconds": 2}]);
        let s = parse_stats(&v).unwrap();
        assert_eq!(
            s.top_artists
                .iter()
                .map(|a| (a.id, a.name.as_str()))
                .collect::<Vec<_>>(),
            [(5, "EvilName"), (6, "")]
        );
        assert_eq!(s.top_tracks.len(), MAX_LIST);
        assert!(s.longest_streak.is_none() && s.busiest_day.is_none());
        assert_eq!(s.by_month.len(), 1);
        let long = parse_stats(&{
            let mut v = answer();
            v["top_artists"][0]["name"] = json!("é".repeat(1000));
            v
        })
        .unwrap();
        assert_eq!(long.top_artists[0].name.chars().count(), MAX_NAME);
    }

    #[test]
    fn requests_are_checked_as_the_gateway_checks_them() {
        assert_eq!(
            stats_path(StatsPeriod::Days { days: 30 }, 60).unwrap(),
            "v1/me/stats?period=days-30&tz=60"
        );
        assert_eq!(
            stats_path(
                StatsPeriod::Month {
                    year: 2026,
                    month: 3
                },
                -300
            )
            .unwrap(),
            "v1/me/stats?period=month-2026-03&tz=-300"
        );
        assert_eq!(
            stats_path(StatsPeriod::Year { year: 2025 }, 0).unwrap(),
            "v1/me/stats?period=year-2025&tz=0"
        );
        assert_eq!(
            stats_path(StatsPeriod::All, 840).unwrap(),
            "v1/me/stats?period=all&tz=840"
        );
        for (p, tz) in [
            (StatsPeriod::Days { days: 6 }, 0),
            (StatsPeriod::Days { days: 401 }, 0),
            (
                StatsPeriod::Month {
                    year: 2026,
                    month: 0,
                },
                0,
            ),
            (
                StatsPeriod::Month {
                    year: 2026,
                    month: 13,
                },
                0,
            ),
            (
                StatsPeriod::Month {
                    year: 1969,
                    month: 1,
                },
                0,
            ),
            (StatsPeriod::Year { year: 2101 }, 0),
            (StatsPeriod::Year { year: 0 }, 0),
            (StatsPeriod::All, 841),
            (StatsPeriod::All, -841),
            (StatsPeriod::All, i32::MIN),
        ] {
            assert!(
                matches!(stats_path(p, tz), Err(ApiError::InvalidInput { .. })),
                "{p:?} {tz}"
            );
            assert!(period_problem(p, tz).is_some());
        }
    }
}
