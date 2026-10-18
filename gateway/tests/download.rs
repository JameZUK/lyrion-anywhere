//! Offline downloads (`/v1/download/track/…`) against a fake LMS, with a fake `ffmpeg`
//! script standing in for the real one (no test needs ffmpeg installed).

mod common;
use common::*;

use std::path::{Path, PathBuf};

use anywhere_gateway::admin;
use anywhere_gateway::transcode::Ffmpeg;
use reqwest::StatusCode as S;
use serde_json::{Value, json};

const AUDIO_LEN: usize = 9 * 20_000;

/// What the fake ffmpeg writes as its "transcode".
fn fake_output() -> String {
    "FAKE-TRANSCODE-".repeat(64)
}

fn scratch(tag: &str) -> PathBuf {
    scratch_dir("anywhere-dl", tag)
}

/// A fake ffmpeg: answers `-encoders` like the real one, records each run's arguments in
/// `<dir>/runs`, then does `action` (by default, writes [`fake_output`] to the output file,
/// which is the last argument, `file:<path>`).
fn fake_ffmpeg(dir: &Path, action: Option<&str>) -> PathBuf {
    let path = dir.join("ffmpeg");
    let runs = dir.join("runs");
    let default = format!("printf '%s' '{}' > \"$out\"", fake_output());
    let script = format!(
        "#!/bin/sh\n\
         for a in \"$@\"; do\n\
           if [ \"$a\" = \"-encoders\" ]; then\n\
             echo ' A....D libmp3lame           MP3 (MPEG audio layer 3)'\n\
             echo ' A....D libopus              Opus'\n\
             exit 0\n\
           fi\n\
         done\n\
         printf '%s\\n' \"$@\" '--' >> '{runs}'\n\
         for a in \"$@\"; do last=\"$a\"; done\n\
         out=\"${{last#file:}}\"\n\
         {action}\n",
        runs = runs.display(),
        action = action.unwrap_or(&default),
    );
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Each ffmpeg run's arguments.
fn runs(dir: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(dir.join("runs")).unwrap_or_default();
    text.split("--\n")
        .filter(|r| !r.is_empty())
        .map(|r| r.lines().map(str::to_string).collect())
        .collect()
}

struct Dl {
    h: Harness,
    dir: PathBuf,
}

async fn with_ffmpeg(action: Option<&str>, tweak: impl FnOnce(&mut Config)) -> Dl {
    let dir = scratch("ff");
    let path = fake_ffmpeg(&dir, action);
    let cache = dir.join("cache");
    let h = harness_with(|c| {
        c.ffmpeg = Some(Ffmpeg {
            path,
            mp3: true,
            opus: true,
        });
        c.download_cache_dir = Some(cache);
        tweak(c);
    })
    .await;
    Dl { h, dir }
}

impl Harness {
    async fn get_dl(
        &self,
        path: &str,
        token: Option<&str>,
        range: Option<&str>,
    ) -> reqwest::Response {
        let mut req = reqwest::Client::new().get(self.url(path));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(r) = range {
            req = req.header("range", r);
        }
        tokio::time::timeout(Duration::from_secs(5), req.send())
            .await
            .expect("download request took too long")
            .unwrap()
    }

    async fn info(&self, id: &str, token: &str) -> reqwest::Response {
        self.get_dl(&format!("/v1/download/track/{id}/info"), Some(token), None)
            .await
    }

    /// Start the admin API on its own loopback listener.
    async fn admin_api(&self) -> (String, String) {
        let secret = "s".repeat(64);
        let app = admin::router(self.gw.clone(), secret.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(l, app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap()
        });
        (format!("http://{addr}"), secret)
    }
}

fn header(r: &reqwest::Response, name: &str) -> String {
    r.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}

fn audio(h: &Harness) -> Vec<u8> {
    h.lms.audio.as_ref().clone()
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

#[tokio::test]
async fn info_describes_a_local_track_without_revealing_its_path() {
    let d = with_ffmpeg(None, |_| {}).await;
    let h = &d.h;
    let r = h.info("1", &h.tok_a).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(header(&r, "cache-control"), "private, no-store");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["id"], 1);
    assert_eq!(v["title"], "Track");
    assert_eq!(v["artist"], "Artist");
    assert_eq!(v["album"], "Album");
    assert_eq!(v["album_id"], 7);
    assert_eq!(v["tracknum"], 3);
    assert_eq!(v["duration"], 241.2);
    assert_eq!(
        v["original"],
        json!({"format": "flc", "content_type": "audio/flac", "size": AUDIO_LEN, "ext": "flac"})
    );
    assert_eq!(v["formats"], json!(["original", "mp3", "opus"]));
    let key = v["key"].as_str().unwrap();
    assert_eq!(key.len(), 64);
    assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
    let text = v.to_string();
    for leak in ["srv", "music/", "file:", "Track.flac", "01%20"] {
        assert!(!text.contains(leak), "{leak} leaked: {text}");
    }
    use sha2::Digest;
    assert_ne!(
        key,
        hex::encode(sha2::Sha256::digest(TRACK_URL.as_bytes())),
        "the key is salted, not a bare hash of the path"
    );
    assert!(h.lms.downloads().is_empty(), "info never fetches the file");
}

#[tokio::test]
async fn keys_survive_a_rescan_and_differ_between_servers() {
    let h = harness().await;
    let key = |v: Value| v["key"].as_str().unwrap().to_string();
    let k1 = key(h.info("1", &h.tok_a).await.json().await.unwrap());
    // Track 7 is the same file, renumbered by a rescan.
    let k7 = key(h.info("7", &h.tok_a).await.json().await.unwrap());
    assert_eq!(k1, k7);
    // Another user sees the same key.
    assert_eq!(k1, key(h.info("1", &h.tok_b).await.json().await.unwrap()));
    let k3 = key(h.info("3", &h.tok_a).await.json().await.unwrap());
    assert_ne!(k1, k3);
    // Another gateway (its own secret salt) gives another key for the same file.
    let other = harness().await;
    assert_ne!(
        k1,
        key(other.info("1", &other.tok_a).await.json().await.unwrap())
    );
}

#[tokio::test]
async fn without_ffmpeg_only_the_original_is_offered() {
    let h = harness().await;
    let v: Value = h.info("1", &h.tok_a).await.json().await.unwrap();
    assert_eq!(v["formats"], json!(["original"]));
    for f in ["mp3", "opus"] {
        let r = h
            .get_dl(
                &format!("/v1/download/track/1?format={f}"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(r.status(), S::NOT_FOUND, "{f}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"], "format not available");
    }
    assert!(h.lms.downloads().is_empty());
}

#[tokio::test]
async fn types_ffmpeg_is_never_given_are_offered_as_original_only() {
    let d = with_ffmpeg(None, |_| {}).await;
    let h = &d.h;
    let v: Value = h.info("5", &h.tok_a).await.json().await.unwrap();
    assert_eq!(v["formats"], json!(["original"]));
    assert_eq!(v["original"]["ext"], "xyz");
    let r = h
        .get_dl("/v1/download/track/5?format=mp3", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert!(runs(&d.dir).is_empty(), "ffmpeg never ran");
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_tracks_are_not_downloadable() {
    let h = harness().await;
    let r = h.info("2", &h.tok_a).await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "not downloadable"
    );
    let r = h.get_dl("/v1/download/track/2", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "not downloadable"
    );
    assert!(h.lms.downloads().is_empty(), "LMS was never asked for it");
}

/// Malformed ids are refused without asking LMS anything; unknown ones after a lookup, but
/// never by fetching `/music/<id>/download` (which would hang on a real LMS).
#[tokio::test]
async fn bad_and_unknown_ids_never_reach_the_download_handler() {
    let h = harness().await;
    let garbage = [
        "0",
        "-1",
        "1e3",
        "01",
        "1.0",
        "%201",
        "abc",
        "..",
        "..%2F..%2Fmusic%2F1%2Fdownload",
        "1%2F..%2F2",
        "99999999999999999999999999999",
        "1234567890123",
        "0x1",
    ];
    for id in garbage {
        for path in [
            format!("/v1/download/track/{id}/info"),
            format!("/v1/download/track/{id}"),
        ] {
            let r = h.get_dl(&path, Some(&h.tok_a), None).await;
            assert_eq!(r.status(), S::NOT_FOUND, "{path}");
        }
    }
    let lookups = h.lms.rpc_calls();
    assert!(lookups.is_empty(), "malformed ids reached LMS: {lookups:?}");
    // Unknown (well-formed) ids: looked up, refused quickly.
    for id in ["999", "123456789012"] {
        let started = std::time::Instant::now();
        let r = h.info(id, &h.tok_a).await;
        assert_eq!(r.status(), S::NOT_FOUND);
        let r = h
            .get_dl(&format!("/v1/download/track/{id}"), Some(&h.tok_a), None)
            .await;
        assert_eq!(r.status(), S::NOT_FOUND);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    assert!(
        h.lms.downloads().is_empty(),
        "download handler reached: {:?}",
        h.lms.downloads()
    );
    // Traversal in the route itself never reaches LMS either.
    let r = h
        .get_dl(
            "/v1/download/track/1/../../../music/1/download",
            Some(&h.tok_a),
            None,
        )
        .await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert!(h.lms.downloads().is_empty());
}

#[tokio::test]
async fn unknown_query_parameters_and_formats_are_refused() {
    let h = harness().await;
    for q in [
        "format=flac",
        "format=",
        "format=MP3",
        "x=1",
        "format=original&x=1",
        "format=original&format=mp3",
    ] {
        let r = h
            .get_dl(&format!("/v1/download/track/1?{q}"), Some(&h.tok_a), None)
            .await;
        assert_eq!(r.status(), S::BAD_REQUEST, "{q}");
    }
    assert!(h.lms.downloads().is_empty());
}

#[tokio::test]
async fn missing_invalid_and_revoked_credentials_are_refused() {
    let h = harness().await;
    let paths = ["/v1/download/track/1/info", "/v1/download/track/1"];
    for p in paths {
        assert_eq!(
            h.get_dl(p, None, None).await.status(),
            S::UNAUTHORIZED,
            "{p}"
        );
        assert_eq!(
            h.get_dl(p, Some("not-a-token"), None).await.status(),
            S::UNAUTHORIZED
        );
        let r = reqwest::Client::new()
            .get(h.url(p))
            .header("authorization", format!("Basic {}", h.tok_a))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), S::UNAUTHORIZED, "{p}: wrong scheme");
    }
    // A revoked device's token stops working at once.
    let dev = h.accounts.authenticate(&h.tok_a).unwrap().device_id;
    h.accounts.revoke_device(dev).unwrap();
    for p in paths {
        assert_eq!(
            h.get_dl(p, Some(&h.tok_a), None).await.status(),
            S::UNAUTHORIZED
        );
    }
    // So does a logged-out one.
    h.accounts.logout(&h.tok_b).unwrap();
    assert_eq!(
        h.get_dl(paths[1], Some(&h.tok_b), None).await.status(),
        S::UNAUTHORIZED
    );
    assert!(h.lms.downloads().is_empty());
    assert!(h.lms.rpc_calls().is_empty());
}

#[tokio::test]
async fn downloads_can_be_turned_off_or_limited_per_user() {
    let d = with_ffmpeg(None, |_| {}).await;
    let h = &d.h;
    h.accounts.set_downloads("alice", false, None).unwrap();
    let r = h.info("1", &h.tok_a).await;
    assert_eq!(r.status(), S::FORBIDDEN);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "downloads are not allowed for this account"
    );
    for f in ["original", "mp3", "opus"] {
        let r = h
            .get_dl(
                &format!("/v1/download/track/1?format={f}"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{f}");
        assert_eq!(
            r.json::<Value>().await.unwrap()["error"],
            "downloads are not allowed for this account"
        );
    }
    assert!(h.lms.downloads().is_empty(), "nothing fetched when refused");
    // Bob is unaffected.
    assert_eq!(
        h.get_dl("/v1/download/track/1", Some(&h.tok_b), None)
            .await
            .status(),
        S::OK
    );

    // Opus only.
    h.accounts
        .set_downloads("alice", true, Some(&["opus".to_string()]))
        .unwrap();
    let v: Value = h.info("1", &h.tok_a).await.json().await.unwrap();
    assert_eq!(v["formats"], json!(["opus"]));
    for f in ["original", "mp3"] {
        let r = h
            .get_dl(
                &format!("/v1/download/track/1?format={f}"),
                Some(&h.tok_a),
                None,
            )
            .await;
        assert_eq!(r.status(), S::FORBIDDEN, "{f}");
        assert_eq!(
            r.json::<Value>().await.unwrap()["error"],
            "this format is not allowed for this account"
        );
    }
    let r = h
        .get_dl("/v1/download/track/1?format=opus", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::OK);
    // The account endpoint tells the app what it may do.
    let acct: Value = reqwest::Client::new()
        .get(h.url("/v1/account"))
        .bearer_auth(&h.tok_a)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        acct["downloads"],
        json!({"enabled": true, "formats": ["opus"]})
    );
}

// ---------------------------------------------------------------------------
// Originals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_original_is_relayed_with_download_headers() {
    let h = harness().await;
    let r = h
        .get_dl("/v1/download/track/1?format=original", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(header(&r, "content-type"), "audio/flac");
    assert_eq!(header(&r, "content-length"), AUDIO_LEN.to_string());
    assert_eq!(header(&r, "accept-ranges"), "bytes");
    assert_eq!(header(&r, "cache-control"), "private, no-store");
    assert_eq!(
        header(&r, "content-disposition"),
        "attachment; filename=\"03 Artist - Track.flac\""
    );
    assert_eq!(r.bytes().await.unwrap().as_ref(), audio(&h).as_slice());
    // Without ?format the original is sent too.
    let r = h.get_dl("/v1/download/track/1", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::OK);
    let asked = h.lms.downloads();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0], ("/music/1/download".to_string(), None));
    assert!(
        !h.lms.http_paths().iter().any(|p| p.contains("stream.mp3")),
        "never the endless transcoding stream"
    );
}

