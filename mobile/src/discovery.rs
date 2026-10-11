//! Discovery: "Start radio" and "More like this", from what this LMS offers.
//!
//! Nothing here is built in; it uses what the server has, found as the web app finds
//! plugins (by asking, never assuming):
//!
//! - **AudioMuse-AI** (an LMS plugin) adds entries to LMS's context menus for tracks, albums
//!   and artists: mixes that play at once, and lists of similar music. We read the item's
//!   own context menu (`trackinfo`/`albuminfo`/`artistinfo`), pick AudioMuse-AI's entries
//!   out of it ([`audiomuse_offers`]) and run them exactly as offered, so the gateway's
//!   learned-action check applies (`gateway/src/menu_actions.rs`).
//! - **Don't Stop The Music** (built into LMS) adds similar or random tracks to a player's
//!   queue whenever it runs low, from a provider chosen per player. Its settings menu
//!   (`dontstopthemusicsetting`) lists the providers installed and the one in use
//!   ([`dstm_from`]); the gateway allows exactly that query and setting the provider to a
//!   strict token, for a player the user may control.
//!
//! "Start radio" ([`radio_plan`]) plays AudioMuse-AI's mix where there is one, otherwise the
//! item itself; with Don't Stop The Music it then never ends. With neither, there is no
//! radio, and the app hides the action.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::commands::{self, Command, InfoKind, Placement, Selection};
use crate::error::ApiError;
use crate::menus::{self, MenuAction, MenuItem, MenuItemKind, MenuPage};
use crate::model::clean_text;
use crate::session::Session;

/// The player preference holding Don't Stop The Music's provider.
pub const DSTM_PREF: &str = "plugin.dontstopthemusic:provider";
/// Most providers read from the settings menu.
const MAX_PROVIDERS: usize = 64;
/// Items read from a context menu when looking for AudioMuse-AI's entries.
const INFO_ITEMS: u32 = 100;

/// What radio or "more like this" starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RadioSeed {
    Track { id: i64 },
    Album { id: i64 },
    Artist { id: i64 },
}

impl RadioSeed {
    fn info_kind(self) -> (InfoKind, i64) {
        match self {
            RadioSeed::Track { id } => (InfoKind::Track, id),
            RadioSeed::Album { id } => (InfoKind::Album, id),
            RadioSeed::Artist { id } => (InfoKind::Artist, id),
        }
    }

    fn selection(self) -> Selection {
        match self {
            RadioSeed::Track { id } => Selection::Track { id },
            RadioSeed::Album { id } => Selection::Album { id },
            RadioSeed::Artist { id } => Selection::Artist { id },
        }
    }
}

/// One of Don't Stop The Music's providers.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DstmProvider {
    /// LMS's token for it, e.g. `PLUGIN_DSTM_SIMILAR` (letters, digits and `_`).
    pub token: String,
    /// What LMS calls it (shown in the app).
    pub name: String,
}

/// Don't Stop The Music for a player: the providers installed and the one in use.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DstmState {
    pub providers: Vec<DstmProvider>,
    /// The provider in use (`None`: off).
    pub current: Option<String>,
}

/// AudioMuse-AI's entries in a context menu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginOffers {
    /// Plays a mix at once (label, action).
    pub mix: Option<(String, MenuAction)>,
    /// Opens a list of similar music (label, action).
    pub similar: Option<(String, MenuAction)>,
    /// A submenu of AudioMuse-AI's own entries, to look inside when nothing above was found.
    pub folder: Option<MenuAction>,
}

/// How "Start radio" works for an item.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RadioPlan {
    /// AudioMuse-AI's mix, run first (it replaces the queue and plays). `None`: the item
    /// itself is played.
    pub mix: Option<MenuAction>,
    /// The mix's own label, e.g. "Instant mix".
    pub mix_label: Option<String>,
    /// Don't Stop The Music's provider, which keeps the queue going.
    pub dstm: Option<DstmProvider>,
    /// Don't Stop The Music must be switched on (to `dstm`) first.
    pub enable_dstm: bool,
}

impl RadioPlan {
    /// The queue keeps going on its own (Don't Stop The Music will top it up).
    pub fn endless(&self) -> bool {
        self.dstm.is_some()
    }
}

