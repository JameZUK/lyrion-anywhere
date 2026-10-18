//! Listening history (docs/plans/wave1-design.md §8; spec D23).
//!
//! The gateway records what each signed-in user plays through Lyrion Anywhere, for the Home
//! shelves ([`shelf`]) and the stations ([`crate::stations`]). Nothing here is shared between
//! users, and nothing is recorded for a user who has switched history off.
//!
//! # What is recorded
//!
//! One row per play that counts, by the same rule as `POST /v1/plays`
//! ([`crate::plays::counts`]: half the track or 240 s, whichever is less): the stable track
//! key (as for downloads; it survives rescans and never reveals the path), the LMS track,
//! album and artist ids and year at the time (for the shelves), when the play started, the
//! seconds heard, and whether it was live or offline.
//!
//! # Where plays come from
//!
//! - **Live, on the user's own devices' players.** Each player session (`crate::session`)
//!   belongs to one device, so to one user. Its client's STAT reports drive a [`Tracker`]:
//!   a track starts at `STMs`, and time heard is the elapsed time the player reports while
//!   playing, counted only as far as the player's own clock (the STAT's jiffies) allows (a
//!   seek forward is not "heard"), so reports queued during a network gap and sent together
//!   on resume still count. In all, a session is never credited more than the wall time
//!   since its first report plus a little slack, whatever the reports claim. The
//!   track itself is asked of LMS (`status - 1`) just after it starts, and must be a local
//!   library track. A seek (LMS restarts the stream) continues the same listen. Hardware
//!   players and other users' devices have no session here, so they are never attributed
//!   to anyone; LMS's own play counts cover them.
//! - **Offline**, when `POST /v1/plays` has LMS record a play.
//!
//! # Keeping it small
//!
//! Rows older than [`RETENTION`] (13 months) are pruned for everyone at most hourly, and a
//! user keeps at most [`MAX_ROWS_PER_USER`] (the newest). Users can list
//! (`GET /v1/me/history`), clear (`DELETE`) and switch it off (`PATCH {"enabled": false}`).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::accounts::{Accounts, AuthError, Principal};

/// How long history is kept: 13 months.
pub const RETENTION: Duration = Duration::from_secs(396 * 86_400);
/// Most rows kept per user (the newest).
pub const MAX_ROWS_PER_USER: usize = 50_000;
/// Largest page of `GET /v1/me/history`.
pub const MAX_PAGE: usize = 200;
/// Default page.
pub const DEFAULT_PAGE: usize = 50;
/// How often everyone's old rows are pruned.
const PRUNE_EVERY: Duration = Duration::from_secs(3600);
/// Check a user's row count after this many of their inserts.
const CAP_CHECK_EVERY: u32 = 64;
/// Most ids a shelf returns.
pub const MAX_SHELF: usize = 50;
/// Requests to `/v1/me/…` a user may make a minute (each may ask LMS a few questions).
pub const ME_REQUESTS_PER_MINUTE: usize = 120;

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// Where a play was heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// On one of the user's own devices' players, through the gateway.
    Live,
    /// Offline, sent back with `POST /v1/plays`.
    Offline,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Live => "live",
            Source::Offline => "offline",
        }
    }
}

/// A library track as history needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub id: u64,
    /// The stable key (64 hex), see `crate::download::stable_key`.
    pub key: String,
    pub album_id: Option<i64>,
    pub artist_id: Option<i64>,
    pub year: Option<i64>,
    pub duration: Option<f64>,
}

impl TrackInfo {
    pub fn from_track(t: &crate::download::Track, salt: &str) -> TrackInfo {
        TrackInfo {
            id: t.id,
            key: crate::download::stable_key(salt, &t.url),
            album_id: t.album_id.filter(|n| *n > 0),
            artist_id: t.artist_id,
            year: t.year,
            duration: t.duration,
        }
    }
}

/// One recorded play.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Entry {
    #[serde(skip)]
    pub id: i64,
    pub track_key: String,
    pub track_id: i64,
    pub album_id: Option<i64>,
    pub artist_id: Option<i64>,
    pub year: Option<i64>,
    pub played_at: i64,
    pub seconds: f64,
    pub source: String,
}

/// A position in the history, newest first: plays before this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub played_at: i64,
    pub id: i64,
}

impl Cursor {
    /// `<played_at>-<id>`, both plain decimal numbers.
    pub fn parse(s: &str) -> Option<Cursor> {
        let (a, b) = s.split_once('-')?;
        let num = |x: &str| {
            (!x.is_empty() && x.len() <= 15 && x.bytes().all(|c| c.is_ascii_digit()))
                .then(|| x.parse::<i64>().ok())
                .flatten()
        };
        Some(Cursor {
            played_at: num(a)?,
            id: num(b)?,
        })
    }

    pub fn encode(self) -> String {
        format!("{}-{}", self.played_at, self.id)
    }
}

/// Seconds since 1970.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Is history on for this user? (On unless they switched it off.)
pub fn enabled(a: &Accounts, user: i64) -> Result<bool, AuthError> {
    a.read_db(|db| {
        Ok(db
            .query_row(
                "SELECT history FROM user_prefs WHERE user_id = ?1",
                [user],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_none_or(|v| v != 0))
    })
}

pub fn set_enabled(a: &Accounts, user: i64, on: bool) -> Result<(), AuthError> {
    a.with_db(|db| {
        db.execute(
            "INSERT INTO user_prefs (user_id, history) VALUES (?1, ?2)
             ON CONFLICT(user_id) DO UPDATE SET history = excluded.history",
            params![user, on as i64],
        )
        .map(|_| ())
    })
}

/// Record a play for `user`, if their history is on. Returns the row's id.
pub fn record(
    a: &Accounts,
    user: i64,
    track: &TrackInfo,
    played_at: u64,
    seconds: f64,
    source: Source,
) -> Result<Option<i64>, AuthError> {
    if !enabled(a, user)? {
        return Ok(None);
    }
    let seconds = if seconds.is_finite() {
        seconds.clamp(0.0, crate::plays::MAX_SECONDS)
    } else {
        0.0
    };
    a.with_db(|db| {
        db.execute(
            "INSERT INTO history (user_id, track_key, track_id, album_id, artist_id, year,
                                  played_at, seconds, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                user,
                track.key,
                track.id as i64,
                track.album_id,
                track.artist_id,
                track.year,
                played_at as i64,
                seconds,
                source.as_str()
            ],
        )?;
        Ok(Some(db.last_insert_rowid()))
    })
}

/// A live play went on after it was recorded: store the seconds heard in the end.
pub fn update_seconds(a: &Accounts, user: i64, row: i64, seconds: f64) -> Result<(), AuthError> {
    let seconds = if seconds.is_finite() {
        seconds.clamp(0.0, crate::plays::MAX_SECONDS)
    } else {
        return Ok(());
    };
    a.with_db(|db| {
        db.execute(
            "UPDATE history SET seconds = MAX(seconds, ?3) WHERE id = ?2 AND user_id = ?1",
            params![user, row, seconds],
        )
        .map(|_| ())
    })
}

/// A page of the user's history, newest first, and the cursor for the next page.
pub fn list(
    a: &Accounts,
    user: i64,
    limit: usize,
    before: Option<Cursor>,
) -> Result<(Vec<Entry>, Option<Cursor>), AuthError> {
    let limit = limit.clamp(1, MAX_PAGE);
    let (at, id) = before.map_or((i64::MAX, i64::MAX), |c| (c.played_at, c.id));
    let rows = a.read_db(|db| {
        let mut st = db.prepare(
            "SELECT id, track_key, track_id, album_id, artist_id, year, played_at, seconds, source
             FROM history WHERE user_id = ?1 AND (played_at < ?2 OR (played_at = ?2 AND id < ?3))
             ORDER BY played_at DESC, id DESC LIMIT ?4",
        )?;
        let rows = st.query_map(params![user, at, id, (limit + 1) as i64], row_entry)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })?;
    let mut rows = rows;
    let next = if rows.len() > limit {
        rows.truncate(limit);
        rows.last().map(|e| Cursor {
            played_at: e.played_at,
            id: e.id,
        })
    } else {
        None
    };
    Ok((rows, next))
}

