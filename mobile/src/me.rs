//! The signed-in user's own things on the gateway (docs/plans/wave1-design.md §6–8):
//! listening history, the customisable Home, stations and pinned menus.
//!
//! - [`Me`]: the `/v1/me/…` calls, typed. Everything the gateway answers is read strictly
//!   ([`parse_history`], [`parse_layout`], [`parse_stations`], [`parse_batch`]): anything
//!   malformed is left out or refused, never guessed.
//! - Home layout editing ([`move_shelf`], [`set_hidden`], [`add_shelf`], [`remove_shelf`],
//!   [`layout_problem`]): pure, shared with the app's edit mode, checked as the gateway
//!   checks (`gateway/src/home.rs`).
//! - Smart filters ([`Me::filters`], [`Me::create_filter`], [`Me::update_filter`],
//!   [`Me::delete_filter`], [`Me::filter_albums`]; docs/plans/wave2-design.md §5.6): every
//!   filter is checked here as the gateway checks it ([`filter_problem`]) before it is sent.
//! - [`StationRunner`]: keeps a station going on a player. It starts the station, and on
//!   every status update asks for another batch when fewer than [`REFILL_BELOW`] tracks are
//!   left after the current one ([`refill_due`]), never two at once. The gateway leaves
//!   out the user's last 200 plays, what it served lately and what the runner says is
//!   queued. The web app follows the same rules (`web/app/src/lib/stations/refill.ts`).

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use crate::commands::{self, AlbumSort, ArtistRole, Command};
use crate::error::ApiError;
use crate::lyrics::{TrackLyrics, parse_lyrics};
use crate::menus::{self, MenuAction, MenuPage};
use crate::model::clean_text;
use crate::session::{Core, Session};
use crate::transport;

/// Refill when fewer than this many tracks are left after the current one.
pub const REFILL_BELOW: u32 = 5;
/// Tracks asked for in one batch.
pub const BATCH: u32 = 20;
/// Track ids sent as "already queued", at most (the gateway's limit).
pub const MAX_EXCLUDE: usize = 200;
/// Shelves on one Home, at most (as the gateway).
pub const MAX_SHELVES: usize = 24;
/// Menu steps from the root to an item, at most (as the gateway).
pub const MAX_STEPS: usize = 4;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One play in the user's history.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistoryEntry {
    pub track_id: i64,
    /// The stable track key (64 hex).
    pub track_key: String,
    pub album_id: Option<i64>,
    pub artist_id: Option<i64>,
    /// When it started (Unix seconds).
    pub played_at: i64,
    pub seconds: f64,
    /// Heard offline and sent back, rather than live through the gateway.
    pub offline: bool,
}

/// A page of history, newest first.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistoryPage {
    /// Is history being recorded?
    pub enabled: bool,
    pub retention_days: u32,
    pub items: Vec<HistoryEntry>,
    /// Pass as `before` for the next page; `None` at the end.
    pub next: Option<String>,
}

/// What a Home shelf shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ShelfKind {
    /// Continue / Recently played.
    Recent,
    Stations,
    /// New Music.
    New,
    Most,
    OnThisDay,
    Random,
    Favourites,
    Playlists,
    /// One station (`id`).
    Station,
    /// One pinned menu (`id`).
    Menu,
    /// Albums of one genre (`id`).
    Genre,
    /// One of the user's smart filters (`id`).
    Filter,
}

