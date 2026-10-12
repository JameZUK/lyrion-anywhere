//! The customisable Home (docs/plans/wave1-design.md §7): one small JSON layout per user,
//! kept on the gateway so it is the same on every device.
//!
//! ```json
//! {"version": 1, "shelves": [{"type": "recent"}, {"type": "station", "id": 4, "hidden": true}]}
//! ```
//!
//! - `version` is 1.
//! - `shelves`: at most [`MAX_SHELVES`], in order. Each is `{"type", "id"?, "hidden"?}`
//!   with `type` from a fixed set ([`ShelfType`]). `station` and `menu` name one of the
//!   user's own stations or pinned menus (checked when saved, and ones since deleted are
//!   left out when read); `genre` names an LMS genre id. Nothing else is accepted, and a
//!   shelf may appear only once.
//! - The whole document is at most [`MAX_BYTES`].
//!
//! Without a saved layout, `GET /v1/me/home` answers [`default_layout`] with
//! `"default": true`.

use std::collections::HashSet;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::accounts::{Accounts, AuthError};
use crate::history::{body_json, caller, error, ok};
use crate::stations::{self, PinKind};

/// The layout format this gateway reads and writes.
pub const VERSION: u64 = 1;
/// Largest layout accepted.
pub const MAX_BYTES: usize = 4096;
/// Most shelves on one Home.
pub const MAX_SHELVES: usize = 24;

/// What a shelf shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShelfType {
    /// Continue / Recently played.
    Recent,
    /// The user's stations.
    Stations,
    /// New Music (recently added).
    New,
    /// Most played.
    Most,
    /// On this day.
    OnThisDay,
    /// Random albums.
    Random,
    /// One of the user's stations (`id`).
    Station,
    /// One of the user's pinned menus (`id`).
    Menu,
    /// Albums of one genre (`id`, an LMS genre id).
    Genre,
}

impl ShelfType {
    pub fn parse(s: &str) -> Option<ShelfType> {
        Some(match s {
            "recent" => ShelfType::Recent,
            "stations" => ShelfType::Stations,
            "new" => ShelfType::New,
            "most" => ShelfType::Most,
            "on_this_day" => ShelfType::OnThisDay,
            "random" => ShelfType::Random,
            "station" => ShelfType::Station,
            "menu" => ShelfType::Menu,
            "genre" => ShelfType::Genre,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ShelfType::Recent => "recent",
            ShelfType::Stations => "stations",
            ShelfType::New => "new",
            ShelfType::Most => "most",
            ShelfType::OnThisDay => "on_this_day",
            ShelfType::Random => "random",
            ShelfType::Station => "station",
            ShelfType::Menu => "menu",
            ShelfType::Genre => "genre",
        }
    }

    /// Does this type name something by `id`?
    pub fn takes_id(self) -> bool {
        matches!(
            self,
            ShelfType::Station | ShelfType::Menu | ShelfType::Genre
        )
    }
}

/// One shelf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shelf {
    pub kind: ShelfType,
    pub id: Option<i64>,
    pub hidden: bool,
}

/// A whole layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub shelves: Vec<Shelf>,
}

/// The Home every user starts with.
pub fn default_layout() -> Layout {
    let s = |kind| Shelf {
        kind,
        id: None,
        hidden: false,
    };
    Layout {
        shelves: vec![
            s(ShelfType::Recent),
            s(ShelfType::Stations),
            s(ShelfType::New),
            s(ShelfType::Most),
            s(ShelfType::OnThisDay),
        ],
    }
}

impl Layout {
    pub fn to_json(&self) -> Value {
        let shelves: Vec<Value> = self
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
        json!({"version": VERSION, "shelves": shelves})
    }

    /// Station and pinned-menu shelves the user must own.
    pub fn owned_refs(&self) -> impl Iterator<Item = (PinKind, i64)> + '_ {
        self.shelves.iter().filter_map(|s| match (s.kind, s.id) {
            (ShelfType::Station, Some(id)) => Some((PinKind::Station, id)),
            (ShelfType::Menu, Some(id)) => Some((PinKind::Menu, id)),
            _ => None,
        })
    }
}

/// Why a layout was refused.
pub type Invalid = &'static str;