fn row_entry(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        track_key: r.get(1)?,
        track_id: r.get(2)?,
        album_id: r.get(3)?,
        artist_id: r.get(4)?,
        year: r.get(5)?,
        played_at: r.get(6)?,
        seconds: r.get(7)?,
        source: r.get(8)?,
    })
}

/// Every play of the user's since `since` (for scoring), newest first, at most `cap`.
pub fn since(a: &Accounts, user: i64, since: i64, cap: usize) -> Result<Vec<Entry>, AuthError> {
    a.read_db(|db| {
        let mut st = db.prepare(
            "SELECT id, track_key, track_id, album_id, artist_id, year, played_at, seconds, source
             FROM history WHERE user_id = ?1 AND played_at >= ?2
             ORDER BY played_at DESC, id DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![user, since, cap as i64], row_entry)?;
        rows.collect()
    })
}

/// The user's plays with `from <= played_at < to`, newest first, at most `cap` (for the
/// listening stats, [`crate::stats`]).
pub fn between(
    a: &Accounts,
    user: i64,
    from: i64,
    to: i64,
    cap: usize,
) -> Result<Vec<Entry>, AuthError> {
    a.read_db(|db| {
        let mut st = db.prepare(
            "SELECT id, track_key, track_id, album_id, artist_id, year, played_at, seconds, source
             FROM history WHERE user_id = ?1 AND played_at >= ?2 AND played_at < ?3
             ORDER BY played_at DESC, id DESC LIMIT ?4",
        )?;
        let rows = st.query_map(params![user, from, to, cap as i64], row_entry)?;
        rows.collect()
    })
}

/// When the user first played each artist, over everything the history keeps (for "new
/// this period" in the listening stats).
pub fn first_plays(a: &Accounts, user: i64) -> Result<HashMap<i64, i64>, AuthError> {
    a.read_db(|db| {
        let mut st = db.prepare(
            "SELECT artist_id, MIN(played_at) FROM history
             WHERE user_id = ?1 AND artist_id > 0 GROUP BY artist_id",
        )?;
        let rows = st.query_map([user], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect()
    })
}

/// Forget all of a user's history. Returns how many plays were removed.
pub fn clear(a: &Accounts, user: i64) -> Result<usize, AuthError> {
    a.with_db(|db| db.execute("DELETE FROM history WHERE user_id = ?1", [user]))
}

/// Remove everyone's plays older than [`RETENTION`] before `now`.
pub fn prune_old(a: &Accounts, now: u64) -> Result<usize, AuthError> {
    let oldest = now.saturating_sub(RETENTION.as_secs()) as i64;
    a.with_db(|db| db.execute("DELETE FROM history WHERE played_at < ?1", [oldest]))
}

/// Keep at most `cap` of the user's plays, the newest.
pub fn prune_user(a: &Accounts, user: i64, cap: usize) -> Result<usize, AuthError> {
    a.with_db(|db| {
        let n: i64 = db.query_row(
            "SELECT count(*) FROM history WHERE user_id = ?1",
            [user],
            |r| r.get(0),
        )?;
        if n as usize <= cap {
            return Ok(0);
        }
        db.execute(
            "DELETE FROM history WHERE user_id = ?1 AND id NOT IN (
               SELECT id FROM history WHERE user_id = ?1
               ORDER BY played_at DESC, id DESC LIMIT ?2)",
            params![user, cap as i64],
        )
    })
}

/// How many plays are kept for this user.
pub fn count(a: &Accounts, user: i64) -> Result<usize, AuthError> {
    a.read_db(|db| {
        db.query_row(
            "SELECT count(*) FROM history WHERE user_id = ?1",
            [user],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n as usize)
    })
}

/// Bookkeeping for pruning: when everyone's rows were last pruned, and inserts per user
/// since their row count was last checked.
#[derive(Default)]
pub struct Upkeep {
    pruned: Mutex<Option<Instant>>,
    inserts: Mutex<HashMap<i64, u32>>,
}

impl crate::Gateway {
    /// Record a play in the user's history (if on), with the pruning that goes with it.
    pub(crate) fn note_play(
        &self,
        user: i64,
        track: &TrackInfo,
        played_at: u64,
        seconds: f64,
        source: Source,
    ) -> Option<i64> {
        let accounts = self.accounts();
        {
            let mut pruned = self.history_upkeep.pruned.lock().unwrap();
            if pruned.is_none_or(|t| t.elapsed() >= PRUNE_EVERY) {
                *pruned = Some(Instant::now());
                if let Err(e) = prune_old(accounts, unix_now()) {
                    tracing::warn!(error = ?e, "could not prune old listening history");
                }
            }
        }
        let row = match record(accounts, user, track, played_at, seconds, source) {
            Ok(r) => r?,
            Err(e) => {
                tracing::warn!(error = ?e, "could not record listening history");
                return None;
            }
        };
        let check = {
            let mut m = self.history_upkeep.inserts.lock().unwrap();
            if m.len() > 10_000 {
                m.clear();
            }
            let n = m.entry(user).or_insert(0);
            *n += 1;
            *n % CAP_CHECK_EVERY == 1
        };
        if check {
            let _ = prune_user(accounts, user, MAX_ROWS_PER_USER);
        }
        Some(row)
    }
}

// ---------------------------------------------------------------------------
// Live plays: following one player's reports
// ---------------------------------------------------------------------------

/// Elapsed time may run this much ahead of the player's clock between two reports and still
/// count (reports are about a second apart; a seek jumps much further).
const SLACK_MS: u64 = 3000;
/// Over a whole session, the time heard may run this much ahead of the wall time since the
/// player's first report (the clocks start a moment apart). A player can't claim more than
/// this, however its reports are timed.
const BUDGET_SLACK_MS: u64 = 10_000;
/// A step of the player's clock (jiffies, ms, wrapping at 2^32) beyond this is not time that
/// passed: the clock went backwards or was reset, or the value is absurd. Nothing is heard
/// over such a step. (A session survives at most an hour detached.)
const MAX_CLOCK_STEP_MS: u32 = 2 * 3600 * 1000;
/// A seek restarts the stream: a new start of the same track this soon after the last one
/// ended continues the same listen.
const MERGE_WITHIN: Duration = Duration::from_secs(15);

/// What the session must do for the tracker.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Ask LMS which track listen `serial` is (the player started a new track).
    Lookup(u64),
    /// Listen `serial` now counts: record it.
    Record {
        serial: u64,
        track: TrackInfo,
        played_at: u64,
        seconds: f64,
    },
    /// A recorded listen ended or went on: store the seconds heard.
    Update { row: i64, seconds: f64 },
}

#[derive(Debug, Clone)]
struct Listen {
    serial: u64,
    started: u64,
    track: Option<TrackInfo>,
    heard_ms: u64,
    /// Its history row, once recorded.
    row: Option<i64>,
    /// A record step has been issued.
    recording: bool,
    /// The serial of the record step that will give this listen its row (its own, or that
    /// of the listen it continues after a seek).
    record_serial: Option<u64>,
    /// LMS has said which track this is (or that it couldn't).
    named: bool,
}

/// Listens that ended while their record step was still being carried out, kept (a few)
/// until it reports their row, so the time heard in the end is still stored.
const MAX_AWAITING_ROW: usize = 16;

