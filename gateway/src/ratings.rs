//! Ratings (docs/plans/wave2-design.md §2): each user's own rating of a track, kept on the
//! gateway by stable track key (so it survives rescans), blended with LMS's shared rating.
//!
//! - `PUT /v1/me/ratings/{track}` `{"rating": 0–100}` (steps of 10; 0 clears): rate a library
//!   track. Also written to LMS's shared rating through Ratings Light, where it is installed
//!   (a clear takes back only the user's own rating, never the shared one).
//! - `GET /v1/me/ratings?tracks=1,2,…` (at most [`MAX_TRACKS`]): for each track, the user's
//!   own rating, LMS's shared rating (tag `R`) and the blend.
//! - `POST /v1/me/ratings` `{"ratings": [{"track", "key", "rating", "at"}]}`: ratings made
//!   offline. Each `key` must be the track's stable key as LMS knows it now (others are
//!   skipped); the newest rating of a track wins, so a batch can be sent again safely.
//! - `DELETE /v1/me/ratings`: forget all of the user's ratings.
//! - `GET`/`PUT /v1/me/ratings/style`: `"stars"` or `"heart"`.
//!
//! **Who:** any signed-in user, for themselves only, within the `/v1/me/…` allowance. Never
//! shown to anyone else (the shared LMS rating is LMS's, as Ratings Light keeps it).
//!
//! **Blending:** the user's own rating counts [`MINE_WEIGHT`] times LMS's shared one
//! (as listening history does); where only one exists, that one.

use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::accounts::{Accounts, AuthError};
use crate::history::{body_json, caller, error, ok, unix_now};
use crate::{AppState, download};

/// Most tracks asked about at once.
pub const MAX_TRACKS: usize = 200;
/// Most ratings in one offline batch.
pub const MAX_BATCH: usize = 500;
/// Most ratings kept per user (the oldest go first).
pub const MAX_PER_USER: usize = 100_000;
/// How much more the user's own rating counts than LMS's shared one.
pub const MINE_WEIGHT: u32 = 3;
const MAX_BODY: usize = 128 * 1024;
/// Time limit for one write of LMS's shared rating (Ratings Light answers at once, or
/// not at all).
pub const SHARED_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// Longest one offline batch may spend writing LMS's shared ratings. What is left when it
/// runs out (or once a write fails or times out) is not written: the user's own ratings are
/// all kept, and the shared rating is LMS's, best effort.
pub const SHARED_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);
/// Seconds a client is told to wait while its previous batch is still running.
const BUSY_RETRY_AFTER: u64 = 5;

/// A rating: 0–100 in steps of 10 (half stars).
pub fn rating_ok(r: i64) -> bool {
    (0..=100).contains(&r) && r % 10 == 0
}

/// The blend of the user's own rating and LMS's shared one.
pub fn blend(mine: Option<u32>, shared: Option<u32>) -> Option<u32> {
    match (mine, shared) {
        (Some(m), Some(s)) => {
            Some(((m * MINE_WEIGHT + s) as f64 / (MINE_WEIGHT + 1) as f64).round() as u32)
        }
        (m, s) => m.or(s),
    }
}

