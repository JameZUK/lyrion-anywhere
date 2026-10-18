//! Shared test harness: a fake LMS that records everything it receives, and a gateway
//! with two users logged in.
#![allow(dead_code, unused_imports)]

pub mod flac;

pub use std::net::{Ipv4Addr, SocketAddr};
pub use std::sync::{Arc, Mutex};
pub use std::time::Duration;

pub use axum::Router;
pub use axum::body::Body;
pub use axum::extract::Request;
pub use axum::http::StatusCode;
pub use axum::routing::any;
pub use futures_util::{SinkExt, StreamExt};
pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub use tokio::net::tcp::OwnedWriteHalf;
pub use tokio::net::{TcpListener, TcpStream};
pub use tokio::sync::mpsc;
pub use tokio_tungstenite::tungstenite::Message;
pub use tokio_tungstenite::tungstenite::client::IntoClientRequest;
pub use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
pub use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
pub use tower::ServiceExt;

pub use anywhere_core::proto::{ClientFrameReader, encode_server_frame, format_mac};
pub use anywhere_gateway::accounts::{Accounts, Role};
pub use anywhere_gateway::{Config, Gateway, router};

pub const PW: &str = "correct horse battery";

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------------------
// Fake LMS
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Recorded {
    pub frames: Vec<[u8; 4]>,
    pub http_paths: Vec<String>,
    /// Slimproto connections the gateway has closed.
    pub closed: usize,
    /// STAT payloads received, in order.
    pub stats: Vec<Vec<u8>>,
    /// JSON-RPC `params` ([player, [cmd...]]) received, in order.
    pub rpc: Vec<serde_json::Value>,
    /// What clients sent to the CLI port.
    pub cli_in: Vec<String>,
    /// Every Slimproto frame received (opcode, payload), in order.
    pub payloads: Vec<([u8; 4], Vec<u8>)>,
    /// `/music/<id>/download` requests: (path, Range header).
    pub downloads: Vec<(String, Option<String>)>,
}

/// The library URL of fake track 1 (and of track 7, the same file renumbered by a rescan).
pub const TRACK_URL: &str = "file:///srv/music/Artist/Album/01%20Track.flac";

/// Fake library tracks for `titles … track_id:<id>`. `size` is the fake audio's length.
/// - 1: a local FLAC; 7: the same file under a new id (after a rescan)
/// - 2: a radio stream (remote)
/// - 3: a local file with hostile tags
/// - 4: a local file whose download never answers
/// - 5: a local file of a type that is never transcoded
/// - 6: a local file without a size, whose download ignores `Range`
/// - 8: a local file whose download stalls after the first bytes
/// - 9: a local file with synced (LRC) lyrics in its tags, some lines naming server paths
///
/// The fake Music & Artist Information plugin (`musicartistinfo lyrics`) has plain lyrics
/// for track 1 only.
///
/// Any other id is unknown (and its download hangs, as on a real LMS).
pub fn fake_track(id: u64, size: usize) -> Option<serde_json::Value> {
    use serde_json::json;
    let local = |url: &str, kind: &str| {
        json!({"id": id, "title": "Track", "artist": "Artist", "album": "Album", "album_id": "7",
               "tracknum": "3", "duration": "241.2", "type": kind, "url": url, "filesize": size,
               "modificationTime": "Tuesday, 1 January 2030"})
    };
    Some(match id {
        // Track 1 has a shared rating in LMS (Ratings Light or another app set it).
        1 => {
            let mut t = local(TRACK_URL, "flc");
            t["rating"] = json!("60");
            t
        }
        7 => local(TRACK_URL, "flc"),
        2 => json!({"id": 2, "title": "Radio", "type": "mp3", "remote": 1,
                    "url": "http://stream.example.com/live.mp3"}),
        3 => {
            let mut t = local("file:///srv/music/3.flac", "flc");
            t["title"] = json!("a\"b\r\nX-Evil: 1/../\u{202e}gpj.exe \u{e9}\u{1f3b5}");
            t["artist"] = json!("..\\..\\x/y");
            t["tracknum"] = json!(1);
            t
        }
        4 => local("file:///srv/music/4.flac", "flc"),
        5 => local("file:///srv/music/5.xyz", "xyz"),
        6 => {
            let mut t = local("file:///srv/music/6.flac", "flc");
            t.as_object_mut().unwrap().remove("filesize");
            t
        }
        8 => local("file:///srv/music/8.flac", "flc"),
        9 => {
            let mut t = local("file:///srv/music/9.flac", "flc");
            t["lyrics"] = json!(
                "[ar:Artist]\r\n[00:12.50]Second line\r\n[00:01.00]First line\r\n\
                                 [00:20.00]file:///srv/music/9.flac\r\n[00:21.00]/srv/music/Artist/9.lrc\r\n"
            );
            t
        }
        _ => return None,
    })
}

/// The fake `/music/<id>/download`: the audio with `Range` support (206), except as noted
/// in [`fake_track`].
fn fake_download(audio: &[u8], id: &str, range: Option<&str>) -> axum::response::Response {
    let hang = || {
        Body::from_stream(futures_util::stream::pending::<
            Result<bytes::Bytes, std::io::Error>,
        >())
    };
    match id {
        "1" | "3" | "5" | "7" => {}
        "6" => {
            return ([("content-type", "audio/flac")], Body::from(audio.to_vec()))
                .into_response_parts();
        }
        "8" => {
            let first = bytes::Bytes::copy_from_slice(&audio[..1000]);
            let body = futures_util::stream::iter([Ok::<_, std::io::Error>(first)])
                .chain(futures_util::stream::pending());
            return axum::response::Response::builder()
                .header("content-type", "audio/flac")
                .header("content-length", audio.len())
                .body(Body::from_stream(body))
                .unwrap();
        }
        // Unknown ids (and 4): LMS never answers.
        _ => {
            return axum::response::Response::builder()
                .header("content-type", "audio/flac")
                .body(hang())
                .unwrap();
        }
    }
    let total = audio.len();
    let span = range.and_then(|r| {
        let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
        let a: usize = a.parse().ok()?;
        // Like real LMS (Perl): an end of 0 reads as "no end", so `bytes=0-0` gets the
        // whole file back (labelled as such in Content-Range).
        let b: usize = if b.is_empty() || b == "0" {
            total - 1
        } else {
            b.parse().ok()?
        };
        Some((a, b.min(total - 1)))
    });
    match span {
        Some((a, _)) if a >= total => axum::response::Response::builder()
            .status(416)
            .header("content-range", format!("bytes */{total}"))
            .body(Body::empty())
            .unwrap(),
        Some((a, b)) => axum::response::Response::builder()
            .status(206)
            .header("content-type", "audio/flac")
            .header("content-range", format!("bytes {a}-{b}/{total}"))
            .body(Body::from(audio[a..=b].to_vec()))
            .unwrap(),
        None => {
            ([("content-type", "audio/flac")], Body::from(audio.to_vec())).into_response_parts()
        }
    }
}

