//! Plugin and context menus: actions LMS offered, and nothing else.
//!
//! LMS menus (apps, radio, track/album/artist info, plugin menus) use the "SlimBrowse"
//! format: each item carries actions (`go`, `play`, `add`, `more`, …), each an LMS command
//! with parameters, often split between the menu's `base` actions and the item's own fields.
//! Plugins add their own commands, so no fixed allow-list can cover them.
//!
//! The rule: a device may run a menu action only if LMS offered exactly that action, in a
//! menu the gateway relayed to that device recently. Requests must match a learned action
//! token for token and parameter for parameter, except text the user types into a search
//! box (`__TAGGEDINPUT__`). On top of that, server-level commands are never allowed, and
//! actions that change shared data are for admins only.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

/// How long a learned action stays usable.
pub const TTL: Duration = Duration::from_secs(60 * 60);
/// Learned actions kept per device (oldest dropped first).
pub const PER_DEVICE: usize = 4000;
/// Paging limit for menu requests (as elsewhere).
const MAX_COUNT: u64 = 500;

/// A parameter value in a learned action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PValue {
    Exact(String),
    /// Whatever the user types (search boxes).
    Input,
}

/// An action LMS offered: command words and parameters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action {
    pub cmd: Vec<String>,
    pub params: BTreeMap<String, PValue>,
    /// Offered as `go` with no `nextWindow`: it opens a menu, which clients page
    /// (`start count`) whatever the command is called.
    pub nav: bool,
}

impl Action {
    /// Menus (`… items`) take paging (`start count`) after the command words.
    fn paged(&self) -> bool {
        self.cmd.last().is_some_and(|w| w == "items")
    }
}

/// Is action `name` of this item a navigation (opens a menu) rather than a command?
fn is_nav(name: &str, item: &Map<String, Value>, action: &Map<String, Value>) -> bool {
    name == "go" && !item.contains_key("nextWindow") && !action.contains_key("nextWindow")
}

/// Commands never runnable through menus, whatever LMS offers: server administration,
/// device control outside playback, and anything this gateway handles itself.
const NEVER: &[&str] = &[
    "pref",
    "playerpref",
    "rescan",
    "wipecache",
    "restartserver",
    "stopserver",
    "debug",
    "logging",
    "client",
    "button",
    "ir",
    "irenable",
    "display",
    "displaynow",
    "show",
    "alarm",
    "alarms",
    "readdirectory",
    "setsncredentials",
    "anywhere",
    "connect",
    "disconnect",
    "firmwareupgrade",
    "getstring",
    "pragma",
    "abortscan",
    "musicfolder_refresh",
    "power",
    "mixer",
    "sync",
    "time",
    "stop",
    "pause",
    "play",
    "sleep",
    "playlist",
];

/// Commands that change data shared by the household: admins only.
const ADMIN_ONLY: &[&str] = &[
    "jivefavorites",
    "favorites",
    "playlists",
    "jiveplaylists",
    "presets",
    // Random Mix's genre and library choices are server-wide prefs, not the player's.
    "randomplaychoosegenre",
    "randomplaygenreselectall",
    "randomplaychooselibrary",
];

fn word_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

fn key_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

/// Text from the user or from LMS menus: bounded, and passing the same check as the fixed
/// allow-list's free text (printable; no path, local URL, LAN address or non-http(s) URL).
fn value_ok(s: &str) -> bool {
    s.len() <= 256 && crate::rpc_policy::text_ok(s)
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some((if *b { "1" } else { "0" }).into()),
        _ => None,
    }
}