/// A list of track ids: `1,2,3`, positive, at most [`MAX_TRACKS`], each once.
pub fn parse_tracks(q: &str) -> Option<Vec<u64>> {
    let v = q.strip_prefix("tracks=")?;
    let mut out = vec![];
    for t in v.split(',') {
        let id = download::parse_track_id(t)?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    (!out.is_empty() && out.len() <= MAX_TRACKS).then_some(out)
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

pub fn get_mine(a: &Accounts, user: i64, keys: &[String]) -> Result<Vec<Option<u32>>, AuthError> {
    a.with_db(|db| {
        let mut st =
            db.prepare("SELECT rating FROM user_ratings WHERE user_id = ?1 AND track_key = ?2")?;
        keys.iter()
            .map(|k| {
                st.query_row(rusqlite::params![user, k], |r| r.get::<_, i64>(0))
                    .optional()
            })
            .map(|r| r.map(|o| o.map(|v| v.clamp(0, 100) as u32)))
            .collect()
    })
}

/// Set (or with 0, clear) a rating made at `at`; an older one never replaces a newer one.
/// Returns whether anything changed.
pub fn set(
    a: &Accounts,
    user: i64,
    key: &str,
    track_id: u64,
    rating: u32,
    at: u64,
) -> Result<bool, AuthError> {
    a.with_db(|db| {
        let newer: Option<i64> = db
            .query_row(
                "SELECT rated_at FROM user_ratings WHERE user_id = ?1 AND track_key = ?2",
                rusqlite::params![user, key],
                |r| r.get(0),
            )
            .optional()?;
        if newer.is_some_and(|t| t > at as i64) {
            return Ok(false);
        }
        if rating == 0 {
            db.execute(
                "DELETE FROM user_ratings WHERE user_id = ?1 AND track_key = ?2",
                rusqlite::params![user, key],
            )?;
        } else {
            db.execute(
                "INSERT INTO user_ratings (user_id, track_key, track_id, rating, rated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (user_id, track_key) DO UPDATE SET
                   track_id = excluded.track_id, rating = excluded.rating, rated_at = excluded.rated_at",
                rusqlite::params![user, key, track_id as i64, rating, at as i64],
            )?;
            // Bounded per user: the oldest go first.
            db.execute(
                "DELETE FROM user_ratings WHERE user_id = ?1 AND track_key IN (
                   SELECT track_key FROM user_ratings WHERE user_id = ?1
                   ORDER BY rated_at DESC LIMIT -1 OFFSET ?2)",
                rusqlite::params![user, MAX_PER_USER as i64],
            )?;
        }
        Ok(true)
    })
}

pub fn clear(a: &Accounts, user: i64) -> Result<usize, AuthError> {
    a.with_db(|db| db.execute("DELETE FROM user_ratings WHERE user_id = ?1", [user]))
}

pub fn count(a: &Accounts, user: i64) -> Result<usize, AuthError> {
    a.with_db(|db| {
        Ok(db.query_row(
            "SELECT count(*) FROM user_ratings WHERE user_id = ?1",
            [user],
            |r| r.get::<_, i64>(0),
        )? as usize)
    })
}

pub fn style(a: &Accounts, user: i64) -> Result<&'static str, AuthError> {
    a.with_db(|db| {
        let s: Option<String> = db
            .query_row(
                "SELECT rating_style FROM user_prefs WHERE user_id = ?1",
                [user],
                |r| r.get(0),
            )
            .optional()?;
        Ok(if s.as_deref() == Some("heart") {
            "heart"
        } else {
            "stars"
        })
    })
}

pub fn set_style(a: &Accounts, user: i64, heart: bool) -> Result<(), AuthError> {
    a.with_db(|db| {
        db.execute(
            "INSERT INTO user_prefs (user_id, rating_style) VALUES (?1, ?2)
             ON CONFLICT (user_id) DO UPDATE SET rating_style = excluded.rating_style",
            rusqlite::params![user, if heart { "heart" } else { "stars" }],
        )?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// LMS
// ---------------------------------------------------------------------------

/// A library track as LMS knows it now: its stable key and LMS's shared rating.
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    pub id: u64,
    pub key: String,
    pub shared: Option<u32>,
}

/// LMS's shared rating (`rating`, 0–100; 0 or missing is none).
fn shared_of(item: &Value) -> Option<u32> {
    let r = match item.get("rating")? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (r.is_finite() && r > 0.0).then(|| r.round().clamp(1.0, 100.0) as u32)
}

/// Look `ids` up in LMS: local library files only (a stream has no stable key).
// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
async fn lookup(gw: &AppState, ids: &[u64], salt: &str) -> Result<Vec<Known>, Response> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let list: Vec<String> = ids.iter().map(u64::to_string).collect();
    let cmd = [
        "titles".to_string(),
        "0".into(),
        ids.len().to_string(),
        format!("track_id:{}", list.join(",")),
        "tags:uR".into(),
    ];
    let v = gw
        .lms_query_retrying("", &cmd, gw.cfg.download_lookup_timeout)
        .await
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "the server didn't answer"))?;
    Ok(v.get("titles_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let id = download::int(t, "id").filter(|n| *n > 0)? as u64;
            let url = t
                .get("url")
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("file://"))?;
            ids.contains(&id).then(|| Known {
                id,
                key: download::stable_key(salt, url),
                shared: shared_of(t),
            })
        })
        .collect())
}

