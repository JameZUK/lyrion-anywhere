//! Player tunnel and stream relay hardening: client opcode filtering, HELO sanitising,
//! stream cancellation and stalls, slow clients and replaced sockets.

mod common;
use common::*;

async fn greeting(ws: &mut Ws) -> serde_json::Value {
    match next(ws).await {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("expected greeting, got {other:?}"),
    }
}

async fn open_session(ws: &mut Ws, token: &str) -> (String, String) {
    ws.send(Message::Text(sign_in_form(token).into()))
        .await
        .unwrap();
    let g = greeting(ws).await;
    (
        g["session"].as_str().unwrap().to_string(),
        g["secret"].as_str().unwrap().to_string(),
    )
}

fn resume_after(token: &str, sid: &str, secret: &str, received: u64) -> Message {
    Message::Text(
        serde_json::json!({
            "token": token, "protocol": 2,
            "resume": { "session": sid, "secret": secret, "received": received }
        })
        .to_string()
        .into(),
    )
}

fn ack(stream: u64, bytes: u64) -> Message {
    Message::Text(
        serde_json::json!({ "ack": { "stream": stream, "bytes": bytes } })
            .to_string()
            .into(),
    )
}

fn helo_with(mac: [u8; 6], device_id: u8, caps: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; 36];
    p[0] = device_id;
    p[2..8].copy_from_slice(&mac);
    p[34..36].copy_from_slice(b"en");
    p.extend_from_slice(caps);
    client_frame(b"HELO", &p)
}

// ---------------------------------------------------------------------------
// Security: client → LMS opcode allow-list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn client_opcodes_outside_the_player_set_are_refused() {
    let cases: [(&[u8; 4], &[u8]); 10] = [
        (b"IR  ", &[0; 10]),
        (b"BUTN", &[0; 8]),
        (b"KNOB", &[0; 8]),
        (b"BODY", b"{}"),
        (b"UREQ", &[]),
        (b"SHUT", &[]),
        // A rename LMS never asked for (an image-proxy URL as the player's name).
        (b"SETD", b"\0http://lms.example.lan/imageproxy/x\0"),
        (b"ALSS", &[0; 4]),
        (b"ZZZZ", &[]),
        (b"stat", &[0; 53]),
    ];
    for (op, payload) in cases {
        let mut h = harness().await;
        let mut ws = h.dial().await;
        connect(&mut ws, &h.tok_a).await;
        let _lms = h.lms.accept().await;
        ws.send(Message::Binary(helo(h.mac_a).into()))
            .await
            .unwrap();
        wait_for(|| h.lms.frames() == ["HELO"]).await;
        ws.send(Message::Binary(client_frame(op, payload).into()))
            .await
            .unwrap();
        expect_close(&mut ws, CloseCode::Policy).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.lms.frames(), ["HELO"], "{}", op.escape_ascii());
        wait_for(|| h.lms.closed() == 1).await;
    }
}

#[tokio::test]
async fn helo_must_claim_the_software_player_device_id() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    ws.send(Message::Binary(helo_with(h.mac_a, 2, b"flc").into()))
        .await
        .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.lms.frames().is_empty());
}

