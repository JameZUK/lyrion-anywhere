//! `/v1/rpc`: the allow-list, player permissions and response filtering, end to end against
//! a fake LMS that records every JSON-RPC command it receives.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

impl Harness {
    async fn rpc(&self, token: &str, player: &str, cmd: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(self.url("/v1/rpc"))
            .bearer_auth(token)
            .json(&json!({ "player": player, "cmd": cmd }))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn browsing_reaches_lms_and_responses_are_filtered() {
    let h = harness().await;
    let r = h
        .rpc(&h.tok_a, "", json!(["albums", 0, 50, "tags:alj"]))
        .await;
    assert_eq!(r.status(), S::OK);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["result"]["albums_loop"][0]["album"], "Album");
    assert!(
        !body.to_string().contains("file://"),
        "paths must not leak: {body}"
    );
    assert_eq!(
        h.lms.rpc_calls(),
        vec![json!(["", ["albums", "0", "50", "tags:alj"]])]
    );
}

#[tokio::test]
async fn refused_commands_never_reach_lms() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let theirs = anywhere_core::proto::format_mac(&h.mac_b);
    let cases = [
        ("", json!(["readdirectory", 0, 100, "folder:/etc"])),
        ("", json!(["pref", "audiodir", "?"])),
        ("", json!(["restartserver"])),
        ("", json!(["rescan"])),
        (
            mine.as_str(),
            json!(["playlist", "play", "file:///etc/passwd"]),
        ),
        (
            mine.as_str(),
            json!(["playlistcontrol", "cmd:load", "url:file:///x"]),
        ),
        (theirs.as_str(), json!(["play"])),
        (theirs.as_str(), json!(["status", 0, 10])),
        ("aa:bb:cc:dd:ee:01", json!(["play"])),
        (mine.as_str(), json!(["sync", theirs])),
        ("", json!(["evilapp", "items", 0, 10])),
    ];
    for (player, cmd) in cases {
        let r = h.rpc(&h.tok_a, player, cmd.clone()).await;
        assert_eq!(r.status(), S::FORBIDDEN, "{player} {cmd}");
    }
    let calls: Vec<Value> = h
        .lms
        .rpc_calls()
        .into_iter()
        .filter(|c| !matches!(c[1][0].as_str(), Some("apps" | "radios")))
        .collect();
    assert!(calls.is_empty(), "refused commands reached LMS: {calls:?}");
}

#[tokio::test]
async fn bad_requests_are_rejected() {
    let h = harness().await;
    let c = reqwest::Client::new();
    let post = |body: String, auth: Option<&str>| {
        let mut r = c
            .post(h.url("/v1/rpc"))
            .header("content-type", "application/json")
            .body(body);
        if let Some(t) = auth {
            r = r.bearer_auth(t.to_string());
        }
        r.send()
    };
    assert_eq!(
        post(json!({"cmd": ["albums", 0, 1]}).to_string(), None)
            .await
            .unwrap()
            .status(),
        S::UNAUTHORIZED
    );
    assert_eq!(
        post(json!({"cmd": ["albums", 0, 1]}).to_string(), Some("nope"))
            .await
            .unwrap()
            .status(),
        S::UNAUTHORIZED
    );
    assert_eq!(
        post("not json".into(), Some(&h.tok_a))
            .await
            .unwrap()
            .status(),
        S::BAD_REQUEST
    );
    assert_eq!(
        post(
            json!({"cmd": ["albums", 0, 1], "id": 99}).to_string(),
            Some(&h.tok_a)
        )
        .await
        .unwrap()
        .status(),
        S::BAD_REQUEST,
        "unknown fields"
    );
    assert_eq!(
        post(
            json!({"cmd": ["x".repeat(9000)]}).to_string(),
            Some(&h.tok_a)
        )
        .await
        .unwrap()
        .status(),
        S::PAYLOAD_TOO_LARGE
    );
    assert!(h.lms.rpc_calls().is_empty());
}

