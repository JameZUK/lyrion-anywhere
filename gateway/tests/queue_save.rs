//! Saving the queue as a playlist: admins only, for a player they may control, under a
//! checked name, never silently over an existing playlist.

mod common;
use common::*;

use anywhere_gateway::accounts::Role;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

const ADMIN_PW: &str = "correct horse battery";

/// The harness with an admin signed in, and saved playlists in the fake LMS.
async fn setup() -> (Harness, String) {
    let h = harness().await;
    h.accounts
        .create_user("carol", ADMIN_PW, Role::Admin)
        .unwrap();
    let admin = h
        .accounts
        .login("carol", ADMIN_PW, None, "Laptop", None, "test")
        .unwrap();
    h.lms.set_saved_playlists(FakeSaved {
        playlists: vec![(5, "Road Trip".into()), (6, "Road trip 2".into())],
        queue_len: 3,
    });
    (h, admin.access)
}

async fn save_raw(
    h: &Harness,
    token: Option<&str>,
    player: &str,
    body: reqwest::Body,
) -> (S, Value) {
    let mut req = reqwest::Client::new()
        .post(h.url(&format!("/v1/players/{player}/queue/save")))
        .header("content-type", "application/json")
        .body(body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = req.send().await.unwrap();
    let s = r.status();
    (s, r.json().await.unwrap_or(Value::Null))
}

async fn save(h: &Harness, token: &str, player: &str, body: Value) -> (S, Value) {
    save_raw(h, Some(token), player, body.to_string().into()).await
}

/// The `playlist save` commands LMS was sent.
fn saves(h: &Harness) -> Vec<Value> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "playlist" && c[1][1] == "save")
        .collect()
}

#[tokio::test]
async fn an_admin_saves_the_queue_and_gets_its_id() {
    let (h, admin) = setup().await;
    let me = format_mac(&h.mac_a);
    let (s, v) = save(&h, &admin, &me, json!({"name": "  Sunday morning "})).await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v, json!({"id": 102, "name": "Sunday morning"}));
    let sent = saves(&h);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0], json!([me, ["playlist", "save", "Sunday morning"]]));
    assert!(
        h.lms
            .saved_playlists()
            .contains(&(102, "Sunday morning".into()))
    );
    // Unicode names are fine.
    let (s, v) = save(&h, &admin, &me, json!({"name": "Sigur Rós – 日本"})).await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["name"], "Sigur Rós – 日本");
    // The answer never carries LMS's file URLs.
    assert!(!v.to_string().contains("file:"));
}

#[tokio::test]
async fn an_existing_name_is_kept_unless_replace_is_asked_for() {
    let (h, admin) = setup().await;
    let me = format_mac(&h.mac_a);
    for name in ["Road Trip", "road trip", "ROAD TRIP "] {
        let (s, v) = save(&h, &admin, &me, json!({"name": name})).await;
        assert_eq!(s, S::CONFLICT, "{name}: {v}");
        assert_eq!(v["exists"], json!({"id": 5, "name": "Road Trip"}));
        let (s, _) = save(&h, &admin, &me, json!({"name": name, "replace": false})).await;
        assert_eq!(s, S::CONFLICT);
    }
    assert!(saves(&h).is_empty(), "nothing was overwritten");
    // "Road trip 2" is only a search hit for "Road trip", not the same name... but "Road Trip"
    // exists, so try a name that only partly matches.
    let (s, v) = save(&h, &admin, &me, json!({"name": "Road"})).await;
    assert_eq!(s, S::OK, "{v}");

    // Replacing overwrites the existing playlist under its own spelling.
    let (s, v) = save(
        &h,
        &admin,
        &me,
        json!({"name": "road trip", "replace": true}),
    )
    .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v, json!({"id": 5, "name": "Road Trip"}));
    assert_eq!(
        saves(&h).last().unwrap()[1],
        json!(["playlist", "save", "Road Trip"])
    );
    let same: Vec<_> = h
        .lms
        .saved_playlists()
        .into_iter()
        .filter(|(_, n)| n.eq_ignore_ascii_case("road trip"))
        .collect();
    assert_eq!(
        same,
        [(5, "Road Trip".to_string())],
        "still one playlist of that name"
    );
}

#[tokio::test]
async fn an_empty_queue_is_not_saved() {
    let (h, admin) = setup().await;
    h.lms.set_saved_playlists(FakeSaved {
        playlists: vec![],
        queue_len: 0,
    });
    let (s, v) = save(
        &h,
        &admin,
        &format_mac(&h.mac_a),
        json!({"name": "Nothing"}),
    )
    .await;
    assert_eq!(s, S::CONFLICT, "{v}");
    assert_eq!(v["error"], "the queue is empty");
    assert!(saves(&h).is_empty());
}