fn action_from(cmd: &Value, params: &[&Map<String, Value>], nav: bool) -> Option<Action> {
    let words: Vec<String> = cmd
        .as_array()?
        .iter()
        .map(|w| w.as_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    if words.is_empty() || words.len() > 6 || !words.iter().all(|w| word_ok(w)) {
        return None;
    }
    let mut out = BTreeMap::new();
    for map in params {
        for (k, v) in map.iter() {
            if !key_ok(k) || k == "url" {
                return None; // never learn actions carrying URLs or odd keys
            }
            let Some(s) = scalar(v) else { continue };
            let pv = if s.contains("__TAGGEDINPUT__") || s.contains("__INPUT__") {
                PValue::Input
            } else if value_ok(&s) {
                PValue::Exact(s)
            } else {
                return None;
            };
            out.insert(k.clone(), pv);
        }
    }
    if out.len() > 16 {
        return None;
    }
    Some(Action {
        cmd: words,
        params: out,
        nav,
    })
}

/// Every action a SlimBrowse response offers (items' own actions, and the menu's base
/// actions combined with each item's parameters).
pub fn learn(v: &Value) -> Vec<Action> {
    let mut out = Vec::new();
    let base = v
        .get("base")
        .and_then(|b| b.get("actions"))
        .and_then(Value::as_object);
    let items = v
        .get("item_loop")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    for item in items.iter().take(2000) {
        let Some(item) = item.as_object() else {
            continue;
        };
        let own = item.get("actions").and_then(Value::as_object);
        if let Some(own) = own {
            for (name, a) in own {
                let Some(a) = a.as_object() else { continue };
                let empty = Map::new();
                let params = a.get("params").and_then(Value::as_object).unwrap_or(&empty);
                if let Some(cmd) = a.get("cmd")
                    && let Some(x) = action_from(cmd, &[params], is_nav(name, item, a))
                {
                    out.push(x);
                }
            }
        }
        if let Some(base) = base {
            for (name, a) in base {
                if own.is_some_and(|o| o.contains_key(name)) {
                    continue; // the item overrides this action
                }
                let Some(a) = a.as_object() else { continue };
                // The item's parameters for base actions live in the field `itemsParams` names.
                let field = a
                    .get("itemsParams")
                    .and_then(Value::as_str)
                    .unwrap_or("params");
                let Some(item_params) = item.get(field).and_then(Value::as_object) else {
                    continue;
                };
                let empty = Map::new();
                let base_params = a.get("params").and_then(Value::as_object).unwrap_or(&empty);
                if let Some(cmd) = a.get("cmd")
                    && let Some(x) =
                        action_from(cmd, &[base_params, item_params], is_nav(name, item, a))
                {
                    out.push(x);
                }
            }
        }
    }
    out
}

/// Why a menu request was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Malformed,
    Never,
    AdminOnly,
    NotOffered,
}

/// A request split into command words, paging and parameters.
#[derive(Debug)]
pub struct Request {
    pub cmd: Vec<String>,
    pub paging: Option<(u64, u64)>,
    pub params: BTreeMap<String, String>,
}

pub fn parse(raw: &[String]) -> Option<Request> {
    let mut i = 0;
    let mut cmd = Vec::new();
    while i < raw.len() && !raw[i].contains(':') && !raw[i].bytes().all(|b| b.is_ascii_digit()) {
        if !word_ok(&raw[i]) {
            return None;
        }
        cmd.push(raw[i].clone());
        i += 1;
    }
    if cmd.is_empty() || cmd.len() > 6 {
        return None;
    }
    let mut paging = None;
    if i + 1 < raw.len()
        && raw[i].bytes().all(|b| b.is_ascii_digit())
        && raw[i + 1].bytes().all(|b| b.is_ascii_digit())
    {
        let start = raw[i].parse::<u64>().ok()?;
        let count = raw[i + 1].parse::<u64>().ok()?;
        if count == 0 || count > MAX_COUNT || start > 1_000_000 {
            return None;
        }
        paging = Some((start, count));
        i += 2;
    }
    let mut params = BTreeMap::new();
    for p in &raw[i..] {
        let (k, v) = p.split_once(':')?;
        if !key_ok(k)
            || k == "url"
            || !value_ok(v)
            || params.insert(k.to_owned(), v.to_owned()).is_some()
        {
            return None;
        }
    }
    Some(Request {
        cmd,
        paging,
        params,
    })
}

impl Action {
    /// Does the request match this action? `strict`: typed text must also pass
    /// [`crate::rpc_policy::typed_text_ok`] (no URL with an unusual port).
    fn matches(&self, r: &Request, strict: bool) -> bool {
        // Menus must be paged; navigations may be; other commands never are.
        let paging_ok = if self.paged() {
            r.paging.is_some()
        } else {
            self.nav || r.paging.is_none()
        };
        if self.cmd != r.cmd || !paging_ok || self.params.len() != r.params.len() {
            return false;
        }
        self.params.iter().all(|(k, v)| match (v, r.params.get(k)) {
            (PValue::Exact(want), Some(got)) => want == got,
            (PValue::Input, Some(got)) => {
                !got.is_empty() && (!strict || crate::rpc_policy::typed_text_ok(got))
            }
            _ => false,
        })
    }
}

