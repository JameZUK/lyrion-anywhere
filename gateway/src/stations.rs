//! Library stations and pinned menus (docs/plans/wave1-design.md §6, §7).
//!
//! Each user keeps their own stations and pinned menus on the gateway (`user_pins`), so
//! they are the same on every device. A station is endless: when the queue runs low the
//! client asks for the next batch (`POST /v1/me/stations/{id}/next`).
//!
//! # Kinds
//!
//! - **Built-in, computed here** from the user's history ([`crate::history`], weighted 3)
//!   and LMS's server-wide play counts, last-played and added dates (weighted 1), as a list
//!   of library track ids the client loads (`playlistcontrol cmd:load|add track_id:…`):
//!   - `forgotten`: often played, not heard for a long time;
//!   - `fresh`: recently added, barely played;
//!   - `deep_cuts`: rarely or never played tracks by artists the user plays a lot;
//!   - `genre` with several genres, and `decade` (LMS's Random Mix can't filter by year).
//!
//!   A batch never repeats the user's last 200 plays, the tracks the client says are queued,
//!   or the last 200 this station served.
//! - **LMS Random Mix** for a single `genre`: the answer is a `randomplay tracks
//!   genre_id:<id>` command for the client to send through `/v1/rpc` (its grammar is fixed
//!   in the allow-list), and LMS keeps the queue topped up itself.
//! - **Pinned from a menu** (`menu`): "Save as station" on a mix or generator item (e.g. an
//!   AudioMuse-AI mix). What is stored is a *reference to an offered action*, never a free
//!   command: the menu requests that led to the item (`steps`) and the item's `play` (and
//!   optional `add`) actions. Every time the station plays, the gateway replays the steps
//!   under the user's own role and players, exactly as `/v1/rpc` would: the first must pass
//!   the fixed allow-list and be a menu, each later one must be a menu action LMS offered
//!   in the previous step's filtered answer, and `play`/`add` must be offered in the last
//!   one ([`crate::menu_actions::allowed`]: never server commands, admin-only actions for
//!   admins only, admin-only plugins hidden). Only then does it run the action, on a player
//!   the user may control. If anything is no longer offered the station is marked broken,
//!   with the reason, and nothing runs. Saving one checks it the same way first.
//!
//! Pinned menus (`/v1/me/menus`) are the same reference without an action: opening one
//! replays its steps and returns the last menu, whose actions the device may then run.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::accounts::{Accounts, AuthError, PlayerScope, Principal, Role};
use crate::history::{self, body_json, caller, error, ok};
use crate::menu_actions;
use crate::rpc_policy::{self, Caller, Kind};

/// Largest station or pinned-menu body.
pub const MAX_BODY: usize = 4096;
/// Stations and pinned menus one user may keep (together).
pub const MAX_PINS_PER_USER: usize = 100;
/// Menu requests from the root to the item, at most.
pub const MAX_STEPS: usize = 4;
/// Words in one stored command, at most.
const MAX_WORDS: usize = 16;
/// Genres in one genre station.
pub const MAX_GENRES: usize = 5;
/// Tracks in one batch.
pub const MAX_BATCH: usize = 50;
/// Track ids a client may ask to leave out.
pub const MAX_EXCLUDE: usize = 200;
/// Recent plays and served tracks never repeated.
pub const AVOID_RECENT: usize = 200;
/// Batches a user may ask for a minute (each asks LMS several questions).
pub const BATCHES_PER_MINUTE: usize = 20;
const LMS_LIMIT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Specs (pure)
// ---------------------------------------------------------------------------

/// What a station plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Genre { genres: Vec<u64> },
    Decade { decade: u32, mine: bool },
    Forgotten,
    Fresh,
    DeepCuts,
    Menu(MenuRef),
}

/// A reference to a menu item LMS offered: how to reach it, and what to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRef {
    pub steps: Vec<Vec<String>>,
    pub play: Option<Vec<String>>,
    pub add: Option<Vec<String>>,
}

impl Spec {
    pub fn kind(&self) -> &'static str {
        match self {
            Spec::Genre { .. } => "genre",
            Spec::Decade { .. } => "decade",
            Spec::Forgotten => "forgotten",
            Spec::Fresh => "fresh",
            Spec::DeepCuts => "deep_cuts",
            Spec::Menu(_) => "menu",
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Spec::Genre { genres } => json!({"kind": "genre", "genres": genres}),
            Spec::Decade { decade, mine } => {
                json!({"kind": "decade", "decade": decade, "mine": mine})
            }
            Spec::Forgotten | Spec::Fresh | Spec::DeepCuts => json!({"kind": self.kind()}),
            Spec::Menu(m) => {
                let mut o = json!({"kind": "menu", "steps": m.steps, "play": m.play});
                if let Some(a) = &m.add {
                    o["add"] = json!(a);
                }
                o
            }
        }
    }
}

/// Why a spec was refused.
pub type Invalid = &'static str;

fn only_keys(o: &Map<String, Value>, allowed: &[&str]) -> Result<(), Invalid> {
    if o.keys().all(|k| allowed.contains(&k.as_str())) {
        Ok(())
    } else {
        Err("unexpected field")
    }
}

fn positive_id(v: &Value) -> Option<u64> {
    v.as_u64()
        .filter(|_| v.is_u64())
        .filter(|n| (1..=crate::plays::MAX_TRACK_ID).contains(n))
}

/// One stored command: 1–16 words, strings (or non-negative integers, kept as text), each
/// at most 256 bytes and passing the free-text check; the command words themselves must be
/// plain words. What it may *do* is checked against LMS's offer when it is used.
pub fn parse_command(v: &Value) -> Result<Vec<String>, Invalid> {
    let a = v.as_array().ok_or("a command must be a list")?;
    if a.is_empty() || a.len() > MAX_WORDS {
        return Err("bad command length");
    }
    let words: Vec<String> = a
        .iter()
        .map(|w| match w {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) if n.is_u64() => Some(n.to_string()),
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or("commands hold strings and numbers only")?;
    if words.iter().any(|w| {
        w.len() > 256
            || !rpc_policy::text_ok(w)
            || w.contains("__TAGGEDINPUT__")
            || w.contains("__INPUT__")
    }) {
        return Err("bad word in command");
    }
    let first = &words[0];
    if !(1..=64).contains(&first.len())
        || !first
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("bad command word");
    }
    // It must at least have the shape of a menu request (words, paging, key:value).
    if menu_actions::parse(&words).is_none() {
        return Err("malformed command");
    }
    Ok(words)
}

/// Why a station whose text holds a URL resolving into the home network won't run.
const LOCAL_LINK: &str = "a link in this menu leads into the home network";

fn parse_steps(v: Option<&Value>) -> Result<Vec<Vec<String>>, Invalid> {
    let a = v.and_then(Value::as_array).ok_or("steps must be a list")?;
    if a.is_empty() || a.len() > MAX_STEPS {
        return Err("between 1 and 4 steps");
    }
    let steps: Vec<Vec<String>> = a.iter().map(parse_command).collect::<Result<_, _>>()?;
    // Every step opens a menu: paged.
    for s in &steps {
        let r = menu_actions::parse(s).ok_or("malformed step")?;
        if r.paging.is_none() {
            return Err("each step must be a menu request with paging");
        }
    }
    Ok(steps)
}

