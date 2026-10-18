//! Artist biographies and photos, and album reviews (`GET /v1/info/artist/{id}`,
//! `GET /v1/info/album/{id}`) against a fake Music & Artist Information plugin. Security
//! first: strangers, hostile ids and extra parameters are refused before LMS is asked;
//! only `musicartistinfo biography|artistphoto|albumreview` with an id ever reach LMS;
//! text arrives cleaned; and a photo is only ever a `/v1/art/imageproxy/…` path, fetchable
//! through the image proxy and never pointing into the home network.

mod common;
use common::*;

use reqwest::StatusCode as S;
use serde_json::{Value, json};

const PHOTO: &str =
    "/v1/art/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fartist%2F1%2Ejpg/image";

async fn get(h: &Harness, path: &str, token: Option<&str>) -> (S, Value) {
    let mut req = reqwest::Client::new().get(h.url(path));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = tokio::time::timeout(Duration::from_secs(20), req.send())
        .await
        .expect("request took too long")
        .unwrap();
    let s = r.status();
    assert_eq!(
        r.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store"),
        "{path}"
    );
    if s == S::TOO_MANY_REQUESTS {
        assert!(r.headers().contains_key("retry-after"), "{path}");
    }
    (s, r.json().await.unwrap_or(Value::Null))
}

/// The plugin commands the gateway sent.
fn mai_calls(h: &Harness) -> Vec<Value> {
    h.lms
        .rpc_calls()
        .into_iter()
        .filter(|c| c[1][0] == "musicartistinfo")
        .collect()
}

/// The image-proxy fetches LMS was asked for.
fn image_paths(h: &Harness) -> Vec<String> {
    h.lms
        .http_paths()
        .into_iter()
        .filter(|p| p.starts_with("/imageproxy/"))
        .collect()
}

fn sorted(mut v: Vec<Value>) -> Vec<Value> {
    v.sort_by_key(|c| c.to_string());
    v
}

