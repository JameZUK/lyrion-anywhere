//! Test harness: the real gateway (in-process) in front of a minimal fake LMS, plus an
//! in-memory secure store, an event collector and a TCP proxy that can cut connections.
#![allow(dead_code)]

pub mod flac;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anywhere_gateway::accounts::{Accounts, Role};
use anywhere_gateway::{Config, Gateway, router};
use anywhere_mobile::{
    EventListener, EventsState, SecureStore, Server, ServerEvent, Session, SessionObserver,
    StoreError, StoredCredentials,
};
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::IntoResponse;
use axum::routing::any;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::OwnedWriteHalf;

pub const PW: &str = "correct horse battery";

/// The secret the test gateway signs plays with (test-only, not a real secret).
pub const PLAYS_SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Plays the fake LMS plugin recorded: (track id, played_at, gateway key).
pub fn recorded_plays(lms: &FakeLms) -> Vec<(u64, u64, String)> {
    lms.rpc_calls()
        .iter()
        .filter(|p| p[1][0] == "anywhere" && p[1][1] == "play_record")
        .filter_map(|p| {
            let arg = |n: &str| {
                p[1].as_array()?.iter().find_map(|a| {
                    a.as_str()?
                        .strip_prefix(&format!("{n}:"))
                        .map(str::to_string)
                })
            };
            Some((
                arg("track_id")?.parse().ok()?,
                arg("played_at")?.parse().ok()?,
                arg("key")?,
            ))
        })
        .collect()
}

/// Home players the fake LMS knows (not granted to anyone in these tests).
pub const HOME_PLAYER: &str = "aa:bb:cc:dd:ee:01";

/// Canned LMS JSON-RPC results.
/// The fake LMS's alarms for any player, and the sounds it offers (with LMS's own URLs,
/// which must never reach the app).
fn fake_alarm(words: &Value) -> Value {
    let w = |i: usize| words[i].as_str().unwrap_or("");
    match (w(0), w(1)) {
        ("alarm", "playlists") => json!({"item_loop": [
            {"category": "CURRENT_PLAYLIST", "title": "{ALARM_USE_CURRENT_PLAYLIST}"},
            {"category": "FAVORITES", "title": "Morning radio", "url": "http://radio.example.com/stream"}
        ]}),
        ("alarms", _) => json!({"count": 1, "alarms_loop": [
            {"id": "1a2b3c4d", "dow": "1,2,3,4,5", "enabled": "1", "repeat": "1", "time": "25200",
             "volume": "40", "shufflemode": "0", "url": "http://radio.example.com/stream"}
        ]}),
        ("alarm", "add") => json!({"id": "5e6f7a8b"}),
        ("playerpref", _) => json!({"_p2": "1"}),
        _ => json!({}),
    }
}

/// A track's credits as LMS gives them for `tags:AS` (invented names).
fn fake_credits() -> Value {
    json!({"count": 1, "titles_loop": [{
        "id": 1, "title": "One",
        "albumartist": "Example Ensemble", "albumartist_ids": "3",
        "artist": "Singer One, Singer Two", "artist_ids": "21,22",
        "composer": "Some Composer", "composer_ids": "41",
        "conductor": "A Conductor", "conductor_ids": "51",
        "url": "file:///srv/music/x/1.flac"
    }]})
}

/// The Music & Artist Information plugin (invented names): artist 3 has a biography with
/// HTML and a photo on an external host; album 8 has a review; anything else has nothing.
fn fake_mai(words: &Value) -> Value {
    match (words[1].as_str(), words[2].as_str()) {
        (Some("biography"), Some("artist_id:3")) => json!({
            "artist_id": "3", "artist": "Invented Artist",
            "biography": "<p>An invented artist &amp; friends.</p><script>x()</script><p>Second.</p>"
        }),
        (Some("artistphoto"), Some("artist_id:3")) => json!({
            "artist_id": "3", "url": "https://images.example.com/a/3.jpg",
            "credits": "Discogs", "width": "600", "height": "600"
        }),
        (Some("albumreview"), Some("album_id:8")) => json!({
            "album_id": "8", "albumreview": "A <i>fine</i> record."
        }),
        _ => json!({"error": "No information found"}),
    }
}