/// One player's listening, from its own status reports (pure: no I/O, the caller supplies
/// the clocks and carries out the [`Step`]s).
#[derive(Debug, Default)]
pub struct Tracker {
    serial: u64,
    listen: Option<Listen>,
    /// The listen before a track change, kept until the new track is known (a seek starts
    /// the same track again).
    prev: Option<(Listen, Instant)>,
    /// Time heard in starts after `prev` that were never named (their lookup was coalesced
    /// into a later one, as when scrubbing): credited to `prev` if the next named start
    /// continues it.
    unnamed_ms: u64,
    /// The last report: elapsed milliseconds and the player's clock (jiffies).
    last: Option<(u32, u32)>,
    playing: bool,
    /// When the first report arrived (the start of the wall-time budget).
    first_report: Option<Instant>,
    /// Milliseconds heard so far in the session, over every listen.
    credited_ms: u64,
    /// Ended listens whose row is not known yet: (record serial, seconds heard).
    awaiting_row: Vec<(u64, f64)>,
}

impl Tracker {
    /// A status report (STAT) from the player: its event, elapsed milliseconds and clock
    /// (`jiffies`, ms), with the wall time it arrived (`now`, `unix`).
    ///
    /// An elapsed step counts as heard if it is no bigger than the player's own clock step
    /// plus [`SLACK_MS`]: reports held back during a network gap and sent in a burst are
    /// judged by when they were made, not when they arrived. A clock that went backwards
    /// (other than wrapping) or jumped absurdly counts nothing. Whatever the reports say, the
    /// session's total heard never exceeds the wall time since its first report plus
    /// [`BUDGET_SLACK_MS`].
    pub fn on_stat(
        &mut self,
        event: [u8; 4],
        elapsed_ms: u32,
        jiffies: u32,
        now: Instant,
        unix: u64,
    ) -> Vec<Step> {
        let mut out = Vec::new();
        let first = *self.first_report.get_or_insert(now);
        if &event == b"STMs" {
            // A new track has started. The listen so far waits until LMS says which track
            // this is (it may be the same one, restarted by a seek).
            match self.listen.take() {
                // A start never named (quick seeks): the listen before it still waits for
                // a named one, which may continue it.
                Some(l) if !l.named && self.prev.is_some() => {
                    self.unnamed_ms = self.unnamed_ms.saturating_add(l.heard_ms);
                    if let Some((_, ended)) = self.prev.as_mut() {
                        *ended = now;
                    }
                }
                cur => {
                    if let Some(old) = self.prev.take() {
                        out.extend(self.end_listen(old.0));
                    }
                    self.unnamed_ms = 0;
                    self.prev = cur.map(|l| (l, now));
                }
            }
            self.serial += 1;
            self.listen = Some(Listen {
                serial: self.serial,
                started: unix,
                track: None,
                heard_ms: 0,
                row: None,
                recording: false,
                record_serial: None,
                named: false,
            });
            self.last = Some((elapsed_ms, jiffies));
            self.playing = true;
            out.push(Step::Lookup(self.serial));
            return out;
        }
        if self.playing
            && let Some((last_ms, last_jiffies)) = self.last
            && let Some(l) = self.listen.as_mut()
        {
            // Wrapping: the clock passing 2^32 is a small step; going backwards is huge.
            let clock = jiffies.wrapping_sub(last_jiffies);
            if clock <= MAX_CLOCK_STEP_MS && elapsed_ms > last_ms {
                let d = (elapsed_ms - last_ms) as u64;
                if d <= clock as u64 + SLACK_MS {
                    let wall = now.saturating_duration_since(first).as_millis() as u64;
                    let budget = (wall + BUDGET_SLACK_MS).saturating_sub(self.credited_ms);
                    let d = d.min(budget);
                    l.heard_ms = l.heard_ms.saturating_add(d);
                    self.credited_ms = self.credited_ms.saturating_add(d);
                }
            }
        }
        self.last = Some((elapsed_ms, jiffies));
        match &event {
            b"STMr" => self.playing = true,
            b"STMp" | b"STMf" | b"STMu" | b"STMn" => self.playing = false,
            _ => {}
        }
        out.extend(self.check());
        out
    }

    /// The latest listen's serial (the one a [`Step::Lookup`] issued now would name).
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// LMS answered which track listen `serial` is (`None`: not a local library track, or it
    /// couldn't say).
    pub fn resolved(&mut self, serial: u64, track: Option<TrackInfo>, now: Instant) -> Vec<Step> {
        let mut out = Vec::new();
        let Some(l) = self.listen.as_mut().filter(|l| l.serial == serial) else {
            return out;
        };
        l.named = true;
        let unnamed_ms = std::mem::take(&mut self.unnamed_ms);
        let mut ended_prev = None;
        if let Some((p, ended)) = self.prev.take() {
            let same = matches!((&p.track, &track), (Some(a), Some(b)) if a.id == b.id);
            if same && now.saturating_duration_since(ended) <= MERGE_WITHIN {
                l.heard_ms = l
                    .heard_ms
                    .saturating_add(p.heard_ms)
                    .saturating_add(unnamed_ms);
                l.row = p.row;
                l.recording = p.recording;
                l.record_serial = p.record_serial;
                l.started = p.started;
            } else {
                ended_prev = Some(p);
            }
        }
        if let Some(p) = ended_prev {
            out.extend(self.end_listen(p));
        }
        if let Some(l) = self.listen.as_mut().filter(|l| l.serial == serial) {
            l.track = track;
        }
        out.extend(self.check());
        out
    }

    /// The record step for listen `serial` was carried out (`row`: its id, or `None` if
    /// history is off or it failed). The step is carried out off the session's task, so the
    /// listen may have gone on (after a seek), or ended, meanwhile: an ended one's final time
    /// heard is stored now.
    pub fn recorded(&mut self, serial: u64, row: Option<i64>) -> Option<Step> {
        let mine = |l: &Listen| l.record_serial == Some(serial);
        if let Some(l) = self.listen.as_mut().filter(|l| mine(l)) {
            l.row = row;
        } else if let Some((p, _)) = self.prev.as_mut().filter(|(p, _)| mine(p)) {
            p.row = row;
        } else if let Some(i) = self.awaiting_row.iter().position(|(s, _)| *s == serial) {
            let (_, seconds) = self.awaiting_row.remove(i);
            return row.map(|row| Step::Update { row, seconds });
        }
        None
    }

    /// The session ended: whatever is still going ends here.
    pub fn finish(&mut self) -> Vec<Step> {
        let mut out = Vec::new();
        if let Some((p, _)) = self.prev.take() {
            out.extend(self.end_listen(p));
        }
        if let Some(l) = self.listen.take() {
            out.extend(self.end_listen(l));
        }
        self.unnamed_ms = 0;
        self.playing = false;
        out
    }

    /// A listen is over: store the seconds heard, now if its row is known, else once its
    /// record step reports it (see [`Self::recorded`]).
    fn end_listen(&mut self, l: Listen) -> Option<Step> {
        let seconds = l.heard_ms as f64 / 1000.0;
        match (l.row, l.record_serial) {
            (Some(row), _) => Some(Step::Update { row, seconds }),
            (None, Some(serial)) if l.recording => {
                if self.awaiting_row.len() >= MAX_AWAITING_ROW {
                    self.awaiting_row.remove(0);
                }
                self.awaiting_row.push((serial, seconds));
                None
            }
            _ => None,
        }
    }

    /// Seconds heard of the current listen (for tests and diagnostics).
    pub fn heard(&self) -> f64 {
        self.listen
            .as_ref()
            .map_or(0.0, |l| l.heard_ms as f64 / 1000.0)
    }