#[tokio::test]
async fn listeners_control_own_devices_and_granted_players() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    assert_eq!(
        h.rpc(&h.tok_a, &mine, json!(["play"])).await.status(),
        S::OK
    );
    assert_eq!(
        h.rpc(&h.tok_a, "aa:bb:cc:dd:ee:01", json!(["play"]))
            .await
            .status(),
        S::FORBIDDEN
    );
    h.accounts
        .grant_player("alice", "aa:bb:cc:dd:ee:01")
        .unwrap();
    assert_eq!(
        h.rpc(&h.tok_a, "AA:BB:CC:DD:EE:01", json!(["play"]))
            .await
            .status(),
        S::OK
    );
    // Sync between two permitted players.
    assert_eq!(
        h.rpc(&h.tok_a, &mine, json!(["sync", "aa:bb:cc:dd:ee:01"]))
            .await
            .status(),
        S::OK
    );
    // A revoked grant takes effect on the next request.
    h.accounts
        .ungrant_player("alice", "aa:bb:cc:dd:ee:01")
        .unwrap();
    assert_eq!(
        h.rpc(&h.tok_a, "aa:bb:cc:dd:ee:01", json!(["play"]))
            .await
            .status(),
        S::FORBIDDEN
    );
}

#[tokio::test]
async fn player_lists_show_only_permitted_players_without_addresses() {
    let h = harness().await;
    h.accounts
        .grant_player("alice", "aa:bb:cc:dd:ee:02")
        .unwrap();
    let body: Value = h
        .rpc(&h.tok_a, "", json!(["serverstatus", 0, 50]))
        .await
        .json()
        .await
        .unwrap();
    let s = body.to_string();
    for leak in [
        "10.0.0.",
        "server-uuid-secret",
        "9000",
        "Home One",
        "Someone's phone",
        "other player",
    ] {
        assert!(!s.contains(leak), "{leak} leaked: {s}");
    }
    let players = body["result"]["players_loop"].as_array().unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0]["name"], "Home Two");
    assert_eq!(body["result"]["player count"], 1);
}

#[tokio::test]
async fn app_commands_are_learned_from_lms_menus() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    // Not seen yet: the gateway refreshes the app list once, then allows it.
    let r = h
        .rpc(&h.tok_a, &mine, json!(["spotty", "items", 0, 20, "menu:1"]))
        .await;
    assert_eq!(r.status(), S::OK);
    let cmds: Vec<String> = h
        .lms
        .rpc_calls()
        .iter()
        .map(|c| c[1][0].as_str().unwrap().to_string())
        .collect();
    assert_eq!(cmds, ["apps", "radios", "spotty"]);
    // An app LMS never listed stays refused, and refreshes are rate-limited.
    assert_eq!(
        h.rpc(&h.tok_a, &mine, json!(["evilapp", "items", 0, 20]))
            .await
            .status(),
        S::FORBIDDEN
    );
    assert_eq!(
        h.rpc(&h.tok_a, &mine, json!(["evilapp2", "items", 0, 20]))
            .await
            .status(),
        S::FORBIDDEN
    );
    let refreshes = h
        .lms
        .rpc_calls()
        .iter()
        .filter(|c| c[1][0] == "apps")
        .count();
    assert_eq!(
        refreshes, 1,
        "unknown commands must not trigger repeated refreshes"
    );
}

#[tokio::test]
async fn lms_unreachable_is_a_502() {
    let h = harness_with(|c| c.lms_http = "127.0.0.1:1".into()).await;
    let r = h.rpc(&h.tok_a, "", json!(["albums", 0, 1])).await;
    assert_eq!(r.status(), S::BAD_GATEWAY);
}

// --- plugin and context menus: actions LMS offered, and nothing else -----------------------