#[tokio::test]
async fn hostile_capabilities_are_sanitised() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    let mut caps = b"Model=squeezelite,ModelName=Lyrion Anywhere,flc,mp3,evil\r\nX: 1,<b>,\
                     Url=http://lms.example.lan/x,a=b=c,\xff\xfe,"
        .to_vec();
    caps.extend(b"flc,".repeat(2000));
    ws.send(Message::Binary(helo_with(h.mac_a, 12, &caps).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;
    let p = h.lms.payloads(b"HELO").remove(0);
    let got = String::from_utf8(p[36..].to_vec()).expect("ASCII only");
    assert!(got.len() <= 1024, "capabilities bounded: {}", got.len());
    assert!(got.starts_with("Model=squeezelite,ModelName=Lyrion Anywhere,flc,mp3,flc,"));
    for bad in ["evil", "<b>", "://", "a=b=c", "\r", "\n"] {
        assert!(!got.contains(bad), "{bad:?} survived in {got:?}");
    }
    assert_eq!(&p[..8], &helo(h.mac_a)[8..16], "fixed fields untouched");
}

#[tokio::test]
async fn name_replies_only_answer_a_query_and_are_sanitised() {
    let mut h = harness().await;
    let mut ws = h.dial().await;
    connect(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    // LMS asks for the player's name.
    lms.write_all(&encode_server_frame(*b"setd", &[0]))
        .await
        .unwrap();
    next(&mut ws).await;
    ws.send(Message::Binary(
        client_frame(b"SETD", b"\0http://lms.example.lan/imageproxy/x\0").into(),
    ))
    .await
    .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "SETD"]).await;
    assert_eq!(h.lms.payloads(b"SETD")[0], b"\0device-a\0");
    // A second, unsolicited reply is a policy violation.
    ws.send(Message::Binary(
        client_frame(b"SETD", b"\0Kitchen\0").into(),
    ))
    .await
    .unwrap();
    expect_close(&mut ws, CloseCode::Policy).await;
    assert_eq!(h.lms.frames(), ["HELO", "SETD"]);
}

// ---------------------------------------------------------------------------
// Stream relay: cancellation and stalls
// ---------------------------------------------------------------------------

async fn read_for(resp: &mut reqwest::Response, dur: Duration) -> (usize, bool) {
    let mut n = 0;
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        match tokio::time::timeout_at(deadline, resp.chunk()).await {
            Ok(Ok(Some(c))) => n += c.len(),
            Ok(Ok(None)) | Ok(Err(_)) => return (n, true),
            Err(_) => return (n, false),
        }
    }
}

/// Regression: ending a session (here by revoking its device) left its stream response
/// running, still holding LMS's stream open.
#[tokio::test]
async fn ending_a_session_stops_its_stream_promptly() {
    let mut h = harness_with(|c| c.replay_window = 8 * 1024).await;
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;
    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let (got, done) = read_for(&mut resp, Duration::from_millis(500)).await;
    assert!(got > 0 && !done, "waiting for acknowledgements");

    let device = h.accounts.authenticate(&h.tok_a).unwrap().device_id;
    h.gw.end_sessions_for(&[device]).await;
    let (_, done) = read_for(&mut resp, Duration::from_secs(2)).await;
    assert!(done, "the stream must end with its session");
}

#[tokio::test]
async fn ending_a_session_stops_a_stalled_stream() {
    let mut h = harness().await;
    h.lms
        .stall_stream
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;
    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let (got, done) = read_for(&mut resp, Duration::from_millis(300)).await;
    assert_eq!((got, done), (1000, false));
    let device = h.accounts.authenticate(&h.tok_a).unwrap().device_id;
    h.gw.end_sessions_for(&[device]).await;
    let (_, done) = read_for(&mut resp, Duration::from_secs(2)).await;
    assert!(done, "the stream must end with its session");
}

/// Regression: a stream response held the relay lock while waiting on LMS, so the
/// player socket's next ack blocked its read loop and STATs stopped reaching LMS.
#[tokio::test]
async fn stalled_stream_does_not_block_the_player_socket() {
    let mut h = harness().await;
    h.lms
        .stall_stream
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut ws = h.dial().await;
    let (sid, _) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    ws.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    lms.write_all(&strm(b's', Ipv4Addr::UNSPECIFIED, &stream_req(h.mac_a)))
        .await
        .unwrap();
    next(&mut ws).await;
    let mut resp = h.get_stream(&sid, 1, Some(&h.tok_a)).await;
    let (got, _) = read_for(&mut resp, Duration::from_millis(300)).await;
    assert_eq!(got, 1000);

    for i in 0..3u32 {
        ws.send(ack(1, 1000)).await.unwrap();
        let mut p = vec![0u8; 53];
        p[0..4].copy_from_slice(b"STMt");
        p[25..29].copy_from_slice(&i.to_be_bytes());
        ws.send(Message::Binary(client_frame(b"STAT", &p).into()))
            .await
            .unwrap();
    }
    wait_for(|| h.lms.stats().len() == 3).await;
}

