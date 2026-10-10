#!/bin/sh
# Build the web app: the core compiled to WebAssembly (web/app/static/player/pkg), then the
# SvelteKit app (web/app/build). Serve with: anywhere-gateway serve ... --static-dir web/app/build
set -eu
cd "$(dirname "$0")/.."
wasm-pack build wasm --release --target web --out-dir ../web/app/static/player/pkg --no-typescript
rm -f web/app/static/player/pkg/.gitignore web/app/static/player/pkg/package.json
cd web/app
npm ci --no-audit --no-fund
npm run build
