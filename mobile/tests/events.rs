//! The event stream against the real gateway: delivery, reconnects after a dropped
//! connection, token renewal, and final closes.

mod common;

use std::sync::Arc;
use std::time::Duration;

use anywhere_mobile::{Events, EventsState, ServerEvent, Session};
use common::*;

/// Wait until the gateway is listening to the fake LMS CLI, then send `line` until the
/// collector sees an event matching `want` (the subscription may still be settling).
async fn deliver(h: &Harness, c: &Collector, line: &str, want: impl Fn(&ServerEvent) -> bool) {
    for _ in 0..100 {
        if h.lms.cli_listeners().await > 0 {
            h.lms.cli_send(line).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        if c.events().iter().any(&want) {
            return;
        }
    }
    panic!("event never arrived: {line}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_arrive_and_resume_after_a_dropped_connection() {
    let h = Harness::start().await;
    let proxy = Proxy::start(h.addr).await;
    let store = MemStore::new();
    let s = Session::new(proxy.server(), store, "Phone".into()).unwrap();
    s.login("alice".into(), PW.into(), None).await.unwrap();
    let me = s.current().unwrap().device.player_id;

    let c = Arc::new(Collector::default());
    let sub = Events::new(s.clone()).subscribe(c.clone()).await;
    wait_for("live", || c.live_count() == 1).await;
    // Connecting always starts with a resync.
    assert_eq!(c.events().first(), Some(&ServerEvent::Resync));

    let mac_escaped = me.replace(':', "%3A");
    deliver(&h, &c, &format!("{mac_escaped} mixer volume 50"), |e| {
        matches!(e, ServerEvent::Player { player_id, name, args }
            if *player_id == me && name == "mixer" && args == &["volume", "50"])
    })
    .await;
    // Events for players this user may not control are never delivered.
    h.lms
        .cli_send("aa%3Abb%3Acc%3Add%3Aee%3A01 mixer volume 1")
        .await;
    deliver(
        &h,
        &c,
        "rescan done",
        |e| matches!(e, ServerEvent::Library { name, .. } if name == "rescan"),
    )
    .await;
    assert!(
        !c.events().iter().any(
            |e| matches!(e, ServerEvent::Player { player_id, .. } if player_id == HOME_PLAYER)
        )
    );

    // The phone loses its connection: the stream comes back by itself.
    proxy.cut_all();
    wait_for("retrying", || {
        c.states()
            .iter()
            .any(|s| matches!(s, EventsState::Retrying { .. }))
    })
    .await;
    wait_for("live again", || c.live_count() == 2).await;
    let n = c.events().len();
    deliver(
        &h,
        &c,
        &format!("{mac_escaped} pause 1"),
        |e| matches!(e, ServerEvent::Player { name, .. } if name == "pause"),
    )
    .await;
    assert!(c.events().len() > n);
    assert!(sub.is_active());

    // Cancelling stops delivery.
    sub.cancel();
    assert!(!sub.is_active());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let n = c.events().len();
    h.lms.cli_send(&format!("{mac_escaped} stop")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c.events().len(), n);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_token_is_renewed_and_the_stream_reconnects() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let saves = store.saves();
    s.debug_invalidate_access_token();
    let c = Arc::new(Collector::default());
    let _sub = Events::new(s.clone()).subscribe(c.clone()).await;
    wait_for("live", || c.live_count() == 1).await;
    // The first attempt used the stale token; the session refreshed once.
    assert!(store.saves() > saves);
    assert!(
        !c.states()
            .iter()
            .any(|s| matches!(s, EventsState::Stopped { .. }))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_connection_stops_for_good() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let events = Events::new(s.clone());
    let first = Arc::new(Collector::default());
    let _a = events.subscribe(first.clone()).await;
    wait_for("first live", || first.live_count() == 1).await;
    // A device may hold 4 event sockets: the fifth replaces the oldest.
    let mut others = Vec::new();
    for _ in 0..4 {
        let c = Arc::new(Collector::default());
        others.push((events.subscribe(c.clone()).await, c.clone()));
        wait_for("live", || c.live_count() == 1).await;
    }
    wait_for("the first to stop", || {
        first
            .states()
            .iter()
            .any(|s| matches!(s, EventsState::Stopped { reason } if reason.contains("replaced")))
    })
    .await;
    // It does not reconnect (which would evict another in turn).
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(first.live_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_out_session_stops_the_stream() {
    let h = Harness::start().await;
    let s = h.session(MemStore::new());
    let c = Arc::new(Collector::default());
    let _sub = Events::new(s.clone()).subscribe(c.clone()).await;
    wait_for("stopped", || {
        c.states()
            .iter()
            .any(|s| matches!(s, EventsState::Stopped { reason } if reason == "signed out"))
    })
    .await;
}
