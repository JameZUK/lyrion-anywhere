//! Checked queue edits: a queue entry is removed or moved only while it is still the track
//! the client saw there (the fake LMS lists track 3 at position 1).

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

async fn edit(h: &Harness, token: Option<&str>, player: &str, body: Value) -> (S, Value) {
    let mut req = reqwest::Client::new()
        .post(h.url(&format!("/v1/players/{player}/queue/edit")))
        .json(&body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = req.send().await.unwrap();
    let s = r.status();
    (s, r.json().await.unwrap_or(Value::Null))
}

/// The queue-changing commands LMS was sent.
fn changes(h: &Harness) -> Vec<Vec<String>> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "playlist")
        .map(|c| {
            c[1].as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_str().unwrap_or("").to_string())
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn an_entry_is_changed_only_while_it_is_the_track_seen() {
    let h = harness().await;
    let me = format_mac(&h.mac_a);
    let (s, v) = edit(
        &h,
        Some(&h.tok_a),
        &me,
        json!({"op":"remove","index":1,"track":3}),
    )
    .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(changes(&h), [["playlist", "delete", "1"]]);
    let (s, _) = edit(
        &h,
        Some(&h.tok_a),
        &me,
        json!({"op":"move","index":1,"to":0,"track":3}),
    )
    .await;
    assert_eq!(s, S::OK);
    assert_eq!(changes(&h).last().unwrap(), &["playlist", "move", "1", "0"]);
    // Another track there now, or another position: nothing changes.
    let before = changes(&h).len();
    for body in [
        json!({"op":"remove","index":1,"track":4}),
        json!({"op":"remove","index":2,"track":3}),
        json!({"op":"move","index":2,"to":0,"track":3}),
    ] {
        let (s, v) = edit(&h, Some(&h.tok_a), &me, body.clone()).await;
        assert_eq!(s, S::CONFLICT, "{body}: {v}");
        assert_eq!(v["error"], "the queue changed");
    }
    assert_eq!(changes(&h).len(), before, "nothing was changed");
}

#[tokio::test]
async fn other_players_and_bad_requests_are_refused_before_lms() {
    let h = harness().await;
    let me = format_mac(&h.mac_a);
    let theirs = format_mac(&h.mac_b);
    let good = json!({"op":"remove","index":1,"track":3});
    assert_eq!(edit(&h, None, &me, good.clone()).await.0, S::UNAUTHORIZED);
    assert_eq!(
        edit(&h, Some("nope"), &me, good.clone()).await.0,
        S::UNAUTHORIZED
    );
    assert_eq!(
        edit(&h, Some(&h.tok_a), &theirs, good.clone()).await.0,
        S::FORBIDDEN
    );
    for player in ["not-a-player", "aa:bb:cc:dd:ee:zz", "..%2F..%2Fx"] {
        assert_eq!(
            edit(&h, Some(&h.tok_a), player, good.clone()).await.0,
            S::FORBIDDEN,
            "{player}"
        );
    }
    for body in [
        json!({"op":"clear"}),
        json!({"op":"remove","index":1}),
        json!({"op":"remove","index":-1,"track":3}),
        json!({"op":"remove","index":1,"track":3,"cmd":["pref","x"]}),
        json!({"op":"move","index":1,"to":1,"track":3}),
        json!("playlist delete 1"),
    ] {
        assert_eq!(
            edit(&h, Some(&h.tok_a), &me, body.clone()).await.0,
            S::BAD_REQUEST,
            "{body}"
        );
    }
    assert!(
        h.lms
            .rpc_calls()
            .iter()
            .all(|c| c[1][0] != "playlist" && c[1][0] != "status"),
        "LMS saw nothing: {:?}",
        h.lms.rpc_calls()
    );
}
