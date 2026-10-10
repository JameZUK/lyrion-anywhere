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