/// Check a request against the actions offered to this device. `admin` is the caller's role.
pub fn allowed(learned: &DeviceActions, raw: &[String], admin: bool) -> Result<(), Refusal> {
    let r = parse(raw).ok_or(Refusal::Malformed)?;
    if NEVER.contains(&r.cmd[0].as_str()) {
        return Err(Refusal::Never);
    }
    if ADMIN_ONLY.contains(&r.cmd[0].as_str()) && !admin {
        return Err(Refusal::AdminOnly);
    }
    if learned.find(&r, true) {
        Ok(())
    } else if learned.find(&r, false) {
        // Offered, but what the user typed isn't allowed: refused like other bad text.
        Err(Refusal::Malformed)
    } else {
        Err(Refusal::NotOffered)
    }
}

/// Actions learned for one device, bounded and expiring.
///
/// Every operation costs time in proportion to the actions it is given, never to the actions
/// held (review low: adding a menu compared each item with every action held for its command,
/// up to 2,000 items against 4,000 actions, under the gateway-wide lock):
/// - an action is found by hashing it whole, so adding, refreshing and matching a request
///   with no typed input are lookups;
/// - `order` lists actions oldest first, by when they were last offered; a refresh appends a
///   new record and leaves the old one stale (its number no longer matches), and stale
///   records are skipped as they reach the front, or swept out once they outnumber the rest;
/// - actions with typed input (search boxes, rare) are also kept by command, since a request
///   can only be matched to them one by one.
#[derive(Debug, Default)]
pub struct DeviceActions {
    /// Every action held: the number of its latest record in `order`, and when it was last
    /// offered.
    held: HashMap<Arc<Action>, (u64, Instant)>,
    /// (record number, when, action), oldest first.
    order: VecDeque<(u64, Instant, Arc<Action>)>,
    next: u64,
    /// The actions with typed input, by command.
    with_input: HashMap<Vec<String>, HashSet<Arc<Action>>>,
    /// When actions were last added or looked up (for evicting idle devices).
    last_used: Option<Instant>,
}

impl DeviceActions {
    pub fn touch(&mut self, now: Instant) {
        self.last_used = Some(now);
    }

    pub fn last_used(&self) -> Option<Instant> {
        self.last_used
    }

    pub fn add(&mut self, actions: Vec<Action>, now: Instant) {
        self.touch(now);
        for a in actions {
            self.next += 1;
            let seq = self.next;
            match self.held.get_key_value(&a) {
                // Offered again: refreshed, and now the newest.
                Some((key, _)) => {
                    let key = key.clone();
                    self.held.insert(key.clone(), (seq, now));
                    self.order.push_back((seq, now, key));
                }
                None => {
                    let a = Arc::new(a);
                    if a.params.values().any(|v| *v == PValue::Input) {
                        self.with_input
                            .entry(a.cmd.clone())
                            .or_default()
                            .insert(a.clone());
                    }
                    self.held.insert(a.clone(), (seq, now));
                    self.order.push_back((seq, now, a));
                }
            }
        }
        while self.held.len() > PER_DEVICE {
            self.evict_oldest();
        }
        // Stale records (left by refreshes) never outnumber the live ones for long.
        if self.order.len() > 2 * self.held.len() + 64 {
            let held = &self.held;
            self.order
                .retain(|(seq, _, a)| held.get(a).is_some_and(|(s, _)| s == seq));
        }
    }

    /// Whether `order`'s record is the latest one of its action.
    fn live(&self, seq: u64, a: &Action) -> bool {
        self.held.get(a).is_some_and(|(s, _)| *s == seq)
    }

    fn forget(&mut self, a: &Arc<Action>) {
        self.held.remove(a);
        if let Some(set) = self.with_input.get_mut(&a.cmd) {
            set.remove(a);
            if set.is_empty() {
                self.with_input.remove(&a.cmd);
            }
        }
    }

    /// Drop the action offered longest ago.
    fn evict_oldest(&mut self) {
        while let Some((seq, _, a)) = self.order.pop_front() {
            if self.live(seq, &a) {
                self.forget(&a);
                return;
            }
        }
    }

