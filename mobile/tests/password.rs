//! Changing one's own password (`POST /v1/account/password`) against the real gateway.

mod common;

use std::sync::Arc;

use anywhere_gateway::accounts::Role;
use anywhere_mobile::ApiError;
use common::*;

const NEW_PW: &str = "a brand new passphrase";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_says_it_can_change_passwords() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    assert!(s.account().await.unwrap().can_change_password);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_password_signs_in_and_the_old_one_does_not() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    // Another device of alice's, signed in before the change.
    let (other, _) = h.alice().await;

    let changed = s
        .change_password(PW.into(), NEW_PW.into(), None, false)
        .await
        .unwrap();
    assert_eq!(changed.signed_out, 0);
    assert_eq!(changed.other_devices.len(), 1, "{changed:?}");
    assert_eq!(changed.other_devices[0].name, "Test phone");

    // This sign-in carries on, through a renewal too.
    s.me().await.unwrap();
    s.debug_invalidate_access_token();
    s.me().await.unwrap();
    assert!(store.get().is_some());
    assert!(observer.signed_out.lock().unwrap().is_empty());
    // So does the other device: it holds its own tokens, not the password.
    other.me().await.unwrap();

    let fresh = h.session(MemStore::new());
    assert_eq!(
        fresh.login("alice".into(), PW.into(), None).await,
        Err(ApiError::Unauthorised)
    );
    fresh
        .login("alice".into(), NEW_PW.into(), None)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn other_devices_can_be_signed_out_with_it() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let (other, _) = h.alice().await;
    let changed = s
        .change_password(PW.into(), NEW_PW.into(), None, true)
        .await
        .unwrap();
    assert_eq!(changed.signed_out, 1);
    assert!(changed.other_devices.is_empty());
    s.me().await.unwrap();
    other.debug_invalidate_access_token();
    assert_eq!(other.me().await, Err(ApiError::Unauthorised));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_is_refused_and_keeps_the_sign_in() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let observer = Arc::new(Observer::default());
    s.set_observer(observer.clone());
    match s
        .change_password("wrong password!".into(), NEW_PW.into(), None, false)
        .await
    {
        Err(ApiError::Forbidden { detail }) => assert!(detail.contains("wrong"), "{detail}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // Still signed in, with the same stored sign-in.
    assert!(observer.signed_out.lock().unwrap().is_empty());
    assert!(store.get().is_some());
    s.me().await.unwrap();
    // And the password is unchanged.
    h.session(MemStore::new())
        .login("alice".into(), PW.into(), None)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn weak_or_unchanged_passwords_are_refused() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    for new in ["short", PW] {
        assert!(matches!(
            s.change_password(PW.into(), new.into(), None, false).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_step_users_need_their_code() {
    let h = Harness::start().await;
    h.accounts.create_user("carol", PW, Role::Listener).unwrap();
    // Signed in first, then two-step turned on (each code is single use, and only a step
    // either side of now is accepted).
    let s = h.session(MemStore::new());
    let who = s.login("carol".into(), PW.into(), None).await.unwrap();
    let user = h.accounts.user_id_by_name("carol").unwrap().unwrap();
    let setup = h
        .accounts
        .totp_begin(user, "carol", who.device.id, PW)
        .unwrap();
    h.accounts
        .totp_confirm(user, "carol", &totp_code(&setup.secret, 0))
        .unwrap();
    assert_eq!(who.user.username, "carol");
    assert!(s.account().await.unwrap().two_step_enabled);

    assert_eq!(
        s.change_password(PW.into(), NEW_PW.into(), None, false)
            .await,
        Err(ApiError::TwoStepRequired)
    );
    assert!(matches!(
        s.change_password(PW.into(), NEW_PW.into(), Some("000000".into()), false)
            .await,
        Err(ApiError::Forbidden { .. })
    ));
    s.change_password(
        PW.into(),
        NEW_PW.into(),
        Some(totp_code(&setup.secret, 1)),
        false,
    )
    .await
    .unwrap();
    s.me().await.unwrap();
}
