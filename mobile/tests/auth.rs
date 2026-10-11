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
    assert_eq!(s.me().await.unwrap(), who);
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
        .totp_begin(t.principal.user_id, "carol", PW)
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