fn parse_action(v: Option<&Value>) -> Result<Option<Vec<String>>, Invalid> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let c = parse_command(v)?;
            if menu_actions::parse(&c).is_some_and(|r| r.paging.is_some()) {
                return Err("an action is a command, not a menu");
            }
            Ok(Some(c))
        }
    }
}

/// Read a station spec (as stored or as sent): a strict, closed set of shapes.
pub fn parse_spec(v: &Value) -> Result<Spec, Invalid> {
    let o = v.as_object().ok_or("a spec is an object")?;
    let kind = o
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("kind missing")?;
    match kind {
        "genre" => {
            only_keys(o, &["kind", "genres"])?;
            let g = o
                .get("genres")
                .and_then(Value::as_array)
                .ok_or("genres missing")?;
            if g.is_empty() || g.len() > MAX_GENRES {
                return Err("between 1 and 5 genres");
            }
            let mut genres: Vec<u64> = g
                .iter()
                .map(positive_id)
                .collect::<Option<_>>()
                .ok_or("bad genre id")?;
            let n = genres.len();
            genres.sort_unstable();
            genres.dedup();
            if genres.len() != n {
                return Err("repeated genre");
            }
            Ok(Spec::Genre { genres })
        }
        "decade" => {
            only_keys(o, &["kind", "decade", "mine"])?;
            let d = o
                .get("decade")
                .and_then(|d| d.as_u64().filter(|_| d.is_u64()))
                .filter(|d| (1900..=2090).contains(d) && d % 10 == 0)
                .ok_or("decade must be a year like 1980")?;
            let mine = match o.get("mine") {
                None => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err("mine must be true or false"),
            };
            Ok(Spec::Decade {
                decade: d as u32,
                mine,
            })
        }
        "forgotten" | "fresh" | "deep_cuts" => {
            only_keys(o, &["kind"])?;
            Ok(match kind {
                "forgotten" => Spec::Forgotten,
                "fresh" => Spec::Fresh,
                _ => Spec::DeepCuts,
            })
        }
        "menu" => {
            only_keys(o, &["kind", "steps", "play", "add"])?;
            let steps = parse_steps(o.get("steps"))?;
            let play = parse_action(o.get("play"))?;
            let add = parse_action(o.get("add"))?;
            if play.is_none() {
                return Err("a station from a menu needs the item's play action");
            }
            Ok(Spec::Menu(MenuRef { steps, play, add }))
        }
        _ => Err("unknown kind"),
    }
}

/// Read a pinned menu (`{"steps": [...]}`).
pub fn parse_menu(v: &Value) -> Result<MenuRef, Invalid> {
    let o = v.as_object().ok_or("a pinned menu is an object")?;
    only_keys(o, &["steps"])?;
    Ok(MenuRef {
        steps: parse_steps(o.get("steps"))?,
        play: None,
        add: None,
    })
}