fn parse_shelf(v: &Value) -> Result<Shelf, Invalid> {
    let o = v.as_object().ok_or("each shelf is an object")?;
    if o.keys()
        .any(|k| !["type", "id", "hidden"].contains(&k.as_str()))
    {
        return Err("unexpected field in a shelf");
    }
    let kind = o
        .get("type")
        .and_then(Value::as_str)
        .and_then(ShelfType::parse)
        .ok_or("unknown shelf type")?;
    let id = match o.get("id") {
        None => None,
        Some(v) => Some(
            v.as_i64()
                .filter(|_| v.is_i64() || v.is_u64())
                .filter(|n| (1..=999_999_999_999).contains(n))
                .ok_or("bad id")?,
        ),
    };
    if kind.takes_id() != id.is_some() {
        return Err(if kind.takes_id() {
            "this shelf needs an id"
        } else {
            "this shelf takes no id"
        });
    }
    let hidden = match o.get("hidden") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("hidden must be true or false"),
    };
    Ok(Shelf { kind, id, hidden })
}

/// Read a layout (as sent or as stored): the size, version and every shelf are checked.
pub fn parse_layout(bytes: &[u8]) -> Result<Layout, Invalid> {
    if bytes.len() > MAX_BYTES {
        return Err("layout too large");
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "not JSON")?;
    let o = v.as_object().ok_or("a layout is an object")?;
    if o.keys()
        .any(|k| !["version", "shelves"].contains(&k.as_str()))
    {
        return Err("unexpected field");
    }
    let version = o.get("version").and_then(Value::as_u64);
    if version != Some(VERSION) || !o.get("version").is_some_and(Value::is_u64) {
        return Err("unsupported version");
    }
    let list = o
        .get("shelves")
        .and_then(Value::as_array)
        .ok_or("shelves missing")?;
    if list.len() > MAX_SHELVES {
        return Err("too many shelves");
    }
    let shelves: Vec<Shelf> = list.iter().map(parse_shelf).collect::<Result<_, _>>()?;
    let mut seen = HashSet::new();
    if !shelves.iter().all(|s| seen.insert((s.kind, s.id))) {
        return Err("a shelf appears twice");
    }
    Ok(Layout { shelves })
}

