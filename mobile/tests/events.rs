//! The event stream against the real gateway: delivery, reconnects after a dropped
//! connection, token renewal, and final closes.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anywhere_mobile::{Events, EventsState, Server, ServerEvent, Session, StoredCredentials};
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
        matches!(e, ServerEvent::PlayerChanged { player_id, name, args }
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
        |e| matches!(e, ServerEvent::LibraryChanged { name, .. } if name == "rescan"),
    )
    .await;
    assert!(!c.events().iter().any(
        |e| matches!(e, ServerEvent::PlayerChanged { player_id, .. } if player_id == HOME_PLAYER)
    ));

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
        |e| matches!(e, ServerEvent::PlayerChanged { name, .. } if name == "pause"),
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

/// A fake gateway whose event socket always closes with 1008 "unauthorised", and whose
/// refresh endpoint always hands out fresh tokens. Counts both.
#[derive(Default)]
struct Refusing {
    refreshes: AtomicUsize,
    sockets: AtomicUsize,
}

async fn refusing_gateway() -> (Arc<Refusing>, std::net::SocketAddr) {
    use axum::extract::ws::{CloseFrame, Message, WebSocketUpgrade};
    use axum::routing::{any, post};

    let counts = Arc::new(Refusing::default());
    let (c1, c2) = (counts.clone(), counts.clone());
    let app = axum::Router::new()
        .route(
            "/v1/auth/refresh",
            post(move || {
                let c = c1.clone();
                async move {
                    let n = c.refreshes.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({
                        "access_token": format!("access-{n}"),
                        "refresh_token": format!("refresh-{n}"),
                        "expires_in": 900,
                        "user": {"username": "alice", "role": "listener"},
                        "device": {"id": 1, "name": "Phone", "mac": "02:4c:41:00:00:01"},
                    }))
                }
            }),
        )
        .route(
            "/v1/events",
            any(move |ws: WebSocketUpgrade| {
                let c = c2.clone();
                async move {
                    c.sockets.fetch_add(1, Ordering::SeqCst);
                    ws.on_upgrade(|mut s| async move {
                        let _ = s.recv().await;
                        let _ = s
                            .send(Message::Close(Some(CloseFrame {
                                code: 1008,
                                reason: "unauthorised".into(),
                            })))
                            .await;
                        let _ = s.recv().await;
                    })
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (counts, addr)
}

/// M8 regression: a gateway that keeps refusing the event socket as "unauthorised" after
/// every renewal gets one immediate retry, then the normal backoff, not a tight loop of
/// refreshes and secure-store saves.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gateway_that_keeps_refusing_gets_backoff_not_a_refresh_loop() {
    let (counts, addr) = refusing_gateway().await;
    let store = MemStore::with(StoredCredentials {
        refresh_token: "refresh-start".into(),
        device_id: 1,
    });
    let server = Server::new(format!("http://{addr}")).unwrap();
    let s = Session::new(server, store.clone(), "Phone".into()).unwrap();
    let c = Arc::new(Collector::default());
    let _sub = Events::new(s.clone()).subscribe(c.clone()).await;
    wait_for("backing off", || {
        c.states()
            .iter()
            .any(|s| matches!(s, EventsState::Retrying { .. }))
    })
    .await;
    // Before the first wait: the first token, then at most two renewals.
    let at_first_wait = counts.refreshes.load(Ordering::SeqCst);
    assert!(
        at_first_wait <= 3,
        "{at_first_wait} refreshes before backing off"
    );
    let delay = c
        .states()
        .iter()
        .find_map(|s| match s {
            EventsState::Retrying { delay_ms } => Some(*delay_ms),
            _ => None,
        })
        .unwrap();
    assert!(delay >= 500, "a real delay: {delay} ms");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let refreshes = counts.refreshes.load(Ordering::SeqCst);
    let sockets = counts.sockets.load(Ordering::SeqCst);
    assert!(refreshes <= 6, "{refreshes} refreshes in about three seconds");
    assert!(sockets <= 5, "{sockets} connections in about three seconds");
    assert!(store.saves() <= 6, "{} secure-store saves", store.saves());
}
