//! M7: a database write that stalls (a disk that has stopped answering) must not stall the
//! gateway. Device bookkeeping and token renewal run on tokio's blocking pool, never on an
//! async worker, so other requests, player sessions and their heartbeats carry on.
//!
//! These tests run the whole gateway on the test's single-threaded runtime, so a write made
//! inline on that thread would hold up every other request until it finished: each test
//! stalls the writes, starts a request that makes one, and times an unrelated `/healthz`
//! from before it started.
//!
//! Reads have a connection of their own (SQLite's WAL lets them run beside the writer), so
//! a request that only reads doesn't wait for a stalled write at all, and the loopback admin
//! API's database work runs on the blocking pool too.

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

// ---------------------------------------------------------------------------
// Uploads of plays, downloads and clearing the history (review M16)
// ---------------------------------------------------------------------------

/// Keep the database's write side busy, back to back, with `rounds` stalled writes from a
/// thread of its own (token renewals of a device of alice's), so that any database work a
/// request does inline meets one of them and holds up the test's only thread.
fn keep_the_writer_busy(h: &Harness, rounds: usize) -> std::thread::JoinHandle<()> {
    let accounts = h.accounts.clone();
    let t = accounts
        .login("alice", PW, None, "Busy", None, "test")
        .unwrap();
    // Bob's device makes no last-seen write of its own here.
    let b = accounts.authenticate(&h.tok_b).unwrap().device_id;
    assert!(accounts.claim_seen(b));
    accounts.stall_writes_for_test(STALL);
    std::thread::spawn(move || {
        let mut refresh = t.refresh;
        for _ in 0..rounds {
            match accounts.refresh(&refresh) {
                Ok(t) => refresh = t.refresh,
                Err(_) => break,
            }
        }
        accounts.stall_writes_for_test(Duration::ZERO);
    })
}

/// Run `request` while timing `/healthz` over and over: it must answer promptly throughout.
async fn healthz_stays_prompt_during(
    h: &Harness,
    request: impl std::future::Future<Output = S> + Send + 'static,
) -> S {
    // Long enough for the busy thread to have taken the write side.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let task = tokio::spawn(request);
    let mut worst = Duration::ZERO;
    while !task.is_finished() {
        let t = Instant::now();
        assert_eq!(healthz(h).await, S::OK);
        worst = worst.max(t.elapsed());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        worst < PROMPT,
        "an unrelated request waited {worst:?} for another's database work"
    );
    task.await.unwrap()
}

#[tokio::test]
async fn an_upload_of_plays_on_a_stalled_disk_does_not_hold_up_other_requests() {
    let h = harness().await;
    let busy = keep_the_writer_busy(&h, 4);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (url, token) = (h.url("/v1/plays"), h.tok_b.clone());
    let upload = async move {
        reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(&json!([{"key": "stalled-play-1", "track_id": 1,
                            "played_at": now - 900, "seconds": 300}]))
            .send()
            .await
            .unwrap()
            .status()
    };
    assert_eq!(healthz_stays_prompt_during(&h, upload).await, S::OK);
    busy.join().unwrap();
}