#[tokio::test]
async fn ranges_resume_downloads() {
    let h = harness().await;
    let a = audio(&h);
    let cases: [(&str, usize, usize); 4] = [
        ("bytes=0-99", 0, 99),
        ("bytes=179990-", 179_990, AUDIO_LEN - 1),
        ("bytes=-5", AUDIO_LEN - 5, AUDIO_LEN - 1),
        ("bytes=100-999999999", 100, AUDIO_LEN - 1),
    ];
    for (range, from, to) in cases {
        let r = h
            .get_dl("/v1/download/track/1", Some(&h.tok_a), Some(range))
            .await;
        assert_eq!(r.status(), S::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            header(&r, "content-range"),
            format!("bytes {from}-{to}/{AUDIO_LEN}")
        );
        assert_eq!(header(&r, "content-length"), (to - from + 1).to_string());
        assert_eq!(r.bytes().await.unwrap().as_ref(), &a[from..=to], "{range}");
        // LMS was asked for exactly that span.
        let (_, sent) = h.lms.downloads().pop().unwrap();
        assert_eq!(sent.unwrap(), format!("bytes={from}-{to}"));
    }
    let asked = h.lms.downloads().len();
    // Past the end: 416 without asking LMS.
    let r = h
        .get_dl(
            "/v1/download/track/1",
            Some(&h.tok_a),
            Some("bytes=180000-"),
        )
        .await;
    assert_eq!(r.status(), S::RANGE_NOT_SATISFIABLE);
    assert_eq!(header(&r, "content-range"), format!("bytes */{AUDIO_LEN}"));
    // Malformed or several ranges: 416.
    for bad in [
        "bytes=5-1",
        "bytes=0-1,4-5",
        "items=0-1",
        "bytes=x-",
        "bytes=-0",
    ] {
        let r = h
            .get_dl("/v1/download/track/1", Some(&h.tok_a), Some(bad))
            .await;
        assert_eq!(r.status(), S::RANGE_NOT_SATISFIABLE, "{bad}");
    }
    assert_eq!(
        h.lms.downloads().len(),
        asked,
        "refused ranges never reach LMS"
    );
}