/// What the app may offer for an item.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiscoveryOptions {
    /// "Start radio" (`None`: hide it).
    pub radio: Option<RadioPlan>,
    /// "More like this": opens AudioMuse-AI's list of similar music (`None`: hide it).
    pub more_like_this: Option<MenuAction>,
    pub more_like_this_label: Option<String>,
}

/// Is this command word AudioMuse-AI's? (Its LMS plugin's words all start so.)
pub fn is_audiomuse(word: &str) -> bool {
    word.len() >= 9 && word.as_bytes()[..9].eq_ignore_ascii_case(b"audiomuse")
}

fn words(text: &str, list: &[&str]) -> bool {
    let t = text.to_lowercase();
    list.iter().any(|w| t.contains(w))
}

const MIX_WORDS: &[&str] = &["mix", "radio", "instant", "play similar"];
const SIMILAR_WORDS: &[&str] = &["similar", "like this", "sonic", "related", "alike"];

/// The item's AudioMuse-AI action, if it has one (its own `go`, else `play`).
fn audiomuse_action(item: &MenuItem) -> Option<&MenuAction> {
    [&item.go, &item.play]
        .into_iter()
        .flatten()
        .find(|a| a.cmd.first().is_some_and(|w| is_audiomuse(w)))
}

/// Pick AudioMuse-AI's entries out of a context menu. Pure.
///
/// - A **mix** is an entry that runs (it doesn't open a menu), or one that opens a list but
///   can also be played; entries named like a mix come first, then like "similar".
/// - **Similar** is an entry that opens a list; entries named like "similar" come first.
/// - A **folder** is an AudioMuse-AI entry that opens a menu but matched neither (its
///   own submenu, to look inside).
pub fn audiomuse_offers(page: &MenuPage) -> PluginOffers {
    let mut mix: Option<(u8, String, MenuAction)> = None;
    let mut similar: Option<(u8, String, MenuAction)> = None;
    let mut folder = None;
    let rank = |text: &str, first: &[&str], second: &[&str]| {
        if words(text, first) {
            0
        } else if words(text, second) {
            1
        } else {
            2
        }
    };
    let better =
        |cur: &Option<(u8, String, MenuAction)>, r: u8| cur.as_ref().is_none_or(|c| r < c.0);
    for item in &page.items {
        let Some(action) = audiomuse_action(item) else {
            continue;
        };
        if item.kind == MenuItemKind::Search || action.needs_input {
            continue;
        }
        let text = item.text.lines().next().unwrap_or("").trim().to_string();
        if !action.opens_menu {
            let r = rank(&text, MIX_WORDS, SIMILAR_WORDS);
            if better(&mix, r) {
                mix = Some((r, text, action.clone()));
            }
            continue;
        }
        let r = rank(&text, SIMILAR_WORDS, MIX_WORDS);
        if r < 2 {
            if better(&similar, r) {
                similar = Some((r, text.clone(), action.clone()));
            }
            // A list that can be played at once also makes a mix.
            if let Some(play) = item.play.as_ref().filter(|p| !p.opens_menu) {
                let r = 1 + rank(&text, MIX_WORDS, &[]).min(1);
                if better(&mix, r) {
                    mix = Some((r, text, play.clone()));
                }
            }
        } else if folder.is_none() {
            folder = Some(action.clone());
        }
    }
    PluginOffers {
        mix: mix.map(|(_, t, a)| (t, a)),
        similar: similar.map(|(_, t, a)| (t, a)),
        folder,
    }
}