#[tokio::test]
async fn plugins_in_the_home_menu_can_be_browsed() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let open = json!(["someplugin", "menu", 0, 100]);
    // Not offered yet.
    assert_eq!(
        h.rpc(&h.tok_a, &mine, open.clone()).await.status(),
        S::FORBIDDEN
    );
    let r = h
        .rpc(&h.tok_a, &mine, json!(["menu", 0, 500, "direct:1"]))
        .await;
    assert_eq!(r.status(), S::OK);
    let menu: Value = r.json().await.unwrap();
    let s = menu.to_string();
    assert!(s.contains("Some Plugin"), "{s}");
    assert!(
        !s.contains("crossfadesettings") && !s.contains("playerpower"),
        "settings and power never reach a device: {s}"
    );
    // The plugin's menu opens now (paged, as a navigation); settings stay refused.
    assert_eq!(h.rpc(&h.tok_a, &mine, open).await.status(), S::OK);
    assert_eq!(
        h.rpc(&h.tok_a, &mine, json!(["crossfadesettings"]))
            .await
            .status(),
        S::FORBIDDEN
    );
    // Learned per device: another device hasn't been shown it.
    let theirs = anywhere_core::proto::format_mac(&h.mac_b);
    assert_eq!(
        h.rpc(&h.tok_b, &theirs, json!(["someplugin", "menu", 0, 100]))
            .await
            .status(),
        S::FORBIDDEN
    );
    // Another device's player: the home menu itself is refused.
    assert_eq!(
        h.rpc(&h.tok_a, &theirs, json!(["menu", 0, 500, "direct:1"]))
            .await
            .status(),
        S::FORBIDDEN
    );
}

/// Plugins the admin marked admin-only: listeners neither see nor run them; admins still do.
#[tokio::test]
async fn admin_only_plugins_are_hidden_from_listeners_and_refused() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    h.accounts
        .set_admin_only_plugins(&["someplugin".into(), "spotty".into()])
        .unwrap();

    let apps: Value = h
        .rpc(&h.tok_a, "", json!(["apps", 0, 100]))
        .await
        .json()
        .await
        .unwrap();
    assert!(!apps.to_string().contains("spotty"), "{apps}");
    let menu: Value = h
        .rpc(&h.tok_a, &mine, json!(["menu", 0, 500, "direct:1"]))
        .await
        .json()
        .await
        .unwrap();
    assert!(!menu.to_string().contains("someplugin"), "{menu}");
    for cmd in [
        json!(["spotty", "items", 0, 20, "menu:1"]),
        json!(["someplugin", "menu", 0, 100]),
    ] {
        let r = h.rpc(&h.tok_a, &mine, cmd.clone()).await;
        assert_eq!(r.status(), S::FORBIDDEN, "{cmd}");
    }
    assert!(
        !h.lms
            .rpc_calls()
            .iter()
            .any(|c| c[1][0] == "spotty" || c[1][0] == "someplugin"),
        "LMS never sees a refused plugin command"
    );

    // An admin sees and runs them.
    h.accounts
        .create_user("carol", "correct horse battery", Role::Admin)
        .unwrap();
    let admin = h
        .accounts
        .login(
            "carol",
            "correct horse battery",
            None,
            "Laptop",
            None,
            "test",
        )
        .unwrap();
    let apps: Value = h
        .rpc(&admin.access, "", json!(["apps", 0, 100]))
        .await
        .json()
        .await
        .unwrap();
    assert!(apps.to_string().contains("spotty"));
    let r = h
        .rpc(
            &admin.access,
            &mine,
            json!(["spotty", "items", 0, 20, "menu:1"]),
        )
        .await;
    assert_eq!(r.status(), S::OK);

    // Lifting the restriction applies at once.
    h.accounts.set_admin_only_plugins(&[]).unwrap();
    let r = h
        .rpc(&h.tok_a, &mine, json!(["spotty", "items", 0, 20, "menu:1"]))
        .await;
    assert_eq!(r.status(), S::OK);
}