/// The fake LMS's alarms for any player: one alarm, and these sounds (LMS's own URLs, which
/// must never reach clients). `alarm add` answers a new id.
pub fn fake_alarm(words: &[serde_json::Value]) -> serde_json::Value {
    use serde_json::json;
    let w = |i: usize| words.get(i).and_then(|x| x.as_str()).unwrap_or("");
    match (w(0), w(1)) {
        ("alarm", "playlists") => json!({"count": 4, "item_loop": [
            {"category": "CURRENT_PLAYLIST", "title": "{ALARM_USE_CURRENT_PLAYLIST}"},
            {"category": "FAVORITES", "title": "Morning radio", "url": "http://radio.example.com/stream?token=secret"},
            {"category": "PLAYLISTS", "title": "Wake up", "url": "file:///srv/music/playlists/wake.m3u"},
            {"category": "RANDOM", "title": "Random songs", "url": "randomplay://track"}
        ]}),
        ("alarms", _) => json!({"fade": "1", "count": 1, "alarms_loop": [
            {"id": "1a2b3c4d", "dow": "1,2,3,4,5", "enabled": "1", "repeat": "1", "time": "25200",
             "volume": "40", "shufflemode": "0", "url": "file:///srv/music/playlists/wake.m3u"}
        ]}),
        ("alarm", "add") => json!({"id": "5e6f7a8b"}),
        ("playerpref", _) => json!({"_p2": "1"}),
        _ => json!({}),
    }
}

/// The fake Music & Artist Information plugin's `biography`, `artistphoto` and
/// `albumreview` (`None`: it never answers). All names are invented.
/// - artist 1: an HTML biography (with a script and a line naming a server path) and a
///   photo on an external host;
/// - artist 2: a plain biography, no photo;
/// - artist 3: no biography, and a photo on the home network (never handed out);
/// - artist 5: a biography, but the photo lookup never answers;
/// - artists and albums 900 and up: never answer;
/// - album 1: an HTML review; album 2: an empty one;
/// - anything else: the plugin's "nothing found".
pub fn fake_mai(words: &[serde_json::Value]) -> Option<serde_json::Value> {
    use serde_json::json;
    let what = words.get(1)?.as_str()?;
    let param = words.get(2).and_then(|w| w.as_str()).unwrap_or("");
    let (kind, id) = param.split_once(':')?;
    let id: u64 = id.parse().ok()?;
    if id >= 900 {
        return None;
    }
    Some(match (what, kind, id) {
        ("biography", "artist_id", 1) => json!({
            "artist_id": "1", "artist": "The Invented Band",
            "biography": "<p>The Invented Band formed in 1990 &amp; played <b>everywhere</b>.</p>\
                          <script>alert('x')</script>\
                          <p>Local notes: /srv/music/The Invented Band/bio.txt</p>\
                          <p>Second paragraph.<br>Last line.</p>"
        }),
        ("biography", "artist_id", 2 | 5) => json!({
            "artist_id": id.to_string(), "artist": "Example Singer",
            "biography": "Plain words\r\n\r\nabout a singer."
        }),
        ("artistphoto", "artist_id", 1) => json!({
            "artist_id": "1", "url": "https://images.example.com/artist/1.jpg",
            "credits": "Discogs", "width": "600", "height": "600"
        }),
        ("artistphoto", "artist_id", 3) => json!({
            "artist_id": "3", "url": "http://10.0.0.10:9000/plugins/x/artist.jpg",
            "credits": "Local"
        }),
        ("artistphoto", "artist_id", 5) => return None,
        ("albumreview", "album_id", 1) => json!({
            "album_id": "1", "album": "Invented Album", "artist": "The Invented Band",
            "albumreview": "<div><i>A fine record</i> &mdash; five stars.<style>p{}</style></div>"
        }),
        ("albumreview", "album_id", 2) => json!({"album_id": "2", "albumreview": ""}),
        _ => json!({"error": "No information found"}),
    })
}

/// The fake LMS's `artists 0 1 artist_id:<id>` (`None`: it never answers). As a real
/// LMS, a list of ids finds nothing. All names are invented.
/// - 13: a name naming a server path (never handed out);
/// - 14: a name with invisible formatting characters and odd spacing;
/// - 15: answers with another artist;
/// - 16: no such artist;
/// - 900 and up: never answers;
/// - anything else: "Invented Artist <id>".
pub fn fake_artist(words: &[serde_json::Value]) -> Option<serde_json::Value> {
    use serde_json::json;
    let ids = words
        .iter()
        .find_map(|w| w.as_str()?.strip_prefix("artist_id:"))?;
    let Ok(id) = ids.parse::<u64>() else {
        return Some(json!({"count": 0}));
    };
    let one =
        |id: u64, name: &str| json!({"count": 1, "artists_loop": [{"id": id, "artist": name}]});
    Some(match id {
        900.. => return None,
        13 => one(13, "Band /srv/music/Band/notes.txt"),
        14 => one(14, "  Odd\u{202e}ly \u{200b}Spaced\t Band "),
        15 => one(99, "Somebody Else"),
        16 => json!({"count": 0}),
        _ => one(id, &format!("Invented Artist {id}")),
    })
}

