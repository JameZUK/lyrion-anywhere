//! What a remote user may ask LMS to do: the JSON-RPC allow-list.
//!
//! Default deny. A request is `{player, cmd}`, with `cmd` LMS's command array (for example
//! `["albums", 0, 50, "tags:al"]`). It is allowed only if:
//! - the command (and sub-command) is in the table for the caller's role;
//! - every positional argument has the expected form;
//! - every `key:value` parameter uses a key allowed for that command, with a well-formed
//!   value;
//! - for player commands, the caller may control the player (and, for `sync`, the other
//!   player too).
//!
//! Responses are filtered too ([`filter_response`]): LAN addresses, server identifiers and
//! URLs (which include file paths and stream credentials) never leave the gateway.

use std::collections::HashSet;

use serde_json::Value;

use crate::accounts::Role;

/// Largest page a client may request.
pub const MAX_PAGE: u64 = 500;
const MAX_ARGS: usize = 16;
const MAX_ARG_LEN: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied(pub String);

fn deny<T>(why: impl Into<String>) -> Result<T, Denied> {
    Err(Denied(why.into()))
}

/// What the caller is allowed to do, as the policy needs it.
pub trait Caller {
    fn role(&self) -> Role;
    fn may_control(&self, player: &str) -> bool;
}

/// A validated request, ready to send to LMS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// Player MAC, or empty for server queries.
    pub player: String,
    pub cmd: Vec<String>,
    /// How to filter the response.
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Ordinary response; strip sensitive fields.
    Plain,
    /// `players` / `serverstatus`: also drop players the caller may not control.
    PlayerList,
    /// `apps` / `radios`: also learn the app commands listed.
    AppMenu,
    /// `menu`: the player's home menu, cut down to library and plugin entries.
    HomeMenu,
}

/// Normalise one argument: strings and integers only, bounded, printable.
fn arg(v: &Value) -> Result<String, Denied> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.is_finite() && (0.0..1e7).contains(&f) => n.to_string(),
            _ => return deny("bad number"),
        },
        _ => return deny("arguments must be strings or numbers"),
    };
    if s.len() > MAX_ARG_LEN {
        return deny("argument too long");
    }
    if s.chars().any(|c| c.is_control()) {
        return deny("control characters not allowed");
    }
    Ok(s)
}

fn is_uint(s: &str) -> bool {
    !s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_ids(s: &str) -> bool {
    // A single id or a comma-separated list of ids.
    !s.is_empty() && s.len() <= 200 && s.split(',').all(is_uint)
}

fn is_signed_num(s: &str) -> bool {
    let t = s.strip_prefix(['+', '-']).unwrap_or(s);
    !t.is_empty()
        && t.len() <= 10
        && t.chars().all(|c| c.is_ascii_digit() || c == '.')
        && t.matches('.').count() <= 1
}

/// LMS player ids are MAC addresses.
pub fn is_player_id(s: &str) -> bool {
    s.len() == 17
        && s.split(':').count() == 6
        && s.split(':')
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Validation for a `key:value` parameter's value.
#[derive(Clone, Copy)]
enum V {
    Ids,
    Uint,
    Bool,
    /// Free text (search terms, names), already bounded and printable.
    Text,
    /// LMS response tags: letters only.
    Tags,
    /// Sort orders: a fixed vocabulary.
    Sort,
    /// Opaque menu/item ids from LMS (e.g. `b99447ab.1`).
    ItemId,
    /// One of a fixed set of words.
    OneOf(&'static [&'static str]),
}

fn value_ok(v: V, s: &str) -> bool {
    match v {
        V::Ids => is_ids(s),
        V::Uint => is_uint(s),
        V::Bool => s == "0" || s == "1",
        V::Text => !s.is_empty(),
        V::Tags => !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric()),
        V::Sort => matches!(
            s,
            "album"
                | "artflow"
                | "artistalbum"
                | "yearalbum"
                | "yearartistalbum"
                | "new"
                | "random"
                | "title"
                | "tracknum"
                | "albumtrack"
                | "artisttrack"
                | "id"
        ),
        V::ItemId => {
            s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }
        V::OneOf(set) => set.contains(&s),
    }
}

/// Check the `key:value` parameters after the positional ones.
fn params(rest: &[String], allowed: &[(&str, V)]) -> Result<(), Denied> {
    let mut seen = HashSet::new();
    for p in rest {
        let Some((k, v)) = p.split_once(':') else {
            return deny(format!("unexpected argument {p:?}"));
        };
        let Some((_, rule)) = allowed.iter().find(|(name, _)| *name == k) else {
            return deny(format!("parameter {k:?} not allowed here"));
        };
        if !seen.insert(k) {
            return deny(format!("parameter {k:?} repeated"));
        }
        if !value_ok(*rule, v) {
            return deny(format!("bad value for {k}"));
        }
    }
    Ok(())
}

/// `start count` paging, bounded.
fn paging(a: &[String]) -> Result<(), Denied> {
    match a {
        [start, count, ..] if is_uint(start) && is_uint(count) => {
            if count.parse::<u64>().unwrap_or(u64::MAX) > MAX_PAGE {
                deny(format!("count over {MAX_PAGE}"))
            } else {
                Ok(())
            }
        }
        _ => deny("expected start and count"),
    }
}

const LIBRARY: &[(&str, V)] = &[("library_id", V::ItemId)];

