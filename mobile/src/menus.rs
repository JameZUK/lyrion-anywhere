//! LMS menus ("SlimBrowse"): apps, radio, plugin menus and context ("More…") menus.
//!
//! Each menu item offers actions (`go`, `play`, `add`, `add-hold`, `more`, …). An action is
//! either on the item itself, or on the menu's `base` combined with the item's parameters
//! (the field named by `itemsParams`). The gateway applies the same rules and lets a device
//! run only actions LMS offered it in a menu it relayed (`gateway/src/menu_actions.rs`), so
//! this module never invents commands: it resolves what a menu offered, exactly as the web
//! app does (`web/app/src/lib/lms/slimbrowse.ts`), and replays it.
//!
//! Everything LMS sends is untrusted: command words, parameter keys and values are checked
//! here (and again before anything is sent), text is kept as inert text, and icons become
//! artwork keys only in the forms the art relay accepts.

use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{Map, Value, json};

use crate::art::{cover_key, image_key};
use crate::commands::{self, Command, InfoKind};
use crate::error::ApiError;
use crate::model::clean_text;
use crate::session::Session;

/// Most command words in an action (as the gateway).
const MAX_WORDS: usize = 6;
/// Most parameters in an action (as the gateway).
const MAX_PARAMS: usize = 16;
/// Longest parameter value the gateway accepts.
const MAX_VALUE: usize = 256;
/// Longest text item shown (info menus can be wordy).
const MAX_ITEM_TEXT: usize = 4000;
/// Items read from one menu page.
const MAX_ITEMS: usize = 2000;
/// Placeholders LMS puts where the user's typed text goes.
const INPUT_MARKERS: [&str; 2] = ["__TAGGEDINPUT__", "__INPUT__"];
/// Apps looked at for "Search on", at most (LMS lists a handful).
const MAX_SEARCHABLE: usize = 64;
/// Top menus read at once for "Search on" (LMS is single-threaded).
const SEARCH_PARALLEL: usize = 3;
/// Items of a top menu read to find its search box.
const TOP_ITEMS: u32 = 100;

/// One `key:value` parameter of an action.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuParam {
    pub key: String,
    pub value: String,
}

/// A runnable action LMS offered: command words and parameters, and where to go after.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuAction {
    pub cmd: Vec<String>,
    pub params: Vec<MenuParam>,
    /// LMS's hint for what to show next (`nowPlaying`, `parent`, `refresh`, …).
    pub next_window: Option<String>,
    /// Opens a menu (paged) rather than running a command.
    pub opens_menu: bool,
    /// Needs text typed by the user (a search box).
    pub needs_input: bool,
}

/// How to present a menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MenuItemKind {
    /// Opens another menu.
    Folder,
    /// A search box: type, then open.
    Search,
    /// Plays when chosen.
    Playable,
    /// Runs something (e.g. makes a mix) rather than opening a menu.
    Action,
    /// Text only.
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuItem {
    /// Plain text; text items may span lines.
    pub text: String,
    pub kind: MenuItemKind,
    /// Artwork key for the item's icon, if it has a usable one.
    pub icon: Option<String>,
    pub go: Option<MenuAction>,
    pub play: Option<MenuAction>,
    pub add: Option<MenuAction>,
    /// "Play next" (`add-hold`, or `insert`).
    pub play_next: Option<MenuAction>,
    pub more: Option<MenuAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MenuPage {
    pub title: Option<String>,
    /// Items in the whole menu (for paging).
    pub total: u32,
    pub items: Vec<MenuItem>,
}

/// What running an action did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MenuOutcome {
    /// It ran. `next_window` is the action's hint for what to show next.
    Done { next_window: Option<String> },
    /// It answered with a menu to show (e.g. a list of choices).
    Menu { page: MenuPage },
}

/// An app or radio service LMS lists (`apps`, `radios`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppEntry {
    pub cmd: String,
    pub name: String,
    pub icon: Option<String>,
    /// Opens its top menu.
    pub menu: MenuAction,
}

/// An entry plugins add to LMS's home menu (My Music, Radio, Extras, …).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PluginEntry {
    pub id: String,
    pub text: String,
    pub icon: Option<String>,
    pub weight: i64,
    /// Opens this (a plugin menu), or `None` for a group of other entries.
    pub go: Option<MenuAction>,
    /// Entries under this one (e.g. Random Mix's mixes).
    pub children: Vec<MenuItem>,
}

/// An app or radio service whose top menu has a search box ("Search on …").
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchableApp {
    pub cmd: String,
    pub name: String,
    pub icon: Option<String>,
    /// Its search box's own action: opens the results, with the typed text as `search:`.
    pub search: MenuAction,
}