/// Canned LMS JSON-RPC results, deliberately full of things that must not leak.
pub fn fake_rpc_result(cmd: &str) -> serde_json::Value {
    use serde_json::json;
    match cmd {
        "serverstatus" | "players" => json!({
            "version": "9.1.1", "uuid": "server-uuid-secret", "ip": "10.0.0.10", "httpport": "9000",
            "player count": 3, "other player count": 1,
            "players_loop": [
                {"playerid": "aa:bb:cc:dd:ee:01", "name": "Home One", "ip": "10.0.0.31:4000"},
                {"playerid": "aa:bb:cc:dd:ee:02", "name": "Home Two", "ip": "10.0.0.32:4000"},
                {"playerid": "02:4c:41:aa:aa:aa", "name": "Someone's phone", "ip": "10.0.0.33:4000"}
            ]
        }),
        "albums" => {
            json!({"count": 1, "albums_loop": [{"id": 7, "album": "Album", "url": "file:///srv/music/x"}]})
        }
        "titles" => {
            json!({"count": 1, "titles_loop": [{"id": 1, "title": "T", "url": "file:///srv/music/x/1.flac"}]})
        }
        // The music folder: every entry carries its file URL, i.e. its path on the server.
        "musicfolder" => json!({"count": 3, "folder_loop": [
            {"id": 11, "filename": "Jazz", "type": "folder", "url": "file:///srv/music/Jazz"},
            {"id": 12, "filename": "01 Song.flac", "type": "track", "coverid": "c0ffee12",
             "url": "file:///srv/music/Jazz/01%20Song.flac", "title": "Song"},
            {"id": 13, "filename": "/srv/music/odd.mp3", "type": "track",
             "url": "smb://10.0.0.10/music/odd.mp3", "title": "Odd"}
        ]}),
        "genres" => json!({"count": 1, "genres_loop": [
            {"id": 3, "genre": "Jazz", "favorites_url": "db:genre.name=Jazz"}]}),
        "libraries" => json!({"folder_loop": [{"id": "1a2b3c4d", "name": "Lossless (12)"}]}),
        "apps" => json!({"count": 1, "appss_loop": [{"cmd": "spotty", "name": "Spotty"}]}),
        "radios" => json!({"count": 1, "radioss_loop": [{"cmd": "bbcsounds", "name": "BBC"}]}),
        "favorites" => json!({"count": 1, "loop_loop": [
            {"id": "f.1", "name": "Station", "image": "https://cdn.example.com/a.jpg", "url": "http://stream.example/x?token=secret"}
        ]}),
        // A context menu, shaped like LMS's track info menu (SlimBrowse).
        "trackinfo" => json!({"count": 7, "base": {"actions": {}}, "item_loop": [
            {"text": "Similar tracks", "type": "redirect", "actions": {"go": {"cmd": ["audiomuse", "similar_track"], "params": {"track_id": "9"}, "player": 0}}},
            {"text": "Save to Favorites", "actions": {"go": {"cmd": ["jivefavorites", "add"], "params": {"url": "file:///srv/music/x/1.flac", "title": "T"}}}},
            {"text": "Server setting", "actions": {"go": {"cmd": ["pref", "audiodir"], "params": {"value": "/x"}}}},
            {"text": "Stream link", "actions": {"go": {"cmd": ["remote", "items"], "params": {"item_id": "http://10.0.0.50/x"}}}},
            {"text": "Location: /srv/music/x/1.flac", "style": "itemNoAction"},
            {"text": "URL: http://10.0.0.10:8200/x", "style": "itemNoAction"},
            {"text": "Artist: AC/DC", "style": "itemNoAction"}
        ]}),
        // Free text LMS relays from the library and players: user-controlled.
        "tracks" => json!({"count": 1, "titles_loop": [
            {"id": 2, "title": "imageproxy/http%3A%2F%2F10.0.0.1%2Fx/image.png",
             "artist": "x /imageproxy/http%3A%2F%2F10.0.0.1%2Fy/image.png",
             "remote_title": "Kitchen imageproxy/http%3A%2F%2F10.0.0.1%2Fz/image.png"}
        ]}),
        // Sync groups name every player, permitted or not.
        "syncgroups" => json!({"syncgroups_loop": [
            {"sync_members": "aa:bb:cc:dd:ee:01,aa:bb:cc:dd:ee:02", "sync_member_names": "Home One,Home Two"}
        ]}),
        "sync" => json!({"_sync": "aa:bb:cc:dd:ee:01,aa:bb:cc:dd:ee:02"}),
        // LMS's status leaving out a queued track it can't resolve: the next track is listed
        // as current, though the player's duration is the playing track's (status_fix.rs).
        "status" => json!({"mode": "play", "playlist_cur_index": "1", "duration": 300.5,
            "playlist_loop": [{"playlist index": 1, "id": 3, "title": "Next Track", "duration": 95.25}]}),
        "playlist" => json!({"_path": "file:///srv/music/Playing%20Track.flac"}),
        "songinfo" => json!({"count": 6, "songinfo_loop": [
            {"id": 4}, {"title": "Playing Track"}, {"duration": 300.5}, {"coverid": "c0ffee"},
            {"url": "file:///srv/music/Playing%20Track.flac"}, {"comment": "ripped to /srv/music/x"}]}),
        // The home menu: a plugin entry in My Music, and a settings entry.
        "menu" => json!({"count": 3, "item_loop": [
            {"id": "pluginSomePlugin", "node": "myMusic", "text": "Some Plugin", "actions": {"go": {"cmd": ["someplugin", "menu"], "player": 0}}},
            {"id": "somegroup", "node": "myMusic", "text": "Some Group", "isANode": 1},
            {"id": "somegroupa", "node": "somegroup", "text": "Mix A", "nextWindow": "nowPlaying", "actions": {"go": {"cmd": ["groupmix", "a"]}}},
            {"id": "somegroupb", "node": "somegroup", "text": "Choose", "actions": {"go": {"cmd": ["groupchooser"]}}},
            {"id": "settingsXfade", "node": "settingsAudio", "actions": {"go": {"cmd": ["crossfadesettings"], "player": 0}}},
            {"id": "playerpower", "node": "home", "actions": {"do": {"cmd": ["power", "0"]}}}
        ]}),
        // A plugin's own menu: a search box (whatever the user types goes to the plugin).
        "someplugin" => json!({"count": 1, "item_loop": [
            {"text": "Search", "type": "search", "actions": {"go": {"cmd": ["someplugin", "items"],
             "params": {"search": "__TAGGEDINPUT__", "menu": "someplugin"}}}}
        ]}),
        // Don't Stop The Music's settings menu for a player: off, with three providers.
        "dontstopthemusicsetting" => json!({"count": 4, "item_loop": [
            {"text": "Disabled", "radio": 1, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", 0]}}},
            {"text": "AudioMuse-AI similar", "radio": 0, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", "PLUGIN_AUDIOMUSEAI_DSTM"]}}},
            {"text": "Random songs", "radio": 0, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", "PLUGIN_RANDOM_TRACK"]}}},
            {"text": "Similar tracks", "radio": 0, "actions": {"do": {"player": 0, "cmd": ["playerpref", "plugin.dontstopthemusic:provider", "PLUGIN_DSTM_SIMILAR"]}}}
        ]}),
        _ => json!({"ok": 1}),
    }
}

/// An album in the fake library used by `albums` queries (see [`FakeLms::set_albums`]).
#[derive(Clone, Debug)]
pub struct FakeAlbum {
    pub id: u64,
    pub title: &'static str,
    pub artist: &'static str,
    pub year: u32,
    pub genres: Vec<u64>,
    /// (artist id, role) contributions.
    pub credits: Vec<(u64, &'static str)>,
    pub compilation: bool,
}

/// The fake LMS's answer to an `albums` query over `library`: the filters LMS applies
/// (`genre_id` lists, `artist_id` with an optional `role_id`, `compilation`, `year`), its
/// sorts and paging. `random` is LMS's choice: reversed id order here, so tests can tell.
pub fn fake_albums(library: &[FakeAlbum], words: &[serde_json::Value]) -> serde_json::Value {
    let arg = |name: &str| {
        words
            .iter()
            .filter_map(|w| w.as_str()?.strip_prefix(&format!("{name}:")))
            .next()
            .map(str::to_string)
    };
    let num = |i: usize| -> usize {
        words
            .get(i)
            .and_then(|w| {
                w.as_str()
                    .map(str::to_string)
                    .or_else(|| w.as_u64().map(|n| n.to_string()))
            })
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    let genres: Option<Vec<u64>> =
        arg("genre_id").map(|g| g.split(',').filter_map(|x| x.parse().ok()).collect());
    let artist: Option<u64> = arg("artist_id").and_then(|a| a.parse().ok());
    let role = arg("role_id");
    let compilation = arg("compilation");
    let year: Option<u32> = arg("year").and_then(|y| y.parse().ok());
    let mut hits: Vec<&FakeAlbum> = library
        .iter()
        .filter(|a| {
            genres
                .as_ref()
                .is_none_or(|g| a.genres.iter().any(|x| g.contains(x)))
        })
        .filter(|a| {
            artist.is_none_or(|id| {
                a.credits
                    .iter()
                    .any(|(c, r)| *c == id && role.as_deref().is_none_or(|want| want == *r))
            })
        })
        .filter(|a| match compilation.as_deref() {
            Some("1") => a.compilation,
            Some("0") => !a.compilation,
            _ => true,
        })
        .filter(|a| year.is_none_or(|y| a.year == y))
        .collect();
    match arg("sort").as_deref() {
        Some("artistalbum") => hits.sort_by_key(|a| (a.artist, a.title)),
        Some("yearalbum") => hits.sort_by_key(|a| (a.year, a.title)),
        Some("new") => hits.sort_by_key(|a| std::cmp::Reverse(a.id)),
        Some("random") => hits.sort_by_key(|a| std::cmp::Reverse(a.id)),
        _ => hits.sort_by_key(|a| a.title),
    }
    let count = hits.len();
    let page: Vec<serde_json::Value> = hits
        .into_iter()
        .skip(num(1))
        .take(num(2))
        .map(|a| {
            serde_json::json!({"id": a.id, "album": a.title, "artist": a.artist, "year": a.year,
                "url": "file:///srv/music/secret"})
        })
        .collect();
    serde_json::json!({"count": count, "albums_loop": page})
}

pub struct FakeLms {
    /// When set, `albums` queries are answered from this library ([`fake_albums`]).
    pub albums: Arc<Mutex<Option<Vec<FakeAlbum>>>>,
    pub slim_addr: SocketAddr,
    pub http_addr: SocketAddr,
    /// Fake CLI (notifications): lines sent with `cli_send` go to every listener.
    pub cli_addr: SocketAddr,
    pub cli_clients: Arc<tokio::sync::Mutex<Vec<OwnedWriteHalf>>>,
    pub rec: Arc<Mutex<Recorded>>,
    pub conns: mpsc::Receiver<OwnedWriteHalf>,
    pub audio: Arc<Vec<u8>>,
    /// When set, `/stream.mp3` sends its first 1000 bytes and then stalls for ever.
    pub stall_stream: Arc<std::sync::atomic::AtomicBool>,
    /// While set, JSON-RPC requests get no answer (as when LMS is blocked on slow storage);
    /// they are answered once it is cleared.
    pub freeze_rpc: Arc<std::sync::atomic::AtomicBool>,
    /// When set, `/stream.mp3` sends this instead of [`FakeLms::audio`].
    pub stream_body: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    /// Tracks added by a test (see [`FakeLms::add_track`]): id → (titles item, file).
    pub extra: ExtraTracks,
    /// The plugin's `anywhere play_record`.
    pub plugin: Arc<Mutex<FakePlugin>>,
    /// When set, `status` says every player is playing this track id.
    pub status_track: Arc<Mutex<Option<u64>>>,
    /// When set, saved playlists and `playlist save` are answered from this
    /// ([`FakeLms::set_saved_playlists`]).
    pub saved: Arc<Mutex<Option<FakeSaved>>>,
    /// JSON-RPC commands (by first word) that are recorded but never answered, as when a
    /// plugin hangs inside LMS.
    pub hang_rpc: Arc<Mutex<std::collections::HashSet<String>>>,
}

/// The fake LMS's saved playlists, and the length of every player's queue.
#[derive(Clone, Debug, Default)]
pub struct FakeSaved {
    /// (id, name), as `playlists` lists them.
    pub playlists: Vec<(u64, String)>,
    /// What `status` reports as `playlist_tracks`.
    pub queue_len: u64,
}

/// The fake LMS's answer to `playlists …` and `playlist save …` and `status …` when saved
/// playlists are set: `search:` matches case-insensitively anywhere in the name (LMS's
/// `LIKE`); `playlist save` overwrites a playlist of exactly that name (a case-sensitive
/// filesystem) or adds one, and answers with its id, as LMS does.
fn fake_saved(
    saved: &Mutex<Option<FakeSaved>>,
    words: &[serde_json::Value],
) -> Option<serde_json::Value> {
    use serde_json::json;
    let mut guard = saved.lock().unwrap();
    let s = guard.as_mut()?;
    let w = |i: usize| words.get(i).and_then(|v| v.as_str());
    match (w(0), w(1)) {
        (Some("playlists"), Some(n)) if n.parse::<u64>().is_ok() => {
            let search = words
                .iter()
                .filter_map(|v| v.as_str()?.strip_prefix("search:"))
                .next()
                .map(str::to_lowercase);
            let found: Vec<serde_json::Value> = s
                .playlists
                .iter()
                .filter(|(_, name)| search.as_ref().is_none_or(|t| name.to_lowercase().contains(t)))
                .map(|(id, name)| json!({"id": id, "playlist": name, "url": format!("file:///srv/playlists/{name}.m3u")}))
                .collect();
            Some(json!({"count": found.len(), "playlists_loop": found}))
        }
        (Some("playlist"), Some("save")) => {
            let name = w(2)?.to_string();
            let id = match s.playlists.iter().find(|(_, n)| *n == name) {
                Some((id, _)) => *id,
                None => {
                    let id = 100 + s.playlists.len() as u64;
                    s.playlists.push((id, name));
                    id
                }
            };
            Some(json!({"__playlist_id": id}))
        }
        (Some("status"), _) => Some(json!({"mode": "stop", "playlist_tracks": s.queue_len})),
        _ => None,
    }
}

/// The secret the test gateway signs plays with (`Config::plays_secret`).
pub const PLAYS_SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How the fake plugin's `anywhere play_record` behaves.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginMode {
    /// Check the signature and record the play (remembering keys, like the plugin).
    Record,
    /// Answer with this result.
    Answer(serde_json::Value),
    /// Never answer.
    Hang,
}

/// The fake plugin: what it recorded, as (track id, played_at, key).
pub struct FakePlugin {
    pub mode: PluginMode,
    pub recorded: Vec<(u64, u64, String)>,
    pub calls: usize,
}

impl Default for FakePlugin {
    fn default() -> Self {
        FakePlugin {
            mode: PluginMode::Record,
            recorded: vec![],
            calls: 0,
        }
    }
}

/// The fake plugin's handling of one `anywhere play_record` (see the real one in
/// plugin/Plugins/LyrionAnywhere/Commands.pm).
fn fake_play_record(
    plugin: &Mutex<FakePlugin>,
    args: &[serde_json::Value],
) -> Option<serde_json::Value> {
    use serde_json::json;
    let mut pl = plugin.lock().unwrap();
    pl.calls += 1;
    match pl.mode.clone() {
        PluginMode::Hang => return None,
        PluginMode::Answer(v) => return Some(v),
        PluginMode::Record => {}
    }
    let arg = |name: &str| {
        args.iter()
            .filter_map(|a| a.as_str()?.strip_prefix(&format!("{name}:")))
            .next()
            .map(str::to_string)
    };
    let (Some(id), Some(at), Some(key), Some(ts), Some(sig)) = (
        arg("track_id").and_then(|v| v.parse::<u64>().ok()),
        arg("played_at").and_then(|v| v.parse::<u64>().ok()),
        arg("key"),
        arg("ts").and_then(|v| v.parse::<u64>().ok()),
        arg("sig"),
    ) else {
        return Some(json!({"error": "invalid"}));
    };
    if anywhere_gateway::plays::signature(PLAYS_SECRET, id, at, &key, ts) != sig {
        return Some(json!({"error": "refused: bad signature"}));
    }
    if pl.recorded.iter().any(|(_, _, k)| *k == key) {
        return Some(json!({"error": "duplicate"}));
    }
    pl.recorded.push((id, at, key));
    Some(json!({"ok": 1}))
}

pub type ExtraTracks = Arc<Mutex<std::collections::HashMap<u64, (serde_json::Value, Vec<u8>)>>>;

impl FakeLms {
    pub async fn start() -> FakeLms {
        let rec = Arc::new(Mutex::new(Recorded::default()));
        let audio = Arc::new(b"fLaCAUDIO".repeat(20_000));

        let slim = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slim_addr = slim.local_addr().unwrap();
        let (tx, conns) = mpsc::channel(4);
        let r = rec.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = slim.accept().await {
                let (mut rd, wr) = sock.into_split();
                let _ = tx.send(wr).await;
                let r = r.clone();
                tokio::spawn(async move {
                    let mut reader = ClientFrameReader::default();
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = rd.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        reader.push(&buf[..n]);
                        while let Ok(Some((op, payload))) = reader.next_frame() {
                            let mut rec = r.lock().unwrap();
                            rec.frames.push(op);
                            rec.payloads.push((op, payload.clone()));
                            if &op == b"STAT" {
                                rec.stats.push(payload);
                            }
                        }
                    }
                    r.lock().unwrap().closed += 1;
                });
            }
        });

        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_addr = http.local_addr().unwrap();
        let stall_stream: Arc<std::sync::atomic::AtomicBool> = Default::default();
        let freeze_rpc: Arc<std::sync::atomic::AtomicBool> = Default::default();
        let frozen = freeze_rpc.clone();
        let extra: ExtraTracks = Default::default();
        let plugin: Arc<Mutex<FakePlugin>> = Default::default();
        let stream_body: Arc<Mutex<Option<Arc<Vec<u8>>>>> = Default::default();
        let sb = stream_body.clone();
        let status_track: Arc<Mutex<Option<u64>>> = Default::default();
        let st_track = status_track.clone();
        let albums: Arc<Mutex<Option<Vec<FakeAlbum>>>> = Default::default();
        let lib = albums.clone();
        let saved: Arc<Mutex<Option<FakeSaved>>> = Default::default();
        let sv = saved.clone();
        let hang_rpc: Arc<Mutex<std::collections::HashSet<String>>> = Default::default();
        let hung = hang_rpc.clone();
        let (r, a, stall, ex, pl) = (
            rec.clone(),
            audio.clone(),
            stall_stream.clone(),
            extra.clone(),
            plugin.clone(),
        );
        let app = Router::new().fallback(any(move |req: Request| {
            let (r, stall, ex, pl) = (r.clone(), stall.clone(), ex.clone(), pl.clone());
            let frozen = frozen.clone();
            let sv = sv.clone();
            let hung = hung.clone();
            let playing = *st_track.lock().unwrap();
            let library = lib.lock().unwrap().clone();
            let a = sb.lock().unwrap().clone().unwrap_or_else(|| a.clone());
            async move {
                let path = req
                    .uri()
                    .path_and_query()
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                r.lock().unwrap().http_paths.push(path);
                if req.uri().path() == "/jsonrpc.js" {
                    while frozen.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                        .await
                        .unwrap_or_default();
                    let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    let params = v["params"].clone();
                    let cmd = params[1][0].as_str().unwrap_or("").to_string();
                    let track_id = params[1]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str()?.strip_prefix("track_id:"))
                        .next()
                        .map(|id| id.parse::<u64>().unwrap_or(0));
                    // `titles … track_id:1,2,3`: several tracks at once.
                    let many: Option<Vec<u64>> = params[1]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str()?.strip_prefix("track_id:"))
                        .find(|ids| ids.contains(','))
                        .map(|ids| ids.split(',').filter_map(|i| i.parse().ok()).collect());
                    r.lock().unwrap().rpc.push(params.clone());
                    if hung.lock().unwrap().contains(&cmd) {
                        futures_util::future::pending::<()>().await;
                    }
                    let added = track_id.and_then(|id| ex.lock().unwrap().get(&id).cloned());
                    let words = params[1].as_array().cloned().unwrap_or_default();
                    let plugin_answer = if cmd == "anywhere"
                        && words.get(1).and_then(|w| w.as_str()) == Some("play_record")
                    {
                        match fake_play_record(&pl, &words[2..]) {
                            Some(v) => Some(v),
                            None => {
                                futures_util::future::pending::<()>().await;
                                None
                            }
                        }
                    } else {
                        None
                    };
                    let saved_answer = fake_saved(&sv, &words);
                    let result = match track_id {
                        _ if plugin_answer.is_some() => plugin_answer.unwrap(),
                        _ if saved_answer.is_some() => saved_answer.unwrap(),
                        _ if cmd == "titles" && many.is_some() => {
                            let loop_: Vec<serde_json::Value> = many
                                .unwrap()
                                .into_iter()
                                .filter_map(|id| {
                                    fake_track(id, a.len()).or_else(|| {
                                        ex.lock().unwrap().get(&id).map(|e| e.0.clone())
                                    })
                                })
                                .collect();
                            serde_json::json!({"count": loop_.len(), "titles_loop": loop_})
                        }
                        Some(_) if cmd == "titles" && added.is_some() => {
                            serde_json::json!({"count": 1, "titles_loop": [added.unwrap().0]})
                        }
                        _ if cmd == "alarm" || cmd == "alarms" || (cmd == "playerpref" && words.get(1).and_then(|w| w.as_str()) == Some("alarmsEnabled")) => {
                            fake_alarm(&words)
                        }
                        _ if cmd == "musicartistinfo"
                            && matches!(
                                words.get(1).and_then(|w| w.as_str()),
                                Some("biography" | "albumreview" | "artistphoto")
                            ) =>
                        {
                            match fake_mai(&words) {
                                Some(v) => v,
                                None => {
                                    futures_util::future::pending::<()>().await;
                                    serde_json::Value::Null
                                }
                            }
                        }
                        Some(id) if cmd == "musicartistinfo" => {
                            if id == 1 && words.len() == 3 {
                                serde_json::json!({"lyrics": "Plain words\nfrom the plugin", "title": "Track"})
                            } else {
                                serde_json::json!({"error": "Not found"})
                            }
                        }
                        Some(id) if cmd == "titles" => match fake_track(id, a.len()) {
                            Some(t) => serde_json::json!({"count": 1, "titles_loop": [t]}),
                            None => serde_json::json!({"count": 0}),
                        },
                        _ if cmd == "albums" && library.is_some() => {
                            fake_albums(library.as_deref().unwrap(), &words)
                        }
                        _ if cmd == "status" && playing.is_some() => serde_json::json!({
                            "mode": "play",
                            "playlist_loop": [{"playlist index": 0, "id": playing.unwrap()}]
                        }),
                        _ if cmd == "artists"
                            && words.iter().any(|w| {
                                w.as_str().is_some_and(|s| s.starts_with("artist_id:"))
                            }) =>
                        {
                            match fake_artist(&words) {
                                Some(v) => v,
                                None => {
                                    futures_util::future::pending::<()>().await;
                                    serde_json::Value::Null
                                }
                            }
                        }
                        _ => fake_rpc_result(&cmd),
                    };
                    let reply =
                        serde_json::json!({"id": 1, "method": "slim.request", "result": result});
                    return (
                        [("content-type", "application/json")],
                        Body::from(reply.to_string()),
                    )
                        .into_response_parts();
                }
                let p = req.uri().path().to_string();
                if let Some(id) = p
                    .strip_prefix("/music/")
                    .and_then(|x| x.strip_suffix("/download"))
                {
                    let range = req
                        .headers()
                        .get("range")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    r.lock().unwrap().downloads.push((p.clone(), range.clone()));
                    let added = id
                        .parse::<u64>()
                        .ok()
                        .and_then(|n| ex.lock().unwrap().get(&n).cloned());
                    if let Some((_, file)) = added {
                        return fake_download(&file, "1", range.as_deref());
                    }
                    if !matches!(id, "1" | "3" | "5" | "6" | "7" | "8") {
                        // Unknown ids (and 4): a real LMS never answers at all.
                        futures_util::future::pending::<()>().await;
                    }
                    return fake_download(&a, id, range.as_deref());
                }
                if p.starts_with("/music/666/") {
                    // A "cover" that isn't an image: must not be relayed.
                    return (
                        [("content-type", "text/html")],
                        Body::from("<script>alert(1)</script>"),
                    )
                        .into_response_parts();
                }
                if p.starts_with("/music/")
                    || p.starts_with("/imageproxy/")
                    || p.starts_with("/html/images/")
                    || p.starts_with("/plugins/")
                {
                    let ct = if p.ends_with(".svg") {
                        "image/svg+xml"
                    } else {
                        "image/jpeg"
                    };
                    return ([("content-type", ct)], Body::from(b"IMAGEDATA".to_vec()))
                        .into_response_parts();
                }
                if req.uri().path() == "/stream.mp3"
                    && stall.load(std::sync::atomic::Ordering::SeqCst)
                {
                    let first = bytes::Bytes::copy_from_slice(&a[..1000]);
                    let body = futures_util::stream::iter([Ok::<_, std::io::Error>(first)])
                        .chain(futures_util::stream::pending());
                    return ([("content-type", "audio/flac")], Body::from_stream(body))
                        .into_response_parts();
                }
                if req.uri().path() == "/stream.mp3" {
                    (
                        [("content-type", "audio/flac")],
                        Body::from(a.as_ref().clone()),
                    )
                        .into_response_parts()
                } else {
                    (
                        [("content-type", "text/plain")],
                        Body::from("LMS internal page"),
                    )
                        .into_response_parts()
                }
            }
        }));
        tokio::spawn(async move { axum::serve(http, app).await.unwrap() });

        let cli = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cli_addr = cli.local_addr().unwrap();
        let cli_clients: Arc<tokio::sync::Mutex<Vec<OwnedWriteHalf>>> = Default::default();
        let (cc, r) = (cli_clients.clone(), rec.clone());
        tokio::spawn(async move {
            while let Ok((sock, _)) = cli.accept().await {
                let (mut rd, wr) = sock.into_split();
                cc.lock().await.push(wr);
                let r = r.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = rd.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        r.lock()
                            .unwrap()
                            .cli_in
                            .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                    }
                });
            }
        });

        FakeLms {
            albums,
            slim_addr,
            http_addr,
            cli_addr,
            cli_clients,
            rec,
            conns,
            audio,
            stall_stream,
            freeze_rpc,
            stream_body,
            extra,
            plugin,
            status_track,
            saved,
            hang_rpc,
        }
    }

    /// Answer saved-playlist queries, `playlist save` and `status` from this from now on.
    pub fn set_saved_playlists(&self, saved: FakeSaved) {
        *self.saved.lock().unwrap() = Some(saved);
    }

    /// The saved playlists now.
    pub fn saved_playlists(&self) -> Vec<(u64, String)> {
        self.saved
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.playlists.clone())
            .unwrap_or_default()
    }

    /// Answer `albums` queries from this library from now on.
    pub fn set_albums(&self, library: Vec<FakeAlbum>) {
        *self.albums.lock().unwrap() = Some(library);
    }

    /// Add a local library track of LMS type `kind` whose file is `file`. Its URL is
    /// `file:///srv/music/added/<id>.<kind>`; `duration` as LMS reports it.
    pub fn add_track(&self, id: u64, kind: &str, file: Vec<u8>, duration: f64) {
        let item = serde_json::json!({
            "id": id, "title": format!("Added {id}"), "artist": "Artist", "album": "Album",
            "type": kind, "url": format!("file:///srv/music/added/{id}.{kind}"),
            "filesize": file.len(), "duration": duration.to_string(),
            "modificationTime": "Tuesday, 1 January 2030",
        });
        self.extra.lock().unwrap().insert(id, (item, file));
    }

    pub async fn accept(&mut self) -> OwnedWriteHalf {
        tokio::time::timeout(Duration::from_secs(3), self.conns.recv())
            .await
            .expect("gateway never connected to LMS")
            .unwrap()
    }

    pub async fn assert_no_conn(&mut self) {
        let r = tokio::time::timeout(Duration::from_millis(150), self.conns.recv()).await;
        assert!(r.is_err(), "gateway connected to LMS but should not have");
    }

    pub fn frames(&self) -> Vec<String> {
        self.rec
            .lock()
            .unwrap()
            .frames
            .iter()
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect()
    }

    pub fn stats(&self) -> Vec<anywhere_core::proto::Stat> {
        self.rec
            .lock()
            .unwrap()
            .stats
            .iter()
            .map(|p| anywhere_core::proto::Stat::parse(p).unwrap())
            .collect()
    }

    /// Send a notification line to every CLI listener.
    pub async fn cli_send(&self, line: &str) {
        for w in self.cli_clients.lock().await.iter_mut() {
            let _ = w.write_all(format!("{line}\n").as_bytes()).await;
        }
    }

    pub async fn cli_listener_count(&self) -> usize {
        self.cli_clients.lock().await.len()
    }

    pub fn rpc_calls(&self) -> Vec<serde_json::Value> {
        self.rec.lock().unwrap().rpc.clone()
    }

    pub fn closed(&self) -> usize {
        self.rec.lock().unwrap().closed
    }

    /// Payloads of every frame received with this opcode.
    pub fn payloads(&self, op: &[u8; 4]) -> Vec<Vec<u8>> {
        self.rec
            .lock()
            .unwrap()
            .payloads
            .iter()
            .filter(|(o, _)| o == op)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn http_paths(&self) -> Vec<String> {
        self.rec.lock().unwrap().http_paths.clone()
    }

    /// `/music/<id>/download` requests received: (path, Range header).
    pub fn downloads(&self) -> Vec<(String, Option<String>)> {
        self.rec.lock().unwrap().downloads.clone()
    }
}