async fn open_track_menu(h: &Harness, token: &str, player: &str) -> Value {
    let r = h
        .rpc(
            token,
            player,
            json!(["trackinfo", "items", 0, 50, "track_id:9", "menu:1"]),
        )
        .await;
    assert_eq!(r.status(), S::OK);
    r.json().await.unwrap()
}

#[tokio::test]
async fn menu_actions_run_only_after_lms_offered_them() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let similar = json!(["audiomuse", "similar_track", "track_id:9"]);

    // Not offered yet: refused, and LMS never sees it.
    let r = h.rpc(&h.tok_a, &mine, similar.clone()).await;
    assert_eq!(r.status(), S::FORBIDDEN);
    assert!(
        !h.lms
            .rpc_calls()
            .iter()
            .any(|c| c.to_string().contains("audiomuse"))
    );

    let menu = open_track_menu(&h, &h.tok_a, &mine).await;
    // The poisoned actions (a file path, a LAN URL) never reach the client.
    let s = menu.to_string();
    assert!(!s.contains("file://") && !s.contains("10.0.0.50"), "{s}");
    assert!(
        !s.contains("jivefavorites"),
        "a broken action must be dropped whole: {s}"
    );

    // Offered now: it runs.
    let r = h.rpc(&h.tok_a, &mine, similar.clone()).await;
    assert_eq!(r.status(), S::OK);
    assert!(
        h.lms
            .rpc_calls()
            .iter()
            .any(|c| c == &json!([mine.clone(), ["audiomuse", "similar_track", "track_id:9"]]))
    );

    // But only exactly as offered.
    for tampered in [
        json!(["audiomuse", "similar_track", "track_id:10"]),
        json!(["audiomuse", "similar_track", "track_id:9", "extra:1"]),
        json!(["audiomuse", "similar_track"]),
        json!(["audiomuse", "other_command", "track_id:9"]),
    ] {
        assert_eq!(
            h.rpc(&h.tok_a, &mine, tampered.clone()).await.status(),
            S::FORBIDDEN,
            "{tampered}"
        );
    }
}

#[tokio::test]
async fn menu_actions_are_per_device_and_respect_player_permissions() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let theirs = anywhere_core::proto::format_mac(&h.mac_b);
    open_track_menu(&h, &h.tok_a, &mine).await;
    let similar = json!(["audiomuse", "similar_track", "track_id:9"]);
    // Bob was never shown the menu: refused, even on his own player.
    assert_eq!(
        h.rpc(&h.tok_b, &theirs, similar.clone()).await.status(),
        S::FORBIDDEN
    );
    // Alice can't aim her learned action at Bob's player.
    assert_eq!(
        h.rpc(&h.tok_a, &theirs, similar).await.status(),
        S::FORBIDDEN
    );
}

#[tokio::test]
async fn server_commands_offered_in_menus_still_never_run() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    open_track_menu(&h, &h.tok_a, &mine).await;
    let r = h
        .rpc(&h.tok_a, &mine, json!(["pref", "audiodir", "value:/x"]))
        .await;
    assert_eq!(r.status(), S::FORBIDDEN);
    assert!(
        !h.lms
            .rpc_calls()
            .iter()
            .any(|c| c.to_string().contains("audiodir"))
    );
}

#[tokio::test]
async fn context_menus_take_only_library_ids() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    for bad in [
        json!(["trackinfo", "items", 0, 50, "track_id:9;rescan", "menu:1"]),
        json!(["trackinfo", "items", 0, 50, "url:file:///x", "menu:1"]),
        json!(["trackinfo", "items", 0, 5000, "track_id:9"]),
        json!(["trackinfo", "delete", "track_id:9"]),
    ] {
        assert_eq!(
            h.rpc(&h.tok_a, &mine, bad.clone()).await.status(),
            S::FORBIDDEN,
            "{bad}"
        );
    }
}