/// Whether an app or plugin is usable from here, judged from its top menu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Suitability {
    Usable,
    /// All it says is a message, e.g. "Log in to … in Advanced settings".
    NeedsSetup {
        message: String,
    },
    Empty,
    Failed {
        message: String,
    },
}

fn word_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Parameter values from LMS or the user: bounded and printable.
fn value_ok(s: &str) -> bool {
    s.len() <= MAX_VALUE && !s.chars().any(char::is_control)
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => Some(n.to_string()),
        Value::Bool(b) => Some((if *b { "1" } else { "0" }).into()),
        _ => None,
    }
}

fn has_input(s: &str) -> bool {
    INPUT_MARKERS.iter().any(|m| s.contains(m))
}

/// An action from its JSON (`a`), the item, and the item's parameters for base actions.
fn to_action(
    name: &str,
    item: &Map<String, Value>,
    a: &Value,
    extra: Option<&Value>,
) -> Option<MenuAction> {
    let act = a.as_object()?;
    let words = act.get("cmd")?.as_array()?;
    if words.is_empty() || words.len() > MAX_WORDS {
        return None;
    }
    let cmd: Vec<String> = words
        .iter()
        .map(|w| w.as_str().filter(|w| word_ok(w)).map(str::to_owned))
        .collect::<Option<_>>()?;
    let mut params: Vec<MenuParam> = Vec::new();
    for src in [act.get("params"), extra].into_iter().flatten() {
        let Some(map) = src.as_object() else { continue };
        for (k, v) in map {
            let Some(s) = scalar(v) else { continue };
            // Never an action carrying a URL (a path on the server) or an odd key.
            if !word_ok(k) || k == "url" || !value_ok(&s) {
                return None;
            }
            match params.iter_mut().find(|p| p.key == *k) {
                Some(p) => p.value = s,
                None => params.push(MenuParam {
                    key: k.clone(),
                    value: s,
                }),
            }
        }
    }
    if params.len() > MAX_PARAMS {
        return None;
    }
    let next_window = act
        .get("nextWindow")
        .and_then(Value::as_str)
        .or_else(|| item.get("nextWindow").and_then(Value::as_str))
        .map(|s| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(32)
                .collect::<String>()
        });
    // As in LMS's own clients (and the gateway): `… items` lists are menus, and so is any
    // `go` that doesn't say where to go next; with `nextWindow` it is a command that runs.
    let opens_menu =
        cmd.last().is_some_and(|w| w == "items") || (name == "go" && next_window.is_none());
    let needs_input = params.iter().any(|p| has_input(&p.value));
    Some(MenuAction {
        cmd,
        params,
        next_window,
        opens_menu,
        needs_input,
    })
}

/// The action `name` for an item in a menu (with its `base`), or `None` if not offered.
fn resolve(
    base: Option<&Map<String, Value>>,
    item: &Map<String, Value>,
    name: &str,
) -> Option<MenuAction> {
    if let Some(own) = item
        .get("actions")
        .and_then(Value::as_object)
        .and_then(|a| a.get(name))
    {
        return to_action(name, item, own, None);
    }
    let b = base?.get(name)?;
    let field = b
        .get("itemsParams")
        .and_then(Value::as_str)
        .unwrap_or("params");
    let item_params = item.get(field).filter(|p| p.is_object())?;
    to_action(name, item, b, Some(item_params))
}

