//! Sign-in, token rotation, pairing and sign-out against the real gateway.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anywhere_gateway::accounts::Role;
use anywhere_mobile::{
    AlbumSort, ApiError, MusicLibrary, Server, Session, StoredCredentials, UserRole,
};
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_keeps_only_the_refresh_token_and_device_id() {
    let h = Harness::start().await;
    let store = MemStore::new();
    let s = h.session(store.clone());
    assert!(!s.has_sign_in());
    let who = s.login(" alice ".into(), PW.into(), None).await.unwrap();
    assert_eq!(who.user.username, "alice");
    assert_eq!(who.user.role, UserRole::Listener);
    assert_eq!(who.device.name, "Test phone");
    assert!(who.device.player_id.starts_with("02:4c:41:"));
    let saved = store.get().expect("refresh token saved");
    assert_eq!(saved.device_id, who.device.id);
    assert!(saved.refresh_token.len() >= 32);
    assert_eq!(s.current(), Some(who.clone()));
    // The sign-in's answer doesn't say the player protocol; `/v1/me` does.
    assert_eq!(who.protocol, None);
    let me = s.me().await.unwrap();
    assert_eq!(me.protocol, Some(2));
    assert_eq!((me.user, me.device), (who.user, who.device));
    let account = s.account().await.unwrap();
    assert_eq!(account.username, "alice");
    assert!(!account.is_admin && !account.two_step_enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_credentials_are_unauthorised_then_rate_limited() {
    let h = Harness::start().await;
    let store = MemStore::new();
    let s = h.session(store.clone());
    for _ in 0..5 {
        assert_eq!(
            s.login("alice".into(), "wrong".into(), None).await,
            Err(ApiError::Unauthorised)
        );
    }
    match s.login("alice".into(), "wrong".into(), None).await {
        Err(ApiError::RateLimited { retry_after_secs }) => assert!(retry_after_secs >= 60),
        other => panic!("expected rate limiting, got {other:?}"),
    }
    assert!(store.get().is_none());
    assert!(!s.has_sign_in());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_input_is_refused_before_sending() {
    let h = Harness::start().await;
    let s = h.session(MemStore::new());
    for (user, pw, code) in [
        ("", PW, None),
        ("alice", "", None),
        ("alice", PW, Some("12345")),
        ("alice", PW, Some("abcdef")),
        ("alice", PW, Some("1234567")),
    ] {
        assert!(matches!(
            s.login(user.into(), pw.into(), code.map(Into::into)).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        s.pair("not a code!".into()).await,
        Err(ApiError::InvalidInput { .. })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_step_sign_in() {
    let h = Harness::start().await;
    h.accounts.create_user("carol", PW, Role::Admin).unwrap();
    let t = h
        .accounts
        .login("carol", PW, None, "setup", None, "test")
        .unwrap();
    let setup = h
        .accounts
        .totp_begin(t.principal.user_id, "carol", t.principal.device_id, PW)
        .unwrap();
    h.accounts
        .totp_confirm(t.principal.user_id, "carol", &totp_code(&setup.secret, 0))
        .unwrap();

    let store = MemStore::new();
    let s = h.session(store.clone());
    assert_eq!(
        s.login("carol".into(), PW.into(), None).await,
        Err(ApiError::TwoStepRequired)
    );
    assert!(store.get().is_none());
    // A wrong code is a plain refusal.
    assert_eq!(
        s.login("carol".into(), PW.into(), Some("000000".into()))
            .await,
        Err(ApiError::Unauthorised)
    );
    let code = totp_code(&setup.secret, 1);
    let spaced = format!("{} {}", &code[..3], &code[3..]);
    let who = s
        .login("carol".into(), PW.into(), Some(spaced))
        .await
        .unwrap();
    assert_eq!(who.user.role, UserRole::Admin);
    assert!(store.get().is_some());
    assert!(s.account().await.unwrap().two_step_enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_rotates_the_stored_token() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let first = store.get().unwrap();
    s.refresh().await.unwrap();
    let second = store.get().unwrap();
    assert_ne!(first.refresh_token, second.refresh_token);
    assert_eq!(first.device_id, second.device_id);
    assert_eq!(
        s.debug_refresh_token().as_deref(),
        Some(second.refresh_token.as_str())
    );
    // Still works with the new token.
    MusicLibrary::new(s.clone())
        .albums(0, 10, AlbumSort::Title, None, Default::default())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_save_never_leaves_a_stale_token_behind() {
    // The rotated token couldn't be stored: the old one must not be presented on the next
    // start (that would look like a stolen token and revoke the device, and its downloads).
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    store.fail_saves.store(true, Ordering::SeqCst);
    s.refresh().await.unwrap();
    assert!(store.get().is_none(), "the stale token is cleared");
    // This run of the app keeps working.
    MusicLibrary::new(s.clone())
        .albums(0, 10, AlbumSort::Title, None, Default::default())
        .await
        .unwrap();
    // The next start asks to sign in again; the device is not revoked.
    store.fail_saves.store(false, Ordering::SeqCst);
    let again = h.session(store.clone());
    assert!(!again.has_sign_in());
    s.refresh().await.unwrap();
    assert!(store.get().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_sign_in_is_restored_by_a_new_session() {
    let h = Harness::start().await;
    let (s1, store) = h.alice().await;
    let who = s1.current().unwrap();
    drop(s1);
    let s2 = h.session(store.clone());
    assert!(s2.has_sign_in());
    assert_eq!(s2.current(), None);
    assert_eq!(s2.restore().await.unwrap(), who);
    let page = MusicLibrary::new(s2.clone())
        .albums(0, 50, AlbumSort::Title, None, Default::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_without_a_saved_sign_in_is_unauthorised() {
    let h = Harness::start().await;
    let s = h.session(MemStore::new());
    assert_eq!(s.restore().await, Err(ApiError::Unauthorised));
    // A garbage token in storage is refused by the server and forgotten.
    let store = MemStore::with(StoredCredentials {
        refresh_token: "f".repeat(64),
        device_id: 1,
    });
    let s = h.session(store.clone());
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    assert_eq!(s.restore().await, Err(ApiError::Unauthorised));
    assert!(store.get().is_none());
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![true]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_access_token_is_renewed_and_the_call_retried_once() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let before = store.get().unwrap();
    let saves = store.saves();
    s.debug_invalidate_access_token();
    let page = MusicLibrary::new(s.clone())
        .albums(0, 10, AlbumSort::Newest, None, Default::default())
        .await
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(store.saves(), saves + 1, "one refresh");
    assert_ne!(store.get().unwrap().refresh_token, before.refresh_token);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_calls_share_one_refresh() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let saves = store.saves();
    s.debug_invalidate_access_token();
    let lib = MusicLibrary::new(s.clone());
    let calls = (0..8).map(|_| {
        let lib = lib.clone();
        tokio::spawn(async move { lib.artists(0, 10, None, Default::default()).await })
    });
    for c in calls {
        c.await.unwrap().unwrap();
    }
    assert_eq!(store.saves(), saves + 1, "exactly one rotation");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copied_refresh_token_revokes_the_device() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    let stolen = store.get().unwrap();
    // The owner refreshes twice (the second use of a replacement ends the grace window).
    s.refresh().await.unwrap();
    s.refresh().await.unwrap();
    // The thief presents the old token: reuse, so the device is revoked.
    let thief = h.session(MemStore::with(stolen));
    assert_eq!(thief.restore().await, Err(ApiError::Unauthorised));
    // The owner's next refresh is refused too: signed out, storage cleared.
    assert_eq!(s.refresh().await, Err(ApiError::Unauthorised));
    assert!(store.get().is_none());
    assert!(!s.has_sign_in());
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![true]);
    assert_eq!(
        MusicLibrary::new(s.clone())
            .artists(0, 1, None, Default::default())
            .await,
        Err(ApiError::Unauthorised)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_revokes_the_device_and_forgets_the_token() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let saved = store.get().unwrap();
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    assert_eq!(s.logout().await, Ok(true));
    assert!(store.get().is_none());
    assert_eq!(store.clears.load(Ordering::SeqCst), 1);
    assert_eq!(s.current(), None);
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![false]);
    assert_eq!(
        MusicLibrary::new(s.clone())
            .artists(0, 1, None, Default::default())
            .await,
        Err(ApiError::Unauthorised)
    );
    // The old refresh token no longer works anywhere.
    let other = h.session(MemStore::with(saved));
    assert_eq!(other.restore().await, Err(ApiError::Unauthorised));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_while_offline_still_forgets_locally() {
    let h = Harness::start().await;
    let (_s, store) = h.alice().await;
    // Point a new session at a port nobody listens on, with the saved token.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    let offline = Session::new(
        Server::new(format!("http://{dead_addr}")).unwrap(),
        MemStore::with(store.get().unwrap()),
        "Phone".into(),
    )
    .unwrap();
    assert!(matches!(
        offline.restore().await,
        Err(ApiError::Network { .. })
    ));
    // A network failure keeps the sign-in.
    assert!(offline.has_sign_in());
    assert_eq!(offline.logout().await, Ok(false));
    assert!(!offline.has_sign_in());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_in_again_keeps_the_same_device() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let device = s.current().unwrap().device;
    // The access and refresh tokens are lost (say, the app's data was partly restored),
    // but the device id is still known.
    let again = h.session(MemStore::with(StoredCredentials {
        refresh_token: "x".repeat(40),
        device_id: device.id,
    }));
    let who = again.login("alice".into(), PW.into(), None).await.unwrap();
    assert_eq!(who.device.id, device.id);
    assert_eq!(who.device.player_id, device.player_id);
    drop(store);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_with_a_one_time_code() {
    let h = Harness::start().await;
    let code = h.accounts.create_pairing_code("bob").unwrap();
    let store = MemStore::new();
    let s = h.session(store.clone());
    // Codes are case-insensitive and may be typed with dashes and spaces.
    let typed = format!("{}-{} ", code[..5].to_lowercase(), &code[5..]);
    let who = s.pair(typed).await.unwrap();
    assert_eq!(who.user.username, "bob");
    assert!(store.get().is_some());
    MusicLibrary::new(s.clone())
        .artists(0, 5, None, Default::default())
        .await
        .unwrap();
    // Single use.
    let s2 = h.session(MemStore::new());
    assert_eq!(s2.pair(code).await, Err(ApiError::Unauthorised));
    assert_eq!(
        s2.pair("ABCDEFGHJK".into()).await,
        Err(ApiError::Unauthorised)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_in_tells_the_gateway_this_is_the_android_app() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let id = s.current().unwrap().device.id;
    let d = h
        .accounts
        .list_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == id)
        .unwrap();
    assert_eq!(d.client_kind.as_deref(), Some("android"));
    assert_eq!(d.client_platform.as_deref(), Some("Test phone"));
    assert_eq!(d.last_addr.as_deref(), Some("127.0.0.1"));
    // Pairing too.
    let code = h.accounts.create_pairing_code("bob").unwrap();
    let p = h.session(MemStore::new());
    let who = p.pair(code).await.unwrap();
    let d = h
        .accounts
        .list_devices()
        .unwrap()
        .into_iter()
        .find(|d| d.id == who.device.id)
        .unwrap();
    assert_eq!(d.client_kind.as_deref(), Some("android"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_device_can_be_renamed() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    assert_eq!(
        s.rename_device("  Kitchen phone ".into()).await.unwrap(),
        "Kitchen phone"
    );
    assert_eq!(s.current().unwrap().device.name, "Kitchen phone");
    assert_eq!(s.me().await.unwrap().device.name, "Kitchen phone");
    // Markup is cleaned by the gateway, and the answer is what it stored.
    assert_eq!(s.rename_device("<b>Den</b>".into()).await.unwrap(), "bDenb");
    // Refused before sending.
    for bad in ["", "   ", "a\nb"] {
        assert!(matches!(
            s.rename_device(bad.into()).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        s.rename_device("x".repeat(65)).await,
        Err(ApiError::InvalidInput { .. })
    ));
    // Refused by the gateway: nothing usable left.
    assert!(matches!(
        s.rename_device("<>/".into()).await,
        Err(ApiError::InvalidInput { .. })
    ));
    assert_eq!(s.current().unwrap().device.name, "bDenb");
    // Not signed in: nothing to rename.
    let out = h.session(MemStore::new());
    assert_eq!(
        out.rename_device("Phone".into()).await,
        Err(ApiError::Unauthorised)
    );
}

/// A gateway from before client types refuses unknown fields: sign-in still works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_gateway_still_signs_in() {
    use axum::Json;
    use axum::http::StatusCode;
    use axum::routing::post;
    use serde_json::{Value, json};
    let seen: Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
    let log = seen.clone();
    let app = axum::Router::new().route(
        "/v1/auth/login",
        post(move |Json(body): Json<Value>| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(body.clone());
                if body.get("client_kind").is_some() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "malformed request" })),
                    );
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "access_token": "acc", "refresh_token": "ref", "expires_in": 900,
                        "user": { "username": "alice", "role": "listener" },
                        "device": { "id": 9, "name": "Test phone", "mac": "02:4c:41:00:00:09" },
                    })),
                )
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let server = Server::new(format!("http://{addr}")).unwrap();
    let s = Session::new(server, MemStore::new(), "Test phone".into()).unwrap();
    let who = s.login("alice".into(), PW.into(), None).await.unwrap();
    assert_eq!(who.device.id, 9);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "asked again without the client fields");
    assert_eq!(seen[0]["client_kind"], "android");
    assert!(seen[1].get("client_kind").is_none());
}

/// Redirects are never followed, to another host least of all: neither a password nor a
/// token goes anywhere but the server signed in to. A gateway (or something in front of
/// it) answering with a redirect to another host is a failure, and the other host hears
/// nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_to_another_host_is_never_followed() {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize};

    // The other host counts every connection made to it.
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other_addr = other.local_addr().unwrap();
    let reached = Arc::new(AtomicUsize::new(0));
    let r = reached.clone();
    tokio::spawn(async move {
        while let Ok((s, _)) = other.accept().await {
            r.fetch_add(1, Ordering::SeqCst);
            drop(s);
        }
    });

    let redirect_login = Arc::new(AtomicBool::new(true));
    let status = Arc::new(AtomicU16::new(307));
    let (rl, st) = (redirect_login.clone(), status.clone());
    let app =
        axum::Router::new().fallback(axum::routing::any(move |req: axum::extract::Request| {
            let (rl, st) = (rl.clone(), st.clone());
            async move {
                let path = req.uri().path().to_string();
                if path == "/v1/auth/login" && !rl.load(Ordering::SeqCst) {
                    return serde_json::json!({
                        "access_token": "acc", "refresh_token": "ref", "expires_in": 900,
                        "user": {"username": "alice", "role": "listener"},
                        "device": {"id": 1, "name": "Test phone"}})
                    .to_string()
                    .into_response();
                }
                let code = StatusCode::from_u16(st.load(Ordering::SeqCst)).unwrap();
                (code, [("location", format!("http://{other_addr}{path}"))]).into_response()
            }
        }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let server = Server::new(format!("http://{addr}")).unwrap();
    let store = MemStore::new();
    let s = Session::new(server, store.clone(), "Test phone".into()).unwrap();

    // Signing in: the password is not sent on.
    for code in [301, 302, 303, 307, 308] {
        status.store(code, Ordering::SeqCst);
        let e = s
            .login("alice".into(), PW.into(), None)
            .await
            .expect_err("a redirected sign-in fails");
        assert!(
            matches!(e, ApiError::Server { status, .. } if status == code),
            "{code}: {e:?}"
        );
        assert!(!s.has_sign_in());
    }
    assert_eq!(store.saves(), 0);

    // Signed in: requests carrying the token are not sent on either, and the sign-in is
    // kept.
    redirect_login.store(false, Ordering::SeqCst);
    s.login("alice".into(), PW.into(), None).await.unwrap();
    for code in [301, 302, 307, 308] {
        status.store(code, Ordering::SeqCst);
        let e = s.me().await.expect_err("a redirected request fails");
        assert!(
            matches!(e, ApiError::Server { status, .. } if status == code),
            "{code}: {e:?}"
        );
        let e = MusicLibrary::new(s.clone())
            .albums(0, 10, AlbumSort::Title, None, Default::default())
            .await
            .expect_err("a redirected request fails");
        assert!(!matches!(e, ApiError::Unauthorised), "{code}: {e:?}");
        assert!(s.has_sign_in());
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        reached.load(Ordering::SeqCst),
        0,
        "nothing reached the other host"
    );
}

// --- Sign-out, expiry and rotation edge cases (review 2026-11-12) ------------------------

/// M11 regression: an access token the gateway no longer accepts (it restarted, so the
/// tokens it held in memory are gone) made the sign-out's 401 count as done: the token was
/// deleted here and the device stayed signed in on the server for up to 90 days. The
/// sign-out now renews the token and asks again, so the device is revoked there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_with_a_stale_access_token_still_revokes_the_device() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let id = s.current().unwrap().device.id;
    let listed = |h: &Harness| {
        h.accounts
            .list_devices()
            .unwrap()
            .iter()
            .any(|d| d.id == id)
    };
    assert!(listed(&h));
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    s.debug_invalidate_access_token();
    assert_eq!(s.logout().await, Ok(true));
    assert!(!listed(&h), "the device is signed out on the server");
    assert!(store.get().is_none() && !s.has_sign_in());
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![false]);
}

/// M11: only the gateway's own answer confirms a sign-out. A proxy's or CDN's page (in the
/// way during maintenance) or no answer at all is "not confirmed"; the sign-in is
/// forgotten here all the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_out_answered_by_something_else_is_not_confirmed() {
    for (status, body) in [
        (401, "<html>Unauthorized</html>"),
        (401, ""),
        (403, "<html>Attention required</html>"),
        (404, "Not found"),
        (400, r#"{"message":"bad request"}"#),
        (502, "Bad gateway"),
        (503, r#"{"error":"busy; try again shortly"}"#),
    ] {
        let fake = FakeAuth::start().await;
        let (s, store) = fake.saved_session();
        s.restore().await.unwrap();
        fake.answer_logout(status, body);
        assert_eq!(s.logout().await, Ok(false), "{status} {body}");
        assert!(store.get().is_none() && !s.has_sign_in(), "{status}");
        assert_eq!(
            fake.state.logouts.lock().unwrap().len(),
            1,
            "{status}: asked once"
        );
    }
}

/// M11: the gateway's own 401 to a sign-out is renewed and asked once more; a sign-in the
/// gateway has already ended (its refusal of the renewal, or of the retried sign-out) is
/// as good as signed out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_out_the_gateway_refuses_is_renewed_and_asked_again() {
    let refused = gateway_refusal(None);
    // The renewal works and the second sign-out is taken.
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    s.restore().await.unwrap();
    fake.answer_logout_once(401, &refused);
    assert_eq!(s.logout().await, Ok(true));
    let logouts = fake.state.logouts.lock().unwrap().clone();
    assert_eq!(logouts.len(), 2, "asked again: {logouts:?}");
    assert_ne!(logouts[0], logouts[1], "with a renewed token");

    // The renewal is refused: the gateway had already signed this device out (revoked
    // it). The app hears that once, as a revocation.
    let fake = FakeAuth::start().await;
    let (s, store) = fake.saved_session();
    s.restore().await.unwrap();
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    fake.answer_logout(401, &refused);
    fake.refuse_refresh(401, &refused);
    assert_eq!(s.logout().await, Ok(true));
    assert_eq!(fake.state.logouts.lock().unwrap().len(), 1);
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![true]);
    assert!(store.get().is_none() && !s.has_sign_in());

    // Refused with a fresh token too: signed out there.
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    s.restore().await.unwrap();
    fake.answer_logout(401, &refused);
    assert_eq!(s.logout().await, Ok(true));
    assert_eq!(
        fake.state.logouts.lock().unwrap().len(),
        2,
        "once more only"
    );

    // The renewal can't reach the gateway: not confirmed.
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    s.restore().await.unwrap();
    fake.answer_logout(401, &refused);
    fake.refuse_refresh(502, "Bad gateway");
    assert_eq!(s.logout().await, Ok(false));
}

/// M18: a sign-in the gateway says has expired (`"reason": "expired"` on its refusal of a
/// refresh) is a plain sign-out: the app keeps the downloads (`revoked: false`). Before,
/// every refusal was a revocation, and a phone used offline past the sign-in's lifetime lost
/// its downloads on reconnecting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_sign_in_signs_out_but_is_not_a_revocation() {
    let fake = FakeAuth::start().await;
    let (s, store) = fake.saved_session();
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    fake.refuse_refresh(401, &gateway_refusal(Some("expired")));
    assert_eq!(s.restore().await, Err(ApiError::Unauthorised));
    assert!(store.get().is_none() && !s.has_sign_in());
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![false]);

    // Found expired while signed in (renewing for a call): the same.
    let fake = FakeAuth::start().await;
    let (s, store) = fake.saved_session();
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    s.restore().await.unwrap();
    fake.refuse_refresh(401, &gateway_refusal(Some("expired")));
    assert_eq!(s.refresh().await, Err(ApiError::Unauthorised));
    assert!(store.get().is_none());
    assert_eq!(*observer.signed_out.lock().unwrap(), vec![false]);
}

/// M18: every other refusal of the gateway's own (no reason, or any other) is a revocation,
/// as before; and a refusal that isn't the gateway's (a proxy's 401 page) ends nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn any_other_refusal_of_a_refresh_is_a_revocation() {
    for reason in [
        None,
        Some("revoked"),
        Some("reused"),
        Some("Expired"),
        Some(""),
    ] {
        let fake = FakeAuth::start().await;
        let (s, store) = fake.saved_session();
        let observer = Arc::new(Observer::default());
        s.set_observer(observer.clone());
        fake.refuse_refresh(401, &gateway_refusal(reason));
        assert_eq!(s.restore().await, Err(ApiError::Unauthorised), "{reason:?}");
        assert!(store.get().is_none(), "{reason:?}");
        assert_eq!(
            *observer.signed_out.lock().unwrap(),
            vec![true],
            "{reason:?}"
        );
    }
    // Not the gateway's: the sign-in is kept, whatever the page says.
    let fake = FakeAuth::start().await;
    let (s, store) = fake.saved_session();
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    fake.refuse_refresh(401, r#"<html>{"reason":"expired"}</html>"#);
    assert!(matches!(
        s.restore().await,
        Err(ApiError::Server { status: 401, .. })
    ));
    assert!(store.get().is_some() && s.has_sign_in());
    assert!(observer.signed_out.lock().unwrap().is_empty());
}

/// After the gateway says a sign-in expired, the next password sign-in names the expired
/// refresh token beside the device id (the gateway takes it as proof of the device, with
/// its own lockout allowance). Only then: never after a revocation, never on a first
/// sign-in, and not again once signed in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_in_after_expiry_names_the_expired_token() {
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    fake.refuse_refresh(401, &gateway_refusal(Some("expired")));
    assert_eq!(s.restore().await, Err(ApiError::Unauthorised));
    *fake.state.refuse_refresh.lock().unwrap() = None;
    s.login("alice".into(), PW.into(), None).await.unwrap();
    s.logout().await.unwrap();
    s.login("alice".into(), PW.into(), None).await.unwrap();
    let logins = fake.state.logins.lock().unwrap().clone();
    assert_eq!(logins.len(), 2);
    assert_eq!(logins[0]["refresh_token"], "refresh-start");
    assert_eq!(logins[0]["device_id"], 1);
    assert!(logins[1].get("refresh_token").is_none(), "{}", logins[1]);

    // A wrong password keeps it for the next attempt (and a two-step retry).
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    fake.refuse_refresh(401, &gateway_refusal(Some("expired")));
    let _ = s.restore().await;
    assert_eq!(
        s.login("alice".into(), "not the password".into(), None)
            .await,
        Err(ApiError::Unauthorised)
    );
    s.login("alice".into(), PW.into(), None).await.unwrap();
    let logins = fake.state.logins.lock().unwrap().clone();
    assert_eq!(logins.len(), 2);
    assert!(logins.iter().all(|l| l["refresh_token"] == "refresh-start"));

    // Revoked: the token proves nothing and is never sent.
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    fake.refuse_refresh(401, &gateway_refusal(None));
    let _ = s.restore().await;
    *fake.state.refuse_refresh.lock().unwrap() = None;
    s.login("alice".into(), PW.into(), None).await.unwrap();
    let logins = fake.state.logins.lock().unwrap().clone();
    assert!(logins[0].get("refresh_token").is_none(), "{}", logins[0]);
    assert_eq!(logins[0]["device_id"], 1);

    // A first sign-in on this phone: nothing to name.
    let fake = FakeAuth::start().await;
    let s = Session::new(fake.server(), MemStore::new(), "Test phone".into()).unwrap();
    s.login("alice".into(), PW.into(), None).await.unwrap();
    let logins = fake.state.logins.lock().unwrap().clone();
    assert!(logins[0].get("refresh_token").is_none() && logins[0].get("device_id").is_none());
}

/// M17 regression: a rotation started on a runtime that is then dropped (the player
/// engine's or the downloader's, when it stops or the network changes) still completes,
/// and its new token is stored. Before, the rotation ran on that runtime and died with it
/// after the gateway had rotated: the old token stayed stored, and presenting it again
/// would look like theft and revoke the device.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rotation_outlives_the_runtime_that_started_it() {
    let fake = FakeAuth::start().await;
    let hold = Arc::new(tokio::sync::Notify::new());
    *fake.state.hold_refresh.lock().unwrap() = Some(hold.clone());
    let (s, store) = fake.saved_session();
    let (s2, state) = (s.clone(), fake.state.clone());
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let asked = async {
                while state.refreshes.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            };
            tokio::select! {
                r = s2.refresh() => panic!("a held refresh answered: {r:?}"),
                () = asked => {}
            }
        });
        // The engine stops: its runtime goes, with whatever it was running.
        drop(rt);
    })
    .await
    .unwrap();
    hold.notify_one();
    wait_for("the rotated token stored", || {
        store.get().is_some_and(|c| c.refresh_token == "refresh-1")
    })
    .await;
    assert_eq!(s.debug_refresh_token().as_deref(), Some("refresh-1"));
    // The sign-in carries on with it.
    *fake.state.hold_refresh.lock().unwrap() = None;
    assert_eq!(s.me().await.unwrap().user.username, "alice");
    assert_eq!(fake.state.refreshes.load(Ordering::SeqCst), 1);
}