/// The user's saved layout, if any (one that no longer reads is treated as none).
pub fn load(a: &Accounts, user: i64) -> Result<Option<Layout>, AuthError> {
    let stored: Option<String> = a.with_db(|db| {
        db.query_row(
            "SELECT home FROM user_prefs WHERE user_id = ?1",
            [user],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
    })?;
    Ok(stored.and_then(|s| parse_layout(s.as_bytes()).ok()))
}

pub fn save(a: &Accounts, user: i64, layout: Option<&Layout>) -> Result<(), AuthError> {
    let text = layout.map(|l| l.to_json().to_string());
    a.with_db(|db| {
        db.execute(
            "INSERT INTO user_prefs (user_id, home) VALUES (?1, ?2)
             ON CONFLICT(user_id) DO UPDATE SET home = excluded.home",
            params![user, text],
        )
        .map(|_| ())
    })
}

/// The layout as the user sees it: saved or default, without shelves for stations or
/// pinned menus they no longer have.
pub fn current(a: &Accounts, user: i64) -> Result<(Layout, bool), AuthError> {
    let (mut layout, default) = match load(a, user)? {
        Some(l) => (l, false),
        None => (default_layout(), true),
    };
    let mut keep = Vec::with_capacity(layout.shelves.len());
    for s in layout.shelves {
        let ok = match (s.kind, s.id) {
            (ShelfType::Station, Some(id)) => stations::owns(a, user, PinKind::Station, id)?,
            (ShelfType::Menu, Some(id)) => stations::owns(a, user, PinKind::Menu, id)?,
            _ => true,
        };
        if ok {
            keep.push(s);
        }
    }
    layout.shelves = keep;
    Ok((layout, default))
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn answer(layout: &Layout, default: bool) -> Response {
    let mut v = layout.to_json();
    v["default"] = default.into();
    ok(v)
}

/// `GET /v1/me/home`
pub async fn get(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match current(gw.accounts(), p.user_id) {
        Ok((layout, default)) => answer(&layout, default),
        Err(_) => internal(),
    }
}

/// `PUT /v1/me/home`: replace the layout.
pub async fn put(State(gw): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    // Read as JSON within the cap, then check it byte for byte as it would be stored.
    let v = match body_json(&headers, body, MAX_BYTES).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let layout = match parse_layout(v.to_string().as_bytes()) {
        Ok(l) => l,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    for (kind, id) in layout.owned_refs() {
        match stations::owns(gw.accounts(), p.user_id, kind, id) {
            Ok(true) => {}
            Ok(false) => return error(StatusCode::BAD_REQUEST, "no such station or pinned menu"),
            Err(_) => return internal(),
        }
    }
    match save(gw.accounts(), p.user_id, Some(&layout)) {
        Ok(()) => answer(&layout, false),
        Err(_) => internal(),
    }
}

/// `DELETE /v1/me/home`: back to the default layout.
pub async fn reset(State(gw): State<AppState>, headers: HeaderMap) -> Response {
    let p = match caller(&gw, &headers) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match save(gw.accounts(), p.user_id, None) {
        Ok(()) => answer(&default_layout(), true),
        Err(_) => internal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> Result<Layout, Invalid> {
        parse_layout(v.to_string().as_bytes())
    }

    #[test]
    fn the_default_is_the_agreed_order() {
        let d = default_layout();
        let types: Vec<&str> = d.shelves.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(types, ["recent", "stations", "new", "most", "on_this_day"]);
        assert_eq!(parse(d.to_json()), Ok(d));
    }

    #[test]
    fn layouts_round_trip() {
        let v = json!({"version": 1, "shelves": [
            {"type": "most"}, {"type": "station", "id": 4, "hidden": true},
            {"type": "genre", "id": 12}, {"type": "menu", "id": 9}, {"type": "random", "hidden": false}
        ]});
        let l = parse(v).unwrap();
        assert_eq!(l.shelves.len(), 5);
        assert!(l.shelves[1].hidden);
        assert_eq!(parse(l.to_json()), Ok(l.clone()));
        assert_eq!(
            l.owned_refs().collect::<Vec<_>>(),
            [(PinKind::Station, 4), (PinKind::Menu, 9)]
        );
        assert!(
            parse(json!({"version": 1, "shelves": []})).is_ok(),
            "an empty Home is allowed"
        );
    }

    #[test]
    fn hostile_layouts_are_refused() {
        let shelf = |s: Value| json!({"version": 1, "shelves": [s]});
        let cases = [
            json!(null),
            json!([]),
            json!({}),
            json!({"version": 2, "shelves": []}),
            json!({"version": "1", "shelves": []}),
            json!({"version": 1.0, "shelves": []}),
            json!({"version": 1}),
            json!({"version": 1, "shelves": {}}),
            json!({"version": 1, "shelves": [], "extra": 1}),
            json!({"version": 1, "shelves": [], "__proto__": {}}),
            shelf(json!("recent")),
            shelf(json!({})),
            shelf(json!({"type": "Recent"})),
            shelf(json!({"type": "script"})),
            shelf(json!({"type": "recent", "id": 1})),
            shelf(json!({"type": "station"})),
            shelf(json!({"type": "station", "id": 0})),
            shelf(json!({"type": "station", "id": -1})),
            shelf(json!({"type": "station", "id": "4"})),
            shelf(json!({"type": "station", "id": 4.5})),
            shelf(json!({"type": "station", "id": 1_000_000_000_000u64})),
            shelf(json!({"type": "genre", "id": u64::MAX})),
            shelf(json!({"type": "recent", "hidden": "yes"})),
            shelf(json!({"type": "recent", "hidden": 1})),
            shelf(json!({"type": "recent", "title": "<script>"})),
            shelf(json!({"type": "menu", "id": 1, "cmd": ["pref"]})),
            json!({"version": 1, "shelves": [{"type": "recent"}, {"type": "recent"}]}),
            json!({"version": 1, "shelves": [{"type": "station", "id": 3}, {"type": "station", "id": 3, "hidden": true}]}),
            json!({"version": 1, "shelves": vec![json!({"type": "genre", "id": 1}); 2]}),
        ];
        for c in cases {
            assert!(parse(c.clone()).is_err(), "accepted {c}");
        }
        let many: Vec<Value> = (1..=MAX_SHELVES as i64 + 1)
            .map(|i| json!({"type": "genre", "id": i}))
            .collect();
        assert_eq!(
            parse(json!({"version": 1, "shelves": many})),
            Err("too many shelves")
        );
        let fits: Vec<Value> = (1..=MAX_SHELVES as i64)
            .map(|i| json!({"type": "genre", "id": i}))
            .collect();
        assert!(parse(json!({"version": 1, "shelves": fits})).is_ok());
        let big = format!(r#"{{"version":1,"shelves":[]{}}}"#, " ".repeat(MAX_BYTES));
        assert_eq!(parse_layout(big.as_bytes()), Err("layout too large"));
        assert_eq!(
            parse_layout(b"{\"version\":1,\"shelves\":["),
            Err("not JSON")
        );
    }

    #[test]
    fn layouts_are_stored_per_user_and_drop_deleted_stations() {
        let a = Accounts::in_memory().unwrap();
        a.create_user(
            "alice",
            "correct horse battery",
            crate::accounts::Role::Listener,
        )
        .unwrap();
        a.create_user(
            "bob",
            "correct horse battery",
            crate::accounts::Role::Listener,
        )
        .unwrap();
        let alice = a.user_id_by_name("alice").unwrap().unwrap();
        let bob = a.user_id_by_name("bob").unwrap().unwrap();
        assert_eq!(current(&a, alice).unwrap(), (default_layout(), true));
        let st =
            stations::add(&a, alice, PinKind::Station, "S", &json!({"kind": "fresh"})).unwrap();
        let l = parse(
            json!({"version": 1, "shelves": [{"type": "station", "id": st}, {"type": "new"}]}),
        )
        .unwrap();
        save(&a, alice, Some(&l)).unwrap();
        assert_eq!(current(&a, alice).unwrap(), (l.clone(), false));
        assert!(current(&a, bob).unwrap().1, "bob still has the default");
        stations::remove(&a, alice, PinKind::Station, st).unwrap();
        let (left, _) = current(&a, alice).unwrap();
        assert_eq!(left.shelves.len(), 1);
        assert_eq!(left.shelves[0].kind, ShelfType::New);
        save(&a, alice, None).unwrap();
        assert!(current(&a, alice).unwrap().1);
    }

    /// Robustness: arbitrary bytes and arbitrary JSON never panic the parser, and whatever
    /// it accepts is within the limits and reads back the same.
    #[test]
    fn fuzz_the_layout_parser() {
        let mut rng = stations::Rng::seeded(0xbeef);
        let atoms = [
            "0",
            "1",
            "-1",
            "2",
            "1.5",
            "null",
            "true",
            "false",
            "\"recent\"",
            "\"station\"",
            "\"genre\"",
            "\"menu\"",
            "\"type\"",
            "\"id\"",
            "\"hidden\"",
            "\"version\"",
            "\"shelves\"",
            "{",
            "}",
            "[",
            "]",
            ",",
            ":",
            "999999999999",
            "1000000000000",
            "\"\\u0000\"",
        ];
        for _ in 0..20_000 {
            let n = rng.below(40) as usize;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(atoms[rng.below(atoms.len() as u64) as usize]);
            }
            check(s.as_bytes());
            let raw: Vec<u8> = (0..rng.below(64)).map(|_| rng.next_u64() as u8).collect();
            check(&raw);
        }
        let types = [
            "recent",
            "stations",
            "new",
            "most",
            "on_this_day",
            "random",
            "station",
            "menu",
            "genre",
            "x",
        ];
        for _ in 0..20_000 {
            let shelves: Vec<Value> = (0..rng.below(30))
                .map(|_| {
                    let mut o = Map::new();
                    if rng.below(10) > 0 {
                        o.insert(
                            "type".into(),
                            types[rng.below(types.len() as u64) as usize].into(),
                        );
                    }
                    match rng.below(4) {
                        0 => {}
                        1 => {
                            o.insert("id".into(), (rng.below(50) as i64 - 5).into());
                        }
                        2 => {
                            o.insert("id".into(), "7".into());
                        }
                        _ => {
                            o.insert("hidden".into(), (rng.below(2) == 0).into());
                        }
                    }
                    Value::Object(o)
                })
                .collect();
            let v = json!({"version": rng.below(3), "shelves": shelves});
            check(v.to_string().as_bytes());
        }
    }

    fn check(bytes: &[u8]) {
        if let Ok(l) = parse_layout(bytes) {
            assert!(l.shelves.len() <= MAX_SHELVES);
            for s in &l.shelves {
                assert_eq!(s.kind.takes_id(), s.id.is_some());
                assert!(s.id.is_none_or(|id| id > 0));
            }
            assert_eq!(parse_layout(l.to_json().to_string().as_bytes()), Ok(l));
        }
    }
}