/// Display text for a menu item: newlines kept, other control characters dropped.
fn menu_text(v: &Value) -> String {
    match v {
        Value::String(s) => s
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .take(MAX_ITEM_TEXT)
            .collect::<String>()
            .trim()
            .to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Artwork key for an icon LMS named (image, icon or icon id).
fn icon_key(v: Option<&Value>) -> Option<String> {
    let v = v?;
    image_key(v).or_else(|| cover_key(v))
}

fn item_from(base: Option<&Map<String, Value>>, item: &Map<String, Value>) -> MenuItem {
    let go = resolve(base, item, "go");
    let play = resolve(base, item, "play");
    let add = resolve(base, item, "add");
    let play_next = resolve(base, item, "add-hold").or_else(|| resolve(base, item, "insert"));
    let more = resolve(base, item, "more");
    let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
    let kind = match (&go, &play) {
        (Some(g), _) if g.needs_input => MenuItemKind::Search,
        (Some(_), _) if item.get("input").is_some_and(Value::is_object) => MenuItemKind::Search,
        // Audio items play when chosen, even if they can also be opened.
        (_, Some(_)) if ty == "audio" => MenuItemKind::Playable,
        (Some(g), _) if g.opens_menu => MenuItemKind::Folder,
        (g, p) if p.is_some() || ty == "audio" || ty == "playlist" => {
            if p.is_some() || g.is_some() {
                MenuItemKind::Playable
            } else {
                MenuItemKind::Text
            }
        }
        (Some(_), _) => MenuItemKind::Action,
        _ => MenuItemKind::Text,
    };
    let icon = icon_key(
        item.get("image")
            .or_else(|| item.get("icon"))
            .or_else(|| item.get("icon-id")),
    );
    MenuItem {
        text: menu_text(item.get("text").unwrap_or(&Value::Null)),
        kind,
        icon,
        go,
        play,
        add,
        play_next,
        more,
    }
}

/// A menu page from LMS's answer.
pub(crate) fn page_from(v: &Value) -> MenuPage {
    let base = v
        .get("base")
        .and_then(|b| b.get("actions"))
        .and_then(Value::as_object);
    let items: Vec<MenuItem> = v
        .get("item_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .take(MAX_ITEMS)
        .map(|i| item_from(base, i))
        .collect();
    let total = v
        .get("count")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(items.len() as u32)
        .max(items.len() as u32);
    MenuPage {
        title: v
            .get("title")
            .and_then(Value::as_str)
            .map(clean_text)
            .filter(|t| !t.trim().is_empty()),
        total,
        items,
    }
}

/// The command to send: words, paging for menus, then parameters (typed text filled in).
/// Everything is checked again here: the action may have been held by the app for a while.
pub fn to_command(
    action: &MenuAction,
    page: Option<(u32, u32)>,
    input: Option<&str>,
) -> Result<Command, ApiError> {
    if action.cmd.is_empty()
        || action.cmd.len() > MAX_WORDS
        || !action.cmd.iter().all(|w| word_ok(w))
        || action.params.len() > MAX_PARAMS
    {
        return Err(ApiError::invalid("not a menu action"));
    }
    let mut out: Command = action.cmd.iter().map(|w| json!(w)).collect();
    if action.opens_menu {
        let (start, n) = page.unwrap_or((0, 100));
        out.push(json!(start));
        out.push(json!(n.clamp(1, commands::MAX_PAGE)));
    }
    let typed = input.map(commands::text).unwrap_or_default();
    let mut seen: Vec<&str> = Vec::new();
    for p in &action.params {
        if !word_ok(&p.key)
            || p.key == "url"
            || !value_ok(&p.value)
            || seen.contains(&p.key.as_str())
        {
            return Err(ApiError::invalid("not a menu action"));
        }
        seen.push(&p.key);
        let value = if has_input(&p.value) {
            if typed.is_empty() {
                return Err(ApiError::invalid("enter something to search for"));
            }
            typed.clone()
        } else {
            p.value.clone()
        };
        out.push(json!(format!("{}:{value}", p.key)));
    }
    Ok(out)
}

/// The "More…" (context) menu for a library item.
#[uniffi::export]
pub fn info_menu(kind: InfoKind, item_id: i64) -> Result<MenuAction, ApiError> {
    let (key, value) = kind.selector(item_id)?;
    Ok(MenuAction {
        cmd: vec![kind.command().into(), "items".into()],
        params: vec![
            MenuParam { key, value },
            MenuParam {
                key: "menu".into(),
                value: "1".into(),
            },
        ],
        next_window: None,
        opens_menu: true,
        needs_input: false,
    })
}

/// An app's top menu, in menu (SlimBrowse) format.
#[uniffi::export]
pub fn app_menu(app: String) -> Result<MenuAction, ApiError> {
    let a = commands::app_command(&app)?;
    Ok(MenuAction {
        cmd: vec![a.into(), "items".into()],
        params: vec![MenuParam {
            key: "menu".into(),
            value: "1".into(),
        }],
        next_window: None,
        opens_menu: true,
        needs_input: false,
    })
}

/// Apps or radio services from an `apps`/`radios` answer.
pub(crate) fn apps_from(v: &Value, key: &str) -> Vec<AppEntry> {
    let mut out: Vec<AppEntry> = Vec::new();
    for x in v
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        let Some(cmd) = x.get("cmd").and_then(Value::as_str) else {
            continue;
        };
        let Ok(menu) = app_menu(cmd.to_string()) else {
            continue;
        };
        if out.iter().any(|a| a.cmd == cmd) {
            continue;
        }
        let name = x
            .get("name")
            .and_then(Value::as_str)
            .map(clean_text)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| cmd.to_string());
        out.push(AppEntry {
            cmd: cmd.to_string(),
            name,
            icon: icon_key(x.get("icon").or_else(|| x.get("image"))),
            menu,
        });
    }
    out
}

fn entry_id_ok(s: &str) -> bool {
    (1..=96).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// Plugin entries from the home menu (`menu`), for listing beside apps and radio:
/// everything the gateway relayed, except what the app already shows its own way (the
/// library, apps and radio listed by `apps`/`radios`) and entries with nothing to open.
pub fn plugin_entries(home: &Value, shown_apps: &[String]) -> Vec<PluginEntry> {
    let items: Vec<&Map<String, Value>> = home
        .get("item_loop")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|i| i.get("id").and_then(Value::as_str).is_some_and(entry_id_ok))
        .take(MAX_ITEMS)
        .collect();
    let id_of = |i: &Map<String, Value>| {
        i.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let node_of = |i: &Map<String, Value>| {
        i.get("node")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut out = Vec::new();
    for item in &items {
        let id = id_of(item);
        let node = node_of(item);
        if id.starts_with("myMusic")
            || ["opmlmyapps", "favorites", "globalSearch", "radios"].contains(&id.as_str())
        {
            continue;
        }
        // Entries under another entry are shown inside it.
        if items.iter().any(|p| id_of(p) == node) {
            continue;
        }
        let go = resolve(None, item, "go");
        if go
            .as_ref()
            .is_some_and(|g| shown_apps.iter().any(|a| *a == g.cmd[0]))
        {
            continue;
        }
        let children: Vec<MenuItem> = items
            .iter()
            .filter(|c| node_of(c) == id)
            .map(|c| item_from(None, c))
            .collect();
        let go = go.filter(|g| g.opens_menu);
        if go.is_none() && children.is_empty() {
            continue;
        }
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .map(clean_text)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let icon = icon_key(
            item.get("icon")
                .or_else(|| item.get("window").and_then(|w| w.get("icon-id")))
                .or_else(|| item.get("menuIcon")),
        );
        out.push(PluginEntry {
            id,
            text,
            icon,
            weight: item
                .get("weight")
                .and_then(Value::as_f64)
                .filter(|w| w.is_finite())
                .map_or(100, |w| w as i64),
            go,
            children,
        });
    }
    out.sort_by(|a, b| a.weight.cmp(&b.weight).then_with(|| a.text.cmp(&b.text)));
    out
}

/// Whether an action is a search box for `search:` text: it opens a menu, and its one and
/// only typed-text parameter is `search` (exactly LMS's placeholder). Text going anywhere
/// else (another parameter, or several) is never used for "Search on".
pub fn is_search_box(action: &MenuAction) -> bool {
    let typed: Vec<&MenuParam> = action
        .params
        .iter()
        .filter(|p| has_input(&p.value))
        .collect();
    action.opens_menu
        && action.needs_input
        && typed.len() == 1
        && typed[0].key == "search"
        && INPUT_MARKERS.contains(&typed[0].value.as_str())
}

/// The first search box among a top menu's items, if it has one.
pub fn search_box(page: &MenuPage) -> Option<MenuAction> {
    page.items
        .iter()
        .filter(|i| i.kind == MenuItemKind::Search)
        .filter_map(|i| i.go.as_ref())
        .find(|a| is_search_box(a))
        .cloned()
}

/// Judge an app from its top menu: usable if it offers anything to open, play or search;
/// "needs setup" if all it says is a message; otherwise empty. Nothing is hard-coded per app.
pub fn judge(page: &MenuPage) -> Suitability {
    if page.items.iter().any(|i| i.kind != MenuItemKind::Text) {
        return Suitability::Usable;
    }
    match page
        .items
        .iter()
        .map(|i| i.text.trim())
        .find(|t| !t.is_empty())
    {
        Some(m) => Suitability::NeedsSetup {
            message: m.chars().take(200).collect(),
        },
        None => Suitability::Empty,
    }
}

/// The verdict for an app whose top menu couldn't be opened.
pub fn judge_error(e: &ApiError) -> Suitability {
    Suitability::Failed {
        message: match e {
            ApiError::Forbidden { .. } => "Not available remotely.".into(),
            _ => "The music server could not open it.".into(),
        },
    }
}

/// LMS menus for a player: apps, radio, plugin entries and context menus.
#[derive(uniffi::Object)]
pub struct Menus {
    session: Arc<Session>,
}

impl Menus {
    async fn send(&self, player: &str, cmd: Command) -> Result<Value, ApiError> {
        let p = commands::player_id(player)?;
        self.session.core().rpc(&p, cmd).await
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Menus {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Menus { session })
    }

    /// Apps installed in LMS (Spotty, Qobuz, …).
    pub async fn apps(&self) -> Result<Vec<AppEntry>, ApiError> {
        let v = self.session.core().rpc("", commands::apps_list()).await?;
        Ok(apps_from(&v, "appss_loop"))
    }

    /// Radio services (TuneIn, BBC Sounds, …).
    pub async fn radios(&self) -> Result<Vec<AppEntry>, ApiError> {
        let v = self.session.core().rpc("", commands::radios_list()).await?;
        Ok(apps_from(&v, "radioss_loop"))
    }

    /// Plugins that add themselves to the player's home menu, except the apps in
    /// `shown_apps` (listed already).
    pub async fn plugins(
        &self,
        player: String,
        shown_apps: Vec<String>,
    ) -> Result<Vec<PluginEntry>, ApiError> {
        let v = self.send(&player, commands::home_menu()).await?;
        Ok(plugin_entries(&v, &shown_apps))
    }

    /// Open a menu (`count` at most 500) from `start`, with typed text for a search.
    pub async fn open(
        &self,
        player: String,
        action: MenuAction,
        start: u32,
        count: u32,
        input: Option<String>,
    ) -> Result<MenuPage, ApiError> {
        if !action.opens_menu {
            return Err(ApiError::invalid("this action doesn't open a menu"));
        }
        let cmd = to_command(&action, Some((start, count)), input.as_deref())?;
        Ok(page_from(&self.send(&player, cmd).await?))
    }

    /// Run an action that doesn't open a menu (play, add, make a mix, …).
    pub async fn run(
        &self,
        player: String,
        action: MenuAction,
        input: Option<String>,
    ) -> Result<MenuOutcome, ApiError> {
        let cmd = to_command(&action, None, input.as_deref())?;
        let v = self.send(&player, cmd).await?;
        let page = page_from(&v);
        Ok(if page.items.is_empty() {
            MenuOutcome::Done {
                next_window: action.next_window,
            }
        } else {
            MenuOutcome::Menu { page }
        })
    }

    /// The apps (of `apps`, as listed by [`Menus::apps`] and [`Menus::radios`]) whose top
    /// menu has a search box, in the order given: each top menu is read again here (a few
    /// at a time), from the app's own command, never from an action handed in. An app whose
    /// menu can't be read is left out.
    pub async fn searchable(
        &self,
        player: String,
        apps: Vec<AppEntry>,
    ) -> Result<Vec<SearchableApp>, ApiError> {
        commands::player_id(&player)?;
        let mut wanted: Vec<AppEntry> = Vec::new();
        for a in apps {
            if wanted.len() >= MAX_SEARCHABLE {
                break;
            }
            if commands::app_command(&a.cmd).is_ok() && !wanted.iter().any(|w| w.cmd == a.cmd) {
                wanted.push(a);
            }
        }
        let found: Vec<Option<SearchableApp>> = futures_util::stream::iter(wanted)
            .map(|a| {
                let player = player.clone();
                async move {
                    let top = app_menu(a.cmd.clone()).ok()?;
                    let page = self.open(player, top, 0, TOP_ITEMS, None).await.ok()?;
                    Some(SearchableApp {
                        search: search_box(&page)?,
                        cmd: a.cmd,
                        name: a.name,
                        icon: a.icon,
                    })
                }
            })
            .buffered(SEARCH_PARALLEL)
            .collect()
            .await;
        Ok(found.into_iter().flatten().collect())
    }

    /// How usable an app or plugin is from here, from the first few items of its menu.
    pub async fn probe(&self, player: String, action: MenuAction) -> Suitability {
        match self.open(player, action, 0, 10, None).await {
            Ok(page) => judge(&page),
            Err(e) => judge_error(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes from real LMS menus (radio, podcasts, track info, a plugin), anonymised.
    fn radio_menu() -> Value {
        json!({
            "count": 7, "title": "Radio",
            "base": {"actions": {
                "play": {"cmd": ["radiofeeds", "playlist", "play"], "params": {"menu": "radiofeeds"}, "itemsParams": "params", "player": 0},
                "add": {"cmd": ["radiofeeds", "playlist", "add"], "params": {"menu": "radiofeeds"}, "itemsParams": "params"},
                "add-hold": {"cmd": ["radiofeeds", "playlist", "insert"], "params": {"menu": "radiofeeds"}, "itemsParams": "params"},
                "go": {"cmd": ["radiofeeds", "items"], "params": {"menu": "radiofeeds"}, "itemsParams": "params"}
            }},
            "item_loop": [
                {"text": "By name", "type": "link", "actions": {"go": {"cmd": ["radiofeeds", "items"], "params": {"item_id": "63f0.0", "menu": "radiofeeds"}}}},
                {"text": "Station", "type": "audio", "params": {"item_id": "63f0.1.4"}, "image": "http://radio.example.com/logo.png"},
                {"text": "Search", "type": "search", "actions": {"go": {"cmd": ["podcasts", "items"], "params": {"search": "__TAGGEDINPUT__", "item_id": "1", "menu": "podcasts"}}}},
                {"text": "Make a mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["someplugin", "mix"], "params": {"track_id": 7}}}},
                {"text": "Bad", "actions": {"go": {"cmd": ["jivefavorites", "add"], "params": {"url": "file:///srv/x.flac"}}}},
                {"text": "Line one\nLine two\u{0}"},
                "garbage"
            ]
        })
    }

    #[test]
    fn items_resolve_their_own_and_base_actions() {
        let p = page_from(&radio_menu());
        assert_eq!(p.title.as_deref(), Some("Radio"));
        assert_eq!(p.total, 7);
        assert_eq!(p.items.len(), 6);

        let folder = &p.items[0];
        assert_eq!(folder.kind, MenuItemKind::Folder);
        let go = folder.go.as_ref().unwrap();
        assert!(go.opens_menu);
        assert_eq!(go.cmd, ["radiofeeds", "items"]);

        // Base actions take the item's parameters; the item's override the base's.
        let station = &p.items[1];
        assert_eq!(station.kind, MenuItemKind::Playable);
        let play = station.play.as_ref().unwrap();
        assert_eq!(play.cmd, ["radiofeeds", "playlist", "play"]);
        assert!(!play.opens_menu);
        assert_eq!(
            to_command(play, None, None).unwrap(),
            vec![
                json!("radiofeeds"),
                json!("playlist"),
                json!("play"),
                json!("menu:radiofeeds"),
                json!("item_id:63f0.1.4")
            ]
        );
        assert_eq!(station.play_next.as_ref().unwrap().cmd[2], "insert");
        assert!(station.add.is_some());
        assert!(station.icon.as_deref().unwrap().starts_with("imageproxy/"));

        let search = &p.items[2];
        assert_eq!(search.kind, MenuItemKind::Search);
        assert!(search.go.as_ref().unwrap().needs_input);

        let mix = &p.items[3];
        assert_eq!(mix.kind, MenuItemKind::Action);
        assert_eq!(
            mix.go.as_ref().unwrap().next_window.as_deref(),
            Some("nowPlaying")
        );
        assert!(!mix.go.as_ref().unwrap().opens_menu);

        // An action carrying a URL is never offered.
        assert_eq!(p.items[4].go, None);
        assert_eq!(p.items[4].kind, MenuItemKind::Text);
        assert_eq!(p.items[5].text, "Line one\nLine two");
    }

    #[test]
    fn commands_page_menus_and_fill_in_typed_text() {
        let p = page_from(&radio_menu());
        let go = p.items[0].go.as_ref().unwrap();
        assert_eq!(
            to_command(go, Some((100, 9999)), None).unwrap(),
            vec![
                json!("radiofeeds"),
                json!("items"),
                json!(100),
                json!(500),
                json!("item_id:63f0.0"),
                json!("menu:radiofeeds")
            ]
        );
        let search = p.items[2].go.as_ref().unwrap();
        let c = to_command(search, Some((0, 100)), Some(" abba\u{0}gold ")).unwrap();
        assert!(c.contains(&json!("search:abba gold")), "{c:?}");
        assert!(matches!(
            to_command(search, Some((0, 100)), Some("   ")),
            Err(ApiError::InvalidInput { .. })
        ));
        assert_eq!(
            to_command(
                &info_menu(InfoKind::Track, 7).unwrap(),
                Some((0, 100)),
                None
            )
            .unwrap(),
            vec![
                json!("trackinfo"),
                json!("items"),
                json!(0),
                json!(100),
                json!("track_id:7"),
                json!("menu:1")
            ]
        );
        assert_eq!(
            to_command(&app_menu("spotty".into()).unwrap(), Some((0, 10)), None).unwrap(),
            vec![
                json!("spotty"),
                json!("items"),
                json!(0),
                json!(10),
                json!("menu:1")
            ]
        );
    }

    /// Hostile menus: nothing odd becomes an action, and nothing odd is ever sent.
    #[test]
    fn hostile_actions_are_never_offered_or_sent() {
        let v = json!({"item_loop": [
            {"text": "a", "actions": {"go": {"cmd": ["../x", "items"]}}},
            {"text": "b", "actions": {"go": {"cmd": []}}},
            {"text": "c", "actions": {"go": {"cmd": ["a", "b", "c", "d", "e", "f", "g"]}}},
            {"text": "d", "actions": {"go": {"cmd": ["x", "items"], "params": {"bad key": "1"}}}},
            {"text": "e", "actions": {"go": {"cmd": ["x", "items"], "params": {"q": "a\r\nb"}}}},
            {"text": "f", "actions": {"go": {"cmd": ["x", "items"], "params": {"q": "x".repeat(300)}}}},
            {"text": "g", "actions": {"go": {"cmd": [1, 2]}}},
            {"text": "h", "actions": {"go": "nonsense"}},
            {"text": "i", "actions": {"go": {"cmd": ["x", "items"], "params": {"URL": 1, "url": "http://example.com/"}}}}
        ]});
        let p = page_from(&v);
        assert_eq!(p.items.len(), 9);
        for i in &p.items {
            assert_eq!(i.go, None, "{}", i.text);
            assert_eq!(i.kind, MenuItemKind::Text);
        }
        // Actions held by the app are checked again before they are sent.
        let ok = app_menu("spotty".into()).unwrap();
        for bad in [
            MenuAction {
                cmd: vec![],
                ..ok.clone()
            },
            MenuAction {
                cmd: vec!["sp otty".into()],
                ..ok.clone()
            },
            MenuAction {
                cmd: vec!["x".into(); 7],
                ..ok.clone()
            },
            MenuAction {
                params: vec![MenuParam {
                    key: "url".into(),
                    value: "x".into(),
                }],
                ..ok.clone()
            },
            MenuAction {
                params: vec![MenuParam {
                    key: "a:b".into(),
                    value: "x".into(),
                }],
                ..ok.clone()
            },
            MenuAction {
                params: vec![MenuParam {
                    key: "q".into(),
                    value: "a\nb".into(),
                }],
                ..ok.clone()
            },
            MenuAction {
                params: vec![MenuParam {
                    key: "q".into(),
                    value: "x".repeat(257),
                }],
                ..ok.clone()
            },
            MenuAction {
                params: vec![
                    MenuParam {
                        key: "q".into(),
                        value: "1".into(),
                    },
                    MenuParam {
                        key: "q".into(),
                        value: "2".into(),
                    },
                ],
                ..ok.clone()
            },
            MenuAction {
                params: (0..17)
                    .map(|i| MenuParam {
                        key: format!("k{i}"),
                        value: "1".into(),
                    })
                    .collect(),
                ..ok.clone()
            },
        ] {
            assert!(to_command(&bad, Some((0, 10)), None).is_err(), "{bad:?}");
        }
        assert!(app_menu("Spotty".into()).is_err());
        assert!(info_menu(InfoKind::Album, 0).is_err());
    }

    #[test]
    fn plugin_entries_come_from_the_home_menu() {
        let home = json!({"item_loop": [
            {"id": "myMusicAlbums", "node": "myMusic", "actions": {"go": {"cmd": ["browselibrary", "items"]}}},
            {"id": "pluginSomePlugin", "node": "myMusic", "text": "Some Plugin", "weight": 20, "actions": {"go": {"cmd": ["someplugin", "items"], "params": {"menu": 1}}}},
            {"id": "randomplay", "node": "myMusic", "isANode": 1, "text": "Random Mix", "weight": 10},
            {"id": "randomtracks", "node": "randomplay", "text": "Song mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["randomplay", "tracks"]}}},
            {"id": "opmlspotty", "node": "", "text": "Spotty", "actions": {"go": {"cmd": ["spotty", "items"], "params": {"menu": "spotty"}}}},
            {"id": "notmenu", "node": "", "text": "Runs", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["x", "do"]}}},
            {"id": "favorites", "node": "home", "actions": {"go": {"cmd": ["favorites", "items"]}}},
            {"id": "bad id!", "node": "", "actions": {"go": {"cmd": ["x", "items"]}}}
        ]});
        let e = plugin_entries(&home, &["spotty".into()]);
        let ids: Vec<&str> = e.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["randomplay", "pluginSomePlugin"]);
        assert_eq!(e[0].go, None);
        assert_eq!(e[0].children.len(), 1);
        assert_eq!(e[0].children[0].kind, MenuItemKind::Action);
        assert!(e[1].go.as_ref().unwrap().opens_menu);
    }

    #[test]
    fn suitability_follows_the_top_menu() {
        let page = |items: Value| page_from(&json!({ "item_loop": items }));
        assert_eq!(judge(&page_from(&radio_menu())), Suitability::Usable);
        assert_eq!(
            judge(&page(
                json!([{"text": "  "}, {"text": "Log in to Example in Advanced settings"}])
            )),
            Suitability::NeedsSetup {
                message: "Log in to Example in Advanced settings".into()
            }
        );
        assert_eq!(judge(&page(json!([]))), Suitability::Empty);
        assert_eq!(
            judge_error(&ApiError::Forbidden { detail: "x".into() }),
            Suitability::Failed {
                message: "Not available remotely.".into()
            }
        );
        assert!(matches!(
            judge_error(&ApiError::Network { detail: "x".into() }),
            Suitability::Failed { .. }
        ));
    }

    #[test]
    fn apps_need_valid_commands() {
        let v = json!({"appss_loop": [
            {"cmd": "spotty", "name": "Spotty", "icon": "plugins/Spotty/html/images/icon.png"},
            {"cmd": "spotty", "name": "Again"},
            {"cmd": "Bad-App", "name": "Bad"},
            {"name": "No command"},
            {"cmd": "qobuz"}
        ]});
        let a = apps_from(&v, "appss_loop");
        assert_eq!(a.len(), 2);
        assert_eq!(
            a[0].icon.as_deref(),
            Some("plugins/Spotty/html/images/icon.png")
        );
        assert_eq!(a[1].name, "qobuz");
    }

    /// Arbitrary JSON never panics and never yields an unsafe action.
    #[test]
    fn robust_on_arbitrary_json() {
        let samples = [
            json!(null),
            json!([]),
            json!({"item_loop": {}}),
            json!({"item_loop": [null, 1, [], {"actions": []}, {"actions": {"go": null}}]}),
            json!({"base": [], "item_loop": [{"params": "x"}]}),
            json!({"base": {"actions": {"go": {"cmd": ["x", "items"], "itemsParams": 5}}}, "item_loop": [{"params": {"a": [1]}}]}),
            json!({"count": -1, "title": 5, "item_loop": [{"text": {"a": 1}, "icon-id": -3, "weight": "x"}]}),
            json!({"item_loop": [{"id": "a", "node": "a", "actions": {"go": {"cmd": ["x", "items"]}}}]}),
        ];
        for v in samples {
            let p = page_from(&v);
            for i in &p.items {
                for a in [&i.go, &i.play, &i.add, &i.play_next, &i.more]
                    .into_iter()
                    .flatten()
                {
                    assert!(to_command(a, Some((0, 10)), Some("x")).is_ok());
                }
            }
            let _ = plugin_entries(&v, &[]);
            let _ = apps_from(&v, "appss_loop");
            let _ = judge(&p);
        }
    }

    /// "Search on": a search box is a menu with the typed text as `search:` and nowhere else.
    #[test]
    fn search_boxes_are_found_generically() {
        let one = |go: Value, extra: Value| {
            let mut item = json!({"text": "Search", "type": "search", "actions": {"go": go}});
            if let (Some(i), Some(e)) = (item.as_object_mut(), extra.as_object()) {
                for (k, v) in e {
                    i.insert(k.clone(), v.clone());
                }
            }
            page_from(&json!({ "item_loop": [item] }))
        };
        // Own action, and through the menu's base actions.
        let own = one(
            json!({"cmd": ["exampleapp", "items"], "params": {"item_id": "0", "search": "__TAGGEDINPUT__", "menu": "exampleapp"}}),
            json!({}),
        );
        let b = search_box(&own).unwrap();
        assert_eq!(b.cmd, ["exampleapp", "items"]);
        assert!(is_search_box(&b));
        let base = page_from(&json!({
            "base": {"actions": {"go": {"cmd": ["exampleradio", "items"], "params": {"menu": "exampleradio"}, "itemsParams": "params"}}},
            "item_loop": [
                {"text": "Browse", "type": "link", "params": {"item_id": "1"}},
                {"text": "Find", "type": "search", "params": {"item_id": "2", "search": "__INPUT__"}}
            ]
        }));
        assert!(search_box(&base).is_some());
        // The radio menu's podcast search counts too; a menu without one has none.
        assert!(search_box(&page_from(&radio_menu())).is_some());
        assert_eq!(
            search_box(&page_from(&json!({"item_loop": [{"text": "x"}]}))),
            None
        );

        for not in [
            // Text into another parameter, into part of one, or into two.
            one(
                json!({"cmd": ["x", "items"], "params": {"q": "__TAGGEDINPUT__"}}),
                json!({}),
            ),
            one(
                json!({"cmd": ["x", "items"], "params": {"search": "a __TAGGEDINPUT__"}}),
                json!({}),
            ),
            one(
                json!({"cmd": ["x", "items"], "params": {"search": "__TAGGEDINPUT__", "title": "__INPUT__"}}),
                json!({}),
            ),
            // A command that runs rather than a menu of results.
            one(
                json!({"cmd": ["x", "add"], "params": {"search": "__TAGGEDINPUT__"}}),
                json!({"nextWindow": "nowPlaying"}),
            ),
            // Malformed or carrying a URL: never an action at all.
            one(
                json!({"cmd": ["../x", "items"], "params": {"search": "__TAGGEDINPUT__"}}),
                json!({}),
            ),
            one(
                json!({"cmd": ["x", "items"], "params": {"search": "__TAGGEDINPUT__", "url": "file:///srv/music"}}),
                json!({}),
            ),
        ] {
            assert_eq!(search_box(&not), None, "{not:?}");
        }
        // An action changed after it was found no longer counts.
        assert!(!is_search_box(&MenuAction {
            opens_menu: false,
            ..b.clone()
        }));
        assert!(!is_search_box(&MenuAction {
            needs_input: false,
            ..b.clone()
        }));
        assert!(!is_search_box(&app_menu("exampleapp".into()).unwrap()));
    }
}