#[tokio::test]
async fn listeners_and_strangers_are_refused_before_lms() {
    let (h, admin) = setup().await;
    let me = format_mac(&h.mac_a);
    let theirs = format_mac(&h.mac_b);
    let good = json!({"name": "Mine"});
    assert_eq!(
        save_raw(&h, None, &me, good.to_string().into()).await.0,
        S::UNAUTHORIZED
    );
    assert_eq!(save(&h, "nope", &me, good.clone()).await.0, S::UNAUTHORIZED);
    // A listener, on their own player: saved playlists are shared household data.
    let (s, v) = save(&h, &h.tok_a, &me, good.clone()).await;
    assert_eq!(s, S::FORBIDDEN);
    assert_eq!(v["error"], "changing playlists needs the admin role");
    // A listener, on another user's player.
    let (s, v) = save(&h, &h.tok_a, &theirs, good.clone()).await;
    assert_eq!(s, S::FORBIDDEN);
    assert_eq!(v["error"], "you may not control this player");
    // Anyone, on something that isn't a player.
    for player in ["not-a-player", "aa:bb:cc:dd:ee:zz", "..%2F..%2Fx", "%20"] {
        for tok in [&admin, &h.tok_a] {
            let (s, v) = save(&h, tok, player, good.clone()).await;
            assert_eq!(s, S::FORBIDDEN, "{player}: {v}");
        }
    }
    assert!(
        h.lms.rpc_calls().is_empty(),
        "LMS saw nothing: {:?}",
        h.lms.rpc_calls()
    );
}

#[tokio::test]
async fn bad_names_and_bodies_are_refused_before_lms() {
    let (h, admin) = setup().await;
    let me = format_mac(&h.mac_a);
    let long = "x".repeat(101);
    for name in [
        "",
        "   ",
        "..",
        "../../etc/passwd",
        "..\\..\\x",
        "a..b",
        "a/b",
        "/srv/music/x",
        "a\\b",
        "C:\\Windows\\x",
        "name:evil",
        "silent:1",
        "file:///etc/passwd",
        "a*b",
        "why?",
        "say \"hi\"",
        "<script>",
        "a|b",
        ".hidden",
        "ends with.",
        "new\nline",
        "carriage\rreturn",
        "tab\there",
        "nul\u{0}",
        "esc\u{1b}[2J",
        "bidi\u{202E}txt.exe",
        "zero\u{200B}width",
        "CON",
        "aux.m3u",
        long.as_str(),
    ] {
        let (s, v) = save(&h, &admin, &me, json!({"name": name})).await;
        assert_eq!(s, S::BAD_REQUEST, "{name:?}: {v}");
    }
    for body in [
        json!({}),
        json!(null),
        json!("Road trip"),
        json!(["Road trip"]),
        json!({"name": 5}),
        json!({"name": null}),
        json!({"name": "x", "replace": "true"}),
        json!({"name": "x", "extra": 1}),
        json!({"name": "x", "cmd": ["playlist", "save", "../x"]}),
    ] {
        let (s, v) = save(&h, &admin, &me, body.clone()).await;
        assert_eq!(s, S::BAD_REQUEST, "{body}: {v}");
    }
    for raw in ["{", "{\"name\":", "not json", "{\"name\":\"x\"}}", "\u{0}"] {
        let (s, _) = save_raw(&h, Some(&admin), &me, raw.to_string().into()).await;
        assert_eq!(s, S::BAD_REQUEST, "{raw:?}");
    }
    let huge = json!({"name": "x".repeat(5_000)}).to_string();
    let (s, _) = save_raw(&h, Some(&admin), &me, huge.into()).await;
    assert_eq!(s, S::PAYLOAD_TOO_LARGE);
    assert!(
        h.lms.rpc_calls().is_empty(),
        "LMS saw nothing: {:?}",
        h.lms.rpc_calls()
    );
}

/// Regression: the raw command stays refused for everyone; only the checked endpoint saves.
#[tokio::test]
async fn raw_playlist_save_is_still_refused() {
    let (h, admin) = setup().await;
    let me = format_mac(&h.mac_a);
    for tok in [&admin, &h.tok_a] {
        for cmd in [
            json!(["playlist", "save", "Road trip"]),
            json!(["playlist", "save", "../../x"]),
            json!(["playlist", "save", "x", "silent:1"]),
        ] {
            let r = reqwest::Client::new()
                .post(h.url("/v1/rpc"))
                .bearer_auth(tok)
                .json(&json!({"player": me, "cmd": cmd}))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), S::FORBIDDEN, "{cmd}");
        }
    }
    assert!(saves(&h).is_empty());
}

#[tokio::test]
async fn me_offers_saving_the_queue() {
    let h = harness().await;
    let me: Value = reqwest::Client::new()
        .get(h.url("/v1/me"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["features"]["save_queue"], true);
}
