//! Transfer, queue editing and sync groups against the real gateway (docs/PROTOCOL.md ›
//! Transfer). Every step must pass the gateway's policy between two players the account
//! may control, and be refused, with nothing reaching LMS, as soon as either is not.

mod common;

use std::sync::{Arc, Mutex};

use anywhere_mobile::{
    ApiError, Players, Session, TransferMethod, TransferOutcome, TransferProgress, TransferRefusal,
    TransferStage,
};
use common::*;
use serde_json::{Value, json};

#[derive(Default)]
struct Stages(Mutex<Vec<TransferStage>>);

impl TransferProgress for Stages {
    fn on_stage(&self, stage: TransferStage) {
        self.0.lock().unwrap().push(stage);
    }
}

fn progress() -> Arc<Stages> {
    Arc::new(Stages::default())
}

async fn bob(h: &Harness) -> Arc<Session> {
    let b = h.session(MemStore::new());
    b.login("bob".into(), PW.into(), None).await.unwrap();
    b
}

fn since(h: &Harness, before: usize) -> Vec<Value> {
    h.lms.rpc_calls()[before..]
        .iter()
        .filter(|c| !matches!(c[1][0].as_str(), Some("apps" | "radios" | "menu")))
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playback_moves_by_grouping_then_leaving() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    h.accounts.grant_player("alice", HOME_PLAYER).unwrap();
    let players = Players::new(s.clone());

    let before = h.lms.rpc_calls().len();
    let stages = progress();
    let r = players
        .transfer(me.clone(), HOME_PLAYER.into(), stages.clone())
        .await
        .unwrap();
    assert_eq!(
        r,
        TransferOutcome::Moved {
            method: TransferMethod::Group
        }
    );
    assert_eq!(
        *stages.0.lock().unwrap(),
        vec![
            TransferStage::Reading,
            TransferStage::Grouping,
            TransferStage::Starting
        ]
    );
    let calls = since(&h, before);
    let status = json!(["status", "-", "2", "tags:aAlcdeJKjNoqrsTIyx"]);
    assert_eq!(
        calls,
        vec![
            json!([me, status]),
            json!([HOME_PLAYER, ["power", "1"]]),
            // The source names the target: the target joins the source's group.
            json!([me, ["sync", HOME_PLAYER]]),
            json!([HOME_PLAYER, status]),
            json!([me, ["sync", "-"]]),
            json!([me, ["stop"]]),
            // The fake source was playing: the target plays on, nothing more to send.
        ]
    );
    // Not left grouped.
    assert!(h.lms.syncs.members.lock().unwrap().is_empty());

    // And back again ("Bring to this phone").
    let r = players
        .transfer(HOME_PLAYER.into(), me.clone(), progress())
        .await
        .unwrap();
    assert_eq!(
        r,
        TransferOutcome::Moved {
            method: TransferMethod::Group
        }
    );
    assert!(
        h.lms
            .rpc_calls()
            .contains(&json!([HOME_PLAYER, ["sync", me]]))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn when_lms_will_not_group_the_list_is_copied() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    h.accounts.grant_player("alice", HOME_PLAYER).unwrap();
    h.lms
        .syncs
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let players = Players::new(s.clone());

    let before = h.lms.rpc_calls().len();
    let stages = progress();
    let r = players
        .transfer(me.clone(), HOME_PLAYER.into(), stages.clone())
        .await
        .unwrap();
    assert_eq!(
        r,
        TransferOutcome::Moved {
            method: TransferMethod::Copy
        }
    );
    assert_eq!(
        stages.0.lock().unwrap().last(),
        Some(&TransferStage::Copying)
    );
    let calls = since(&h, before);
    let tail: Vec<Value> = calls
        .iter()
        .skip_while(|c| c[1] != json!(["sync", "-"]))
        .cloned()
        .collect();
    assert_eq!(tail[0], json!([HOME_PLAYER, ["sync", "-"]]));
    for want in [
        json!([me, ["status", "0", "500", "tags:alcdJKj"]]),
        json!([HOME_PLAYER, ["playlist", "shuffle", "0"]]),
        json!([HOME_PLAYER, ["playlist", "repeat", "0"]]),
        json!([
            HOME_PLAYER,
            [
                "playlistcontrol",
                "cmd:load",
                "track_id:1,2",
                "play_index:0"
            ]
        ]),
        json!([HOME_PLAYER, ["time", "12"]]),
    ] {
        assert!(tail.contains(&want), "{want} not sent: {tail:?}");
    }
    assert_eq!(tail.last(), Some(&json!([me, ["stop"]])));
    // The source never left a group it was never in.
    assert!(!calls.contains(&json!([me, ["sync", "-"]])));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_needs_both_players() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let players = Players::new(s.clone());
    let bobs = bob(&h).await.current().unwrap().device.player_id;

    // A target alice may not control: refused at the first step that names it, and nothing
    // that names it reaches LMS.
    for target in [HOME_PLAYER.to_string(), bobs.clone()] {
        let before = h.lms.rpc_calls().len();
        let r = players
            .transfer(me.clone(), target.clone(), progress())
            .await;
        assert!(matches!(r, Err(ApiError::Forbidden { .. })), "{r:?}");
        let calls = since(&h, before);
        assert!(
            calls
                .iter()
                .all(|c| c[0] == json!(me) && c[1][0] == "status"),
            "{calls:?}"
        );
        assert!(h.lms.syncs.members.lock().unwrap().is_empty());
    }
    // A source alice may not control: refused before anything is sent.
    let before = h.lms.rpc_calls().len();
    assert!(matches!(
        players.transfer(bobs.clone(), me.clone(), progress()).await,
        Err(ApiError::Forbidden { .. })
    ));
    assert!(since(&h, before).is_empty());

    // Malformed or identical players: nothing leaves the device.
    let before = h.lms.rpc_calls().len();
    for bad in ["", "-", "../x", "aa:bb:cc:dd:ee", "AA-BB-CC-DD-EE-01"] {
        assert!(matches!(
            players.transfer(me.clone(), bad.into(), progress()).await,
            Err(ApiError::InvalidInput { .. })
        ));
        assert!(matches!(
            players.transfer(bad.into(), me.clone(), progress()).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    assert_eq!(
        players
            .transfer(me.clone(), me.to_uppercase(), progress())
            .await
            .unwrap(),
        TransferOutcome::Refused {
            reason: TransferRefusal::Same
        }
    );
    assert!(since(&h, before).is_empty());
}

/// The gateway test `transfer_recipe_needs_both_players`, through the app's session: every
/// command either recipe sends passes between permitted players, and is refused otherwise.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_transfer_step_passes_the_gateway_only_between_permitted_players() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let bobs = bob(&h).await.current().unwrap().device.player_id;
    h.accounts.grant_player("alice", HOME_PLAYER).unwrap();
    let raw = |v: Value| v.as_array().unwrap().clone();

    let steps = |source: &str, target: &str| {
        vec![
            (target.to_string(), json!(["power", 1])),
            (source.to_string(), json!(["sync", target])),
            (
                target.to_string(),
                json!(["status", "-", 1, "tags:aAlcdeJKjNoqrsTIyx"]),
            ),
            (source.to_string(), json!(["sync", "-"])),
            (source.to_string(), json!(["stop"])),
            (target.to_string(), json!(["pause", 0])),
            (target.to_string(), json!(["play"])),
            (
                source.to_string(),
                json!(["status", 0, 500, "tags:alcdJKj"]),
            ),
            (target.to_string(), json!(["playlist", "shuffle", 0])),
            (target.to_string(), json!(["playlist", "repeat", 2])),
            (
                target.to_string(),
                json!([
                    "playlistcontrol",
                    "cmd:load",
                    "track_id:11,12,13",
                    "play_index:1"
                ]),
            ),
            (
                target.to_string(),
                json!(["playlistcontrol", "cmd:add", "track_id:14,15"]),
            ),
            (target.to_string(), json!(["playlist", "index", 40])),
            (target.to_string(), json!(["time", 83])),
        ]
    };
    for (player, cmd) in steps(&me, HOME_PLAYER)
        .into_iter()
        .chain(steps(HOME_PLAYER, &me))
    {
        s.rpc_raw(&player, raw(cmd.clone()))
            .await
            .unwrap_or_else(|e| panic!("{player} {cmd} refused: {e:?}"));
    }

    let before = h.lms.rpc_calls().len();
    for (player, cmd) in steps(&me, &bobs) {
        let names_bob = player == bobs || cmd.to_string().contains(&bobs);
        let r = s.rpc_raw(&player, raw(cmd.clone())).await;
        if names_bob {
            assert!(
                matches!(r, Err(ApiError::Forbidden { .. })),
                "{player} {cmd} was not refused"
            );
        }
    }
    assert!(
        since(&h, before)
            .iter()
            .all(|c| !c.to_string().contains(&bobs)),
        "a command naming bob's player reached LMS"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_edits_pass_the_allow_list() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let players = Players::new(s.clone());

    players.remove_from_queue(me.clone(), 2).await.unwrap();
    players.move_in_queue(me.clone(), 3, 0).await.unwrap();
    players.clear_queue(me.clone()).await.unwrap();
    // The fake queue never grows, so Undo reports that it couldn't put the track back in
    // place, after adding it.
    let r = players.restore_to_queue(me.clone(), 7, 1).await;
    assert!(
        matches!(r, Err(ApiError::Server { status: 409, .. })),
        "{r:?}"
    );
    let calls = h.lms.rpc_calls();
    for want in [
        json!([me, ["playlist", "delete", "2"]]),
        json!([me, ["playlist", "move", "3", "0"]]),
        json!([me, ["playlist", "clear"]]),
        json!([me, ["playlistcontrol", "cmd:add", "track_id:7"]]),
    ] {
        assert!(calls.contains(&want), "{want} not sent");
    }

    // Another account's player: refused.
    let bobs = bob(&h).await.current().unwrap().device.player_id;
    for r in [
        players.remove_from_queue(bobs.clone(), 0).await,
        players.move_in_queue(bobs.clone(), 1, 0).await,
        players.clear_queue(bobs.clone()).await,
        players.restore_to_queue(bobs.clone(), 7, 0).await,
    ] {
        assert!(matches!(r, Err(ApiError::Forbidden { .. })), "{r:?}");
    }
    // Bad ids never leave the device.
    let before = h.lms.rpc_calls().len();
    assert!(matches!(
        players.restore_to_queue(me.clone(), 0, 0).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        players.clear_queue("../x".into()).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(since(&h, before).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_groups_join_in_the_right_direction() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let players = Players::new(s.clone());

    // Not permitted yet: refused.
    assert!(matches!(
        players
            .set_synced(me.clone(), HOME_PLAYER.into(), true)
            .await,
        Err(ApiError::Forbidden { .. })
    ));
    h.accounts.grant_player("alice", HOME_PLAYER).unwrap();
    players
        .set_synced(me.clone(), HOME_PLAYER.into(), true)
        .await
        .unwrap();
    players
        .set_synced(me.clone(), HOME_PLAYER.into(), false)
        .await
        .unwrap();
    let calls = h.lms.rpc_calls();
    // Joining is sent for the selected player, naming the one that joins; leaving for the
    // one that leaves.
    assert!(calls.contains(&json!([me, ["sync", HOME_PLAYER]])));
    assert!(calls.contains(&json!([HOME_PLAYER, ["sync", "-"]])));
    assert!(!calls.contains(&json!([HOME_PLAYER, ["sync", me]])));

    let before = h.lms.rpc_calls().len();
    assert!(matches!(
        players.set_synced(me.clone(), me.clone(), true).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(matches!(
        players.set_synced(me.clone(), "-".into(), true).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert!(since(&h, before).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checked_queue_edits_change_only_the_entry_seen() {
    // The gateway checks and changes in one step (the fake queue has track 1 at position 0).
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.current().unwrap().device.player_id;
    let players = Players::new(s.clone());
    assert!(players.remove_if_still(me.clone(), 0, 1).await.unwrap());
    assert!(players.move_if_still(me.clone(), 0, 1, 1).await.unwrap());
    let calls = h.lms.rpc_calls();
    for want in [
        json!([me, ["playlist", "delete", "0"]]),
        json!([me, ["playlist", "move", "0", "1"]]),
    ] {
        assert!(calls.contains(&want), "{want} not sent");
    }
    // Another track there now: nothing is changed.
    let before = h
        .lms
        .rpc_calls()
        .iter()
        .filter(|c| c[1][0] == "playlist")
        .count();
    assert!(!players.remove_if_still(me.clone(), 0, 2).await.unwrap());
    assert!(!players.move_if_still(me.clone(), 0, 1, 2).await.unwrap());
    let after = h
        .lms
        .rpc_calls()
        .iter()
        .filter(|c| c[1][0] == "playlist")
        .count();
    assert_eq!(before, after);
    // Another account's player, and bad input (never sent).
    let bobs = bob(&h).await.current().unwrap().device.player_id;
    assert!(matches!(
        players.remove_if_still(bobs, 0, 1).await,
        Err(ApiError::Forbidden { .. })
    ));
    let n = h.lms.rpc_calls().len();
    for r in [
        players.remove_if_still(me.clone(), 0, 0).await,
        players.remove_if_still(me.clone(), 200_000, 1).await,
        players.move_if_still(me.clone(), 2, 2, 1).await,
        players.remove_if_still("not a player".into(), 0, 1).await,
    ] {
        assert!(matches!(r, Err(ApiError::InvalidInput { .. })), "{r:?}");
    }
    assert_eq!(h.lms.rpc_calls().len(), n);
}
