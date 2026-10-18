//! Smart filters, v1 (docs/plans/wave2-design.md §5.6): saved rules that produce albums.
//!
//! Each user keeps their own filters on the gateway (`user_filters`), so they are the same
//! on every device and never shown to anyone else. A filter is
//!
//! ```json
//! {"id": 3, "name": "Late jazz", "rules": [{"field": "genre", "in": [4, 9]},
//!   {"field": "decade", "is": 1960}], "sort": "yearalbum", "limit": 200}
//! ```
//!
//! and every rule must match. Running one (`GET /v1/me/filters/{id}/albums`) translates
//! its rules into LMS `albums` queries built here from the parsed rules only, never from
//! client text: `genre_id`, `artist_id` (with an optional `role_id`), `compilation` and
//! `year`. A decade is one query per year, merged, de-duplicated and sorted again; each
//! query is also checked against the `/v1/rpc` allow-list before it is sent. Results are
//! kept for a short while so later pages (and a random order) stay the same; the first
//! page always runs the filter afresh.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::accounts::{Accounts, AuthError, Role};
use crate::history::{body_json, caller, error, ok};
use crate::rpc_policy::{self, Caller};
use crate::stations::{Rng, parse_id, parse_name};

/// Largest filter body.
pub const MAX_BODY: usize = 4096;
/// Filters one user may keep.
pub const MAX_FILTERS_PER_USER: usize = 50;
/// Rules in one filter.
pub const MAX_RULES: usize = 8;
/// Genres in one genre rule.
pub const MAX_GENRES: usize = 20;
/// Albums a filter produces, at most.
pub const MAX_LIMIT: u32 = 500;
/// Albums in one page of results.
pub const MAX_PAGE: u32 = 100;
/// Largest LMS id a rule may name (genre and artist ids are small; this keeps a list of
/// 20 genres within the allow-list's 200 characters).
pub const MAX_RULE_ID: u64 = 999_999_999;
/// Filter runs (that ask LMS) a user may make a minute; pages served from a kept result
/// don't count.
pub const RUNS_PER_MINUTE: usize = 30;
/// Filter runs in flight: per user, and for everyone.
pub const RUNS_PER_USER: usize = 2;
pub const RUNS_TOTAL: usize = 8;
/// How long a result is kept for later pages.
pub const KEEP_RESULTS: Duration = Duration::from_secs(120);
/// Results kept, for everyone.
const MAX_KEPT: usize = 256;
/// Each LMS query's time limit.
const LMS_LIMIT: Duration = Duration::from_secs(10);

/// LMS's contributor roles (`role_id:`), as the allow-list takes them.
pub const ROLES: [&str; 6] = [
    "ALBUMARTIST",
    "ARTIST",
    "COMPOSER",
    "CONDUCTOR",
    "BAND",
    "TRACKARTIST",
];

// ---------------------------------------------------------------------------
// Specs (pure)
// ---------------------------------------------------------------------------

/// One rule. Every rule of a filter must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// The album has any of these genres (sorted, distinct).
    Genre(Vec<u64>),
    /// The album's year is in this decade (1900, 1910, … 2090).
    Decade(u32),
    /// By this artist, in a role if given.
    Artist { id: u64, role: Option<&'static str> },
    /// It is (or isn't) a compilation.
    Compilation(bool),
}

impl Rule {
    pub fn field(&self) -> &'static str {
        match self {
            Rule::Genre(_) => "genre",
            Rule::Decade(_) => "decade",
            Rule::Artist { .. } => "artist",
            Rule::Compilation(_) => "compilation",
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Rule::Genre(ids) => json!({"field": "genre", "in": ids}),
            Rule::Decade(d) => json!({"field": "decade", "is": d}),
            Rule::Artist { id, role: None } => json!({"field": "artist", "is": id}),
            Rule::Artist { id, role: Some(r) } => {
                json!({"field": "artist", "is": id, "role": r})
            }
            Rule::Compilation(b) => json!({"field": "compilation", "is": b}),
        }
    }
}

/// How results are ordered (LMS's own album sorts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// A–Z by album title.
    Album,
    /// By artist, then album.
    ArtistAlbum,
    /// By year, then album.
    YearAlbum,
    /// Recently added first.
    New,
    /// Shuffled.
    Random,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Sort> {
        Some(match s {
            "album" => Sort::Album,
            "artistalbum" => Sort::ArtistAlbum,
            "yearalbum" => Sort::YearAlbum,
            "new" => Sort::New,
            "random" => Sort::Random,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Album => "album",
            Sort::ArtistAlbum => "artistalbum",
            Sort::YearAlbum => "yearalbum",
            Sort::New => "new",
            Sort::Random => "random",
        }
    }
}

/// What a filter selects (everything but its name and id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub rules: Vec<Rule>,
    pub sort: Sort,
    pub limit: u32,
}

impl Spec {
    pub fn to_json(&self) -> Value {
        json!({
            "rules": self.rules.iter().map(Rule::to_json).collect::<Vec<_>>(),
            "sort": self.sort.as_str(),
            "limit": self.limit,
        })
    }
}

/// A filter as sent: its name and what it selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub spec: Spec,
}

/// Why a filter was refused.
pub type Invalid = &'static str;

fn only_keys(o: &Map<String, Value>, allowed: &[&str]) -> Result<(), Invalid> {
    if o.keys().all(|k| allowed.contains(&k.as_str())) {
        Ok(())
    } else {
        Err("unexpected field")
    }
}

/// A JSON integer (not a float or a string) within `range`.
fn uint_in(v: Option<&Value>, range: std::ops::RangeInclusive<u64>) -> Option<u64> {
    v.filter(|v| v.is_u64())
        .and_then(Value::as_u64)
        .filter(|n| range.contains(n))
}

