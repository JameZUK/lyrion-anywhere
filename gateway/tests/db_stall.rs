//! M7: a database write that stalls (a disk that has stopped answering) must not stall the
//! gateway. Device bookkeeping and token renewal run on tokio's blocking pool, never on an
//! async worker, so other requests, player sessions and their heartbeats carry on.
//!
//! These tests run the whole gateway on the test's single-threaded runtime, so a write made
//! inline on that thread would hold up every other request until it finished: each test
//! stalls the writes, starts a request that makes one, and times an unrelated `/healthz`
//! from before it started.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::json;
use std::time::Instant;

const STALL: Duration = Duration::from_millis(1500);
/// Far below the stall: an unrelated request answers at once.
const PROMPT: Duration = Duration::from_millis(700);

async fn healthz(h: &Harness) -> S {
    reqwest::Client::new()
        .get(h.url("/healthz"))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_stalled_token_renewal_does_not_hold_up_other_requests() {
    let h = harness().await;
    let t = h
        .accounts
        .login("alice", PW, None, "Stalling", None, "test")
        .unwrap();
    h.accounts.stall_writes_for_test(STALL);
    let started = Instant::now();
    let renewal = {
        let url = h.url("/v1/auth/refresh");
        tokio::spawn(async move {
            reqwest::Client::new()
                .post(url)
                .header("x-anywhere-client", "1")
                .json(&json!({ "refresh_token": t.refresh }))
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(healthz(&h).await, S::OK);
    assert!(
        started.elapsed() < PROMPT,
        "an unrelated request waited {:?} for a stalled renewal",
        started.elapsed()
    );
    // The renewal itself still completes once the disk answers.
    assert_eq!(renewal.await.unwrap(), S::OK);
}

#[tokio::test]
async fn a_stalled_player_attach_does_not_hold_up_other_requests() {
    let h = harness().await;
    h.accounts.stall_writes_for_test(STALL);
    let started = Instant::now();
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(healthz(&h).await, S::OK);
    assert!(
        started.elapsed() < PROMPT,
        "an unrelated request waited {:?} for a stalled player attach",
        started.elapsed()
    );
    let _ = ws.close(None).await;
}

#[tokio::test]
async fn a_stalled_last_seen_write_does_not_hold_up_other_requests() {
    let h = harness().await;
    h.accounts.stall_writes_for_test(STALL);
    let started = Instant::now();
    // The first authenticated request from a device records where it was seen.
    let seen = {
        let (url, token) = (h.url("/healthz"), h.tok_a.clone());
        tokio::spawn(async move {
            reqwest::Client::new()
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(healthz(&h).await, S::OK);
    assert!(
        started.elapsed() < PROMPT,
        "an unrelated request waited {:?} for a stalled last-seen write",
        started.elapsed()
    );
    assert_eq!(seen.await.unwrap(), S::OK);
}
