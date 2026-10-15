//! "Your devices" (`GET /v1/devices`, `DELETE /v1/devices/{id}`,
//! `POST /v1/devices/sign-out-others`) against the real gateway.

mod common;

use std::sync::Arc;

use anywhere_mobile::ApiError;
use common::*;

async fn bob(h: &Harness) -> Arc<anywhere_mobile::Session> {
    let s = h.session(MemStore::new());
    s.login("bob".into(), PW.into(), None).await.unwrap();
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_says_it_can_list_devices() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    assert!(s.account().await.unwrap().can_manage_devices);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_sees_their_own_devices_with_this_one_marked() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let (other, _) = h.alice().await;
    let b = bob(&h).await;
    let me = s.me().await.unwrap().device.id;
    let them = other.me().await.unwrap().device.id;
    let bobs = b.me().await.unwrap().device.id;

    let list = s.my_devices().await.unwrap();
    let ids: Vec<i64> = list.iter().map(|d| d.id).collect();
    assert_eq!(ids, vec![me, them]);
    assert!(!ids.contains(&bobs));
    assert!(list[0].current && !list[1].current);
    assert_eq!(list[1].name, "Test phone");
    assert!(list[0].created_at.is_some());

    let from_other = other.my_devices().await.unwrap();
    assert!(!from_other[0].current && from_other[1].current);
    assert_eq!(b.my_devices().await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_out_another_device_ends_its_sign_in() {
    let h = Harness::start().await;
    let (s, store) = h.alice().await;
    let (other, other_store) = h.alice().await;
    let observer = Arc::new(Observer::default());
    other.set_observer(observer.clone());
    let them = other.me().await.unwrap().device.id;

    assert!(s.sign_out_device(them).await.unwrap());
    // Already gone: no error, just nothing to do.
    assert!(!s.sign_out_device(them).await.unwrap());
    // The other device finds itself signed out on its next renewal.
    other.debug_invalidate_access_token();
    assert_eq!(other.me().await, Err(ApiError::Unauthorised));
    assert!(other_store.get().is_none(), "its stored sign-in is cleared");
    assert!(!observer.signed_out.lock().unwrap().is_empty());
    // This one carries on.
    s.me().await.unwrap();
    assert!(store.get().is_some());
    assert_eq!(s.my_devices().await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_users_device_is_left_alone() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let b = bob(&h).await;
    let bobs = b.me().await.unwrap().device.id;
    assert!(!s.sign_out_device(bobs).await.unwrap());
    b.debug_invalidate_access_token();
    b.me().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn this_device_and_bad_ids_are_refused() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let me = s.me().await.unwrap().device.id;
    assert!(matches!(
        s.sign_out_device(me).await,
        Err(ApiError::InvalidInput { .. })
    ));
    for bad in [0, -1, i64::MIN] {
        assert!(matches!(
            s.sign_out_device(bad).await,
            Err(ApiError::InvalidInput { .. })
        ));
    }
    s.me().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_other_devices_can_be_signed_out() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let (x, _) = h.alice().await;
    let (y, _) = h.alice().await;
    let b = bob(&h).await;
    assert_eq!(s.sign_out_other_devices().await.unwrap(), 2);
    for gone in [&x, &y] {
        gone.debug_invalidate_access_token();
        assert_eq!(gone.me().await, Err(ApiError::Unauthorised));
    }
    s.me().await.unwrap();
    b.debug_invalidate_access_token();
    b.me().await.unwrap();
    assert_eq!(s.sign_out_other_devices().await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signed_out_sessions_cannot_list_or_sign_out() {
    let h = Harness::start().await;
    let (s, _) = h.alice().await;
    let (other, _) = h.alice().await;
    let them = other.me().await.unwrap().device.id;
    s.logout().await.unwrap();
    assert_eq!(s.my_devices().await, Err(ApiError::Unauthorised));
    assert_eq!(s.sign_out_device(them).await, Err(ApiError::Unauthorised));
    assert_eq!(
        s.sign_out_other_devices().await,
        Err(ApiError::Unauthorised)
    );
    other.me().await.unwrap();
}