    fn check(&mut self) -> Option<Step> {
        let l = self.listen.as_mut()?;
        let track = l.track.as_ref()?;
        let seconds = l.heard_ms as f64 / 1000.0;
        if l.recording || !crate::plays::counts(seconds, track.duration) {
            return None;
        }
        l.recording = true;
        l.record_serial = Some(l.serial);
        Some(Step::Record {
            serial: l.serial,
            track: track.clone(),
            played_at: l.started,
            seconds,
        })
    }
}

impl crate::Gateway {
    /// Which library track `mac`'s player is playing (for a [`Step::Lookup`]): `None` if it
    /// isn't a local library track or LMS can't say.
    pub(crate) async fn playing_track(&self, mac: &str) -> Option<TrackInfo> {
        let cmd = ["status", "-", "1", "tags:"].map(String::from);
        let status = self
            .lms_command_within(mac, &cmd, Duration::from_secs(5))
            .await
            .ok()?;
        let id = status
            .get("playlist_loop")
            .and_then(Value::as_array)
            .and_then(|l| l.first())
            .and_then(|t| t.get("id"))
            .and_then(|v| match v {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => s.parse().ok(),
                _ => None,
            })?;
        let id = u64::try_from(id).ok().filter(|n| *n > 0)?;
        let track = self.library_track(id).await.ok()?;
        let salt = self.download_salt()?;
        Some(TrackInfo::from_track(&track, &salt))
    }
}

// ---------------------------------------------------------------------------
// Scoring (pure)
// ---------------------------------------------------------------------------

/// Weights for blending the user's own history with LMS's server-wide numbers (which also
/// cover hardware players and other users, so count for less). Tunable.
pub const USER_WEIGHT: f64 = 3.0;
pub const SERVER_WEIGHT: f64 = 1.0;

/// "Recently played" albums: the user's own, most recent first (from `plays`, newest
/// first). Not blended with LMS's `sort:recentlyplayed`, which is server-wide and would show
/// one user what the others played lately.
pub fn recently_played(plays: &[Entry], limit: usize) -> Vec<i64> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for p in plays {
        if let Some(a) = p.album_id.filter(|a| *a > 0)
            && seen.insert(a)
        {
            order.push(a);
        }
    }
    order.truncate(limit);
    order
}

/// "Most played" albums over the plays given: the user's own plays per album, most first,
/// then the most recent. Not blended with LMS's server-wide play counts, for the same reason.
pub fn most_played(plays: &[Entry], limit: usize) -> Vec<i64> {
    let mut counts: HashMap<i64, u64> = HashMap::new();
    let mut order = Vec::new();
    for p in plays {
        if let Some(a) = p.album_id.filter(|a| *a > 0) {
            let c = counts.entry(a).or_insert(0);
            if *c == 0 {
                order.push(a);
            }
            *c += 1;
        }
    }
    // A stable sort: most plays first, then by recency.
    order.sort_by(|a, b| counts[b].cmp(&counts[a]));
    order.truncate(limit);
    order
}

/// "On this day": albums the user played within `window_days` of this date a year ago
/// (history is kept 13 months), most played first. LMS keeps only each track's last play,
/// so this shelf is the user's own.
pub fn on_this_day(plays: &[Entry], now: i64, window_days: i64, limit: usize) -> Vec<i64> {
    let year_ago = now - 365 * 86_400;
    let (lo, hi) = (
        year_ago - window_days * 86_400,
        year_ago + window_days * 86_400,
    );
    let picked: Vec<Entry> = plays
        .iter()
        .filter(|p| (lo..=hi).contains(&p.played_at))
        .cloned()
        .collect();
    most_played(&picked, limit)
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

pub(crate) fn error(status: StatusCode, message: &str) -> Response {
    let mut r = (status, Json(json!({ "error": message }))).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

pub(crate) fn ok(v: Value) -> Response {
    let mut r = Json(v).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    r
}

/// The caller, within their per-minute allowance of `/v1/me/…` requests.
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
pub(crate) fn caller(gw: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    let Some(p) = crate::auth::principal(gw, headers) else {
        return Err(error(StatusCode::UNAUTHORIZED, "invalid credentials"));
    };
    if !gw.me_requests.take(p.user_id, Instant::now()) {
        let mut r = error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many requests; try again shortly",
        );
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("30"));
        return Err(r);
    }
    Ok(p)
}

/// A JSON body of at most `max` bytes.
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
pub(crate) async fn body_json(
    headers: &HeaderMap,
    body: Body,
    max: usize,
) -> Result<Value, Response> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > max as u64) {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "request too large"));
    }
    let Ok(bytes) = axum::body::to_bytes(body, max).await else {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "request too large"));
    };
    serde_json::from_slice(&bytes).map_err(|_| error(StatusCode::BAD_REQUEST, "malformed request"))
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    limit: Option<String>,
    before: Option<String>,
}

/// `GET /v1/me/history?limit=&before=`
pub async fn get(
    State(gw): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<HistoryQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Ok(Query(q)) = q else {
        return error(StatusCode::BAD_REQUEST, "bad query");
    };
    let limit = match q.limit.as_deref() {
        None => DEFAULT_PAGE,
        Some(s) => match s.parse::<usize>() {
            Ok(n) if (1..=MAX_PAGE).contains(&n) && s.len() <= 3 => n,
            _ => return error(StatusCode::BAD_REQUEST, "bad limit"),
        },
    };
    let before = match q.before.as_deref() {
        None => None,
        Some(s) => match Cursor::parse(s) {
            Some(c) => Some(c),
            None => return error(StatusCode::BAD_REQUEST, "bad cursor"),
        },
    };
    let accounts = gw.accounts();
    let (Ok(on), Ok((items, next))) = (
        enabled(accounts, p.user_id),
        list(accounts, p.user_id, limit, before),
    ) else {
        return internal();
    };
    ok(json!({
        "enabled": on,
        "retention_days": RETENTION.as_secs() / 86_400,
        "items": items,
        "next": next.map(Cursor::encode),
    }))
}

/// `DELETE /v1/me/history`: forget all of it.
pub async fn delete(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let user = p.user_id;
    match gw.db(move |a| clear(a, user)).await {
        Ok(n) => {
            gw.station_served.forget_user(p.user_id);
            gw.stats.forget_user(p.user_id);
            gw.accounts().audit(
                "history.cleared",
                Some(&p.username),
                Some(p.device_id),
                &format!("{n} plays"),
            );
            ok(json!({ "removed": n }))
        }
        Err(_) => internal(),
    }
}

/// `PATCH /v1/me/history` `{"enabled": bool}`: switch recording on or off.
pub async fn patch(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let v = match body_json(&headers, body, 256).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let on = match v.as_object() {
        Some(o) if o.len() == 1 => match o.get("enabled") {
            Some(Value::Bool(b)) => *b,
            _ => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "expected {\"enabled\": true|false}",
                );
            }
        },
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "expected {\"enabled\": true|false}",
            );
        }
    };
    let user = p.user_id;
    match gw.db(move |a| set_enabled(a, user, on)).await {
        Ok(()) => {
            // Stats answers are dropped too (switched off, they must show nothing).
            gw.stats.forget_user(p.user_id);
            ok(json!({ "enabled": on }))
        }
        Err(_) => internal(),
    }
}

/// The shelves `GET /v1/me/shelves/{name}` serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    Recent,
    Most,
    OnThisDay,
}

impl Shelf {
    pub fn parse(s: &str) -> Option<Shelf> {
        match s {
            "recent" => Some(Shelf::Recent),
            "most" => Some(Shelf::Most),
            "on_this_day" => Some(Shelf::OnThisDay),
            _ => None,
        }
    }
}

/// Plays considered for the shelves: the newest, at most this many.
const SHELF_PLAYS: usize = 5000;
/// "Most played" looks back this far.
const MOST_PLAYED_DAYS: i64 = 90;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShelfQuery {
    limit: Option<String>,
}