pub trait IntoParts {
    fn into_response_parts(self) -> axum::response::Response;
}
impl<T: axum::response::IntoResponse> IntoParts for T {
    fn into_response_parts(self) -> axum::response::Response {
        self.into_response()
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

pub struct Harness {
    pub lms: FakeLms,
    pub addr: SocketAddr,
    pub gw: anywhere_gateway::AppState,
    pub accounts: Arc<Accounts>,
    /// Access tokens and assigned player MACs for two users' devices.
    pub tok_a: String,
    pub mac_a: [u8; 6],
    pub tok_b: String,
    pub mac_b: [u8; 6],
}

pub async fn harness_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    let lms = FakeLms::start().await;
    let mut cfg = Config {
        lms_slimproto: lms.slim_addr.to_string(),
        lms_http: lms.http_addr.to_string(),
        lms_cli: lms.cli_addr.to_string(),
        lms_ips: vec![Ipv4Addr::LOCALHOST],
        auth_timeout: Duration::from_millis(300),
        // Off unless a test asks: tests read player sockets message by message.
        player_keepalive: Duration::from_secs(3600),
        plays_secret: Some(PLAYS_SECRET.to_string()),
        ..Config::default()
    };
    mutate(&mut cfg);
    let accounts = Arc::new(Accounts::in_memory().unwrap());
    accounts.create_user("alice", PW, Role::Listener).unwrap();
    accounts.create_user("bob", PW, Role::Listener).unwrap();
    let a = accounts
        .login("alice", PW, None, "device-a", None, "test")
        .unwrap();
    let b = accounts
        .login("bob", PW, None, "device-b", None, "test")
        .unwrap();
    let gw = Gateway::new(cfg, accounts.clone());
    let app = router(gw.clone());
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
        gw,
        accounts,
        tok_a: a.access,
        mac_a: a.principal.mac,
        tok_b: b.access,
        mac_b: b.principal.mac,
    }
}