fn with<'a>(base: &'a [(&'a str, V)], extra: &'a [(&'a str, V)]) -> Vec<(&'a str, V)> {
    base.iter().chain(extra).copied().collect()
}

/// Check a request against the policy.
pub fn check(
    caller: &impl Caller,
    player: &str,
    raw: &[Value],
    apps: &HashSet<String>,
) -> Result<Checked, Denied> {
    if raw.is_empty() || raw.len() > MAX_ARGS {
        return deny("bad command length");
    }
    let cmd: Vec<String> = raw.iter().map(arg).collect::<Result<_, _>>()?;
    let c: Vec<&str> = cmd.iter().map(String::as_str).collect();
    let server = |kind| {
        Ok(Checked {
            player: String::new(),
            cmd: cmd.clone(),
            kind,
        })
    };

    // --- Server queries (no player) -----------------------------------------
    let query = match c[0] {
        "serverstatus" | "players" => {
            paging(&cmd[1..])?;
            params(&cmd[3..], &[])?;
            Some(Kind::PlayerList)
        }
        "artists" => {
            paging(&cmd[1..])?;
            params(
                &cmd[3..],
                &with(
                    LIBRARY,
                    &[
                        ("search", V::Text),
                        ("genre_id", V::Ids),
                        ("album_id", V::Ids),
                        ("year", V::Uint),
                        ("role_id", V::Text),
                        ("tags", V::Tags),
                    ],
                ),
            )?;
            Some(Kind::Plain)
        }
        "albums" => {
            paging(&cmd[1..])?;
            params(
                &cmd[3..],
                &with(
                    LIBRARY,
                    &[
                        ("search", V::Text),
                        ("genre_id", V::Ids),
                        ("artist_id", V::Ids),
                        ("album_id", V::Ids),
                        ("track_id", V::Ids),
                        ("year", V::Uint),
                        ("sort", V::Sort),
                        ("tags", V::Tags),
                        ("role_id", V::Text),
                        ("compilation", V::Bool),
                    ],
                ),
            )?;
            Some(Kind::Plain)
        }
        "titles" | "tracks" => {
            paging(&cmd[1..])?;
            params(
                &cmd[3..],
                &with(
                    LIBRARY,
                    &[
                        ("search", V::Text),
                        ("genre_id", V::Ids),
                        ("artist_id", V::Ids),
                        ("album_id", V::Ids),
                        ("year", V::Uint),
                        ("sort", V::Sort),
                        ("tags", V::Tags),
                    ],
                ),
            )?;
            Some(Kind::Plain)
        }
        "genres" => {
            paging(&cmd[1..])?;
            params(
                &cmd[3..],
                &with(
                    LIBRARY,
                    &[
                        ("search", V::Text),
                        ("artist_id", V::Ids),
                        ("album_id", V::Ids),
                        ("year", V::Uint),
                        ("tags", V::Tags),
                    ],
                ),
            )?;
            Some(Kind::Plain)
        }
        "years" => {
            paging(&cmd[1..])?;
            params(&cmd[3..], &with(LIBRARY, &[("hasAlbums", V::Bool)]))?;
            Some(Kind::Plain)
        }
        "search" => {
            paging(&cmd[1..])?;
            params(&cmd[3..], &[("term", V::Text), ("extended", V::Bool)])?;
            Some(Kind::Plain)
        }
        "musicfolder" => {
            paging(&cmd[1..])?;
            params(
                &cmd[3..],
                &[
                    ("folder_id", V::Ids),
                    ("tags", V::Tags),
                    ("return_top", V::Bool),
                ],
            )?;
            Some(Kind::Plain)
        }
        "apps" | "radios" => {
            paging(&cmd[1..])?;
            params(&cmd[3..], &[])?;
            Some(Kind::AppMenu)
        }
        "playlists" if c.get(1) == Some(&"tracks") => {
            paging(&cmd[2..])?;
            params(&cmd[4..], &[("playlist_id", V::Ids), ("tags", V::Tags)])?;
            Some(Kind::Plain)
        }
        "playlists" if c.get(1).is_some_and(|s| is_uint(s)) => {
            paging(&cmd[1..])?;
            params(&cmd[3..], &[("search", V::Text), ("tags", V::Tags)])?;
            Some(Kind::Plain)
        }
        "playlists" if matches!(c.get(1), Some(&"new" | &"rename" | &"delete" | &"edit")) => {
            // Saved playlists are shared household data: listeners may play them, not change them.
            if caller.role() != Role::Admin {
                return deny("changing playlists needs the admin role");
            }
            playlists_edit(&cmd[1..])?;
            Some(Kind::Plain)
        }
        "favorites" if c.get(1) == Some(&"items") => {
            paging(&cmd[2..])?;
            params(
                &cmd[4..],
                &[("item_id", V::ItemId), ("menu", V::OneOf(&["1"]))],
            )?;
            Some(Kind::Plain)
        }
        "syncgroups" if c.get(1) == Some(&"?") && cmd.len() == 2 => Some(Kind::Plain),
        "rescan" => {
            if caller.role() != Role::Admin {
                return deny("rescan needs the admin role");
            }
            if !(cmd.len() == 1 || (cmd.len() == 2 && matches!(c[1], "?" | "full"))) {
                return deny("bad rescan arguments");
            }
            Some(Kind::Plain)
        }
        _ => None,
    };
    if let Some(kind) = query {
        if !player.is_empty() {
            return deny("server queries take no player");
        }
        return server(kind);
    }

    // --- Player commands ----------------------------------------------------
    if !is_player_id(player) {
        return deny(format!("command {:?} not allowed", c[0]));
    }
    if !caller.may_control(player) {
        return deny("you may not control this player");
    }
    let n = cmd.len();
    let ok = match c[0] {
        "status" => {
            paging(&cmd[1..]).is_ok()
                && params(
                    &cmd[3..],
                    &[("tags", V::Tags), ("menu", V::OneOf(&["menu"]))],
                )
                .is_ok()
                || (n == 2 && c[1] == "-")
                || (n >= 3
                    && c[1] == "-"
                    && is_uint(c[2])
                    && params(&cmd[3..], &[("tags", V::Tags)]).is_ok())
        }
        "play" | "stop" => n == 1,
        "pause" => n == 1 || (n == 2 && matches!(c[1], "0" | "1")),
        "mode" => n == 2 && c[1] == "?",
        "time" => n == 2 && (c[1] == "?" || is_signed_num(c[1])),
        "power" => n == 2 && matches!(c[1], "0" | "1" | "?"),
        "sleep" => n == 2 && (c[1] == "?" || is_uint(c[1])),
        "mixer" => {
            n == 3
                && match c[1] {
                    "volume" => c[2] == "?" || is_signed_num(c[2]),
                    "muting" => matches!(c[2], "0" | "1" | "toggle" | "?"),
                    _ => false,
                }
        }
        "playlist" => match (c.get(1).copied(), n) {
            (Some("index" | "jump"), 3) => c[2] == "?" || is_signed_num(c[2]),
            (Some("move"), 4) => is_uint(c[2]) && is_uint(c[3]),
            (Some("delete"), 3) => is_uint(c[2]),
            (Some("clear"), 2) => true,
            (Some("shuffle" | "repeat"), 3) => matches!(c[2], "0" | "1" | "2" | "?"),
            (Some("tracks"), 3) => c[2] == "?",
            // Anything that takes a URL or file path (play, add, insert, load, …) is refused.
            _ => false,
        },
        "playlistcontrol" => playlistcontrol(&cmd[1..]),
        // The home menu, where plugins add entries (My Music, Radio, Extras): read-only, and
        // filtered (see `home_menu`). Its actions are then learned like any other menu's.
        "menu" => {
            paging(&cmd[1..]).is_ok() && params(&cmd[3..], &[("direct", V::OneOf(&["1"]))]).is_ok()
        }
        // Context menus ("More…") for library items; their actions are then learned.
        "trackinfo" | "albuminfo" | "artistinfo" | "genreinfo" | "yearinfo" | "playlistinfo"
        | "folderinfo"
            if c.get(1) == Some(&"items") =>
        {
            paging(&cmd[2..]).is_ok()
                && params(
                    &cmd[4..],
                    &[
                        ("track_id", V::Ids),
                        ("album_id", V::Ids),
                        ("artist_id", V::Ids),
                        ("genre_id", V::Ids),
                        ("year", V::Uint),
                        ("playlist_id", V::Ids),
                        ("folder_id", V::Ids),
                        ("menu", V::OneOf(&["1"])),
                    ],
                )
                .is_ok()
        }
        "sync" => {
            n == 2
                && (matches!(c[1], "-" | "?")
                    || (is_player_id(c[1]) && c[1] != player && caller.may_control(c[1])))
        }
        "favorites" if c.get(1) == Some(&"playlist") => {
            n >= 4
                && matches!(c[2], "play" | "add" | "insert")
                && params(&cmd[3..], &[("item_id", V::ItemId)]).is_ok()
                && cmd[3..].iter().any(|p| p.starts_with("item_id:"))
        }
        app if apps.contains(app) => match c.get(1).copied() {
            Some("items") => {
                paging(&cmd[2..]).is_ok()
                    && params(
                        &cmd[4..],
                        &[
                            ("item_id", V::ItemId),
                            ("search", V::Text),
                            ("menu", V::OneOf(&["1"])),
                        ],
                    )
                    .is_ok()
            }
            Some("playlist") => {
                n >= 4
                    && matches!(c[2], "play" | "add" | "insert")
                    && params(&cmd[3..], &[("item_id", V::ItemId)]).is_ok()
                    && cmd[3..].iter().any(|p| p.starts_with("item_id:"))
            }
            _ => false,
        },
        _ => false,
    };
    if !ok {
        return deny(format!(
            "command {:?} not allowed with these arguments",
            c[0]
        ));
    }
    let kind = if c[0] == "menu" {
        Kind::HomeMenu
    } else {
        Kind::Plain
    };
    Ok(Checked {
        player: player.to_string(),
        cmd,
        kind,
    })
}

/// `playlistcontrol cmd:<load|add|insert|delete> <exactly one id selector> [play_index:n]`.
fn playlistcontrol(a: &[String]) -> bool {
    let selectors = [
        "album_id",
        "artist_id",
        "track_id",
        "genre_id",
        "year",
        "playlist_id",
        "folder_id",
    ];
    let mut cmd_ok = false;
    let mut selector_count = 0;
    let mut seen = HashSet::new();
    for p in a {
        let Some((k, v)) = p.split_once(':') else {
            return false;
        };
        if !seen.insert(k) {
            return false;
        }
        match k {
            "cmd" => cmd_ok = matches!(v, "load" | "add" | "insert" | "delete"),
            "play_index" => {
                if !is_uint(v) {
                    return false;
                }
            }
            k if selectors.contains(&k) => {
                if !is_ids(v) {
                    return false;
                }
                selector_count += 1;
            }
            // url:, playlist_name:, and anything else are refused.
            _ => return false,
        }
    }
    cmd_ok && selector_count == 1
}

/// `playlists new|rename|delete|edit` with ids and names only.
fn playlists_edit(a: &[String]) -> Result<(), Denied> {
    let rules: &[(&str, V)] = match a[0].as_str() {
        "new" => &[("name", V::Text)],
        "rename" => &[
            ("playlist_id", V::Ids),
            ("newname", V::Text),
            ("dry_run", V::Bool),
        ],
        "delete" => &[("playlist_id", V::Ids)],
        "edit" => &[
            ("playlist_id", V::Ids),
            ("cmd", V::OneOf(&["up", "down", "move", "delete", "add"])),
            ("index", V::Uint),
            ("toindex", V::Uint),
            ("track_id", V::Ids),
        ],
        _ => return deny("bad playlists command"),
    };
    params(&a[1..], rules)?;
    let has = |k: &str| a[1..].iter().any(|p| p.starts_with(&format!("{k}:")));
    let required: &[&str] = match a[0].as_str() {
        "new" => &["name"],
        "rename" => &["playlist_id", "newname"],
        "delete" | "edit" => &["playlist_id"],
        _ => &[],
    };
    if required.iter().all(|k| has(k)) {
        Ok(())
    } else {
        deny("missing required parameter")
    }
}

/// Keys never returned to clients: LAN addresses, server identity, and URLs (which carry
/// file paths and streaming-service credentials).
const STRIPPED: &[&str] = &[
    "ip",
    "uuid",
    "httpport",
    "url",
    "mac",
    "seq_no",
    "remote_url",
    "favorites_url",
    "path",
];

/// A value that reveals the server's filesystem or home network, whatever key it is under:
/// `file:` URLs, and URLs whose host is private, loopback, link-local or a local name.
pub(crate) fn sensitive_text(s: &str) -> bool {
    sensitive_value(s)
}

fn sensitive_value(s: &str) -> bool {
    let lower = s.trim_start().to_ascii_lowercase();
    if lower.starts_with("file:") {
        return true;
    }
    let Some((scheme, rest)) = lower.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https" | "ws" | "wss" | "rtsp" | "mms") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    local_host(host)
}

fn local_host(host: &str) -> bool {
    use std::net::IpAddr;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                let seg0 = v6.segments()[0];
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (seg0 & 0xfe00) == 0xfc00 // unique local (fc00::/7)
                    || (seg0 & 0xffc0) == 0xfe80 // link local
                    || v6.to_ipv4_mapped().is_some_and(|v4| local_host(&v4.to_string()))
            }
        };
    }
    host == "localhost"
        || [
            ".local",
            ".lan",
            ".home",
            ".internal",
            ".localdomain",
            ".home.arpa",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

/// Filter a response. `may_control` hides players the caller can't use.
pub fn filter_response(kind: Kind, mut v: Value, may_control: &dyn Fn(&str) -> bool) -> Value {
    strip(&mut v);
    if kind == Kind::HomeMenu {
        home_menu(&mut v);
    }
    if kind == Kind::PlayerList
        && let Some(players) = v.get_mut("players_loop").and_then(Value::as_array_mut)
    {
        players.retain(|p| {
            p.get("playerid")
                .and_then(Value::as_str)
                .is_some_and(may_control)
        });
        let count = players.len();
        if let Some(obj) = v.as_object_mut() {
            for k in ["player count", "count"] {
                if obj.contains_key(k) {
                    obj.insert(k.to_string(), Value::from(count));
                }
            }
            // Players on other servers or the network are none of a remote user's business.
            obj.retain(|k, _| !k.starts_with("other player") && !k.starts_with("sn player"));
        }
    }
    v
}

/// Home-menu nodes whose entries are kept: the top level, My Music, Radio, Extras and apps.
const HOME_ROOTS: &[&str] = &["home", "", "myMusic", "radios", "extras", "plugins"];

/// Home-menu entries never relayed: player and server settings, power, and entries the app
/// has its own pages for (the library, favourites, search).
fn home_entry_refused(id: &str, node: &str) -> bool {
    let refused_prefix = |s: &str| {
        ["settings", "advancedSettings", "myMusic", "playerpower"]
            .iter()
            .any(|p| s.starts_with(p))
    };
    refused_prefix(id)
        || matches!(id, "globalSearch" | "favorites" | "home")
        || node.starts_with("settings")
        || node.starts_with("advancedSettings")
}

/// Cut the home menu (`menu`) down to library and plugin entries: an entry is kept if it is
/// not refused and sits under a root node or under another kept entry (e.g. Random Mix's
/// items). Everything else, settings above all, is dropped before the device sees it, so
/// its actions are never learned.
fn home_menu(v: &mut Value) {
    let Some(items) = v.get_mut("item_loop").and_then(Value::as_array_mut) else {
        return;
    };
    let field = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    items.retain(|x| x.is_object() && !home_entry_refused(&field(x, "id"), &field(x, "node")));
    let mut kept: HashSet<String> = HOME_ROOTS.iter().map(|s| s.to_string()).collect();
    let mut keep = vec![false; items.len()];
    loop {
        let mut changed = false;
        for (i, x) in items.iter().enumerate() {
            let id = field(x, "id");
            if !keep[i] && kept.contains(&field(x, "node")) {
                keep[i] = true;
                changed |= !id.is_empty() && kept.insert(id);
            }
        }
        if !changed {
            break;
        }
    }
    let mut i = 0;
    items.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    let count = items.len();
    if let Some(o) = v.as_object_mut() {
        o.insert("count".into(), Value::from(count));
    }
}

/// A menu action (`{"cmd": [...], "params": {...}}`) whose parameters carry a URL, path or
/// LAN address. It is removed whole: without them it would be a broken command.
fn poisoned_action(x: &Value) -> bool {
    let Some(o) = x.as_object() else { return false };
    if !o.contains_key("cmd") {
        return false;
    }
    o.get("params").and_then(Value::as_object).is_some_and(|p| {
        p.iter()
            .any(|(k, v)| STRIPPED.contains(&k.as_str()) || v.as_str().is_some_and(sensitive_value))
    })
}

fn strip(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.retain(|k, x| {
                !STRIPPED.contains(&k.as_str())
                    && !x.as_str().is_some_and(sensitive_value)
                    && !poisoned_action(x)
            });
            for x in map.values_mut() {
                strip(x);
            }
        }
        Value::Array(items) => {
            for x in items.iter_mut() {
                if x.as_str().is_some_and(sensitive_value) {
                    *x = Value::String(String::new());
                } else {
                    strip(x);
                }
            }
        }
        _ => {}
    }
}