#[tokio::test]
async fn clearing_the_history_on_a_stalled_disk_does_not_hold_up_other_requests() {
    let h = harness().await;
    let busy = keep_the_writer_busy(&h, 4);
    let (url, token) = (h.url("/v1/me/history"), h.tok_b.clone());
    let clear = async move {
        reqwest::Client::new()
            .delete(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    };
    assert_eq!(healthz_stays_prompt_during(&h, clear).await, S::OK);
    busy.join().unwrap();
    assert!(
        h.accounts
            .audit_log(100)
            .unwrap()
            .iter()
            .any(|e| e.event == "history.cleared" && e.username.as_deref() == Some("bob")),
        "still audited"
    );
}

#[tokio::test]
async fn a_download_on_a_stalled_disk_does_not_hold_up_other_requests() {
    let h = harness().await;
    let busy = keep_the_writer_busy(&h, 4);
    let (url, token) = (h.url("/v1/download/track/1"), h.tok_b.clone());
    let download = async move {
        let r = reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        let status = r.status();
        let _ = r.bytes().await;
        status
    };
    assert_eq!(healthz_stays_prompt_during(&h, download).await, S::OK);
    busy.join().unwrap();
    assert!(
        h.accounts
            .audit_log(100)
            .unwrap()
            .iter()
            .any(|e| e.event == "download" && e.username.as_deref() == Some("bob")),
        "still audited"
    );
}

// ---------------------------------------------------------------------------
// Reads have their own connection; admin writes run off the async workers
// ---------------------------------------------------------------------------

/// Stall the writes and start one from device A that holds the database's write side for
/// the whole stall (its first authenticated request records where it was seen). Device B's
/// own last-seen write is counted as done first, so B's requests make no write of their own.
async fn hold_the_writer(h: &Harness) -> tokio::task::JoinHandle<S> {
    let b = h.accounts.authenticate(&h.tok_b).unwrap().device_id;
    assert!(h.accounts.claim_seen(b));
    h.accounts.stall_writes_for_test(STALL);
    let (url, token) = (h.url("/healthz"), h.tok_a.clone());
    let held = tokio::spawn(async move {
        reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    });
    // Long enough for the write to have started (and taken the write side).
    tokio::time::sleep(Duration::from_millis(100)).await;
    held
}

async fn get_as(h: &Harness, token: &str, path: &str) -> S {
    reqwest::Client::new()
        .get(h.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_stalled_write_does_not_hold_up_requests_that_only_read() {
    let h = harness().await;
    let mine = format_mac(&h.mac_b);
    let held = hold_the_writer(&h).await;
    let started = Instant::now();
    // The player permission check every RPC makes first.
    let rpc = reqwest::Client::new()
        .post(h.url("/v1/rpc"))
        .bearer_auth(&h.tok_b)
        .json(&json!({ "player": mine, "cmd": ["status", 0, 10] }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(rpc, S::OK);
    // Account settings (two-step state, download permission), the user's own devices and
    // the per-user stores.
    for path in [
        "/v1/account",
        "/v1/devices",
        "/v1/me/history",
        "/v1/me/ratings/style",
        "/v1/me/filters",
        "/v1/me/home",
    ] {
        assert_eq!(get_as(&h, &h.tok_b, path).await, S::OK, "{path}");
    }
    assert!(
        started.elapsed() < PROMPT,
        "reads waited {:?} for a stalled write",
        started.elapsed()
    );
    assert_eq!(held.await.unwrap(), S::OK);
}

/// Start `request` (which needs the database's write side) while device A's stalled write
/// holds it, and check that an unrelated request is answered at once meanwhile: the write
/// waits on the blocking pool, not on the test's only async thread (review M16).
async fn a_write_waiting_for_the_database_is_off_the_workers(
    h: &Harness,
    request: reqwest::RequestBuilder,
    want: S,
    what: &str,
) {
    let held = hold_the_writer(h).await;
    let started = Instant::now();
    let write = tokio::spawn(async move { request.send().await.unwrap().status() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(healthz(h).await, S::OK);
    assert!(
        started.elapsed() < PROMPT,
        "an unrelated request waited {:?} for {what}",
        started.elapsed()
    );
    assert_eq!(write.await.unwrap(), want, "{what}");
    assert_eq!(held.await.unwrap(), S::OK);
}

/// The 6-digit code for a base32 secret now.
fn current_code(secret_b32: &str) -> String {
    let mut bits = (0u64, 0u32);
    let mut secret = Vec::new();
    for c in secret_b32.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("bad base32"),
        };
        bits = ((bits.0 << 5) | u64::from(v), bits.1 + 5);
        if bits.1 >= 8 {
            bits.1 -= 8;
            secret.push((bits.0 >> bits.1) as u8);
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!(
        "{:06}",
        anywhere_gateway::totp::code_at(&secret, now / anywhere_gateway::totp::STEP_SECS)
    )
}

/// Regression (review M16): confirming two-step set-up wrote (the code's step, the switch,
/// the audit row) inline on an async worker.
#[tokio::test]
async fn a_two_step_confirmation_waiting_for_the_database_does_not_hold_up_other_requests() {
    let h = harness().await;
    let bob = h.accounts.authenticate(&h.tok_b).unwrap();
    let setup = h
        .accounts
        .totp_begin(bob.user_id, "bob", bob.device_id, PW)
        .unwrap();
    let request = reqwest::Client::new()
        .post(h.url("/v1/account/totp/confirm"))
        .header("x-anywhere-client", "1")
        .bearer_auth(&h.tok_b)
        .json(&json!({ "code": current_code(&setup.secret) }));
    a_write_waiting_for_the_database_is_off_the_workers(
        &h,
        request,
        S::NO_CONTENT,
        "a two-step confirmation",
    )
    .await;
    assert!(h.accounts.totp_enabled(bob.user_id).unwrap());
}

/// Regression (review M16): a diagnostics report was stored inline on an async worker.
#[tokio::test]
async fn a_diagnostics_report_waiting_for_the_database_does_not_hold_up_other_requests() {
    let h = harness().await;
    let request = reqwest::Client::new()
        .post(h.url("/v1/diagnostics"))
        .bearer_auth(&h.tok_b)
        .json(&json!({ "playback": { "mode": "media" } }));
    a_write_waiting_for_the_database_is_off_the_workers(
        &h,
        request,
        S::NO_CONTENT,
        "a diagnostics report",
    )
    .await;
    let bob = h.accounts.authenticate(&h.tok_b).unwrap();
    assert!(h.accounts.diagnostics(bob.device_id).unwrap().is_some());
}

/// The loopback admin API, served as the gateway serves it.
async fn admin_api(h: &Harness) -> (String, String) {
    let secret = "s".repeat(64);
    let app = anywhere_gateway::admin::router(h.gw.clone(), secret.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    (format!("http://{addr}"), secret)
}

#[tokio::test]
async fn a_stalled_write_does_not_hold_up_admin_reads() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let held = hold_the_writer(&h).await;
    let started = Instant::now();
    for path in [
        "/admin/v1/users",
        "/admin/v1/devices",
        "/admin/v1/settings",
        "/admin/v1/audit",
        "/admin/v1/users/alice/players",
    ] {
        let r = reqwest::Client::new()
            .get(format!("{base}{path}"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::OK, "{path}");
    }
    assert!(
        started.elapsed() < PROMPT,
        "admin reads waited {:?} for a stalled write",
        started.elapsed()
    );
    assert_eq!(held.await.unwrap(), S::OK);
}

#[tokio::test]
async fn an_admin_write_waiting_for_the_database_does_not_hold_up_other_requests() {
    let h = harness().await;
    let (base, secret) = admin_api(&h).await;
    let held = hold_the_writer(&h).await;
    let started = Instant::now();
    // This write has to wait for the stalled one: made inline, it would hold up the async
    // worker (here the test's only thread) until then.
    let write = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base}/admin/v1/settings"))
            .bearer_auth(&secret)
            .json(&json!({ "session_days": 30 }))
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(healthz(&h).await, S::OK);
    assert!(
        started.elapsed() < PROMPT,
        "an unrelated request waited {:?} for an admin write",
        started.elapsed()
    );
    assert_eq!(write.await.unwrap(), S::NO_CONTENT);
    assert_eq!(held.await.unwrap(), S::OK);
    assert_eq!(h.accounts.session_days(), 30);
}
