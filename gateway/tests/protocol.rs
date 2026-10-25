//! The player protocol version (protocol 2, docs/plans/signal-path-redesign.md): `GET /v1/me`
//! says it, the player socket's sign-in form must give it, and the device lists show what
//! each device last attached with.

mod common;
use common::*;

use anywhere_gateway::admin;
use serde_json::{Value, json};

/// Every sign-in form that is too old or not a whole number of at least 2, as the
/// `protocol` field (`None`: left out) with what the audit trail records.
fn refused() -> Vec<(Option<&'static str>, &'static str)> {
    vec![
        (None, "none"),
        (Some("1"), "1"),
        (Some(r#""2""#), r#""2""#),
        (Some("2.0"), "2.0"),
        (Some("-1"), "-1"),
        (Some("0"), "0"),
        (Some("99999999999999999999"), "1e+20"),
        (Some("true"), "true"),
        (Some("null"), "null"),
        (Some("[2]"), "[2]"),
        (Some("{}"), "{}"),
    ]
}

/// A sign-in form with `protocol` spliced in as raw JSON (or left out), and `rest` (raw
/// JSON members, may be empty).
fn form(token: &str, protocol: Option<&str>, rest: &str) -> String {
    let mut members = vec![format!(r#""token":{}"#, json!(token))];
    if let Some(p) = protocol {
        members.push(format!(r#""protocol":{p}"#));
    }
    if !rest.is_empty() {
        members.push(rest.to_string());
    }
    format!("{{{}}}", members.join(","))
}

/// The close the socket ends with: its code and reason.
async fn close_of(ws: &mut Ws) -> (u16, String) {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("no close in time")
        {
            Some(Ok(Message::Close(Some(f)))) => return (u16::from(f.code), f.reason.to_string()),
            Some(Ok(Message::Close(None))) | None => panic!("closed without a code"),
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("socket error instead of a close: {e}"),
        }
    }
}

fn too_old_events(h: &Harness) -> Vec<anywhere_gateway::accounts::AuditEntry> {
    h.accounts
        .audit_log(1000)
        .unwrap()
        .into_iter()
        .filter(|e| e.event == "player.too_old")
        .collect()
}

#[tokio::test]
async fn me_says_the_protocol() {
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
    assert_eq!(me["protocol"], 2);
    assert_eq!(anywhere_gateway::PROTOCOL, 2);
}

/// Without `protocol`, below 2, or anything but a whole JSON number: closed with 4426
/// `update the app`, after the token was checked and before any session is opened (LMS
/// never hears of it), with an audit event saying what was given.
#[tokio::test]
async fn an_old_or_malformed_protocol_is_told_to_update() {
    let mut h = harness().await;
    let mut cases: Vec<(String, &str)> = refused()
        .into_iter()
        .map(|(p, given)| (form(&h.tok_a, p, ""), given))
        .collect();
    // The bare token (protocol 1's form) and a form with other fields but no protocol.
    cases.push((h.tok_a.clone(), "none"));
    cases.push((
        form(&h.tok_a, None, r#""quality":"high","keepalive":true"#),
        "none",
    ));
    for (i, (text, given)) in cases.iter().enumerate() {
        let mut ws = h.dial().await;
        ws.send(Message::Text(text.clone().into())).await.unwrap();
        assert_eq!(
            close_of(&mut ws).await,
            (4426, "update the app".to_string()),
            "{text}"
        );
        h.lms.assert_no_conn().await;
        let events = too_old_events(&h);
        assert_eq!(events.len(), i + 1, "{text}");
        let e = &events[0]; // newest first
        assert_eq!(e.username.as_deref(), Some("alice"));
        assert_eq!(e.device_id, Some(h.dev_a));
        assert_eq!(e.detail, *given, "{text}");
    }
}

/// The token is checked first: a bad token with an old protocol is unauthorised, as ever,
/// and nothing is audited as too old.
#[tokio::test]
async fn the_token_is_checked_before_the_protocol() {
    let mut h = harness().await;
    for text in [
        form("not-a-token", Some("1"), ""),
        form("not-a-token", None, ""),
        "not-a-token".to_string(),
    ] {
        let mut ws = h.dial().await;
        ws.send(Message::Text(text.into())).await.unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
    }
    h.lms.assert_no_conn().await;
    assert!(too_old_events(&h).is_empty());
}

/// 2, and any higher whole number (the newer client decides), get the usual greeting.
#[tokio::test]
async fn protocol_two_and_above_are_greeted() {
    for p in ["2", "3", "18446744073709551615"] {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        let sid = connect_form(&mut ws, &form(&h.tok_a, Some(p), "")).await;
        assert_eq!(sid.len(), 32, "{p}");
        let _lms = h.lms.accept().await;
        assert!(too_old_events(&h).is_empty());
    }
}

/// A resume form is held to the same rule. The session it names is left alone: it can
/// still be resumed by a current client within its grace.
#[tokio::test]
async fn a_resume_form_must_give_the_protocol_too() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    let sid_line = sign_in_form(&h.tok_a);
    ws.send(Message::Text(sid_line.into())).await.unwrap();
    let g: Value = match next(&mut ws).await {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("expected greeting, got {other:?}"),
    };
    let (sid, secret) = (
        g["session"].as_str().unwrap().to_string(),
        g["secret"].as_str().unwrap().to_string(),
    );
    let _lms = h.lms.accept().await;
    drop(ws);
    let resume = format!(
        r#""resume":{{"session":{},"secret":{},"received":0}}"#,
        json!(sid),
        json!(secret)
    );
    for (i, (p, given)) in refused().into_iter().enumerate() {
        let mut ws = h.dial().await;
        ws.send(Message::Text(form(&h.tok_a, p, &resume).into()))
            .await
            .unwrap();
        assert_eq!(close_of(&mut ws).await, (4426, "update the app".into()));
        let events = too_old_events(&h);
        assert_eq!(events.len(), i + 1);
        assert_eq!(events[0].detail, given);
    }
    h.lms.assert_no_conn().await;
    for p in ["2", "3"] {
        let mut ws = h.dial().await;
        ws.send(Message::Text(form(&h.tok_a, Some(p), &resume).into()))
            .await
            .unwrap();
        let g: Value = match next(&mut ws).await {
            Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
            other => panic!("expected greeting, got {other:?}"),
        };
        assert_eq!(g["session"], sid.as_str(), "{p}");
        assert_eq!(g["resumed"], true);
    }
    h.lms.assert_no_conn().await;
}

/// The harness's form helpers: the default carries the protocol; it can be replaced or
/// left out.
#[test]
fn the_form_helpers() {
    let v: Value = serde_json::from_str(&sign_in_form("t")).unwrap();
    assert_eq!(v, json!({"token": "t", "protocol": 2}));
    let v: Value =
        serde_json::from_str(&sign_in_form_with("t", json!({"protocol": 1, "grace": 5}))).unwrap();
    assert_eq!(v, json!({"token": "t", "protocol": 1, "grace": 5}));
    let v: Value =
        serde_json::from_str(&sign_in_form_with("t", json!({"protocol": null}))).unwrap();
    assert_eq!(v, json!({"token": "t"}));
}

// ---------------------------------------------------------------------------
// The devices' protocol, in both device lists
// ---------------------------------------------------------------------------

async fn admin_api(h: &Harness) -> (String, String) {
    let secret = "s".repeat(64);
    let app = admin::router(h.gw.clone(), secret.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    (format!("http://{addr}"), secret)
}

/// Each device's `protocol` as the admin list shows it, by username.
async fn admin_protocols(h: &Harness) -> Vec<(String, Value)> {
    let (base, secret) = admin_api(h).await;
    let v: Value = reqwest::Client::new()
        .get(format!("{base}/admin/v1/devices"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            assert!(d.get("protocol").is_some(), "{d}");
            (
                d["username"].as_str().unwrap().to_string(),
                d["protocol"].clone(),
            )
        })
        .collect()
}

/// The user's own list (`GET /v1/devices`): `protocol` of the current device.
async fn own_protocol(h: &Harness, token: &str) -> Value {
    let v: Value = reqwest::Client::new()
        .get(h.url("/v1/devices"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mine = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["current"] == true)
        .unwrap()
        .clone();
    assert!(mine.get("protocol").is_some(), "{mine}");
    mine["protocol"].clone()
}

/// Wait until alice's device has `want` recorded (written off the socket's task), then
/// check her own list says so.
async fn wait_for_protocol(h: &Harness, want: Value) {
    let want_db = want.as_i64();
    let dev = h.dev_a;
    wait_for(|| {
        h.accounts
            .list_devices()
            .unwrap()
            .iter()
            .any(|d| d.id == dev && d.protocol == want_db)
    })
    .await;
    assert_eq!(own_protocol(h, &h.tok_a).await, want);
}

#[tokio::test]
async fn devices_show_the_protocol_they_last_attached_with() {
    let mut h = harness().await;
    // Never attached: null in both lists.
    assert_eq!(
        admin_protocols(&h).await,
        vec![("alice".into(), Value::Null), ("bob".into(), Value::Null)]
    );
    assert_eq!(own_protocol(&h, &h.tok_a).await, Value::Null);
    // A refused attach records what it spoke, when that is a protocol at all: the old
    // forms without a number are protocol 1, so an admin sees who is behind.
    for (given, want) in [
        (Some("0"), json!(0)),
        (None, json!(1)),
        (Some(r#""2""#), json!(1)), // junk: not a protocol, left as it was
        (Some("2.0"), json!(1)),
    ] {
        let mut ws = h.dial().await;
        ws.send(Message::Text(form(&h.tok_a, given, "").into()))
            .await
            .unwrap();
        assert_eq!(close_of(&mut ws).await.0, 4426);
        wait_for_protocol(&h, want).await;
    }
    let mut ws = h.dial().await;
    ws.send(Message::Text(h.tok_a.clone().into()))
        .await
        .unwrap();
    assert_eq!(close_of(&mut ws).await.0, 4426);
    wait_for_protocol(&h, json!(1)).await;
    assert_eq!(
        admin_protocols(&h).await,
        vec![("alice".into(), json!(1)), ("bob".into(), Value::Null)]
    );

    // Attached with 3: recorded, alice's only.
    let mut ws = h.dial().await;
    connect_form(&mut ws, &form(&h.tok_a, Some("3"), "")).await;
    let _lms = h.lms.accept().await;
    wait_for_protocol(&h, json!(3)).await;
    assert_eq!(
        admin_protocols(&h).await,
        vec![("alice".into(), json!(3)), ("bob".into(), Value::Null)]
    );
    assert_eq!(own_protocol(&h, &h.tok_b).await, Value::Null);
    // Every attach records it again: a new session with 2.
    drop(ws);
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    wait_for_protocol(&h, json!(2)).await;
    assert_eq!(admin_protocols(&h).await[0], ("alice".into(), json!(2)));
}

/// A protocol beyond SQLite's integer range is stored as its largest, never refused.
#[tokio::test]
async fn a_huge_protocol_is_stored_bounded() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect_form(&mut ws, &form(&h.tok_a, Some("18446744073709551615"), "")).await;
    let _lms = h.lms.accept().await;
    wait_for_protocol(&h, json!(i64::MAX)).await;
}