/// Browsing the music folder: by numeric id only, and the answer never carries a path.
#[tokio::test]
async fn music_folder_browsing_never_reveals_or_takes_paths() {
    let h = harness().await;
    let r = h
        .rpc(
            &h.tok_a,
            "",
            json!(["musicfolder", 0, 50, "folder_id:11", "tags:cdt"]),
        )
        .await;
    assert_eq!(r.status(), S::OK);
    let body: Value = r.json().await.unwrap();
    let text = body.to_string();
    for leak in ["file:", "/srv/", "smb:", "10.0.0.10"] {
        assert!(!text.contains(leak), "{leak} leaked: {text}");
    }
    assert_eq!(body["result"]["folder_loop"][0]["filename"], "Jazz");
    assert_eq!(body["result"]["folder_loop"][1]["title"], "Song");
    for bad in [
        json!(["musicfolder", 0, 50, "url:file:///srv/music"]),
        json!(["musicfolder", 0, 50, "folder_id:/srv/music"]),
        json!(["musicfolder", 0, 50, "folder_id:1,2"]),
        json!(["musicfolder", 0, 50, "folder:/etc"]),
        json!(["artists", 0, 50, "role_id:FOO"]),
        json!(["albums", 0, 50, "library_id:../x"]),
        json!(["libraries", "getid"]),
    ] {
        assert_eq!(
            h.rpc(&h.tok_a, "", bad.clone()).await.status(),
            S::FORBIDDEN,
            "{bad}"
        );
    }
    let calls: Vec<Value> = h
        .lms
        .rpc_calls()
        .into_iter()
        .filter(|c| !matches!(c[1][0].as_str(), Some("apps" | "radios")))
        .collect();
    assert_eq!(calls.len(), 1, "refused commands reached LMS: {calls:?}");
    // Virtual libraries: listed with their ids and names.
    let r = h.rpc(&h.tok_a, "", json!(["libraries"])).await;
    assert_eq!(r.status(), S::OK);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["result"]["folder_loop"][0]["id"], "1a2b3c4d");
}

/// Regression: an `imageproxy/<url>` in any text LMS relays (a track title, a player name)
/// taught the gateway that URL, so a user could make LMS fetch from the home network.
#[tokio::test]
async fn image_proxy_urls_in_free_text_never_become_fetchable() {
    let h = harness().await;
    let r = h.rpc(&h.tok_a, "", json!(["tracks", 0, 10])).await;
    assert_eq!(r.status(), S::OK);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["result"]["titles_loop"][0]["id"], 2);
    for target in ["x", "y", "z"] {
        let path = format!("/v1/art/imageproxy/http%3A%2F%2F10.0.0.1%2F{target}/image.png");
        let r = reqwest::Client::new()
            .get(h.url(&path))
            .bearer_auth(&h.tok_a)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::NOT_FOUND, "{path}");
    }
    assert!(
        !h.lms
            .http_paths()
            .iter()
            .any(|p| p.starts_with("/imageproxy/")),
        "LMS was asked to fetch a local URL"
    );
}

/// Regression: track info lines such as `Location: /srv/music/…` and `URL: http://10.0.0.10…`
/// reached clients, revealing the server's filesystem and address.
#[tokio::test]
async fn track_info_lines_with_paths_or_lan_urls_are_dropped() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let menu = open_track_menu(&h, &h.tok_a, &mine).await;
    let s = menu.to_string();
    for leaked in ["/srv/", "10.0.0.10", "Location:", "URL:"] {
        assert!(!s.contains(leaked), "{leaked} leaked: {s}");
    }
    let texts: Vec<&str> = menu["result"]["item_loop"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["text"].as_str())
        .collect();
    assert!(texts.contains(&"Artist: AC/DC"), "{texts:?}");
    assert!(texts.contains(&"Similar tracks"), "{texts:?}");
}