/// M17: the shared connection pool no longer holds connections made on a private runtime.
/// A request the engine made through it, followed by its thread being busy (nothing drives
/// its runtime), left a pooled connection that the app's next request then waited on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_private_runtime_never_stalls_the_shared_pool() {
    let fake = FakeAuth::start().await;
    let (s, _) = fake.saved_session();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (done, finished) = tokio::sync::oneshot::channel();
    let s2 = s.clone();
    let engine = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // The first requests to this server: any pooled connection is made here.
        rt.block_on(async {
            s2.restore().await.unwrap();
            s2.me().await.unwrap();
        });
        let _ = done.send(());
        // Busy: the runtime is alive, but nothing drives it.
        let _ = released.recv();
        drop(rt);
    });
    finished.await.unwrap();
    let r = tokio::time::timeout(std::time::Duration::from_secs(5), s.me()).await;
    release.send(()).unwrap();
    engine.join().unwrap();
    assert!(matches!(r, Ok(Ok(_))), "{r:?}");
}

/// A secure store whose saves wait at a gate, as a slow Keystore would.
#[derive(Default)]
struct SlowStore {
    creds: std::sync::Mutex<Option<StoredCredentials>>,
    closed: std::sync::Mutex<bool>,
    opened: std::sync::Condvar,
    saving: std::sync::atomic::AtomicBool,
}