fn fake_result(cmd: &str) -> Value {
    match cmd {
        "albums" => json!({"count": 2, "albums_loop": [
            {"id": 7, "album": "First Album", "artist": "Artist", "artist_id": 3, "year": 1999, "artwork_track_id": "abcdef12"},
            {"id": 8, "album": "Second Album", "url": "file:///srv/music/x"}
        ]}),
        "titles" => json!({"count": 2, "titles_loop": [
            {"id": 1, "title": "One", "tracknum": "1", "duration": "61.5", "coverid": "abcdef12"},
            {"id": 2, "title": "Two", "tracknum": "2", "url": "file:///srv/music/x/2.flac"}
        ]}),
        "artists" => json!({"count": 1, "artists_loop": [{"id": 3, "artist": "Artist"}]}),
        "genres" => json!({"count": 1, "genres_loop": [{"id": 4, "genre": "Jazz"}]}),
        "search" => json!({"count": 3,
            "contributors_count": 1, "albums_count": 4, "tracks_count": 30,
            "contributors_loop": [{"contributor": "Artist", "contributor_id": 3}],
            "albums_loop": [{"album": "First Album", "album_id": 7, "artist": "Artist"}],
            "tracks_loop": [{"track": "One", "track_id": 1, "url": "file:///srv/music/x/1.flac"}]}),
        "players" => json!({"count": 1, "players_loop": [
            {"playerid": HOME_PLAYER, "name": "Home One", "ip": "10.0.0.31:4000", "connected": 1}
        ]}),
        "status" => json!({"player_name": "Phone", "mode": "play", "time": 12.5, "duration": 61.5,
            "mixer volume": 40, "playlist_tracks": 2, "playlist_cur_index": "0",
            "playlist_loop": [{"id": 1, "title": "One", "coverid": "abcdef12", "playlist index": 0}]}),
        "years" => json!({"count": 2, "years_loop": [{"year": 2001}, {"year": 1999}]}),
        "musicfolder" => json!({"count": 2, "folder_loop": [
            {"id": 12, "type": "folder", "title": "Jazz"},
            {"id": 13, "type": "track", "title": "One", "coverid": "abcdef12", "duration": "1:01",
             "url": "file:///srv/music/Jazz/one.flac"}
        ]}),
        "libraries" => json!({"folder_loop": [{"id": "ab12", "name": "Classical"}]}),
        "favorites" => json!({"title": "Favourites", "count": 2, "loop_loop": [
            {"id": "f0.0", "name": "Radio", "isaudio": 1, "hasitems": 0},
            {"id": "f0.1", "name": "Folder", "isaudio": 0, "hasitems": 1}
        ]}),
        "playlists" => json!({"count": 1, "playlists_loop": [{"id": 5, "playlist": "Road trip"}],
            "playlisttracks_loop": [{"id": 1, "title": "One", "duration": 61}]}),
        "apps" => json!({"count": 1, "appss_loop": [
            {"cmd": "someapp", "name": "Some App", "icon": "plugins/SomeApp/html/images/icon.png"}
        ]}),
        "radios" => {
            json!({"count": 1, "radioss_loop": [{"cmd": "someradio", "name": "Some Radio"}]})
        }
        // The player's home menu: library entries (dropped by the gateway) and plugins.
        "menu" => json!({"count": 5, "item_loop": [
            {"id": "myMusicAlbums", "node": "myMusic", "actions": {"go": {"cmd": ["browselibrary", "items"], "params": {"mode": "albums"}}}},
            {"id": "pluginSomePlugin", "node": "myMusic", "text": "Some Plugin", "actions": {"go": {"cmd": ["someplugin", "items"], "params": {"menu": "1"}}}},
            {"id": "randomplay", "node": "myMusic", "isANode": 1, "text": "Random Mix"},
            {"id": "randomtracks", "node": "randomplay", "text": "Song Mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["randomplay", "tracks"]}}},
            {"id": "opmlsomeapp", "node": "", "text": "Some App", "actions": {"go": {"cmd": ["someapp", "items"], "params": {"menu": "someapp"}}}}
        ]}),
        // A context menu, as LMS builds them: library links, a plugin's "more like this",
        // and the file's location (which the gateway must never pass on).
        "trackinfo" | "albuminfo" | "artistinfo" | "genreinfo" | "yearinfo" | "folderinfo"
        | "playlistinfo" => json!({"count": 4, "title": "One", "item_loop": [
            {"text": "Artist: Artist", "actions": {"go": {"cmd": ["browselibrary", "items"], "params": {"mode": "albums", "artist_id": "3", "menu": "1"}}}},
            {"text": "More like this", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["someplugin", "mix"], "params": {"track_id": "7"}}}},
            {"text": "Location: /srv/music/Jazz/one.flac"},
            {"text": "Duration: 1:01"},
            {"text": "Instant mix", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["audiomuseai", "mix"], "params": {"track_id": "7"}}}},
            {"text": "Similar tracks", "actions": {"go": {"cmd": ["audiomuseai", "similar"], "params": {"track_id": "7"}}}}
        ]}),
        // AudioMuse-AI's list of similar tracks.
        "audiomuseai" => json!({"count": 2, "item_loop": [
            {"text": "Two\nArtist", "type": "audio", "actions": {"play": {"cmd": ["playlistcontrol"], "params": {"cmd": "load", "track_id": "2"}}}},
            {"text": "Three\nArtist", "type": "audio", "actions": {"play": {"cmd": ["playlistcontrol"], "params": {"cmd": "load", "track_id": "3"}}}}
        ]}),
        // Don't Stop The Music's settings for a player: off, two providers.
        "dontstopthemusicsetting" => json!({"count": 3, "item_loop": [
            {"text": "Disabled", "radio": 1, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", 0]}}},
            {"text": "Random songs", "radio": 0, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", "PLUGIN_RANDOM_TRACK"]}}},
            {"text": "Similar tracks", "radio": 0, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", "PLUGIN_DSTM_SIMILAR"]}}}
        ]}),
        // An app's menus: a search box and something to play.
        "someapp" => json!({"count": 2, "item_loop": [
            {"text": "Search", "type": "search", "actions": {"go": {"cmd": ["someapp", "items"], "params": {"item_id": "0", "search": "__TAGGEDINPUT__", "menu": "someapp"}}}},
            {"text": "A track", "type": "audio", "actions": {"play": {"cmd": ["someapp", "playlist", "play"], "params": {"item_id": "1"}}}}
        ]}),
        // A plugin that needs setting up first.
        "someplugin" => {
            json!({"count": 1, "item_loop": [{"text": "Log in to Some Plugin in its settings"}]})
        }
        _ => json!({}),
    }
}

/// Slimproto frames received (opcode, payload), shared with the reader tasks.
pub type SlimFrames = Arc<Mutex<Vec<([u8; 4], Vec<u8>)>>>;

/// One `/stream.mp3` answer of the fake LMS.
pub struct StreamPlan {
    pub body: Vec<u8>,
    /// Send this many bytes, then wait for the gate before sending the rest.
    pub gate_at: Option<(usize, Arc<tokio::sync::Notify>)>,
}

impl StreamPlan {
    pub fn whole(body: Vec<u8>) -> StreamPlan {
        StreamPlan {
            body,
            gate_at: None,
        }
    }
}

pub struct FakeLms {
    pub http_addr: SocketAddr,
    pub cli_addr: SocketAddr,
    pub slim_addr: SocketAddr,
    pub rpc: Arc<Mutex<Vec<Value>>>,
    pub http_paths: Arc<Mutex<Vec<String>>>,
    cli_clients: Arc<tokio::sync::Mutex<Vec<OwnedWriteHalf>>>,
    /// Slimproto frames received from players, in order (opcode, payload).
    pub slim_frames: SlimFrames,
    /// Slimproto connections accepted so far.
    pub slim_accepted: Arc<AtomicUsize>,
    slim_conns: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<OwnedWriteHalf>>,
    /// Answers for `/stream.mp3`, in order; 404 once they run out.
    pub streams: Arc<Mutex<std::collections::VecDeque<StreamPlan>>>,
    /// Library files (see [`FakeLms::add_track`]): id → (titles item, file).
    pub library: Library,
    /// Sync groups, as LMS keeps them: member → the player whose group it joined.
    pub syncs: Syncs,
}

pub type Library = Arc<Mutex<std::collections::HashMap<u64, (Value, Vec<u8>)>>>;

/// Sync state of the fake LMS. With `refuse` set, `sync <player>` is accepted but does
/// nothing (as when LMS won't group two players).
#[derive(Clone, Default)]
pub struct Syncs {
    pub members: Arc<Mutex<std::collections::HashMap<String, String>>>,
    pub refuse: Arc<std::sync::atomic::AtomicBool>,
}

impl Syncs {
    /// `<player> sync <arg>`: `arg` joins `player`'s group; `-` takes `player` out.
    fn apply(&self, player: &str, arg: &str) {
        let mut m = self.members.lock().unwrap();
        if arg == "-" {
            m.remove(player);
            let followers: Vec<String> = m
                .iter()
                .filter(|(_, master)| *master == player)
                .map(|(k, _)| k.clone())
                .collect();
            // The others carry on together under the first of them.
            if let Some((first, rest)) = followers.split_first() {
                m.remove(first);
                for r in rest {
                    m.insert(r.clone(), first.clone());
                }
            }
        } else if !self.refuse.load(Ordering::SeqCst) {
            let master = m.get(player).cloned().unwrap_or_else(|| player.to_string());
            m.remove(arg);
            m.insert(arg.to_string(), master);
        }
    }

    /// Add `sync_master`/`sync_slaves` to a status answer, as LMS does for grouped players.
    fn annotate(&self, player: &str, status: &mut Value) {
        let m = self.members.lock().unwrap();
        let master = m.get(player).cloned().unwrap_or_else(|| player.to_string());
        let slaves: Vec<&String> = m
            .iter()
            .filter(|(_, v)| **v == master)
            .map(|(k, _)| k)
            .collect();
        if !slaves.is_empty() {
            status["sync_master"] = json!(master);
            status["sync_slaves"] = json!(
                slaves
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
}

/// Size of the "huge" cover served for `/music/99999999/…`.
pub const HUGE_COVER: usize = 3 * 1024 * 1024;

impl FakeLms {
    pub async fn start() -> FakeLms {
        let rpc: Arc<Mutex<Vec<Value>>> = Default::default();
        let http_paths: Arc<Mutex<Vec<String>>> = Default::default();
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_addr = http.local_addr().unwrap();
        let streams: Arc<Mutex<std::collections::VecDeque<StreamPlan>>> = Default::default();
        let library: Library = Default::default();
        let syncs = Syncs::default();
        let (r, paths, plans, lib, sy) = (
            rpc.clone(),
            http_paths.clone(),
            streams.clone(),
            library.clone(),
            syncs.clone(),
        );
        let app = Router::new().fallback(any(move |req: Request| {
            let (r, paths, plans, lib, sy) = (
                r.clone(),
                paths.clone(),
                plans.clone(),
                lib.clone(),
                sy.clone(),
            );
            async move {
                let path = req.uri().path().to_string();
                paths.lock().unwrap().push(path.clone());
                if path == "/stream.mp3" {
                    let Some(plan) = plans.lock().unwrap().pop_front() else {
                        return (axum::http::StatusCode::NOT_FOUND, "no stream").into_response();
                    };
                    return stream_response(plan);
                }
                if path == "/jsonrpc.js" {
                    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                        .await
                        .unwrap_or_default();
                    let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                    let params = v["params"].clone();
                    let cmd = params[1][0].as_str().unwrap_or("").to_string();
                    let track_id = params[1]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str()?.strip_prefix("track_id:"))
                        .find_map(|id| id.parse::<u64>().ok());
                    let player = params[0].as_str().unwrap_or("").to_string();
                    let arg1 = params[1][1].as_str().unwrap_or("").to_string();
                    r.lock().unwrap().push(params.clone());
                    // `titles … track_id:1,2,3`: several tracks at once.
                    let many: Option<Vec<u64>> = params[1]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str()?.strip_prefix("track_id:"))
                        .find(|ids| ids.contains(','))
                        .map(|ids| ids.split(',').filter_map(|i| i.parse().ok()).collect());
                    let has = |want: &str| {
                        params[1]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|p| p.as_str() == Some(want))
                    };
                    let role = params[1]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find_map(|p| p.as_str()?.strip_prefix("role_id:"))
                        .map(str::to_string);
                    let mut result = match track_id {
                        // A track's credits (`tags:AS`), with LMS's comma-joined names and ids.
                        Some(_) if cmd == "titles" && has("tags:AS") => fake_credits(),
                        _ if cmd == "titles" && many.is_some() => {
                            // Added tracks, then the canned ones (tracks 1 and 2).
                            let l = lib.lock().unwrap();
                            let canned = fake_result("titles");
                            let items: Vec<Value> = many
                                .unwrap()
                                .iter()
                                .filter_map(|id| {
                                    l.get(id).map(|(i, _)| i.clone()).or_else(|| {
                                        canned["titles_loop"]
                                            .as_array()?
                                            .iter()
                                            .find(|t| t["id"].as_u64() == Some(*id))
                                            .cloned()
                                    })
                                })
                                .collect();
                            json!({"count": items.len(), "titles_loop": items})
                        }
                        Some(id) if cmd == "titles" => match lib.lock().unwrap().get(&id) {
                            Some((item, _)) => json!({"count": 1, "titles_loop": [item]}),
                            None => json!({"count": 0}),
                        },
                        _ if cmd == "alarm"
                            || cmd == "alarms"
                            || (cmd == "playerpref" && arg1 == "alarmsEnabled") =>
                        {
                            fake_alarm(&params[1])
                        }
                        _ if cmd == "musicartistinfo" => fake_mai(&params[1]),
                        _ => fake_result(&cmd),
                    };
                    // The A–Z index (`tags:Z`) of albums and artists.
                    if (cmd == "albums" || cmd == "artists") && has("tags:Z") {
                        result["indexList"] = json!([["A", 2], ["B", 1], ["Ö", 3]]);
                    }
                    // "Appears as": one row of an artist's albums in a role, and the count.
                    if cmd == "albums" && params[1][2] == json!("1") && role.is_some() {
                        result["count"] = json!(match role.as_deref() {
                            Some("COMPOSER") => 12,
                            Some("CONDUCTOR") => 3,
                            _ => 0,
                        });
                    }
                    if cmd == "sync" {
                        sy.apply(&player, &arg1);
                    }
                    if cmd == "anywhere" && arg1 == "play_record" {
                        // The plugin's play_record (the gateway's own tests check its
                        // signature): record once per key, like the plugin.
                        let key_of = |p: &Value| {
                            p[1].as_array()
                                .into_iter()
                                .flatten()
                                .find_map(|a| a.as_str()?.strip_prefix("key:").map(str::to_string))
                        };
                        let log = r.lock().unwrap();
                        let me = key_of(log.last().unwrap());
                        let seen = log
                            .iter()
                            .filter(|p| p[1][0] == "anywhere" && key_of(p) == me)
                            .count();
                        result = if seen > 1 {
                            json!({"error": "duplicate"})
                        } else {
                            json!({"ok": 1})
                        };
                    }
                    if cmd == "status" {
                        sy.annotate(&player, &mut result);
                        // The whole queue (`status 0 N`): both of its library tracks.
                        if arg1 == "0" {
                            result["playlist_loop"] = json!([
                                {"id": 1, "title": "One", "playlist index": 0},
                                {"id": 2, "title": "Two", "playlist index": 1}
                            ]);
                        }
                    }
                    let reply = json!({"id": 1, "method": "slim.request", "result": result});
                    return ([("content-type", "application/json")], reply.to_string())
                        .into_response();
                }
                if let Some(id) = path
                    .strip_prefix("/music/")
                    .and_then(|p| p.strip_suffix("/download"))
                {
                    let file = id
                        .parse::<u64>()
                        .ok()
                        .and_then(|id| lib.lock().unwrap().get(&id).map(|(_, f)| f.clone()));
                    return match file {
                        Some(f) => ([("content-type", "audio/flac")], f).into_response(),
                        None => (axum::http::StatusCode::NOT_FOUND, "").into_response(),
                    };
                }
                if path.starts_with("/music/99999999/") {
                    return ([("content-type", "image/jpeg")], vec![0xffu8; HUGE_COVER])
                        .into_response();
                }
                if path.starts_with("/music/66666666/") {
                    return ([("content-type", "text/html")], "<script>alert(1)</script>")
                        .into_response();
                }
                if path.starts_with("/music/")
                    || path.starts_with("/html/images/")
                    || path.starts_with("/imageproxy/")
                {
                    return (
                        [("content-type", "image/jpeg")],
                        Body::from(b"IMAGEDATA".to_vec()),
                    )
                        .into_response();
                }
                ([("content-type", "text/plain")], "LMS internal page").into_response()
            }
        }));
        tokio::spawn(async move { axum::serve(http, app).await.unwrap() });

        let cli = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cli_addr = cli.local_addr().unwrap();
        let cli_clients: Arc<tokio::sync::Mutex<Vec<OwnedWriteHalf>>> = Default::default();
        let cc = cli_clients.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = cli.accept().await {
                let (mut rd, wr) = sock.into_split();
                cc.lock().await.push(wr);
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = rd.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                    }
                });
            }
        });

        // Slimproto: record what players send; tests write frames to the connections.
        let slim = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slim_addr = slim.local_addr().unwrap();
        let slim_frames: SlimFrames = Default::default();
        let slim_accepted = Arc::new(AtomicUsize::new(0));
        let (conn_tx, conn_rx) = tokio::sync::mpsc::unbounded_channel();
        let (frames, accepted) = (slim_frames.clone(), slim_accepted.clone());
        tokio::spawn(async move {
            while let Ok((sock, _)) = slim.accept().await {
                accepted.fetch_add(1, Ordering::SeqCst);
                let (mut rd, wr) = sock.into_split();
                let _ = conn_tx.send(wr);
                let frames = frames.clone();
                tokio::spawn(async move {
                    let mut reader = anywhere_core::proto::ClientFrameReader::default();
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = rd.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        reader.push(&buf[..n]);
                        while let Ok(Some((op, payload))) = reader.next_frame() {
                            frames.lock().unwrap().push((op, payload));
                        }
                    }
                });
            }
        });

        FakeLms {
            http_addr,
            cli_addr,
            slim_addr,
            rpc,
            http_paths,
            cli_clients,
            slim_frames,
            slim_accepted,
            slim_conns: tokio::sync::Mutex::new(conn_rx),
            streams,
            library,
            syncs,
        }
    }

    /// Lyrics in a library track's tags (LMS's `lyrics`, tag `w`).
    pub fn set_lyrics(&self, id: u64, text: &str) {
        if let Some((item, _)) = self.library.lock().unwrap().get_mut(&id) {
            item["lyrics"] = json!(text);
        }
    }

    /// A local library file the gateway may analyse (LMS type `kind`).
    pub fn add_track(&self, id: u64, kind: &str, file: Vec<u8>) {
        let item = json!({
            "id": id, "title": format!("Track {id}"), "type": kind,
            "url": format!("file:///srv/music/{id}.{kind}"), "filesize": file.len(),
            "modificationTime": "Tuesday, 1 January 2030",
        });
        self.library.lock().unwrap().insert(id, (item, file));
    }

    /// The next Slimproto connection from the gateway (5 s at most).
    pub async fn slim_conn(&self) -> OwnedWriteHalf {
        tokio::time::timeout(Duration::from_secs(5), self.slim_conns.lock().await.recv())
            .await
            .expect("the gateway never connected to LMS")
            .unwrap()
    }

    /// Opcodes received, as strings; STAT frames as their event (`STMs`).
    pub fn slim_events(&self) -> Vec<String> {
        self.slim_frames
            .lock()
            .unwrap()
            .iter()
            .map(|(op, p)| {
                if op == b"STAT" && p.len() >= 4 {
                    String::from_utf8_lossy(&p[..4]).into_owned()
                } else {
                    String::from_utf8_lossy(op).into_owned()
                }
            })
            .collect()
    }

    /// Payloads of every frame received with this opcode.
    pub fn slim_payloads(&self, op: &[u8; 4]) -> Vec<Vec<u8>> {
        self.slim_frames
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _)| o == op)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn stream_requests(&self) -> usize {
        self.http_paths()
            .iter()
            .filter(|p| *p == "/stream.mp3")
            .count()
    }

    /// Send a notification line to every CLI listener (the gateway's event feed).
    pub async fn cli_send(&self, line: &str) {
        for w in self.cli_clients.lock().await.iter_mut() {
            let _ = w.write_all(format!("{line}\n").as_bytes()).await;
        }
    }

    pub async fn cli_listeners(&self) -> usize {
        self.cli_clients.lock().await.len()
    }

    pub fn rpc_calls(&self) -> Vec<Value> {
        self.rpc.lock().unwrap().clone()
    }

    pub fn http_paths(&self) -> Vec<String> {
        self.http_paths.lock().unwrap().clone()
    }
}

