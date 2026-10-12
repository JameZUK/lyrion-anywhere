//! Ratings and alarms (docs/plans/wave2-design.md §1–2) for the native apps: typed calls to
//! the gateway, with every answer checked as untrusted input and every request built from
//! checked values only. Offline ratings are queued by the offline library and sent with
//! [`sync_ratings`].

use std::sync::Arc;

use serde_json::{Value, json};

use crate::error::ApiError;
use crate::model::clean_text;
use crate::session::Session;

/// A track's rating: the user's own, LMS's shared one, and the blend shown (all 0–100).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrackRating {
    pub track_id: i64,
    pub mine: Option<u32>,
    pub shared: Option<u32>,
    pub rating: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RatingStyle {
    Stars,
    Heart,
}

/// A sound an alarm can play (from the server's own list).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AlarmSound {
    /// `current`, or 16 hex digits.
    pub id: String,
    pub category: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AlarmInfo {
    pub id: String,
    /// Seconds after midnight.
    pub time_secs: u32,
    /// 0 (Sunday) – 6, sorted.
    pub days: Vec<u8>,
    pub enabled: bool,
    pub repeat: bool,
    pub volume: Option<u8>,
    /// 0 off, 1 songs, 2 albums.
    pub shuffle: u8,
    /// The sound's id (`None`: something the server doesn't list).
    pub sound_id: Option<String>,
    pub sound_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AlarmListing {
    pub enabled: bool,
    pub alarms: Vec<AlarmInfo>,
    pub sounds: Vec<AlarmSound>,
    /// The new alarm's id, after adding.
    pub new_id: Option<String>,
}

/// A new alarm or a change (unset fields are left as they are).
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct AlarmChange {
    pub time_secs: Option<u32>,
    pub days: Option<Vec<u8>>,
    pub enabled: Option<bool>,
    pub repeat: Option<bool>,
    pub volume: Option<u8>,
    pub shuffle: Option<u8>,
    pub sound_id: Option<String>,
}

fn rating(v: &Value) -> Option<u32> {
    v.as_u64().filter(|r| *r <= 100).map(|r| r as u32)
}

fn alarm_id_ok(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn sound_id_ok(id: &str) -> bool {
    id == "current"
        || (id.len() == 16
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

fn text(v: &Value, max: usize) -> String {
    clean_text(v.as_str().unwrap_or(""))
        .chars()
        .take(max)
        .collect()
}

pub fn parse_rating(v: &Value) -> Option<TrackRating> {
    let track_id = v
        .get("track")?
        .as_i64()
        .filter(|n| (1..=999_999_999_999).contains(n))?;
    Some(TrackRating {
        track_id,
        mine: v.get("mine").and_then(rating),
        shared: v.get("shared").and_then(rating),
        rating: v.get("rating").and_then(rating),
    })
}

pub fn parse_alarms(v: &Value) -> AlarmListing {
    let sounds: Vec<AlarmSound> = v
        .get("sounds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(500)
        .filter_map(|s| {
            let id = s.get("id")?.as_str().filter(|i| sound_id_ok(i))?;
            Some(AlarmSound {
                id: id.into(),
                category: text(&s["category"], 20),
                title: text(&s["title"], 200),
            })
        })
        .collect();
    let alarms = v
        .get("alarms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
        .filter_map(|a| {
            let id = a.get("id")?.as_str().filter(|i| alarm_id_ok(i))?;
            let time_secs = a.get("time")?.as_u64().filter(|t| *t < 86_400)? as u32;
            let mut days: Vec<u8> = a
                .get("days")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_u64().filter(|d| *d <= 6).map(|d| d as u8))
                .collect();
            days.sort_unstable();
            days.dedup();
            let sound = a.get("sound");
            Some(AlarmInfo {
                id: id.into(),
                time_secs,
                days,
                enabled: a.get("enabled").and_then(Value::as_bool) == Some(true),
                repeat: a.get("repeat").and_then(Value::as_bool) != Some(false),
                volume: a
                    .get("volume")
                    .and_then(Value::as_u64)
                    .filter(|v| *v <= 100)
                    .map(|v| v as u8),
                shuffle: a
                    .get("shuffle")
                    .and_then(Value::as_u64)
                    .filter(|v| *v <= 2)
                    .unwrap_or(0) as u8,
                sound_id: sound
                    .and_then(|s| s.get("id"))
                    .and_then(Value::as_str)
                    .filter(|i| sound_id_ok(i))
                    .map(str::to_string),
                sound_title: sound.map(|s| text(&s["title"], 200)).unwrap_or_default(),
            })
        })
        .collect();
    AlarmListing {
        enabled: v.get("enabled").and_then(Value::as_bool) != Some(false),
        alarms,
        sounds,
        new_id: v
            .get("id")
            .and_then(Value::as_str)
            .filter(|i| alarm_id_ok(i))
            .map(str::to_string),
    }
}

/// The JSON body for a change, refusing anything out of range.
pub fn alarm_body(c: &AlarmChange) -> Result<Value, ApiError> {
    let mut o = serde_json::Map::new();
    if let Some(t) = c.time_secs {
        if t >= 86_400 {
            return Err(ApiError::invalid("bad time"));
        }
        o.insert("time".into(), json!(t));
    }
    if let Some(days) = &c.days {
        let mut d = days.clone();
        d.sort_unstable();
        d.dedup();
        if d.is_empty() || d.iter().any(|x| *x > 6) {
            return Err(ApiError::invalid("bad days"));
        }
        o.insert("days".into(), json!(d));
    }
    if let Some(e) = c.enabled {
        o.insert("enabled".into(), json!(e));
    }
    if let Some(r) = c.repeat {
        o.insert("repeat".into(), json!(r));
    }
    if let Some(v) = c.volume {
        o.insert("volume".into(), json!(v.min(100)));
    }
    if let Some(s) = c.shuffle {
        if s > 2 {
            return Err(ApiError::invalid("bad shuffle"));
        }
        o.insert("shuffle".into(), json!(s));
    }
    if let Some(s) = &c.sound_id {
        if !sound_id_ok(s) {
            return Err(ApiError::invalid("bad sound"));
        }
        o.insert("sound".into(), json!(s));
    }
    Ok(Value::Object(o))
}

/// A rating to send: 0–100 in steps of 10.
pub fn rating_value(v: u32) -> u32 {
    (v.min(100) + 5) / 10 * 10
}

fn player_ok(p: &str) -> Result<String, ApiError> {
    crate::commands::player_id(p)
}

/// The user's ratings and the players' alarms.
#[derive(uniffi::Object)]
pub struct Routine {
    session: Arc<Session>,
}

impl Routine {
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        crate::me::exchange(self.session.core(), method, path, body)
            .await
            .map(|(_, v)| v)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Routine {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Routine { session })
    }

    /// Ratings for these tracks (at most 200; others ignored).
    pub async fn ratings(&self, track_ids: Vec<i64>) -> Result<Vec<TrackRating>, ApiError> {
        let mut ids: Vec<i64> = track_ids
            .into_iter()
            .filter(|n| (1..=999_999_999_999).contains(n))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(200);
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let list: Vec<String> = ids.iter().map(i64::to_string).collect();
        let v = self
            .call(
                "GET",
                &format!("v1/me/ratings?tracks={}", list.join(",")),
                None,
            )
            .await?;
        Ok(v.get("ratings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_rating)
            .collect())
    }

    /// Rate a track (0 clears).
    pub async fn rate(&self, track_id: i64, value: u32) -> Result<Option<TrackRating>, ApiError> {
        if !(1..=999_999_999_999).contains(&track_id) {
            return Err(ApiError::invalid("bad track"));
        }
        let v = self
            .call(
                "PUT",
                &format!("v1/me/ratings/{track_id}"),
                Some(&json!({"rating": rating_value(value)})),
            )
            .await?;
        Ok(parse_rating(&v))
    }

    pub async fn rating_style(&self) -> Result<RatingStyle, ApiError> {
        let v = self.call("GET", "v1/me/ratings/style", None).await?;
        Ok(if v["style"] == "heart" {
            RatingStyle::Heart
        } else {
            RatingStyle::Stars
        })
    }

    pub async fn set_rating_style(&self, style: RatingStyle) -> Result<(), ApiError> {
        let s = if style == RatingStyle::Heart {
            "heart"
        } else {
            "stars"
        };
        self.call("PUT", "v1/me/ratings/style", Some(&json!({"style": s})))
            .await?;
        Ok(())
    }

    pub async fn clear_ratings(&self) -> Result<u64, ApiError> {
        let v = self.call("DELETE", "v1/me/ratings", None).await?;
        Ok(v.get("removed").and_then(Value::as_u64).unwrap_or(0))
    }

    pub async fn alarms(&self, player: String) -> Result<AlarmListing, ApiError> {
        let p = player_ok(&player)?;
        Ok(parse_alarms(
            &self
                .call("GET", &format!("v1/players/{p}/alarms"), None)
                .await?,
        ))
    }

    pub async fn add_alarm(
        &self,
        player: String,
        change: AlarmChange,
    ) -> Result<AlarmListing, ApiError> {
        let p = player_ok(&player)?;
        if change.time_secs.is_none() {
            return Err(ApiError::invalid("a new alarm needs a time"));
        }
        let body = alarm_body(&change)?;
        Ok(parse_alarms(
            &self
                .call("POST", &format!("v1/players/{p}/alarms"), Some(&body))
                .await?,
        ))
    }

    pub async fn change_alarm(
        &self,
        player: String,
        id: String,
        change: AlarmChange,
    ) -> Result<AlarmListing, ApiError> {
        let p = player_ok(&player)?;
        if !alarm_id_ok(&id) {
            return Err(ApiError::invalid("bad alarm id"));
        }
        let body = alarm_body(&change)?;
        Ok(parse_alarms(
            &self
                .call("PATCH", &format!("v1/players/{p}/alarms/{id}"), Some(&body))
                .await?,
        ))
    }

    pub async fn delete_alarm(&self, player: String, id: String) -> Result<AlarmListing, ApiError> {
        let p = player_ok(&player)?;
        if !alarm_id_ok(&id) {
            return Err(ApiError::invalid("bad alarm id"));
        }
        Ok(parse_alarms(
            &self
                .call("DELETE", &format!("v1/players/{p}/alarms/{id}"), None)
                .await?,
        ))
    }

    pub async fn set_all_alarms(
        &self,
        player: String,
        enabled: bool,
    ) -> Result<AlarmListing, ApiError> {
        let p = player_ok(&player)?;
        Ok(parse_alarms(
            &self
                .call(
                    "PUT",
                    &format!("v1/players/{p}/alarms/enabled"),
                    Some(&json!({"enabled": enabled})),
                )
                .await?,
        ))
    }
}

/// The next time one of `alarms` rings, as seconds after `now_secs` (Unix), counting days
/// from `weekday_now` (0 Sunday) and `secs_today` (seconds since local midnight).
#[uniffi::export]
pub fn next_alarm_in(alarms: Vec<AlarmInfo>, weekday_now: u8, secs_today: u32) -> Option<u32> {
    let mut best: Option<u32> = None;
    for a in alarms.iter().filter(|a| a.enabled && !a.days.is_empty()) {
        for ahead in 0..=7u32 {
            let day = ((weekday_now as u32 + ahead) % 7) as u8;
            if !a.days.contains(&day) {
                continue;
            }
            let at = ahead * 86_400 + a.time_secs;
            if at > secs_today {
                let wait = at - secs_today;
                best = Some(best.map_or(wait, |b| b.min(wait)));
                break;
            }
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Offline ratings
// ---------------------------------------------------------------------------

/// A rating made offline, waiting to be sent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueuedRating {
    pub track_id: i64,
    /// The download's stable key (64 hex digits).
    pub track_key: String,
    pub rating: u32,
    /// Unix seconds.
    pub at: u64,
}

/// Most queued ratings (one per track; the oldest go first beyond this).
pub const MAX_QUEUED: usize = 10_000;
/// Ratings sent per request.
pub const BATCH: usize = 500;

/// Add a rating to the queue: the newest per track key replaces an older one.
pub fn queue(q: &mut Vec<QueuedRating>, r: QueuedRating) -> bool {
    if !(1..=999_999_999_999).contains(&r.track_id)
        || r.track_key.len() != 64
        || !r
            .track_key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || r.rating > 100
    {
        return false;
    }
    let r = QueuedRating {
        rating: rating_value(r.rating),
        ..r
    };
    if let Some(i) = q.iter().position(|x| x.track_key == r.track_key) {
        if q[i].at > r.at {
            return false;
        }
        q.remove(i);
    }
    q.push(r);
    if q.len() > MAX_QUEUED {
        q.sort_by_key(|x| x.at);
        let n = q.len() - MAX_QUEUED;
        q.drain(..n);
    }
    true
}

/// The queue as stored on the device.
pub fn serialise_queue(q: &[QueuedRating]) -> Vec<u8> {
    serde_json::to_vec(&q.iter().map(|r| json!({"track": r.track_id, "key": r.track_key, "rating": r.rating, "at": r.at})).collect::<Vec<_>>())
        .unwrap_or_default()
}

/// The stored queue, checked (a damaged or tampered file loses only its bad entries).
pub fn parse_queue(bytes: &[u8]) -> Vec<QueuedRating> {
    let v: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let mut q = vec![];
    for e in v.as_array().into_iter().flatten().take(MAX_QUEUED * 2) {
        let (Some(track_id), Some(key), Some(rating), Some(at)) = (
            e.get("track").and_then(Value::as_i64),
            e.get("key").and_then(Value::as_str),
            e.get("rating").and_then(Value::as_u64),
            e.get("at").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let _ = queue(
            &mut q,
            QueuedRating {
                track_id,
                track_key: key.to_string(),
                rating: rating.min(101) as u32,
                at,
            },
        );
    }
    q
}

/// The request body. A time later than `now` (this device's clock was ahead) is sent as
/// `now`, which the server accepts.
pub fn batch_body(q: &[QueuedRating], now: u64) -> Value {
    let items: Vec<Value> = q
        .iter()
        .map(|r| json!({"track": r.track_id, "key": r.track_key, "rating": r.rating, "at": r.at.min(now)}))
        .collect();
    json!({ "ratings": items })
}

/// What sending the queue achieved.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RatingsSyncOutcome {
    pub sent: u32,
    pub pending: u32,
    /// The server has no ratings endpoint yet (an older version): kept for later.
    pub unsupported: bool,
}

enum Posted {
    Taken,
    /// 400/413: refused as malformed; sending it again can't help.
    Refused,
    /// 404/405: a server without `POST /v1/me/ratings`.
    Unsupported,
}

async fn post_batch(session: &Arc<Session>, chunk: &[QueuedRating]) -> Result<Posted, ApiError> {
    let core = session.core();
    let url = core.server().endpoint("v1/me/ratings");
    let body = batch_body(chunk, crate::offline::plays::now_secs());
    core.authed(|token| {
        let req = core
            .http()
            .post(url.clone())
            .bearer_auth(&token)
            .json(&body);
        async move {
            let resp = req.send().await?;
            match resp.status().as_u16() {
                404 | 405 => Ok(Posted::Unsupported),
                400 | 413 => Ok(Posted::Refused),
                s if (200..300).contains(&s) => Ok(Posted::Taken),
                _ => Err(crate::transport::error_from_response(resp).await),
            }
        }
    })
    .await
}

/// Send the ratings made offline. Safe to call often: a rating sent twice is harmless (the
/// newest per track wins on the server), and nothing is dropped until the server took it.
#[uniffi::export(async_runtime = "tokio")]
pub async fn sync_ratings(
    session: Arc<Session>,
    library: Arc<crate::offline::OfflineLibrary>,
) -> Result<RatingsSyncOutcome, ApiError> {
    if !session.has_sign_in() {
        return Err(ApiError::Unauthorised);
    }
    let mut sent = 0;
    loop {
        let q = library.queued_ratings();
        if q.is_empty() {
            break;
        }
        let chunk: Vec<QueuedRating> = q.into_iter().take(BATCH).collect();
        match post_batch(&session, &chunk).await? {
            Posted::Taken => {
                library.drop_ratings(&chunk);
                sent += chunk.len() as u32;
            }
            // Never block the queue on a batch the server won't take.
            Posted::Refused => library.drop_ratings(&chunk),
            Posted::Unsupported => {
                return Ok(RatingsSyncOutcome {
                    sent,
                    pending: library.queued_ratings().len() as u32,
                    unsupported: true,
                });
            }
        }
    }
    Ok(RatingsSyncOutcome {
        sent,
        pending: 0,
        unsupported: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> String {
        c.to_string().repeat(64)
    }

    #[test]
    fn answers_are_checked() {
        assert_eq!(
            parse_rating(&json!({"track": 9, "mine": 80, "shared": 60, "rating": 75})),
            Some(TrackRating {
                track_id: 9,
                mine: Some(80),
                shared: Some(60),
                rating: Some(75)
            })
        );
        assert_eq!(
            parse_rating(&json!({"track": 9, "mine": 101, "shared": -1}))
                .unwrap()
                .mine,
            None
        );
        for bad in [
            json!({}),
            json!({"track": 0}),
            json!({"track": "9"}),
            json!(null),
        ] {
            assert_eq!(parse_rating(&bad), None, "{bad}");
        }
        let l = parse_alarms(&json!({
            "enabled": false,
            "alarms": [
                {"id": "1a2b3c4d", "time": 25200, "days": [5, 1, 1, 9], "enabled": true, "repeat": false, "volume": 40, "shuffle": 2,
                 "sound": {"id": "current", "title": "Current queue"}},
                {"id": "XYZ", "time": 1}, {"id": "1", "time": 86400},
                {"id": "2", "time": 60, "sound": {"id": "http://evil", "title": "x\u{0}y"}}
            ],
            "sounds": [{"id": "current", "category": "current", "title": "Current queue"}, {"id": "file:///x"}],
            "id": "5e6f"
        }));
        assert!(!l.enabled);
        assert_eq!(l.alarms.len(), 2);
        assert_eq!(l.alarms[0].days, vec![1, 5]);
        assert!(!l.alarms[0].repeat);
        assert_eq!(l.alarms[1].sound_id, None);
        assert_eq!(l.alarms[1].sound_title, "xy");
        assert_eq!(l.sounds.len(), 1);
        assert_eq!(l.new_id.as_deref(), Some("5e6f"));
    }

    #[test]
    fn bodies_are_checked() {
        let b = alarm_body(&AlarmChange {
            time_secs: Some(0),
            days: Some(vec![3, 1, 3]),
            volume: Some(200),
            shuffle: Some(1),
            sound_id: Some("current".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            b,
            json!({"time": 0, "days": [1, 3], "volume": 100, "shuffle": 1, "sound": "current"})
        );
        for bad in [
            AlarmChange {
                time_secs: Some(86_400),
                ..Default::default()
            },
            AlarmChange {
                days: Some(vec![]),
                ..Default::default()
            },
            AlarmChange {
                days: Some(vec![7]),
                ..Default::default()
            },
            AlarmChange {
                shuffle: Some(3),
                ..Default::default()
            },
            AlarmChange {
                sound_id: Some("http://evil.example/".into()),
                ..Default::default()
            },
            AlarmChange {
                sound_id: Some("0123456789ABCDEF".into()),
                ..Default::default()
            },
        ] {
            assert!(alarm_body(&bad).is_err(), "{bad:?}");
        }
        assert_eq!(rating_value(84), 80);
        assert_eq!(rating_value(85), 90);
        assert_eq!(rating_value(999), 100);
    }

    #[test]
    fn the_next_alarm() {
        let a = |t: u32, days: Vec<u8>| AlarmInfo {
            id: "1".into(),
            time_secs: t,
            days,
            enabled: true,
            repeat: true,
            volume: None,
            shuffle: 0,
            sound_id: None,
            sound_title: String::new(),
        };
        // Monday 09:00.
        assert_eq!(
            next_alarm_in(vec![a(10 * 3600, vec![1])], 1, 9 * 3600),
            Some(3600)
        );
        assert_eq!(
            next_alarm_in(vec![a(7 * 3600, vec![1])], 1, 9 * 3600),
            Some(7 * 86_400 - 2 * 3600)
        );
        assert_eq!(
            next_alarm_in(
                vec![a(7 * 3600, vec![2]), a(6 * 3600, vec![2])],
                1,
                9 * 3600
            ),
            Some(86_400 - 3 * 3600)
        );
        let mut off = a(1, vec![1]);
        off.enabled = false;
        assert_eq!(next_alarm_in(vec![off], 1, 0), None);
    }

    #[test]
    fn the_offline_queue_keeps_the_newest_per_track() {
        let mut q = vec![];
        assert!(queue(
            &mut q,
            QueuedRating {
                track_id: 1,
                track_key: key('a'),
                rating: 84,
                at: 100
            }
        ));
        assert_eq!(q[0].rating, 80, "rounded to a half star");
        assert!(
            !queue(
                &mut q,
                QueuedRating {
                    track_id: 1,
                    track_key: key('a'),
                    rating: 20,
                    at: 50
                }
            ),
            "older"
        );
        assert!(queue(
            &mut q,
            QueuedRating {
                track_id: 1,
                track_key: key('a'),
                rating: 0,
                at: 200
            }
        ));
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].rating, 0);
        for bad in [
            QueuedRating {
                track_id: 0,
                track_key: key('a'),
                rating: 50,
                at: 1,
            },
            QueuedRating {
                track_id: 1,
                track_key: "x".into(),
                rating: 50,
                at: 1,
            },
            QueuedRating {
                track_id: 1,
                track_key: key('A'),
                rating: 50,
                at: 1,
            },
            QueuedRating {
                track_id: 1,
                track_key: key('a'),
                rating: 101,
                at: 1,
            },
        ] {
            assert!(!queue(&mut q, bad.clone()), "{bad:?}");
        }
        let body = batch_body(&q, 1_000);
        assert_eq!(
            body["ratings"][0],
            json!({"track": 1, "key": key('a'), "rating": 0, "at": 200})
        );
        assert_eq!(
            batch_body(&q, 150)["ratings"][0]["at"],
            150,
            "never later than this device's clock"
        );
        // The stored queue round-trips, and a damaged file loses only its bad entries.
        assert_eq!(parse_queue(&serialise_queue(&q)), q);
        let damaged = br#"[{"track":1,"key":"x","rating":50,"at":1},{"track":2,"key":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","rating":40,"at":5},"junk"]"#;
        assert_eq!(parse_queue(damaged).len(), 1);
        assert!(parse_queue(b"not json").is_empty());
    }
}