/// Also set LMS's shared rating, through Ratings Light where it is installed (ignored
/// otherwise), within `limit` (at most [`SHARED_WRITE_TIMEOUT`]). `false` if LMS didn't
/// take it.
async fn write_shared(gw: &AppState, id: u64, rating: u32, limit: std::time::Duration) -> bool {
    let cmd = [
        "ratingslight".to_string(),
        "setratingpercentnoclient".into(),
        id.to_string(),
        rating.to_string(),
    ];
    match gw
        .lms_command_within("", &cmd, limit.min(SHARED_WRITE_TIMEOUT))
        .await
    {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!(error = %e, "Ratings Light not reachable");
            false
        }
    }
}

/// A user's one batch slot, freed when dropped.
struct BatchSlot<'a> {
    gw: &'a AppState,
    user: i64,
}

impl<'a> BatchSlot<'a> {
    fn take(gw: &'a AppState, user: i64) -> Option<Self> {
        // Not `then_some`: that would build (and so drop, freeing the slot) a guard even
        // when the slot is taken.
        let free = gw.rating_batches.lock().unwrap().insert(user);
        if free {
            Some(BatchSlot { gw, user })
        } else {
            None
        }
    }
}

impl Drop for BatchSlot<'_> {
    fn drop(&mut self) {
        self.gw.rating_batches.lock().unwrap().remove(&self.user);
    }
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

// The refusal is the handler's own answer, built once on the error path.
#[allow(clippy::result_large_err)]
fn salt(gw: &AppState) -> Result<String, Response> {
    gw.download_salt().ok_or_else(internal)
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// `GET /v1/me/ratings?tracks=1,2,3`
pub async fn get(
    State(gw): State<AppState>,
    headers: HeaderMap,
    RawQuery(q): RawQuery,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(ids) = q.as_deref().and_then(parse_tracks) else {
        return error(
            StatusCode::BAD_REQUEST,
            "expected ?tracks=1,2,… (at most 200)",
        );
    };
    let salt = match salt(&gw) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let known = match lookup(&gw, &ids, &salt).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let keys: Vec<String> = known.iter().map(|k| k.key.clone()).collect();
    let Ok(mine) = get_mine(gw.accounts(), p.user_id, &keys) else {
        return internal();
    };
    let ratings: Vec<Value> = known
        .iter()
        .zip(mine)
        .map(|(k, m)| json!({"track": k.id, "mine": m, "shared": k.shared, "rating": blend(m, k.shared)}))
        .collect();
    ok(json!({ "ratings": ratings }))
}

/// `PUT /v1/me/ratings/{track}` `{"rating": 0–100}`
pub async fn put(
    State(gw): State<AppState>,
    headers: HeaderMap,
    Path(track): Path<String>,
    body: Body,
) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(id) = download::parse_track_id(&track) else {
        return error(StatusCode::NOT_FOUND, "no such track");
    };
    let body = match body_json(&headers, body, 256).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let rating = match body.as_object().map(|o| (o.len(), o.get("rating"))) {
        Some((1, Some(Value::Number(n)))) => match n.as_i64().filter(|r| rating_ok(*r)) {
            Some(r) => r as u32,
            None => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "rating must be 0–100 in steps of 10",
                );
            }
        },
        _ => return error(StatusCode::BAD_REQUEST, "expected {\"rating\": 0–100}"),
    };
    let salt = match salt(&gw) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let known = match lookup(&gw, &[id], &salt).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let Some(k) = known.into_iter().next() else {
        return error(StatusCode::NOT_FOUND, "no such track");
    };
    if set(gw.accounts(), p.user_id, &k.key, id, rating, unix_now()).is_err() {
        return internal();
    }
    // Clearing takes back only the user's own rating: LMS's shared one is everyone's.
    if rating > 0 {
        write_shared(&gw, id, rating, SHARED_WRITE_TIMEOUT).await;
    }
    let mine = (rating > 0).then_some(rating);
    ok(json!({"track": id, "mine": mine, "shared": k.shared, "rating": blend(mine, k.shared)}))
}