/// A `/stream.mp3` answer: the body in 16 KiB pieces, pausing at the gate if there is one.
fn stream_response(plan: StreamPlan) -> axum::response::Response {
    use axum::body::Bytes;
    let StreamPlan { body, gate_at } = plan;
    let body = Arc::new(body);
    let stream = futures_util::stream::unfold(0usize, move |pos| {
        let (body, gate) = (body.clone(), gate_at.clone());
        async move {
            if pos >= body.len() {
                return None;
            }
            let mut end = (pos + 16 * 1024).min(body.len());
            if let Some((at, notify)) = &gate {
                if pos == *at {
                    notify.notified().await;
                } else if pos < *at {
                    end = end.min(*at);
                }
            }
            Some((
                Ok::<_, std::io::Error>(Bytes::copy_from_slice(&body[pos..end])),
                end,
            ))
        }
    });
    axum::response::Response::builder()
        .header("content-type", "audio/flac")
        .body(Body::from_stream(stream))
        .unwrap()
}

pub struct Harness {
    pub lms: FakeLms,
    pub addr: SocketAddr,
    pub accounts: Arc<Accounts>,
}

impl Harness {
    pub async fn start() -> Harness {
        Self::start_with(|_| {}).await
    }

    /// With track analysis on (a fresh cache directory) and `tweak` applied to the config.
    pub async fn start_with(tweak: impl FnOnce(&mut Config)) -> Harness {
        let lms = FakeLms::start().await;
        let cache = std::env::temp_dir().join(format!(
            "anywhere-mobile-an-{}",
            anywhere_gateway::random_id()
        ));
        let mut cfg = Config {
            lms_slimproto: lms.slim_addr.to_string(),
            lms_http: lms.http_addr.to_string(),
            lms_cli: lms.cli_addr.to_string(),
            lms_ips: vec![Ipv4Addr::LOCALHOST],
            auth_timeout: Duration::from_millis(500),
            analysis_cache_dir: Some(cache),
            plays_secret: Some(PLAYS_SECRET.into()),
            ..Config::default()
        };
        tweak(&mut cfg);
        let accounts = Arc::new(Accounts::in_memory().unwrap());
        accounts.create_user("alice", PW, Role::Listener).unwrap();
        accounts.create_user("bob", PW, Role::Listener).unwrap();
        let gw = Gateway::new(cfg, accounts.clone());
        let app = router(gw);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap()
        });
        Harness {
            lms,
            addr,
            accounts,
        }
    }

    pub fn server(&self) -> Arc<Server> {
        Server::new(format!("http://{}", self.addr)).unwrap()
    }

    pub fn session(&self, store: Arc<MemStore>) -> Arc<Session> {
        Session::new(self.server(), store, "Test phone".into()).unwrap()
    }

    /// A session for alice, signed in with her password.
    pub async fn alice(&self) -> (Arc<Session>, Arc<MemStore>) {
        let store = MemStore::new();
        let s = self.session(store.clone());
        s.login("alice".into(), PW.into(), None).await.unwrap();
        (s, store)
    }
}