pub async fn harness() -> Harness {
    harness_with(|_| {}).await
}

impl Harness {
    pub async fn dial(&self) -> Ws {
        self.dial_with(&[]).await.expect("websocket dial")
    }

    pub async fn dial_with(
        &self,
        headers: &[(&str, &str)],
    ) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}/v1/player", self.addr)
            .into_client_request()
            .unwrap();
        for (k, v) in headers {
            let name = axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap();
            req.headers_mut().insert(name, v.parse().unwrap());
        }
        connect_async(req).await.map(|(ws, _)| ws)
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub async fn get_stream(&self, sid: &str, n: u64, token: Option<&str>) -> reqwest::Response {
        let mut req = reqwest::Client::new().get(self.url(&format!("/v1/stream/{sid}/{n}")));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap()
    }
}

pub async fn connect(ws: &mut Ws, token: &str) -> String {
    ws.send(Message::Text(token.into())).await.unwrap();
    match next(ws).await {
        Message::Text(t) => {
            let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
            let sid = v["session"].as_str().unwrap().to_string();
            assert_eq!(sid.len(), 32);
            let mac = v["mac"].as_str().unwrap();
            assert!(
                mac.starts_with("02:4c:41:") && mac.len() == 17,
                "assigned MAC {mac}"
            );
            assert!(v["name"].is_string());
            sid
        }
        other => panic!("expected session message, got {other:?}"),
    }
}

