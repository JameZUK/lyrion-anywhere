//! The pinned `cloudflared` download: fetched from the official GitHub release, checked
//! against the SHA-256 compiled into this release ([`super::pins`]) before it is renamed
//! into place, and nothing else.
//!
//! Refused: a sum that differs, a body over the size cap (64 MiB), plain HTTP, a redirect
//! to any host that isn't `github.com` (or under it) or under `githubusercontent.com`, more
//! than two redirects, and a download that takes longer than ten minutes. The body is
//! written to a temporary file in `<data>/tunnel/`, synced, and renamed over
//! `<data>/tunnel/cloudflared` (mode 0700) only once its sum is right.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use super::pins;

/// Largest download accepted.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Longest the whole download may take.
pub const TIME_LIMIT: Duration = Duration::from_secs(600);
/// Redirects followed at most (GitHub sends one, to its asset host).
pub const MAX_REDIRECTS: usize = 2;
/// Where the release assets are.
pub const RELEASES: &str = "https://github.com/cloudflare/cloudflared/releases/download";

/// The release asset for this machine, by Rust's names for the OS and architecture
/// (`std::env::consts`). Only the plugin's three Linux platforms are supported.
pub fn asset_for(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    if os != "linux" {
        return None;
    }
    match arch {
        "x86_64" => Some(("cloudflared-linux-amd64", pins::SHA256_AMD64)),
        "aarch64" => Some(("cloudflared-linux-arm64", pins::SHA256_ARM64)),
        // Rust's "arm" is 32-bit ARM; the plugin's builds are hard-float (armv7, armhf).
        "arm" => Some(("cloudflared-linux-armhf", pins::SHA256_ARMHF)),
        _ => None,
    }
}

/// Which addresses a download may come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlPolicy {
    /// HTTPS only, from GitHub's hosts: production.
    GitHub,
    /// As [`UrlPolicy::GitHub`], and also anything at exactly this origin (e.g.
    /// `http://127.0.0.1:4000`). Tests only: never set from the command line.
    TestOrigin(String),
}

/// Why a download was refused or failed. Each says what happened in a sentence the admin
/// can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    PlainHttp,
    NotGitHub(String),
    TooManyRedirects,
    BadRedirect,
    Status(u16),
    TooBig,
    WrongSum,
    TimedOut,
    Network(String),
    Write(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::PlainHttp => f.write_str(
                "The cloudflared download was sent to a plain-HTTP address, so it was refused.",
            ),
            DownloadError::NotGitHub(host) => write!(
                f,
                "The cloudflared download was redirected to {host}, which is not GitHub, so it was refused."
            ),
            DownloadError::TooManyRedirects => f.write_str(
                "The cloudflared download was redirected too many times, so it was refused.",
            ),
            DownloadError::BadRedirect => {
                f.write_str("GitHub sent a redirect this gateway could not follow.")
            }
            DownloadError::Status(s) => write!(
                f,
                "GitHub answered {s} for the cloudflared download, so nothing was installed."
            ),
            DownloadError::TooBig => write!(
                f,
                "The cloudflared download was larger than {} MiB, so it was refused.",
                MAX_BYTES / 1024 / 1024
            ),
            DownloadError::WrongSum => f.write_str(
                "The downloaded cloudflared did not match the checksum pinned in this release, so it was not installed.",
            ),
            DownloadError::TimedOut => f.write_str(
                "The cloudflared download took longer than 10 minutes and was abandoned.",
            ),
            DownloadError::Network(e) => write!(
                f,
                "Could not download cloudflared from GitHub ({e}). The LMS server must be able to reach github.com."
            ),
            DownloadError::Write(e) => write!(
                f,
                "Could not write cloudflared to the gateway's data folder ({e})."
            ),
        }
    }
}

/// Is `url` somewhere a download may come from?
pub fn check_url(url: &reqwest::Url, policy: &UrlPolicy) -> Result<(), DownloadError> {
    if let UrlPolicy::TestOrigin(origin) = policy
        && url.origin().ascii_serialization() == *origin
    {
        return Ok(());
    }
    if url.scheme() != "https" {
        return Err(DownloadError::PlainHttp);
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let github = host == "github.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com");
    if !github || url.port().is_some_and(|p| p != 443) || !url.username().is_empty() {
        return Err(DownloadError::NotGitHub(host));
    }
    Ok(())
}