fn parse_rule(v: &Value) -> Result<Rule, Invalid> {
    let o = v.as_object().ok_or("each rule is an object")?;
    let field = o
        .get("field")
        .and_then(Value::as_str)
        .ok_or("rule field missing")?;
    match field {
        "genre" => {
            only_keys(o, &["field", "in"])?;
            let list = o
                .get("in")
                .and_then(Value::as_array)
                .ok_or("genre needs a list of genre ids")?;
            if list.is_empty() || list.len() > MAX_GENRES {
                return Err("between 1 and 20 genres");
            }
            let mut ids: Vec<u64> = list
                .iter()
                .map(|g| uint_in(Some(g), 1..=MAX_RULE_ID))
                .collect::<Option<_>>()
                .ok_or("bad genre id")?;
            ids.sort_unstable();
            ids.dedup();
            if ids.len() != list.len() {
                return Err("repeated genre");
            }
            Ok(Rule::Genre(ids))
        }
        "decade" => {
            only_keys(o, &["field", "is"])?;
            let d = uint_in(o.get("is"), 1900..=2090)
                .filter(|d| d % 10 == 0)
                .ok_or("decade must be a year like 1980, from 1900 to 2090")?;
            Ok(Rule::Decade(d as u32))
        }
        "artist" => {
            only_keys(o, &["field", "is", "role"])?;
            let id = uint_in(o.get("is"), 1..=MAX_RULE_ID).ok_or("bad artist id")?;
            let role = match o.get("role") {
                None => None,
                Some(r) => Some(
                    r.as_str()
                        .and_then(|r| ROLES.iter().find(|x| **x == r).copied())
                        .ok_or("unknown role")?,
                ),
            };
            Ok(Rule::Artist { id, role })
        }
        "compilation" => {
            only_keys(o, &["field", "is"])?;
            match o.get("is") {
                Some(Value::Bool(b)) => Ok(Rule::Compilation(*b)),
                _ => Err("compilation must be true or false"),
            }
        }
        _ => Err("unknown rule field"),
    }
}

/// Read a filter's rules, sort and limit (as sent or as stored): a strict, closed set of
/// shapes. `extra` names further keys the object may hold (checked by the caller).
fn parse_spec_in(o: &Map<String, Value>, extra: &[&str]) -> Result<Spec, Invalid> {
    let allowed: Vec<&str> = ["rules", "sort", "limit"]
        .iter()
        .chain(extra)
        .copied()
        .collect();
    only_keys(o, &allowed)?;
    let list = o
        .get("rules")
        .and_then(Value::as_array)
        .ok_or("rules must be a list")?;
    if list.is_empty() || list.len() > MAX_RULES {
        return Err("between 1 and 8 rules");
    }
    let rules: Vec<Rule> = list.iter().map(parse_rule).collect::<Result<_, _>>()?;
    let mut seen = HashSet::new();
    if !rules.iter().all(|r| seen.insert(r.field())) {
        return Err("each field may be used once");
    }
    let sort = o
        .get("sort")
        .and_then(Value::as_str)
        .and_then(Sort::parse)
        .ok_or("sort must be album, artistalbum, yearalbum, new or random")?;
    let limit = uint_in(o.get("limit"), 1..=MAX_LIMIT as u64).ok_or("limit must be 1 to 500")?;
    Ok(Spec {
        rules,
        sort,
        limit: limit as u32,
    })
}

/// A stored spec (`{"rules", "sort", "limit"}`).
pub fn parse_spec(v: &Value) -> Result<Spec, Invalid> {
    parse_spec_in(v.as_object().ok_or("a filter is an object")?, &[])
}

/// A filter as a client sends it: `{"name", "rules", "sort", "limit"}`, and an `id` only
/// where `id_ok` allows it (`PUT` may send the filter back as it was read).
pub fn parse_draft(v: &Value, id_ok: Option<i64>) -> Result<Draft, Invalid> {
    let o = v.as_object().ok_or("a filter is an object")?;
    if let Some(id) = o.get("id") {
        match id_ok {
            Some(want) if id.as_i64() == Some(want) && id.is_i64() => {}
            Some(_) => return Err("id doesn't match"),
            None => return Err("unexpected field"),
        }
    }
    let spec = parse_spec_in(o, &["name", "id"])?;
    let name = parse_name(o.get("name"))?;
    Ok(Draft { name, spec })
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// A saved filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub id: i64,
    pub name: String,
    pub spec: Spec,
}

