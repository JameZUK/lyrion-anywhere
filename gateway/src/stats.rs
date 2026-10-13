//! Listening stats (docs/plans/wave3-design.md §1): `GET /v1/me/stats?period=&tz=`.
//!
//! - **Who:** any signed-in user, within their `/v1/me/…` allowance ([`history::caller`]),
//!   about their own listening history only. Nothing new is stored: everything is worked
//!   out from the history rows that already exist ([`crate::history`]), with their
//!   retention, opt-out and clearing.
//! - **Request:** `period` (`days-N` with N from 7 to 400, `month-YYYY-MM`, `year-YYYY` with
//!   the year from 1970 to 2100, or `all`; `days-30` by default) and `tz` (minutes east of
//!   UTC, −840 to 840, 0 by default), each at most once, written plainly ([`parse_query`]).
//!   Anything else is 400.
//! - **The period** ([`Period::range`]), in the caller's fixed offset: `days-N` is today and
//!   the N − 1 days before it, from local midnight until now; a month or a year runs from
//!   local midnight on its first day to its end (or now, if sooner); `all` is the whole
//!   retention period until now. Plays count when `from <= played_at < to`.
//! - **Counting** ([`compute`]): at most [`MAX_ROWS`] rows (the newest). Tracks are counted
//!   by the history's stable track key (library ids can change after a rescan), reporting
//!   the newest track id seen for it; albums and artists by their ids. Top lists are by
//!   plays, then seconds, then the latest play, then the lower id, [`TOP`] at most. An
//!   artist is new when their first play in the whole kept history falls in the period.
//!   Hours, weekdays (Monday first), months and days are local to `tz`. The longest streak
//!   and the busiest day prefer the most recent on a tie.
//! - **Artist names:** LMS looks an artist up by one id only, so each name the answer needs
//!   (top and new artists, at most 2 × [`TOP`]) is asked of LMS with exactly
//!   `artists 0 1 artist_id:<id>`, [`LOOKUPS_AT_ONCE`] at a time, each within
//!   [`crate::Config::stats_lookup_timeout`] and all within twice that. Names are cleaned
//!   ([`clean_name`]: control and formatting characters removed, whitespace collapsed, at
//!   most [`MAX_NAME`] characters, and nothing naming a server path or a LAN address) and
//!   remembered for [`NAME_TTL`] ([`NAMES`] at most; a failed lookup for [`STALL_TTL`]).
//!   A name LMS can't give is `""`.
//! - **Caching:** a whole answer is remembered per user, period and offset for
//!   [`ANSWER_TTL`] ([`ANSWERS`] at most), and dropped when the user clears their history,
//!   switches it on or off, or is deleted. An answer missing a name because LMS failed is
//!   not remembered.
//! - **History off:** `enabled: false`, with zeros and empty lists.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::AppState;
use crate::history::{self, Entry, caller, error, ok};
use crate::{info, rpc_policy};

/// History rows read for one answer, at most (the newest).
pub const MAX_ROWS: usize = 200_000;
/// Longest top list (and list of new artists).
pub const TOP: usize = 20;
/// How long a whole answer is remembered.
pub const ANSWER_TTL: Duration = Duration::from_secs(5 * 60);
/// Answers remembered, at most.
pub const ANSWERS: usize = 1024;
/// How long an artist's name (or LMS not knowing it) is remembered.
pub const NAME_TTL: Duration = Duration::from_secs(60 * 60);
/// How long a failed name lookup is remembered (so a stalled LMS doesn't hold every visit up).
pub const STALL_TTL: Duration = Duration::from_secs(2 * 60);
/// Names remembered, at most.
pub const NAMES: usize = 4096;
/// Name lookups running at once for one answer.
pub const LOOKUPS_AT_ONCE: usize = 4;
/// Default time limit for one name lookup.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);
/// Longest artist name, in characters.
pub const MAX_NAME: usize = 200;
/// The largest offset from UTC, in minutes (UTC+14).
pub const MAX_TZ: i32 = 840;
/// Longest query string accepted.
const MAX_QUERY: usize = 64;
/// Bytes of an LMS name looked at, at most.
const MAX_NAME_INPUT: usize = 4096;
const DAY: i64 = 86_400;

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

/// The stretch of history asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    /// Today and the days before it: N days in all.
    Days(u32),
    Month {
        year: i32,
        month: u32,
    },
    Year(i32),
    /// Everything the history keeps.
    All,
}

/// Plain decimal digits, `len` of them exactly.
fn fixed_digits(s: &str, len: usize) -> Option<u32> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

fn year_ok(y: u32) -> Option<i32> {
    (1970..=2100).contains(&y).then_some(y as i32)
}

impl Period {
    /// `days-N` (7–400, no leading zero), `month-YYYY-MM`, `year-YYYY` (1970–2100) or `all`.
    pub fn parse(s: &str) -> Option<Period> {
        if s == "all" {
            return Some(Period::All);
        }
        if let Some(n) = s.strip_prefix("days-") {
            if n.starts_with('0') {
                return None;
            }
            let n = (1..=3).find_map(|len| fixed_digits(n, len))?;
            return (7..=400).contains(&n).then_some(Period::Days(n));
        }
        if let Some(ym) = s.strip_prefix("month-") {
            let (y, m) = ym.split_once('-')?;
            let year = year_ok(fixed_digits(y, 4)?)?;
            let month = fixed_digits(m, 2)?;
            return (1..=12)
                .contains(&month)
                .then_some(Period::Month { year, month });
        }
        if let Some(y) = s.strip_prefix("year-") {
            return Some(Period::Year(year_ok(fixed_digits(y, 4)?)?));
        }
        None
    }

    /// Does the answer have a month-by-month list?
    pub fn monthly(self) -> bool {
        match self {
            Period::Days(n) => n > 60,
            Period::Month { .. } => false,
            Period::Year(_) | Period::All => true,
        }
    }

    /// `(from, to)` in Unix seconds for this period seen from `now` at `tz` minutes east of
    /// UTC: plays with `from <= played_at < to`. Never `to < from`.
    pub fn range(self, now: i64, tz: i32) -> (i64, i64) {
        let today = local_day(now, tz);
        let (from, end) = match self {
            Period::Days(n) => (day_start(today - (n as i64 - 1), tz), now),
            Period::Month { year, month } => {
                let (ny, nm) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
                (
                    day_start(days_from_civil(year as i64, month, 1), tz),
                    day_start(days_from_civil(ny as i64, nm, 1), tz),
                )
            }
            Period::Year(y) => (
                day_start(days_from_civil(y as i64, 1, 1), tz),
                day_start(days_from_civil(y as i64 + 1, 1, 1), tz),
            ),
            Period::All => (now - history::RETENTION.as_secs() as i64, now),
        };
        (from, end.min(now).max(from))
    }
}