// ---------------------------------------------------------------------------
// Slow clients and replaced sockets
// ---------------------------------------------------------------------------

/// Regression: a client too slow to drain its queue was dropped from the session without
/// its socket being closed or the resume grace (and heartbeats) starting.
#[tokio::test]
async fn slow_client_is_closed_and_can_resume() {
    let mut h = harness_with(|c| c.detached_queue_limit = 512 * 1024 * 1024).await;
    let mut ws = h.dial().await;
    let (sid, secret) = open_session(&mut ws, &h.tok_a).await;
    let mut lms = h.lms.accept().await;
    let mut p = vec![0u8; 53];
    p[0..4].copy_from_slice(b"STMs");
    let mut first = helo(h.mac_a);
    first.extend(client_frame(b"STAT", &p));
    ws.send(Message::Binary(first.into())).await.unwrap();
    wait_for(|| h.lms.stats().len() == 1).await;

    // LMS floods the player while the client reads nothing.
    const FRAMES: usize = 3000;
    let payload = vec![b'x'; 16 * 1024];
    let flood = tokio::spawn(async move {
        for _ in 0..FRAMES {
            lms.write_all(&encode_server_frame(*b"vers", &payload))
                .await
                .unwrap();
        }
        lms
    });
    // Detached: the gateway now reports to LMS for the player.
    wait_for(|| h.lms.stats().len() >= 2).await;
    let _lms = flood.await.unwrap();

    let mut received = 0u64;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("socket must close")
        {
            Some(Ok(Message::Binary(_))) => received += 1,
            Some(Ok(Message::Close(Some(f)))) => {
                assert_eq!(f.code, CloseCode::Again, "{}", f.reason);
                break;
            }
            other => panic!("expected close 1013, got {other:?}"),
        }
    }
    assert!((received as usize) < FRAMES);

    let mut ws = h.dial().await;
    ws.send(resume_after(&h.tok_a, &sid, &secret, received))
        .await
        .unwrap();
    assert_eq!(greeting(&mut ws).await["resumed"], true);
    let mut rest = 0;
    while received + rest < FRAMES as u64 {
        match next(&mut ws).await {
            Message::Binary(b) => assert_eq!(&b[2..6], b"vers"),
            m => panic!("{m:?}"),
        }
        rest += 1;
    }
    assert_eq!(h.lms.closed(), 0, "LMS never sees the player leave");
}

/// Regression: after a resume took over, the old socket's reader kept forwarding frames.
#[tokio::test]
async fn replaced_socket_cannot_feed_the_session() {
    let mut h = harness().await;
    let mut old = h.dial().await;
    let (sid, secret) = open_session(&mut old, &h.tok_a).await;
    let _lms = h.lms.accept().await;
    old.send(Message::Binary(helo(h.mac_a).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO"]).await;

    let mut new = h.dial().await;
    new.send(resume_after(&h.tok_a, &sid, &secret, 0))
        .await
        .unwrap();
    assert_eq!(greeting(&mut new).await["resumed"], true);
    // The old socket hasn't processed its close yet and keeps talking.
    let _ = old
        .send(Message::Binary(client_frame(b"STAT", &[0; 53]).into()))
        .await;
    let _ = old.send(ack(1, 1)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.lms.frames(), ["HELO"], "the replaced socket was heard");

    new.send(Message::Binary(client_frame(b"STAT", &[0; 53]).into()))
        .await
        .unwrap();
    wait_for(|| h.lms.frames() == ["HELO", "STAT"]).await;
}