impl Filter {
    pub fn to_json(&self) -> Value {
        let mut v = self.spec.to_json();
        v["id"] = self.id.into();
        v["name"] = self.name.clone().into();
        v
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn row_filter((id, name, spec): (i64, String, String)) -> Option<Filter> {
    let v: Value = serde_json::from_str(&spec).ok()?;
    Some(Filter {
        id,
        name,
        spec: parse_spec(&v).ok()?,
    })
}

/// A user's filters, oldest first. Anything stored that no longer parses is left out.
pub fn list(a: &Accounts, user: i64) -> Result<Vec<Filter>, AuthError> {
    let rows: Vec<(i64, String, String)> = a.read_db(|db| {
        let mut st =
            db.prepare("SELECT id, name, spec FROM user_filters WHERE user_id = ?1 ORDER BY id")?;
        let rows = st.query_map([user], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    })?;
    Ok(rows.into_iter().filter_map(row_filter).collect())
}

/// One of the user's own filters (never another user's).
pub fn get(a: &Accounts, user: i64, id: i64) -> Result<Option<Filter>, AuthError> {
    let row: Option<(i64, String, String)> = a.read_db(|db| {
        db.query_row(
            "SELECT id, name, spec FROM user_filters WHERE user_id = ?1 AND id = ?2",
            params![user, id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
    })?;
    Ok(row.and_then(row_filter))
}

/// Does this user own a filter with this id?
pub fn owns(a: &Accounts, user: i64, id: i64) -> Result<bool, AuthError> {
    a.read_db(|db| {
        db.query_row(
            "SELECT 1 FROM user_filters WHERE user_id = ?1 AND id = ?2",
            params![user, id],
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

/// Save a new filter; refused beyond [`MAX_FILTERS_PER_USER`].
pub fn add(a: &Accounts, user: i64, d: &Draft) -> Result<i64, AddError> {
    a.with_db(|db| {
        let n: i64 = db.query_row(
            "SELECT count(*) FROM user_filters WHERE user_id = ?1",
            [user],
            |r| r.get(0),
        )?;
        if n as usize >= MAX_FILTERS_PER_USER {
            return Ok(None);
        }
        let t = now();
        db.execute(
            "INSERT INTO user_filters (user_id, name, spec, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![user, d.name, d.spec.to_json().to_string(), t],
        )?;
        Ok(Some(db.last_insert_rowid()))
    })
    .map_err(|_| AddError::Store)?
    .ok_or(AddError::Full)
}

/// Replace one of the user's own filters. False if there was none.
pub fn replace(a: &Accounts, user: i64, id: i64, d: &Draft) -> Result<bool, AuthError> {
    a.with_db(|db| {
        db.execute(
            "UPDATE user_filters SET name = ?3, spec = ?4, updated_at = ?5
             WHERE user_id = ?1 AND id = ?2",
            params![user, id, d.name, d.spec.to_json().to_string(), now()],
        )
        .map(|n| n > 0)
    })
}

/// Remove one of the user's own filters. False if there was none.
pub fn remove(a: &Accounts, user: i64, id: i64) -> Result<bool, AuthError> {
    a.with_db(|db| {
        db.execute(
            "DELETE FROM user_filters WHERE user_id = ?1 AND id = ?2",
            params![user, id],
        )
        .map(|n| n > 0)
    })
}

// ---------------------------------------------------------------------------
// Running a filter (pure parts)
// ---------------------------------------------------------------------------

/// The LMS `albums` queries for a spec: one, or one per year of a decade (in year order).
/// Every word comes from the parsed rules; nothing a client sent is passed on as text.
pub fn queries(spec: &Spec) -> Vec<Vec<String>> {
    let mut base: Vec<String> = Vec::new();
    let mut years: Option<u32> = None;
    for r in &spec.rules {
        match r {
            Rule::Genre(ids) => base.push(format!(
                "genre_id:{}",
                ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
            )),
            Rule::Decade(d) => years = Some(*d),
            Rule::Artist { id, role } => {
                base.push(format!("artist_id:{id}"));
                if let Some(role) = role {
                    base.push(format!("role_id:{role}"));
                }
            }
            Rule::Compilation(b) => base.push(format!("compilation:{}", u8::from(*b))),
        }
    }
    let one = |year: Option<u32>| {
        let mut cmd = vec!["albums".to_string(), "0".into(), spec.limit.to_string()];
        cmd.extend(base.iter().cloned());
        if let Some(y) = year {
            cmd.push(format!("year:{y}"));
        }
        cmd.push(format!("sort:{}", spec.sort.as_str()));
        cmd.push("tags:lay".into());
        cmd
    };
    match years {
        Some(d) => (d..d + 10).map(|y| one(Some(y))).collect(),
        None => vec![one(None)],
    }
}

/// The allow-list's view of a gateway-built query: a plain server query.
struct Server;

impl Caller for Server {
    fn role(&self) -> Role {
        Role::Listener
    }
    fn may_control(&self, _: &str) -> bool {
        false
    }
}

/// Would `/v1/rpc` let a listener send this exact query? (Defence in depth: the queries
/// are built from parsed rules, and must also pass the allow-list.)
pub fn allowed(cmd: &[String]) -> bool {
    let raw: Vec<Value> = cmd.iter().map(|w| Value::String(w.clone())).collect();
    rpc_policy::check(&Server, "", &raw, &HashSet::new())
        .is_ok_and(|c| c.player.is_empty() && c.cmd == cmd)
}

/// An album as LMS lists it for a filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LmsAlbum {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub year: u32,
}

/// Albums in an LMS `albums` answer (tags `l`, `a`, `y`), read strictly: anything without
/// a positive id is left out.
pub fn lms_albums(v: &Value) -> Vec<LmsAlbum> {
    let num = |x: Option<&Value>| match x {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    let text = |x: Option<&Value>| x.and_then(Value::as_str).unwrap_or("").to_lowercase();
    v.get("albums_loop")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|a| {
                    Some(LmsAlbum {
                        id: num(a.get("id")).filter(|n| *n > 0)?,
                        title: text(a.get("album")),
                        artist: text(a.get("artist")),
                        year: num(a.get("year"))
                            .and_then(|y| u32::try_from(y).ok())
                            .unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One filter result from LMS's answers to [`queries`] (in the same order): merged,
/// de-duplicated, sorted and cut to the limit. A single answer keeps LMS's own order
/// (shuffled for `random`). Answers for several years are merged by the sort: A–Z by
/// title, by artist then title, by year (the years in order, LMS's order within each),
/// newest first by album id (LMS numbers albums as it adds them), or shuffled.
pub fn merge(answers: &[Vec<LmsAlbum>], sort: Sort, limit: u32, rng: &mut Rng) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut all: Vec<&LmsAlbum> = answers
        .iter()
        .flatten()
        .filter(|a| seen.insert(a.id))
        .collect();
    if answers.len() > 1 {
        match sort {
            Sort::Album => all.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id))),
            Sort::ArtistAlbum => all.sort_by(|a, b| {
                a.artist
                    .cmp(&b.artist)
                    .then(a.title.cmp(&b.title))
                    .then(a.id.cmp(&b.id))
            }),
            Sort::New => all.sort_by_key(|a| std::cmp::Reverse(a.id)),
            // The queries go year by year, in order.
            Sort::YearAlbum | Sort::Random => {}
        }
    }
    let mut ids: Vec<i64> = all.into_iter().map(|a| a.id).collect();
    if sort == Sort::Random {
        // Fisher–Yates.
        for i in (1..ids.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            ids.swap(i, j);
        }
    }
    ids.truncate(limit as usize);
    ids
}

// ---------------------------------------------------------------------------
// Kept results
// ---------------------------------------------------------------------------

/// Recent results, per user, filter and spec, for later pages.
#[derive(Default)]
pub struct Results(Mutex<HashMap<(i64, i64), Kept>>);

/// A kept result: when it was made, the spec it was made for, and the album ids.
type Kept = (Instant, String, Vec<i64>);

impl Results {
    fn get(&self, key: (i64, i64), spec: &str, now: Instant) -> Option<Vec<i64>> {
        let m = self.0.lock().unwrap();
        let (at, s, ids) = m.get(&key)?;
        (s == spec && now.duration_since(*at) < KEEP_RESULTS).then(|| ids.clone())
    }

    fn put(&self, key: (i64, i64), spec: String, ids: Vec<i64>, now: Instant) {
        let mut m = self.0.lock().unwrap();
        if m.len() >= MAX_KEPT {
            m.retain(|_, (at, _, _)| now.duration_since(*at) < KEEP_RESULTS);
            if m.len() >= MAX_KEPT {
                m.clear();
            }
        }
        m.insert(key, (now, spec, ids));
    }

    pub(crate) fn forget(&self, user: i64, id: i64) {
        self.0.lock().unwrap().remove(&(user, id));
    }
}

/// Why running a filter failed.
#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// LMS couldn't be reached or refused a query.
    Unavailable,
    /// A query didn't pass the allow-list (never expected).
    Refused,
}

impl crate::Gateway {
    /// Run a filter's queries against LMS and merge the answers.
    pub(crate) async fn run_filter(&self, spec: &Spec) -> Result<Vec<i64>, RunError> {
        let mut answers = Vec::new();
        for cmd in queries(spec) {
            if !allowed(&cmd) {
                return Err(RunError::Refused);
            }
            let v = self
                .lms_query_retrying("", &cmd, LMS_LIMIT)
                .await
                .map_err(|_| RunError::Unavailable)?;
            answers.push(lms_albums(&v));
        }
        Ok(merge(&answers, spec.sort, spec.limit, &mut Rng::new()))
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn busy(message: &str) -> Response {
    let mut r = error(StatusCode::TOO_MANY_REQUESTS, message);
    r.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    r
}

fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not found")
}

/// `GET /v1/me/filters`
pub async fn list_filters(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match list(gw.accounts(), p.user_id) {
        Ok(f) => ok(json!({"filters": f.iter().map(Filter::to_json).collect::<Vec<_>>()})),
        Err(_) => internal(),
    }
}

/// `POST /v1/me/filters` `{"name", "rules", "sort", "limit"}` → 201 with the filter.
pub async fn create(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let v = match body_json(&headers, body, MAX_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let draft = match parse_draft(&v, None) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let (user, stored) = (p.user_id, draft.clone());
    match gw.db(move |a| add(a, user, &stored)).await {
        Ok(id) => {
            let f = Filter {
                id,
                name: draft.name,
                spec: draft.spec,
            };
            let mut r = ok(f.to_json());
            *r.status_mut() = StatusCode::CREATED;
            r
        }
        Err(AddError::Full) => error(StatusCode::CONFLICT, "too many filters"),
        Err(AddError::Store) => internal(),
    }
}

/// `PUT /v1/me/filters/{id}`: replace one of the caller's filters.
pub async fn update(
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
        return not_found();
    };
    let v = match body_json(&headers, body, MAX_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let draft = match parse_draft(&v, Some(id)) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let (user, stored) = (p.user_id, draft.clone());
    match gw.db(move |a| replace(a, user, id, &stored)).await {
        Ok(true) => {
            gw.filter_results.forget(p.user_id, id);
            ok(Filter {
                id,
                name: draft.name,
                spec: draft.spec,
            }
            .to_json())
        }
        Ok(false) => not_found(),
        Err(_) => internal(),
    }
}

/// `DELETE /v1/me/filters/{id}`
pub async fn delete(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let user = p.user_id;
    match gw.db(move |a| remove(a, user, id)).await {
        Ok(true) => {
            gw.filter_results.forget(p.user_id, id);
            ok(json!({"removed": true}))
        }
        Ok(false) => not_found(),
        Err(_) => internal(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    from: Option<String>,
    n: Option<String>,
}

/// A small decimal number from a query string, within `range`.
fn query_num(s: Option<&str>, default: u32, range: std::ops::RangeInclusive<u32>) -> Option<u32> {
    match s {
        None => Some(default),
        Some(s) if !s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse().ok().filter(|n| range.contains(n))
        }
        Some(_) => None,
    }
}

/// `GET /v1/me/filters/{id}/albums?from=0&n=50` → `{"albums": [ids], "count": total}`.
pub async fn albums(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    q: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let Ok(Query(q)) = q else {
        return error(StatusCode::BAD_REQUEST, "bad query");
    };
    let Some(from) = query_num(q.from.as_deref(), 0, 0..=MAX_LIMIT) else {
        return error(StatusCode::BAD_REQUEST, "from must be 0 to 500");
    };
    let Some(n) = query_num(q.n.as_deref(), 50, 1..=MAX_PAGE) else {
        return error(StatusCode::BAD_REQUEST, "n must be 1 to 100");
    };
    let filter = match get(gw.accounts(), p.user_id, id) {
        Ok(Some(f)) => f,
        Ok(None) => return not_found(),
        Err(_) => return internal(),
    };
    let key = (p.user_id, id);
    let spec_text = filter.spec.to_json().to_string();
    let now = Instant::now();
    // Later pages continue the result the first page made, while it is kept.
    let kept = (from > 0)
        .then(|| gw.filter_results.get(key, &spec_text, now))
        .flatten();
    let ids = match kept {
        Some(ids) => ids,
        None => {
            if !gw.filter_runs.take(p.user_id, now) {
                return busy("too many filter requests; try again shortly");
            }
            let Some(_slot) = gw
                .filter_slots
                .acquire(p.user_id, Duration::from_secs(5))
                .await
            else {
                return busy("too many filter requests at once");
            };
            match gw.run_filter(&filter.spec).await {
                Ok(ids) => {
                    gw.filter_results
                        .put(key, spec_text, ids.clone(), Instant::now());
                    ids
                }
                Err(RunError::Unavailable) => {
                    return error(StatusCode::BAD_GATEWAY, "music server unavailable");
                }
                Err(RunError::Refused) => return internal(),
            }
        }
    };
    let page: Vec<i64> = ids
        .iter()
        .skip(from as usize)
        .take(n as usize)
        .copied()
        .collect();
    ok(json!({"albums": page, "count": ids.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(v: Value) -> Result<Spec, Invalid> {
        parse_spec(&v)
    }

    fn with_rules(rules: Value) -> Value {
        json!({"rules": rules, "sort": "album", "limit": 100})
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn every_field_and_operator_parses() {
        let v = json!({"rules": [
            {"field": "genre", "in": [9, 4]},
            {"field": "decade", "is": 1990},
            {"field": "artist", "is": 12, "role": "COMPOSER"},
            {"field": "compilation", "is": false}
        ], "sort": "yearalbum", "limit": 500});
        let sp = spec(v).unwrap();
        assert_eq!(
            sp.rules,
            vec![
                Rule::Genre(vec![4, 9]),
                Rule::Decade(1990),
                Rule::Artist {
                    id: 12,
                    role: Some("COMPOSER")
                },
                Rule::Compilation(false),
            ]
        );
        assert_eq!(sp.sort, Sort::YearAlbum);
        assert_eq!(sp.limit, 500);
        assert_eq!(parse_spec(&sp.to_json()), Ok(sp.clone()));
        for role in ROLES {
            let r = spec(with_rules(
                json!([{"field": "artist", "is": 1, "role": role}]),
            ));
            assert!(r.is_ok(), "{role}");
        }
        assert_eq!(
            spec(with_rules(json!([{"field": "artist", "is": 3}])))
                .unwrap()
                .rules,
            vec![Rule::Artist { id: 3, role: None }]
        );
        assert_eq!(
            spec(with_rules(json!([{"field": "compilation", "is": true}])))
                .unwrap()
                .rules,
            vec![Rule::Compilation(true)]
        );
        for (d, ok) in [
            (1900, true),
            (2090, true),
            (1980, true),
            (1890, false),
            (2100, false),
            (1985, false),
            (0, false),
        ] {
            assert_eq!(
                spec(with_rules(json!([{"field": "decade", "is": d}]))).is_ok(),
                ok,
                "{d}"
            );
        }
        for sort in ["album", "artistalbum", "yearalbum", "new", "random"] {
            let v =
                json!({"rules": [{"field": "compilation", "is": true}], "sort": sort, "limit": 1});
            assert_eq!(spec(v).unwrap().sort.as_str(), sort);
        }
        let twenty: Vec<u64> = (1..=20).collect();
        assert!(spec(with_rules(json!([{"field": "genre", "in": twenty}]))).is_ok());
    }

    #[test]
    fn hostile_specs_are_refused() {
        let rule = |r: Value| with_rules(json!([r]));
        let twenty_one: Vec<u64> = (1..=21).collect();
        let nine: Vec<Value> = (0..9)
            .map(|_| json!({"field": "compilation", "is": true}))
            .collect();
        let cases = [
            json!(null),
            json!([]),
            json!("rules"),
            json!({}),
            json!({"rules": [], "sort": "album", "limit": 10}),
            json!({"rules": {}, "sort": "album", "limit": 10}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album"}),
            json!({"rules": [{"field": "compilation", "is": true}], "limit": 10}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": 10, "x": 1}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": 10, "__proto__": {}}),
            json!({"rules": nine, "sort": "album", "limit": 10}),
            // Sort and limit.
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "Album", "limit": 10}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "title", "limit": 10}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album; DROP", "limit": 10}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": 1, "limit": 10}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": 0}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": 501}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": -1}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": 10.5}),
            json!({"rules": [{"field": "compilation", "is": true}], "sort": "album", "limit": "10"}),
            // Rules.
            rule(json!("genre")),
            rule(json!([])),
            rule(json!({})),
            rule(json!({"field": "Genre", "in": [1]})),
            rule(json!({"field": "title", "is": "x"})),
            rule(json!({"field": "rating", "is": 5})),
            rule(json!({"field": "genre"})),
            rule(json!({"field": "genre", "in": []})),
            rule(json!({"field": "genre", "in": twenty_one})),
            rule(json!({"field": "genre", "in": [0]})),
            rule(json!({"field": "genre", "in": [-3]})),
            rule(json!({"field": "genre", "in": ["3"]})),
            rule(json!({"field": "genre", "in": [1.5]})),
            rule(json!({"field": "genre", "in": [3, 3]})),
            rule(json!({"field": "genre", "in": [[3]]})),
            rule(json!({"field": "genre", "in": [{"id": 3}]})),
            rule(json!({"field": "genre", "in": [1_000_000_000u64]})),
            rule(json!({"field": "genre", "in": 3})),
            rule(json!({"field": "genre", "is": 3})),
            rule(json!({"field": "genre", "in": [3], "x": 1})),
            rule(json!({"field": "decade"})),
            rule(json!({"field": "decade", "is": "1990"})),
            rule(json!({"field": "decade", "is": 1990.0})),
            rule(json!({"field": "decade", "in": [1990]})),
            rule(json!({"field": "decade", "is": 1990, "role": "ARTIST"})),
            rule(json!({"field": "artist"})),
            rule(json!({"field": "artist", "is": 0})),
            rule(json!({"field": "artist", "is": "12"})),
            rule(json!({"field": "artist", "is": [12]})),
            rule(json!({"field": "artist", "is": 12, "role": "composer"})),
            rule(json!({"field": "artist", "is": 12, "role": "PRODUCER"})),
            rule(json!({"field": "artist", "is": 12, "role": "COMPOSER,BAND"})),
            rule(json!({"field": "artist", "is": 12, "role": null})),
            rule(json!({"field": "artist", "is": 12, "role": 1})),
            rule(json!({"field": "artist", "is": 12, "role": ""})),
            rule(json!({"field": "compilation"})),
            rule(json!({"field": "compilation", "is": 1})),
            rule(json!({"field": "compilation", "is": "true"})),
            rule(json!({"field": "compilation", "is": null})),
            rule(json!({"field": "compilation", "in": [true]})),
            rule(json!({"field": null})),
            rule(json!({"field": ["genre"]})),
            // Each field at most once.
            with_rules(json!([
                {"field": "genre", "in": [1]},
                {"field": "genre", "in": [2]}
            ])),
            with_rules(json!([
                {"field": "decade", "is": 1980},
                {"field": "decade", "is": 1990}
            ])),
            with_rules(json!([
                {"field": "compilation", "is": true},
                {"field": "compilation", "is": false}
            ])),
            with_rules(json!([
                {"field": "artist", "is": 1},
                {"field": "artist", "is": 2, "role": "BAND"}
            ])),
        ];
        for c in cases {
            assert!(spec(c.clone()).is_err(), "accepted {c}");
        }
    }

    #[test]
    fn drafts_check_the_name_and_the_id() {
        let good = json!({"name": "  Late   jazz ", "rules": [{"field": "genre", "in": [4]}], "sort": "new", "limit": 50});
        let d = parse_draft(&good, None).unwrap();
        assert_eq!(d.name, "Late jazz");
        assert_eq!(d.spec.sort, Sort::New);
        let with = |k: &str, v: Value| {
            let mut g = good.clone();
            g[k] = v;
            g
        };
        // A new filter has no id; a replaced one may repeat its own.
        assert!(parse_draft(&with("id", json!(3)), None).is_err());
        assert!(parse_draft(&with("id", json!(3)), Some(3)).is_ok());
        assert!(parse_draft(&with("id", json!(4)), Some(3)).is_err());
        assert!(parse_draft(&with("id", json!("3")), Some(3)).is_err());
        assert!(parse_draft(&with("id", json!(3.0)), Some(3)).is_err());
        for bad in [
            json!(""),
            json!("   "),
            json!(5),
            Value::Null,
            json!("x".repeat(65)),
            json!("\u{202e}"),
            json!(["Late"]),
        ] {
            assert!(
                parse_draft(&with("name", bad.clone()), None).is_err(),
                "{bad}"
            );
        }
        let mut no_name = good.clone();
        no_name.as_object_mut().unwrap().remove("name");
        assert!(parse_draft(&no_name, None).is_err());
        let markup = parse_draft(&with("name", json!("<b>Mix</b>")), None).unwrap();
        assert!(!markup.name.contains('<'));
        assert!(parse_draft(&with("owner", json!(2)), None).is_err());
        assert!(parse_draft(&with("user_id", json!(2)), Some(3)).is_err());
    }

    #[test]
    fn queries_carry_only_the_parsed_rules() {
        let sp = Spec {
            rules: vec![
                Rule::Genre(vec![3, 4]),
                Rule::Artist {
                    id: 9,
                    role: Some("BAND"),
                },
                Rule::Compilation(false),
            ],
            sort: Sort::ArtistAlbum,
            limit: 120,
        };
        assert_eq!(
            queries(&sp),
            vec![s(&[
                "albums",
                "0",
                "120",
                "genre_id:3,4",
                "artist_id:9",
                "role_id:BAND",
                "compilation:0",
                "sort:artistalbum",
                "tags:lay"
            ])]
        );
        let decade = Spec {
            rules: vec![Rule::Decade(1970), Rule::Compilation(true)],
            sort: Sort::Random,
            limit: 10,
        };
        let q = queries(&decade);
        assert_eq!(q.len(), 10);
        for (i, cmd) in q.iter().enumerate() {
            assert_eq!(
                cmd,
                &s(&[
                    "albums",
                    "0",
                    "10",
                    "compilation:1",
                    &format!("year:{}", 1970 + i),
                    "sort:random",
                    "tags:lay"
                ])
            );
        }
        // Every query any valid spec makes passes the /v1/rpc allow-list, at the largest
        // sizes too.
        let big = Spec {
            rules: vec![
                Rule::Genre((MAX_RULE_ID - 19..=MAX_RULE_ID).collect()),
                Rule::Decade(2090),
                Rule::Artist {
                    id: MAX_RULE_ID,
                    role: Some("TRACKARTIST"),
                },
                Rule::Compilation(true),
            ],
            sort: Sort::YearAlbum,
            limit: MAX_LIMIT,
        };
        for cmd in queries(&big).iter().chain(queries(&sp).iter()) {
            assert!(allowed(cmd), "{cmd:?}");
        }
        // The check itself refuses what the allow-list refuses.
        assert!(!allowed(&s(&["albums", "0", "501"])));
        assert!(!allowed(&s(&["albums", "0", "10", "role_id:PRODUCER"])));
        assert!(!allowed(&s(&["pref", "audiodir"])));
        assert!(!allowed(&s(&["albums", "0", "10", "search:file:///srv"])));
    }

    fn album(id: i64, title: &str, artist: &str, year: u32) -> LmsAlbum {
        LmsAlbum {
            id,
            title: title.to_lowercase(),
            artist: artist.to_lowercase(),
            year,
        }
    }

    #[test]
    fn decades_merge_deduplicate_sort_and_limit() {
        let mut rng = Rng::seeded(1);
        let y0 = vec![
            album(5, "Zebra", "Bee", 1990),
            album(2, "Apple", "Cee", 1990),
        ];
        let y1 = vec![
            album(9, "Mango", "Ant", 1991),
            album(5, "Zebra", "Bee", 1990),
        ];
        let y2 = vec![album(1, "Banana", "Ant", 1992)];
        let answers = [y0.clone(), y1.clone(), y2.clone()];
        assert_eq!(merge(&answers, Sort::Album, 10, &mut rng), vec![2, 1, 9, 5]);
        assert_eq!(
            merge(&answers, Sort::ArtistAlbum, 10, &mut rng),
            vec![1, 9, 5, 2]
        );
        // By year: the years in order, LMS's order within each.
        assert_eq!(
            merge(&answers, Sort::YearAlbum, 10, &mut rng),
            vec![5, 2, 9, 1]
        );
        assert_eq!(merge(&answers, Sort::New, 10, &mut rng), vec![9, 5, 2, 1]);
        assert_eq!(merge(&answers, Sort::Album, 2, &mut rng), vec![2, 1]);
        let mut shuffled = merge(&answers, Sort::Random, 10, &mut rng);
        shuffled.sort();
        assert_eq!(shuffled, vec![1, 2, 5, 9]);
        assert_eq!(merge(&answers, Sort::Random, 3, &mut rng).len(), 3);
        // One answer keeps LMS's order.
        assert_eq!(
            merge(std::slice::from_ref(&y1), Sort::Album, 10, &mut rng),
            vec![9, 5]
        );
        assert!(merge(&[], Sort::Album, 10, &mut rng).is_empty());
        // Random really does reorder.
        let many = vec![
            (1..=50)
                .map(|i| album(i, "x", "y", 2000))
                .collect::<Vec<_>>(),
        ];
        let orders: HashSet<Vec<i64>> = (0..5)
            .map(|_| merge(&many, Sort::Random, 50, &mut rng))
            .collect();
        assert!(orders.len() > 1);
    }

    #[test]
    fn lms_album_lists_are_read_strictly() {
        let v = json!({"albums_loop": [
            {"id": 1, "album": "One", "artist": "A", "year": 1990},
            {"id": "2", "album": 5, "year": "x"},
            {"id": -3, "album": "Bad"},
            {"album": "No id"},
            {"id": 4, "year": -1990}
        ]});
        assert_eq!(
            lms_albums(&v),
            vec![
                album(1, "one", "a", 1990),
                album(2, "", "", 0),
                album(4, "", "", 0)
            ]
        );
        assert!(lms_albums(&json!({"albums_loop": 5})).is_empty());
        assert!(lms_albums(&json!(null)).is_empty());
    }

    #[test]
    fn page_numbers_are_small_plain_decimals() {
        assert_eq!(query_num(None, 50, 1..=100), Some(50));
        assert_eq!(query_num(Some("100"), 50, 1..=100), Some(100));
        assert_eq!(query_num(Some("0"), 0, 0..=500), Some(0));
        for bad in [
            "", "101", "0", "-1", "+1", "1e2", "0x10", "99999", "1 ", " 1",
        ] {
            assert_eq!(query_num(Some(bad), 50, 1..=100), None, "{bad:?}");
        }
    }

    #[test]
    fn filters_are_stored_per_user_and_capped() {
        let a = Accounts::in_memory().unwrap();
        for u in ["alice", "bob"] {
            a.create_user(u, "correct horse battery", Role::Listener)
                .unwrap();
        }
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let bob = a.user_id_by_name("bob").unwrap().unwrap();
        let d = parse_draft(
            &json!({"name": "Mine", "rules": [{"field": "decade", "is": 1980}], "sort": "album", "limit": 20}),
            None,
        )
        .unwrap();
        let id = add(&a, alice, &d).unwrap();
        assert_eq!(list(&a, alice).unwrap().len(), 1);
        assert!(list(&a, bob).unwrap().is_empty());
        assert!(owns(&a, alice, id).unwrap());
        assert!(!owns(&a, bob, id).unwrap());
        assert_eq!(get(&a, bob, id).unwrap(), None);
        assert!(!replace(&a, bob, id, &d).unwrap());
        assert!(!remove(&a, bob, id).unwrap());
        let mut d2 = d.clone();
        d2.name = "Renamed".into();
        assert!(replace(&a, alice, id, &d2).unwrap());
        assert_eq!(get(&a, alice, id).unwrap().unwrap().name, "Renamed");
        for _ in 1..MAX_FILTERS_PER_USER {
            add(&a, alice, &d).unwrap();
        }
        assert_eq!(add(&a, alice, &d), Err(AddError::Full));
        assert!(add(&a, bob, &d).is_ok(), "bob has his own allowance");
        // A stored row that no longer reads is left out.
        a.with_db(|db| {
            db.execute(
                "UPDATE user_filters SET spec = '{\"rules\": 1}' WHERE id = ?1",
                [id],
            )
        })
        .unwrap();
        assert_eq!(get(&a, alice, id).unwrap(), None);
        assert_eq!(list(&a, alice).unwrap().len(), MAX_FILTERS_PER_USER - 1);
        assert!(remove(&a, alice, id).unwrap());
        // Filters go with the user.
        a.delete_user("alice").unwrap();
        let left: i64 = a
            .with_db(|db| db.query_row("SELECT count(*) FROM user_filters", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(left, 1);
    }

    /// Robustness: arbitrary JSON (and arbitrary bytes) never panic the parsers, and what
    /// they accept is within the limits, reads back the same and makes only allowed
    /// queries.
    #[test]
    fn fuzz_the_filter_parser() {
        let mut rng = Rng::seeded(0xf117e5);
        let atoms = [
            json!(null),
            json!(true),
            json!(false),
            json!(0),
            json!(1),
            json!(-1),
            json!(12),
            json!(1990),
            json!(1985),
            json!(500),
            json!(501),
            json!(1.5),
            json!(1_000_000_000u64),
            json!(u64::MAX),
            json!("genre"),
            json!("decade"),
            json!("artist"),
            json!("compilation"),
            json!("album"),
            json!("random"),
            json!("COMPOSER"),
            json!("BAND"),
            json!("x".repeat(300)),
            json!("file:///srv/music"),
            json!("\u{0}"),
            json!([]),
            json!({}),
        ];
        let keys = [
            "name", "rules", "sort", "limit", "id", "field", "in", "is", "role", "x",
        ];
        fn pick<'a, T>(rng: &mut Rng, v: &'a [T]) -> &'a T {
            &v[rng.below(v.len() as u64) as usize]
        }
        fn object(rng: &mut Rng, keys: &[&str], atoms: &[Value], depth: u32) -> Value {
            let mut o = Map::new();
            for _ in 0..rng.below(6) {
                let k = pick(rng, keys).to_string();
                let v = match rng.below(5) {
                    0 | 1 => pick(rng, atoms).clone(),
                    2 => Value::Array(
                        (0..rng.below(25))
                            .map(|_| pick(rng, atoms).clone())
                            .collect(),
                    ),
                    3 if depth < 3 => Value::Array(
                        (0..rng.below(10))
                            .map(|_| object(rng, keys, atoms, depth + 1))
                            .collect(),
                    ),
                    _ if depth < 3 => object(rng, keys, atoms, depth + 1),
                    _ => Value::Null,
                };
                o.insert(k, v);
            }
            Value::Object(o)
        }
        let fields = ["genre", "decade", "artist", "compilation", "other"];
        let sorts = ["album", "artistalbum", "yearalbum", "new", "random", "x"];
        let mut accepted = 0;
        for i in 0..30_000 {
            let v = if i % 2 == 0 {
                object(&mut rng, &keys, &atoms, 0)
            } else {
                // Nearly valid filters, so the deep checks are reached too.
                let rules: Vec<Value> = (0..rng.below(4) + rng.below(2) * rng.below(8))
                    .map(|_| {
                        let field = *pick(&mut rng, &fields);
                        let mut r = Map::new();
                        r.insert("field".into(), json!(field));
                        // Mostly well-formed for its field, sometimes not.
                        match (field, rng.below(4)) {
                            ("genre", 0..=2) => {
                                let n = rng.below(23);
                                let ids: Vec<u64> = (0..n).map(|_| rng.below(40)).collect();
                                r.insert("in".into(), json!(ids));
                            }
                            ("decade", 0..=2) => {
                                r.insert("is".into(), json!(1890 + 10 * rng.below(22)));
                            }
                            ("artist", 0..=2) => {
                                r.insert("is".into(), json!(rng.below(5)));
                                if rng.below(2) == 0 {
                                    let role = pick(&mut rng, &["BAND", "COMPOSER", "x", ""]);
                                    r.insert("role".into(), json!(role));
                                }
                            }
                            ("compilation", 0..=2) => {
                                r.insert("is".into(), json!(rng.below(2) == 0));
                            }
                            _ => {
                                let k = pick(&mut rng, &["in", "is", "role", "x"]).to_string();
                                r.insert(k, pick(&mut rng, &atoms).clone());
                            }
                        }
                        Value::Object(r)
                    })
                    .collect();
                json!({"name": "F", "rules": rules, "sort": pick(&mut rng, &sorts),
                       "limit": rng.below(520)})
            };
            if let Ok(d) = parse_draft(&v, Some(1)) {
                accepted += 1;
                let sp = &d.spec;
                assert!((1..=MAX_RULES).contains(&sp.rules.len()));
                assert!((1..=MAX_LIMIT).contains(&sp.limit));
                assert!((1..=64).contains(&d.name.chars().count()));
                assert_eq!(parse_spec(&sp.to_json()), Ok(sp.clone()));
                for cmd in queries(sp) {
                    assert!(allowed(&cmd), "{cmd:?}");
                }
            }
            let _ = parse_spec(&v);
            let _ = parse_draft(&v, None);
        }
        assert!(
            accepted > 100,
            "the fuzzer reached valid filters only {accepted} times"
        );
        for _ in 0..5000 {
            let bytes: Vec<u8> = (0..rng.below(100)).map(|_| rng.next_u64() as u8).collect();
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                let _ = parse_spec(&v);
                let _ = parse_draft(&v, None);
            }
        }
        // LMS answers too.
        for _ in 0..5000 {
            let v = json!({"albums_loop": (0..rng.below(5)).map(|_| object(&mut rng, &["id", "album", "artist", "year"], &atoms, 2)).collect::<Vec<_>>()});
            let list = lms_albums(&v);
            assert!(list.iter().all(|a| a.id > 0));
            let _ = merge(&[list.clone(), list], Sort::Album, 10, &mut rng);
        }
    }
}