/// An in-memory [`SecureStore`] that counts writes.
#[derive(Default)]
pub struct MemStore {
    pub creds: Mutex<Option<StoredCredentials>>,
    pub saves: AtomicUsize,
    pub clears: AtomicUsize,
    /// Fail every save, as a broken keystore would.
    pub fail_saves: std::sync::atomic::AtomicBool,
}

impl MemStore {
    pub fn new() -> Arc<MemStore> {
        Arc::new(MemStore::default())
    }

    pub fn with(creds: StoredCredentials) -> Arc<MemStore> {
        let s = MemStore::default();
        *s.creds.lock().unwrap() = Some(creds);
        Arc::new(s)
    }

    pub fn get(&self) -> Option<StoredCredentials> {
        self.creds.lock().unwrap().clone()
    }

    pub fn saves(&self) -> usize {
        self.saves.load(Ordering::SeqCst)
    }
}

impl SecureStore for MemStore {
    fn load(&self) -> Result<Option<StoredCredentials>, StoreError> {
        Ok(self.get())
    }
    fn save(&self, credentials: StoredCredentials) -> Result<(), StoreError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        if self.fail_saves.load(Ordering::SeqCst) {
            return Err(StoreError::Failed {
                detail: "keystore unavailable".into(),
            });
        }
        *self.creds.lock().unwrap() = Some(credentials);
        Ok(())
    }
    fn clear(&self) -> Result<(), StoreError> {
        self.clears.fetch_add(1, Ordering::SeqCst);
        *self.creds.lock().unwrap() = None;
        Ok(())
    }
}