impl ShelfKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ShelfKind::Recent => "recent",
            ShelfKind::Stations => "stations",
            ShelfKind::New => "new",
            ShelfKind::Most => "most",
            ShelfKind::OnThisDay => "on_this_day",
            ShelfKind::Random => "random",
            ShelfKind::Favourites => "favourites",
            ShelfKind::Playlists => "playlists",
            ShelfKind::Station => "station",
            ShelfKind::Menu => "menu",
            ShelfKind::Genre => "genre",
            ShelfKind::Filter => "filter",
        }
    }

    fn parse(s: &str) -> Option<ShelfKind> {
        Some(match s {
            "recent" => ShelfKind::Recent,
            "stations" => ShelfKind::Stations,
            "new" => ShelfKind::New,
            "most" => ShelfKind::Most,
            "on_this_day" => ShelfKind::OnThisDay,
            "random" => ShelfKind::Random,
            "favourites" => ShelfKind::Favourites,
            "playlists" => ShelfKind::Playlists,
            "station" => ShelfKind::Station,
            "menu" => ShelfKind::Menu,
            "genre" => ShelfKind::Genre,
            "filter" => ShelfKind::Filter,
            _ => return None,
        })
    }

    /// Does this shelf name something by id?
    pub fn takes_id(self) -> bool {
        matches!(
            self,
            ShelfKind::Station | ShelfKind::Menu | ShelfKind::Genre | ShelfKind::Filter
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HomeShelf {
    pub kind: ShelfKind,
    pub id: Option<i64>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HomeLayout {
    pub shelves: Vec<HomeShelf>,
    /// Nothing saved yet: the default Home.
    pub is_default: bool,
}

/// What a station plays.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StationSpec {
    /// One or more genres (at most 5); one genre plays as LMS's Random Mix.
    Genre { genres: Vec<i64> },
    /// A decade (1980, …); `mine` favours what the user played from it.
    Decade { decade: u32, mine: bool },
    /// Often played, not heard for a long time.
    Forgotten,
    /// Recently added, barely played.
    Fresh,
    /// Rarely played tracks by artists the user plays a lot.
    DeepCuts,
    /// Pinned from a menu: replayed (and re-checked) by the gateway each time.
    Menu,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Station {
    pub id: i64,
    pub name: String,
    pub spec: StationSpec,
    /// Why it no longer works (a pinned menu item LMS stopped offering).
    pub broken: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PinnedMenu {
    pub id: i64,
    pub name: String,
    pub broken: Option<String>,
}

/// One menu on the way from a root menu to an item, as the app browsed it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MenuStep {
    /// A menu opened by an action, at this page.
    Opened {
        action: MenuAction,
        start: u32,
        count: u32,
    },
    /// The player's home menu (plugin entries are shown from it).
    HomeMenu,
}

/// What the gateway said to do for a station's next batch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StationBatch {
    /// Load (start) or add (refill) these library tracks.
    Tracks { ids: Vec<i64> },
    /// Send this command (LMS's Random Mix); LMS keeps the queue filled itself.
    RandomPlay { command: Vec<String> },
    /// The gateway ran the station's menu action; `refill`: it can add more.
    Ran { refill: bool },
    /// Nothing more to add.
    NoMore,
    /// A pinned menu item LMS no longer offers.
    Broken { reason: String },
}

// ---------------------------------------------------------------------------
// Reading the gateway's answers (pure)
// ---------------------------------------------------------------------------

fn pos_id(v: &Value) -> Option<i64> {
    v.as_i64().filter(|n| (1..=999_999_999_999).contains(n))
}

fn opt_id(v: &Value) -> Option<i64> {
    pos_id(v)
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A history cursor as the gateway makes them (`<played_at>-<id>`).
pub fn cursor_ok(s: &str) -> bool {
    s.split_once('-').is_some_and(|(a, b)| {
        [a, b]
            .iter()
            .all(|x| !x.is_empty() && x.len() <= 15 && x.bytes().all(|c| c.is_ascii_digit()))
    })
}

pub fn parse_history(v: &Value) -> Result<HistoryPage, ApiError> {
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_response("no history"))?;
    let items = items
        .iter()
        .filter_map(|e| {
            let key = e.get("track_key")?.as_str().filter(|k| hex64(k))?;
            Some(HistoryEntry {
                track_id: pos_id(e.get("track_id")?)?,
                track_key: key.to_ascii_lowercase(),
                album_id: e.get("album_id").and_then(opt_id),
                artist_id: e.get("artist_id").and_then(opt_id),
                played_at: e.get("played_at")?.as_i64().filter(|t| *t >= 0)?,
                seconds: e
                    .get("seconds")?
                    .as_f64()
                    .filter(|s| s.is_finite() && *s >= 0.0)?,
                offline: e.get("source").and_then(Value::as_str) == Some("offline"),
            })
        })
        .collect();
    Ok(HistoryPage {
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        retention_days: v
            .get("retention_days")
            .and_then(Value::as_u64)
            .and_then(|d| u32::try_from(d).ok())
            .unwrap_or(396),
        items,
        next: v
            .get("next")
            .and_then(Value::as_str)
            .filter(|c| cursor_ok(c))
            .map(str::to_string),
    })
}

/// Album ids from a shelf answer, in order.
pub fn parse_albums(v: &Value) -> Vec<i64> {
    v.get("albums")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(pos_id).collect())
        .unwrap_or_default()
}

fn shelf_from(v: &Value) -> Option<HomeShelf> {
    let o = v.as_object()?;
    let kind = ShelfKind::parse(o.get("type")?.as_str()?)?;
    let id = match o.get("id") {
        None => None,
        Some(v) => Some(pos_id(v)?),
    };
    if kind.takes_id() != id.is_some() {
        return None;
    }
    Some(HomeShelf {
        kind,
        id,
        hidden: o.get("hidden").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The gateway's layout. Unknown shelves (a newer gateway's) are left out.
pub fn parse_layout(v: &Value) -> Result<HomeLayout, ApiError> {
    if v.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(ApiError::bad_response("unknown Home layout version"));
    }
    let shelves = v
        .get("shelves")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_response("no shelves"))?;
    Ok(HomeLayout {
        shelves: shelves
            .iter()
            .filter_map(shelf_from)
            .take(MAX_SHELVES)
            .collect(),
        is_default: v.get("default").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The layout as the gateway takes it.
pub fn layout_json(l: &HomeLayout) -> Value {
    let shelves: Vec<Value> = l
        .shelves
        .iter()
        .map(|s| {
            let mut o = Map::new();
            o.insert("type".into(), s.kind.as_str().into());
            if let Some(id) = s.id {
                o.insert("id".into(), id.into());
            }
            if s.hidden {
                o.insert("hidden".into(), true.into());
            }
            Value::Object(o)
        })
        .collect();
    json!({"version": 1, "shelves": shelves})
}

fn spec_from(v: &Value) -> Option<StationSpec> {
    Some(match v.get("kind")?.as_str()? {
        "genre" => StationSpec::Genre {
            genres: v
                .get("genres")?
                .as_array()?
                .iter()
                .map(pos_id)
                .collect::<Option<_>>()?,
        },
        "decade" => StationSpec::Decade {
            decade: u32::try_from(v.get("decade")?.as_u64()?).ok()?,
            mine: v.get("mine").and_then(Value::as_bool).unwrap_or(false),
        },
        "forgotten" => StationSpec::Forgotten,
        "fresh" => StationSpec::Fresh,
        "deep_cuts" => StationSpec::DeepCuts,
        "menu" => StationSpec::Menu,
        _ => return None,
    })
}

/// Bidirectional and invisible formatting characters, which could make a name read as
/// something else.
fn is_format(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

fn name_of(v: &Value) -> Option<String> {
    let n: String = clean_text(v.get("name")?.as_str()?)
        .chars()
        .filter(|c| !is_format(*c))
        .collect();
    let n: String = n.chars().take(64).collect();
    (!n.trim().is_empty()).then_some(n)
}

fn broken_of(v: &Value) -> Option<String> {
    v.get("broken")
        .and_then(Value::as_str)
        .map(|r| clean_text(r).chars().take(200).collect())
}

pub fn parse_stations(v: &Value) -> Vec<Station> {
    v.get("stations")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(Station {
                        id: pos_id(s.get("id")?)?,
                        name: name_of(s)?,
                        spec: spec_from(s.get("spec")?)?,
                        broken: broken_of(s),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_menus(v: &Value) -> Vec<PinnedMenu> {
    v.get("menus")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(PinnedMenu {
                        id: pos_id(s.get("id")?)?,
                        name: name_of(s)?,
                        broken: broken_of(s),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A station batch answer. A Random Mix command must be exactly the shape the gateway's
/// allow-list takes (`randomplay <mode> [genre_id:<n>]`): anything else is refused here too.
pub fn parse_batch(v: &Value) -> Result<StationBatch, ApiError> {
    let bad = || ApiError::bad_response("unexpected station answer");
    Ok(match v.get("mode").and_then(Value::as_str) {
        Some("tracks") => StationBatch::Tracks {
            ids: v
                .get("tracks")
                .and_then(Value::as_array)
                .ok_or_else(bad)?
                .iter()
                .filter_map(pos_id)
                .take(50)
                .collect(),
        },
        Some("randomplay") => {
            let words: Vec<String> = v
                .get("command")
                .and_then(Value::as_array)
                .ok_or_else(bad)?
                .iter()
                .map(|w| w.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .ok_or_else(bad)?;
            if !random_mix_ok(&words) {
                return Err(bad());
            }
            StationBatch::RandomPlay { command: words }
        }
        Some("ran") => StationBatch::Ran {
            refill: v.get("refill").and_then(Value::as_bool).unwrap_or(false),
        },
        Some("none") => StationBatch::NoMore,
        _ => return Err(bad()),
    })
}

/// `randomplay <tracks|albums|…> [genre_id:<n>]`, as the gateway allows it.
pub fn random_mix_ok(words: &[String]) -> bool {
    const MODES: [&str; 6] = [
        "tracks",
        "albums",
        "contributors",
        "artists",
        "year",
        "works",
    ];
    match words {
        [w, mode] => w == "randomplay" && MODES.contains(&mode.as_str()),
        [w, mode, g] => {
            w == "randomplay"
                && MODES.contains(&mode.as_str())
                && g.strip_prefix("genre_id:").is_some_and(|n| {
                    !n.is_empty()
                        && n.len() <= 12
                        && !n.starts_with('0')
                        && n.bytes().all(|b| b.is_ascii_digit())
                })
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Editing the Home layout (pure, shared with the app's edit mode)
// ---------------------------------------------------------------------------

/// Move the shelf at `from` to `to` (drag and drop, or "Move up/down"). Out of range: no
/// change.
#[uniffi::export]
pub fn move_shelf(layout: HomeLayout, from: u32, to: u32) -> HomeLayout {
    let mut l = layout;
    let (from, to) = (from as usize, to as usize);
    if from < l.shelves.len() && to < l.shelves.len() && from != to {
        let s = l.shelves.remove(from);
        l.shelves.insert(to, s);
        l.is_default = false;
    }
    l
}

#[uniffi::export]
pub fn set_hidden(layout: HomeLayout, index: u32, hidden: bool) -> HomeLayout {
    let mut l = layout;
    if let Some(s) = l.shelves.get_mut(index as usize) {
        s.hidden = hidden;
        l.is_default = false;
    }
    l
}

/// Add a shelf at the end. Refused (layout unchanged, `Err`) if it is malformed, already
/// there, or the Home is full.
#[uniffi::export]
pub fn add_shelf(layout: HomeLayout, shelf: HomeShelf) -> Result<HomeLayout, ApiError> {
    let mut l = layout;
    l.shelves.push(HomeShelf {
        hidden: false,
        ..shelf
    });
    if let Some(problem) = layout_problem(l.clone()) {
        return Err(ApiError::invalid(problem));
    }
    l.is_default = false;
    Ok(l)
}

#[uniffi::export]
pub fn remove_shelf(layout: HomeLayout, index: u32) -> HomeLayout {
    let mut l = layout;
    if (index as usize) < l.shelves.len() {
        l.shelves.remove(index as usize);
        l.is_default = false;
    }
    l
}

/// What the gateway would refuse in this layout (`None`: nothing).
#[uniffi::export]
pub fn layout_problem(layout: HomeLayout) -> Option<String> {
    if layout.shelves.len() > MAX_SHELVES {
        return Some("a Home has at most 24 shelves".into());
    }
    let mut seen = std::collections::HashSet::new();
    for s in &layout.shelves {
        if s.kind.takes_id() != s.id.is_some() {
            return Some("a shelf is missing what it shows".into());
        }
        if s.id.is_some_and(|id| !(1..=999_999_999_999).contains(&id)) {
            return Some("bad id".into());
        }
        if !seen.insert((s.kind, s.id)) {
            return Some("that shelf is already on Home".into());
        }
    }
    None
}

/// The Home every user starts with.
#[uniffi::export]
pub fn default_home() -> HomeLayout {
    let s = |kind| HomeShelf {
        kind,
        id: None,
        hidden: false,
    };
    HomeLayout {
        shelves: vec![
            s(ShelfKind::Recent),
            s(ShelfKind::Stations),
            s(ShelfKind::New),
            s(ShelfKind::Most),
            s(ShelfKind::OnThisDay),
        ],
        is_default: true,
    }
}

// ---------------------------------------------------------------------------
// Station refill rules (pure)
// ---------------------------------------------------------------------------

/// Is it time for another batch? `tracks` in the queue, `current` the playing index.
#[uniffi::export]
pub fn refill_due(tracks: u32, current: u32) -> bool {
    tracks.saturating_sub(current.saturating_add(1)) < REFILL_BELOW
}

/// The commands that queue a batch: the first replaces the queue when starting
/// (`load`), the rest add; each id list fits the gateway's limit.
pub fn batch_commands(ids: &[i64], start: bool) -> Result<Vec<Command>, ApiError> {
    commands::chunk_ids(ids, commands::ID_LIST_MAX)
        .iter()
        .enumerate()
        .map(|(i, chunk)| commands::load_tracks(chunk, !(start && i == 0), None))
        .collect()
}

fn step_command(step: &MenuStep) -> Result<Vec<Value>, ApiError> {
    match step {
        MenuStep::HomeMenu => Ok(commands::home_menu()),
        MenuStep::Opened {
            action,
            start,
            count,
        } => {
            if !action.opens_menu || action.needs_input {
                return Err(ApiError::invalid("this menu can't be saved"));
            }
            menus::to_command(action, Some((*start, *count)), None)
        }
    }
}

fn action_command(action: &MenuAction) -> Result<Vec<Value>, ApiError> {
    if action.opens_menu || action.needs_input {
        return Err(ApiError::invalid("this item can't be saved as a station"));
    }
    menus::to_command(action, None, None)
}

/// The spec of a station pinned from a menu: the steps to the item, and its actions.
pub fn menu_station_spec(
    steps: &[MenuStep],
    play: &MenuAction,
    add: Option<&MenuAction>,
) -> Result<Value, ApiError> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(ApiError::invalid("this menu is too deep to save"));
    }
    let steps: Vec<Vec<Value>> = steps.iter().map(step_command).collect::<Result<_, _>>()?;
    let mut spec = json!({"kind": "menu", "steps": steps, "play": action_command(play)?});
    if let Some(a) = add {
        spec["add"] = json!(action_command(a)?);
    }
    Ok(spec)
}

fn spec_json(spec: &StationSpec) -> Result<Value, ApiError> {
    Ok(match spec {
        StationSpec::Genre { genres } => {
            if genres.is_empty() || genres.len() > 5 || genres.iter().any(|g| *g <= 0) {
                return Err(ApiError::invalid("choose 1 to 5 genres"));
            }
            json!({"kind": "genre", "genres": genres})
        }
        StationSpec::Decade { decade, mine } => {
            if !(1900..=2090).contains(decade) || decade % 10 != 0 {
                return Err(ApiError::invalid("choose a decade"));
            }
            json!({"kind": "decade", "decade": decade, "mine": mine})
        }
        StationSpec::Forgotten => json!({"kind": "forgotten"}),
        StationSpec::Fresh => json!({"kind": "fresh"}),
        StationSpec::DeepCuts => json!({"kind": "deep_cuts"}),
        StationSpec::Menu => {
            return Err(ApiError::invalid("save a menu station from its menu"));
        }
    })
}

// ---------------------------------------------------------------------------
// The calls
// ---------------------------------------------------------------------------

/// What a `/v1/me/…` call answered: status and JSON.
pub(crate) async fn exchange(
    core: &Arc<Core>,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), ApiError> {
    let url = core.server().endpoint(path);
    core.authed(|token| {
        let url = url.clone();
        async move {
            let http = core.http();
            let mut req = match method {
                "GET" => http.get(url),
                "POST" => http.post(url),
                "PUT" => http.put(url),
                "PATCH" => http.patch(url),
                _ => http.delete(url),
            }
            .bearer_auth(&token);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await?;
            let status = resp.status().as_u16();
            // 409: a pinned menu item that LMS no longer offers (the body says why).
            if status == 409 {
                let bytes = resp.read_limited(64 * 1024).await.unwrap_or_default();
                return Ok((
                    status,
                    serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                ));
            }
            if !(200..300).contains(&status) {
                return Err(transport::error_from_response(resp).await);
            }
            let bytes = resp.read_limited(transport::MAX_JSON).await?;
            let v = serde_json::from_slice(&bytes)
                .map_err(|_| ApiError::bad_response("malformed answer"))?;
            Ok((status, v))
        }
    })
    .await
}

/// The user's history, Home, stations and pinned menus.
#[derive(uniffi::Object)]
pub struct Me {
    session: Arc<Session>,
}

impl Me {
    fn core(&self) -> &Arc<Core> {
        self.session.core()
    }

    /// A `/v1/me` call. A 409 (too many stations, menus or filters) is a refusal with the
    /// gateway's reason, like any other 4xx; it used to read as a malformed answer.
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        match exchange(self.core(), method, path, body).await? {
            (409, v) => Err(ApiError::invalid(
                v.get("error")
                    .and_then(Value::as_str)
                    .map(|e| clean_text(e).chars().take(200).collect::<String>())
                    .unwrap_or_else(|| "too many".into()),
            )),
            (_, v) => Ok(v),
        }
    }

    /// A filter call (the same rules as [`Self::call`]).
    async fn filter_call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        self.call(method, path, body).await
    }
}

fn check_id(id: i64) -> Result<i64, ApiError> {
    if (1..=999_999_999_999).contains(&id) {
        Ok(id)
    } else {
        Err(ApiError::invalid("bad id"))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Me {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Me { session })
    }

    /// A page of history (`limit` 1–200), before a cursor from the previous page.
    pub async fn history(
        &self,
        limit: u32,
        before: Option<String>,
    ) -> Result<HistoryPage, ApiError> {
        let limit = limit.clamp(1, 200);
        let mut path = format!("v1/me/history?limit={limit}");
        if let Some(c) = before {
            if !cursor_ok(&c) {
                return Err(ApiError::invalid("bad cursor"));
            }
            path.push_str(&format!("&before={c}"));
        }
        parse_history(&self.call("GET", &path, None).await?)
    }

    /// Forget the whole history. Returns how many plays were removed.
    pub async fn clear_history(&self) -> Result<u64, ApiError> {
        let v = self.call("DELETE", "v1/me/history", None).await?;
        Ok(v.get("removed").and_then(Value::as_u64).unwrap_or(0))
    }

    /// Switch recording on or off.
    pub async fn set_history_enabled(&self, enabled: bool) -> Result<bool, ApiError> {
        let v = self
            .call("PATCH", "v1/me/history", Some(&json!({"enabled": enabled})))
            .await?;
        Ok(v.get("enabled").and_then(Value::as_bool).unwrap_or(enabled))
    }

    /// Album ids for a history shelf (`Recent`, `Most` or `OnThisDay`), best first.
    pub async fn shelf_albums(&self, kind: ShelfKind, limit: u32) -> Result<Vec<i64>, ApiError> {
        if !matches!(
            kind,
            ShelfKind::Recent | ShelfKind::Most | ShelfKind::OnThisDay
        ) {
            return Err(ApiError::invalid("not a history shelf"));
        }
        let path = format!(
            "v1/me/shelves/{}?limit={}",
            kind.as_str(),
            limit.clamp(1, 50)
        );
        Ok(parse_albums(&self.call("GET", &path, None).await?))
    }

    pub async fn home(&self) -> Result<HomeLayout, ApiError> {
        parse_layout(&self.call("GET", "v1/me/home", None).await?)
    }

    pub async fn save_home(&self, layout: HomeLayout) -> Result<HomeLayout, ApiError> {
        if let Some(p) = layout_problem(layout.clone()) {
            return Err(ApiError::invalid(p));
        }
        parse_layout(
            &self
                .call("PUT", "v1/me/home", Some(&layout_json(&layout)))
                .await?,
        )
    }

    pub async fn reset_home(&self) -> Result<HomeLayout, ApiError> {
        parse_layout(&self.call("DELETE", "v1/me/home", None).await?)
    }

    pub async fn stations(&self) -> Result<Vec<Station>, ApiError> {
        Ok(parse_stations(
            &self.call("GET", "v1/me/stations", None).await?,
        ))
    }

    /// Add a built-in station (genre, decade, forgotten favourites, fresh, deep cuts).
    pub async fn create_station(&self, name: String, spec: StationSpec) -> Result<i64, ApiError> {
        let body = json!({"name": clean_text(&name), "spec": spec_json(&spec)?});
        let v = self.call("POST", "v1/me/stations", Some(&body)).await?;
        v.get("id")
            .and_then(pos_id)
            .ok_or_else(|| ApiError::bad_response("no id"))
    }

    /// "Save as station" on a menu item: the menus that led to it, and its play (and add)
    /// actions. The gateway checks LMS offers it now, on `player`, before saving.
    pub async fn save_menu_station(
        &self,
        name: String,
        steps: Vec<MenuStep>,
        play: MenuAction,
        add: Option<MenuAction>,
        player: String,
    ) -> Result<i64, ApiError> {
        let spec = menu_station_spec(&steps, &play, add.as_ref())?;
        let body = json!({"name": clean_text(&name), "spec": spec, "player": commands::player_id(&player)?});
        let v = self.call("POST", "v1/me/stations", Some(&body)).await?;
        v.get("id")
            .and_then(pos_id)
            .ok_or_else(|| ApiError::bad_response("no id"))
    }

    pub async fn delete_station(&self, id: i64) -> Result<(), ApiError> {
        self.call("DELETE", &format!("v1/me/stations/{}", check_id(id)?), None)
            .await
            .map(|_| ())
    }

    /// A station's next batch for `player` (`start`: it begins now), leaving out
    /// `queued` (track ids already in the queue).
    pub async fn next_batch(
        &self,
        id: i64,
        player: String,
        start: bool,
        queued: Vec<i64>,
    ) -> Result<StationBatch, ApiError> {
        let exclude: Vec<i64> = queued
            .into_iter()
            .filter(|t| (1..=999_999_999_999).contains(t))
            .take(MAX_EXCLUDE)
            .collect();
        let body = json!({"player": commands::player_id(&player)?, "start": start, "count": BATCH, "exclude": exclude});
        let path = format!("v1/me/stations/{}/next", check_id(id)?);
        let (status, v) = exchange(self.core(), "POST", &path, Some(&body)).await?;
        if status == 409 {
            let reason = v
                .get("error")
                .and_then(Value::as_str)
                .map(|r| clean_text(r).chars().take(200).collect())
                .unwrap_or_else(|| "This station's menu item is no longer offered.".into());
            return Ok(StationBatch::Broken { reason });
        }
        parse_batch(&v)
    }

    pub async fn menus(&self) -> Result<Vec<PinnedMenu>, ApiError> {
        Ok(parse_menus(&self.call("GET", "v1/me/menus", None).await?))
    }

    /// "Pin to Home" for a menu: the menus that lead to it (the last one is the menu
    /// itself). Checked by the gateway on `player` first.
    pub async fn pin_menu(
        &self,
        name: String,
        steps: Vec<MenuStep>,
        player: String,
    ) -> Result<i64, ApiError> {
        if steps.is_empty() || steps.len() > MAX_STEPS {
            return Err(ApiError::invalid("this menu is too deep to pin"));
        }
        let steps: Vec<Vec<Value>> = steps.iter().map(step_command).collect::<Result<_, _>>()?;
        let body = json!({"name": clean_text(&name), "steps": steps, "player": commands::player_id(&player)?});
        let v = self.call("POST", "v1/me/menus", Some(&body)).await?;
        v.get("id")
            .and_then(pos_id)
            .ok_or_else(|| ApiError::bad_response("no id"))
    }

    pub async fn delete_menu(&self, id: i64) -> Result<(), ApiError> {
        self.call("DELETE", &format!("v1/me/menus/{}", check_id(id)?), None)
            .await
            .map(|_| ())
    }

    /// The user's smart filters, oldest first.
    pub async fn filters(&self) -> Result<Vec<SmartFilter>, ApiError> {
        Ok(parse_filters(
            &self.call("GET", "v1/me/filters", None).await?,
        ))
    }

    /// Save a new smart filter (checked here first, see [`filter_problem`]). Returns it as
    /// saved, with its id.
    pub async fn create_filter(
        &self,
        name: String,
        rules: Vec<FilterRule>,
        sort: AlbumSort,
        limit: u32,
    ) -> Result<SmartFilter, ApiError> {
        let body = filter_json(&name, &rules, sort, limit)?;
        let v = self
            .filter_call("POST", "v1/me/filters", Some(&body))
            .await?;
        filter_from(&v).ok_or_else(|| ApiError::bad_response("unexpected filter answer"))
    }

    /// Replace one of the user's smart filters (checked here first).
    pub async fn update_filter(
        &self,
        id: i64,
        name: String,
        rules: Vec<FilterRule>,
        sort: AlbumSort,
        limit: u32,
    ) -> Result<SmartFilter, ApiError> {
        let path = format!("v1/me/filters/{}", check_id(id)?);
        let body = filter_json(&name, &rules, sort, limit)?;
        let v = self.filter_call("PUT", &path, Some(&body)).await?;
        filter_from(&v).ok_or_else(|| ApiError::bad_response("unexpected filter answer"))
    }

    pub async fn delete_filter(&self, id: i64) -> Result<(), ApiError> {
        self.call("DELETE", &format!("v1/me/filters/{}", check_id(id)?), None)
            .await
            .map(|_| ())
    }

    /// A page of a filter's albums: `count` (1–100) from `from` (0–500). The first page
    /// runs the filter afresh (a new order for a random one); later pages continue it.
    pub async fn filter_albums(
        &self,
        id: i64,
        from: u32,
        count: u32,
    ) -> Result<FilterAlbums, ApiError> {
        if from > MAX_FILTER_LIMIT || !(1..=MAX_FILTER_PAGE).contains(&count) {
            return Err(ApiError::invalid("bad page"));
        }
        let path = format!(
            "v1/me/filters/{}/albums?from={from}&n={count}",
            check_id(id)?
        );
        parse_filter_albums(&self.call("GET", &path, None).await?)
    }

    /// Open a pinned menu on `player`: the menu as LMS offers it now.
    /// A library track's lyrics (from its tags, or the Music & Artist Information plugin on
    /// the server), or `None` when it has none.
    pub async fn lyrics(&self, track_id: i64) -> Result<Option<TrackLyrics>, ApiError> {
        let path = format!("v1/lyrics/track/{}", check_id(track_id)?);
        let core = self.core();
        let url = core.server().endpoint(&path);
        core.authed(|token| {
            let req = core.http().get(url.clone()).bearer_auth(&token);
            async move {
                let resp = req.send().await?;
                match resp.status().as_u16() {
                    // "no lyrics" (or a track LMS doesn't know): nothing to show.
                    404 => Ok(None),
                    200 => {
                        let bytes = resp.read_limited(transport::MAX_JSON).await?;
                        let v: Value = serde_json::from_slice(&bytes)
                            .map_err(|_| ApiError::bad_response("malformed answer"))?;
                        Ok(parse_lyrics(&v))
                    }
                    _ => Err(transport::error_from_response(resp).await),
                }
            }
        })
        .await
    }

    pub async fn open_menu(&self, id: i64, player: String) -> Result<MenuPage, ApiError> {
        let body = json!({"player": commands::player_id(&player)?});
        let path = format!("v1/me/menus/{}/open", check_id(id)?);
        let (status, v) = exchange(self.core(), "POST", &path, Some(&body)).await?;
        if status == 409 {
            return Err(ApiError::Forbidden {
                detail: v
                    .get("error")
                    .and_then(Value::as_str)
                    .map(clean_text)
                    .unwrap_or_else(|| "this menu is no longer offered".into()),
            });
        }
        Ok(menus::page_from(v.get("result").unwrap_or(&Value::Null)))
    }
}

// ---------------------------------------------------------------------------
// Smart filters (pure)
// ---------------------------------------------------------------------------

/// Filters one user may keep (as the gateway).
pub const MAX_FILTERS: usize = 50;
/// Rules in one filter, at most (as the gateway).
pub const MAX_FILTER_RULES: usize = 8;
/// Genres in one genre rule, at most (as the gateway).
pub const MAX_FILTER_GENRES: usize = 20;
/// Albums one filter produces, at most (as the gateway).
pub const MAX_FILTER_LIMIT: u32 = 500;
/// Albums in one page of a filter's results, at most (as the gateway).
pub const MAX_FILTER_PAGE: u32 = 100;
/// Largest genre or artist id a rule may name (as the gateway).
pub const MAX_RULE_ID: i64 = 999_999_999;

/// One rule of a smart filter. Every rule must match.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FilterRule {
    /// The album has any of these genres (1–20 LMS genre ids).
    Genre { genres: Vec<i64> },
    /// The album's year is in this decade (1900, 1910, … 2090).
    Decade { decade: u32 },
    /// By this artist (an LMS contributor id), in a role if given.
    Artist {
        artist: i64,
        role: Option<ArtistRole>,
    },
    /// It is (or isn't) a compilation.
    Compilation { compilation: bool },
}

impl FilterRule {
    fn field(&self) -> &'static str {
        match self {
            FilterRule::Genre { .. } => "genre",
            FilterRule::Decade { .. } => "decade",
            FilterRule::Artist { .. } => "artist",
            FilterRule::Compilation { .. } => "compilation",
        }
    }
}

/// A saved smart filter.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SmartFilter {
    pub id: i64,
    pub name: String,
    pub rules: Vec<FilterRule>,
    pub sort: AlbumSort,
    /// Albums it produces, at most (1–500).
    pub limit: u32,
}

/// A page of a filter's albums.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilterAlbums {
    /// Album ids, in the filter's order; load details by id, as for shelves.
    pub albums: Vec<i64>,
    /// How many albums the filter produces in all (at most its limit).
    pub count: u32,
}

fn sort_word(s: AlbumSort) -> &'static str {
    match s {
        AlbumSort::Title => "album",
        AlbumSort::Artist => "artistalbum",
        AlbumSort::Year => "yearalbum",
        AlbumSort::Newest => "new",
        AlbumSort::Random => "random",
    }
}

fn sort_of(s: &str) -> Option<AlbumSort> {
    Some(match s {
        "album" => AlbumSort::Title,
        "artistalbum" => AlbumSort::Artist,
        "yearalbum" => AlbumSort::Year,
        "new" => AlbumSort::Newest,
        "random" => AlbumSort::Random,
        _ => return None,
    })
}

const ROLES: [ArtistRole; 6] = [
    ArtistRole::AlbumArtist,
    ArtistRole::Artist,
    ArtistRole::Composer,
    ArtistRole::Conductor,
    ArtistRole::Band,
    ArtistRole::TrackArtist,
];

fn role_of(s: &str) -> Option<ArtistRole> {
    ROLES.into_iter().find(|r| r.lms() == s)
}

fn rule_id_ok(id: i64) -> bool {
    (1..=MAX_RULE_ID).contains(&id)
}

/// A filter name as it will be sent: control and invisible formatting characters removed
/// and trimmed. `None` if nothing is left or it is over 64 characters.
fn filter_name(name: &str) -> Option<String> {
    let n: String = clean_text(name)
        .chars()
        .filter(|c| !is_format(*c))
        .collect();
    let n = n.trim();
    (!n.is_empty() && n.chars().count() <= 64).then(|| n.to_string())
}

/// What the gateway would refuse in this filter (`None`: nothing), checked before it is
/// sent. Every sort is allowed.
#[uniffi::export]
pub fn filter_problem(name: String, rules: Vec<FilterRule>, limit: u32) -> Option<String> {
    if filter_name(&name).is_none() {
        return Some("a filter needs a name of 1 to 64 characters".into());
    }
    if rules.is_empty() || rules.len() > MAX_FILTER_RULES {
        return Some("a filter has 1 to 8 rules".into());
    }
    let mut seen = std::collections::HashSet::new();
    for r in &rules {
        if !seen.insert(r.field()) {
            return Some("each kind of rule can be used once".into());
        }
        match r {
            FilterRule::Genre { genres } => {
                if genres.is_empty() || genres.len() > MAX_FILTER_GENRES {
                    return Some("choose 1 to 20 genres".into());
                }
                if !genres.iter().all(|g| rule_id_ok(*g)) {
                    return Some("bad genre".into());
                }
                let distinct: std::collections::HashSet<_> = genres.iter().collect();
                if distinct.len() != genres.len() {
                    return Some("a genre is chosen twice".into());
                }
            }
            FilterRule::Decade { decade } => {
                if !(1900..=2090).contains(decade) || decade % 10 != 0 {
                    return Some("choose a decade".into());
                }
            }
            FilterRule::Artist { artist, .. } => {
                if !rule_id_ok(*artist) {
                    return Some("bad artist".into());
                }
            }
            FilterRule::Compilation { .. } => {}
        }
    }
    if !(1..=MAX_FILTER_LIMIT).contains(&limit) {
        return Some("a filter shows 1 to 500 albums".into());
    }
    None
}

/// A filter as the gateway takes it; refused here first if [`filter_problem`] finds one.
pub fn filter_json(
    name: &str,
    rules: &[FilterRule],
    sort: AlbumSort,
    limit: u32,
) -> Result<Value, ApiError> {
    if let Some(p) = filter_problem(name.to_string(), rules.to_vec(), limit) {
        return Err(ApiError::invalid(p));
    }
    let rules: Vec<Value> = rules
        .iter()
        .map(|r| match r {
            FilterRule::Genre { genres } => json!({"field": "genre", "in": genres}),
            FilterRule::Decade { decade } => json!({"field": "decade", "is": decade}),
            FilterRule::Artist { artist, role: None } => json!({"field": "artist", "is": artist}),
            FilterRule::Artist {
                artist,
                role: Some(role),
            } => json!({"field": "artist", "is": artist, "role": role.lms()}),
            FilterRule::Compilation { compilation } => {
                json!({"field": "compilation", "is": compilation})
            }
        })
        .collect();
    Ok(json!({
        "name": filter_name(name).unwrap_or_default(),
        "rules": rules,
        "sort": sort_word(sort),
        "limit": limit,
    }))
}

fn rule_from(v: &Value) -> Option<FilterRule> {
    let o = v.as_object()?;
    let id = |x: &Value| x.as_i64().filter(|n| rule_id_ok(*n));
    Some(match o.get("field")?.as_str()? {
        "genre" => FilterRule::Genre {
            genres: o
                .get("in")?
                .as_array()?
                .iter()
                .map(id)
                .collect::<Option<Vec<_>>>()
                .filter(|g| (1..=MAX_FILTER_GENRES).contains(&g.len()))?,
        },
        "decade" => FilterRule::Decade {
            decade: o
                .get("is")?
                .as_u64()
                .filter(|d| (1900..=2090).contains(d) && d % 10 == 0)? as u32,
        },
        "artist" => FilterRule::Artist {
            artist: id(o.get("is")?)?,
            role: match o.get("role") {
                None | Some(Value::Null) => None,
                Some(r) => Some(role_of(r.as_str()?)?),
            },
        },
        "compilation" => FilterRule::Compilation {
            compilation: o.get("is")?.as_bool()?,
        },
        _ => return None,
    })
}

/// One filter as the gateway answers it. A filter with a rule this app doesn't know (a
/// newer gateway's) is left out rather than shown in part.
pub fn filter_from(v: &Value) -> Option<SmartFilter> {
    let rules: Vec<FilterRule> = v
        .get("rules")?
        .as_array()?
        .iter()
        .map(rule_from)
        .collect::<Option<_>>()?;
    if rules.is_empty() || rules.len() > MAX_FILTER_RULES {
        return None;
    }
    Some(SmartFilter {
        id: pos_id(v.get("id")?)?,
        name: name_of(v)?,
        rules,
        sort: sort_of(v.get("sort")?.as_str()?)?,
        limit: v
            .get("limit")?
            .as_u64()
            .filter(|l| (1..=MAX_FILTER_LIMIT as u64).contains(l))? as u32,
    })
}

/// The gateway's filter list, read strictly.
pub fn parse_filters(v: &Value) -> Vec<SmartFilter> {
    v.get("filters")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(filter_from).take(MAX_FILTERS).collect())
        .unwrap_or_default()
}

/// A page of a filter's albums, read strictly.
pub fn parse_filter_albums(v: &Value) -> Result<FilterAlbums, ApiError> {
    let bad = || ApiError::bad_response("unexpected filter answer");
    let albums: Vec<i64> = v
        .get("albums")
        .and_then(Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .filter_map(pos_id)
        .take(MAX_FILTER_PAGE as usize)
        .collect();
    let count = v
        .get("count")
        .and_then(Value::as_u64)
        .filter(|c| *c <= MAX_FILTER_LIMIT as u64)
        .ok_or_else(bad)? as u32;
    Ok(FilterAlbums { albums, count })
}

// ---------------------------------------------------------------------------
// Keeping a station going
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Active {
    station: i64,
    player: String,
    /// More batches can be asked for.
    refill: bool,
    busy: bool,
    /// Tracks the station queued (for track stations): playing anything else means the
    /// user moved on.
    queued: Option<std::collections::HashSet<i64>>,
}

/// What the runner did with a status update.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RunnerUpdate {
    /// Nothing to do.
    Idle,
    /// Added another batch of this many tracks.
    Refilled { added: u32 },
    /// The station can't go on (with why); it has stopped.
    Stopped { reason: String },
    /// Something else is playing now: the station has stopped.
    Ended,
}

/// Keeps one station going on one player (the app holds one).
#[derive(uniffi::Object)]
pub struct StationRunner {
    me: Arc<Me>,
    active: Mutex<Option<Active>>,
}

impl StationRunner {
    async fn queue(&self, player: &str, cmds: Vec<Command>) -> Result<(), ApiError> {
        for c in cmds {
            self.me.core().rpc(player, c).await?;
        }
        Ok(())
    }

    /// Carry out a batch. Returns whether more can follow and how many tracks were added.
    async fn apply(
        &self,
        player: &str,
        batch: StationBatch,
        start: bool,
    ) -> Result<(bool, u32), ApiError> {
        match batch {
            StationBatch::Tracks { ids } => {
                if ids.is_empty() {
                    return Ok((!start, 0));
                }
                self.queue(player, batch_commands(&ids, start)?).await?;
                if let Some(a) = self.active.lock().unwrap().as_mut()
                    && let Some(q) = a.queued.as_mut()
                {
                    q.extend(ids.iter().copied());
                }
                Ok((true, ids.len() as u32))
            }
            StationBatch::RandomPlay { command } => {
                let cmd = command.into_iter().map(Value::String).collect();
                self.me.core().rpc(player, cmd).await?;
                Ok((false, 0))
            }
            StationBatch::Ran { refill } => Ok((refill, 0)),
            StationBatch::NoMore => Ok((false, 0)),
            StationBatch::Broken { reason } => Err(ApiError::Forbidden { detail: reason }),
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl StationRunner {
    #[uniffi::constructor]
    pub fn new(me: Arc<Me>) -> Arc<Self> {
        Arc::new(StationRunner {
            me,
            active: Mutex::new(None),
        })
    }

    /// Start a station on `player`, replacing the queue. Returns the batch it started
    /// with (the app shows a broken station's reason).
    pub async fn start(&self, station: i64, player: String) -> Result<StationBatch, ApiError> {
        let player = commands::player_id(&player)?;
        *self.active.lock().unwrap() = None;
        let batch = self
            .me
            .next_batch(station, player.clone(), true, Vec::new())
            .await?;
        if let StationBatch::Broken { .. } = &batch {
            return Ok(batch);
        }
        let queued = match &batch {
            StationBatch::Tracks { ids } => Some(ids.iter().copied().collect()),
            _ => None,
        };
        let (refill, _) = self.apply(&player, batch.clone(), true).await?;
        *self.active.lock().unwrap() = Some(Active {
            station,
            player,
            refill,
            busy: false,
            queued,
        });
        Ok(batch)
    }

    /// The station playing, if any.
    pub fn current(&self) -> Option<i64> {
        self.active.lock().unwrap().as_ref().map(|a| a.station)
    }

    /// Stop following (the user chose something else, or another player).
    pub fn stop(&self) {
        *self.active.lock().unwrap() = None;
    }

    /// A status update for `player`: `tracks` in its queue, `current` the playing index,
    /// `playing` the current track's id, `queued` the track ids in the queue. Adds a batch
    /// when it runs low (one at a time); stops when something the station didn't queue plays.
    pub async fn on_status(
        &self,
        player: String,
        tracks: u32,
        current: u32,
        playing: Option<i64>,
        queued: Vec<i64>,
    ) -> RunnerUpdate {
        let (station, p) = {
            let mut a = self.active.lock().unwrap();
            let Some(act) = a.as_mut() else {
                return RunnerUpdate::Idle;
            };
            if !act.player.eq_ignore_ascii_case(&player) {
                return RunnerUpdate::Idle;
            }
            if let (Some(mine), Some(now)) = (&act.queued, playing)
                && !mine.contains(&now)
            {
                *a = None;
                return RunnerUpdate::Ended;
            }
            if !act.refill || act.busy || !refill_due(tracks, current) {
                return RunnerUpdate::Idle;
            }
            act.busy = true;
            (act.station, act.player.clone())
        };
        let result = match self.me.next_batch(station, p.clone(), false, queued).await {
            Ok(b) => self.apply(&p, b, false).await,
            Err(e) => Err(e),
        };
        let mut a = self.active.lock().unwrap();
        let Some(act) = a.as_mut().filter(|x| x.station == station) else {
            return RunnerUpdate::Idle;
        };
        act.busy = false;
        match result {
            Ok((more, added)) => {
                act.refill = more;
                if added > 0 {
                    RunnerUpdate::Refilled { added }
                } else {
                    RunnerUpdate::Idle
                }
            }
            Err(ApiError::Forbidden { detail }) => {
                *a = None;
                RunnerUpdate::Stopped { reason: detail }
            }
            // A network hiccup: try again on the next update.
            Err(_) => RunnerUpdate::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::MenuParam;

    fn s(kind: ShelfKind, id: Option<i64>) -> HomeShelf {
        HomeShelf {
            kind,
            id,
            hidden: false,
        }
    }

    #[test]
    fn refill_is_due_below_five_left() {
        assert!(refill_due(0, 0));
        assert!(refill_due(5, 0), "4 left after the current one");
        assert!(!refill_due(6, 0), "5 left");
        assert!(refill_due(20, 15));
        assert!(!refill_due(20, 14));
        assert!(refill_due(3, 10), "index past the end");
        assert!(refill_due(u32::MAX, u32::MAX));
    }

    #[test]
    fn batches_load_then_add_within_the_id_limit() {
        let ids: Vec<i64> = (100_000_000..100_000_050).collect();
        let c = batch_commands(&ids, true).unwrap();
        assert!(c.len() > 1);
        assert_eq!(c[0][1], json!("cmd:load"));
        assert!(c[1..].iter().all(|x| x[1] == json!("cmd:add")));
        for x in &c {
            let list = x[2].as_str().unwrap().strip_prefix("track_id:").unwrap();
            assert!(list.len() <= commands::ID_LIST_MAX);
        }
        let all: Vec<i64> = c
            .iter()
            .flat_map(|x| {
                x[2].as_str().unwrap()[9..]
                    .split(',')
                    .map(|n| n.parse::<i64>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(all, ids, "every id once, in order");
        let refill = batch_commands(&[1, 2], false).unwrap();
        assert_eq!(
            refill,
            vec![
                json!(["playlistcontrol", "cmd:add", "track_id:1,2"])
                    .as_array()
                    .unwrap()
                    .clone()
            ]
        );
        assert!(batch_commands(&[0], true).is_err());
        assert!(batch_commands(&[], true).unwrap().is_empty());
    }

    #[test]
    fn random_mix_answers_must_be_the_allowed_shape() {
        let ok = |w: &[&str]| random_mix_ok(&w.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert!(ok(&["randomplay", "tracks"]));
        assert!(ok(&["randomplay", "albums", "genre_id:3"]));
        for bad in [
            &["randomplay"][..],
            &["randomplay", ".*"],
            &["randomplay", "tracks", "genre_id:0"],
            &["randomplay", "tracks", "genres:Jazz"],
            &["playlist", "play", "x"],
            &["randomplay", "tracks", "genre_id:1", "x"],
        ] {
            assert!(!ok(bad), "{bad:?}");
        }
        assert!(
            parse_batch(&json!({"mode": "randomplay", "command": ["playlist", "clear"]})).is_err()
        );
        assert_eq!(
            parse_batch(
                &json!({"mode": "randomplay", "command": ["randomplay", "tracks", "genre_id:3"]})
            )
            .unwrap(),
            StationBatch::RandomPlay {
                command: vec!["randomplay".into(), "tracks".into(), "genre_id:3".into()]
            }
        );
    }

    #[test]
    fn batches_are_read_strictly() {
        assert_eq!(
            parse_batch(&json!({"mode": "tracks", "tracks": [3, "4", -1, 0, 5.5, 6]})).unwrap(),
            StationBatch::Tracks { ids: vec![3, 6] }
        );
        assert_eq!(
            parse_batch(&json!({"mode": "ran", "refill": true})).unwrap(),
            StationBatch::Ran { refill: true }
        );
        assert_eq!(
            parse_batch(&json!({"mode": "none"})).unwrap(),
            StationBatch::NoMore
        );
        for bad in [
            json!({}),
            json!({"mode": "rm"}),
            json!({"mode": "tracks"}),
            json!(null),
            json!([]),
        ] {
            assert!(parse_batch(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn layouts_read_strictly_and_round_trip() {
        let v = json!({"version": 1, "default": false, "shelves": [
            {"type": "recent"}, {"type": "station", "id": 4, "hidden": true}, {"type": "future_shelf"},
            {"type": "station"}, {"type": "genre", "id": -2}, {"type": "most", "id": 3}
        ]});
        let l = parse_layout(&v).unwrap();
        assert_eq!(
            l.shelves,
            vec![
                s(ShelfKind::Recent, None),
                HomeShelf {
                    hidden: true,
                    ..s(ShelfKind::Station, Some(4))
                }
            ]
        );
        assert_eq!(parse_layout(&layout_json(&l)).unwrap().shelves, l.shelves);
        assert!(parse_layout(&json!({"version": 2, "shelves": []})).is_err());
        assert!(parse_layout(&json!({"version": 1})).is_err());
        assert!(default_home().is_default);
        assert_eq!(layout_problem(default_home()), None);
    }

    #[test]
    fn home_edits_move_hide_add_and_remove() {
        let l = default_home();
        let moved = move_shelf(l.clone(), 0, 4);
        assert_eq!(moved.shelves[4].kind, ShelfKind::Recent);
        assert_eq!(moved.shelves[0].kind, ShelfKind::Stations);
        assert!(!moved.is_default);
        assert_eq!(move_shelf(l.clone(), 0, 9), l, "out of range: unchanged");
        assert_eq!(move_shelf(l.clone(), 2, 2), l);
        let hidden = set_hidden(l.clone(), 1, true);
        assert!(hidden.shelves[1].hidden);
        assert_eq!(set_hidden(l.clone(), 99, true), l);
        let added = add_shelf(l.clone(), s(ShelfKind::Station, Some(7))).unwrap();
        assert_eq!(added.shelves.len(), 6);
        assert!(
            add_shelf(added.clone(), s(ShelfKind::Station, Some(7))).is_err(),
            "twice"
        );
        assert!(
            add_shelf(l.clone(), s(ShelfKind::Recent, None)).is_err(),
            "already there"
        );
        assert!(
            add_shelf(l.clone(), s(ShelfKind::Station, None)).is_err(),
            "needs an id"
        );
        assert!(
            add_shelf(l.clone(), s(ShelfKind::Random, Some(1))).is_err(),
            "takes no id"
        );
        assert!(add_shelf(l.clone(), s(ShelfKind::Genre, Some(0))).is_err());
        let removed = remove_shelf(l.clone(), 0);
        assert_eq!(removed.shelves.len(), 4);
        assert_eq!(remove_shelf(l.clone(), 9), l);
        let mut full = HomeLayout {
            shelves: vec![],
            is_default: false,
        };
        for i in 1..=MAX_SHELVES as i64 {
            full = add_shelf(full, s(ShelfKind::Genre, Some(i))).unwrap();
        }
        assert!(
            add_shelf(full, s(ShelfKind::Genre, Some(99))).is_err(),
            "full"
        );
    }

    #[test]
    fn history_and_lists_are_read_strictly() {
        let key = "a".repeat(64);
        let v = json!({"enabled": false, "retention_days": 396, "next": "1790000000-12", "items": [
            {"track_id": 5, "track_key": key, "album_id": 7, "artist_id": null, "played_at": 1790000000, "seconds": 241.5, "source": "offline"},
            {"track_id": 5, "track_key": "short", "played_at": 1, "seconds": 1},
            {"track_id": -5, "track_key": key, "played_at": 1, "seconds": 1},
            {"track_id": 6, "track_key": key, "played_at": 1, "seconds": "x"}
        ]});
        let h = parse_history(&v).unwrap();
        assert!(!h.enabled);
        assert_eq!(h.items.len(), 1);
        assert!(h.items[0].offline);
        assert_eq!(h.next.as_deref(), Some("1790000000-12"));
        let h = parse_history(&json!({"items": [], "next": "../../x"})).unwrap();
        assert_eq!(h.next, None);
        assert!(parse_history(&json!({})).is_err());
        assert_eq!(
            parse_albums(&json!({"albums": [1, -2, "3", 4]})),
            vec![1, 4]
        );
        let st = parse_stations(&json!({"stations": [
            {"id": 1, "name": "Fresh", "spec": {"kind": "fresh"}, "broken": null},
            {"id": 2, "name": "Mix\u{202e}", "spec": {"kind": "menu"}, "broken": "this menu item is no longer offered"},
            {"id": 3, "name": "", "spec": {"kind": "fresh"}},
            {"id": 4, "name": "X", "spec": {"kind": "unknown"}},
            {"id": "5", "name": "X", "spec": {"kind": "fresh"}}
        ]}));
        assert_eq!(st.len(), 2);
        assert_eq!(
            st[1].broken.as_deref(),
            Some("this menu item is no longer offered")
        );
        assert!(!st[1].name.contains('\u{202e}'));
        assert!(cursor_ok("1-2") && !cursor_ok("1-") && !cursor_ok("a-1") && !cursor_ok("1-2&x=1"));
    }

    fn action(cmd: &[&str], params: &[(&str, &str)], opens_menu: bool) -> MenuAction {
        MenuAction {
            cmd: cmd.iter().map(|x| x.to_string()).collect(),
            params: params
                .iter()
                .map(|(k, v)| MenuParam {
                    key: k.to_string(),
                    value: v.to_string(),
                })
                .collect(),
            next_window: None,
            opens_menu,
            needs_input: params.iter().any(|(_, v)| v.contains("__TAGGEDINPUT__")),
        }
    }

    #[test]
    fn menu_stations_hold_the_steps_and_the_offered_actions() {
        let steps = vec![MenuStep::Opened {
            action: action(
                &["trackinfo", "items"],
                &[("track_id", "9"), ("menu", "1")],
                true,
            ),
            start: 0,
            count: 100,
        }];
        let play = action(&["audiomuse", "similar_track"], &[("track_id", "9")], false);
        let spec = menu_station_spec(&steps, &play, None).unwrap();
        assert_eq!(
            spec,
            json!({"kind": "menu", "steps": [["trackinfo", "items", 0, 100, "track_id:9", "menu:1"]],
                   "play": ["audiomuse", "similar_track", "track_id:9"]})
        );
        let home = menu_station_spec(&[MenuStep::HomeMenu], &play, Some(&play)).unwrap();
        assert_eq!(home["steps"][0], json!(["menu", 0, 500, "direct:1"]));
        assert!(home["add"].is_array());
        // Never: menus as actions, typed text, too deep, nothing.
        assert!(menu_station_spec(&steps, &steps_action(), None).is_err());
        let search = action(&["x", "go"], &[("q", "__TAGGEDINPUT__")], false);
        assert!(menu_station_spec(&steps, &search, None).is_err());
        assert!(menu_station_spec(&[], &play, None).is_err());
        assert!(menu_station_spec(&vec![MenuStep::HomeMenu; 5], &play, None).is_err());
        let not_a_menu = vec![MenuStep::Opened {
            action: play.clone(),
            start: 0,
            count: 10,
        }];
        assert!(menu_station_spec(&not_a_menu, &play, None).is_err());
        let url = action(&["x", "y"], &[("url", "file:///srv/x")], false);
        assert!(menu_station_spec(&steps, &url, None).is_err());
    }

    fn steps_action() -> MenuAction {
        action(&["someplugin", "items"], &[], true)
    }

    #[test]
    fn built_in_specs_are_checked_before_sending() {
        assert!(spec_json(&StationSpec::Genre { genres: vec![] }).is_err());
        assert!(
            spec_json(&StationSpec::Genre {
                genres: vec![1, 2, 3, 4, 5, 6]
            })
            .is_err()
        );
        assert!(spec_json(&StationSpec::Genre { genres: vec![-1] }).is_err());
        assert!(
            spec_json(&StationSpec::Decade {
                decade: 1985,
                mine: false
            })
            .is_err()
        );
        assert!(spec_json(&StationSpec::Menu).is_err());
        assert_eq!(
            spec_json(&StationSpec::Decade {
                decade: 1980,
                mine: true
            })
            .unwrap(),
            json!({"kind": "decade", "decade": 1980, "mine": true})
        );
        assert_eq!(
            spec_json(&StationSpec::DeepCuts).unwrap(),
            json!({"kind": "deep_cuts"})
        );
    }

    /// Robustness: arbitrary JSON never panics the readers.
    #[test]
    fn hostile_answers_never_panic() {
        let mut x: u64 = 0x0123_4567_89ab_cdef;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let atoms = [
            json!(null),
            json!(1),
            json!(-1),
            json!("x"),
            json!([]),
            json!({}),
            json!(1e300),
            json!("a".repeat(64)),
            json!(true),
        ];
        let keys = [
            "items",
            "shelves",
            "stations",
            "menus",
            "mode",
            "tracks",
            "command",
            "version",
            "type",
            "id",
            "kind",
            "spec",
            "name",
            "track_key",
            "track_id",
            "played_at",
            "seconds",
        ];
        for _ in 0..20_000 {
            let mut build = |depth: u32| -> Value {
                let mut o = Map::new();
                for _ in 0..(rnd() % 6) {
                    let k = keys[(rnd() % keys.len() as u64) as usize];
                    let v = if depth > 0 && rnd() % 3 == 0 {
                        Value::Array(
                            (0..rnd() % 4)
                                .map(|_| atoms[(rnd() % atoms.len() as u64) as usize].clone())
                                .collect(),
                        )
                    } else {
                        atoms[(rnd() % atoms.len() as u64) as usize].clone()
                    };
                    o.insert(k.into(), v);
                }
                Value::Object(o)
            };
            let v = build(1);
            let _ = parse_history(&v);
            let _ = parse_layout(&v);
            let _ = parse_stations(&v);
            let _ = parse_menus(&v);
            let _ = parse_batch(&v);
            let _ = parse_albums(&v);
        }
    }

    fn rules() -> Vec<FilterRule> {
        vec![
            FilterRule::Genre { genres: vec![4, 9] },
            FilterRule::Decade { decade: 1990 },
            FilterRule::Artist {
                artist: 12,
                role: Some(ArtistRole::Composer),
            },
            FilterRule::Compilation { compilation: false },
        ]
    }

    #[test]
    fn filters_are_sent_in_the_gateways_shape() {
        let v = filter_json(" Late\u{200b} jazz ", &rules(), AlbumSort::Year, 200).unwrap();
        assert_eq!(
            v,
            json!({"name": "Late jazz", "rules": [
                {"field": "genre", "in": [4, 9]}, {"field": "decade", "is": 1990},
                {"field": "artist", "is": 12, "role": "COMPOSER"},
                {"field": "compilation", "is": false}
            ], "sort": "yearalbum", "limit": 200})
        );
        let plain = filter_json(
            "A",
            &[FilterRule::Artist {
                artist: 3,
                role: None,
            }],
            AlbumSort::Random,
            1,
        )
        .unwrap();
        assert_eq!(plain["rules"], json!([{"field": "artist", "is": 3}]));
        // What is sent reads back the same.
        let mut back = v.clone();
        back["id"] = json!(7);
        let f = filter_from(&back).unwrap();
        assert_eq!(f.rules, rules());
        assert_eq!((f.id, f.sort, f.limit), (7, AlbumSort::Year, 200));
        for (sort, word) in [
            (AlbumSort::Title, "album"),
            (AlbumSort::Artist, "artistalbum"),
            (AlbumSort::Year, "yearalbum"),
            (AlbumSort::Newest, "new"),
            (AlbumSort::Random, "random"),
        ] {
            assert_eq!(sort_word(sort), word);
            assert_eq!(sort_of(word), Some(sort));
        }
        for role in ROLES {
            assert_eq!(role_of(role.lms()), Some(role));
        }
    }

    #[test]
    fn bad_filters_are_refused_before_sending() {
        let ok = |name: &str, r: Vec<FilterRule>, limit| filter_problem(name.into(), r, limit);
        assert_eq!(ok("Mine", rules(), 500), None);
        let g = |genres: Vec<i64>| vec![FilterRule::Genre { genres }];
        let long = "x".repeat(65);
        let cases: Vec<(&str, Vec<FilterRule>, u32)> = vec![
            ("", rules(), 10),
            ("   ", rules(), 10),
            ("\u{202e}\u{200b}", rules(), 10),
            (&long, rules(), 10),
            ("F", vec![], 10),
            ("F", rules(), 0),
            ("F", rules(), 501),
            ("F", g(vec![]), 10),
            ("F", g((1..=21).collect()), 10),
            ("F", g(vec![0]), 10),
            ("F", g(vec![-4]), 10),
            ("F", g(vec![1_000_000_000]), 10),
            ("F", g(vec![3, 3]), 10),
            ("F", vec![FilterRule::Decade { decade: 1985 }], 10),
            ("F", vec![FilterRule::Decade { decade: 1890 }], 10),
            ("F", vec![FilterRule::Decade { decade: 2100 }], 10),
            (
                "F",
                vec![FilterRule::Artist {
                    artist: 0,
                    role: None,
                }],
                10,
            ),
            (
                "F",
                vec![
                    FilterRule::Compilation { compilation: true },
                    FilterRule::Compilation { compilation: false },
                ],
                10,
            ),
            (
                "F",
                (0..9)
                    .map(|i| FilterRule::Decade {
                        decade: 1900 + i * 10,
                    })
                    .collect(),
                10,
            ),
        ];
        for (name, r, limit) in cases {
            assert!(
                ok(name, r.clone(), limit).is_some(),
                "{name:?} {r:?} {limit}"
            );
            assert!(matches!(
                filter_json(name, &r, AlbumSort::Title, limit),
                Err(ApiError::InvalidInput { .. })
            ));
        }
        assert_eq!(ok("F", g((1..=20).collect()), 1), None);
        assert_eq!(ok(&"x".repeat(64), rules(), 1), None);
    }

    #[test]
    fn filter_answers_are_read_strictly() {
        let good = json!({"id": 3, "name": "F", "rules": [{"field": "decade", "is": 1970}],
            "sort": "new", "limit": 20});
        assert!(filter_from(&good).is_some());
        let with = |k: &str, v: Value| {
            let mut g = good.clone();
            g[k] = v;
            g
        };
        for bad in [
            with("id", json!(0)),
            with("id", json!("3")),
            with("name", json!("")),
            with("sort", json!("title")),
            with("limit", json!(0)),
            with("limit", json!(501)),
            with("rules", json!([])),
            with("rules", json!([{"field": "rating", "is": 5}])),
            with("rules", json!([{"field": "decade", "is": 1975}])),
            with("rules", json!([{"field": "genre", "in": []}])),
            with("rules", json!([{"field": "genre", "in": ["4"]}])),
            with(
                "rules",
                json!([{"field": "artist", "is": 4, "role": "PRODUCER"}]),
            ),
            with("rules", json!([{"field": "compilation", "is": 1}])),
            json!(null),
        ] {
            assert_eq!(filter_from(&bad), None, "{bad}");
        }
        // A list leaves out what it can't read.
        let list = json!({"filters": [good.clone(), with("sort", json!("x")), good]});
        assert_eq!(parse_filters(&list).len(), 2);
        assert!(parse_filters(&json!({"filters": 5})).is_empty());
        assert_eq!(
            parse_filter_albums(&json!({"albums": [5, -1, "x", 7], "count": 9})).unwrap(),
            FilterAlbums {
                albums: vec![5, 7],
                count: 9
            }
        );
        assert!(parse_filter_albums(&json!({"albums": [1]})).is_err());
        assert!(parse_filter_albums(&json!({"albums": [1], "count": 501})).is_err());
        assert!(parse_filter_albums(&json!({"count": 1})).is_err());
        let many: Vec<i64> = (1..=150).collect();
        assert_eq!(
            parse_filter_albums(&json!({"albums": many, "count": 150}))
                .unwrap()
                .albums
                .len(),
            100
        );
    }

    #[test]
    fn filter_shelves_name_a_filter() {
        assert!(ShelfKind::Filter.takes_id());
        assert_eq!(ShelfKind::parse("filter"), Some(ShelfKind::Filter));
        let l = parse_layout(
            &json!({"version": 1, "shelves": [{"type": "filter", "id": 4}, {"type": "filter"}]}),
        )
        .unwrap();
        assert_eq!(l.shelves, vec![s(ShelfKind::Filter, Some(4))]);
        assert_eq!(
            layout_json(&l)["shelves"],
            json!([{"type": "filter", "id": 4}])
        );
        assert!(
            layout_problem(HomeLayout {
                shelves: vec![s(ShelfKind::Filter, None)],
                is_default: false
            })
            .is_some()
        );
    }

    /// Robustness: hostile filter answers never panic the readers.
    #[test]
    fn hostile_filter_answers_never_panic() {
        let mut x: u64 = 0xfeed_f00d_1234_5678;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let atoms = [
            json!(null),
            json!(1),
            json!(-1),
            json!(1970),
            json!(500),
            json!(1e300),
            json!("genre"),
            json!("decade"),
            json!("artist"),
            json!("compilation"),
            json!("COMPOSER"),
            json!("album"),
            json!(true),
            json!([]),
            json!([1, 2]),
            json!({}),
        ];
        let keys = [
            "filters", "id", "name", "rules", "sort", "limit", "field", "in", "is", "role",
            "albums", "count",
        ];
        for _ in 0..20_000 {
            let mut obj = |n: u64| -> Value {
                let mut o = Map::new();
                for _ in 0..(rnd() % (n + 1)) {
                    let k = keys[(rnd() % keys.len() as u64) as usize];
                    o.insert(
                        k.into(),
                        atoms[(rnd() % atoms.len() as u64) as usize].clone(),
                    );
                }
                Value::Object(o)
            };
            let rule = obj(3);
            let mut f = obj(5);
            f["rules"] = json!([rule]);
            let other = obj(4);
            let v = json!({"filters": [f.clone(), other], "albums": [1, "2"], "count": 3});
            let _ = parse_filters(&v);
            let _ = filter_from(&f);
            let _ = parse_filter_albums(&f);
            let _ = parse_filter_albums(&v);
        }
    }
}
