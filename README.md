# Lyrion Anywhere

Secure internet access to a [Lyrion Music Server](https://lyrion.org) through one authenticated HTTPS/WebSocket endpoint, from a web app and later from Android and iOS apps. LMS itself is never exposed. Remote devices act as real LMS players (gapless, sync), and offline downloads are supported.

**Status:** early development (Phase 1: gateway and LMS plugin). See [`SPEC.md`](SPEC.md) for the design and [`docs/phase0-report.md`](docs/phase0-report.md) for progress.

## Layout

| Path | What |
|---|---|
| `core/` | Rust player core: Slimproto, buffering, decoding (no I/O; native + WASM) |
| `cli/` | Native test player for the core |
| `gateway/` | Rust gateway: authenticated Slimproto tunnel and stream relay |
| `wasm/` | WebAssembly bindings for the core (browser player) |
| `web/spike/` | Phase 0 browser player page (`tools/build-web-spike.sh`) |
| `plugin/` | LMS plugin: settings, accounts and gateway supervision |
| `web/` | SvelteKit web app (planned) |
| `deploy/` | Example reverse-proxy config (nginx) |
| `docs/` | Documentation and reports; deployment in [`docs/DEPLOY.md`](docs/DEPLOY.md) |

## Development

```sh
cargo test -p anywhere-core              # unit + robustness tests
cargo run -p anywhere-cli -- --server lms.example.lan --null -v   # silent test player
```

Plugin tests (Perl stubs for LMS, plus the real gateway once built):

```sh
cargo build -p anywhere-gateway
cd plugin && PERL5LIB=t/stubs:. prove -r t/ && node --test t/js/*.test.mjs
```

Put local, environment-specific settings in `dev/local.env` (git-ignored). Never commit real hostnames or addresses.

## Licence

GPL-2.0-or-later. See [`LICENSE`](LICENSE). Binaries that include Apache-2.0-licensed dependencies are distributed under the terms of GPLv3.

Squeezelite and Material Skin served as behavioural references only. No code was copied from either.