async fn fetch(h: &Harness, path: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(h.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn an_artist_has_a_clean_biography_and_a_proxied_photo() {
    let h = harness().await;
    // Before the gateway has handed the photo out, the proxy refuses it.
    assert_eq!(fetch(&h, PHOTO, &h.tok_a).await.status(), S::NOT_FOUND);
    assert!(image_paths(&h).is_empty());

    let (s, v) = get(&h, "/v1/info/artist/1", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({
            "biography": "The Invented Band formed in 1990 & played everywhere.\n\nSecond paragraph.\nLast line.",
            "photo": PHOTO,
            "credits": "Discogs"
        })
    );
    let text = v.to_string();
    for leak in ["/srv/", "script", "alert", "<", "images.example.com"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    // Exactly the id, nothing else, as a server query.
    assert_eq!(
        sorted(mai_calls(&h)),
        vec![
            json!(["", ["musicartistinfo", "artistphoto", "artist_id:1"]]),
            json!(["", ["musicartistinfo", "biography", "artist_id:1"]]),
        ]
    );

    // Now the photo comes through LMS's image proxy, as an image, for any signed-in user.
    let r = fetch(&h, PHOTO, &h.tok_b).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"IMAGEDATA");
    // Sized, too, as clients ask.
    let sized = format!("{PHOTO}_300x300_o");
    assert_eq!(fetch(&h, &sized, &h.tok_a).await.status(), S::OK);
    let fetched = image_paths(&h);
    assert_eq!(fetched.len(), 2);
    assert!(
        fetched
            .iter()
            .all(|p| p.starts_with("/imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fartist")),
        "{fetched:?}"
    );
    // Still not without a token.
    let r = reqwest::Client::new()
        .get(h.url(PHOTO))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_and_local_data_are_null() {
    let h = harness().await;
    let (s, v) = get(&h, "/v1/info/artist/2", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({"biography": "Plain words\n\nabout a singer.", "photo": null, "credits": null})
    );
    // A photo on the home network is never handed out, nor learned by the proxy.
    let (s, v) = get(&h, "/v1/info/artist/3", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({"biography": null, "photo": null, "credits": null})
    );
    let local =
        "/v1/art/imageproxy/http%3A%2F%2F10%2E0%2E0%2E10%3A9000%2Fplugins%2Fx%2Fartist%2Ejpg/image";
    assert_eq!(fetch(&h, local, &h.tok_a).await.status(), S::NOT_FOUND);
    // Nothing known.
    let (s, v) = get(&h, "/v1/info/artist/4", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    assert_eq!(
        v,
        json!({"biography": null, "photo": null, "credits": null})
    );
    assert!(image_paths(&h).is_empty());
}

#[tokio::test]
async fn albums_have_a_clean_review() {
    let h = harness().await;
    let (s, v) = get(&h, "/v1/info/album/1", Some(&h.tok_b)).await;
    assert_eq!(s, S::OK);
    assert_eq!(v, json!({"review": "A fine record — five stars."}));
    for id in [2, 4] {
        let (s, v) = get(&h, &format!("/v1/info/album/{id}"), Some(&h.tok_b)).await;
        assert_eq!(s, S::OK);
        assert_eq!(v, json!({"review": null}));
    }
    assert_eq!(
        mai_calls(&h),
        vec![
            json!(["", ["musicartistinfo", "albumreview", "album_id:1"]]),
            json!(["", ["musicartistinfo", "albumreview", "album_id:2"]]),
            json!(["", ["musicartistinfo", "albumreview", "album_id:4"]]),
        ]
    );
}

#[tokio::test]
async fn answers_are_remembered() {
    let h = harness().await;
    for _ in 0..3 {
        let (s, v) = get(&h, "/v1/info/artist/1", Some(&h.tok_a)).await;
        assert_eq!(s, S::OK);
        assert_eq!(v["photo"], PHOTO);
        let (s, _) = get(&h, "/v1/info/artist/4", Some(&h.tok_b)).await;
        assert_eq!(s, S::OK);
        let (s, _) = get(&h, "/v1/info/album/1", Some(&h.tok_a)).await;
        assert_eq!(s, S::OK);
    }
    // Asked once each (two lookups per artist).
    assert_eq!(mai_calls(&h).len(), 5);
    // Artist and album ids are separate.
    let (s, v) = get(&h, "/v1/info/album/4", Some(&h.tok_a)).await;
    assert_eq!((s, v), (S::OK, json!({"review": null})));
    assert_eq!(mai_calls(&h).len(), 6);
    // A remembered photo stays fetchable.
    assert_eq!(fetch(&h, PHOTO, &h.tok_a).await.status(), S::OK);
}

#[tokio::test]
async fn needs_a_valid_token() {
    let h = harness().await;
    for t in [None, Some("garbage"), Some("Bearer x.y.z")] {
        for p in ["/v1/info/artist/1", "/v1/info/album/1"] {
            let (s, _) = get(&h, p, t).await;
            assert_eq!(s, S::UNAUTHORIZED, "{p} {t:?}");
        }
    }
    assert!(mai_calls(&h).is_empty());
}

#[tokio::test]
async fn hostile_ids_and_parameters_are_refused_before_lms() {
    let h = harness().await;
    for kind in ["artist", "album"] {
        for id in [
            "0",
            "-1",
            "01",
            "+1",
            "1.5",
            "1e3",
            "0x10",
            "9999999999999",
            "18446744073709551616",
            "abc",
            "1%20",
            "%2Fsrv%2Fmusic%2Fa.flac",
            "..%2F..%2Fetc%2Fpasswd",
            "1;rescan",
            "1%0Arescan",
            "artist_id:1",
            "1%20album_id:2",
        ] {
            let (s, _) = get(&h, &format!("/v1/info/{kind}/{id}"), Some(&h.tok_a)).await;
            assert_eq!(s, S::NOT_FOUND, "{kind} {id}");
        }
        for q in ["?url=https://images.example.com/x", "?artist=a", "?"] {
            let (s, _) = get(&h, &format!("/v1/info/{kind}/2{q}"), Some(&h.tok_a)).await;
            if q == "?" {
                assert!(s == S::BAD_REQUEST || s == S::OK, "{q}: {s}");
            } else {
                assert_eq!(s, S::BAD_REQUEST, "{q}");
            }
        }
        // Nothing else is reachable under the path.
        for p in [
            format!("/v1/info/{kind}/1/extra"),
            format!("/v1/info/{kind}/"),
            "/v1/info/track/1".to_string(),
            "/v1/info/1".to_string(),
        ] {
            let r = fetch(&h, &p, &h.tok_a).await;
            assert_eq!(r.status(), S::NOT_FOUND, "{p}");
        }
    }
    // Only well-formed ids, in exactly the three plugin commands, ever reached LMS.
    for c in mai_calls(&h) {
        let words = c[1].as_array().unwrap();
        assert_eq!(words.len(), 3, "{c}");
        assert!(
            matches!(
                (words[1].as_str().unwrap(), words[2].as_str().unwrap()),
                ("biography" | "artistphoto", "artist_id:2") | ("albumreview", "album_id:2")
            ),
            "unexpected LMS call {c}"
        );
    }
}

#[tokio::test]
async fn a_stalled_lookup_is_empty_and_remembered_briefly() {
    // The plugin's online source can hang for one artist: the answer is empty (the app
    // hides the section) and remembered for a while, so each visit doesn't wait again.
    let h = harness_with(|c| c.info_lookup_timeout = Duration::from_millis(300)).await;
    for _ in 0..2 {
        let (s, v) = get(&h, "/v1/info/artist/900", Some(&h.tok_a)).await;
        assert_eq!(s, S::OK);
        assert_eq!(
            v,
            json!({"biography": null, "photo": null, "credits": null})
        );
        let (s, v) = get(&h, "/v1/info/album/900", Some(&h.tok_a)).await;
        assert_eq!(s, S::OK);
        assert_eq!(v, json!({"review": null}));
    }
    // Asked once each (both artist lookups, the review), then remembered.
    assert_eq!(mai_calls(&h).len(), 3);

    // Half an answer is given, and remembered as briefly.
    for _ in 0..2 {
        let (s, v) = get(&h, "/v1/info/artist/5", Some(&h.tok_a)).await;
        assert_eq!(s, S::OK);
        assert_eq!(
            v,
            json!({"biography": "Plain words\n\nabout a singer.", "photo": null, "credits": null})
        );
    }
    assert_eq!(mai_calls(&h).len(), 5);
    assert!(anywhere_gateway::info::STALL_TTL < anywhere_gateway::info::MISS_TTL);
}

#[tokio::test]
async fn plugin_lookups_are_limited() {
    let h = harness_with(|c| c.info_lookup_timeout = Duration::from_secs(3)).await;
    let n = anywhere_gateway::info::LOOKUPS;
    let mut slow = Vec::new();
    for i in 0..n {
        let url = h.url(&format!("/v1/info/album/{}", 900 + i));
        let tok = h.tok_a.clone();
        slow.push(tokio::spawn(async move {
            reqwest::Client::new()
                .get(url)
                .bearer_auth(tok)
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    let lms = &h.lms;
    wait_for(|| {
        lms.rpc_calls()
            .iter()
            .filter(|c| c[1][0] == "musicartistinfo")
            .count()
            == n
    })
    .await;
    // Every slot is taken: another lookup is refused at once, and asks nothing of LMS.
    let (s, v) = get(&h, "/v1/info/album/1", Some(&h.tok_b)).await;
    assert_eq!(s, S::TOO_MANY_REQUESTS);
    assert_eq!(
        v["error"],
        "too many information lookups; try again shortly"
    );
    let (s, _) = get(&h, "/v1/info/artist/1", Some(&h.tok_b)).await;
    assert_eq!(s, S::TOO_MANY_REQUESTS);
    assert_eq!(mai_calls(&h).len(), n);
    for t in slow {
        assert_eq!(t.await.unwrap(), S::OK, "timed out: an empty answer");
    }
    // Free again.
    let (s, _) = get(&h, "/v1/info/album/1", Some(&h.tok_b)).await;
    assert_eq!(s, S::OK);
}

#[tokio::test]
async fn shares_the_per_user_request_allowance() {
    let h = harness().await;
    let mut limited = false;
    for i in 0..anywhere_gateway::history::ME_REQUESTS_PER_MINUTE + 1 {
        let p = if i % 2 == 0 {
            "/v1/info/artist/2"
        } else {
            "/v1/info/album/1"
        };
        let (s, _) = get(&h, p, Some(&h.tok_a)).await;
        if s == S::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
    }
    assert!(limited);
    // Another user is unaffected.
    let (s, _) = get(&h, "/v1/info/artist/2", Some(&h.tok_b)).await;
    assert_eq!(s, S::OK);
}

/// An admin's token (carol), for the admin-only plugin tests.
fn admin_token(h: &Harness) -> String {
    h.accounts
        .create_user("carol", "correct horse battery", Role::Admin)
        .unwrap();
    h.accounts
        .login(
            "carol",
            "correct horse battery",
            None,
            "Laptop",
            None,
            "test",
        )
        .unwrap()
        .access
}

/// The plugin reserved for admins (`admin_only_plugins`): listeners are refused as in
/// `/v1/rpc`, even when an admin's answer is cached, and LMS is never asked for them
/// (review low: this path ignored the setting).
#[tokio::test]
async fn an_admin_only_plugin_gives_listeners_nothing() {
    let h = harness().await;
    let admin = admin_token(&h);
    h.accounts
        .set_admin_only_plugins(&["musicartistinfo".to_string()])
        .unwrap();
    // A listener first: refused before LMS.
    for path in ["/v1/info/artist/1", "/v1/info/album/1"] {
        let (s, v) = get(&h, path, Some(&h.tok_a)).await;
        assert_eq!(s, S::FORBIDDEN, "{path}");
        assert_eq!(
            v,
            json!({"error": "this plugin is for admins only"}),
            "{path}"
        );
    }
    assert!(mai_calls(&h).is_empty());
    // An admin gets the answer, which is then cached ...
    let (s, v) = get(&h, "/v1/info/artist/1", Some(&admin)).await;
    assert_eq!(s, S::OK);
    assert_eq!(v["photo"], PHOTO);
    let (s, _) = get(&h, "/v1/info/album/1", Some(&admin)).await;
    assert_eq!(s, S::OK);
    let asked = mai_calls(&h).len();
    // ... but never reaches a listener from the cache.
    for path in ["/v1/info/artist/1", "/v1/info/album/1"] {
        let (s, v) = get(&h, path, Some(&h.tok_b)).await;
        assert_eq!(s, S::FORBIDDEN, "{path}");
        assert_eq!(v, json!({"error": "this plugin is for admins only"}));
    }
    assert_eq!(mai_calls(&h).len(), asked);
    // Lifting the restriction applies at once.
    h.accounts.set_admin_only_plugins(&[]).unwrap();
    let (s, v) = get(&h, "/v1/info/artist/1", Some(&h.tok_a)).await;
    assert_eq!(s, S::OK);
    assert_eq!(v["photo"], PHOTO);
}