    /// Drop the actions not offered again within [`TTL`].
    pub fn expire(&mut self, now: Instant) {
        while let Some((seq, at, a)) = self.order.front() {
            let (seq, at) = (*seq, *at);
            if self.live(seq, a) && now.saturating_duration_since(at) < TTL {
                break;
            }
            let (_, _, a) = self.order.pop_front().expect("just looked");
            if self.live(seq, &a) {
                self.forget(&a);
            }
        }
    }

    fn find(&self, r: &Request, strict: bool) -> bool {
        // An action without typed input matches only a request with exactly its parameters.
        let exact: BTreeMap<String, PValue> = r
            .params
            .iter()
            .map(|(k, v)| (k.clone(), PValue::Exact(v.clone())))
            .collect();
        let mut probe = Action {
            cmd: r.cmd.clone(),
            params: exact,
            nav: false,
        };
        for nav in [false, true] {
            probe.nav = nav;
            if self.held.contains_key(&probe) && probe.matches(r, strict) {
                return true;
            }
        }
        self.with_input
            .get(&r.cmd)
            .is_some_and(|set| set.iter().any(|a| a.matches(r, strict)))
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Every action held (to hand them to a device: see `crate::stations`).
    pub fn into_actions(mut self) -> Vec<Action> {
        // The other references go first, so each action is moved out rather than copied.
        self.order.clear();
        self.with_input.clear();
        std::mem::take(&mut self.held)
            .into_keys()
            .map(|a| Arc::try_unwrap(a).unwrap_or_else(|a| (*a).clone()))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Shapes taken from real LMS menus (radio, podcasts, track info), anonymised.
    fn radio_menu() -> Value {
        json!({
            "base": {"actions": {
                "play": {"cmd": ["radiofeeds", "playlist", "play"], "params": {"menu": "radiofeeds"}, "itemsParams": "params", "player": 0},
                "add": {"cmd": ["radiofeeds", "playlist", "add"], "params": {"menu": "radiofeeds"}, "itemsParams": "params", "player": 0},
                "go": {"cmd": ["radiofeeds", "items"], "params": {"menu": "radiofeeds"}, "itemsParams": "params"}
            }},
            "item_loop": [
                {"text": "By name", "type": "link", "actions": {"go": {"cmd": ["radiofeeds", "items"], "params": {"item_id": "63f0.0", "menu": "radiofeeds"}}}},
                {"text": "Station", "type": "audio", "params": {"item_id": "63f0.1.4"}},
                {"text": "Search", "type": "search", "actions": {"go": {"cmd": ["podcasts", "items"], "params": {"search": "__TAGGEDINPUT__", "item_id": "1", "menu": "podcasts"}}}},
                {"text": "Save", "actions": {"go": {"cmd": ["jivefavorites", "add"], "params": {"title": "Some Track", "isContextMenu": 1}}}},
                {"text": "Bad", "actions": {"go": {"cmd": ["jivefavorites", "add"], "params": {"url": "file:///srv/x.flac"}}}},
                {"text": "LAN", "actions": {"go": {"cmd": ["x", "items"], "params": {"item_id": "http://10.0.0.10/x"}}}},
                {"text": "Similar", "type": "redirect", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["audiomuse", "similar_track"], "params": {"track_id": "4242"}, "player": 0}}}
            ]
        })
    }

    fn learned() -> DeviceActions {
        let mut d = DeviceActions::default();
        d.add(learn(&radio_menu()), Instant::now());
        d
    }

    #[test]
    fn learns_item_actions_and_base_actions_with_item_params() {
        let a = learn(&radio_menu());
        let has = |cmd: &[&str], params: &[(&str, &str)]| {
            a.iter().any(|x| {
                x.cmd == strs(cmd)
                    && x.params
                        == params
                            .iter()
                            .map(|(k, v)| (k.to_string(), PValue::Exact(v.to_string())))
                            .collect::<BTreeMap<_, _>>()
            })
        };
        assert!(has(
            &["radiofeeds", "items"],
            &[("item_id", "63f0.0"), ("menu", "radiofeeds")]
        ));
        // Base "play" + the item's own params.
        assert!(has(
            &["radiofeeds", "playlist", "play"],
            &[("item_id", "63f0.1.4"), ("menu", "radiofeeds")]
        ));
        assert!(has(
            &["audiomuse", "similar_track"],
            &[("track_id", "4242")]
        ));
        // Never learned: URL params, LAN URLs.
        assert!(!a.iter().any(|x| x.params.contains_key("url")));
        assert!(!a.iter().any(|x| {
            x.params
                .values()
                .any(|v| matches!(v, PValue::Exact(s) if s.contains("10.0.0.10")))
        }));
    }