impl SlowStore {
    fn set_closed(&self, closed: bool) {
        *self.closed.lock().unwrap() = closed;
        self.opened.notify_all();
    }
}

impl anywhere_mobile::SecureStore for SlowStore {
    fn load(&self) -> Result<Option<StoredCredentials>, anywhere_mobile::StoreError> {
        Ok(self.creds.lock().unwrap().clone())
    }
    fn save(&self, c: StoredCredentials) -> Result<(), anywhere_mobile::StoreError> {
        self.saving.store(true, Ordering::SeqCst);
        let mut closed = self.closed.lock().unwrap();
        while *closed {
            closed = self.opened.wait(closed).unwrap();
        }
        drop(closed);
        *self.creds.lock().unwrap() = Some(c);
        self.saving.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn clear(&self) -> Result<(), anywhere_mobile::StoreError> {
        *self.creds.lock().unwrap() = None;
        Ok(())
    }
}

/// Review 2026-11-12: `adopt()` and `forget()` wrote the secure store while holding the
/// session's state lock, so a slow Keystore write stalled every call that looks at the
/// sign-in (each request's token, `current()`, `has_sign_in()`). They write it outside the
/// lock now, and a sign-out that comes in meanwhile still leaves nothing stored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_secure_store_never_blocks_the_session() {
    let h = Harness::start().await;
    let store = Arc::new(SlowStore::default());
    let s = Session::new(h.server(), store.clone(), "Test phone".into()).unwrap();
    s.login("alice".into(), PW.into(), None).await.unwrap();
    // A rotation started from a thread of its own (the engine's, say), so that a store
    // write blocking where it runs can't stall this test's runtime.
    let rotate = |s: Arc<Session>| {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(s.refresh())
        })
    };
    store.set_closed(true);
    let rotation = rotate(s.clone());
    let st = store.clone();
    wait_for("the save to start", move || {
        st.saving.load(Ordering::SeqCst)
    })
    .await;
    let s3 = s.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((s3.current().is_some(), s3.has_sign_in()));
    });
    let looked = rx.recv_timeout(std::time::Duration::from_secs(3));
    store.set_closed(false);
    rotation.join().unwrap().unwrap();
    assert_eq!(
        looked,
        Ok((true, true)),
        "the session answers while the store is busy"
    );
    assert_eq!(
        store
            .creds
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.refresh_token.clone()),
        s.debug_refresh_token(),
        "what is stored is what is used"
    );

    // A sign-out while a rotation is being saved: nothing is left stored after both.
    store.set_closed(true);
    let rotation = rotate(s.clone());
    let st = store.clone();
    wait_for("the save to start", move || {
        st.saving.load(Ordering::SeqCst)
    })
    .await;
    let s3 = s.clone();
    let signing_out = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(s3.logout())
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    store.set_closed(false);
    let _ = rotation.join().unwrap();
    assert_eq!(signing_out.join().unwrap(), Ok(true));
    assert!(store.creds.lock().unwrap().is_none());
    assert!(!s.has_sign_in() && s.current().is_none());
}