/// Records sign-outs.
#[derive(Default)]
pub struct Observer {
    pub signed_out: Mutex<Vec<bool>>,
}

impl SessionObserver for Observer {
    fn on_signed_out(&self, revoked: bool) {
        self.signed_out.lock().unwrap().push(revoked);
    }
}

/// Collects events and states.
#[derive(Default)]
pub struct Collector {
    pub events: Mutex<Vec<ServerEvent>>,
    pub states: Mutex<Vec<EventsState>>,
}

impl EventListener for Collector {
    fn on_event(&self, event: ServerEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn on_state(&self, state: EventsState) {
        self.states.lock().unwrap().push(state);
    }
}

impl Collector {
    pub fn events(&self) -> Vec<ServerEvent> {
        self.events.lock().unwrap().clone()
    }
    pub fn states(&self) -> Vec<EventsState> {
        self.states.lock().unwrap().clone()
    }
    pub fn live_count(&self) -> usize {
        self.states()
            .iter()
            .filter(|s| **s == EventsState::Live)
            .count()
    }
}

/// Poll `cond` until it holds (10 s at most).
pub async fn wait_for(what: &str, cond: impl Fn() -> bool) {
    for _ in 0..1000 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A TCP proxy in front of the gateway whose connections can all be cut at once.
pub struct Proxy {
    pub addr: SocketAddr,
    cut: tokio::sync::broadcast::Sender<()>,
    freeze: tokio::sync::broadcast::Sender<()>,
    pub accepted: Arc<AtomicUsize>,
}

impl Proxy {
    pub async fn start(target: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (cut, _) = tokio::sync::broadcast::channel::<()>(4);
        let (freeze, _) = tokio::sync::broadcast::channel::<()>(4);
        let accepted = Arc::new(AtomicUsize::new(0));
        let (c, f, a) = (cut.clone(), freeze.clone(), accepted.clone());
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                a.fetch_add(1, Ordering::SeqCst);
                let mut stop = c.subscribe();
                let mut frozen = f.subscribe();
                tokio::spawn(async move {
                    let Ok(mut upstream) = tokio::net::TcpStream::connect(target).await else {
                        return;
                    };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {}
                        _ = stop.recv() => {}
                        _ = frozen.recv() => {
                            // Nothing more gets through, but both ends stay open, as when a
                            // phone's network changes under a connection.
                            let _ = stop.recv().await;
                            drop((client, upstream));
                        }
                    }
                });
            }
        });
        Proxy {
            addr,
            cut,
            freeze,
            accepted,
        }
    }

    /// Drop every open connection (as a phone losing its network would).
    pub fn cut_all(&self) {
        let _ = self.cut.send(());
    }

    /// Stop every open connection passing anything, without closing it (as when a phone's
    /// network changes under it). New connections still work.
    pub fn freeze_all(&self) {
        let _ = self.freeze.send(());
    }

    pub fn server(&self) -> Arc<Server> {
        Server::new(format!("http://{}", self.addr)).unwrap()
    }
}

/// Decode base32 (RFC 4648, no padding), for TOTP secrets.
pub fn base32_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u64, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("bad base32"),
        } as u64;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// The TOTP code `steps_ahead` steps from now.
pub fn totp_code(secret_b32: &str, steps_ahead: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let step = now / anywhere_gateway::totp::STEP_SECS + steps_ahead;
    format!(
        "{:06}",
        anywhere_gateway::totp::code_at(&base32_decode(secret_b32), step)
    )
}