/// Regression: `syncgroups ?` and `sync ?` named players the caller may not control.
#[tokio::test]
async fn sync_information_names_only_permitted_players() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let groups = || async {
        h.rpc(&h.tok_a, "", json!(["syncgroups", "?"]))
            .await
            .json::<Value>()
            .await
            .unwrap()
    };
    let none = groups().await;
    let s = none.to_string();
    assert!(!s.contains("ee:01") && !s.contains("Home One"), "{s}");
    assert_eq!(none["result"]["syncgroups_loop"], json!([]));

    h.accounts
        .grant_player("alice", "aa:bb:cc:dd:ee:01")
        .unwrap();
    let one = groups().await;
    assert_eq!(
        one["result"]["syncgroups_loop"],
        json!([{"sync_members": "aa:bb:cc:dd:ee:01", "sync_member_names": "Home One"}])
    );
    assert!(!one.to_string().contains("Home Two"));

    let sync: Value = h
        .rpc(&h.tok_a, &mine, json!(["sync", "?"]))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(sync["result"]["_sync"], "aa:bb:cc:dd:ee:01");
}

#[tokio::test]
async fn a_status_that_skips_the_playing_track_is_repaired() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let r = h
        .rpc(&h.tok_a, &mine, json!(["status", "-", 1, "tags:adc"]))
        .await;
    assert_eq!(r.status(), S::OK);
    let body: Value = r.json().await.unwrap();
    let cur = &body["result"]["playlist_loop"][0];
    assert_eq!(cur["title"], "Playing Track");
    assert_eq!(cur["id"], 4);
    assert_eq!(cur["playlist index"], 1);
    // Filtered like any status: no path or URL reaches the client.
    let text = body.to_string();
    assert!(!text.contains("/srv/music"), "{text}");
    assert!(cur.get("url").is_none());
    // The gateway asked for exactly the current position, then that file's details.
    let calls = h.lms.rpc_calls();
    let asked: Vec<&Value> = calls.iter().map(|c| &c[1]).collect();
    assert!(
        asked.contains(&&json!(["playlist", "path", "1", "?"])),
        "{asked:?}"
    );
    assert!(asked.iter().any(|c| c[0] == "songinfo"
        && c[3] == "url:file:///srv/music/Playing%20Track.flac"
        && c[4] == "tags:adc"));
}

#[tokio::test]
async fn a_queue_listing_is_never_repaired() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let r = h.rpc(&h.tok_a, &mine, json!(["status", 0, 10])).await;
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["result"]["playlist_loop"][0]["title"], "Next Track");
    assert!(h.lms.rpc_calls().iter().all(|c| c[1][0] != "songinfo"));
}

/// Don't Stop The Music: a listener reads its menu and chooses a provider for their own
/// player; nothing else of `playerpref`, no other player, no malformed provider.
#[tokio::test]
async fn dont_stop_the_music_for_permitted_players_only() {
    let h = harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let theirs = anywhere_core::proto::format_mac(&h.mac_b);
    let pref = "plugin.dontstopthemusic:provider";
    let r = h
        .rpc(&h.tok_a, &mine, json!(["dontstopthemusicsetting"]))
        .await;
    assert_eq!(r.status(), S::OK);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["result"]["item_loop"].as_array().unwrap().len(), 4);
    let r = h
        .rpc(
            &h.tok_a,
            &mine,
            json!(["playerpref", pref, "PLUGIN_DSTM_SIMILAR"]),
        )
        .await;
    assert_eq!(r.status(), S::OK);
    assert!(
        h.lms
            .rpc_calls()
            .iter()
            .any(|c| c == &json!([mine.clone(), ["playerpref", pref, "PLUGIN_DSTM_SIMILAR"]]))
    );

    let lms_calls = || {
        h.lms
            .rpc_calls()
            .into_iter()
            .filter(|c| !matches!(c[1][0].as_str(), Some("apps" | "radios")))
            .count()
    };
    let before = lms_calls();
    for (player, cmd) in [
        (theirs.as_str(), json!(["dontstopthemusicsetting"])),
        (
            theirs.as_str(),
            json!(["playerpref", pref, "PLUGIN_DSTM_SIMILAR"]),
        ),
        ("", json!(["playerpref", pref, "PLUGIN_DSTM_SIMILAR"])),
        (mine.as_str(), json!(["playerpref", pref, "/srv/music"])),
        (mine.as_str(), json!(["playerpref", pref, "PLUGIN X"])),
        (mine.as_str(), json!(["playerpref", "audiodir", "/"])),
        (
            mine.as_str(),
            json!(["playerpref", "plugin.anywhere:secret", "x"]),
        ),
        (mine.as_str(), json!(["dontstopthemusicsetting", 0, 100])),
    ] {
        let r = h.rpc(&h.tok_a, player, cmd.clone()).await;
        assert_eq!(r.status(), S::FORBIDDEN, "{player} {cmd}");
    }
    assert_eq!(lms_calls(), before, "refused commands reached LMS");
}

