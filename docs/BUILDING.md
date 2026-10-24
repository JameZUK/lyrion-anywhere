# Building from source

How to build and test every part of Lyrion Anywhere from this repository, run the demo, and contribute.

The repository holds the Rust workspace (`core`, `gateway`, `wasm`, `mobile`, `cli`), the web app (`web/app`), the Perl plugin (`plugin`) and the Android app (`apps/android`). The commands below are the ones CI runs ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)), from the repository root unless a directory is named.

## Rust workspace

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                       # core, gateway (unit and end-to-end against a fake LMS), mobile
cargo test -p anywhere-core --all-features
```

- The gateway's live-encode tests need `ffmpeg`; CI sets `ANYWHERE_REQUIRE_FFMPEG=1` so they never skip.
- The vendored Opus decoder is its own workspace: `cargo test --locked --manifest-path third_party/opus-decoder/Cargo.toml`.
- The core also builds for WebAssembly on its own: `cargo build -p anywhere-core --no-default-features --target wasm32-unknown-unknown`.

## Web app

Needs `wasm-pack` and Node 22 or later.

```sh
wasm-pack test --node wasm                   # the WebAssembly bindings
tools/build-web.sh                           # the core compiled to WebAssembly, then the SvelteKit app (web/app/build)
cd web/app && npm run check && npm test      # strict type check; Vitest and Node tests
```

The gateway serves the built app with `--static-dir web/app/build`.

## LMS plugin

```sh
cargo build -p anywhere-gateway              # the plugin's tests run the real gateway
cd plugin && PERL5LIB=t/stubs:. prove -r t/  # Perl, against LMS stubs
cd plugin && node --test t/js/*.test.mjs     # the settings page
```

A plugin release (the zip with static gateways for x86_64, aarch64 and armhf Linux, and `repo.xml`) is built with `tools/build-release.sh` (needs Docker) and checked with `tools/test-release.sh dist`. The Docker image is built from the root `Dockerfile` and tested behind nginx with `tools/test-deploy.sh`. The release process is in [`RELEASING.md`](RELEASING.md).

## Android app

```sh
cd apps/android && ./gradlew assembleDebug testDebugUnitTest lint
```

The SDK, NDK and `cargo-ndk` set-up, release builds and build properties are in [`apps/android/README.md`](../apps/android/README.md).

## Documentation

```sh
python3 tools/check-docs.py --self-test   # the checker catches a missing file, anchor and malformed SVG
python3 tools/check-docs.py               # every relative link, anchor and image in README.md, SPEC.md, CHANGELOG.md and docs/ resolves
```

The README's *How it works* diagram is a hand-drawn SVG, `docs/images/how-it-works.svg`, with a `prefers-color-scheme` media query for dark mode; edit it as text and check both schemes in a browser.

## Try it without an LMS

`tools/demo/run.sh` runs the real gateway and web app against an invented library (see [`tools/demo`](../tools/demo)); sign in at `http://127.0.0.1:19443` as `demo` / `demo-password-123`.

## Live tests

Opt-in tests against a real LMS use a dedicated, muted test player and read their configuration from the git-ignored `dev/local.env`; without it the Rust ones say so and pass without touching anything, which is how CI runs them. For example, the live check that a phone rejoins a sync group in step: `ANYWHERE_LIVE=1 cargo test -p anywhere-mobile --test live_sync -- --nocapture`. The testing strategy is in [`ARCHITECTURE.md`](ARCHITECTURE.md#11-testing-strategy).

The web app over plain `http://` to a trusted network (SPEC D28) has an opt-in browser check, never run in CI: start a gateway bound to a private address with `--trusted-network` and `--static-dir web/app/build` in front of a throwaway LMS, then run `web/app/tests/e2e/insecure-context.mjs` with `ADDRESS`, `PORT`, `USERNAME`, `PASSWORD`, `BROWSERS` and `PLAYWRIGHT_DIRS` set (the header of the file says how). It signs in, checks the unencrypted-connection notice, plays a track in the media-element mode for ten seconds and downloads an album, in Chromium and Firefox.

Two more opt-in browser checks start a gateway on loopback themselves and need only a built gateway, a built web app and Playwright: `web/app/tests/e2e/pwa-update.mjs` (how the installed app survives an update, with a previous release's web build as `OLD_WEB`) and `web/app/tests/e2e/history.mjs` (Back, Forward and a route typed into the address bar, against the demo LMS of `tools/demo`, so it also needs python3 with Pillow). Each file's header says which variables to set. Run them after touching the service worker or `lib/nav.ts`: SvelteKit's router does not run under jsdom, so the unit tests cannot see what they check.

## Contributing

- Read [`CLAUDE.md`](../CLAUDE.md) first.
- Every change ships with tests, security first: negative tests for every reachable surface, robustness tests for every parser, a regression test for every security fix.
- No personal or environment details go in the repository: use placeholders such as `music.example.com`, `10.0.0.10` and `/srv/music`.
- British English in docs and UI.

## Acknowledgements

Lyrion Anywhere is built on open-source building blocks, among them Symphonia (decoding), tokio and axum (the gateway), rustls, SQLite, UniFFI, SvelteKit, Jetpack Compose and Media3. A patched copy of the `opus-decoder` crate is kept in [`third_party/`](../third_party) until a fixed release.

[squeezelite](https://github.com/ralph-irving/squeezelite) and [Material Skin](https://github.com/CDrummond/lms-material) were references for behaviour only: this is a clean-room implementation, and no code was copied from either. The logo is an original design in the spirit of the Lyrion Music Server logo: a waveform turning into broadcast waves, for your music reaching you wherever you are.