    #[test]
    fn only_offered_actions_run() {
        let d = learned();
        let ok = |raw: &[&str], admin| allowed(&d, &strs(raw), admin);
        assert_eq!(
            ok(
                &[
                    "radiofeeds",
                    "items",
                    "0",
                    "50",
                    "item_id:63f0.0",
                    "menu:radiofeeds"
                ],
                false
            ),
            Ok(())
        );
        assert_eq!(
            ok(
                &[
                    "radiofeeds",
                    "playlist",
                    "play",
                    "item_id:63f0.1.4",
                    "menu:radiofeeds"
                ],
                false
            ),
            Ok(())
        );
        assert_eq!(
            ok(&["audiomuse", "similar_track", "track_id:4242"], false),
            Ok(())
        );
        // A different item, extra or missing params, wrong paging: refused.
        for bad in [
            &[
                "radiofeeds",
                "items",
                "0",
                "50",
                "item_id:63f0.9",
                "menu:radiofeeds",
            ][..],
            &["radiofeeds", "items", "0", "50", "item_id:63f0.0"],
            &[
                "radiofeeds",
                "items",
                "0",
                "50",
                "item_id:63f0.0",
                "menu:radiofeeds",
                "x:1",
            ],
            &["radiofeeds", "items", "item_id:63f0.0", "menu:radiofeeds"],
            &["audiomuse", "similar_track", "track_id:1"],
            &["audiomuse", "similar_track", "0", "10", "track_id:4242"],
            &["unknown", "items", "0", "10"],
        ] {
            assert_eq!(ok(bad, false), Err(Refusal::NotOffered), "{bad:?}");
        }
        // Over-large pages are refused before matching.
        let big = [
            "radiofeeds",
            "items",
            "0",
            "5000",
            "item_id:63f0.0",
            "menu:radiofeeds",
        ];
        assert_eq!(ok(&big, false), Err(Refusal::Malformed));
    }

    #[test]
    fn search_boxes_accept_typed_text_but_not_urls() {
        let d = learned();
        let run = |q: &str| {
            allowed(
                &d,
                &strs(&[
                    "podcasts",
                    "items",
                    "0",
                    "20",
                    &format!("search:{q}"),
                    "item_id:1",
                    "menu:podcasts",
                ]),
                false,
            )
        };
        assert_eq!(run("in our time"), Ok(()));
        assert_eq!(run(""), Err(Refusal::NotOffered));
        assert_eq!(run("file:///etc/passwd"), Err(Refusal::Malformed));
        assert_eq!(run("http://192.168.250.1/admin"), Err(Refusal::Malformed));
        assert_eq!(run(&"x".repeat(300)), Err(Refusal::Malformed));
        assert_eq!(run("a\nb"), Err(Refusal::Malformed));
        // Regression: single-label hosts, bare LAN addresses and other schemes got through.
        for bad in [
            "http://nas:9000/x",
            "10.0.0.5:9000",
            "smb://10.0.0.5/music",
            "http://2130706433/",
            "/etc/passwd",
        ] {
            assert_eq!(run(bad), Err(Refusal::Malformed), "{bad}");
        }
        for good in [
            "AC/DC",
            "Live 1990/91",
            "http",
            "https://feeds.example.com/a.xml",
        ] {
            assert_eq!(run(good), Ok(()), "{good}");
        }
    }