/// `tz`: minutes east of UTC, −840 to 840, plain decimal (no `+`, no leading zero, no `-0`).
pub fn parse_tz(s: &str) -> Option<i32> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s),
    };
    if digits.is_empty()
        || digits.len() > 3
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.starts_with('0') && (digits.len() > 1 || neg))
    {
        return None;
    }
    let n: i32 = digits.parse().ok()?;
    let n = if neg { -n } else { n };
    (-MAX_TZ..=MAX_TZ).contains(&n).then_some(n)
}

/// The query string: `period` and `tz`, each at most once, and nothing else.
pub fn parse_query(raw: Option<&str>) -> Option<(Period, i32)> {
    let mut period = None;
    let mut tz = None;
    let raw = raw.unwrap_or("");
    if raw.len() > MAX_QUERY {
        return None;
    }
    if !raw.is_empty() {
        for part in raw.split('&') {
            let (k, v) = part.split_once('=')?;
            match k {
                "period" if period.is_none() => period = Some(Period::parse(v)?),
                "tz" if tz.is_none() => tz = Some(parse_tz(v)?),
                _ => return None,
            }
        }
    }
    Some((period.unwrap_or(Period::Days(30)), tz.unwrap_or(0)))
}

// ---------------------------------------------------------------------------
// Calendar maths (fixed offsets, no daylight saving)
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `(year, month, day)` of a day number from [`days_from_civil`].
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// The local day number of an instant.
pub fn local_day(t: i64, tz: i32) -> i64 {
    t.saturating_add(tz as i64 * 60).div_euclid(DAY)
}

/// When local day `day` starts, in Unix seconds.
pub fn day_start(day: i64, tz: i32) -> i64 {
    day.saturating_mul(DAY) - tz as i64 * 60
}

/// The local hour (0–23) of an instant.
pub fn local_hour(t: i64, tz: i32) -> usize {
    (t.saturating_add(tz as i64 * 60).rem_euclid(DAY) / 3600) as usize
}

/// The weekday of a day number, Monday 0 to Sunday 6 (1970-01-01 was a Thursday).
pub fn weekday(day: i64) -> usize {
    (day + 3).rem_euclid(7) as usize
}

/// `YYYY-MM-DD`.
pub fn date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

// ---------------------------------------------------------------------------
// Counting (pure)
// ---------------------------------------------------------------------------