/// An admin-only plugin's Don't Stop The Music provider is neither listed nor choosable by
/// a listener.
#[tokio::test]
async fn admin_only_plugins_dstm_providers_are_hidden() {
    let h = harness().await;
    h.accounts
        .set_admin_only_plugins(&["audiomuseai".to_string()])
        .unwrap();
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let pref = "plugin.dontstopthemusic:provider";
    let body: Value = h
        .rpc(&h.tok_a, &mine, json!(["dontstopthemusicsetting"]))
        .await
        .json()
        .await
        .unwrap();
    let s = body.to_string();
    assert!(
        !s.contains("AUDIOMUSE") && s.contains("PLUGIN_RANDOM_TRACK"),
        "{s}"
    );
    assert_eq!(body["result"]["count"], 3);
    let r = h
        .rpc(
            &h.tok_a,
            &mine,
            json!(["playerpref", pref, "PLUGIN_AUDIOMUSEAI_DSTM"]),
        )
        .await;
    assert_eq!(r.status(), S::FORBIDDEN);
    assert!(
        !h.lms
            .rpc_calls()
            .iter()
            .any(|c| c.to_string().contains("AUDIOMUSEAI"))
    );
}

// --- URLs in free text (M2): judged by where they resolve, not only by name ----------------

/// A harness whose resolver gives public-looking names fixed answers (like a hosts file):
/// `loop.example.com` is loopback, `mapped.example.com` an IPv4-mapped private address,
/// `cdn.example.com` a public (TEST-NET) address.
async fn resolving_harness() -> Harness {
    harness_with(|c| {
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        c.host_answers
            .insert("loop.example.com".into(), vec![ip("127.0.0.1")]);
        c.host_answers.insert(
            "mapped.example.com".into(),
            vec![ip("203.0.113.7"), ip("::ffff:10.0.0.10")],
        );
        c.host_answers
            .insert("cdn.example.com".into(), vec![ip("203.0.113.10")]);
    })
    .await
}

/// Free-text values that reached LMS (search terms and typed input).
fn free_text_calls(h: &Harness) -> Vec<String> {
    h.lms
        .rpc_calls()
        .iter()
        .flat_map(|c| c[1].as_array().cloned().unwrap_or_default())
        .filter_map(|w| w.as_str().map(str::to_owned))
        .filter(|w| w.starts_with("search:") || w.starts_with("term:"))
        .collect()
}

/// Regression (M2): a URL in a search term whose name resolves to a local address (here
/// loopback, or a private address hidden in an IPv4-mapped IPv6 answer) reached LMS, so a
/// plugin could be made to fetch LMS's own web UI or CLI port. Refused like any other
/// sensitive text, and never sent.
#[tokio::test]
async fn free_text_urls_resolving_to_local_addresses_are_refused() {
    let h = resolving_harness().await;
    for url in [
        "http://loop.example.com/feed.xml",
        "https://LOOP.example.com./x",
        "listen to http://mapped.example.com/rss now",
        "https://cdn.example.com/a http://loop.example.com/b",
    ] {
        let r = h
            .rpc(
                &h.tok_a,
                "",
                json!(["albums", 0, 10, format!("search:{url}")]),
            )
            .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{url}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["error"], "bad value for search", "{url}");
        let r = h
            .rpc(
                &h.tok_a,
                "",
                json!(["search", 0, 10, format!("term:{url}")]),
            )
            .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{url}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["error"], "bad value for term", "{url}");
    }
    assert!(
        free_text_calls(&h).is_empty(),
        "reached LMS: {:?}",
        free_text_calls(&h)
    );
}

