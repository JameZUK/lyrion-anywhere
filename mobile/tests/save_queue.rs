//! Saving the queue as a playlist against the real gateway: admins only, the name checked
//! before anything is sent, and an existing playlist (the fake LMS has "Road trip") never
//! overwritten without `replace`.

mod common;

use std::sync::Arc;

use anywhere_gateway::accounts::Role;
use anywhere_mobile::{ApiError, Players, SaveQueueOutcome, Session};
use common::*;
use serde_json::{Value, json};

async fn admin(h: &Harness) -> Arc<Session> {
    h.accounts.create_user("carol", PW, Role::Admin).unwrap();
    let s = h.session(MemStore::new());
    s.login("carol".into(), PW.into(), None).await.unwrap();
    s
}

fn saves(h: &Harness) -> Vec<Value> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "playlist" && c[1][1] == "save")
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admin_saves_the_queue() {
    let h = Harness::start().await;
    let s = admin(&h).await;
    let players = Players::new(s.clone());
    assert!(players.can_save_queue().await.unwrap());
    let me = s.current().unwrap().device.player_id;
    let r = players
        .save_queue(me.clone(), "  Sunday morning ".into(), false)
        .await
        .unwrap();
    assert_eq!(
        r,
        SaveQueueOutcome::Saved {
            id: 77,
            name: "Sunday morning".into()
        }
    );
    assert_eq!(
        saves(&h),
        [json!([me, ["playlist", "save", "Sunday morning"]])]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_existing_name_needs_replace() {
    let h = Harness::start().await;
    let s = admin(&h).await;
    let players = Players::new(s.clone());
    let me = s.current().unwrap().device.player_id;
    let r = players
        .save_queue(me.clone(), "ROAD TRIP".into(), false)
        .await
        .unwrap();
    assert_eq!(
        r,
        SaveQueueOutcome::Exists {
            id: 5,
            name: "Road trip".into()
        }
    );
    assert!(saves(&h).is_empty(), "nothing overwritten");
    let r = players
        .save_queue(me.clone(), "road trip".into(), true)
        .await
        .unwrap();
    assert!(matches!(r, SaveQueueOutcome::Saved { .. }), "{r:?}");
    // Saved over the existing playlist, under its own spelling.
    assert_eq!(saves(&h), [json!([me, ["playlist", "save", "Road trip"]])]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_queue_is_not_saved() {
    let h = Harness::start().await;
    let s = admin(&h).await;
    *h.lms.status.lock().unwrap() = Some(json!({"mode": "stop", "playlist_tracks": 0}));
    let players = Players::new(s.clone());
    let me = s.current().unwrap().device.player_id;
    let r = players.save_queue(me, "Nothing".into(), false).await;
    assert_eq!(r, Ok(SaveQueueOutcome::Empty));
    assert!(saves(&h).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listeners_and_bad_names_are_refused() {
    let h = Harness::start().await;
    let (alice, _) = h.alice().await;
    let players = Players::new(alice.clone());
    assert!(!players.can_save_queue().await.unwrap());
    let me = alice.current().unwrap().device.player_id;
    let r = players.save_queue(me.clone(), "Mine".into(), false).await;
    assert!(matches!(r, Err(ApiError::Forbidden { .. })), "{r:?}");

    let s = admin(&h).await;
    let players = Players::new(s.clone());
    let mine = s.current().unwrap().device.player_id;
    let n = h.lms.rpc_calls().len();
    for bad in [
        "",
        "  ",
        "../x",
        "a/b",
        "a\\b",
        "x:y",
        ".x",
        "x.",
        "a..b",
        "nul\0",
        "CON",
        "a\u{202E}b",
    ] {
        let r = players.save_queue(mine.clone(), bad.into(), false).await;
        assert!(
            matches!(r, Err(ApiError::InvalidInput { .. })),
            "{bad:?}: {r:?}"
        );
    }
    let long = "x".repeat(101);
    assert!(matches!(
        players.save_queue(mine.clone(), long, false).await,
        Err(ApiError::InvalidInput { .. })
    ));
    for p in ["not a player", "../../v1/me", ""] {
        assert!(matches!(
            players.save_queue(p.into(), "Fine".into(), false).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    assert_eq!(h.lms.rpc_calls().len(), n, "nothing reached LMS");
    assert!(saves(&h).is_empty());
}