/// `GET /v1/me/shelves/{recent|most|on_this_day}?limit=`: album ids, best first.
pub async fn shelf(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    q: Result<Query<ShelfQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(which) = Shelf::parse(&name) else {
        return error(StatusCode::NOT_FOUND, "no such shelf");
    };
    let Ok(Query(q)) = q else {
        return error(StatusCode::BAD_REQUEST, "bad query");
    };
    let limit = match q.limit.as_deref() {
        None => 20,
        Some(s) => match s.parse::<usize>() {
            Ok(n) if (1..=MAX_SHELF).contains(&n) && s.len() <= 2 => n,
            _ => return error(StatusCode::BAD_REQUEST, "bad limit"),
        },
    };
    let now = unix_now() as i64;
    let window = match which {
        Shelf::Most => now - MOST_PLAYED_DAYS * 86_400,
        _ => now - RETENTION.as_secs() as i64,
    };
    let Ok(plays) = since(gw.accounts(), p.user_id, window, SHELF_PLAYS) else {
        return internal();
    };
    let albums = match which {
        // The user's own only (review low): LMS's lists are server-wide.
        Shelf::Recent => recently_played(&plays, limit),
        Shelf::Most => most_played(&plays, limit),
        Shelf::OnThisDay => on_this_day(&plays, now, 3, limit),
    };
    ok(json!({ "shelf": name, "albums": albums }))
}

impl crate::Gateway {
    /// Album ids LMS lists in this order (`sort:recentlyplayed`, `sort:playcount`, `new`):
    /// empty if LMS can't (LMS before 9.1 has no play-count sorts).
    pub(crate) async fn server_albums(&self, sort: &str, count: usize) -> Vec<i64> {
        let cmd = [
            "albums".to_string(),
            "0".into(),
            count.min(500).to_string(),
            format!("sort:{sort}"),
            "tags:".into(),
        ];
        let Ok(v) = self
            .lms_query_retrying("", &cmd, Duration::from_secs(10))
            .await
        else {
            return Vec::new();
        };
        ids_in(&v, "albums_loop")
    }
}