/// Regression (M2): the same through a plugin's search box (`__TAGGEDINPUT__`), where the
/// typed text is the only part of the action LMS did not offer.
#[tokio::test]
async fn typed_menu_input_resolving_to_local_addresses_is_refused() {
    let h = resolving_harness().await;
    let mine = anywhere_core::proto::format_mac(&h.mac_a);
    let r = h
        .rpc(&h.tok_a, &mine, json!(["menu", 0, 500, "direct:1"]))
        .await;
    assert_eq!(r.status(), S::OK);
    let r = h
        .rpc(&h.tok_a, &mine, json!(["someplugin", "menu", 0, 100]))
        .await;
    assert_eq!(r.status(), S::OK);
    let typed = |q: &str| {
        json!([
            "someplugin",
            "items",
            0,
            20,
            format!("search:{q}"),
            "menu:someplugin"
        ])
    };
    for bad in [
        "http://loop.example.com/",
        "http://mapped.example.com/feed",
        "https://cdn.example.com:9000/feed",
    ] {
        let r = h.rpc(&h.tok_a, &mine, typed(bad)).await;
        assert_eq!(r.status(), S::FORBIDDEN, "{bad}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["error"], "malformed command", "{bad}");
    }
    assert!(
        free_text_calls(&h).is_empty(),
        "reached LMS: {:?}",
        free_text_calls(&h)
    );
    // Ordinary words and public URLs still work.
    for good in ["in our time", "https://cdn.example.com/feed.xml"] {
        let r = h.rpc(&h.tok_a, &mine, typed(good)).await;
        assert_eq!(r.status(), S::OK, "{good}");
    }
    assert_eq!(free_text_calls(&h).len(), 2);
}

/// Regression (M2): an explicit port other than 80 or 443 in free text (LMS's web UI on
/// 9000, its CLI on 9090) is refused whatever the name resolves to; plugins never need one.
#[tokio::test]
async fn free_text_urls_with_unusual_ports_are_refused() {
    let h = resolving_harness().await;
    for url in [
        "http://cdn.example.com:9000/",
        "https://cdn.example.com:9090/x",
        "http://user@cdn.example.com:8080/",
        "HTTP://cdn.example.com:00080x/",
        "http://[2001:db8::1]:9000/",
        "http://no-such-host.invalid:9000/",
    ] {
        let r = h
            .rpc(
                &h.tok_a,
                "",
                json!(["albums", 0, 10, format!("search:{url}")]),
            )
            .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{url}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["error"], "bad value for search", "{url}");
    }
    assert!(
        free_text_calls(&h).is_empty(),
        "reached LMS: {:?}",
        free_text_calls(&h)
    );
}

/// Plain text, public URLs (with or without the standard ports) and names that don't resolve
/// at all still reach LMS: apps search by feed URL.
#[tokio::test]
async fn ordinary_text_and_public_urls_still_pass() {
    let h = resolving_harness().await;
    let good = [
        "AC/DC",
        "Live 1990/91",
        "https://cdn.example.com/feed.xml",
        "http://cdn.example.com:80/feed",
        "https://cdn.example.com:443/feed",
        "https://podcasts.example.invalid/feed",
        "10:30 news",
    ];
    for text in good {
        let r = h
            .rpc(
                &h.tok_a,
                "",
                json!(["albums", 0, 10, format!("search:{text}")]),
            )
            .await;
        assert_eq!(r.status(), S::OK, "{text}");
    }
    assert_eq!(free_text_calls(&h).len(), good.len());
}