/// A user-given name: 1–64 characters, cleaned like device names.
pub fn parse_name(v: Option<&Value>) -> Result<String, Invalid> {
    let s = v.and_then(Value::as_str).ok_or("name missing")?;
    if s.chars().count() > 64 {
        return Err("name too long");
    }
    let clean = crate::accounts::clean_name(s).ok_or("bad name")?;
    if clean.chars().count() > 64 {
        return Err("name too long");
    }
    Ok(clean)
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinKind {
    Station,
    Menu,
}

impl PinKind {
    fn as_str(self) -> &'static str {
        match self {
            PinKind::Station => "station",
            PinKind::Menu => "menu",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pin {
    pub id: i64,
    pub name: String,
    pub spec: Value,
    pub broken: Option<String>,
}

/// The built-in stations every user starts with (they may delete them).
const DEFAULTS: [(&str, &str); 3] = [
    ("Forgotten favourites", "forgotten"),
    ("Fresh", "fresh"),
    ("Deep cuts", "deep_cuts"),
];

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Add the built-in stations for a user who has never had them.
pub fn seed(a: &Accounts, user: i64) -> Result<(), AuthError> {
    a.with_db(|db| {
        let seeded: Option<i64> = db
            .query_row(
                "SELECT stations_seeded FROM user_prefs WHERE user_id = ?1",
                [user],
                |r| r.get(0),
            )
            .optional()?;
        if seeded == Some(1) {
            return Ok(());
        }
        let t = now();
        for (name, kind) in DEFAULTS {
            db.execute(
                "INSERT INTO user_pins (user_id, kind, name, spec, created_at)
                 VALUES (?1, 'station', ?2, ?3, ?4)",
                params![user, name, json!({"kind": kind}).to_string(), t],
            )?;
        }
        db.execute(
            "INSERT INTO user_prefs (user_id, stations_seeded) VALUES (?1, 1)
             ON CONFLICT(user_id) DO UPDATE SET stations_seeded = 1",
            [user],
        )?;
        Ok(())
    })
}

/// A user's stations or pinned menus, oldest first. Anything stored that no longer
/// parses is left out.
pub fn list(a: &Accounts, user: i64, kind: PinKind) -> Result<Vec<Pin>, AuthError> {
    let rows: Vec<(i64, String, String, Option<String>)> = a.with_db(|db| {
        let mut st = db.prepare(
            "SELECT id, name, spec, broken FROM user_pins WHERE user_id = ?1 AND kind = ?2 ORDER BY id",
        )?;
        let rows = st.query_map(params![user, kind.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    })?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, name, spec, broken)| {
            let v: Value = serde_json::from_str(&spec).ok()?;
            let ok = match kind {
                PinKind::Station => parse_spec(&v).is_ok(),
                PinKind::Menu => parse_menu(&v).is_ok(),
            };
            ok.then_some(Pin {
                id,
                name,
                spec: v,
                broken,
            })
        })
        .collect())
}

/// One of the user's own pins (never another user's).
pub fn get(a: &Accounts, user: i64, kind: PinKind, id: i64) -> Result<Option<Pin>, AuthError> {
    Ok(list(a, user, kind)?.into_iter().find(|p| p.id == id))
}

/// Does this user own a pin of this kind with this id?
pub fn owns(a: &Accounts, user: i64, kind: PinKind, id: i64) -> Result<bool, AuthError> {
    a.with_db(|db| {
        db.query_row(
            "SELECT 1 FROM user_pins WHERE user_id = ?1 AND kind = ?2 AND id = ?3",
            params![user, kind.as_str(), id],
            |_| Ok(()),
        )
        .optional()
        .map(|r| r.is_some())
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum AddError {
    Full,
    Store,
}

pub fn add(
    a: &Accounts,
    user: i64,
    kind: PinKind,
    name: &str,
    spec: &Value,
) -> Result<i64, AddError> {
    a.with_db(|db| {
        let n: i64 = db.query_row("SELECT count(*) FROM user_pins WHERE user_id = ?1", [user], |r| r.get(0))?;
        if n as usize >= MAX_PINS_PER_USER {
            return Ok(None);
        }
        db.execute(
            "INSERT INTO user_pins (user_id, kind, name, spec, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user, kind.as_str(), name, spec.to_string(), now()],
        )?;
        Ok(Some(db.last_insert_rowid()))
    })
    .map_err(|_| AddError::Store)?
    .ok_or(AddError::Full)
}

/// Remove one of the user's own pins. False if there was none.
pub fn remove(a: &Accounts, user: i64, kind: PinKind, id: i64) -> Result<bool, AuthError> {
    a.with_db(|db| {
        db.execute(
            "DELETE FROM user_pins WHERE user_id = ?1 AND kind = ?2 AND id = ?3",
            params![user, kind.as_str(), id],
        )
        .map(|n| n > 0)
    })
}

/// Mark a pin broken (with the reason), or working again (`None`).
pub fn set_broken(a: &Accounts, user: i64, id: i64, reason: Option<&str>) -> Result<(), AuthError> {
    a.with_db(|db| {
        db.execute(
            "UPDATE user_pins SET broken = ?3 WHERE user_id = ?1 AND id = ?2",
            params![user, id, reason],
        )
        .map(|_| ())
    })
}

/// An id from a URL path: a plain positive decimal number.
pub fn parse_id(s: &str) -> Option<i64> {
    (!s.is_empty() && s.len() <= 15 && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

// ---------------------------------------------------------------------------
// Picking tracks (pure)
// ---------------------------------------------------------------------------

/// A small random source (xorshift), seeded from the OS.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Rng {
        let mut b = [0u8; 8];
        let _ = getrandom::fill(&mut b);
        Rng(u64::from_le_bytes(b) | 1)
    }

    pub fn seeded(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// 0..n (0 if n is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Default for Rng {
    fn default() -> Self {
        Rng::new()
    }
}

/// Pick up to `n` distinct ids, each with probability in proportion to its weight, leaving
/// out `exclude` and ids that aren't positive. Weights that aren't finite and positive
/// count as nothing.
pub fn weighted_pick(
    cands: &[(i64, f64)],
    exclude: &HashSet<i64>,
    n: usize,
    rng: &mut Rng,
) -> Vec<i64> {
    let mut pool: Vec<(i64, f64)> = Vec::new();
    let mut seen = HashSet::new();
    for (id, w) in cands {
        if *id > 0 && w.is_finite() && *w > 0.0 && !exclude.contains(id) && seen.insert(*id) {
            pool.push((*id, *w));
        }
    }
    let mut out = Vec::new();
    while out.len() < n && !pool.is_empty() {
        let total: f64 = pool.iter().map(|(_, w)| w).sum();
        let mut x = rng.unit() * total;
        let mut idx = pool.len() - 1;
        for (i, (_, w)) in pool.iter().enumerate() {
            if x < *w {
                idx = i;
                break;
            }
            x -= w;
        }
        out.push(pool.swap_remove(idx).0);
    }
    out
}

/// A library track as LMS lists it for station picking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LmsTrack {
    pub id: i64,
    pub playcount: u64,
}

/// Tracks in an LMS `titles` answer, with play counts (tag `O`).
pub fn lms_tracks(v: &Value) -> Vec<LmsTrack> {
    let num = |x: Option<&Value>| match x {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    v.get("titles_loop")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|t| {
                    let id = num(t.get("id")).filter(|n| *n > 0)?;
                    let playcount = num(t.get("playcount")).unwrap_or(0).max(0) as u64;
                    Some(LmsTrack { id, playcount })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Per-track numbers from the user's history.
#[derive(Debug, Clone, Default)]
pub struct UserTracks {
    /// track id → (plays, last played).
    pub plays: HashMap<i64, (u32, i64)>,
    /// artist id → plays.
    pub artists: HashMap<i64, u32>,
    /// Track ids of the newest [`AVOID_RECENT`] plays.
    pub recent: Vec<i64>,
}

impl UserTracks {
    pub fn from_history(plays: &[history::Entry]) -> UserTracks {
        let mut u = UserTracks::default();
        for (i, p) in plays.iter().enumerate() {
            if i < AVOID_RECENT {
                u.recent.push(p.track_id);
            }
            let e = u.plays.entry(p.track_id).or_insert((0, p.played_at));
            e.0 += 1;
            e.1 = e.1.max(p.played_at);
            if let Some(a) = p.artist_id {
                *u.artists.entry(a).or_insert(0) += 1;
            }
        }
        u
    }
}

/// "Forgotten favourites" from the user's side: tracks played at least twice, none of it in
/// the last `quiet_days`, weighted by plays (scaled to the most played) × the user weight.
pub fn forgotten_user(u: &UserTracks, now: i64, quiet_days: i64) -> Vec<(i64, f64)> {
    let cutoff = now - quiet_days * 86_400;
    let picked: Vec<(i64, u32)> = u
        .plays
        .iter()
        .filter(|(_, (n, last))| *n >= 2 && *last < cutoff)
        .map(|(id, (n, _))| (*id, *n))
        .collect();
    let max = picked.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f64;
    let mut out: Vec<(i64, f64)> = picked
        .into_iter()
        .map(|(id, n)| (id, history::USER_WEIGHT * n as f64 / max))
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// LMS's side of a station from tracks it listed: `keep` decides which count, each weighted
/// by the server weight (scaled by play count when `by_plays`).
pub fn server_side(
    tracks: &[LmsTrack],
    keep: impl Fn(&LmsTrack) -> bool,
    by_plays: bool,
) -> Vec<(i64, f64)> {
    let max = tracks.iter().map(|t| t.playcount).max().unwrap_or(1).max(1) as f64;
    tracks
        .iter()
        .filter(|t| keep(t))
        .map(|t| {
            let w = if by_plays {
                t.playcount.max(1) as f64 / max
            } else {
                1.0
            };
            (t.id, history::SERVER_WEIGHT * w)
        })
        .collect()
}

/// Artists for "Deep cuts": the user's most played (× 3, scaled) blended with LMS's
/// play-count ranking (× 1), best first.
pub fn top_artists(u: &UserTracks, server_ranked: &[i64], limit: usize) -> Vec<i64> {
    let max = u.artists.values().copied().max().unwrap_or(1).max(1) as f64;
    let n = server_ranked.len().max(1) as f64;
    let mut score: HashMap<i64, f64> = HashMap::new();
    for (a, c) in &u.artists {
        *score.entry(*a).or_insert(0.0) += history::USER_WEIGHT * *c as f64 / max;
    }
    for (i, a) in server_ranked.iter().enumerate() {
        *score.entry(*a).or_insert(0.0) += history::SERVER_WEIGHT * (1.0 - i as f64 / n);
    }
    let mut out: Vec<(i64, f64)> = score.into_iter().filter(|(a, _)| *a > 0).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out.into_iter().take(limit).map(|(a, _)| a).collect()
}

// ---------------------------------------------------------------------------
// Making batches (talks to LMS)
// ---------------------------------------------------------------------------

/// Tracks each station served lately, per user and station.
#[derive(Default)]
pub struct Served(Mutex<HashMap<(i64, i64), VecDeque<i64>>>);

impl Served {
    fn recent(&self, key: (i64, i64)) -> Vec<i64> {
        self.0
            .lock()
            .unwrap()
            .get(&key)
            .map(|q| q.iter().copied().collect())
            .unwrap_or_default()
    }

    fn note(&self, key: (i64, i64), ids: &[i64]) {
        let mut m = self.0.lock().unwrap();
        if m.len() > 10_000 {
            m.clear();
        }
        let q = m.entry(key).or_default();
        q.extend(ids.iter().copied());
        while q.len() > AVOID_RECENT {
            q.pop_front();
        }
    }

    pub(crate) fn forget_user(&self, user: i64) {
        self.0.lock().unwrap().retain(|(u, _), _| *u != user);
    }
}

/// What the client should do with a station's next batch.
#[derive(Debug, Clone, PartialEq)]
pub enum Batch {
    /// Load or add these library tracks.
    Tracks(Vec<i64>),
    /// Send this command through `/v1/rpc`; LMS keeps the queue filled itself.
    RandomPlay(Vec<String>),
    /// The gateway ran the station's menu action on the player.
    Ran { refill: bool },
    /// Nothing to add (a menu station whose item offers no `add`).
    Nothing,
}

impl crate::Gateway {
    async fn lms(&self, cmd: Vec<String>) -> Option<Value> {
        self.lms_query_retrying("", &cmd, LMS_LIMIT).await.ok()
    }

    async fn titles(&self, start: u64, count: u64, filter: &str) -> (u64, Vec<LmsTrack>) {
        let cmd = vec![
            "titles".into(),
            start.to_string(),
            count.to_string(),
            filter.to_string(),
            "tags:O".into(),
        ];
        match self.lms(cmd).await {
            Some(v) => (
                v.get("count").and_then(Value::as_u64).unwrap_or(0),
                lms_tracks(&v),
            ),
            None => (0, Vec::new()),
        }
    }

    /// Up to `pages` random pages of 50 tracks matching `filter` (e.g. `genre_id:3,4`).
    async fn random_pages(&self, filter: &str, pages: usize, rng: &mut Rng) -> Vec<LmsTrack> {
        let (total, _) = self.titles(0, 1, filter).await;
        if total == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut starts = HashSet::new();
        for _ in 0..pages {
            let start = rng.below(total.saturating_sub(49).max(1));
            if starts.insert(start / 50) {
                out.extend(self.titles(start, 50, filter).await.1);
            }
        }
        out
    }

    /// The next batch of a built-in station's tracks (`Spec::Menu` is handled elsewhere).
    pub(crate) async fn built_in_batch(
        &self,
        user: i64,
        station: i64,
        spec: &Spec,
        count: usize,
        exclude: &[i64],
    ) -> Batch {
        if let Spec::Genre { genres } = spec
            && genres.len() == 1
        {
            return Batch::RandomPlay(vec![
                "randomplay".into(),
                "tracks".into(),
                format!("genre_id:{}", genres[0]),
            ]);
        }
        let mut rng = Rng::new();
        let now = now();
        let plays = history::since(
            self.accounts(),
            user,
            now - history::RETENTION.as_secs() as i64,
            5000,
        )
        .unwrap_or_default();
        let mine = UserTracks::from_history(&plays);
        let mut avoid: HashSet<i64> = exclude.iter().copied().collect();
        avoid.extend(mine.recent.iter().copied());
        avoid.extend(self.station_served.recent((user, station)));
        let mut cands: Vec<(i64, f64)> = Vec::new();
        match spec {
            Spec::Genre { genres } => {
                let list = genres
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let tracks = self
                    .random_pages(&format!("genre_id:{list}"), 3, &mut rng)
                    .await;
                cands.extend(server_side(&tracks, |_| true, false));
            }
            Spec::Decade {
                decade,
                mine: only_mine,
            } => {
                let years: Vec<u32> = (*decade..*decade + 10).collect();
                if *only_mine {
                    for p in &plays {
                        if p.year.is_some_and(|y| years.contains(&(y as u32))) {
                            cands.push((p.track_id, history::USER_WEIGHT));
                        }
                    }
                }
                for _ in 0..3 {
                    let y = years[rng.below(10) as usize];
                    let tracks = self.random_pages(&format!("year:{y}"), 1, &mut rng).await;
                    cands.extend(server_side(&tracks, |_| true, false));
                }
            }
            Spec::Forgotten => {
                cands.extend(forgotten_user(&mine, now, 60));
                let most = self.server_albums("playcount", 100).await;
                let recent: HashSet<i64> = self
                    .server_albums("recentlyplayed", 200)
                    .await
                    .into_iter()
                    .collect();
                let quiet: Vec<i64> = most.into_iter().filter(|a| !recent.contains(a)).collect();
                for album in pick_some(&quiet, 6, &mut rng) {
                    let (_, tracks) = self.titles(0, 100, &format!("album_id:{album}")).await;
                    cands.extend(server_side(&tracks, |t| t.playcount >= 1, true));
                }
            }
            Spec::Fresh => {
                let new = self.server_albums("new", 30).await;
                for album in pick_some(&new, 6, &mut rng) {
                    let (_, tracks) = self.titles(0, 100, &format!("album_id:{album}")).await;
                    cands.extend(server_side(
                        &tracks,
                        |t| t.playcount <= 1 && !mine.plays.contains_key(&t.id),
                        false,
                    ));
                }
            }
            Spec::DeepCuts => {
                let server = server_artists(self).await;
                let artists = top_artists(&mine, &server, 10);
                for artist in pick_some(&artists, 4, &mut rng) {
                    let (_, tracks) = self.titles(0, 200, &format!("artist_id:{artist}")).await;
                    cands.extend(server_side(
                        &tracks,
                        |t| t.playcount <= 1 && !mine.plays.contains_key(&t.id),
                        false,
                    ));
                }
            }
            Spec::Menu(_) => return Batch::Tracks(Vec::new()),
        }
        let picked = weighted_pick(&cands, &avoid, count, &mut rng);
        self.station_served.note((user, station), &picked);
        Batch::Tracks(picked)
    }
}

async fn server_artists(gw: &crate::Gateway) -> Vec<i64> {
    let cmd = vec![
        "artists".into(),
        "0".into(),
        "50".into(),
        "sort:playcount".into(),
        "tags:".into(),
    ];
    gw.lms(cmd)
        .await
        .map(|v| history::ids_in(&v, "artists_loop"))
        .unwrap_or_default()
}

/// Up to `n` of `from`, chosen at random.
fn pick_some(from: &[i64], n: usize, rng: &mut Rng) -> Vec<i64> {
    let cands: Vec<(i64, f64)> = from.iter().map(|id| (*id, 1.0)).collect();
    weighted_pick(&cands, &HashSet::new(), n, rng)
}

// ---------------------------------------------------------------------------
// Replaying a menu reference
// ---------------------------------------------------------------------------

struct Who<'a> {
    principal: &'a Principal,
    scope: &'a PlayerScope,
}

impl Caller for Who<'_> {
    fn role(&self) -> Role {
        self.principal.role
    }
    fn may_control(&self, player: &str) -> bool {
        self.scope.allows(player)
    }
}

/// Why a menu reference can't be used (any more).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replay {
    /// LMS no longer offers it this way (or the user may no longer use it).
    Broken(&'static str),
    /// LMS couldn't be reached: try later; the station is not marked broken.
    Unavailable,
}

/// Menu commands a reference may start from: menus of the fixed allow-list.
fn root_menu(cmd: &[String], apps: &HashSet<String>) -> bool {
    let w = |i: usize| cmd.get(i).map(String::as_str);
    match w(0) {
        Some("menu" | "apps" | "radios") => true,
        Some(
            "trackinfo" | "albuminfo" | "artistinfo" | "genreinfo" | "yearinfo" | "playlistinfo"
            | "folderinfo" | "favorites",
        ) => w(1) == Some("items"),
        Some(app) => apps.contains(app) && w(1) == Some("items"),
        None => false,
    }
}

impl crate::Gateway {
    /// Replay a menu reference's steps for `p` on `player`, exactly as `/v1/rpc` would
    /// filter and check them, and return the last menu (filtered) and the actions it
    /// offered. Nothing in it runs.
    pub(crate) async fn replay_steps(
        &self,
        p: &Principal,
        scope: &PlayerScope,
        player: &str,
        steps: &[Vec<String>],
    ) -> Result<(Value, menu_actions::DeviceActions), Replay> {
        let admin = p.role == Role::Admin;
        let hidden = if admin {
            HashSet::new()
        } else {
            self.accounts().admin_only_plugins()
        };
        let who = Who {
            principal: p,
            scope,
        };
        let mut offered = menu_actions::DeviceActions::default();
        let mut last = Value::Null;
        for (i, step) in steps.iter().enumerate() {
            if hidden.contains(&step[0]) {
                return Err(Replay::Broken("this plugin is for admins only"));
            }
            let target = if i == 0 {
                let raw: Vec<Value> = step.iter().map(|s| Value::String(s.clone())).collect();
                let mut apps = self.apps();
                // Throttled as for /v1/rpc, so replaying stations can't hammer LMS.
                if !root_menu(step, &apps) && !apps.contains(&step[0]) && self.may_refresh_apps() {
                    self.refresh_apps().await;
                    apps = self.apps();
                }
                if !root_menu(step, &apps) {
                    return Err(Replay::Broken("this menu is no longer offered"));
                }
                // A player's menu, or a server's (apps, radios).
                let checked = rpc_policy::check(&who, player, &raw, &apps)
                    .or_else(|_| rpc_policy::check(&who, "", &raw, &apps))
                    .map_err(|_| Replay::Broken("this menu is no longer offered"))?;
                (checked.player, checked.kind)
            } else {
                menu_actions::allowed(&offered, step, admin)
                    .map_err(|_| Replay::Broken("this menu item is no longer offered"))?;
                (player.to_string(), Kind::Plain)
            };
            if crate::art::first_resolving_local(step, &self.cfg.host_answers)
                .await
                .is_some()
            {
                return Err(Replay::Broken(LOCAL_LINK));
            }
            let result = self
                .lms_command(&target.0, step)
                .await
                .map_err(|_| Replay::Unavailable)?;
            if target.1 == Kind::AppMenu {
                self.add_apps(rpc_policy::learn_apps(&result));
            }
            let mut filtered = rpc_policy::filter_response(target.1, result, &|x| scope.allows(x));
            rpc_policy::hide_plugins(&mut filtered, &hidden);
            offered = menu_actions::DeviceActions::default();
            offered.add(menu_actions::learn(&filtered), Instant::now());
            last = filtered;
        }
        Ok((last, offered))
    }

    /// Run a station's menu action (`play` to start, else `add`) on `player`, after
    /// replaying its steps. Marks the station broken if it is no longer offered.
    pub(crate) async fn run_menu_station(
        &self,
        p: &Principal,
        scope: &PlayerScope,
        player: &str,
        station: i64,
        m: &MenuRef,
        start: bool,
    ) -> Result<Batch, Replay> {
        let action = if start {
            m.play.as_ref()
        } else {
            m.add.as_ref()
        };
        let Some(action) = action else {
            return Ok(Batch::Nothing);
        };
        let result = self.check_menu_ref(p, scope, player, m).await;
        match &result {
            Err(Replay::Broken(why)) => {
                let _ = set_broken(self.accounts(), p.user_id, station, Some(why));
                return Err(Replay::Broken(why));
            }
            Err(Replay::Unavailable) => return Err(Replay::Unavailable),
            Ok(()) => {}
        }
        let _ = set_broken(self.accounts(), p.user_id, station, None);
        self.lms_command(player, action)
            .await
            .map_err(|_| Replay::Unavailable)?;
        Ok(Batch::Ran {
            refill: m.add.is_some(),
        })
    }

    /// Is this reference's every step, and its `play`/`add`, offered to `p` right now?
    pub(crate) async fn check_menu_ref(
        &self,
        p: &Principal,
        scope: &PlayerScope,
        player: &str,
        m: &MenuRef,
    ) -> Result<(), Replay> {
        let (_, offered) = self.replay_steps(p, scope, player, &m.steps).await?;
        for action in m.play.iter().chain(m.add.iter()) {
            if self.accounts().admin_only_plugins().contains(&action[0]) && p.role != Role::Admin {
                return Err(Replay::Broken("this plugin is for admins only"));
            }
            menu_actions::allowed(&offered, action, p.role == Role::Admin)
                .map_err(|_| Replay::Broken("this menu item is no longer offered"))?;
            if crate::art::first_resolving_local(action, &self.cfg.host_answers)
                .await
                .is_some()
            {
                return Err(Replay::Broken(LOCAL_LINK));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn pin_json(p: &Pin, kind: PinKind) -> Value {
    let mut o = json!({"id": p.id, "name": p.name, "broken": p.broken});
    match kind {
        PinKind::Station => {
            let spec = parse_spec(&p.spec).ok();
            o["kind"] = json!(spec.as_ref().map(Spec::kind));
            o["spec"] = spec.as_ref().map_or(Value::Null, Spec::to_json);
        }
        PinKind::Menu => {
            o["steps"] = p.spec.get("steps").cloned().unwrap_or(Value::Null);
        }
    }
    o
}

/// The player named in a request body: one the caller may control.
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
fn player_of(o: &Map<String, Value>, scope: &PlayerScope) -> Result<String, Response> {
    let player = o
        .get("player")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "player missing"))?;
    if !rpc_policy::is_player_id(&player) || !scope.allows(&player) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "you may not control this player",
        ));
    }
    Ok(player)
}

/// `GET /v1/me/stations`
pub async fn list_stations(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if seed(gw.accounts(), p.user_id).is_err() {
        return internal();
    }
    match list(gw.accounts(), p.user_id, PinKind::Station) {
        Ok(pins) => ok(
            json!({"stations": pins.iter().map(|x| pin_json(x, PinKind::Station)).collect::<Vec<_>>()}),
        ),
        Err(_) => internal(),
    }
}

/// `POST /v1/me/stations` `{"name", "spec", "player"?}` (a menu station is checked on
/// `player` before it is saved).
pub async fn create_station(
    State(gw): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let v = match body_json(&headers, body, MAX_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(o) = v.as_object() else {
        return error(StatusCode::BAD_REQUEST, "expected an object");
    };
    if o.keys()
        .any(|k| !["name", "spec", "player"].contains(&k.as_str()))
    {
        return error(StatusCode::BAD_REQUEST, "unexpected field");
    }
    let name = match parse_name(o.get("name")) {
        Ok(n) => n,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let spec = match o.get("spec").map(parse_spec) {
        Some(Ok(s)) => s,
        Some(Err(e)) => return error(StatusCode::BAD_REQUEST, e),
        None => return error(StatusCode::BAD_REQUEST, "spec missing"),
    };
    if let Spec::Menu(m) = &spec {
        let Ok(scope) = gw.accounts().player_scope(&p) else {
            return internal();
        };
        let player = match player_of(o, &scope) {
            Ok(x) => x,
            Err(r) => return r,
        };
        match gw.check_menu_ref(&p, &scope, &player, m).await {
            Ok(()) => {}
            Err(Replay::Broken(why)) => return error(StatusCode::UNPROCESSABLE_ENTITY, why),
            Err(Replay::Unavailable) => {
                return error(StatusCode::BAD_GATEWAY, "music server unavailable");
            }
        }
    }
    if seed(gw.accounts(), p.user_id).is_err() {
        return internal();
    }
    match add(
        gw.accounts(),
        p.user_id,
        PinKind::Station,
        &name,
        &spec.to_json(),
    ) {
        Ok(id) => {
            let mut r = ok(
                json!({"id": id, "name": name, "kind": spec.kind(), "spec": spec.to_json(), "broken": null}),
            );
            *r.status_mut() = StatusCode::CREATED;
            r
        }
        Err(AddError::Full) => error(StatusCode::CONFLICT, "too many stations and pinned menus"),
        Err(AddError::Store) => internal(),
    }
}

/// `DELETE /v1/me/stations/{id}`
pub async fn delete_station(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    delete_pin(gw, headers, id, PinKind::Station).await
}

async fn delete_pin(gw: AppState, headers: HeaderMap, id: String, kind: PinKind) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(id) = parse_id(&id) else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match remove(gw.accounts(), p.user_id, kind, id) {
        Ok(true) => ok(json!({"removed": true})),
        Ok(false) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => internal(),
    }
}

/// `POST /v1/me/stations/{id}/next` `{"player", "start"?, "count"?, "exclude"?}`
pub async fn next(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Body,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(id) = parse_id(&id) else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    let v = match body_json(&headers, body, MAX_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(o) = v.as_object() else {
        return error(StatusCode::BAD_REQUEST, "expected an object");
    };
    if o.keys()
        .any(|k| !["player", "start", "count", "exclude"].contains(&k.as_str()))
    {
        return error(StatusCode::BAD_REQUEST, "unexpected field");
    }
    let start = match o.get("start") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return error(StatusCode::BAD_REQUEST, "start must be true or false"),
    };
    let count = match o.get("count") {
        None => 20,
        Some(c) => match c.as_u64().filter(|_| c.is_u64()) {
            Some(n) if (1..=MAX_BATCH as u64).contains(&n) => n as usize,
            _ => return error(StatusCode::BAD_REQUEST, "count must be 1 to 50"),
        },
    };
    let exclude: Vec<i64> = match o.get("exclude") {
        None => Vec::new(),
        Some(Value::Array(a)) if a.len() <= MAX_EXCLUDE => {
            match a.iter().map(positive_id).collect::<Option<Vec<u64>>>() {
                Some(ids) => ids.into_iter().map(|n| n as i64).collect(),
                None => return error(StatusCode::BAD_REQUEST, "bad track id in exclude"),
            }
        }
        Some(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "exclude must be at most 200 track ids",
            );
        }
    };
    let Ok(scope) = gw.accounts().player_scope(&p) else {
        return internal();
    };
    let player = match player_of(o, &scope) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let pin = match get(gw.accounts(), p.user_id, PinKind::Station, id) {
        Ok(Some(pin)) => pin,
        Ok(None) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return internal(),
    };
    let Ok(spec) = parse_spec(&pin.spec) else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    if !gw.station_batches.take(p.user_id, Instant::now()) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many station requests; try again shortly",
        );
    }
    let batch = match &spec {
        Spec::Menu(m) => match gw.run_menu_station(&p, &scope, &player, id, m, start).await {
            Ok(b) => b,
            Err(Replay::Broken(why)) => {
                return {
                    let mut r = ok(json!({"error": why, "broken": true}));
                    *r.status_mut() = StatusCode::CONFLICT;
                    r
                };
            }
            Err(Replay::Unavailable) => {
                return error(StatusCode::BAD_GATEWAY, "music server unavailable");
            }
        },
        built_in => {
            gw.built_in_batch(p.user_id, id, built_in, count, &exclude)
                .await
        }
    };
    ok(match batch {
        Batch::Tracks(t) => json!({"mode": "tracks", "tracks": t, "refill": true}),
        Batch::RandomPlay(c) => json!({"mode": "randomplay", "command": c, "refill": false}),
        Batch::Ran { refill } => json!({"mode": "ran", "refill": refill}),
        Batch::Nothing => json!({"mode": "none", "refill": false}),
    })
}

/// `GET /v1/me/menus`
pub async fn list_menus(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match list(gw.accounts(), p.user_id, PinKind::Menu) {
        Ok(pins) => ok(
            json!({"menus": pins.iter().map(|x| pin_json(x, PinKind::Menu)).collect::<Vec<_>>()}),
        ),
        Err(_) => internal(),
    }
}

/// `POST /v1/me/menus` `{"name", "steps", "player"}`: checked on `player` first.
pub async fn create_menu(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let v = match body_json(&headers, body, MAX_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(o) = v.as_object() else {
        return error(StatusCode::BAD_REQUEST, "expected an object");
    };
    if o.keys()
        .any(|k| !["name", "steps", "player"].contains(&k.as_str()))
    {
        return error(StatusCode::BAD_REQUEST, "unexpected field");
    }
    let name = match parse_name(o.get("name")) {
        Ok(n) => n,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let menu = match parse_menu(&json!({"steps": o.get("steps").cloned().unwrap_or(Value::Null)})) {
        Ok(m) => m,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let Ok(scope) = gw.accounts().player_scope(&p) else {
        return internal();
    };
    let player = match player_of(o, &scope) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match gw.replay_steps(&p, &scope, &player, &menu.steps).await {
        Ok(_) => {}
        Err(Replay::Broken(why)) => return error(StatusCode::UNPROCESSABLE_ENTITY, why),
        Err(Replay::Unavailable) => {
            return error(StatusCode::BAD_GATEWAY, "music server unavailable");
        }
    }
    let spec = json!({"steps": menu.steps});
    match add(gw.accounts(), p.user_id, PinKind::Menu, &name, &spec) {
        Ok(id) => {
            let mut r = ok(json!({"id": id, "name": name, "steps": menu.steps, "broken": null}));
            *r.status_mut() = StatusCode::CREATED;
            r
        }
        Err(AddError::Full) => error(StatusCode::CONFLICT, "too many stations and pinned menus"),
        Err(AddError::Store) => internal(),
    }
}

/// `DELETE /v1/me/menus/{id}`
pub async fn delete_menu(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    delete_pin(gw, headers, id, PinKind::Menu).await
}

/// `POST /v1/me/menus/{id}/open` `{"player"}`: the pinned menu, as `/v1/rpc` would answer
/// its last step. The device may then run what it offers.
pub async fn open_menu(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Body,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(id) = parse_id(&id) else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    let v = match body_json(&headers, body, 512).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(o) = v.as_object().filter(|o| o.keys().all(|k| k == "player")) else {
        return error(StatusCode::BAD_REQUEST, "expected {\"player\": …}");
    };
    let Ok(scope) = gw.accounts().player_scope(&p) else {
        return internal();
    };
    let player = match player_of(o, &scope) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let pin = match get(gw.accounts(), p.user_id, PinKind::Menu, id) {
        Ok(Some(pin)) => pin,
        Ok(None) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return internal(),
    };
    let Ok(menu) = parse_menu(&pin.spec) else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match gw.replay_steps(&p, &scope, &player, &menu.steps).await {
        Ok((result, offered)) => {
            let _ = set_broken(gw.accounts(), p.user_id, id, None);
            gw.remember_menu_actions(p.device_id, offered.into_actions());
            ok(json!({"result": result}))
        }
        Err(Replay::Broken(why)) => {
            let _ = set_broken(gw.accounts(), p.user_id, id, Some(why));
            let mut r = ok(json!({"error": why, "broken": true}));
            *r.status_mut() = StatusCode::CONFLICT;
            r
        }
        Err(Replay::Unavailable) => error(StatusCode::BAD_GATEWAY, "music server unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn built_in_specs_parse_strictly() {
        assert_eq!(parse_spec(&json!({"kind": "fresh"})), Ok(Spec::Fresh));
        assert_eq!(
            parse_spec(&json!({"kind": "forgotten"})),
            Ok(Spec::Forgotten)
        );
        assert_eq!(
            parse_spec(&json!({"kind": "deep_cuts"})),
            Ok(Spec::DeepCuts)
        );
        assert_eq!(
            parse_spec(&json!({"kind": "genre", "genres": [4, 3]})),
            Ok(Spec::Genre { genres: vec![3, 4] })
        );
        assert_eq!(
            parse_spec(&json!({"kind": "decade", "decade": 1980, "mine": true})),
            Ok(Spec::Decade {
                decade: 1980,
                mine: true
            })
        );
        for bad in [
            json!(null),
            json!([]),
            json!("fresh"),
            json!({}),
            json!({"kind": "Fresh"}),
            json!({"kind": "fresh", "extra": 1}),
            json!({"kind": "sql"}),
            json!({"kind": "genre"}),
            json!({"kind": "genre", "genres": []}),
            json!({"kind": "genre", "genres": [1, 2, 3, 4, 5, 6]}),
            json!({"kind": "genre", "genres": [0]}),
            json!({"kind": "genre", "genres": [-1]}),
            json!({"kind": "genre", "genres": ["3"]}),
            json!({"kind": "genre", "genres": [1.5]}),
            json!({"kind": "genre", "genres": [3, 3]}),
            json!({"kind": "genre", "genres": [1_000_000_000_000u64]}),
            json!({"kind": "decade", "decade": 1985}),
            json!({"kind": "decade", "decade": 1890}),
            json!({"kind": "decade", "decade": 2100}),
            json!({"kind": "decade", "decade": "1980"}),
            json!({"kind": "decade", "decade": 1980, "mine": 1}),
            json!({"kind": "decade"}),
        ] {
            assert!(parse_spec(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn menu_references_hold_offered_shapes_only() {
        let good = json!({"kind": "menu",
            "steps": [["trackinfo", "items", 0, 100, "track_id:4242", "menu:1"]],
            "play": ["audiomuse", "similar_track", "track_id:4242"]});
        let Spec::Menu(m) = parse_spec(&good).unwrap() else {
            panic!()
        };
        assert_eq!(
            m.steps[0],
            s(&["trackinfo", "items", "0", "100", "track_id:4242", "menu:1"])
        );
        assert_eq!(m.add, None);
        let with = |k: &str, v: Value| {
            let mut g = good.clone();
            g[k] = v;
            g
        };
        for bad in [
            with("play", Value::Null),
            with("play", json!("audiomuse similar_track")),
            with("play", json!([])),
            // A menu (paged) is not an action.
            with("play", json!(["audiomuse", "items", 0, 10])),
            with(
                "play",
                json!(["audiomuse", "similar", "url:file:///srv/music/x.flac"]),
            ),
            with("play", json!(["audiomuse", "similar", "url:x"])),
            with("play", json!(["x;rm", "y"])),
            with("play", json!(["a", "b", "c", "d", "e", "f", "g"])),
            with(
                "play",
                json!(["audiomuse", "instant", "prompt:__TAGGEDINPUT__"]),
            ),
            with(
                "play",
                json!(["audiomuse", "similar", "id:http://10.0.0.5/x"]),
            ),
            with("play", json!(["audiomuse", "similar", {"a": 1}])),
            with("play", json!(["audiomuse", "similar", -1])),
            with("play", json!(["audiomuse", "similar", "a:\u{0}"])),
            with("add", json!(["audiomuse", "items", 0, 10])),
            with("steps", json!([])),
            with("steps", json!("trackinfo")),
            with("steps", json!([["trackinfo", "items", "track_id:1"]])),
            with("steps", json!([["m", "items", 0, 9999]])),
            with(
                "steps",
                json!([
                    ["m", "items", 0, 1],
                    ["m", "items", 0, 1],
                    ["m", "items", 0, 1],
                    ["m", "items", 0, 1],
                    ["m", "items", 0, 1]
                ]),
            ),
            with("steps", json!([["m", "items", 0, 10, "k:1", "k:2"]])),
            with("cmd", json!(["play"])),
        ] {
            assert!(parse_spec(&bad).is_err(), "accepted {bad}");
        }
        assert!(parse_menu(&json!({"steps": [["menu", 0, 100, "direct:1"]]})).is_ok());
        assert!(parse_menu(&json!({"steps": [["menu", 0, 100]], "play": ["x"]})).is_err());
        assert!(parse_menu(&json!({"steps": []})).is_err());
    }

    #[test]
    fn names_are_cleaned_and_bounded() {
        assert_eq!(
            parse_name(Some(&json!("  Late   night  "))),
            Ok("Late night".into())
        );
        for bad in [
            json!(""),
            json!("   "),
            json!(5),
            Value::Null,
            json!("x".repeat(65)),
            json!("\u{202e}"),
        ] {
            assert!(parse_name(Some(&bad)).is_err(), "accepted {bad}");
        }
        assert!(parse_name(None).is_err());
        let markup = parse_name(Some(&json!("<b>Mix</b>"))).unwrap();
        assert!(!markup.contains('<'));
    }

    #[test]
    fn ids_in_paths_are_plain_positive_numbers() {
        assert_eq!(parse_id("1"), Some(1));
        assert_eq!(parse_id("123456789012345"), Some(123_456_789_012_345));
        for bad in [
            "",
            "0",
            "01",
            "-1",
            "+1",
            "1.0",
            "1e3",
            "abc",
            "1 ",
            "%31",
            "1234567890123456",
            "../1",
        ] {
            assert_eq!(parse_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn weighted_picks_are_distinct_and_respect_exclusions() {
        let mut rng = Rng::seeded(7);
        let cands = vec![
            (1, 1.0),
            (2, 1.0),
            (3, 1.0),
            (3, 5.0),
            (4, 0.0),
            (5, f64::NAN),
            (-6, 1.0),
            (7, f64::INFINITY),
        ];
        let ex: HashSet<i64> = [2].into();
        for _ in 0..100 {
            let mut p = weighted_pick(&cands, &ex, 10, &mut rng);
            p.sort();
            assert_eq!(p, vec![1, 3]);
        }
        assert!(weighted_pick(&[], &ex, 5, &mut rng).is_empty());
        assert_eq!(weighted_pick(&cands, &ex, 1, &mut rng).len(), 1);
        // Heavier ids come first far more often.
        let mut first = HashMap::new();
        for _ in 0..2000 {
            let p = weighted_pick(&[(1, 9.0), (2, 1.0)], &HashSet::new(), 1, &mut rng);
            *first.entry(p[0]).or_insert(0) += 1;
        }
        assert!(first[&1] > first[&2] * 4, "{first:?}");
    }

    fn entry(track: i64, artist: i64, at: i64) -> history::Entry {
        history::Entry {
            id: at,
            track_key: "k".into(),
            track_id: track,
            album_id: Some(1),
            artist_id: Some(artist),
            year: Some(1984),
            played_at: at,
            seconds: 200.0,
            source: "live".into(),
        }
    }

    #[test]
    fn forgotten_favourites_are_played_often_but_not_lately() {
        let now = 1_800_000_000;
        let old = now - 100 * 86_400;
        let plays = vec![
            entry(1, 9, old),
            entry(1, 9, old - 10),
            entry(1, 9, old - 20),
            entry(2, 9, old),
            entry(2, 9, old - 5),
            entry(3, 9, old), // once only
            entry(4, 9, now - 86_400),
            entry(4, 9, old), // played lately
        ];
        let u = UserTracks::from_history(&plays);
        let f = forgotten_user(&u, now, 60);
        assert_eq!(f.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(f[0].1, history::USER_WEIGHT);
        assert_eq!(f[1].1, history::USER_WEIGHT * 2.0 / 3.0);
        assert_eq!(u.recent.len(), plays.len());
        assert_eq!(u.artists[&9], 8);
    }

    #[test]
    fn server_side_weights_never_outweigh_the_users() {
        let tracks = [
            LmsTrack {
                id: 1,
                playcount: 10,
            },
            LmsTrack {
                id: 2,
                playcount: 0,
            },
            LmsTrack {
                id: 3,
                playcount: 5,
            },
        ];
        let all = server_side(&tracks, |_| true, true);
        assert!(
            all.iter()
                .all(|(_, w)| *w <= history::SERVER_WEIGHT && *w > 0.0)
        );
        let fresh = server_side(&tracks, |t| t.playcount <= 1, false);
        assert_eq!(fresh, vec![(2, history::SERVER_WEIGHT)]);
    }

    #[test]
    fn deep_cut_artists_blend_user_and_server() {
        let plays: Vec<history::Entry> = (0..10)
            .map(|i| entry(i, 5, i))
            .chain((0..2).map(|i| entry(i, 6, i)))
            .collect();
        let u = UserTracks::from_history(&plays);
        assert_eq!(top_artists(&u, &[], 5), vec![5, 6]);
        // LMS's top artist (7) lands behind the user's clear favourite.
        assert_eq!(top_artists(&u, &[7, 6], 5), vec![5, 6, 7]);
        assert_eq!(
            top_artists(&UserTracks::default(), &[8, -1, 9], 5),
            vec![8, 9]
        );
    }

    #[test]
    fn lms_track_lists_are_read_strictly() {
        let v = json!({"titles_loop": [{"id": 1, "playcount": 3}, {"id": "2", "playcount": "x"}, {"id": -3}, {"title": "no id"}, {"id": 4, "playcount": -2}]});
        assert_eq!(
            lms_tracks(&v),
            vec![
                LmsTrack {
                    id: 1,
                    playcount: 3
                },
                LmsTrack {
                    id: 2,
                    playcount: 0
                },
                LmsTrack {
                    id: 4,
                    playcount: 0
                }
            ]
        );
        assert!(lms_tracks(&json!({"titles_loop": 5})).is_empty());
    }

    #[test]
    fn roots_are_menus_of_the_fixed_allow_list() {
        let apps: HashSet<String> = ["audiomuse".to_string()].into();
        assert!(root_menu(&s(&["menu", "0", "100"]), &apps));
        assert!(root_menu(
            &s(&["trackinfo", "items", "0", "100", "track_id:1"]),
            &apps
        ));
        assert!(root_menu(&s(&["audiomuse", "items", "0", "100"]), &apps));
        for bad in [
            s(&["trackinfo", "0", "100"]),
            s(&["someplugin", "items", "0", "100"]),
            s(&["playlist", "play", "x"]),
            s(&["status", "0", "1"]),
            s(&["randomplay", "tracks"]),
            vec![],
        ] {
            assert!(!root_menu(&bad, &apps), "{bad:?}");
        }
    }

    /// Robustness: arbitrary JSON never panics the spec and menu parsers, and whatever
    /// they accept obeys their limits.
    #[test]
    fn fuzz_the_spec_parsers() {
        let mut rng = Rng::seeded(0x5eed);
        let atoms = [
            json!(null),
            json!(true),
            json!(0),
            json!(1),
            json!(-1),
            json!(1980),
            json!(1.5),
            json!("menu"),
            json!("genre"),
            json!("decade"),
            json!("fresh"),
            json!("items"),
            json!("track_id:1"),
            json!("url:file:///x"),
            json!("__TAGGEDINPUT__"),
            json!("x".repeat(300)),
            json!([]),
            json!({}),
        ];
        let keys = [
            "kind", "genres", "decade", "mine", "steps", "play", "add", "x",
        ];
        fn pick<'a, T>(rng: &mut Rng, v: &'a [T]) -> &'a T {
            &v[rng.below(v.len() as u64) as usize]
        }
        for _ in 0..20_000 {
            let mut o = Map::new();
            for _ in 0..rng.below(5) {
                let k = pick(&mut rng, &keys).to_string();
                let v = match rng.below(4) {
                    0 => pick(&mut rng, &atoms).clone(),
                    1 => Value::Array(
                        (0..rng.below(6))
                            .map(|_| pick(&mut rng, &atoms).clone())
                            .collect(),
                    ),
                    2 => Value::Array(
                        (0..rng.below(6))
                            .map(|_| {
                                Value::Array(
                                    (0..rng.below(8))
                                        .map(|_| pick(&mut rng, &atoms).clone())
                                        .collect(),
                                )
                            })
                            .collect(),
                    ),
                    _ => json!(pick(
                        &mut rng,
                        &["menu", "genre", "decade", "fresh", "deep_cuts", "forgotten"]
                    )),
                };
                o.insert(k, v);
            }
            let v = Value::Object(o);
            if let Ok(spec) = parse_spec(&v) {
                // Whatever is accepted reads back the same.
                assert_eq!(parse_spec(&spec.to_json()), Ok(spec.clone()));
                if let Spec::Menu(m) = &spec {
                    assert!((1..=MAX_STEPS).contains(&m.steps.len()));
                    assert!(m.play.is_some());
                }
                if let Spec::Genre { genres } = &spec {
                    assert!((1..=MAX_GENRES).contains(&genres.len()));
                }
            }
            let _ = parse_menu(&v);
        }
        // Raw bytes through the JSON reader too.
        for _ in 0..5000 {
            let bytes: Vec<u8> = (0..rng.below(80)).map(|_| rng.next_u64() as u8).collect();
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                let _ = parse_spec(&v);
                let _ = parse_menu(&v);
            }
        }
    }
}