    #[test]
    fn server_commands_are_never_runnable_even_if_offered() {
        let mut d = DeviceActions::default();
        let menu = json!({"item_loop": [
            {"actions": {"go": {"cmd": ["pref", "audiodir"], "params": {"value": "x"}}}},
            {"actions": {"go": {"cmd": ["client", "forget"], "params": {}}}},
            {"actions": {"go": {"cmd": ["playlist", "play"], "params": {"item": "x"}}}},
            {"actions": {"go": {"cmd": ["anywhere", "users"], "params": {}}}}
        ]});
        d.add(learn(&menu), Instant::now());
        assert_eq!(d.len(), 4);
        for raw in [
            &["pref", "audiodir", "value:x"][..],
            &["client", "forget"],
            &["playlist", "play", "item:x"],
            &["anywhere", "users"],
        ] {
            assert_eq!(
                allowed(&d, &strs(raw), true),
                Err(Refusal::Never),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn shared_data_changes_are_for_admins() {
        let d = learned();
        let raw = strs(&[
            "jivefavorites",
            "add",
            "isContextMenu:1",
            "title:Some Track",
        ]);
        assert_eq!(allowed(&d, &raw, false), Err(Refusal::AdminOnly));
        assert_eq!(allowed(&d, &raw, true), Ok(()));
    }

    /// Random Mix's genre and library choices are server-wide prefs (LMS's
    /// `exclude_genres` and `library`): even when a menu offers them, listeners can't.
    #[test]
    fn random_mix_server_wide_choices_are_for_admins() {
        let mut d = DeviceActions::default();
        let menu = json!({"item_loop": [
            {"actions": {"on": {"cmd": ["randomplaychoosegenre", "Jazz", "1"], "player": 0}}},
            {"actions": {"do": {"cmd": ["randomplaygenreselectall", "0"], "player": 0}}},
            {"actions": {"do": {"cmd": ["randomplaychooselibrary"], "params": {"library": "1a2b"}, "player": 0}}},
            {"actions": {"do": {"cmd": ["randomplaychoosegenre"], "params": {"genre": "Jazz", "value": "0"}}}}
        ]});
        d.add(learn(&menu), Instant::now());
        for raw in [
            &["randomplaychoosegenre", "Jazz", "1"][..],
            &["randomplaygenreselectall", "0"],
            &["randomplaychooselibrary", "library:1a2b"],
            &["randomplaychoosegenre", "genre:Jazz", "value:0"],
        ] {
            assert!(allowed(&d, &strs(raw), false).is_err(), "{raw:?}");
        }
        assert_eq!(
            allowed(
                &d,
                &strs(&["randomplaychooselibrary", "library:1a2b"]),
                false
            ),
            Err(Refusal::AdminOnly)
        );
        assert_eq!(
            allowed(
                &d,
                &strs(&["randomplaychooselibrary", "library:1a2b"]),
                true
            ),
            Ok(())
        );
    }

    #[test]
    fn learned_actions_can_be_handed_over_whole() {
        let mut d = DeviceActions::default();
        d.add(learn(&radio_menu()), Instant::now());
        let n = d.len();
        let all = d.into_actions();
        assert_eq!(all.len(), n);
        let mut again = DeviceActions::default();
        again.add(all, Instant::now());
        assert_eq!(again.len(), n);
    }

    #[test]
    fn malformed_requests_are_refused() {
        let d = learned();
        for bad in [
            &[][..],
            &["radiofeeds;rm", "items"],
            &[
                "radiofeeds",
                "items",
                "0",
                "50",
                "item_id:63f0.0",
                "item_id:63f0.0",
                "menu:radiofeeds",
            ],
            &["radiofeeds", "items", "0", "50", "url:x"],
            &["radiofeeds", "items", "0", "50", "bad key:1"],
            &["a", "b", "c", "d", "e", "f", "g"],
        ] {
            assert_eq!(
                allowed(&d, &strs(bad), true),
                Err(Refusal::Malformed),
                "{bad:?}"
            );
        }
    }

    /// A plugin menu in the Jive style (as AudioMuse-AI's): `go` opens a submenu unless the
    /// item has `nextWindow`, in which case it is a command that runs.
    fn plugin_menu() -> Value {
        json!({
            "item_loop": [
                {"text": "Browse by mood", "actions": {"go": {"cmd": ["someplugin", "menu_mood"], "player": 0}}},
                {"text": "Similar to current track", "nextWindow": "refresh",
                 "actions": {"go": {"cmd": ["someplugin", "similar_now"], "player": 0}}},
                {"text": "Pick an artist", "actions": {"go": {"cmd": ["someplugin", "browse"], "params": {"start": 0, "target": "x"}}}},
                {"text": "Describe it", "nextWindow": "refresh",
                 "actions": {"go": {"cmd": ["someplugin", "instant"], "params": {"prompt": "__TAGGEDINPUT__"}}}}
            ]
        })
    }

    #[test]
    fn navigations_may_be_paged_but_commands_may_not() {
        let mut d = DeviceActions::default();
        d.add(learn(&plugin_menu()), Instant::now());
        let ok = |raw: &[&str]| allowed(&d, &strs(raw), false);
        // A submenu opens with or without paging.
        assert_eq!(ok(&["someplugin", "menu_mood", "0", "100"]), Ok(()));
        assert_eq!(ok(&["someplugin", "menu_mood"]), Ok(()));
        assert_eq!(
            ok(&["someplugin", "browse", "0", "50", "start:0", "target:x"]),
            Ok(())
        );
        // A command (nextWindow) runs exactly as offered: no paging slipped in.
        assert_eq!(ok(&["someplugin", "similar_now"]), Ok(()));
        assert_eq!(
            ok(&["someplugin", "similar_now", "0", "100"]),
            Err(Refusal::NotOffered)
        );
        assert_eq!(ok(&["someplugin", "instant", "prompt:quiet piano"]), Ok(()));
        assert_eq!(
            ok(&["someplugin", "instant", "0", "1", "prompt:x"]),
            Err(Refusal::NotOffered)
        );
        // Navigation doesn't loosen parameters or command words.
        assert_eq!(
            ok(&["someplugin", "browse", "0", "50", "start:0", "target:y"]),
            Err(Refusal::NotOffered)
        );
        assert_eq!(
            ok(&["someplugin", "browse", "0", "50", "start:0"]),
            Err(Refusal::NotOffered)
        );
        assert_eq!(
            ok(&["someplugin", "menu_mood", "0", "100", "extra:1"]),
            Err(Refusal::NotOffered)
        );
        assert_eq!(
            ok(&["someplugin", "menu_mood", "0", "9999"]),
            Err(Refusal::Malformed)
        );
        assert_eq!(ok(&["someplugin", "menu_other"]), Err(Refusal::NotOffered));
    }

    #[test]
    fn learned_actions_expire_and_are_bounded() {
        let t0 = Instant::now();
        let mut d = DeviceActions::default();
        d.add(learn(&radio_menu()), t0);
        assert!(!d.is_empty());
        d.expire(t0 + TTL + Duration::from_secs(1));
        assert!(d.is_empty());
        let many: Vec<Action> = (0..PER_DEVICE + 50)
            .map(|i| Action {
                cmd: strs(&["x", "items"]),
                params: [("item_id".to_string(), PValue::Exact(i.to_string()))].into(),
                nav: true,
            })
            .collect();
        d.add(many, t0);
        assert_eq!(d.len(), PER_DEVICE);
        // The oldest went first.
        let req =
            |i: usize| parse(&strs(&["x", "items", "0", "1", &format!("item_id:{i}")])).unwrap();
        assert!(!d.find(&req(0), true));
        assert!(d.find(&req(PER_DEVICE + 49), true));
    }

    fn item(cmd: &[&str], i: usize) -> Action {
        Action {
            cmd: strs(cmd),
            params: [("item_id".to_string(), PValue::Exact(format!("{i}.0")))].into(),
            nav: true,
        }
    }

    /// Regression (review low): adding a menu compared every item with every action held for
    /// its command, and each eviction scanned them again, all under the gateway-wide lock:
    /// a device holding 4,000 actions of one command and shown 2,000 more items of it cost
    /// millions of comparisons a page. Now adding, refreshing, evicting and matching cost
    /// the same however many are held.
    #[test]
    fn adding_and_matching_cost_the_same_however_many_actions_are_held() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let mut d = DeviceActions::default();
            d.add(
                (0..PER_DEVICE).map(|i| item(&["x", "items"], i)).collect(),
                t0,
            );
            // Page after page of new items of the same command, each evicting the oldest.
            // Linear, this takes well under a second in a debug build on an idle machine;
            // as it was, each page cost 16 million comparisons.
            for page in 0..20 {
                let base = PER_DEVICE + page * 2000;
                d.add(
                    (base..base + 2000)
                        .map(|i| item(&["x", "items"], i))
                        .collect(),
                    t0,
                );
            }
            // The same page offered again and again only refreshes it.
            let again: Vec<Action> = (0..2000)
                .map(|i| item(&["x", "items"], PER_DEVICE + 19 * 2000 + i))
                .collect();
            for _ in 0..30 {
                d.add(again.clone(), t0);
            }
            let mut found = 0;
            for i in 0..20_000 {
                let r = parse(&strs(&["x", "items", "0", "10", &format!("item_id:{i}.0")]));
                found += usize::from(d.find(&r.unwrap(), true));
            }
            let _ = tx.send((d.len(), d.order.len(), found));
        });
        let (len, records, found) = rx
            .recv_timeout(Duration::from_secs(60))
            .expect("adding and matching took far too long");
        assert_eq!(len, PER_DEVICE);
        assert!(records <= 2 * PER_DEVICE + 64, "{records} records kept");
        assert_eq!(found, 0, "the first 20,000 were evicted long ago");
    }