/// `POST /v1/me/ratings` (ratings made offline)
pub async fn batch(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    // One batch at a time per user, as for uploads of plays.
    let Some(_slot) = BatchSlot::take(&gw, p.user_id) else {
        let mut r = error(
            StatusCode::TOO_MANY_REQUESTS,
            "another upload of ratings is running",
        );
        r.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from(BUSY_RETRY_AFTER),
        );
        return r;
    };
    let body = match body_json(&headers, body, MAX_BODY).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(items) = body
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get("ratings"))
        .and_then(Value::as_array)
    else {
        return error(StatusCode::BAD_REQUEST, "expected {\"ratings\": […]}");
    };
    if items.len() > MAX_BATCH {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "too many ratings at once");
    }
    let now = unix_now();
    let mut wanted = vec![];
    for it in items {
        let Some(o) = it.as_object().filter(|o| o.len() == 4) else {
            return error(StatusCode::BAD_REQUEST, "malformed rating");
        };
        let id = o
            .get("track")
            .and_then(Value::as_u64)
            .filter(|n| (1..=999_999_999_999).contains(n));
        let key = o.get("key").and_then(Value::as_str).filter(|k| {
            k.len() == 64
                && k.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        let rating = o
            .get("rating")
            .and_then(Value::as_i64)
            .filter(|r| rating_ok(*r));
        // Not in the future (a little clock skew allowed), not before 2020.
        let at = o
            .get("at")
            .and_then(Value::as_u64)
            .filter(|t| (1_577_836_800..=now + 300).contains(t));
        match (id, key, rating, at) {
            (Some(id), Some(key), Some(r), Some(at)) => {
                wanted.push((id, key.to_string(), r as u32, at.min(now)))
            }
            _ => return error(StatusCode::BAD_REQUEST, "malformed rating"),
        }
    }
    let salt = match salt(&gw) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let mut ids: Vec<u64> = wanted.iter().map(|w| w.0).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut known = vec![];
    for chunk in ids.chunks(MAX_TRACKS) {
        match lookup(&gw, chunk, &salt).await {
            Ok(k) => known.extend(k),
            Err(r) => return r,
        }
    }
    let (mut applied, mut skipped) = (0, 0);
    // What LMS's shared rating ends as: one write per track, never a clear.
    let mut shared: std::collections::BTreeMap<u64, u32> = Default::default();
    for (id, key, rating, at) in wanted {
        if !known.iter().any(|k| k.id == id && k.key == key) {
            skipped += 1; // not that track (any more)
            continue;
        }
        match set(gw.accounts(), p.user_id, &key, id, rating, at) {
            Ok(true) => {
                applied += 1;
                shared.insert(id, rating);
            }
            Ok(false) => skipped += 1, // a newer rating is kept
            Err(_) => return internal(),
        }
    }
    // Within the budget, and only while LMS takes them: a hanging or missing Ratings
    // Light costs one short wait, not one per track.
    let deadline = std::time::Instant::now() + SHARED_BUDGET;
    for (id, rating) in shared.into_iter().filter(|(_, r)| *r > 0) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || !write_shared(&gw, id, rating, left).await {
            break;
        }
    }
    ok(json!({ "applied": applied, "skipped": skipped }))
}

/// `DELETE /v1/me/ratings`
pub async fn delete_all(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match clear(gw.accounts(), p.user_id) {
        Ok(n) => ok(json!({ "removed": n })),
        Err(_) => internal(),
    }
}

/// `GET /v1/me/ratings/style`
pub async fn get_style(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match (
        style(gw.accounts(), p.user_id),
        count(gw.accounts(), p.user_id),
    ) {
        (Ok(s), Ok(n)) => ok(json!({ "style": s, "count": n })),
        _ => internal(),
    }
}