/// Plays and time for one track, album or artist.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub id: i64,
    pub plays: u64,
    pub seconds: f64,
    /// The latest play (`played_at`, row id).
    pub last: (i64, i64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewArtist {
    pub id: i64,
    pub first_played: i64,
    pub plays: u64,
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Month {
    /// `YYYY-MM`.
    pub month: String,
    pub plays: u64,
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Streak {
    pub days: u64,
    pub from: i64,
    pub to: i64,
}

/// Everything in an answer but the artists' names.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stats {
    pub from: i64,
    pub to: i64,
    pub plays: u64,
    pub seconds: f64,
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    pub days: u64,
    pub top_artists: Vec<Ranked>,
    pub top_albums: Vec<Ranked>,
    pub top_tracks: Vec<Ranked>,
    pub new_artists: Vec<NewArtist>,
    pub by_hour: [u64; 24],
    pub by_weekday: [u64; 7],
    pub by_month: Vec<Month>,
    pub longest_streak: Option<Streak>,
    /// (local day, seconds).
    pub busiest_day: Option<(i64, f64)>,
}

impl Stats {
    /// The artists whose names the answer needs: top artists first, then new ones.
    pub fn artist_ids(&self) -> Vec<i64> {
        let mut seen = HashSet::new();
        self.top_artists
            .iter()
            .map(|a| a.id)
            .chain(self.new_artists.iter().map(|a| a.id))
            .filter(|id| seen.insert(*id))
            .collect()
    }
}

fn seconds_of(e: &Entry) -> f64 {
    if e.seconds.is_finite() {
        e.seconds.clamp(0.0, crate::plays::MAX_SECONDS)
    } else {
        0.0
    }
}

fn tally<K: std::hash::Hash + Eq>(m: &mut HashMap<K, Ranked>, key: K, id: i64, e: &Entry) {
    let at = (e.played_at, e.id);
    let r = m.entry(key).or_insert(Ranked {
        id,
        plays: 0,
        seconds: 0.0,
        last: at,
    });
    r.plays += 1;
    r.seconds += seconds_of(e);
    if at >= r.last {
        r.last = at;
        // The newest id seen for a track (ids can change after a rescan).
        r.id = id;
    }
}

/// Most plays first, then most time, then the latest play, then the lower id.
fn rank(m: HashMap<impl std::hash::Hash + Eq, Ranked>) -> Vec<Ranked> {
    let mut v: Vec<Ranked> = m.into_values().collect();
    v.sort_by(|a, b| {
        b.plays
            .cmp(&a.plays)
            .then(b.seconds.total_cmp(&a.seconds))
            .then(b.last.cmp(&a.last))
            .then(a.id.cmp(&b.id))
    });
    v.truncate(TOP);
    v
}

/// The stats of `rows` (the user's plays in `from..to`) at `tz` minutes east of UTC.
/// `first_plays`: when the user first played each artist, over the whole history.
pub fn compute(
    rows: &[Entry],
    first_plays: &HashMap<i64, i64>,
    period: Period,
    from: i64,
    to: i64,
    tz: i32,
) -> Stats {
    let mut s = Stats {
        from,
        to,
        ..Stats::default()
    };
    let mut tracks: HashMap<&str, Ranked> = HashMap::new();
    let mut albums: HashMap<i64, Ranked> = HashMap::new();
    let mut artists: HashMap<i64, Ranked> = HashMap::new();
    let mut days: HashMap<i64, f64> = HashMap::new();
    let mut months: HashMap<(i64, u32), (u64, f64)> = HashMap::new();
    for e in rows.iter().filter(|e| (from..to).contains(&e.played_at)) {
        let secs = seconds_of(e);
        s.plays += 1;
        s.seconds += secs;
        tally(&mut tracks, e.track_key.as_str(), e.track_id, e);
        if let Some(a) = e.album_id.filter(|a| *a > 0) {
            tally(&mut albums, a, a, e);
        }
        if let Some(a) = e.artist_id.filter(|a| *a > 0) {
            tally(&mut artists, a, a, e);
        }
        let day = local_day(e.played_at, tz);
        *days.entry(day).or_insert(0.0) += secs;
        s.by_hour[local_hour(e.played_at, tz)] += 1;
        s.by_weekday[weekday(day)] += 1;
        let (y, m, _) = civil_from_days(day);
        let mo = months.entry((y, m)).or_insert((0, 0.0));
        mo.0 += 1;
        mo.1 += secs;
    }
    s.tracks = tracks.len() as u64;
    s.albums = albums.len() as u64;
    s.artists = artists.len() as u64;
    s.days = days.len() as u64;

    let mut new: Vec<NewArtist> = artists
        .values()
        .filter_map(|a| {
            let first = *first_plays.get(&a.id)?;
            (from..to).contains(&first).then_some(NewArtist {
                id: a.id,
                first_played: first,
                plays: a.plays,
                seconds: a.seconds,
            })
        })
        .collect();
    // The biggest discoveries first.
    new.sort_by(|a, b| {
        b.plays
            .cmp(&a.plays)
            .then(b.seconds.total_cmp(&a.seconds))
            .then(a.first_played.cmp(&b.first_played))
            .then(a.id.cmp(&b.id))
    });
    new.truncate(TOP);
    s.new_artists = new;
    s.top_tracks = rank(tracks);
    s.top_albums = rank(albums);
    s.top_artists = rank(artists);

    if period.monthly() && to > from {
        let (mut y, mut m, _) = civil_from_days(local_day(from, tz));
        let (ly, lm, _) = civil_from_days(local_day(to - 1, tz));
        // At most 400 days, or a year, or the retention period: a few dozen months.
        for _ in 0..64 {
            let (plays, seconds) = months.get(&(y, m)).copied().unwrap_or((0, 0.0));
            s.by_month.push(Month {
                month: format!("{y:04}-{m:02}"),
                plays,
                seconds,
            });
            if (y, m) >= (ly, lm) {
                break;
            }
            (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        }
    }

    let mut ordered: Vec<i64> = days.keys().copied().collect();
    ordered.sort_unstable();
    let mut run_start = 0;
    for (i, d) in ordered.iter().enumerate() {
        if i > 0 && *d != ordered[i - 1] + 1 {
            run_start = i;
        }
        let len = (i - run_start + 1) as u64;
        // `>=`: the most recent of equal streaks.
        if s.longest_streak.as_ref().is_none_or(|b| len >= b.days) {
            s.longest_streak = Some(Streak {
                days: len,
                from: ordered[run_start],
                to: *d,
            });
        }
    }
    for d in &ordered {
        let secs = days[d];
        if s.busiest_day.is_none_or(|(_, b)| secs >= b) {
            s.busiest_day = Some((*d, secs));
        }
    }
    s
}

fn whole(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        seconds.round() as u64
    } else {
        0
    }
}

/// The answer, with artists' names from `names` (`""` when unknown).
pub fn to_json(s: &Stats, enabled: bool, names: &HashMap<i64, String>) -> Value {
    let name = |id: i64| names.get(&id).cloned().unwrap_or_default();
    let item = |r: &Ranked| json!({"id": r.id, "plays": r.plays, "seconds": whole(r.seconds)});
    json!({
        "enabled": enabled,
        "retention_days": history::RETENTION.as_secs() / 86_400,
        "period": {"from": s.from, "to": s.to},
        "totals": {
            "plays": s.plays,
            "seconds": whole(s.seconds),
            "tracks": s.tracks,
            "albums": s.albums,
            "artists": s.artists,
            "days": s.days,
        },
        "top_artists": s.top_artists.iter().map(|r| json!({
            "id": r.id, "name": name(r.id), "plays": r.plays, "seconds": whole(r.seconds),
        })).collect::<Vec<_>>(),
        "top_albums": s.top_albums.iter().map(item).collect::<Vec<_>>(),
        "top_tracks": s.top_tracks.iter().map(item).collect::<Vec<_>>(),
        "new_artists": s.new_artists.iter().map(|a| json!({
            "id": a.id, "name": name(a.id), "first_played": a.first_played,
        })).collect::<Vec<_>>(),
        "by_hour": s.by_hour.to_vec(),
        "by_weekday": s.by_weekday.to_vec(),
        "by_month": s.by_month.iter().map(|m| json!({
            "month": m.month, "plays": m.plays, "seconds": whole(m.seconds),
        })).collect::<Vec<_>>(),
        "longest_streak": s.longest_streak.as_ref().map(|k| json!({
            "days": k.days, "from": date(k.from), "to": date(k.to),
        })),
        "busiest_day": s.busiest_day.map(|(d, secs)| json!({
            "date": date(d), "seconds": whole(secs),
        })),
    })
}

// ---------------------------------------------------------------------------
// Artist names
// ---------------------------------------------------------------------------

/// An artist's name from LMS made safe to show: control and invisible formatting
/// characters removed, whitespace collapsed, at most [`MAX_NAME`] characters, and `None`
/// if nothing is left or it names a server path or a LAN address.
pub fn clean_name(raw: &str) -> Option<String> {
    let mut cut = raw.len().min(MAX_NAME_INPUT);
    while !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    let spaced: String = raw[..cut]
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !info::formatting(*c))
        .collect();
    let name: String = spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME)
        .collect();
    let name = name.trim_end().to_string();
    (!name.is_empty() && !rpc_policy::sensitive_text(&name)).then_some(name)
}

/// The name in LMS's answer to `artists 0 1 artist_id:<id>`, if it is that artist's.
pub fn name_in(v: &Value, id: i64) -> Option<String> {
    let a = v.get("artists_loop")?.as_array()?.first()?;
    let got = match a.get("id")? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    };
    (got == Some(id))
        .then(|| clean_name(a.get("artist")?.as_str()?))
        .flatten()
}

#[derive(Clone)]
enum Known {
    /// The name, or `None`: LMS has no such artist (or nothing showable).
    Name(Option<String>),
    /// The lookup failed or timed out.
    Stalled,
}

struct NameEntry {
    at: Instant,
    ttl: Duration,
    known: Known,
}

type AnswerKey = (i64, String, Period, i32);

#[derive(Default)]
struct Answers {
    map: HashMap<AnswerKey, (Instant, Value)>,
    /// Bumped whenever a user's answers are dropped, so one worked out before is never
    /// remembered after.
    epochs: HashMap<i64, u64>,
}

/// Remembered answers and artist names.
#[derive(Default)]
pub struct StatsCache {
    answers: Mutex<Answers>,
    names: Mutex<HashMap<i64, NameEntry>>,
}