/// Regression: LMS answers `bytes=0-0` with the whole file (206, Content-Range 0-end). The
/// gateway refused that as a wrong range (502); it now relays just the byte asked for.
#[tokio::test]
async fn a_range_wider_than_asked_is_trimmed_to_the_request() {
    let h = harness().await;
    let a = audio(&h);
    let r = h
        .get_dl("/v1/download/track/1", Some(&h.tok_a), Some("bytes=0-0"))
        .await;
    assert_eq!(r.status(), S::PARTIAL_CONTENT);
    assert_eq!(
        header(&r, "content-range"),
        format!("bytes 0-0/{AUDIO_LEN}")
    );
    assert_eq!(header(&r, "content-length"), "1");
    assert_eq!(r.bytes().await.unwrap().as_ref(), &a[0..=0]);
    let (_, sent) = h.lms.downloads().pop().unwrap();
    assert_eq!(
        sent.unwrap(),
        "bytes=0-0",
        "LMS was asked for exactly one byte"
    );
}

/// An LMS that ignores `Range` (and gives no size up front) still gives a correct 206.
#[tokio::test]
async fn ranges_work_even_if_lms_ignores_them() {
    let h = harness().await;
    let a = audio(&h);
    let r = h
        .get_dl("/v1/download/track/6", Some(&h.tok_a), Some("bytes=10-19"))
        .await;
    assert_eq!(r.status(), S::PARTIAL_CONTENT);
    assert_eq!(
        header(&r, "content-range"),
        format!("bytes 10-19/{AUDIO_LEN}")
    );
    assert_eq!(r.bytes().await.unwrap().as_ref(), &a[10..20]);
    let r = h
        .get_dl(
            "/v1/download/track/6",
            Some(&h.tok_a),
            Some("bytes=999999-"),
        )
        .await;
    assert_eq!(r.status(), S::RANGE_NOT_SATISFIABLE);
    let r = h.get_dl("/v1/download/track/6", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(r.bytes().await.unwrap().len(), AUDIO_LEN);
    let v: Value = h.info("6", &h.tok_a).await.json().await.unwrap();
    assert_eq!(v["original"]["size"], Value::Null);
}

#[tokio::test]
async fn hostile_tags_give_a_safe_file_name() {
    let h = harness().await;
    let r = h.get_dl("/v1/download/track/3", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::OK);
    let cd = header(&r, "content-disposition");
    assert!(r.headers().get("x-evil").is_none(), "no injected header");
    let (ascii, rest) = cd
        .strip_prefix("attachment; filename=\"")
        .and_then(|x| x.split_once('"'))
        .unwrap();
    assert!(ascii.is_ascii() && ascii.ends_with(".flac"), "{cd}");
    assert!(!ascii.contains(['/', '\\', '"', '\r', '\n']), "{cd}");
    assert!(!ascii.starts_with('.'), "{cd}");
    // The Unicode name is percent-encoded, without separators or the bidi override.
    let star = rest.strip_prefix("; filename*=UTF-8''").unwrap();
    assert!(!star.contains(['/', '\\', '"', ' ']), "{cd}");
    assert!(!star.contains("%E2%80%AE"), "{cd}");
    assert!(star.contains("%C3%A9"), "é kept in the Unicode name: {cd}");
}

#[tokio::test]
async fn a_known_track_whose_download_hangs_times_out() {
    let h = harness_with(|c| c.download_header_timeout = Duration::from_millis(300)).await;
    let started = std::time::Instant::now();
    let r = h.get_dl("/v1/download/track/4", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::GATEWAY_TIMEOUT);
    assert!(started.elapsed() < Duration::from_secs(3));
    // The slot was freed: the next download works.
    let r = h.get_dl("/v1/download/track/1", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::OK);
}

#[tokio::test]
async fn a_stalled_download_is_cut_off() {
    let h = harness_with(|c| c.download_read_timeout = Duration::from_millis(300)).await;
    let r = h.get_dl("/v1/download/track/8", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::OK);
    let body = tokio::time::timeout(Duration::from_secs(5), r.bytes())
        .await
        .expect("stalled body never ended");
    assert!(
        body.is_err(),
        "an incomplete download must not look complete"
    );
}

#[tokio::test]
async fn at_most_three_downloads_per_user_at_once() {
    let h = harness().await;
    // Track 8 stalls after its first bytes, so these stay open.
    let mut open = Vec::new();
    for _ in 0..3 {
        let r = h.get_dl("/v1/download/track/8", Some(&h.tok_a), None).await;
        assert_eq!(r.status(), S::OK);
        open.push(r);
    }
    let r = h.get_dl("/v1/download/track/1", Some(&h.tok_a), None).await;
    assert_eq!(r.status(), S::TOO_MANY_REQUESTS);
    assert_eq!(header(&r, "retry-after"), "5");
    // Info isn't a download.
    assert_eq!(h.info("1", &h.tok_a).await.status(), S::OK);
    // Another user isn't affected.
    let r = h.get_dl("/v1/download/track/1", Some(&h.tok_b), None).await;
    assert_eq!(r.status(), S::OK);
    // Closing one frees its slot.
    drop(open.pop());
    let mut freed = false;
    for _ in 0..100 {
        let r = h.get_dl("/v1/download/track/1", Some(&h.tok_a), None).await;
        if r.status() == S::OK {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(freed, "slot not freed after the client went away");
}

#[tokio::test]
async fn downloads_are_audited_without_flooding_the_log() {
    let h = harness().await;
    for range in [None, Some("bytes=0-9"), Some("bytes=10-")] {
        let r = h
            .get_dl("/v1/download/track/1", Some(&h.tok_a), range)
            .await;
        assert!(r.status().is_success());
    }
    let r = h.get_dl("/v1/download/track/7", Some(&h.tok_a), None).await;
    assert!(r.status().is_success());
    let entries: Vec<_> = h
        .accounts
        .audit_log(100)
        .unwrap()
        .into_iter()
        .filter(|e| e.event == "download")
        .collect();
    let details: Vec<&str> = entries.iter().map(|e| e.detail.as_str()).collect();
    assert_eq!(details, ["track 7 original", "track 1 original"]);
    assert_eq!(entries[0].username.as_deref(), Some("alice"));
}

// ---------------------------------------------------------------------------
// Transcodes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transcodes_are_made_by_ffmpeg_cached_and_served_with_ranges() {
    let d = with_ffmpeg(None, |_| {}).await;
    let h = &d.h;
    let want = fake_output();
    let r = h
        .get_dl("/v1/download/track/1?format=mp3", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(header(&r, "content-type"), "audio/mpeg");
    assert_eq!(header(&r, "content-length"), want.len().to_string());
    assert_eq!(header(&r, "accept-ranges"), "bytes");
    assert_eq!(header(&r, "cache-control"), "private, no-store");
    assert_eq!(
        header(&r, "content-disposition"),
        "attachment; filename=\"03 Artist - Track.mp3\""
    );
    assert_eq!(r.text().await.unwrap(), want);

    let all = runs(&d.dir);
    assert_eq!(all.len(), 1);
    let args = &all[0];
    let at = |w: &str| {
        args.iter()
            .position(|a| a == w)
            .unwrap_or_else(|| panic!("{w} missing: {args:?}"))
    };
    assert_eq!(args[at("-protocol_whitelist") + 1], "file");
    assert_eq!(
        args[at("-f") + 1],
        "flac",
        "input format forced from LMS's type"
    );
    let input = &args[at("-i") + 1];
    let cache = d.dir.join("cache");
    assert!(
        input.starts_with(&format!("file:{}/.tmp-", cache.display())),
        "input is a file the gateway wrote: {input}"
    );
    assert!(args.contains(&"libmp3lame".to_string()));
    assert!(args.contains(&"title=Track".to_string()));
    assert!(
        !args
            .iter()
            .any(|a| a.contains("http") || a.contains("/srv/music"))
    );
    // The original was fetched from LMS's download handler, whole.
    assert_eq!(h.lms.downloads(), [("/music/1/download".to_string(), None)]);
    // One finished cache entry, no leftovers.
    let names: Vec<String> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with(".mp3") && names[0].len() == 68);

    // Again (even as the renumbered track 7): from the cache, ffmpeg not run.
    let r = h
        .get_dl(
            "/v1/download/track/7?format=mp3",
            Some(&h.tok_a),
            Some("bytes=15-29"),
        )
        .await;
    assert_eq!(r.status(), S::PARTIAL_CONTENT);
    assert_eq!(
        header(&r, "content-range"),
        format!("bytes 15-29/{}", want.len())
    );
    assert_eq!(r.text().await.unwrap(), want[15..30]);
    let r = h
        .get_dl(
            "/v1/download/track/1?format=mp3",
            Some(&h.tok_a),
            Some(&format!("bytes={}-", want.len())),
        )
        .await;
    assert_eq!(r.status(), S::RANGE_NOT_SATISFIABLE);
    assert_eq!(runs(&d.dir).len(), 1, "served from the cache");

    // Opus is its own entry.
    let r = h
        .get_dl("/v1/download/track/1?format=opus", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::OK);
    assert_eq!(header(&r, "content-type"), "audio/ogg");
    assert!(header(&r, "content-disposition").ends_with(".opus\""));
    let all = runs(&d.dir);
    assert_eq!(all.len(), 2);
    assert!(all[1].contains(&"libopus".to_string()));
}

#[tokio::test]
async fn the_transcode_cache_evicts_the_least_recently_used() {
    let size = fake_output().len() as u64;
    // Room for one transcode, not two.
    let d = with_ffmpeg(None, |c| c.download_cache_bytes = size + size / 2).await;
    let h = &d.h;
    let get = |id: u64| {
        let path = format!("/v1/download/track/{id}?format=mp3");
        async move {
            let r = h.get_dl(&path, Some(&h.tok_a), None).await;
            assert_eq!(r.status(), S::OK);
            r.bytes().await.unwrap();
        }
    };
    get(1).await;
    get(1).await;
    assert_eq!(runs(&d.dir).len(), 1);
    get(3).await; // evicts track 1
    assert_eq!(runs(&d.dir).len(), 2);
    get(3).await;
    assert_eq!(runs(&d.dir).len(), 2, "track 3 still cached");
    get(1).await; // transcoded again
    assert_eq!(runs(&d.dir).len(), 3);
    let files = std::fs::read_dir(d.dir.join("cache")).unwrap().count();
    assert_eq!(files, 1, "the cache holds one transcode");
}

#[tokio::test]
async fn a_transcode_that_runs_too_long_is_killed() {
    let d = with_ffmpeg(Some("exec sleep 30"), |c| {
        c.transcode_timeout = Duration::from_millis(300)
    })
    .await;
    let h = &d.h;
    let started = std::time::Instant::now();
    let r = h
        .get_dl("/v1/download/track/1?format=opus", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::GATEWAY_TIMEOUT);
    assert!(started.elapsed() < Duration::from_secs(4));
    // Nothing half-made is left behind or cached.
    let cache = d.dir.join("cache");
    wait_for(|| std::fs::read_dir(&cache).unwrap().count() == 0).await;
}

#[tokio::test]
async fn a_failed_transcode_is_not_cached() {
    let d = with_ffmpeg(Some("exit 1"), |_| {}).await;
    let h = &d.h;
    for _ in 0..2 {
        let r = h
            .get_dl("/v1/download/track/1?format=mp3", Some(&h.tok_a), None)
            .await;
        assert_eq!(r.status(), S::BAD_GATEWAY);
    }
    assert_eq!(runs(&d.dir).len(), 2, "tried again: nothing was cached");
    // An empty output is a failure too.
    let d = with_ffmpeg(Some(": > \"$out\""), |_| {}).await;
    let r =
        d.h.get_dl("/v1/download/track/1?format=mp3", Some(&d.h.tok_a), None)
            .await;
    assert_eq!(r.status(), S::BAD_GATEWAY);
}

#[tokio::test]
async fn remote_tracks_are_never_transcoded() {
    let d = with_ffmpeg(None, |_| {}).await;
    let h = &d.h;
    let r = h
        .get_dl("/v1/download/track/2?format=opus", Some(&h.tok_a), None)
        .await;
    assert_eq!(r.status(), S::NOT_FOUND);
    assert!(runs(&d.dir).is_empty());
    assert!(h.lms.downloads().is_empty());
}

#[tokio::test]
async fn ffmpeg_is_probed_for_its_encoders() {
    let dir = scratch("probe");
    let path = fake_ffmpeg(&dir, None);
    let f = anywhere_gateway::transcode::probe(&path, Duration::from_secs(5)).unwrap();
    assert!(f.mp3 && f.opus);
    assert_eq!(f.path, path);
    // A program without either encoder is not used.
    let none = dir.join("none");
    std::fs::write(&none, "#!/bin/sh\necho ' A....D aac AAC'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&none, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(anywhere_gateway::transcode::probe(&none, Duration::from_secs(5)).is_none());
    // One that hangs is given up on.
    let hang = dir.join("hang");
    std::fs::write(&hang, "#!/bin/sh\nexec sleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hang, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let started = std::time::Instant::now();
    assert!(anywhere_gateway::transcode::probe(&hang, Duration::from_millis(300)).is_none());
    assert!(started.elapsed() < Duration::from_secs(5));
}

// ---------------------------------------------------------------------------
// Admin API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admins_set_download_permissions() {
    let d = with_ffmpeg(None, |_| {}).await;
    let h = &d.h;
    let (base, secret) = h.admin_api().await;
    let c = reqwest::Client::new();
    let users = || async {
        let v: Value = c
            .get(format!("{base}/admin/v1/users"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        v["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == "alice")
            .cloned()
            .unwrap()
    };
    let set = |name: &str, body: Value| {
        c.post(format!("{base}/admin/v1/users/{name}/downloads"))
            .bearer_auth(&secret)
            .json(&body)
            .send()
    };
    let u = users().await;
    assert_eq!(u["downloads"], true);
    assert_eq!(u["download_formats"], json!(["original", "mp3", "opus"]));

    let r = set(
        "alice",
        json!({"enabled": true, "formats": ["mp3", "original"]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    let u = users().await;
    assert_eq!(u["download_formats"], json!(["original", "mp3"]));
    let r = set("alice", json!({"enabled": false})).await.unwrap();
    assert_eq!(r.status(), S::NO_CONTENT);
    assert_eq!(users().await["downloads"], false);
    assert_eq!(
        h.info("1", &h.tok_a).await.status(),
        S::FORBIDDEN,
        "applies at once"
    );

    for bad in [
        json!({"enabled": true, "formats": []}),
        json!({"enabled": true, "formats": ["flac"]}),
        json!({"enabled": true, "formats": "mp3"}),
        json!({"enabled": "yes"}),
        json!({"formats": ["mp3"]}),
        json!({"enabled": true, "admin": true}),
    ] {
        let st = set("alice", bad.clone()).await.unwrap().status();
        assert!(st.is_client_error(), "{bad} gave {st}");
    }
    assert_eq!(
        users().await["download_formats"],
        json!(["original", "mp3"])
    );
    let st = set("nobody", json!({"enabled": true}))
        .await
        .unwrap()
        .status();
    assert_eq!(st, S::BAD_REQUEST);
    // Without the secret: refused.
    let r = c
        .post(format!("{base}/admin/v1/users/alice/downloads"))
        .json(&json!({"enabled": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), S::FORBIDDEN);
    // The settings say what this server can make.
    let s: Value = c
        .get(format!("{base}/admin/v1/settings"))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["download_formats"], json!(["original", "mp3", "opus"]));
}