/// The positive ids of a loop in an LMS answer, in order.
pub(crate) fn ids_in(v: &Value, list: &str) -> Vec<i64> {
    v.get(list)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|x| match x.get("id") {
                    Some(Value::Number(n)) => n.as_i64(),
                    Some(Value::String(s)) => s.parse().ok(),
                    _ => None,
                })
                .filter(|n| *n > 0)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Role;

    fn track(id: u64, album: i64, duration: f64) -> TrackInfo {
        TrackInfo {
            id,
            key: format!("{id:064x}"),
            album_id: Some(album),
            artist_id: Some(100 + album),
            year: Some(1985),
            duration: Some(duration),
        }
    }

    fn users() -> (Accounts, i64, i64) {
        let a = Accounts::in_memory().unwrap();
        a.create_user("alice", "correct horse battery", Role::Listener)
            .unwrap();
        a.create_user("bob", "correct horse battery", Role::Listener)
            .unwrap();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let bob = a.user_id_by_name("bob").unwrap().unwrap();
        (a, alice, bob)
    }

    fn entry(album: i64, at: i64) -> Entry {
        Entry {
            id: at,
            track_key: "k".into(),
            track_id: album * 10,
            album_id: Some(album),
            artist_id: None,
            year: None,
            played_at: at,
            seconds: 200.0,
            source: "live".into(),
        }
    }

    // -- the store ------------------------------------------------------------

    #[test]
    fn plays_are_recorded_per_user_and_listed_newest_first() {
        let (a, alice, bob) = users();
        for i in 0..5u64 {
            record(
                &a,
                alice,
                &track(i + 1, 1, 300.0),
                1000 + i,
                200.0,
                Source::Live,
            )
            .unwrap();
        }
        record(&a, bob, &track(9, 2, 300.0), 2000, 200.0, Source::Offline).unwrap();
        let (items, next) = list(&a, alice, 3, None).unwrap();
        assert_eq!(
            items.iter().map(|e| e.played_at).collect::<Vec<_>>(),
            [1004, 1003, 1002]
        );
        let (rest, end) = list(&a, alice, 3, next).unwrap();
        assert_eq!(
            rest.iter().map(|e| e.played_at).collect::<Vec<_>>(),
            [1001, 1000]
        );
        assert_eq!(end, None);
        // Never another user's.
        assert!(items.iter().chain(&rest).all(|e| e.track_id != 9));
        let (bobs, _) = list(&a, bob, 50, None).unwrap();
        assert_eq!(bobs.len(), 1);
        assert_eq!(bobs[0].source, "offline");
    }

    #[test]
    fn plays_at_the_same_second_page_without_gaps() {
        let (a, alice, _) = users();
        for i in 0..7u64 {
            record(
                &a,
                alice,
                &track(i + 1, 1, 300.0),
                5000,
                200.0,
                Source::Live,
            )
            .unwrap();
        }
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let (items, next) = list(&a, alice, 2, cursor).unwrap();
            seen.extend(items.iter().map(|e| e.track_id));
            if next.is_none() {
                break;
            }
            cursor = next;
        }
        seen.sort();
        assert_eq!(seen, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn switched_off_means_nothing_is_recorded() {
        let (a, alice, bob) = users();
        assert!(enabled(&a, alice).unwrap(), "on by default");
        set_enabled(&a, alice, false).unwrap();
        assert!(!enabled(&a, alice).unwrap());
        assert_eq!(
            record(&a, alice, &track(1, 1, 300.0), 1, 200.0, Source::Live).unwrap(),
            None
        );
        assert_eq!(count(&a, alice).unwrap(), 0);
        assert!(enabled(&a, bob).unwrap(), "only alice's setting changed");
        set_enabled(&a, alice, true).unwrap();
        assert!(
            record(&a, alice, &track(1, 1, 300.0), 1, 200.0, Source::Live)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn clearing_removes_only_the_users_own_history() {
        let (a, alice, bob) = users();
        record(&a, alice, &track(1, 1, 300.0), 1, 200.0, Source::Live).unwrap();
        record(&a, bob, &track(2, 1, 300.0), 1, 200.0, Source::Live).unwrap();
        assert_eq!(clear(&a, alice).unwrap(), 1);
        assert_eq!(count(&a, alice).unwrap(), 0);
        assert_eq!(count(&a, bob).unwrap(), 1);
    }

    #[test]
    fn plays_older_than_13_months_are_pruned() {
        let (a, alice, _) = users();
        let now = 2_000_000_000u64;
        let limit = RETENTION.as_secs();
        assert_eq!(limit / 86_400, 396, "13 months");
        record(
            &a,
            alice,
            &track(1, 1, 300.0),
            now - limit - 1,
            200.0,
            Source::Live,
        )
        .unwrap();
        record(
            &a,
            alice,
            &track(2, 1, 300.0),
            now - limit,
            200.0,
            Source::Live,
        )
        .unwrap();
        record(
            &a,
            alice,
            &track(3, 1, 300.0),
            now - 10,
            200.0,
            Source::Live,
        )
        .unwrap();
        assert_eq!(prune_old(&a, now).unwrap(), 1);
        let (left, _) = list(&a, alice, 10, None).unwrap();
        assert_eq!(left.iter().map(|e| e.track_id).collect::<Vec<_>>(), [3, 2]);
    }

    #[test]
    fn a_user_keeps_at_most_the_cap_newest() {
        let (a, alice, bob) = users();
        for i in 0..20u64 {
            record(
                &a,
                alice,
                &track(i + 1, 1, 300.0),
                100 + i,
                200.0,
                Source::Live,
            )
            .unwrap();
        }
        record(&a, bob, &track(99, 1, 300.0), 1, 200.0, Source::Live).unwrap();
        assert_eq!(prune_user(&a, alice, 5).unwrap(), 15);
        let (left, _) = list(&a, alice, 50, None).unwrap();
        assert_eq!(
            left.iter().map(|e| e.track_id).collect::<Vec<_>>(),
            [20, 19, 18, 17, 16]
        );
        assert_eq!(count(&a, bob).unwrap(), 1, "other users untouched");
        assert_eq!(prune_user(&a, alice, 5).unwrap(), 0);
    }

    #[test]
    fn seconds_are_updated_only_for_the_users_own_row_and_never_lowered() {
        let (a, alice, bob) = users();
        let row = record(&a, alice, &track(1, 1, 300.0), 1, 150.0, Source::Live)
            .unwrap()
            .unwrap();
        update_seconds(&a, bob, row, 999.0).unwrap();
        update_seconds(&a, alice, row, 100.0).unwrap();
        assert_eq!(list(&a, alice, 1, None).unwrap().0[0].seconds, 150.0);
        update_seconds(&a, alice, row, 290.5).unwrap();
        assert_eq!(list(&a, alice, 1, None).unwrap().0[0].seconds, 290.5);
        update_seconds(&a, alice, row, f64::NAN).unwrap();
        update_seconds(&a, alice, row, 1e12).unwrap();
        assert_eq!(
            list(&a, alice, 1, None).unwrap().0[0].seconds,
            crate::plays::MAX_SECONDS
        );
    }

    #[test]
    fn cursors_are_strict() {
        assert_eq!(
            Cursor::parse("1790000000-42"),
            Some(Cursor {
                played_at: 1_790_000_000,
                id: 42
            })
        );
        for bad in [
            "",
            "-",
            "1-",
            "-1",
            "1-2-3",
            "a-1",
            "1-a",
            "+1-1",
            "1 -1",
            "1-1 ",
            "1234567890123456-1",
            "1.0-1",
        ] {
            assert_eq!(Cursor::parse(bad), None, "{bad:?}");
        }
        let c = Cursor {
            played_at: 5,
            id: 6,
        };
        assert_eq!(Cursor::parse(&c.encode()), Some(c));
    }

    // -- the tracker ----------------------------------------------------------

    struct Clock(Instant);
    impl Clock {
        fn at(&self, ms: u64) -> Instant {
            self.0 + Duration::from_millis(ms)
        }
    }

    /// Play `seconds` of reports, one a second from `from_ms` of wall time, with elapsed
    /// starting at `elapsed0`.
    fn play_for(
        t: &mut Tracker,
        c: &Clock,
        from_ms: u64,
        elapsed0: u32,
        seconds: u32,
    ) -> Vec<Step> {
        let mut out = Vec::new();
        for s in 1..=seconds {
            let wall = from_ms + s as u64 * 1000;
            out.extend(t.on_stat(*b"STMt", elapsed0 + s * 1000, wall as u32, c.at(wall), 0));
        }
        out
    }

    /// The record step runs off the session's task (M7), so its row can arrive after the
    /// listen ended: the final time heard is stored then, and only once.
    #[test]
    fn a_listen_that_ends_before_its_row_is_known_still_stores_its_time() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1000);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500));
        let steps = play_for(&mut t, &c, 0, 0, 50);
        assert!(
            matches!(steps[..], [Step::Record { serial: 1, .. }]),
            "{steps:?}"
        );
        play_for(&mut t, &c, 50_000, 50_000, 10);
        // The session ends while the record step is still being carried out.
        assert!(t.finish().is_empty());
        assert_eq!(
            t.recorded(1, Some(77)),
            Some(Step::Update {
                row: 77,
                seconds: 60.0
            })
        );
        assert_eq!(t.recorded(1, Some(77)), None);
        // A record step that failed (no row): nothing to update.
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1000);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500));
        play_for(&mut t, &c, 0, 0, 50);
        t.finish();
        assert_eq!(t.recorded(1, None), None);
    }

    /// A seek continues a listen whose row is still on its way: the row reaches it.
    #[test]
    fn a_row_arriving_after_a_seek_reaches_the_continued_listen() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1000);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500));
        play_for(&mut t, &c, 0, 0, 50);
        // A seek: the same track starts again.
        t.on_stat(*b"STMs", 70_000, 51_000, c.at(51_000), 1051);
        assert!(
            t.resolved(2, Some(track(5, 1, 100.0)), c.at(52_000))
                .is_empty()
        );
        assert_eq!(t.recorded(1, Some(77)), None);
        // The next track ends it: the row is known.
        t.on_stat(*b"STMs", 0, 60_000, c.at(60_000), 1060);
        let steps = t.resolved(3, Some(track(6, 1, 100.0)), c.at(61_000));
        assert!(
            matches!(steps[..], [Step::Update { row: 77, .. }]),
            "{steps:?}"
        );
    }

    /// The row arriving between the seek's start and LMS naming the track reaches the
    /// continued listen too.
    #[test]
    fn a_row_arriving_before_the_seek_is_resolved_reaches_the_continued_listen() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1000);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500));
        play_for(&mut t, &c, 0, 0, 50);
        t.on_stat(*b"STMs", 70_000, 51_000, c.at(51_000), 1051);
        assert_eq!(t.recorded(1, Some(77)), None);
        assert!(
            t.resolved(2, Some(track(5, 1, 100.0)), c.at(52_000))
                .is_empty()
        );
        play_for(&mut t, &c, 52_000, 70_000, 10);
        t.on_stat(*b"STMs", 0, 63_000, c.at(63_000), 1063);
        let steps = t.resolved(3, Some(track(6, 1, 100.0)), c.at(64_000));
        assert_eq!(
            steps,
            vec![Step::Update {
                row: 77,
                seconds: 60.0
            }]
        );
    }

    /// Two quick seeks (scrubbing) before LMS names the track: the lookup is coalesced, so
    /// the middle start is never resolved. The listen still continues, its row still
    /// arrives, and the play is not recorded a second time (review low: before, the first
    /// listen ended at the second start and its row kept only the seconds at the seek).
    #[test]
    fn quick_seeks_before_the_track_is_named_continue_the_listen() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1000);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500));
        let steps = play_for(&mut t, &c, 0, 0, 50);
        assert!(matches!(steps[..], [Step::Record { serial: 1, .. }]));
        // Two seeks in quick succession; the row is still on its way.
        assert_eq!(
            t.on_stat(*b"STMs", 60_000, 51_000, c.at(51_000), 1051),
            vec![Step::Lookup(2)]
        );
        play_for(&mut t, &c, 51_000, 60_000, 1);
        assert_eq!(
            t.on_stat(*b"STMs", 80_000, 53_000, c.at(53_000), 1053),
            vec![Step::Lookup(3)]
        );
        // The row arrives: the listen is still going, so nothing to store yet.
        assert_eq!(t.recorded(1, Some(77)), None);
        assert!(
            t.resolved(3, Some(track(5, 1, 100.0)), c.at(54_000))
                .is_empty()
        );
        // It goes on, and is not recorded again.
        let steps = play_for(&mut t, &c, 54_000, 80_000, 19);
        assert!(steps.is_empty(), "{steps:?}");
        // The next track ends it: every second heard, the middle one included.
        t.on_stat(*b"STMs", 0, 74_000, c.at(74_000), 1074);
        let steps = t.resolved(4, Some(track(6, 1, 100.0)), c.at(75_000));
        assert_eq!(
            steps,
            vec![Step::Update {
                row: 77,
                seconds: 70.0
            }]
        );
    }

    /// A start that is never named and is followed by a different track: the listen before
    /// it ends with its own seconds (the unnamed one's are not credited to it).
    #[test]
    fn an_unnamed_start_followed_by_another_track_ends_the_listen() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1000);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500));
        play_for(&mut t, &c, 0, 0, 50);
        t.recorded(1, Some(77));
        t.on_stat(*b"STMs", 0, 51_000, c.at(51_000), 1051);
        play_for(&mut t, &c, 51_000, 0, 2);
        t.on_stat(*b"STMs", 0, 54_000, c.at(54_000), 1054);
        let steps = t.resolved(3, Some(track(6, 1, 100.0)), c.at(55_000));
        assert_eq!(
            steps,
            vec![Step::Update {
                row: 77,
                seconds: 50.0
            }]
        );
    }

    #[test]
    fn a_play_is_recorded_once_it_counts() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        assert_eq!(
            t.on_stat(*b"STMs", 0, 0, c.at(0), 1000),
            vec![Step::Lookup(1)]
        );
        assert!(
            t.resolved(1, Some(track(5, 1, 100.0)), c.at(1500))
                .is_empty()
        );
        // Half of 100 s: 50 s.
        let steps = play_for(&mut t, &c, 0, 0, 49);
        assert!(steps.is_empty(), "{steps:?}");
        let steps = play_for(&mut t, &c, 49_000, 49_000, 1);
        assert_eq!(
            steps,
            vec![Step::Record {
                serial: 1,
                track: track(5, 1, 100.0),
                played_at: 1000,
                seconds: 50.0
            }]
        );
        t.recorded(1, Some(77));
        // Only once.
        assert!(play_for(&mut t, &c, 50_000, 50_000, 30).is_empty());
        // The next track ends it: the seconds heard are stored.
        let steps = t.on_stat(*b"STMs", 0, 81_000, c.at(81_000), 2000);
        assert_eq!(steps, vec![Step::Lookup(2)]);
        let steps = t.resolved(2, Some(track(6, 1, 100.0)), c.at(82_000));
        assert_eq!(
            steps,
            vec![Step::Update {
                row: 77,
                seconds: 80.0
            }]
        );
    }

    #[test]
    fn short_plays_seeks_forward_and_pauses_are_not_heard() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1);
        t.resolved(1, Some(track(5, 1, 600.0)), c.at(100));
        play_for(&mut t, &c, 0, 0, 10);
        assert_eq!(t.heard(), 10.0);
        // Elapsed jumps 200 s in one second: a seek, not listening.
        t.on_stat(*b"STMt", 210_000, 11_000, c.at(11_000), 0);
        assert_eq!(t.heard(), 10.0);
        // Paused: reports go on but nothing is heard, even if elapsed creeps.
        t.on_stat(*b"STMp", 210_000, 12_000, c.at(12_000), 0);
        t.on_stat(*b"STMt", 211_000, 13_000, c.at(13_000), 0);
        assert_eq!(t.heard(), 10.0);
        t.on_stat(*b"STMr", 211_000, 14_000, c.at(14_000), 0);
        t.on_stat(*b"STMt", 212_000, 15_000, c.at(15_000), 0);
        assert_eq!(t.heard(), 11.0);
        // Elapsed going backwards (a seek back) counts nothing but carries on from there.
        t.on_stat(*b"STMt", 5_000, 16_000, c.at(16_000), 0);
        t.on_stat(*b"STMt", 6_000, 17_000, c.at(17_000), 0);
        assert_eq!(t.heard(), 12.0);
        // Never recorded: 12 s of a 600 s track.
        assert!(t.finish().is_empty());
    }

    #[test]
    fn a_seek_that_restarts_the_stream_continues_the_listen() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 500);
        t.resolved(1, Some(track(5, 1, 400.0)), c.at(100));
        play_for(&mut t, &c, 0, 0, 100);
        // LMS restarts the stream at the seek point: a new STMs of the same track.
        assert_eq!(
            t.on_stat(*b"STMs", 0, 101_000, c.at(101_000), 600),
            vec![Step::Lookup(2)]
        );
        assert!(
            t.resolved(2, Some(track(5, 1, 400.0)), c.at(101_500))
                .is_empty()
        );
        assert_eq!(t.heard(), 100.0);
        let steps = play_for(&mut t, &c, 101_000, 0, 100);
        // 200 s heard of a 400 s track: counts, started at the first start.
        assert_eq!(
            steps,
            vec![Step::Record {
                serial: 2,
                track: track(5, 1, 400.0),
                played_at: 500,
                seconds: 200.0
            }]
        );
    }

    #[test]
    fn radio_and_unknown_tracks_are_never_recorded() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1);
        assert!(t.resolved(1, None, c.at(100)).is_empty());
        assert!(play_for(&mut t, &c, 0, 0, 600).is_empty());
        // A lookup for an older listen is ignored.
        t.on_stat(*b"STMs", 0, 601_000, c.at(601_000), 2);
        assert!(
            t.resolved(1, Some(track(1, 1, 10.0)), c.at(601_500))
                .is_empty()
        );
        assert!(play_for(&mut t, &c, 601_000, 0, 30).is_empty());
    }

    #[test]
    fn the_session_ending_finishes_the_listen() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1);
        t.resolved(1, Some(track(5, 1, 60.0)), c.at(100));
        let steps = play_for(&mut t, &c, 0, 0, 40);
        assert!(matches!(steps.as_slice(), [Step::Record { .. }]));
        t.recorded(1, Some(3));
        assert_eq!(
            t.finish(),
            vec![Step::Update {
                row: 3,
                seconds: 40.0
            }]
        );
        assert!(t.finish().is_empty());
    }

    /// Regression (rolling cache, A4): reports queued during a network gap arrive together
    /// on resume. Judged by arrival time, all but the first counted nothing; judged by the
    /// player's own clock they count what was really heard.
    #[test]
    fn a_burst_of_reports_after_a_gap_counts_by_the_players_clock() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 1_000, c.at(0), 1);
        t.resolved(1, Some(track(5, 1, 100.0)), c.at(100));
        for s in 1..=10u32 {
            let wall = s as u64 * 1000;
            t.on_stat(*b"STMt", s * 1000, 1_000 + s * 1000, c.at(wall), 0);
        }
        assert_eq!(t.heard(), 10.0);
        // 60 s gap, then 12 reports made 5 s apart (more than the slack) arrive within a few
        // milliseconds.
        let mut steps = Vec::new();
        for s in 1..=12u32 {
            steps.extend(t.on_stat(
                *b"STMt",
                10_000 + s * 5000,
                11_000 + s * 5000,
                c.at(70_000 + s as u64),
                0,
            ));
        }
        assert_eq!(t.heard(), 70.0);
        assert!(
            matches!(steps.as_slice(), [Step::Record { seconds, .. }] if *seconds == 50.0),
            "{steps:?}"
        );
    }

    #[test]
    fn the_players_clock_wraps_but_never_runs_backwards() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        let j0 = u32::MAX - 1_500;
        t.on_stat(*b"STMs", 0, j0, c.at(0), 1);
        // Across the wrap: 1 s steps still count.
        for s in 1..=3u32 {
            t.on_stat(
                *b"STMt",
                s * 1000,
                j0.wrapping_add(s * 1000),
                c.at(s as u64 * 1000),
                0,
            );
        }
        assert_eq!(t.heard(), 3.0);
        // Backwards (not a wrap): nothing, and the next step is judged from there.
        t.on_stat(*b"STMt", 4_000, 100, c.at(4_000), 0);
        assert_eq!(t.heard(), 3.0);
        t.on_stat(*b"STMt", 5_000, 1_100, c.at(5_000), 0);
        assert_eq!(t.heard(), 4.0);
        // An absurd jump of the clock (days): nothing.
        t.on_stat(*b"STMt", 6_000, 1_100 + 3 * 86_400_000, c.at(6_000), 0);
        assert_eq!(t.heard(), 4.0);
        // A clock that stands still allows only the slack: a seek of 200 s is not heard.
        t.on_stat(*b"STMt", 206_000, 1_100 + 3 * 86_400_000, c.at(7_000), 0);
        assert_eq!(t.heard(), 4.0);
        // A step bigger than the clock's (plus slack) is a seek, even after a long gap.
        t.on_stat(
            *b"STMt",
            306_000,
            1_100 + 3 * 86_400_000 + 1_000,
            c.at(600_000),
            0,
        );
        assert_eq!(t.heard(), 4.0);
    }

    #[test]
    fn a_player_claiming_more_clock_than_wall_time_is_capped() {
        let c = Clock(Instant::now());
        let mut t = Tracker::default();
        t.on_stat(*b"STMs", 0, 0, c.at(0), 1);
        t.resolved(1, Some(track(5, 1, 3600.0)), c.at(100));
        // Jiffies and elapsed both race ahead: an hour claimed within one second of wall time.
        let mut steps = Vec::new();
        for s in 1..=3600u32 {
            steps.extend(t.on_stat(*b"STMt", s * 1000, s * 1000, c.at(s as u64 / 4), 0));
        }
        assert!(
            t.heard() * 1000.0 <= (900 + BUDGET_SLACK_MS) as f64,
            "{}",
            t.heard()
        );
        assert!(steps.is_empty(), "never counts as a play: {steps:?}");
        // Time that really passes is heard again, but never beyond the wall time.
        for s in 1..=5u32 {
            let wall = 60_000 + s as u64 * 1000;
            t.on_stat(
                *b"STMt",
                3_600_000 + s * 1000,
                3_600_000 + s * 1000,
                c.at(wall),
                0,
            );
        }
        assert!(t.heard() > 10.0);
        assert!(t.heard() * 1000.0 <= (65_000 + BUDGET_SLACK_MS) as f64);
    }

    /// Small deterministic PRNG (xorshift64), so failures are reproducible from the seed.
    fn rng(seed: u64) -> impl FnMut() -> u64 {
        let mut x = seed | 1;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
    }

    /// Security: whatever a player reports (random values, clocks racing, backwards or
    /// wrapping, many reports in one millisecond), nothing panics, and the time heard over
    /// the whole session, and in all the plays recorded, never exceeds the wall time since
    /// its first report plus the budget slack.
    #[test]
    fn hostile_reports_never_panic_or_overcount() {
        let events = [
            *b"STMs",
            *b"STMt",
            *b"STMp",
            *b"STMr",
            *b"STMf",
            *b"STMu",
            *b"STMd",
            *b"STMo",
            *b"\0\0\0\0",
        ];
        for seed in 1..=40u64 {
            let mut rnd = rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let start = Instant::now();
            let mut t = Tracker::default();
            let mut wall = 0u64;
            let (mut elapsed, mut jiffies) = (0u32, rnd() as u32);
            let mut first_wall = None;
            let mut rows: HashMap<i64, f64> = HashMap::new();
            let mut next_row = 1;
            for _ in 0..5_000 {
                // Wall time: often none at all (a burst), sometimes a long gap.
                wall += match rnd() % 4 {
                    0 => 0,
                    1 => rnd() % 3,
                    2 => rnd() % 3000,
                    _ => rnd() % 120_000,
                };
                let ev = match rnd() % 6 {
                    0 => events[(rnd() % events.len() as u64) as usize],
                    _ => *b"STMt",
                };
                // Clock and elapsed: honest, racing together, random or backwards.
                match rnd() % 5 {
                    0 => (elapsed, jiffies) = (rnd() as u32, rnd() as u32),
                    1 => {
                        let d = (rnd() % 600_000) as u32;
                        elapsed = elapsed.wrapping_add(d);
                        jiffies = jiffies.wrapping_add(d);
                    }
                    2 => jiffies = jiffies.wrapping_sub((rnd() % 5000) as u32),
                    _ => {
                        let d = (rnd() % 4000) as u32;
                        elapsed = elapsed.wrapping_add(d);
                        jiffies = jiffies.wrapping_add(d);
                    }
                }
                let first = *first_wall.get_or_insert(wall);
                let now = start + Duration::from_millis(wall);
                let mut steps = t.on_stat(ev, elapsed, jiffies, now, rnd());
                while let Some(s) = steps.pop() {
                    match s {
                        Step::Lookup(g) => {
                            if !rnd().is_multiple_of(3) {
                                let dur = (rnd() % 1000) as f64;
                                steps.extend(t.resolved(g, Some(track(1, 1, dur)), now));
                            }
                        }
                        Step::Record {
                            serial, seconds, ..
                        } => {
                            rows.insert(next_row, seconds);
                            t.recorded(serial, Some(next_row));
                            next_row += 1;
                        }
                        Step::Update { row, seconds } => {
                            rows.insert(row, seconds);
                        }
                    }
                }
                let budget = (wall - first + BUDGET_SLACK_MS) as f64;
                assert!(t.credited_ms as f64 <= budget, "seed {seed}");
                // (+1: seconds are floating point.)
                assert!(t.heard() * 1000.0 <= budget + 1.0, "seed {seed}");
                // Plays are separate listens: together they fit the budget too.
                let total: f64 = rows.values().sum();
                assert!(total * 1000.0 <= budget + 1.0, "seed {seed}: {total}");
            }
            for s in t.finish() {
                if let Step::Update { row, seconds } = s {
                    rows.insert(row, seconds);
                }
            }
            let total: f64 = rows.values().sum();
            let budget = (wall - first_wall.unwrap() + BUDGET_SLACK_MS) as f64;
            assert!(total * 1000.0 <= budget + 1.0, "seed {seed}: {total}");
        }
    }

    // -- scoring --------------------------------------------------------------

    #[test]
    fn recently_played_is_the_users_own_order() {
        let plays = vec![entry(1, 300), entry(2, 200), entry(1, 100), entry(3, 50)];
        assert_eq!(recently_played(&plays, 10), vec![1, 2, 3]);
        assert_eq!(recently_played(&plays, 2), vec![1, 2]);
        assert!(recently_played(&[], 5).is_empty());
    }

    #[test]
    fn most_played_counts_only_the_users_plays() {
        let mut plays = vec![];
        for _ in 0..5 {
            plays.push(entry(1, 10));
        }
        for _ in 0..4 {
            plays.push(entry(2, 10));
        }
        plays.push(entry(3, 10));
        plays.push(entry(4, 10));
        plays.push(entry(3, 10));
        assert_eq!(most_played(&plays, 10), vec![1, 2, 3, 4]);
        assert_eq!(most_played(&plays, 1), vec![1]);
        // Plays without an album are ignored; ids are never negative.
        let odd = vec![
            Entry {
                album_id: None,
                ..entry(1, 1)
            },
            Entry {
                album_id: Some(-4),
                ..entry(1, 1)
            },
        ];
        assert!(most_played(&odd, 10).is_empty());
    }

    #[test]
    fn on_this_day_is_this_date_last_year() {
        let now = 1_800_000_000i64;
        let year_ago = now - 365 * 86_400;
        let plays = vec![
            entry(1, year_ago),
            entry(1, year_ago + 2 * 86_400),
            entry(2, year_ago - 3 * 86_400),
            entry(3, year_ago - 4 * 86_400),
            entry(4, now - 86_400),
        ];
        assert_eq!(on_this_day(&plays, now, 3, 10), vec![1, 2]);
        assert!(on_this_day(&[], now, 3, 10).is_empty());
    }

    #[test]
    fn lms_ids_are_read_strictly() {
        let v = json!({"albums_loop": [{"id": 3}, {"id": "4"}, {"id": -1}, {"id": "x"}, {}, {"id": 0}, {"id": 5.5}]});
        assert_eq!(ids_in(&v, "albums_loop"), vec![3, 4]);
        assert!(ids_in(&json!({}), "albums_loop").is_empty());
        assert!(ids_in(&json!({"albums_loop": "x"}), "albums_loop").is_empty());
    }
}