impl StatsCache {
    fn answer(&self, key: &AnswerKey, now: Instant) -> Option<Value> {
        let a = self.answers.lock().unwrap();
        a.map
            .get(key)
            .filter(|(at, _)| now.duration_since(*at) < ANSWER_TTL)
            .map(|(_, v)| v.clone())
    }

    fn epoch(&self, user: i64) -> u64 {
        self.answers
            .lock()
            .unwrap()
            .epochs
            .get(&user)
            .copied()
            .unwrap_or(0)
    }

    fn put_answer(&self, key: AnswerKey, epoch: u64, v: Value, now: Instant) {
        let mut a = self.answers.lock().unwrap();
        if a.epochs.get(&key.0).copied().unwrap_or(0) != epoch {
            return;
        }
        if a.map.len() >= ANSWERS && !a.map.contains_key(&key) {
            a.map
                .retain(|_, (at, _)| now.duration_since(*at) < ANSWER_TTL);
            if a.map.len() >= ANSWERS
                && let Some(oldest) = a
                    .map
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(k, _)| k.clone())
            {
                a.map.remove(&oldest);
            }
        }
        a.map.insert(key, (now, v));
    }

    /// Drop a user's remembered answers (history cleared, switched on or off).
    pub(crate) fn forget_user(&self, user: i64) {
        let mut a = self.answers.lock().unwrap();
        a.map.retain(|k, _| k.0 != user);
        if a.epochs.len() > 100_000 {
            a.epochs.clear();
        }
        *a.epochs.entry(user).or_insert(0) += 1;
    }

    /// Drop a deleted user's remembered answers.
    pub(crate) fn forget_name(&self, username: &str) {
        let mut a = self.answers.lock().unwrap();
        let users: Vec<i64> = a
            .map
            .keys()
            .filter(|k| k.1 == username)
            .map(|k| k.0)
            .collect();
        a.map.retain(|k, _| k.1 != username);
        for u in users {
            *a.epochs.entry(u).or_insert(0) += 1;
        }
    }

    /// How many answers are remembered (for tests).
    pub fn answers_len(&self) -> usize {
        self.answers.lock().unwrap().map.len()
    }

    fn name(&self, id: i64, now: Instant) -> Option<Known> {
        let n = self.names.lock().unwrap();
        n.get(&id)
            .filter(|e| now.duration_since(e.at) < e.ttl)
            .map(|e| e.known.clone())
    }

    fn put_name(&self, id: i64, known: Known, now: Instant) {
        let ttl = match known {
            Known::Stalled => STALL_TTL,
            Known::Name(_) => NAME_TTL,
        };
        let mut n = self.names.lock().unwrap();
        if n.len() >= NAMES && !n.contains_key(&id) {
            n.retain(|_, e| now.duration_since(e.at) < e.ttl);
            if n.len() >= NAMES
                && let Some(oldest) = n.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| *k)
            {
                n.remove(&oldest);
            }
        }
        n.insert(
            id,
            NameEntry {
                at: now,
                ttl,
                known,
            },
        );
    }
}

impl crate::Gateway {
    /// How many stats answers are remembered (for tests).
    pub fn stats_answers(&self) -> usize {
        self.stats.answers_len()
    }

    /// One artist's name from LMS: `Ok(None)` if LMS has no such artist, `Err` if it
    /// failed or took too long.
    async fn artist_name(&self, id: i64) -> Result<Option<String>, ()> {
        let cmd = [
            "artists".to_string(),
            "0".into(),
            "1".into(),
            format!("artist_id:{id}"),
        ];
        let v = self
            .lms_command_within("", &cmd, self.cfg.stats_lookup_timeout)
            .await
            .map_err(|_| ())?;
        Ok(name_in(&v, id))
    }