/// App commands listed in an `apps`/`radios` response (`appss_loop`/`radioss_loop`).
pub fn learn_apps(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["appss_loop", "radioss_loop"] {
        if let Some(items) = v.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(cmd) = item.get("cmd").and_then(Value::as_str)
                    && !cmd.is_empty()
                    && cmd.len() <= 64
                    && cmd
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    && !RESERVED.contains(&cmd)
                {
                    out.push(cmd.to_string());
                }
            }
        }
    }
    out
}

/// Core LMS commands that must never be treated as an app, whatever a menu says.
const RESERVED: &[&str] = &[
    "readdirectory",
    "pref",
    "playerpref",
    "rescan",
    "wipecache",
    "restartserver",
    "stopserver",
    "debug",
    "logging",
    "getstring",
    "playlist",
    "playlists",
    "playlistcontrol",
    "status",
    "mixer",
    "power",
    "sync",
    "sleep",
    "button",
    "ir",
    "display",
    "show",
    "client",
    "connect",
    "disconnect",
    "login",
    "listen",
    "subscribe",
    "serverstatus",
    "players",
    "favorites",
    "artwork",
    "setsncredentials",
    // The LMS plugin's own management commands: never callable remotely.
    "anywhere",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINE: &str = "02:4c:41:00:00:01";
    const OTHER: &str = "02:4c:41:00:00:02";
    const HOME: &str = "aa:bb:cc:dd:ee:01";

    struct Who(Role);
    impl Caller for Who {
        fn role(&self) -> Role {
            self.0
        }
        fn may_control(&self, p: &str) -> bool {
            self.0 == Role::Admin || p == MINE || p == HOME
        }
    }

    fn apps() -> HashSet<String> {
        ["spotty", "bbcsounds"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn ok(player: &str, cmd: Value) -> Checked {
        let raw = cmd.as_array().unwrap().clone();
        check(&Who(Role::Listener), player, &raw, &apps())
            .unwrap_or_else(|e| panic!("{cmd} denied: {e:?}"))
    }

    fn admin_ok(cmd: Value) {
        let raw = cmd.as_array().unwrap().clone();
        check(&Who(Role::Admin), "", &raw, &apps())
            .unwrap_or_else(|e| panic!("{cmd} denied: {e:?}"));
    }

    /// Refused even for an admin: the arguments themselves are invalid.
    fn admin_denied(cmd: Value) {
        let raw = cmd.as_array().unwrap().clone();
        assert!(
            check(&Who(Role::Admin), "", &raw, &apps()).is_err(),
            "{cmd} was allowed"
        );
    }

    fn denied(player: &str, cmd: Value) {
        let raw = cmd.as_array().unwrap().clone();
        assert!(
            check(&Who(Role::Listener), player, &raw, &apps()).is_err(),
            "{player} {cmd} was allowed"
        );
    }

    /// Every command the web app can build (web/app/tests/fixtures/commands.json, kept in
    /// step with the builders by a Vitest test) must pass the policy as a listener.
    #[test]
    fn web_app_commands_are_allowed() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../web/app/tests/fixtures/commands.json"))
                .unwrap();
        let examples = fixture.as_array().unwrap();
        assert!(examples.len() >= 30, "fixture looks truncated");
        for e in examples {
            let player = if e["player"].as_bool().unwrap() {
                MINE
            } else {
                ""
            };
            let raw = e["cmd"].as_array().unwrap().clone();
            if e["admin"].as_bool() == Some(true) {
                // Admin-only in the app too: refused for listeners, allowed for admins.
                assert!(
                    check(&Who(Role::Listener), player, &raw, &apps()).is_err(),
                    "{e}"
                );
                check(&Who(Role::Admin), player, &raw, &apps())
                    .unwrap_or_else(|err| panic!("{e} denied for admin: {err:?}"));
            } else {
                ok(player, e["cmd"].clone());
            }
        }
    }

    #[test]
    fn only_admins_change_saved_playlists() {
        let changes = [
            json!(["playlists", "new", "name:Road trip"]),
            json!(["playlists", "rename", "playlist_id:5", "newname:X"]),
            json!(["playlists", "delete", "playlist_id:5"]),
            json!([
                "playlists",
                "edit",
                "playlist_id:5",
                "cmd:delete",
                "index:0"
            ]),
            json!([
                "playlists",
                "edit",
                "playlist_id:5",
                "cmd:add",
                "track_id:9"
            ]),
        ];
        for cmd in &changes {
            let raw = cmd.as_array().unwrap().clone();
            assert!(
                check(&Who(Role::Listener), "", &raw, &apps()).is_err(),
                "listener could run {cmd}"
            );
            assert!(
                check(&Who(Role::Admin), "", &raw, &apps()).is_ok(),
                "admin refused {cmd}"
            );
        }
        // Listing and playing stay open to listeners.
        ok("", json!(["playlists", 0, 50]));
        ok("", json!(["playlists", "tracks", 0, 50, "playlist_id:5"]));
        ok(
            MINE,
            json!(["playlistcontrol", "cmd:load", "playlist_id:5"]),
        );
    }

    #[test]
    fn album_id_filter_takes_ids_only() {
        ok("", json!(["albums", 0, 1, "tags:al", "album_id:7"]));
        for bad in [
            "album_id:",
            "album_id:x",
            "album_id:7;rescan",
            "album_id:-1",
            "album_id:../7",
        ] {
            denied("", json!(["albums", 0, 1, bad]));
        }
        denied("", json!(["albums", 0, 1, "album_id:7", "album_id:8"]));
    }

    #[test]
    fn library_browsing_is_allowed() {
        for cmd in [
            json!(["artists", 0, 50, "tags:s", "search:abba"]),
            json!([
                "albums",
                0,
                100,
                "tags:alj",
                "sort:artistalbum",
                "artist_id:12"
            ]),
            json!(["titles", 0, 50, "album_id:7", "tags:adl", "sort:tracknum"]),
            json!(["genres", 0, 50]),
            json!(["years", 0, 50, "hasAlbums:1"]),
            json!(["search", 0, 20, "term:blue"]),
            json!(["playlists", 0, 50]),
            json!(["playlists", "tracks", 0, 50, "playlist_id:3", "tags:al"]),
            json!(["musicfolder", 0, 50, "folder_id:9"]),
            json!(["favorites", "items", 0, 50, "item_id:b99447ab.1"]),
            json!(["apps", 0, 50]),
            json!(["radios", 0, 50]),
            json!(["players", 0, 50]),
            json!(["serverstatus", 0, 50]),
            json!(["syncgroups", "?"]),
        ] {
            assert!(ok("", cmd).player.is_empty());
        }
    }

    #[test]
    fn playback_on_permitted_players_is_allowed() {
        for cmd in [
            json!(["status", 0, 50, "tags:adlKj"]),
            json!(["status", "-", 1, "tags:adl"]),
            json!(["play"]),
            json!(["pause", "1"]),
            json!(["stop"]),
            json!(["mode", "?"]),
            json!(["time", "93.5"]),
            json!(["time", "?"]),
            json!(["mixer", "volume", "+5"]),
            json!(["mixer", "volume", 40]),
            json!(["mixer", "muting", "toggle"]),
            json!(["power", "1"]),
            json!(["sleep", "900"]),
            json!(["playlist", "index", "+1"]),
            json!(["playlist", "move", 3, 0]),
            json!(["playlist", "delete", 2]),
            json!(["playlist", "clear"]),
            json!(["playlist", "shuffle", "1"]),
            json!(["playlistcontrol", "cmd:load", "album_id:140912"]),
            json!(["playlistcontrol", "cmd:add", "track_id:1,2,3"]),
            json!([
                "playlistcontrol",
                "cmd:insert",
                "artist_id:9",
                "play_index:0"
            ]),
            json!(["sync", HOME]),
            json!(["sync", "-"]),
            json!(["favorites", "playlist", "play", "item_id:b99447ab.1"]),
            json!(["spotty", "items", 0, 50, "item_id:3.1", "menu:1"]),
            json!(["bbcsounds", "playlist", "play", "item_id:0.2"]),
        ] {
            assert_eq!(ok(MINE, cmd).player, MINE);
        }
    }

    #[test]
    fn dangerous_commands_are_denied() {
        for cmd in [
            json!(["readdirectory", 0, 100, "folder:/etc"]),
            json!(["pref", "audiodir", "/"]),
            json!(["pref", "audiodir", "?"]),
            json!(["playerpref", "volume", "100"]),
            json!(["restartserver"]),
            json!(["stopserver"]),
            json!(["wipecache"]),
            json!(["debug", "server", "DEBUG"]),
            json!(["logging", "group:everything"]),
            json!(["getstring", "SETUP_AUDIODIR"]),
            json!(["setsncredentials", "x", "y"]),
            json!(["button", "power"]),
            json!(["display", "a", "b"]),
            json!(["rescan"]),
            json!(["nonexistent"]),
        ] {
            denied("", cmd.clone());
            denied(MINE, cmd);
        }
    }

    #[test]
    fn urls_and_paths_are_refused_everywhere() {
        for cmd in [
            json!(["playlist", "play", "file:///etc/passwd"]),
            json!(["playlist", "add", "http://evil.example/x.mp3"]),
            json!(["playlist", "insert", "/srv/music/a.flac"]),
            json!(["playlist", "load", "file:///x"]),
            json!(["playlist", "save", "name"]),
            json!(["playlistcontrol", "cmd:load", "url:file:///etc/passwd"]),
            json!(["playlistcontrol", "cmd:load", "playlist_name:x"]),
            json!(["playlistcontrol", "cmd:load", "album_id:1", "url:file:///x"]),
            json!(["playlistcontrol", "cmd:load"]),
            json!(["playlistcontrol", "cmd:load", "album_id:1", "artist_id:2"]),
            json!(["playlistcontrol", "cmd:play", "album_id:1"]),
            json!(["playlistcontrol", "cmd:load", "album_id:../1"]),
            json!(["favorites", "playlist", "play", "url:file:///x"]),
            json!(["spotty", "playlist", "play", "url:http://x"]),
            json!(["spotty", "playlist", "play"]),
        ] {
            denied(MINE, cmd);
        }
    }

    #[test]
    fn players_the_caller_cannot_control_are_refused() {
        denied(OTHER, json!(["play"]));
        denied(OTHER, json!(["status", 0, 10]));
        denied(MINE, json!(["sync", OTHER]));
        denied(MINE, json!(["sync", MINE]));
        denied("not-a-mac", json!(["play"]));
        denied("", json!(["play"]));
        // Admins may control any player.
        let raw = json!(["play"]).as_array().unwrap().clone();
        assert!(check(&Who(Role::Admin), OTHER, &raw, &apps()).is_ok());
    }

    #[test]
    fn the_home_menu_is_read_only_and_player_scoped() {
        assert_eq!(
            ok(MINE, json!(["menu", 0, 500, "direct:1"])).kind,
            Kind::HomeMenu
        );
        assert_eq!(ok(MINE, json!(["menu", 0, 100])).kind, Kind::HomeMenu);
        denied(OTHER, json!(["menu", 0, 500, "direct:1"]));
        denied("", json!(["menu", 0, 500]));
        denied(MINE, json!(["menu", 0, 500, "direct:0"]));
        denied(MINE, json!(["menu", 0, 500, "item_id:1"]));
        denied(MINE, json!(["menu", "status", 0, 10]));
        denied(MINE, json!(["menu", 0, 100000]));
        denied(MINE, json!(["menu"]));
    }

    /// A home menu like LMS's: library entries, plugin entries in several nodes, a node with
    /// children (Random Mix), and settings and power entries that must never reach a device.
    fn home_menu_fixture() -> Value {
        json!({"count": 14, "item_loop": [
            {"id": "myMusic", "node": "home", "text": "My Music"},
            {"id": "favorites", "node": "home", "actions": {"go": {"cmd": ["favorites", "items"]}}},
            {"id": "playerpower", "node": "home", "actions": {"do": {"cmd": ["power", "0"]}}},
            {"id": "globalSearch", "node": "home", "actions": {"go": {"cmd": ["globalsearch", "items"]}}},
            {"id": "myMusicAlbums", "node": "myMusic", "actions": {"go": {"cmd": ["browselibrary", "items"], "params": {"mode": "albums"}}}},
            {"id": "pluginSomePlugin", "node": "myMusic", "text": "Some Plugin", "actions": {"go": {"cmd": ["someplugin", "menu"], "player": 0}}},
            {"id": "randomplay", "node": "myMusic", "isANode": 1, "text": "Random Mix"},
            {"id": "randomtracks", "node": "randomplay", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["randomplay", "tracks"]}}},
            {"id": "opmlsomeradio", "node": "radios", "actions": {"go": {"cmd": ["someradio", "items"], "params": {"menu": "someradio"}}}},
            {"id": "opmlsomeapp", "node": "", "actions": {"go": {"cmd": ["someapp", "items"], "params": {"menu": "someapp"}}}},
            {"id": "settingsAudio", "node": "settings", "text": "Audio"},
            {"id": "settingsXfade", "node": "settingsAudio", "actions": {"go": {"cmd": ["crossfadesettings"]}}},
            {"id": "settingsRescan", "node": "advancedSettings", "actions": {"go": {"cmd": ["rescanplugin", "menu"]}}},
            {"id": "sneaky", "node": "settingsAudio", "actions": {"go": {"cmd": ["someplugin", "wipe"]}}},
            {"id": "orphan", "node": "nowhere", "actions": {"go": {"cmd": ["someplugin", "orphan"]}}},
            {"id": "lanlink", "node": "myMusic", "actions": {"go": {"cmd": ["someplugin", "items"], "params": {"url": "http://10.0.0.10/x"}}}}
        ]})
    }

    #[test]
    fn the_home_menu_keeps_only_library_and_plugin_entries() {
        let out = filter_response(Kind::HomeMenu, home_menu_fixture(), &|_| true);
        let ids: Vec<&str> = out["item_loop"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "pluginSomePlugin",
                "randomplay",
                "randomtracks",
                "opmlsomeradio",
                "opmlsomeapp",
                "lanlink"
            ]
        );
        assert_eq!(out["count"], 6);
        // The LAN URL's action is removed whole, so it can't be learned.
        assert!(
            out["item_loop"][5]
                .get("actions")
                .is_none_or(|a| a.get("go").is_none())
        );
        // Nothing from settings, power or search is offered, so none of it can be learned.
        let learned = crate::menu_actions::learn(&out);
        let cmds: Vec<String> = learned.iter().map(|a| a.cmd.join(" ")).collect();
        for never in [
            "power 0",
            "crossfadesettings",
            "rescanplugin menu",
            "someplugin wipe",
            "someplugin orphan",
            "globalsearch items",
            "browselibrary items",
        ] {
            assert!(
                !cmds.iter().any(|c| c == never),
                "{never} learned: {cmds:?}"
            );
        }
        assert!(cmds.iter().any(|c| c == "someplugin menu"));
        assert!(cmds.iter().any(|c| c == "randomplay tracks"));
        // Other responses are untouched by this filter.
        let plain = filter_response(Kind::Plain, home_menu_fixture(), &|_| true);
        assert_eq!(plain["item_loop"].as_array().unwrap().len(), 16);
    }

    #[test]
    fn hostile_home_menus_never_panic() {
        for v in [
            json!({"item_loop": "x"}),
            json!({"item_loop": [1, null, [], {"id": 5, "node": []}]}),
            json!({"item_loop": [{"id": "a", "node": "b"}, {"id": "b", "node": "a"}]}),
            json!([]),
            json!(null),
        ] {
            let _ = filter_response(Kind::HomeMenu, v, &|_| true);
        }
        // A cycle not attached to a root is dropped.
        let out = filter_response(
            Kind::HomeMenu,
            json!({"item_loop": [{"id": "a", "node": "b"}, {"id": "b", "node": "a"}]}),
            &|_| true,
        );
        assert_eq!(out["item_loop"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn server_queries_reject_a_player() {
        denied(MINE, json!(["albums", 0, 10]));
        denied(MINE, json!(["serverstatus", 0, 10]));
    }

    #[test]
    fn parameters_are_validated() {
        for cmd in [
            json!(["albums", 0, 10, "tags:al", "tags:al"]),
            json!(["albums", 0, 10, "unknown:1"]),
            json!(["albums", 0, 10, "sort:'; DROP TABLE"]),
            json!(["albums", 0, 10, "artist_id:abc"]),
            json!(["albums", 0, 10, "tags:a l"]),
            json!(["albums", 0, 10, "positional"]),
            json!(["albums", 0, 501]),
            json!(["albums", "x", 10]),
            json!(["albums"]),
            json!(["artists", 0, 10, "search:\n"]),
            json!(["titles", 0, 10, "search:".to_string() + &"a".repeat(300)]),
            json!(["albums", 0, 10, {"a": 1}]),
            json!(["albums", 0, 10, [1]]),
            json!(["albums", 0, 10, null]),
            json!(["albums", 0, -1]),
            json!(["albums", 0, 1e300]),
        ] {
            denied("", cmd);
        }
        denied(MINE, json!(["time", "abc"]));
        denied(MINE, json!(["mixer", "volume", "1e9"]));
        denied(MINE, json!(["mixer", "treble", "3"]));
        denied(MINE, json!(["status", 0, 10, "subscribe:30"]));
        denied(MINE, json!(["pause", "2"]));
        denied(MINE, json!(["play", "extra"]));
        let long: Vec<Value> = (0..20).map(|_| json!("x")).collect();
        assert!(check(&Who(Role::Listener), "", &long, &apps()).is_err());
        assert!(check(&Who(Role::Listener), "", &[], &apps()).is_err());
    }

    #[test]
    fn rescan_is_admin_only() {
        denied("", json!(["rescan"]));
        let raw = json!(["rescan"]).as_array().unwrap().clone();
        assert!(check(&Who(Role::Admin), "", &raw, &apps()).is_ok());
        let raw = json!(["rescan", "playlists"]).as_array().unwrap().clone();
        assert!(check(&Who(Role::Admin), "", &raw, &apps()).is_err());
    }

    #[test]
    fn saved_playlists_can_be_edited_by_id() {
        for cmd in [
            json!(["playlists", "new", "name:Road trip"]),
            json!(["playlists", "rename", "playlist_id:3", "newname:Drive"]),
            json!(["playlists", "delete", "playlist_id:3"]),
            json!([
                "playlists",
                "edit",
                "playlist_id:3",
                "cmd:add",
                "track_id:9"
            ]),
            json!([
                "playlists",
                "edit",
                "playlist_id:3",
                "cmd:move",
                "index:1",
                "toindex:0"
            ]),
        ] {
            admin_ok(cmd);
        }
        admin_denied(json!(["playlists", "new"]));
        admin_denied(json!([
            "playlists",
            "edit",
            "playlist_id:3",
            "cmd:add",
            "url:file:///x"
        ]));
        admin_denied(json!(["playlists", "edit", "cmd:delete"]));
    }

    #[test]
    fn unknown_apps_are_not_callable() {
        denied(MINE, json!(["evilapp", "items", 0, 10]));
        // The plugin's management commands are never reachable, even if a menu named them.
        let mut with_anywhere = apps();
        with_anywhere.insert("anywhere".into());
        for cmd in [
            json!(["anywhere", "users"]),
            json!(["anywhere", "user_create", "username:x"]),
        ] {
            let raw = cmd.as_array().unwrap().clone();
            assert!(check(&Who(Role::Admin), "", &raw, &with_anywhere).is_err());
            assert!(check(&Who(Role::Admin), MINE, &raw, &with_anywhere).is_err());
        }
        denied("", json!(["evilapp", "items", 0, 10]));
    }

    #[test]
    fn learned_apps_exclude_core_commands() {
        let menu = json!({
            "appss_loop": [
                {"cmd": "spotty"}, {"cmd": "readdirectory"}, {"cmd": "pref"},
                {"cmd": "Bad Name"}, {"cmd": "../x"}, {"cmd": ""}, {"cmd": "anywhere"}
            ],
            "radioss_loop": [{"cmd": "bbcsounds"}]
        });
        assert_eq!(learn_apps(&menu), vec!["spotty", "bbcsounds"]);
    }

    #[test]
    fn responses_lose_addresses_paths_and_unpermitted_players() {
        let raw = json!({
            "version": "9.1.1", "uuid": "secret-uuid", "ip": "10.0.0.10", "httpport": "9000",
            "player count": 3, "other player count": 2,
            "players_loop": [
                {"playerid": MINE, "name": "Mine", "ip": "10.0.0.20:4000"},
                {"playerid": OTHER, "name": "Theirs", "ip": "10.0.0.21:4000"},
                {"playerid": HOME, "name": "Home", "ip": "10.0.0.22:4000"}
            ]
        });
        let may = |p: &str| p == MINE || p == HOME;
        let out = filter_response(Kind::PlayerList, raw, &may);
        let s = out.to_string();
        for leaked in ["10.0.0.", "secret-uuid", "9000", "Theirs", "other player"] {
            assert!(!s.contains(leaked), "{leaked} leaked: {s}");
        }
        assert_eq!(out["player count"], 2);
        assert_eq!(out["players_loop"].as_array().unwrap().len(), 2);

        let tracks =
            json!({"titles_loop": [{"id": 1, "url": "file:///srv/music/a.flac", "title": "A"}]});
        let out = filter_response(Kind::Plain, tracks, &may);
        assert!(!out.to_string().contains("file://"));
        assert_eq!(out["titles_loop"][0]["title"], "A");
    }

    #[test]
    fn sensitive_values_are_recognised() {
        for yes in [
            "file:///srv/a.flac",
            " FILE:/x",
            "http://10.1.2.3/x",
            "https://192.168.250.7:9000",
            "http://172.20.0.1/",
            "http://127.0.0.1:9000/",
            "http://169.254.1.1/",
            "http://[::1]:9000/",
            "http://[fd12::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:10.0.0.1]/",
            "http://localhost/",
            "http://user:pw@10.0.0.1/",
            "rtsp://cam.lan/stream",
            "http://nas.home.arpa/x",
            "ws://player.local:80",
        ] {
            assert!(sensitive_value(yes), "{yes}");
        }
        for no in [
            "https://images.example.com/a.jpg",
            "http://8.8.8.8/x",
            "http://172.32.0.1/",
            "/imageproxy/x/image.png",
            "html/images/cover.png",
            "Local Hero",
            "spotify:track:1",
            "https://[2001:db8::1]/",
            "http://example.com/file:/x",
        ] {
            assert!(!sensitive_value(no), "{no}");
        }
    }

    /// Regression: `playlists` returned `favorites_url: file:///…`, leaking the server's
    /// filesystem layout. Local paths and LAN URLs must go whatever key LMS puts them under.
    #[test]
    fn local_paths_and_lan_urls_never_leak_under_any_key() {
        let raw = json!({
            "playlists_loop": [
                {"id": 7, "playlist": "Gym", "favorites_url": "file:///srv/music/Playlists/Gym.m3u"},
                {"id": 8, "playlist": "Road", "some_new_key": "FILE:///srv/x.m3u",
                 "stream": "http://10.0.0.10:9000/stream.mp3?player=x",
                 "lan6": "http://[fd00::10]:9000/x", "loop": "http://127.0.0.1:9000/y",
                 "mdns": "http://lms.local:9000/z", "home": "https://music.example.lan/x"}
            ],
            "list": ["file:///etc/passwd", "kept"]
        });
        let out = filter_response(Kind::Plain, raw, &|_| true);
        let s = out.to_string();
        for leaked in [
            "file:",
            "FILE:",
            "/srv/",
            "10.0.0.10",
            "fd00::",
            "127.0.0.1",
            "lms.local",
            ".lan/",
        ] {
            assert!(
                !s.to_lowercase().contains(&leaked.to_lowercase()),
                "{leaked} leaked: {s}"
            );
        }
        // Everything else survives, including public artwork URLs clients need.
        assert_eq!(out["playlists_loop"][0]["playlist"], "Gym");
        assert_eq!(out["playlists_loop"][1]["id"], 8);
        assert_eq!(out["list"], json!(["", "kept"]));
        let art = json!({"artwork_url": "https://images.example.com/a.jpg", "icon": "/imageproxy/x/image.png",
                         "image": "html/images/cover.png"});
        assert_eq!(filter_response(Kind::Plain, art.clone(), &|_| true), art);
    }

    #[test]
    fn arbitrary_input_never_panics() {
        // Deterministic pseudo-random argument lists.
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let words = [
            "albums",
            "status",
            "playlist",
            "play",
            "mixer",
            "volume",
            "sync",
            "playlists",
            "edit",
            "playlistcontrol",
            "cmd:load",
            "album_id:1",
            "tags:al",
            "spotty",
            "items",
            "0",
            "10",
            "-",
            "?",
            "url:file:///x",
            ":",
            "",
            "a:b:c",
            "item_id:x",
            "favorites",
            "rescan",
            MINE,
        ];
        for _ in 0..20_000 {
            let n = (next() % 8) as usize;
            let raw: Vec<Value> = (0..n)
                .map(|_| json!(words[(next() % words.len() as u64) as usize]))
                .collect();
            let player = [MINE, OTHER, "", "zz"][(next() % 4) as usize];
            let _ = check(&Who(Role::Listener), player, &raw, &apps());
        }
    }
}
