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
| `web/app/` | Web app: SvelteKit + the core as WebAssembly (`tools/build-web.sh`) |
| `plugin/` | LMS plugin: settings, accounts and gateway supervision |
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

Web app (needs `wasm-pack` and Node 22+):

```sh
tools/build-web.sh                       # WebAssembly core + SvelteKit build into web/app/build
cd web/app && npm run check && npm test  # strict type check; Vitest + Node tests
npm run dev                              # dev server; proxies /v1 to a gateway on 127.0.0.1:9443
```

Put local, environment-specific settings in `dev/local.env` (git-ignored). Never commit real hostnames or addresses.

## Licence

GPL-2.0-or-later. See [`LICENSE`](LICENSE). Binaries that include Apache-2.0-licensed dependencies are distributed under the terms of GPLv3.

Squeezelite and Material Skin served as behavioural references only. No code was copied from either.