    /// Refreshing keeps an action alive past its first offer's lifetime; expiry and eviction
    /// go by when an action was last offered; typed-input actions follow the same rules.
    #[test]
    fn refreshed_actions_live_on_and_expired_ones_go_everywhere() {
        let t0 = Instant::now();
        let mut d = DeviceActions::default();
        d.add(learn(&radio_menu()), t0);
        let n = d.len();
        let half = t0 + TTL / 2;
        // Only the search box is offered again.
        let search = learn(&radio_menu())
            .into_iter()
            .filter(|a| a.params.values().any(|v| *v == PValue::Input))
            .collect::<Vec<_>>();
        assert_eq!(search.len(), 1);
        d.add(search, half);
        assert_eq!(d.len(), n, "a refresh adds nothing");
        d.expire(t0 + TTL + Duration::from_secs(1));
        assert_eq!(d.len(), 1, "only the refreshed one is left");
        let typed = strs(&[
            "podcasts",
            "items",
            "0",
            "20",
            "search:in our time",
            "item_id:1",
            "menu:podcasts",
        ]);
        assert_eq!(allowed(&d, &typed, false), Ok(()));
        let other = strs(&[
            "radiofeeds",
            "items",
            "0",
            "50",
            "item_id:63f0.0",
            "menu:radiofeeds",
        ]);
        assert_eq!(allowed(&d, &other, false), Err(Refusal::NotOffered));
        d.expire(half + TTL);
        assert!(d.is_empty());
        assert!(d.with_input.is_empty(), "typed-input index emptied too");
        assert_eq!(allowed(&d, &typed, false), Err(Refusal::NotOffered));

        // Eviction drops the action offered longest ago: a refreshed old one survives.
        let mut d = DeviceActions::default();
        d.add(
            (0..PER_DEVICE).map(|i| item(&["y", "items"], i)).collect(),
            t0,
        );
        d.add(vec![item(&["y", "items"], 0)], t0 + Duration::from_secs(1));
        d.add(
            vec![item(&["y", "items"], PER_DEVICE)],
            t0 + Duration::from_secs(2),
        );
        let has = |d: &DeviceActions, i: usize| {
            d.find(
                &parse(&strs(&["y", "items", "0", "1", &format!("item_id:{i}.0")])).unwrap(),
                true,
            )
        };
        assert!(has(&d, 0), "refreshed, so not the oldest");
        assert!(!has(&d, 1), "the oldest went");
        assert!(has(&d, PER_DEVICE));
        assert_eq!(d.len(), PER_DEVICE);
    }

    #[test]
    fn hostile_menus_never_panic() {
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let pieces = [
            json!(null),
            json!(1),
            json!("s"),
            json!([]),
            json!({}),
            json!(["a"]),
            json!({"cmd": 1}),
            json!({"cmd": ["a", "items"], "params": {"k": [1]}}),
            json!({"cmd": ["a"], "params": "x"}),
            json!("__TAGGEDINPUT__"),
        ];
        for _ in 0..3000 {
            let p = |r: u64| pieces[(r % pieces.len() as u64) as usize].clone();
            let v = json!({"base": {"actions": {"go": p(rnd()), "play": {"cmd": p(rnd()), "itemsParams": p(rnd())}}},
                           "item_loop": [p(rnd()), {"actions": {"go": p(rnd())}, "params": p(rnd())}]});
            let _ = learn(&v);
        }
    }
}
