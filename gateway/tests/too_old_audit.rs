//! Security (audit flooding): a refused old client is audited and warned about once an
//! hour per device.
//!
//! Its own test binary: the test reads the gateway's log lines through a subscriber set on
//! its thread, and tracing's per-callsite caching is only reliable for that when no other
//! test is logging from other threads at the same time.

mod common;
use common::*;

use serde_json::json;

/// Log lines written while the guard lives, on this thread (the test runtime runs every
/// task on it).
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, tracing::subscriber::DefaultGuard) {
    let buf: Arc<Mutex<Vec<u8>>> = Default::default();
    let b = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || Logs(b.clone()))
        .finish();
    (buf, tracing::subscriber::set_default(sub))
}

fn too_old_warnings(logs: &Mutex<Vec<u8>>) -> usize {
    String::from_utf8_lossy(&logs.lock().unwrap())
        .lines()
        .filter(|l| l.contains("player protocol too old"))
        .count()
}

fn too_old_events(h: &Harness) -> Vec<anywhere_gateway::accounts::AuditEntry> {
    h.accounts
        .audit_log(1000)
        .unwrap()
        .into_iter()
        .filter(|e| e.event == "player.too_old")
        .collect()
}

/// Sign in with `form` and expect the 4426 close.
async fn refused(h: &Harness, form: String) {
    let mut ws = h.dial().await;
    ws.send(Message::Text(form.into())).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no close in time")
        {
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(u16::from(f.code), 4426);
                return;
            }
            Some(Ok(_)) => continue,
            other => panic!("expected the 4426 close, got {other:?}"),
        }
    }
}

/// An app from before protocol 2 takes 4426 for a dropped connection and signs in again
/// every few seconds. Every sign-in is refused, but the device is audited and warned about
/// once, so it cannot push the rest of the audit trail out; another device gets its own
/// row; and once the device has attached with a current protocol, a refusal is news again.
/// (The hour itself is checked with a made-up clock in the unit tests.)
#[tokio::test]
async fn a_stale_device_reconnecting_is_audited_once() {
    let (logs, _guard) = capture_logs();
    let mut h = harness().await;
    for _ in 0..10 {
        refused(&h, json!({"token": h.tok_a, "protocol": 1}).to_string()).await;
    }
    h.lms.assert_no_conn().await;
    let events = too_old_events(&h);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].device_id, Some(h.dev_a));
    assert_eq!(too_old_warnings(&logs), 1);
    // Bob's device (the bare-token form): its own row and warning, once too.
    for _ in 0..3 {
        refused(&h, h.tok_b.clone()).await;
    }
    let events = too_old_events(&h);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].username.as_deref(), Some("bob"));
    assert_eq!(too_old_warnings(&logs), 2);
    // Alice updates: a current attach; an old build after that is news once more.
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    let dev = h.dev_a;
    wait_for(|| {
        h.accounts
            .list_devices()
            .unwrap()
            .iter()
            .any(|d| d.id == dev && d.protocol == Some(2))
    })
    .await;
    drop(ws);
    for _ in 0..3 {
        refused(&h, json!({"token": h.tok_a}).to_string()).await;
    }
    assert_eq!(too_old_events(&h).len(), 3);
    assert_eq!(too_old_warnings(&logs), 3);
}