pub async fn next(ws: &mut Ws) -> Message {
    tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timed out waiting for message")
        .expect("socket ended")
        .expect("socket error")
}

pub async fn expect_close(ws: &mut Ws, want: CloseCode) {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no close in time")
        {
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(f.code, want, "close reason: {}", f.reason);
                return;
            }
            Some(Ok(Message::Close(None))) | None => panic!("closed without code, wanted {want:?}"),
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("socket error instead of close {want:?}: {e}"),
        }
    }
}

pub fn helo(mac: [u8; 6]) -> Vec<u8> {
    let mut p = vec![0u8; 36];
    p[0] = 12;
    p[2..8].copy_from_slice(&mac);
    p[34..36].copy_from_slice(b"en");
    p.extend_from_slice(b"Model=squeezelite,flc");
    client_frame(b"HELO", &p)
}

pub fn client_frame(op: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = op.to_vec();
    v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn strm(cmd: u8, ip: Ipv4Addr, request: &str) -> Vec<u8> {
    strm_format(cmd, b'f', ip, request)
}

/// A `strm` with the given format code (`f` FLAC, `m` MP3, …).
pub fn strm_format(cmd: u8, format: u8, ip: Ipv4Addr, request: &str) -> Vec<u8> {
    let mut p = vec![0u8; 24];
    p[0] = cmd;
    p[1] = b'1';
    p[2] = format;
    p[18..20].copy_from_slice(&9000u16.to_be_bytes());
    p[20..24].copy_from_slice(&ip.octets());
    p.extend_from_slice(request.as_bytes());
    encode_server_frame(*b"strm", &p)
}

pub fn stream_req(mac: [u8; 6]) -> String {
    format!(
        "GET /stream.mp3?player={} HTTP/1.0\r\n\r\n",
        format_mac(&mac)
    )
}

pub async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met in time");
}

/// A fresh scratch directory under the system's temp folder, named `<prefix>-<tag>-<random>`.
/// It also removes this prefix's leftovers from earlier runs (over an hour old), so the test
/// suites don't pile up directories there: a development machine had collected 16,500.
pub fn scratch_dir(prefix: &str, tag: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir();
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    if let Ok(entries) = std::fs::read_dir(&tmp) {
        for e in entries.flatten() {
            let stale = e
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{prefix}-"))
                && e.metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t < cutoff);
            if stale {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let d = tmp.join(format!("{prefix}-{tag}-{}", anywhere_gateway::random_id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
