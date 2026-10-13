//! Smart filters (docs/plans/wave2-design.md §5.6) against a fake LMS: saved per user,
//! never readable, runnable or removable by anyone else; run only as allowed `albums`
//! queries built from the parsed rules (a decade is one query per year, merged); and
//! every hostile body, id, query and over-the-limit request is refused.

mod common;
use common::*;

use anywhere_gateway::filters;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

impl Harness {
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (S, Value) {
        let c = reqwest::Client::new();
        let url = self.url(path);
        let mut req = match method {
            "GET" => c.get(url),
            "PUT" => c.put(url),
            "POST" => c.post(url),
            "DELETE" => c.delete(url),
            _ => unreachable!(),
        };
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let r = tokio::time::timeout(Duration::from_secs(30), req.send())
            .await
            .expect("request took too long")
            .unwrap();
        let status = r.status();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn raw(&self, method: &str, path: &str, token: &str, body: Vec<u8>) -> S {
        let c = reqwest::Client::new();
        let url = self.url(path);
        let req = match method {
            "PUT" => c.put(url),
            _ => c.post(url),
        };
        req.bearer_auth(token)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn create(&self, token: &str, filter: Value) -> i64 {
        let (s, v) = self
            .call("POST", "/v1/me/filters", Some(token), Some(filter))
            .await;
        assert_eq!(s, S::CREATED, "{v}");
        v["id"].as_i64().unwrap()
    }

    async fn run(&self, token: &str, id: i64, query: &str) -> (Vec<i64>, u64) {
        let (s, v) = self
            .call(
                "GET",
                &format!("/v1/me/filters/{id}/albums{query}"),
                Some(token),
                None,
            )
            .await;
        assert_eq!(s, S::OK, "{v}");
        assert_eq!(v.as_object().unwrap().len(), 2, "{v}");
        let ids = v["albums"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect();
        (ids, v["count"].as_u64().unwrap())
    }

    /// The `albums` queries LMS received.
    fn album_queries(&self) -> Vec<Vec<String>> {
        self.lms
            .rpc_calls()
            .into_iter()
            .filter(|p| p[1][0] == "albums")
            .map(|p| {
                assert_eq!(p[0], "", "albums is a server query");
                p[1].as_array()
                    .unwrap()
                    .iter()
                    .map(|w| w.as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }
}

/// A small invented library.
fn library() -> Vec<FakeAlbum> {
    let a = |id, title, artist, year, genres: &[u64], credits: &[(u64, &'static str)], comp| {
        FakeAlbum {
            id,
            title,
            artist,
            year,
            genres: genres.to_vec(),
            credits: credits.to_vec(),
            compilation: comp,
        }
    };
    vec![
        a(
            1,
            "Amber Lights",
            "Coral Sound",
            1971,
            &[3],
            &[(10, "ALBUMARTIST")],
            false,
        ),
        a(
            2,
            "Night Ferry",
            "Coral Sound",
            1975,
            &[3, 4],
            &[(10, "ALBUMARTIST"), (11, "COMPOSER")],
            false,
        ),
        a(
            3,
            "Harbour Mix",
            "Various",
            1978,
            &[4],
            &[(12, "ALBUMARTIST"), (10, "TRACKARTIST")],
            true,
        ),
        a(
            4,
            "Glass Fields",
            "Delta Choir",
            1983,
            &[3],
            &[(13, "ALBUMARTIST")],
            false,
        ),
        a(
            5,
            "Early Hours",
            "Bright Owls",
            1979,
            &[5],
            &[(14, "ALBUMARTIST"), (11, "COMPOSER")],
            false,
        ),
        a(
            6,
            "Cold Rivers",
            "Delta Choir",
            1970,
            &[3],
            &[(13, "ALBUMARTIST")],
            false,
        ),
    ]
}

async fn with_library() -> Harness {
    let h = harness().await;
    h.lms.set_albums(library());
    h
}

fn filter(name: &str, rules: Value, sort: &str, limit: u32) -> Value {
    json!({"name": name, "rules": rules, "sort": sort, "limit": limit})
}

/// Every word of every `albums` query is one the filter translation makes.
fn only_allowed_words(queries: &[Vec<String>]) {
    for q in queries {
        assert_eq!(q[0], "albums");
        assert!(q[1].bytes().all(|b| b.is_ascii_digit()), "{q:?}");
        assert!(q[2].bytes().all(|b| b.is_ascii_digit()), "{q:?}");
        for w in &q[3..] {
            let (k, v) = w.split_once(':').expect("key:value");
            let ok = match k {
                "genre_id" => v.split(',').all(|x| x.parse::<u64>().is_ok()),
                "artist_id" | "year" => v.parse::<u64>().is_ok(),
                "compilation" => v == "0" || v == "1",
                "role_id" => filters::ROLES.contains(&v),
                "sort" => ["album", "artistalbum", "yearalbum", "new", "random"].contains(&v),
                "tags" => v == "lay",
                _ => false,
            };
            assert!(ok, "{w:?} in {q:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Authentication and ownership
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_filter_endpoint_needs_a_valid_token() {
    let h = harness().await;
    for (m, p) in [
        ("GET", "/v1/me/filters"),
        ("POST", "/v1/me/filters"),
        ("PUT", "/v1/me/filters/1"),
        ("DELETE", "/v1/me/filters/1"),
        ("GET", "/v1/me/filters/1/albums"),
    ] {
        let body = (m == "POST" || m == "PUT").then(|| {
            filter(
                "F",
                json!([{"field": "compilation", "is": true}]),
                "album",
                5,
            )
        });
        for token in [None, Some("garbage"), Some("")] {
            let (s, _) = h.call(m, p, token, body.clone()).await;
            assert_eq!(s, S::UNAUTHORIZED, "{m} {p} with {token:?}");
        }
    }
    assert!(h.lms.rpc_calls().is_empty());
}

#[tokio::test]
async fn filters_are_kept_per_user_and_only_their_owner_can_use_them() {
    let h = with_library().await;
    let (s, v) = h.call("GET", "/v1/me/filters", Some(&h.tok_a), None).await;
    assert_eq!((s, v), (S::OK, json!({"filters": []})));
    let body = filter(
        "  Seventies   jazz ",
        json!([{"field": "genre", "in": [4, 3]}, {"field": "decade", "is": 1970}]),
        "yearalbum",
        200,
    );
    let (s, created) = h
        .call("POST", "/v1/me/filters", Some(&h.tok_a), Some(body))
        .await;
    assert_eq!(s, S::CREATED, "{created}");
    let id = created["id"].as_i64().unwrap();
    let want = json!({"id": id, "name": "Seventies jazz",
        "rules": [{"field": "genre", "in": [3, 4]}, {"field": "decade", "is": 1970}],
        "sort": "yearalbum", "limit": 200});
    assert_eq!(created, want);
    let (_, list) = h.call("GET", "/v1/me/filters", Some(&h.tok_a), None).await;
    assert_eq!(list, json!({"filters": [want]}));

    // Bob sees nothing of it, and can't replace, run or delete it: 404, as for no filter.
    let (_, bobs) = h.call("GET", "/v1/me/filters", Some(&h.tok_b), None).await;
    assert_eq!(bobs, json!({"filters": []}));
    let path = format!("/v1/me/filters/{id}");
    let replacement = filter(
        "Taken",
        json!([{"field": "compilation", "is": true}]),
        "new",
        5,
    );
    let (s, _) = h
        .call("PUT", &path, Some(&h.tok_b), Some(replacement.clone()))
        .await;
    assert_eq!(s, S::NOT_FOUND);
    let (s, _) = h
        .call("GET", &format!("{path}/albums"), Some(&h.tok_b), None)
        .await;
    assert_eq!(s, S::NOT_FOUND);
    let (s, _) = h.call("DELETE", &path, Some(&h.tok_b), None).await;
    assert_eq!(s, S::NOT_FOUND);
    assert!(h.album_queries().is_empty(), "nothing ran for bob");
    let (_, list) = h.call("GET", "/v1/me/filters", Some(&h.tok_a), None).await;
    assert_eq!(list["filters"][0]["name"], "Seventies jazz");

    // Alice replaces it (sending it back with its own id is fine; another id isn't).
    let (s, v) = h
        .call("PUT", &path, Some(&h.tok_a), Some(replacement.clone()))
        .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["name"], "Taken");
    assert_eq!(v["id"], id);
    let mut back = v.clone();
    back["limit"] = json!(6);
    let (s, v) = h
        .call("PUT", &path, Some(&h.tok_a), Some(back.clone()))
        .await;
    assert_eq!((s, v["limit"].clone()), (S::OK, json!(6)));
    back["id"] = json!(id + 1);
    let (s, _) = h.call("PUT", &path, Some(&h.tok_a), Some(back)).await;
    assert_eq!(s, S::BAD_REQUEST);
    let (s, _) = h
        .call(
            "PUT",
            &format!("/v1/me/filters/{}", id + 100),
            Some(&h.tok_a),
            Some(replacement),
        )
        .await;
    assert_eq!(s, S::NOT_FOUND);

    let (s, v) = h.call("DELETE", &path, Some(&h.tok_a), None).await;
    assert_eq!((s, v), (S::OK, json!({"removed": true})));
    let (s, _) = h.call("DELETE", &path, Some(&h.tok_a), None).await;
    assert_eq!(s, S::NOT_FOUND);
    let (s, _) = h
        .call("GET", &format!("{path}/albums"), Some(&h.tok_a), None)
        .await;
    assert_eq!(s, S::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_decade_is_one_query_per_year_merged_and_sorted() {
    let h = with_library().await;
    let rules = json!([{"field": "decade", "is": 1970}, {"field": "genre", "in": [3]}]);
    for (sort, want) in [
        ("album", vec![1, 6, 2]),
        ("artistalbum", vec![1, 2, 6]),
        ("yearalbum", vec![6, 1, 2]),
        ("new", vec![6, 2, 1]),
    ] {
        let id = h
            .create(&h.tok_a, filter(sort, rules.clone(), sort, 50))
            .await;
        let before = h.album_queries().len();
        let (ids, count) = h.run(&h.tok_a, id, "").await;
        assert_eq!((ids, count), (want.clone(), 3), "{sort}");
        let sent = &h.album_queries()[before..];
        assert_eq!(sent.len(), 10, "one query per year");
        for (i, q) in sent.iter().enumerate() {
            assert!(q.contains(&format!("year:{}", 1970 + i)), "{q:?}");
            assert!(q.contains(&"genre_id:3".to_string()), "{q:?}");
            assert!(q.contains(&format!("sort:{sort}")), "{q:?}");
        }
        // Paging within the result.
        let (page, count) = h.run(&h.tok_a, id, "?from=1&n=1").await;
        assert_eq!((page, count), (vec![want[1]], 3));
        let (page, _) = h.run(&h.tok_a, id, "?from=3&n=10").await;
        assert!(page.is_empty());
    }
    // The limit cuts the merged result.
    let id = h.create(&h.tok_a, filter("Two", rules, "album", 2)).await;
    assert_eq!(h.run(&h.tok_a, id, "").await, (vec![1, 6], 2));
    only_allowed_words(&h.album_queries());
}

#[tokio::test]
async fn artist_role_compilation_and_genre_rules_narrow_the_albums() {
    let h = with_library().await;
    let cases = [
        (
            json!([{"field": "artist", "is": 11, "role": "COMPOSER"}]),
            vec![5, 2],
        ),
        (json!([{"field": "artist", "is": 10}]), vec![1, 3, 2]),
        (
            json!([{"field": "artist", "is": 10, "role": "TRACKARTIST"}]),
            vec![3],
        ),
        (json!([{"field": "compilation", "is": true}]), vec![3]),
        (
            json!([{"field": "compilation", "is": false}, {"field": "genre", "in": [4, 5]}]),
            vec![5, 2],
        ),
        (
            json!([{"field": "compilation", "is": false}, {"field": "artist", "is": 10, "role": "TRACKARTIST"}]),
            vec![],
        ),
        (json!([{"field": "genre", "in": [99]}]), vec![]),
    ];
    for (rules, want) in cases {
        let id = h
            .create(&h.tok_a, filter("F", rules.clone(), "album", 100))
            .await;
        let before = h.album_queries().len();
        let (ids, count) = h.run(&h.tok_a, id, "").await;
        assert_eq!(ids, want, "{rules}");
        assert_eq!(count, want.len() as u64);
        assert_eq!(
            h.album_queries().len(),
            before + 1,
            "one query without a decade"
        );
        h.call(
            "DELETE",
            &format!("/v1/me/filters/{id}"),
            Some(&h.tok_a),
            None,
        )
        .await;
    }
    let sent = h.album_queries();
    assert!(sent.contains(&vec![
        "albums".to_string(),
        "0".into(),
        "100".into(),
        "artist_id:11".into(),
        "role_id:COMPOSER".into(),
        "sort:album".into(),
        "tags:lay".into()
    ]));
    only_allowed_words(&sent);
}

#[tokio::test]
async fn random_is_shuffled_and_its_pages_hold_together() {
    let h = with_library().await;
    let id = h
        .create(
            &h.tok_a,
            filter(
                "Shuffle",
                json!([{"field": "decade", "is": 1970}]),
                "random",
                10,
            ),
        )
        .await;
    let mut orders = std::collections::HashSet::new();
    for _ in 0..8 {
        let (first, count) = h.run(&h.tok_a, id, "?from=0&n=2").await;
        assert_eq!(count, 5);
        let (rest, _) = h.run(&h.tok_a, id, "?from=2&n=100").await;
        let mut all: Vec<i64> = first.iter().chain(rest.iter()).copied().collect();
        orders.insert(all.clone());
        all.sort();
        assert_eq!(
            all,
            vec![1, 2, 3, 5, 6],
            "later pages continue the same shuffle"
        );
    }
    assert!(orders.len() > 1, "never shuffled: {orders:?}");
}

#[tokio::test]
async fn lms_sees_only_allowed_queries_and_no_client_text() {
    let h = with_library().await;
    // Text in the name is cleaned and never reaches LMS; rules can't carry text at all.
    let id = h
        .create(
            &h.tok_a,
            filter(
                "x year:1999 sort:id search:file:///srv",
                json!([{"field": "artist", "is": 10, "role": "BAND"}, {"field": "decade", "is": 2000}]),
                "artistalbum",
                500,
            ),
        )
        .await;
    h.run(&h.tok_a, id, "?n=100").await;
    let sent = h.album_queries();
    assert_eq!(sent.len(), 10);
    only_allowed_words(&sent);
    let all = serde_json::to_string(&h.lms.rpc_calls()).unwrap();
    for leak in ["1999", "search", "file:", "/srv", "sort:id"] {
        assert!(!all.contains(leak), "{leak} reached LMS: {all}");
    }
    // Every query is also one /v1/rpc itself would allow.
    for q in &sent {
        assert!(filters::allowed(q), "{q:?}");
    }
    // LMS's own fields (file URLs) never come back.
    let (_, v) = h
        .call(
            "GET",
            &format!("/v1/me/filters/{id}/albums"),
            Some(&h.tok_a),
            None,
        )
        .await;
    assert!(!v.to_string().contains("srv"), "{v}");
    for cmd in h.lms.rpc_calls() {
        assert_eq!(cmd[1][0], "albums", "only albums queries: {cmd}");
    }
}

#[tokio::test]
async fn an_unreachable_music_server_is_502() {
    let h = harness_with(|c| c.lms_http = "127.0.0.1:1".into()).await;
    let id = h
        .create(
            &h.tok_a,
            filter(
                "F",
                json!([{"field": "compilation", "is": true}]),
                "album",
                5,
            ),
        )
        .await;
    let (s, v) = h
        .call(
            "GET",
            &format!("/v1/me/filters/{id}/albums"),
            Some(&h.tok_a),
            None,
        )
        .await;
    assert_eq!(s, S::BAD_GATEWAY, "{v}");
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hostile_bodies_ids_and_queries_are_refused() {
    let h = with_library().await;
    let good = filter("F", json!([{"field": "genre", "in": [3]}]), "album", 10);
    let with = |k: &str, v: Value| {
        let mut g = good.clone();
        g[k] = v;
        g
    };
    let bad = [
        json!(null),
        json!([]),
        json!("filter"),
        json!({}),
        with("id", json!(1)),
        with("owner", json!("bob")),
        with("user_id", json!(2)),
        with("name", json!("")),
        with("name", json!("x".repeat(65))),
        with("name", json!({"a": 1})),
        with("sort", json!("title")),
        with("sort", json!(null)),
        with("limit", json!(0)),
        with("limit", json!(501)),
        with("limit", json!("10")),
        with("rules", json!([])),
        with("rules", json!({"field": "genre", "in": [3]})),
        with(
            "rules",
            json!([{"field": "genre", "in": [3]}, {"field": "genre", "in": [4]}]),
        ),
        with("rules", json!([{"field": "genre", "in": [[3]]}])),
        with("rules", json!([{"field": "genre", "in": [{"$gt": 0}]}])),
        with(
            "rules",
            json!([{"field": "genre", "in": (1..=21).collect::<Vec<u64>>()}]),
        ),
        with("rules", json!([{"field": "genre", "in": ["3 sort:id"]}])),
        with("rules", json!([{"field": "decade", "is": 1975}])),
        with(
            "rules",
            json!([{"field": "artist", "is": 1, "role": "PRODUCER"}]),
        ),
        with("rules", json!([{"field": "artist", "is": "1 search:x"}])),
        with("rules", json!([{"field": "compilation", "is": "1"}])),
        with("rules", json!([{"field": "title", "is": "x"}])),
        with(
            "rules",
            json!([{"field": "genre", "in": [3], "search": "x"}]),
        ),
        with("rules", json!([[{"field": "genre", "in": [3]}]])),
    ];
    for b in bad {
        let (s, v) = h
            .call("POST", "/v1/me/filters", Some(&h.tok_a), Some(b.clone()))
            .await;
        assert_eq!(s, S::BAD_REQUEST, "accepted {b}: {v}");
        assert!(v["error"].is_string());
    }
    // Deeply nested, oversize and malformed bodies.
    let nested = format!("{}{}", "[".repeat(2000), "]".repeat(2000));
    assert_eq!(
        h.raw("POST", "/v1/me/filters", &h.tok_a, nested.into_bytes())
            .await,
        S::BAD_REQUEST
    );
    let mut big = good.clone();
    big["name"] = json!("x".repeat(5000));
    assert_eq!(
        h.raw(
            "POST",
            "/v1/me/filters",
            &h.tok_a,
            big.to_string().into_bytes()
        )
        .await,
        S::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        h.raw("POST", "/v1/me/filters", &h.tok_a, b"{\"name\":".to_vec())
            .await,
        S::BAD_REQUEST
    );
    let (_, list) = h.call("GET", "/v1/me/filters", Some(&h.tok_a), None).await;
    assert_eq!(list, json!({"filters": []}), "nothing was saved");

    let id = h.create(&h.tok_a, good.clone()).await;
    // A replacement is checked just as strictly.
    let (s, _) = h
        .call(
            "PUT",
            &format!("/v1/me/filters/{id}"),
            Some(&h.tok_a),
            Some(with("limit", json!(9999))),
        )
        .await;
    assert_eq!(s, S::BAD_REQUEST);
    for bad_id in [
        "0",
        "-1",
        "01",
        "abc",
        "1.0",
        "1e3",
        "%2e%2e",
        "99999999999999999",
    ] {
        for (m, p) in [
            ("PUT", format!("/v1/me/filters/{bad_id}")),
            ("DELETE", format!("/v1/me/filters/{bad_id}")),
            ("GET", format!("/v1/me/filters/{bad_id}/albums")),
        ] {
            let (s, _) = h.call(m, &p, Some(&h.tok_a), Some(good.clone())).await;
            assert_eq!(s, S::NOT_FOUND, "{m} {p}");
        }
    }
    for q in [
        "?n=0",
        "?n=101",
        "?n=-1",
        "?n=abc",
        "?from=501",
        "?from=-1",
        "?from=1.5",
        "?x=1",
        "?n=10&n=20",
        "?from=0&player=aa:bb:cc:dd:ee:01",
    ] {
        let (s, _) = h
            .call(
                "GET",
                &format!("/v1/me/filters/{id}/albums{q}"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{q}");
    }
    assert!(h.album_queries().is_empty(), "nothing hostile ran");
    // The edges are fine.
    h.run(&h.tok_a, id, "?from=500&n=100").await;
    h.run(&h.tok_a, id, "?n=1").await;
}

#[tokio::test]
async fn a_user_keeps_at_most_fifty_filters() {
    let h = harness().await;
    let f = filter(
        "F",
        json!([{"field": "compilation", "is": true}]),
        "album",
        5,
    );
    for _ in 0..filters::MAX_FILTERS_PER_USER {
        h.create(&h.tok_a, f.clone()).await;
    }
    let (s, v) = h
        .call("POST", "/v1/me/filters", Some(&h.tok_a), Some(f.clone()))
        .await;
    assert_eq!((s, v), (S::CONFLICT, json!({"error": "too many filters"})));
    // Bob has his own allowance.
    h.create(&h.tok_b, f).await;
}

#[tokio::test]
async fn running_filters_is_rate_limited_per_user() {
    let h = with_library().await;
    let id = h
        .create(
            &h.tok_a,
            filter(
                "F",
                json!([{"field": "compilation", "is": true}]),
                "album",
                5,
            ),
        )
        .await;
    let path = format!("/v1/me/filters/{id}/albums");
    let mut limited = None;
    for i in 0..filters::RUNS_PER_MINUTE + 1 {
        let (s, _) = h.call("GET", &path, Some(&h.tok_a), None).await;
        if s == S::TOO_MANY_REQUESTS {
            limited = Some(i);
            break;
        }
        assert_eq!(s, S::OK);
    }
    assert_eq!(limited, Some(filters::RUNS_PER_MINUTE));
    // Later pages of a kept result still work; nothing more reaches LMS.
    let before = h.album_queries().len();
    let (s, v) = h
        .call("GET", &format!("{path}?from=1"), Some(&h.tok_a), None)
        .await;
    assert_eq!((s, v["count"].clone()), (S::OK, json!(1)));
    assert_eq!(h.album_queries().len(), before);
    // Bob is unaffected.
    let bobs = h
        .create(
            &h.tok_b,
            filter(
                "F",
                json!([{"field": "compilation", "is": true}]),
                "album",
                5,
            ),
        )
        .await;
    h.run(&h.tok_b, bobs, "").await;
}

// ---------------------------------------------------------------------------
// Home shelves
// ---------------------------------------------------------------------------

#[tokio::test]
async fn filter_shelves_must_be_the_users_own() {
    let h = with_library().await;
    let f = filter("F", json!([{"field": "decade", "is": 1980}]), "album", 5);
    let mine = h.create(&h.tok_a, f.clone()).await;
    let bobs = h.create(&h.tok_b, f).await;
    let layout = |id: i64| json!({"version": 1, "shelves": [{"type": "recent"}, {"type": "filter", "id": id}]});
    let (s, v) = h
        .call("PUT", "/v1/me/home", Some(&h.tok_a), Some(layout(mine)))
        .await;
    assert_eq!(s, S::OK, "{v}");
    assert_eq!(v["shelves"][1], json!({"type": "filter", "id": mine}));
    for other in [bobs, mine + 1000] {
        let (s, v) = h
            .call("PUT", "/v1/me/home", Some(&h.tok_a), Some(layout(other)))
            .await;
        assert_eq!(s, S::BAD_REQUEST, "{v}");
        assert_eq!(v["error"], "no such filter");
    }
    for bad in [
        json!({"type": "filter"}),
        json!({"type": "filter", "id": "1"}),
        json!({"type": "filter", "id": 0}),
        json!({"type": "filter", "id": mine, "rules": []}),
    ] {
        let (s, _) = h
            .call(
                "PUT",
                "/v1/me/home",
                Some(&h.tok_a),
                Some(json!({"version": 1, "shelves": [bad]})),
            )
            .await;
        assert_eq!(s, S::BAD_REQUEST);
    }
    // A deleted filter's shelf is left out, as a deleted station's is.
    h.call(
        "DELETE",
        &format!("/v1/me/filters/{mine}"),
        Some(&h.tok_a),
        None,
    )
    .await;
    let (_, v) = h.call("GET", "/v1/me/home", Some(&h.tok_a), None).await;
    assert_eq!(v["shelves"], json!([{"type": "recent"}]));
    assert_eq!(v["default"], false);
}