    /// Names for these artists (at most 2 × [`TOP`] are looked up), and whether every one
    /// was answered (by LMS or the cache) rather than failing.
    pub(crate) async fn artist_names(&self, ids: &[i64]) -> (HashMap<i64, String>, bool) {
        let now = Instant::now();
        let mut out = HashMap::new();
        let mut complete = true;
        let mut todo = Vec::new();
        for &id in ids.iter().take(2 * TOP) {
            match self.stats.name(id, now) {
                Some(Known::Name(n)) => {
                    out.insert(id, n.unwrap_or_default());
                }
                Some(Known::Stalled) => complete = false,
                None => todo.push(id),
            }
        }
        if todo.is_empty() {
            return (out, complete);
        }
        // Once one lookup has failed, LMS is likely stalled: start no more.
        let stalled = AtomicBool::new(false);
        let stalled = &stalled;
        let mut lookups = futures_util::stream::iter(todo)
            .map(|id| async move {
                if stalled.load(Ordering::Relaxed) {
                    return (id, None);
                }
                let r = self.artist_name(id).await;
                if r.is_err() {
                    stalled.store(true, Ordering::Relaxed);
                }
                (id, Some(r))
            })
            .buffer_unordered(LOOKUPS_AT_ONCE);
        let deadline = tokio::time::Instant::now() + self.cfg.stats_lookup_timeout * 2;
        loop {
            match tokio::time::timeout_at(deadline, lookups.next()).await {
                Ok(Some((id, Some(Ok(name))))) => {
                    self.stats
                        .put_name(id, Known::Name(name.clone()), Instant::now());
                    out.insert(id, name.unwrap_or_default());
                }
                Ok(Some((id, Some(Err(()))))) => {
                    self.stats.put_name(id, Known::Stalled, Instant::now());
                    complete = false;
                }
                Ok(Some((_, None))) => complete = false,
                Ok(None) => break,
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        (out, complete)
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// `GET /v1/me/stats?period=&tz=`
pub async fn get(
    State(gw): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((period, tz)) = parse_query(query.as_deref()) else {
        return error(StatusCode::BAD_REQUEST, "bad query");
    };
    let now = history::unix_now() as i64;
    let (from, to) = period.range(now, tz);
    let Ok(on) = history::enabled(gw.accounts(), p.user_id) else {
        return internal();
    };
    if !on {
        let empty = Stats {
            from,
            to,
            ..Stats::default()
        };
        return ok(to_json(&empty, false, &HashMap::new()));
    }
    let key: AnswerKey = (p.user_id, p.username.clone(), period, tz);
    if let Some(v) = gw.stats.answer(&key, Instant::now()) {
        return ok(v);
    }
    let epoch = gw.stats.epoch(p.user_id);
    let accounts = gw.accounts().clone();
    let user = p.user_id;
    let worked = tokio::task::spawn_blocking(move || {
        let rows = history::between(&accounts, user, from, to, MAX_ROWS)?;
        let firsts = history::first_plays(&accounts, user)?;
        Ok::<_, crate::accounts::AuthError>(compute(&rows, &firsts, period, from, to, tz))
    })
    .await;
    let Ok(Ok(stats)) = worked else {
        return internal();
    };
    let (names, complete) = gw.artist_names(&stats.artist_ids()).await;
    let v = to_json(&stats, true, &names);
    if complete {
        gw.stats.put_answer(key, epoch, v.clone(), Instant::now());
    }
    ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "a";

    fn row(id: i64, key: &str, track: i64, album: i64, artist: i64, at: i64, secs: f64) -> Entry {
        Entry {
            id,
            track_key: key.to_string(),
            track_id: track,
            album_id: Some(album),
            artist_id: Some(artist),
            year: None,
            played_at: at,
            seconds: secs,
            source: "live".into(),
        }
    }

    /// 2026-10-05 00:00:00 UTC, a Monday.
    const MON: i64 = 1_791_158_400;

    // -- the period parser ----------------------------------------------------

    #[test]
    fn periods_parse_strictly() {
        assert_eq!(Period::parse("all"), Some(Period::All));
        assert_eq!(Period::parse("days-7"), Some(Period::Days(7)));
        assert_eq!(Period::parse("days-30"), Some(Period::Days(30)));
        assert_eq!(Period::parse("days-400"), Some(Period::Days(400)));
        assert_eq!(
            Period::parse("month-2026-10"),
            Some(Period::Month {
                year: 2026,
                month: 10
            })
        );
        assert_eq!(
            Period::parse("month-1970-01"),
            Some(Period::Month {
                year: 1970,
                month: 1
            })
        );
        assert_eq!(Period::parse("year-2100"), Some(Period::Year(2100)));
        assert_eq!(Period::parse("year-1970"), Some(Period::Year(1970)));
        for bad in [
            "",
            "ALL",
            "all ",
            " all",
            "days-6",
            "days-401",
            "days-0",
            "days-007",
            "days-30x",
            "days--30",
            "days-+30",
            "days-",
            "days-1000",
            "days-99999999999999999999",
            "days-3 0",
            "days-٣٠",
            "month-2026-13",
            "month-2026-00",
            "month-2026-1",
            "month-2026-001",
            "month-26-10",
            "month-1969-12",
            "month-2101-01",
            "month-2026",
            "month-2026-10-01",
            "month-+026-10",
            "month-2026/10",
            "year-1969",
            "year-2101",
            "year-26",
            "year-02026",
            "year--2026",
            "year-",
            "year-２０２６",
            "week-1",
            "days-30&",
            "days%2D30",
        ] {
            assert_eq!(Period::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn offsets_parse_strictly() {
        for (s, want) in [
            ("0", 0),
            ("60", 60),
            ("-300", -300),
            ("840", 840),
            ("-840", -840),
        ] {
            assert_eq!(parse_tz(s), Some(want), "{s}");
        }
        for bad in [
            "",
            "-",
            "+60",
            "841",
            "-841",
            "1000",
            "060",
            "-0",
            "00",
            "6 0",
            "60.0",
            "1e2",
            "0x10",
            "−60",
            "9999999999",
        ] {
            assert_eq!(parse_tz(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn queries_take_period_and_tz_once_and_nothing_else() {
        assert_eq!(parse_query(None), Some((Period::Days(30), 0)));
        assert_eq!(parse_query(Some("")), Some((Period::Days(30), 0)));
        assert_eq!(
            parse_query(Some("period=year-2025&tz=-300")),
            Some((Period::Year(2025), -300))
        );
        assert_eq!(parse_query(Some("tz=60")), Some((Period::Days(30), 60)));
        for bad in [
            "period=all&period=all",
            "tz=0&tz=0",
            "period=all&",
            "&period=all",
            "period",
            "period=",
            "tz=",
            "other=1",
            "period=all&x=1",
            "Period=all",
            "period=all;tz=0",
            "period=days%2D30",
            "period=all&tz=60&tz=60",
            &format!("period=all&tz=0{}", "&".repeat(80)),
        ] {
            assert_eq!(parse_query(Some(bad)), None, "{bad:?}");
        }
    }

    /// A small deterministic generator (no extra dependencies).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn the_parsers_never_panic_on_random_input() {
        let alphabet: Vec<char> = "adyslmonthr-0123456789=&%+ \u{0}\u{202e}é٣２"
            .chars()
            .collect();
        let mut rng = Rng(0x5eed_1234_abcd_0001);
        for _ in 0..50_000 {
            let len = (rng.next() % 24) as usize;
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize])
                .collect();
            if let Some(p) = Period::parse(&s) {
                // Anything accepted is one of the documented forms, in range.
                match p {
                    Period::Days(n) => assert!((7..=400).contains(&n), "{s}"),
                    Period::Month { year, month } => {
                        assert!((1970..=2100).contains(&year) && (1..=12).contains(&month))
                    }
                    Period::Year(y) => assert!((1970..=2100).contains(&y)),
                    Period::All => assert_eq!(s, "all"),
                }
                let (from, to) = p.range(MON, 0);
                assert!(from <= to);
            }
            if let Some(tz) = parse_tz(&s) {
                assert!((-MAX_TZ..=MAX_TZ).contains(&tz), "{s}");
            }
            let _ = parse_query(Some(&s));
            // Prefixed forms too, to reach deeper.
            for prefix in ["days-", "month-20", "year-1", "period=", "tz=-"] {
                let t = format!("{prefix}{s}");
                let _ = Period::parse(&t);
                let _ = parse_tz(&t);
                let _ = parse_query(Some(&t));
            }
        }
    }

    // -- calendar maths -------------------------------------------------------

    #[test]
    fn dates_round_trip_and_known_days_are_right() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(-1), "1969-12-31");
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(date(days_from_civil(2024, 2, 29)), "2024-02-29");
        assert_eq!(MON / DAY, days_from_civil(2026, 10, 5));
        assert_eq!(weekday(MON / DAY), 0, "a Monday");
        assert_eq!(weekday(0), 3, "1970-01-01 was a Thursday");
        assert_eq!(weekday(MON / DAY + 6), 6, "Sunday last");
        for z in (-800_000..800_000).step_by(997) {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
            assert!((1..=12).contains(&m) && (1..=31).contains(&d));
        }
    }

    #[test]
    fn fixed_offsets_move_plays_across_day_boundaries() {
        // 23:30 UTC on Sunday 4 October.
        let t = MON - 1800;
        assert_eq!(
            (date(local_day(t, 0)), local_hour(t, 0)),
            ("2026-10-04".into(), 23)
        );
        // UTC+1: 00:30 on Monday.
        assert_eq!(
            (date(local_day(t, 60)), local_hour(t, 60)),
            ("2026-10-05".into(), 0)
        );
        assert_eq!(weekday(local_day(t, 60)), 0);
        // UTC−14 and UTC+14 at the extremes.
        assert_eq!(local_hour(t, -840), 9);
        assert_eq!(date(local_day(t, -840)), "2026-10-04");
        assert_eq!(local_hour(t, 840), 13);
        assert_eq!(date(local_day(t, 840)), "2026-10-05");
        // Half-hour and 45-minute offsets.
        assert_eq!(local_hour(MON, 330), 5);
        assert_eq!(local_hour(MON - 1, 345), 5);
        // Exactly at local midnight it is the new day; a second before, the old one.
        let midnight = day_start(MON / DAY, -300);
        assert_eq!(date(local_day(midnight, -300)), "2026-10-05");
        assert_eq!(date(local_day(midnight - 1, -300)), "2026-10-04");
        assert_eq!(local_hour(midnight - 1, -300), 23);
    }

    #[test]
    fn periods_cover_local_days() {
        let now = MON + 10 * 3600; // 10:00 UTC Monday
        assert_eq!(Period::Days(7).range(now, 0), (MON - 6 * DAY, now));
        assert_eq!(
            Period::Days(7).range(now, 60),
            (MON - 6 * DAY - 3600, now),
            "local midnight at UTC+1"
        );
        let (from, to) = Period::Month {
            year: 2026,
            month: 9,
        }
        .range(now, 0);
        assert_eq!(
            (date(from / DAY), date(to / DAY)),
            ("2026-09-01".into(), "2026-10-01".into())
        );
        // This month ends now; a future month is empty.
        assert_eq!(
            Period::Month {
                year: 2026,
                month: 10
            }
            .range(now, 0),
            (days_from_civil(2026, 10, 1) * DAY, now)
        );
        let (f, t) = Period::Month {
            year: 2027,
            month: 1,
        }
        .range(now, 0);
        assert_eq!(f, t);
        // December rolls into the next year.
        let (f, t) = Period::Month {
            year: 2025,
            month: 12,
        }
        .range(now, -120);
        assert_eq!(f, days_from_civil(2025, 12, 1) * DAY + 7200);
        assert_eq!(t, days_from_civil(2026, 1, 1) * DAY + 7200);
        let (f, t) = Period::Year(2025).range(now, 0);
        assert_eq!(
            (date(f / DAY), date(t / DAY)),
            ("2025-01-01".into(), "2026-01-01".into())
        );
        let (f, t) = Period::All.range(now, 0);
        assert_eq!((t - f) / DAY, 396);
    }

    // -- counting -------------------------------------------------------------

    #[test]
    fn totals_and_buckets_follow_the_offset() {
        let rows = vec![
            // Sunday 23:30 UTC = Monday 00:30 at UTC+1.
            row(1, KEY_A, 10, 1, 100, MON - 1800, 200.0),
            // Monday 12:00 UTC.
            row(2, KEY_A, 10, 1, 100, MON + 12 * 3600, 100.0),
            row(3, "b", 11, 2, 101, MON + 12 * 3600 + 60, 50.4),
        ];
        let p = Period::Days(30);
        let s = compute(&rows, &HashMap::new(), p, 0, i64::MAX, 0);
        assert_eq!(
            (s.plays, s.tracks, s.albums, s.artists, s.days),
            (3, 2, 2, 2, 2)
        );
        assert_eq!(s.by_hour[23], 1);
        assert_eq!(s.by_hour[12], 2);
        assert_eq!(s.by_weekday, [2, 0, 0, 0, 0, 0, 1]);
        let s = compute(&rows, &HashMap::new(), p, 0, i64::MAX, 60);
        assert_eq!(s.days, 1);
        assert_eq!(s.by_hour[0], 1);
        assert_eq!(s.by_hour[13], 2);
        assert_eq!(s.by_weekday, [3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(s.by_hour.iter().sum::<u64>(), 3);
        let v = to_json(&s, true, &HashMap::new());
        assert_eq!(v["totals"]["seconds"], 350);
        assert_eq!(v["by_hour"].as_array().unwrap().len(), 24);
        assert_eq!(v["by_weekday"].as_array().unwrap().len(), 7);
        assert_eq!(v["by_month"], json!([]), "no months for 30 days");
        assert_eq!(
            v["busiest_day"],
            json!({"date": "2026-10-05", "seconds": 350})
        );
    }

    #[test]
    fn rows_outside_the_period_are_not_counted() {
        let rows = vec![
            row(1, KEY_A, 10, 1, 100, 1000, 200.0),
            row(2, KEY_A, 10, 1, 100, 2000, 200.0),
            row(3, KEY_A, 10, 1, 100, 3000, 200.0),
        ];
        let s = compute(&rows, &HashMap::new(), Period::All, 2000, 3000, 0);
        assert_eq!(s.plays, 1, "from inclusive, to exclusive");
    }

    #[test]
    fn tracks_count_by_stable_key_and_report_the_newest_id() {
        // The same track renumbered by a rescan.
        let rows = vec![
            row(3, KEY_A, 77, 1, 100, 3000, 10.0),
            row(2, KEY_A, 10, 1, 100, 2000, 10.0),
            row(1, KEY_A, 10, 1, 100, 1000, 10.0),
            row(4, "b", 77, 1, 100, 500, 10.0),
        ];
        let s = compute(&rows, &HashMap::new(), Period::All, 0, 10_000, 0);
        assert_eq!(s.tracks, 2);
        assert_eq!(s.top_tracks[0].id, 77);
        assert_eq!(s.top_tracks[0].plays, 3);
        // Rows in any order give the same answer.
        let mut rev = rows.clone();
        rev.reverse();
        assert_eq!(
            compute(&rev, &HashMap::new(), Period::All, 0, 10_000, 0).top_tracks,
            s.top_tracks
        );
    }

    #[test]
    fn top_lists_break_ties_by_seconds_then_recency_then_id() {
        let rows = vec![
            row(1, "a", 1, 10, 1, 100, 100.0),
            row(2, "b", 2, 20, 2, 100, 300.0),
            row(3, "c", 3, 30, 3, 200, 100.0),
            row(4, "d", 4, 40, 4, 200, 100.0),
            row(5, "e", 5, 50, 5, 50, 10.0),
            row(6, "e", 5, 50, 5, 60, 10.0),
        ];
        let s = compute(&rows, &HashMap::new(), Period::All, 0, 1000, 0);
        let ids = |v: &[Ranked]| v.iter().map(|r| r.id).collect::<Vec<_>>();
        // Two plays first; then most seconds; then the latest play (row 4 is newer than row
        // 3 at the same second); then the earlier play.
        assert_eq!(ids(&s.top_artists), [5, 2, 4, 3, 1]);
        assert_eq!(ids(&s.top_albums), [50, 20, 40, 30, 10]);
        assert_eq!(ids(&s.top_tracks), [5, 2, 4, 3, 1]);
        // Equal in everything: the lower id first.
        let rows = vec![
            Entry {
                id: 1,
                ..row(1, "x", 9, 9, 9, 100, 1.0)
            },
            Entry {
                id: 1,
                ..row(1, "y", 8, 8, 8, 100, 1.0)
            },
        ];
        let s = compute(&rows, &HashMap::new(), Period::All, 0, 1000, 0);
        assert_eq!(ids(&s.top_artists), [8, 9]);
    }

    #[test]
    fn top_lists_hold_twenty_at_most() {
        let rows: Vec<Entry> = (1..=50)
            .map(|i| row(i, &format!("k{i}"), i, i, i, 1000 + i, 1.0))
            .collect();
        let s = compute(&rows, &HashMap::new(), Period::All, 0, 10_000, 0);
        assert_eq!(s.top_tracks.len(), TOP);
        assert_eq!(s.top_albums.len(), TOP);
        assert_eq!(s.top_artists.len(), TOP);
        assert_eq!((s.tracks, s.albums, s.artists), (50, 50, 50));
    }

    #[test]
    fn missing_or_bad_ids_and_seconds_are_not_counted_as_things() {
        let mut odd = row(1, "k", 5, 0, -3, 100, f64::NAN);
        odd.album_id = None;
        let rows = vec![odd, row(2, "k", 5, -1, 0, 101, f64::INFINITY)];
        let s = compute(&rows, &HashMap::new(), Period::All, 0, 1000, 0);
        assert_eq!((s.plays, s.albums, s.artists), (2, 0, 0));
        assert!(s.top_albums.is_empty() && s.top_artists.is_empty());
        assert!(s.seconds.is_finite());
    }

    #[test]
    fn new_artists_are_first_played_inside_the_period() {
        let rows = vec![
            row(1, "a", 1, 1, 100, 5000, 10.0),
            row(2, "b", 2, 2, 200, 5100, 10.0),
            row(3, "c", 3, 3, 200, 5200, 10.0),
            row(4, "d", 4, 4, 300, 5300, 10.0),
        ];
        // 100 was first played before the period; 200 and 300 inside it.
        let firsts = HashMap::from([(100, 1000), (200, 5100), (300, 5300)]);
        let s = compute(&rows, &firsts, Period::All, 4000, 6000, 0);
        let ids: Vec<i64> = s.new_artists.iter().map(|a| a.id).collect();
        assert_eq!(ids, [200, 300], "most played first");
        assert_eq!(s.new_artists[0].first_played, 5100);
        // Exactly at `from` counts; at `to` doesn't.
        let s = compute(&rows, &firsts, Period::All, 5100, 5300, 0);
        assert_eq!(
            s.new_artists.iter().map(|a| a.id).collect::<Vec<_>>(),
            [200]
        );
        // The names needed: top artists, then new ones, each once.
        let s = compute(&rows, &firsts, Period::All, 4000, 6000, 0);
        assert_eq!(s.artist_ids(), [200, 300, 100]);
        // An artist missing from the first plays (rows changed meanwhile) is not new.
        let s = compute(&rows, &HashMap::new(), Period::All, 4000, 6000, 0);
        assert!(s.new_artists.is_empty());
    }

    #[test]
    fn streaks_are_runs_of_local_days() {
        let at = |day: i64, hour: i64| MON + day * DAY + hour * 3600;
        let rows = vec![
            row(1, "a", 1, 1, 1, at(0, 10), 100.0),
            row(2, "a", 1, 1, 1, at(1, 10), 100.0),
            row(3, "a", 1, 1, 1, at(2, 10), 100.0),
            // A gap on day 3.
            row(4, "a", 1, 1, 1, at(4, 10), 500.0),
            row(5, "a", 1, 1, 1, at(5, 10), 100.0),
            row(6, "a", 1, 1, 1, at(6, 10), 100.0),
            row(7, "a", 1, 1, 1, at(6, 11), 400.0),
        ];
        let s = compute(&rows, &HashMap::new(), Period::All, 0, i64::MAX, 0);
        let k = s.longest_streak.clone().unwrap();
        assert_eq!(k.days, 3);
        // Equal runs: the most recent.
        assert_eq!(
            (date(k.from), date(k.to)),
            ("2026-10-09".into(), "2026-10-11".into())
        );
        let (d, secs) = s.busiest_day.unwrap();
        // Days 4 and 6 both have 500 s: the most recent.
        assert_eq!((date(d), secs), ("2026-10-11".into(), 500.0));
        // At UTC−12 every play is the day before: still two runs of three.
        let s = compute(&rows, &HashMap::new(), Period::All, 0, i64::MAX, -720);
        assert_eq!(s.longest_streak.unwrap().days, 3);
        // At UTC−11 the 11:00 UTC play is local midnight on day 6, the rest the day
        // before: days 3 to 6 make four.
        let s = compute(&rows, &HashMap::new(), Period::All, 0, i64::MAX, -660);
        let k = s.longest_streak.unwrap();
        assert_eq!((k.days, date(k.to)), (4, "2026-10-11".into()));
        // A play at 23:30 UTC on day 2 fills the gap at UTC+1.
        let mut more = rows.clone();
        more.push(row(8, "a", 1, 1, 1, at(2, 23) + 1800, 1.0));
        let s = compute(&more, &HashMap::new(), Period::All, 0, i64::MAX, 60);
        let k = s.longest_streak.unwrap();
        assert_eq!(
            (k.days, date(k.from), date(k.to)),
            (7, "2026-10-05".into(), "2026-10-11".into())
        );
        // No plays: no streak, no busiest day.
        let s = compute(&[], &HashMap::new(), Period::All, 0, 1, 0);
        assert!(s.longest_streak.is_none() && s.busiest_day.is_none());
        let v = to_json(&s, true, &HashMap::new());
        assert!(v["longest_streak"].is_null() && v["busiest_day"].is_null());
    }

    #[test]
    fn months_are_listed_for_long_periods_only() {
        let now = days_from_civil(2026, 10, 13) * DAY + 3600;
        let rows = vec![
            row(
                1,
                "a",
                1,
                1,
                1,
                days_from_civil(2026, 3, 31) * DAY + 23 * 3600,
                60.0,
            ),
            row(2, "a", 1, 1, 1, days_from_civil(2026, 10, 1) * DAY, 60.0),
        ];
        let p = Period::Year(2026);
        let (from, to) = p.range(now, 0);
        let s = compute(&rows, &HashMap::new(), p, from, to, 0);
        let months: Vec<&str> = s.by_month.iter().map(|m| m.month.as_str()).collect();
        assert_eq!(months.len(), 10, "January to this month: {months:?}");
        assert_eq!(months[0], "2026-01");
        assert_eq!(months[9], "2026-10");
        assert_eq!(s.by_month[2].plays, 1);
        // At UTC+2 the March play is in April.
        let (from, to) = p.range(now, 120);
        let s = compute(&rows, &HashMap::new(), p, from, to, 120);
        assert_eq!((s.by_month[2].plays, s.by_month[3].plays), (0, 1));
        // A whole past year: twelve months.
        let p = Period::Year(2025);
        let (from, to) = p.range(now, 0);
        assert_eq!(
            compute(&[], &HashMap::new(), p, from, to, 0).by_month.len(),
            12
        );
        // `days-N`: only beyond 60 days.
        for (n, want) in [(60, false), (61, true), (400, true)] {
            let p = Period::Days(n);
            let (from, to) = p.range(now, 0);
            let s = compute(&[], &HashMap::new(), p, from, to, 0);
            assert_eq!(!s.by_month.is_empty(), want, "{n}");
        }
        // `all`: every month the retention period touches.
        let (from, to) = Period::All.range(now, 0);
        let s = compute(&[], &HashMap::new(), Period::All, from, to, 0);
        assert!((13..=15).contains(&s.by_month.len()));
        // A month: none.
        let p = Period::Month {
            year: 2026,
            month: 3,
        };
        let (from, to) = p.range(now, 0);
        assert!(
            compute(&rows, &HashMap::new(), p, from, to, 0)
                .by_month
                .is_empty()
        );
    }

    // -- names ------------------------------------------------------------------

    #[test]
    fn names_are_cleaned_like_other_lms_text() {
        assert_eq!(
            clean_name("  An  Example\tBand \n"),
            Some("An Example Band".into())
        );
        assert_eq!(clean_name("AC/DC"), Some("AC/DC".into()));
        assert_eq!(
            clean_name("Evil\u{202e}dnaB\u{200b}\u{0}\u{7}"),
            Some("EvildnaB".into())
        );
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("\u{200b}\u{202e} \n"), None);
        assert_eq!(clean_name("Band /srv/music/Band/cover.jpg"), None);
        assert_eq!(clean_name("Live at http://10.0.0.10:9000/x"), None);
        assert_eq!(clean_name("file:///srv/music"), None);
        let long = "é".repeat(5000);
        assert_eq!(clean_name(&long).unwrap().chars().count(), MAX_NAME);
        assert_eq!(
            name_in(
                &json!({"artists_loop": [{"id": 12, "artist": "An Example Band"}]}),
                12
            ),
            Some("An Example Band".into())
        );
        assert_eq!(
            name_in(
                &json!({"artists_loop": [{"id": "12", "artist": "An Example Band"}]}),
                12
            ),
            Some("An Example Band".into())
        );
        // Someone else's name, or nothing usable.
        for v in [
            json!({"artists_loop": [{"id": 13, "artist": "Other"}]}),
            json!({"artists_loop": []}),
            json!({"artists_loop": [{"id": 12}]}),
            json!({"artists_loop": [{"id": 12, "artist": 5}]}),
            json!({"artists_loop": "x"}),
            json!({}),
            json!(null),
        ] {
            assert_eq!(name_in(&v, 12), None, "{v}");
        }
    }

    #[test]
    fn caches_are_bounded_and_forget_users() {
        let c = StatsCache::default();
        let now = Instant::now();
        for i in 0..(ANSWERS as i64 + 10) {
            c.put_answer((i, "u".into(), Period::All, 0), 0, json!(i), now);
        }
        assert_eq!(c.answers_len(), ANSWERS);
        let key = (5_000_000, "carol".to_string(), Period::All, 0);
        c.put_answer(key.clone(), 0, json!(1), now);
        assert!(c.answer(&key, now).is_some());
        assert!(c.answer(&key, now + ANSWER_TTL).is_none(), "expires");
        // Dropped on clear, and an answer worked out before is not remembered after.
        let epoch = c.epoch(5_000_000);
        c.forget_user(5_000_000);
        assert!(c.answer(&key, now).is_none());
        c.put_answer(key.clone(), epoch, json!(1), now);
        assert!(c.answer(&key, now).is_none(), "stale answer kept out");
        c.put_answer(key.clone(), c.epoch(5_000_000), json!(2), now);
        assert!(c.answer(&key, now).is_some());
        c.forget_name("carol");
        assert!(c.answer(&key, now).is_none());
        for i in 0..(NAMES as i64 + 10) {
            let at = now + Duration::from_nanos(i as u64);
            c.put_name(i, Known::Name(Some("n".into())), at);
        }
        assert_eq!(c.names.lock().unwrap().len(), NAMES);
        assert!(c.name(0, now).is_none(), "the oldest went first");
        let later = now + Duration::from_secs(1);
        c.put_name(-1, Known::Stalled, later);
        assert_eq!(c.names.lock().unwrap().len(), NAMES);
        assert!(c.name(-1, later).is_some());
        assert!(
            c.name(-1, later + STALL_TTL).is_none(),
            "a failure is retried soon"
        );
        let newest = NAMES as i64 + 9;
        assert!(c.name(newest, later + STALL_TTL).is_some());
        assert!(c.name(newest, later + NAME_TTL).is_none());
    }

    #[test]
    fn switched_off_answers_are_empty() {
        let s = Stats {
            from: 1,
            to: 2,
            ..Stats::default()
        };
        let v = to_json(&s, false, &HashMap::new());
        assert_eq!(v["enabled"], false);
        assert_eq!(v["totals"]["plays"], 0);
        assert_eq!(v["top_artists"], json!([]));
        assert_eq!(v["by_hour"], json!(vec![0; 24]));
        assert_eq!(v["by_weekday"], json!(vec![0; 7]));
        assert_eq!(v["period"], json!({"from": 1, "to": 2}));
    }
}
