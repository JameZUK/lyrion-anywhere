#!/bin/sh
# Build the browser player spike: compile the core to WebAssembly into web/spike/pkg.
# Serve with: anywhere-gateway serve ... --static-dir web/spike
set -eu
cd "$(dirname "$0")/.."
wasm-pack build wasm --release --target web --out-dir ../web/spike/pkg --no-typescript
