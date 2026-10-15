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

use std::collections::{BTreeMap, HashMap, VecDeque};
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
#[derive(Debug, Default)]
pub struct DeviceActions {
    by_cmd: HashMap<Vec<String>, Vec<(Action, Instant)>>,
    order: VecDeque<(Vec<String>, Instant)>,
    len: usize,
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
            let list = self.by_cmd.entry(a.cmd.clone()).or_default();
            if let Some(existing) = list.iter_mut().find(|(x, _)| *x == a) {
                existing.1 = now; // refresh
                continue;
            }
            self.order.push_back((a.cmd.clone(), now));
            list.push((a, now));
            self.len += 1;
        }
        while self.len > PER_DEVICE {
            self.evict_oldest();
        }
    }

    fn evict_oldest(&mut self) {
        let Some((cmd, _)) = self.order.pop_front() else {
            return;
        };
        if let Some(list) = self.by_cmd.get_mut(&cmd)
            && !list.is_empty()
        {
            // Drop the oldest action for that command.
            let idx = list
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(i, _)| i)
                .unwrap_or(0);
            list.remove(idx);
            self.len -= 1;
            if list.is_empty() {
                self.by_cmd.remove(&cmd);
            }
        }
    }

    pub fn expire(&mut self, now: Instant) {
        for list in self.by_cmd.values_mut() {
            let before = list.len();
            list.retain(|(_, t)| now.duration_since(*t) < TTL);
            self.len -= before - list.len();
        }
        self.by_cmd.retain(|_, l| !l.is_empty());
        self.order.retain(|(_, t)| now.duration_since(*t) < TTL);
    }

    fn find(&self, r: &Request, strict: bool) -> bool {
        self.by_cmd
            .get(&r.cmd)
            .is_some_and(|l| l.iter().any(|(a, _)| a.matches(r, strict)))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Every action held (to hand them to a device: see `crate::stations`).
    pub fn into_actions(self) -> Vec<Action> {
        self.by_cmd
            .into_values()
            .flatten()
            .map(|(a, _)| a)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
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