/// `PUT /v1/me/ratings/style` `{"style": "stars"|"heart"}`
pub async fn put_style(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let body = match body_json(&headers, body, 256).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let heart = match body
        .as_object()
        .map(|o| (o.len(), o.get("style").and_then(Value::as_str)))
    {
        Some((1, Some("heart"))) => true,
        Some((1, Some("stars"))) => false,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "expected {\"style\": \"stars\"|\"heart\"}",
            );
        }
    };
    match set_style(gw.accounts(), p.user_id, heart) {
        Ok(()) => ok(json!({ "style": if heart { "heart" } else { "stars" } })),
        Err(_) => internal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Role;

    #[test]
    fn values_and_blending() {
        for good in [0, 10, 50, 100] {
            assert!(rating_ok(good));
        }
        for bad in [-10, 5, 101, 110, i64::MAX, i64::MIN] {
            assert!(!rating_ok(bad), "{bad}");
        }
        assert_eq!(blend(None, None), None);
        assert_eq!(blend(Some(80), None), Some(80));
        assert_eq!(blend(None, Some(40)), Some(40));
        assert_eq!(blend(Some(100), Some(20)), Some(80)); // (300 + 20) / 4
        assert_eq!(blend(Some(60), Some(100)), Some(70));
    }

    #[test]
    fn track_lists() {
        assert_eq!(parse_tracks("tracks=1,2,2,3"), Some(vec![1, 2, 3]));
        let many: Vec<String> = (1..=200).map(|n| n.to_string()).collect();
        assert_eq!(
            parse_tracks(&format!("tracks={}", many.join(",")))
                .unwrap()
                .len(),
            200
        );
        let too_many: Vec<String> = (1..=201).map(|n| n.to_string()).collect();
        for bad in [
            "".to_string(),
            "tracks=".into(),
            "tracks=0".into(),
            "tracks=-1".into(),
            "tracks=1,,2".into(),
            "tracks=1;2".into(),
            "tracks=a".into(),
            "track=1".into(),
            "tracks=1&x=2".into(),
            format!("tracks={}", too_many.join(",")),
        ] {
            assert_eq!(parse_tracks(&bad), None, "{bad}");
        }
    }

    #[test]
    fn storage_keeps_the_newest_and_is_per_user() {
        let a = Accounts::in_memory().unwrap();
        a.create_user("alice", "correct horse battery", Role::Listener)
            .unwrap();
        a.create_user("bob", "correct horse battery", Role::Listener)
            .unwrap();
        let id = |n: &str| {
            a.with_db(|db| {
                db.query_row("SELECT id FROM users WHERE username = ?1", [n], |r| {
                    r.get::<_, i64>(0)
                })
            })
            .unwrap()
        };
        let (alice, bob) = (id("alice"), id("bob"));
        let k = "a".repeat(64);
        assert!(set(&a, alice, &k, 7, 80, 1_000).unwrap());
        assert!(
            !set(&a, alice, &k, 7, 20, 999).unwrap(),
            "an older rating never replaces a newer one"
        );
        assert_eq!(
            get_mine(&a, alice, &[k.clone(), "b".repeat(64)]).unwrap(),
            vec![Some(80), None]
        );
        assert_eq!(
            get_mine(&a, bob, std::slice::from_ref(&k)).unwrap(),
            vec![None],
            "private to each user"
        );
        assert!(set(&a, alice, &k, 7, 0, 2_000).unwrap());
        assert_eq!(
            get_mine(&a, alice, std::slice::from_ref(&k)).unwrap(),
            vec![None],
            "0 clears"
        );
        set(&a, alice, &k, 7, 60, 3_000).unwrap();
        assert_eq!(count(&a, alice).unwrap(), 1);
        assert_eq!(clear(&a, alice).unwrap(), 1);
        assert_eq!(style(&a, alice).unwrap(), "stars");
        set_style(&a, alice, true).unwrap();
        assert_eq!(style(&a, alice).unwrap(), "heart");
        assert_eq!(style(&a, bob).unwrap(), "stars");
        // Deleting the account deletes its ratings.
        set(&a, bob, &k, 7, 40, 1).unwrap();
        a.delete_user("bob").unwrap();
        let left: i64 = a
            .with_db(|db| {
                db.query_row(
                    "SELECT count(*) FROM user_ratings WHERE user_id = ?1",
                    [bob],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn shared_ratings_from_lms() {
        assert_eq!(shared_of(&json!({"rating": "80"})), Some(80));
        assert_eq!(shared_of(&json!({"rating": 60})), Some(60));
        for none in [
            json!({}),
            json!({"rating": "0"}),
            json!({"rating": "x"}),
            json!({"rating": -5}),
            json!({"rating": null}),
        ] {
            assert_eq!(shared_of(&none), None, "{none}");
        }
        assert_eq!(shared_of(&json!({"rating": 1e9})), Some(100));
    }
}
