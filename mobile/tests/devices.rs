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
        Err(ApiError::Conflict { .. })
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

// --- A scripted gateway: exact answers to DELETE /v1/devices/{id} and /v1/me ----------------

/// What the scripted gateway answers.
struct Answer {
    status: u16,
    body: String,
    headers: Vec<(&'static str, &'static str)>,
}

fn answer(status: u16, body: &str) -> Answer {
    Answer {
        status,
        body: body.into(),
        headers: vec![],
    }
}

/// A gateway that signs alice in and answers `DELETE /v1/devices/…` with `delete`, and
/// `GET /v1/me` with `me`.
async fn scripted(delete: Answer, me: Answer) -> Arc<anywhere_mobile::Session> {
    use axum::response::IntoResponse;
    use axum::routing::any;
    let delete = Arc::new(delete);
    let me = Arc::new(me);
    let app = axum::Router::new().fallback(any(move |req: axum::extract::Request| {
        let (delete, me) = (delete.clone(), me.clone());
        async move {
            let path = req.uri().path().to_string();
            let reply = |a: &Answer| {
                let mut r = (
                    axum::http::StatusCode::from_u16(a.status).unwrap(),
                    a.body.clone(),
                )
                    .into_response();
                for (k, v) in &a.headers {
                    r.headers_mut().insert(*k, v.parse().unwrap());
                }
                r
            };
            match (req.method().as_str(), path.as_str()) {
                ("POST", "/v1/auth/login" | "/v1/auth/refresh") => serde_json::json!({
                    "access_token": "access", "refresh_token": "refresh", "expires_in": 900,
                    "user": {"username": "alice", "role": "listener"},
                    "device": {"id": 1, "name": "Test phone"}})
                .to_string()
                .into_response(),
                ("GET", "/v1/account") => {
                    serde_json::json!({"username": "alice", "role": "listener"})
                        .to_string()
                        .into_response()
                }
                ("GET", "/v1/me") => reply(&me),
                ("DELETE", p) if p.starts_with("/v1/devices/") => reply(&delete),
                _ => axum::http::StatusCode::NOT_FOUND.into_response(),
            }
        }
    }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let server = anywhere_mobile::Server::new(format!("http://{addr}")).unwrap();
    let s = anywhere_mobile::Session::new(server, MemStore::new(), "Test phone".into()).unwrap();
    s.login("alice".into(), PW.into(), None).await.unwrap();
    s
}

fn devices_feature() -> Answer {
    answer(200, r#"{"features":{"devices":true}}"#)
}

/// M16 regression: "already signed out" is the gateway's 404, whatever its body says (a
/// proxy's page, an older gateway's wording).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn any_404_means_already_signed_out() {
    for body in [
        r#"{"error":"no such device"}"#,
        "<html>Not found</html>",
        "",
        r#"{"error":"device not found"}"#,
        r#"{"error":42}"#,
    ] {
        let s = scripted(answer(404, body), devices_feature()).await;
        assert_eq!(s.sign_out_device(7).await, Ok(false), "{body:?}");
    }
}

/// M16 regression: the text alone means nothing: another status whose error happens to
/// read "no such device" is an error, not "already signed out".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_error_text_alone_is_not_already_signed_out() {
    let s = scripted(
        answer(400, r#"{"error":"no such device"}"#),
        devices_feature(),
    )
    .await;
    assert!(
        matches!(
            s.sign_out_device(7).await,
            Err(ApiError::InvalidInput { .. })
        ),
        "a 400 is refused"
    );
    let s = scripted(
        answer(403, r#"{"error":"no such device"}"#),
        devices_feature(),
    )
    .await;
    assert!(matches!(
        s.sign_out_device(7).await,
        Err(ApiError::Forbidden { .. })
    ));
}

/// M16: this device (the gateway's 409) is a typed conflict, not "already signed out".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn this_device_is_a_typed_conflict() {
    let s = scripted(
        answer(
            409,
            r#"{"error":"this is the device you are using; sign out instead"}"#,
        ),
        devices_feature(),
    )
    .await;
    assert_eq!(
        s.sign_out_device(7).await,
        Err(ApiError::Conflict {
            detail: "this is the device you are using; sign out instead".into()
        })
    );
}

/// Too many device requests: the wait the gateway asks for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_many_requests_is_rate_limited() {
    let mut busy = answer(429, r#"{"error":"too many requests"}"#);
    busy.headers.push(("retry-after", "42"));
    let s = scripted(busy, devices_feature()).await;
    assert_eq!(
        s.sign_out_device(7).await,
        Err(ApiError::RateLimited {
            retry_after_secs: 42
        })
    );
}

/// Negative: a gateway that doesn't offer "Your devices" (an older one, a feature that
/// isn't exactly `true`, or `/v1/me` failing) is never offered it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_feature_devices_are_not_offered() {
    for me in [
        answer(200, r#"{"features":{}}"#),
        answer(200, r#"{}"#),
        answer(200, r#"{"features":{"devices":"true"}}"#),
        answer(200, r#"{"features":{"devices":1}}"#),
        answer(200, r#"{"features":{"devices":false}}"#),
        answer(500, r#"{"error":"boom"}"#),
        answer(404, ""),
    ] {
        let body = me.body.clone();
        let s = scripted(answer(404, ""), me).await;
        assert!(!s.account().await.unwrap().can_manage_devices, "{body:?}");
    }
    let s = scripted(answer(404, ""), devices_feature()).await;
    assert!(s.account().await.unwrap().can_manage_devices);
}