/// What a download is to be checked against.
#[derive(Debug, Clone)]
pub struct Fetch {
    pub url: String,
    /// Lower-case hex SHA-256.
    pub sha256: String,
    pub max_bytes: u64,
    pub time_limit: Duration,
    pub policy: UrlPolicy,
}

/// Download to a temporary file in `dir`, check its sum, and rename it to `dir/<name>`
/// (mode 0700). `progress(bytes, total)` is called as the body arrives. A refused or failed
/// download leaves nothing behind.
pub async fn fetch(
    client: &reqwest::Client,
    job: &Fetch,
    dir: &Path,
    name: &str,
    progress: impl Fn(u64, Option<u64>),
) -> Result<PathBuf, DownloadError> {
    let tmp = dir.join(format!(".{name}.download-{}", crate::random_id()));
    let result = tokio::time::timeout(job.time_limit, fetch_into(client, job, &tmp, &progress))
        .await
        .unwrap_or(Err(DownloadError::TimedOut));
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    let dest = dir.join(name);
    let renamed = async {
        set_mode(&tmp, 0o700).await?;
        tokio::fs::rename(&tmp, &dest).await?;
        sync_dir(dir).await;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if let Err(e) = renamed {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(DownloadError::Write(e.to_string()));
    }
    Ok(dest)
}

async fn fetch_into(
    client: &reqwest::Client,
    job: &Fetch,
    tmp: &Path,
    progress: &impl Fn(u64, Option<u64>),
) -> Result<(), DownloadError> {
    use tokio::io::AsyncWriteExt;
    let net = |e: reqwest::Error| DownloadError::Network(e.without_url().to_string());
    let mut url = reqwest::Url::parse(&job.url).map_err(|_| DownloadError::BadRedirect)?;
    let mut redirects = 0;
    let resp = loop {
        check_url(&url, &job.policy)?;
        let resp = client.get(url.clone()).send().await.map_err(net)?;
        if resp.status().is_redirection() {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(DownloadError::TooManyRedirects);
            }
            let next = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| url.join(l).ok())
                .ok_or(DownloadError::BadRedirect)?;
            url = next;
            continue;
        }
        break resp;
    };
    if !resp.status().is_success() {
        return Err(DownloadError::Status(resp.status().as_u16()));
    }
    let total = resp.content_length();
    if total.is_some_and(|t| t > job.max_bytes) {
        return Err(DownloadError::TooBig);
    }
    let write = |e: std::io::Error| DownloadError::Write(e.to_string());
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut file = opts.open(tmp).await.map_err(write)?;
    let mut hasher = Sha256::new();
    let mut got: u64 = 0;
    let mut body = resp.bytes_stream();
    progress(0, total);
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(net)?;
        got += chunk.len() as u64;
        if got > job.max_bytes {
            return Err(DownloadError::TooBig);
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(write)?;
        progress(got, total);
    }
    file.flush().await.map_err(write)?;
    file.sync_all().await.map_err(write)?;
    drop(file);
    let sum = hex::encode(hasher.finalize());
    if !sum.eq_ignore_ascii_case(&job.sha256) {
        return Err(DownloadError::WrongSum);
    }
    Ok(())
}

async fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Make a rename in `dir` durable (best effort).
pub async fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = tokio::fs::File::open(dir).await {
        let _ = d.sync_all().await;
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// `cloudflared --version`'s output, `cloudflared version 2026.10.1 (built …)`: the version.
pub fn parse_version_output(out: &str) -> Option<String> {
    let line = out.lines().find(|l| l.contains("version"))?;
    let mut words = line.split_whitespace();
    words.by_ref().find(|w| *w == "version")?;
    let v = words.next()?;
    let ok = !v.is_empty()
        && v.len() <= 32
        && v.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
    ok.then(|| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Redirect};
    use axum::routing::get;

    #[test]
    fn each_platform_has_its_asset() {
        assert_eq!(
            asset_for("linux", "x86_64"),
            Some(("cloudflared-linux-amd64", pins::SHA256_AMD64))
        );
        assert_eq!(
            asset_for("linux", "aarch64"),
            Some(("cloudflared-linux-arm64", pins::SHA256_ARM64))
        );
        assert_eq!(
            asset_for("linux", "arm"),
            Some(("cloudflared-linux-armhf", pins::SHA256_ARMHF))
        );
        for (os, arch) in [
            ("linux", "x86"),
            ("linux", "riscv64"),
            ("linux", "powerpc64"),
            ("macos", "aarch64"),
            ("windows", "x86_64"),
            ("freebsd", "x86_64"),
        ] {
            assert_eq!(asset_for(os, arch), None, "{os} {arch}");
        }
    }

    #[test]
    fn only_https_github_addresses_are_allowed() {
        let ok = |u: &str| check_url(&reqwest::Url::parse(u).unwrap(), &UrlPolicy::GitHub);
        for good in [
            "https://github.com/cloudflare/cloudflared/releases/download/2026.10.1/x",
            "https://objects.githubusercontent.com/github-production-release-asset/x",
            "https://release-assets.githubusercontent.com/x?sig=1",
            "https://codeload.github.com/x",
            "https://GITHUB.com/x",
        ] {
            assert_eq!(ok(good), Ok(()), "{good}");
        }
        assert_eq!(
            ok("http://github.com/cloudflare/cloudflared"),
            Err(DownloadError::PlainHttp)
        );
        assert_eq!(
            ok("http://objects.githubusercontent.com/x"),
            Err(DownloadError::PlainHttp)
        );
        assert_eq!(ok("ftp://github.com/x"), Err(DownloadError::PlainHttp));
        for bad in [
            "https://evilgithub.com/x",
            "https://github.com.evil.example/x",
            "https://githubusercontent.com.evil.example/x",
            "https://evilgithubusercontent.com/x",
            "https://example.com/x",
            "https://192.0.2.1/x",
            "https://github.com:8443/x",
            "https://user@github.com/x",
        ] {
            assert!(
                matches!(ok(bad), Err(DownloadError::NotGitHub(_))),
                "{bad}: {:?}",
                ok(bad)
            );
        }
        // The test origin is allowed only exactly.
        let test = UrlPolicy::TestOrigin("http://127.0.0.1:4000".into());
        let t = |u: &str| check_url(&reqwest::Url::parse(u).unwrap(), &test);
        assert_eq!(t("http://127.0.0.1:4000/a"), Ok(()));
        assert_eq!(t("http://127.0.0.1:4001/a"), Err(DownloadError::PlainHttp));
        assert_eq!(t("https://github.com/a"), Ok(()));
    }

    #[test]
    fn version_output_is_parsed() {
        assert_eq!(
            parse_version_output("cloudflared version 2026.10.1 (built 2026-10-15-17:08 UTC)\n"),
            Some("2026.10.1".into())
        );
        assert_eq!(
            parse_version_output("cloudflared version 2025.1.0\n"),
            Some("2025.1.0".into())
        );
        for bad in [
            "",
            "cloudflared",
            "cloudflared version",
            "cloudflared version DEV",
            "cloudflared version 2026.10.1;rm",
            "\u{0}\u{1}",
        ] {
            assert_eq!(parse_version_output(bad), None, "{bad:?}");
        }
    }

    // ------------------------------------------------------------------
    // The verifier against a small asset server.
    // ------------------------------------------------------------------

    const BODY: &[u8] = b"#!/bin/sh\necho pretend cloudflared\n";

    async fn server() -> String {
        let app = Router::new()
            .route("/asset", get(|| async { BODY }))
            .route("/one", get(|| async { Redirect::temporary("/asset") }))
            .route("/two", get(|| async { Redirect::temporary("/one") }))
            .route("/three", get(|| async { Redirect::temporary("/two") }))
            .route(
                "/away",
                get(|| async { Redirect::temporary("https://evil.example/asset") }),
            )
            .route(
                "/plain",
                get(|| async { Redirect::temporary("http://objects.githubusercontent.com/a") }),
            )
            .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
            .route("/big", get(|| async { vec![0u8; 5000].into_response() }))
            .route(
                "/endless",
                get(|| async {
                    let s = futures_util::stream::repeat_with(|| {
                        Ok::<_, std::io::Error>(bytes::Bytes::from_static(&[1u8; 1024]))
                    });
                    axum::body::Body::from_stream(s)
                }),
            )
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    BODY
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client() -> reqwest::Client {
        crate::http_client_builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .unwrap()
    }

    fn job(base: &str, path: &str, sum: &str) -> Fetch {
        Fetch {
            url: format!("{base}{path}"),
            sha256: sum.into(),
            max_bytes: 1000,
            time_limit: Duration::from_secs(5),
            policy: UrlPolicy::TestOrigin(base.into()),
        }
    }

    fn good_sum() -> String {
        hex::encode(Sha256::digest(BODY))
    }

    async fn run(job: &Fetch) -> (Result<PathBuf, DownloadError>, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("anywhere-cfdl-{}", crate::random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = fetch(&client(), job, &dir, "cloudflared", |_, _| {}).await;
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        (r, left)
    }

    #[tokio::test]
    async fn a_good_download_is_renamed_into_place_with_mode_0700() {
        let base = server().await;
        let dir = std::env::temp_dir().join(format!("anywhere-cfdl-{}", crate::random_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let seen = std::sync::Mutex::new(vec![]);
        let path = fetch(
            &client(),
            &job(&base, "/one", &good_sum()),
            &dir,
            "cloudflared",
            |b, t| seen.lock().unwrap().push((b, t)),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), BODY);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file left");
        assert_eq!(
            seen.lock().unwrap().last().copied(),
            Some((BODY.len() as u64, Some(BODY.len() as u64)))
        );
        let _ = std::fs::remove_dir_all(&dir);
        // Upper-case pins are compared as hex.
        assert!(
            run(&job(&base, "/asset", &good_sum().to_uppercase()))
                .await
                .0
                .is_ok()
        );
    }

    #[tokio::test]
    async fn the_verifier_refuses_what_it_should_and_leaves_nothing_behind() {
        let base = server().await;
        let sum = good_sum();
        let wrong = "0".repeat(64);
        let cases: Vec<(Fetch, DownloadError)> = vec![
            (job(&base, "/asset", &wrong), DownloadError::WrongSum),
            (job(&base, "/three", &sum), DownloadError::TooManyRedirects),
            (
                job(&base, "/away", &sum),
                DownloadError::NotGitHub("evil.example".into()),
            ),
            (job(&base, "/plain", &sum), DownloadError::PlainHttp),
            (job(&base, "/missing", &sum), DownloadError::Status(404)),
            (job(&base, "/big", &sum), DownloadError::TooBig),
            (job(&base, "/endless", &sum), DownloadError::TooBig),
            (
                Fetch {
                    time_limit: Duration::from_millis(300),
                    ..job(&base, "/slow", &sum)
                },
                DownloadError::TimedOut,
            ),
            // The production policy refuses the plain-HTTP test server outright.
            (
                Fetch {
                    policy: UrlPolicy::GitHub,
                    ..job(&base, "/asset", &sum)
                },
                DownloadError::PlainHttp,
            ),
        ];
        for (j, want) in cases {
            let (got, left) = run(&j).await;
            assert_eq!(got.as_ref().err(), Some(&want), "{}", j.url);
            assert!(left.is_empty(), "{}: left {left:?}", j.url);
            assert!(!want.to_string().is_empty());
        }
    }

    #[test]
    fn every_error_is_a_sentence() {
        for e in [
            DownloadError::PlainHttp,
            DownloadError::NotGitHub("evil.example".into()),
            DownloadError::TooManyRedirects,
            DownloadError::BadRedirect,
            DownloadError::Status(404),
            DownloadError::TooBig,
            DownloadError::WrongSum,
            DownloadError::TimedOut,
            DownloadError::Network("connection refused".into()),
            DownloadError::Write("disk full".into()),
        ] {
            let s = e.to_string();
            assert!(s.ends_with('.'), "{s}");
            assert!(s.chars().next().unwrap().is_uppercase(), "{s}");
        }
    }
}