/// A provider token as LMS names them (`PLUGIN_…`): letters, digits and `_`, starting with
/// a letter, at most 64 characters. The gateway applies the same rule.
pub fn provider_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Don't Stop The Music's settings from its menu (`dontstopthemusicsetting`). Pure;
/// untrusted input: only entries whose action sets exactly the provider preference to a
/// well-formed token count. `None` if the answer isn't that menu at all.
pub fn dstm_from(v: &Value) -> Option<DstmState> {
    let items = v.get("item_loop")?.as_array()?;
    let mut providers: Vec<DstmProvider> = Vec::new();
    let mut current = None;
    let mut off_seen = false;
    for item in items.iter().take(MAX_PROVIDERS * 2) {
        let Some(cmd) = item
            .get("actions")
            .and_then(|a| a.get("do").or_else(|| a.get("go")))
            .and_then(|d| d.get("cmd"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let words: Vec<&str> = cmd.iter().filter_map(Value::as_str).collect();
        let token = match (words.as_slice(), cmd.get(2)) {
            (["playerpref", pref, _], _) | (["playerpref", pref], _) if *pref != DSTM_PREF => {
                continue;
            }
            (["playerpref", _, t], _) => t.to_string(),
            (["playerpref", _], Some(Value::Number(n))) => n.to_string(),
            _ => continue,
        };
        let chosen = matches!(item.get("radio"), Some(Value::Number(n)) if n.as_u64() == Some(1))
            || matches!(item.get("radio"), Some(Value::String(s)) if s == "1");
        if token == "0" {
            off_seen = true;
            continue;
        }
        if !provider_ok(&token) || providers.iter().any(|p| p.token == token) {
            continue;
        }
        if providers.len() >= MAX_PROVIDERS {
            break;
        }
        let name = item
            .get("text")
            .and_then(Value::as_str)
            .map(clean_text)
            .map(|t| t.trim().chars().take(80).collect::<String>())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| token.clone());
        if chosen {
            current = Some(token.clone());
        }
        providers.push(DstmProvider { token, name });
    }
    if providers.is_empty() && !off_seen {
        return None;
    }
    Some(DstmState { providers, current })
}

/// The provider radio should use, and whether it must be switched on: the one in use if
/// any (the user's choice stands); otherwise AudioMuse-AI's, then one that finds similar
/// music, then anything but random, then whatever there is.
pub fn choose_provider(s: &DstmState) -> Option<(DstmProvider, bool)> {
    if let Some(cur) = &s.current
        && let Some(p) = s.providers.iter().find(|p| &p.token == cur)
    {
        return Some((p.clone(), false));
    }
    let score = |p: &DstmProvider| {
        let t = format!("{} {}", p.token, p.name).to_lowercase();
        if t.contains("audiomuse") {
            0
        } else if t.contains("similar") || t.contains("mix") || t.contains("like") {
            1
        } else if t.contains("random") {
            3
        } else {
            2
        }
    };
    s.providers
        .iter()
        .min_by_key(|p| score(p))
        .map(|p| (p.clone(), true))
}

/// "Start radio" for an item, from what the server offers (`None`: nothing to offer).
pub fn radio_plan(offers: &PluginOffers, dstm: Option<&DstmState>) -> Option<RadioPlan> {
    let provider = dstm.and_then(choose_provider);
    if offers.mix.is_none() && provider.is_none() {
        return None;
    }
    let (dstm, enable_dstm) = match provider {
        Some((p, enable)) => (Some(p), enable),
        None => (None, false),
    };
    Some(RadioPlan {
        mix: offers.mix.as_ref().map(|(_, a)| a.clone()),
        mix_label: offers.mix.as_ref().map(|(t, _)| t.clone()),
        dstm,
        enable_dstm,
    })
}

/// `dontstopthemusicsetting`: the settings menu for a player.
pub fn dstm_menu() -> Command {
    vec![json!("dontstopthemusicsetting")]
}

/// Switch Don't Stop The Music to `provider` for a player.
pub fn dstm_set(provider: &str) -> Result<Command, ApiError> {
    if !provider_ok(provider) {
        return Err(ApiError::invalid("not a Don't Stop The Music provider"));
    }
    Ok(vec![json!("playerpref"), json!(DSTM_PREF), json!(provider)])
}

/// "Start radio" and "More like this", for a player.
#[derive(uniffi::Object)]
pub struct Discovery {
    session: Arc<Session>,
}

impl Discovery {
    async fn send(&self, player: &str, cmd: Command) -> Result<Value, ApiError> {
        let p = commands::player_id(player)?;
        self.session.core().rpc(&p, cmd).await
    }

    async fn offers(&self, player: &str, seed: RadioSeed) -> Result<PluginOffers, ApiError> {
        let (kind, id) = seed.info_kind();
        let action = menus::info_menu(kind, id)?;
        let cmd = menus::to_command(&action, Some((0, INFO_ITEMS)), None)?;
        let page = menus::page_from(&self.send(player, cmd).await?);
        let mut offers = audiomuse_offers(&page);
        if offers.mix.is_none()
            && offers.similar.is_none()
            && let Some(folder) = offers.folder.clone()
        {
            // AudioMuse-AI's entries may sit one level down, in its own submenu.
            let cmd = menus::to_command(&folder, Some((0, INFO_ITEMS)), None)?;
            if let Ok(v) = self.send(player, cmd).await {
                let inner = audiomuse_offers(&menus::page_from(&v));
                offers.mix = inner.mix;
                offers.similar = inner.similar.or(Some((String::new(), folder)));
            }
        }
        Ok(offers)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Discovery {
    #[uniffi::constructor]
    pub fn new(session: Arc<Session>) -> Arc<Self> {
        Arc::new(Discovery { session })
    }

    /// Don't Stop The Music for `player` (`None`: not available, e.g. refused or absent).
    pub async fn dont_stop_the_music(&self, player: String) -> Option<DstmState> {
        self.send(&player, dstm_menu())
            .await
            .ok()
            .and_then(|v| dstm_from(&v))
    }

    /// What can be offered for an item: radio and "more like this". `dstm` is the
    /// player's Don't Stop The Music state, if already known (else it is asked for).
    pub async fn options(
        &self,
        player: String,
        seed: RadioSeed,
        dstm: Option<DstmState>,
    ) -> Result<DiscoveryOptions, ApiError> {
        let offers = self.offers(&player, seed).await?;
        let dstm = match dstm {
            Some(d) => Some(d),
            None => self.dont_stop_the_music(player.clone()).await,
        };
        let (label, similar) = match offers.similar.clone() {
            Some((t, a)) => (Some(t).filter(|t| !t.is_empty()), Some(a)),
            None => (None, None),
        };
        Ok(DiscoveryOptions {
            radio: radio_plan(&offers, dstm.as_ref()),
            more_like_this: similar,
            more_like_this_label: label,
        })
    }

    /// Start radio from `seed` as `plan` says: switch Don't Stop The Music on if needed,
    /// then run the mix, or play the item.
    pub async fn start_radio(
        &self,
        player: String,
        seed: RadioSeed,
        plan: RadioPlan,
    ) -> Result<(), ApiError> {
        if let (Some(p), true) = (&plan.dstm, plan.enable_dstm) {
            self.send(&player, dstm_set(&p.token)?).await?;
        }
        match &plan.mix {
            Some(mix) => {
                if mix.opens_menu || mix.needs_input {
                    return Err(ApiError::invalid("not a mix"));
                }
                let cmd = menus::to_command(mix, None, None)?;
                self.send(&player, cmd).await?;
            }
            None => {
                let cmd = commands::enqueue(seed.selection(), Placement::PlayNow, None)?;
                self.send(&player, cmd).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::page_from;

    fn ctx(items: Value) -> MenuPage {
        page_from(&json!({"count": 10, "item_loop": items}))
    }

    #[test]
    fn finds_audiomuse_mixes_and_similar_lists() {
        let page = ctx(json!([
            {"text": "Play next", "actions": {"go": {"cmd": ["playlistcontrol"], "params": {"cmd": "insert", "track_id": 9}}, "nextWindow": "parent"}},
            {"text": "Similar tracks", "actions": {"go": {"cmd": ["audiomuseai", "similar"], "params": {"track_id": 9}}}},
            {"text": "Instant mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["audiomuseai", "mix"], "params": {"track_id": 9}}}},
            {"text": "Some other plugin mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["otherplugin", "mix"], "params": {"track_id": 9}}}}
        ]));
        let o = audiomuse_offers(&page);
        let (label, mix) = o.mix.unwrap();
        assert_eq!(label, "Instant mix");
        assert_eq!(mix.cmd, ["audiomuseai", "mix"]);
        assert!(!mix.opens_menu);
        let (label, similar) = o.similar.unwrap();
        assert_eq!(label, "Similar tracks");
        assert!(similar.opens_menu);
        assert_eq!(similar.cmd, ["audiomuseai", "similar"]);
    }

    #[test]
    fn nothing_from_other_plugins_or_core_entries() {
        let page = ctx(json!([
            {"text": "Similar artists (Last.fm)", "actions": {"go": {"cmd": ["lastfm", "similar"], "params": {"artist_id": 3}}}},
            {"text": "Mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["randomplay", "tracks"]}}},
            {"text": "Search AudioMuse", "actions": {"go": {"cmd": ["audiomuseai", "search"], "params": {"search": "__TAGGEDINPUT__"}}}}
        ]));
        assert_eq!(audiomuse_offers(&page), PluginOffers::default());
        assert_eq!(
            radio_plan(&PluginOffers::default(), None),
            None,
            "no radio: hidden"
        );
    }

    #[test]
    fn a_playable_similar_list_makes_a_mix_and_a_folder_is_looked_into() {
        let page = ctx(json!([
            {"text": "Sonically similar", "type": "playlist",
             "actions": {
                "go": {"cmd": ["audiomuseai", "list"], "params": {"album_id": 4}},
                "play": {"cmd": ["audiomuseai", "playlist"], "params": {"album_id": 4, "cmd": "play"}}}}
        ]));
        let o = audiomuse_offers(&page);
        assert_eq!(o.similar.as_ref().unwrap().1.cmd, ["audiomuseai", "list"]);
        assert_eq!(o.mix.as_ref().unwrap().1.cmd, ["audiomuseai", "playlist"]);
        let folder = ctx(json!([
            {"text": "AudioMuse-AI", "actions": {"go": {"cmd": ["audiomuseai", "items"], "params": {"track_id": 1}}}}
        ]));
        let o = audiomuse_offers(&folder);
        assert!(o.mix.is_none() && o.similar.is_none());
        assert_eq!(o.folder.unwrap().cmd, ["audiomuseai", "items"]);
    }

    #[test]
    fn actions_carrying_urls_are_never_offered() {
        // The menu parser refuses the action; nothing AudioMuse-like is left to offer.
        let page = ctx(json!([
            {"text": "Instant mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["audiomuseai", "mix"], "params": {"url": "file:///srv/music/a.flac"}}}}
        ]));
        assert_eq!(audiomuse_offers(&page), PluginOffers::default());
    }

    fn dstm_answer(current: &str) -> Value {
        let item = |text: &str, token: Value| {
            json!({"text": text, "radio": if token == json!(current) { 1 } else { 0 },
                   "actions": {"do": {"player": 0, "cmd": ["playerpref", DSTM_PREF, token]}}})
        };
        json!({"count": 4, "item_loop": [
            item("Disabled", json!(0)),
            item("Random songs", json!("PLUGIN_RANDOM_TRACK")),
            item("AudioMuse-AI similar tracks", json!("PLUGIN_AUDIOMUSEAI_DSTM")),
            item("Similar tracks (Last.fm)", json!("PLUGIN_DSTM_SIMILARITY")),
        ]})
    }

    #[test]
    fn reads_dont_stop_the_music() {
        let off = dstm_from(&dstm_answer("0")).unwrap();
        assert_eq!(off.current, None);
        assert_eq!(off.providers.len(), 3);
        assert_eq!(off.providers[1].name, "AudioMuse-AI similar tracks");
        // Off: AudioMuse-AI's provider is chosen, and must be switched on.
        let (p, enable) = choose_provider(&off).unwrap();
        assert_eq!(
            (p.token.as_str(), enable),
            ("PLUGIN_AUDIOMUSEAI_DSTM", true)
        );
        // The user's own choice stands.
        let on = dstm_from(&dstm_answer("PLUGIN_RANDOM_TRACK")).unwrap();
        assert_eq!(on.current.as_deref(), Some("PLUGIN_RANDOM_TRACK"));
        let (p, enable) = choose_provider(&on).unwrap();
        assert_eq!((p.token.as_str(), enable), ("PLUGIN_RANDOM_TRACK", false));
        // Radio from DSTM alone plays the item and keeps going.
        let plan = radio_plan(&PluginOffers::default(), Some(&off)).unwrap();
        assert!(plan.mix.is_none() && plan.endless() && plan.enable_dstm);
        // Without AudioMuse-AI, a similarity provider beats random.
        let mut s = off.clone();
        s.providers.retain(|p| !p.token.contains("AUDIOMUSE"));
        assert_eq!(
            choose_provider(&s).unwrap().0.token,
            "PLUGIN_DSTM_SIMILARITY"
        );
        // Only "Disabled": DSTM exists but has nothing to offer.
        let empty = dstm_from(&json!({"item_loop": [
            {"text": "Disabled", "radio": 1, "actions": {"do": {"cmd": ["playerpref", DSTM_PREF, 0]}}}
        ]}))
        .unwrap();
        assert!(empty.providers.is_empty());
        assert_eq!(choose_provider(&empty), None);
        assert_eq!(radio_plan(&PluginOffers::default(), Some(&empty)), None);
    }

    #[test]
    fn hostile_dstm_answers_are_refused() {
        for bad in [
            json!({}),
            json!({"item_loop": "x"}),
            json!([1, 2]),
            json!({"item_loop": [{"actions": {"do": {"cmd": ["pref", "x", "y"]}}}]}),
            json!({"item_loop": [{"actions": {"do": {"cmd": ["playerpref", "plugin.other:pref", "X"]}}}]}),
        ] {
            assert_eq!(dstm_from(&bad), None, "{bad}");
        }
        // Malformed tokens are dropped, never passed on.
        for token in [
            json!("PLUGIN X"),
            json!("http://example.com/x"),
            json!("/srv/music"),
            json!("_LEADING"),
            json!("A".repeat(65)),
            json!("PLUGIN;rescan"),
            json!(""),
            json!(1.5),
        ] {
            let v = json!({"item_loop": [
                {"text": "x", "radio": 1, "actions": {"do": {"cmd": ["playerpref", DSTM_PREF, token]}}}
            ]});
            assert_eq!(dstm_from(&v), None, "{token}");
        }
        // Text is cleaned and bounded.
        let v = json!({"item_loop": [
            {"text": format!("Nice\u{0007}{}", "x".repeat(500)), "actions": {"do": {"cmd": ["playerpref", DSTM_PREF, "PLUGIN_A"]}}}
        ]});
        let s = dstm_from(&v).unwrap();
        assert!(s.providers[0].name.len() <= 80);
        assert!(!s.providers[0].name.chars().any(char::is_control));
    }

    #[test]
    fn dstm_answers_survive_arbitrary_json() {
        let parts = [
            json!(null),
            json!(1),
            json!("PLUGIN_A"),
            json!("playerpref"),
            json!(DSTM_PREF),
            json!([]),
            json!({}),
            json!(1e300),
            json!(-1),
        ];
        let mut seed = 0x51ed_270b_27b0_3b1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..5_000 {
            let mut pick = || parts[(next() % parts.len() as u64) as usize].clone();
            let v = json!({"item_loop": [
                {"text": pick(), "radio": pick(), "actions": {"do": {"cmd": [pick(), pick(), pick()]}}},
                {"actions": pick()},
                pick(),
            ]});
            if let Some(s) = dstm_from(&v) {
                assert!(s.providers.iter().all(|p| provider_ok(&p.token)));
            }
        }
    }

    #[test]
    fn dstm_commands_are_strict() {
        assert_eq!(dstm_menu(), vec![json!("dontstopthemusicsetting")]);
        assert_eq!(
            dstm_set("PLUGIN_AUDIOMUSEAI_DSTM").unwrap(),
            vec![
                json!("playerpref"),
                json!(DSTM_PREF),
                json!("PLUGIN_AUDIOMUSEAI_DSTM")
            ]
        );
        for bad in ["", "0", "a b", "x:y", "/srv", "PLUGIN_A\n", "é"] {
            assert!(dstm_set(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn audiomuse_is_recognised_by_its_command_word_only() {
        assert!(is_audiomuse("audiomuseai"));
        assert!(is_audiomuse("AudioMuseAI"));
        assert!(is_audiomuse("audiomuse"));
        assert!(!is_audiomuse("audio"));
        assert!(!is_audiomuse("myaudiomuse"));
        assert!(!is_audiomuse("éééééééééé"), "never panics on other text");
    }

    #[test]
    fn radio_from_a_mix_keeps_going_with_dstm() {
        let page = ctx(json!([
            {"text": "Instant mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["audiomuseai", "mix"], "params": {"track_id": 9}}}}
        ]));
        let offers = audiomuse_offers(&page);
        let plan = radio_plan(&offers, None).unwrap();
        assert_eq!(plan.mix_label.as_deref(), Some("Instant mix"));
        assert!(!plan.endless());
        let dstm = dstm_from(&dstm_answer("PLUGIN_DSTM_SIMILARITY")).unwrap();
        let plan = radio_plan(&offers, Some(&dstm)).unwrap();
        assert!(plan.endless() && !plan.enable_dstm);
    }
}
