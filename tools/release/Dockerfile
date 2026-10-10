# Release artefacts: static gateway binaries for each Linux platform, and the web app.
# Used by tools/build-release.sh:
#   docker build -f tools/release/Dockerfile --target out --output type=local,dest=dist/build .
#
# Base images are pinned by digest (tag kept for reading); Dependabot proposes updates.

FROM rust:1.99-alpine3.22@sha256:c3a5ad77ff2e5ec99fffaf62cb518e61a9dcbd9e1d7526d281a380db0be3bb3b AS build
RUN apk add --no-cache musl-dev zig=0.14.1-r0 \
 && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
      armv7-unknown-linux-musleabihf wasm32-unknown-unknown \
 && cargo install --locked cargo-zigbuild --version 0.23.4 \
 && cargo install --locked wasm-bindgen-cli --version 0.2.129
WORKDIR /src
COPY . .

# Gateway: one fully static binary per platform (named as the plugin's Bin/<platform>/).
RUN set -e; \
    for pair in x86_64-unknown-linux-musl:x86_64-linux \
                aarch64-unknown-linux-musl:aarch64-linux \
                armv7-unknown-linux-musleabihf:armhf-linux; do \
      target=${pair%%:*}; plat=${pair##*:}; \
      cargo zigbuild --locked --release -p anywhere-gateway --target "$target"; \
      install -D -m 0755 "target/$target/release/anywhere-gateway" "/out/Bin/$plat/anywhere-gateway"; \
    done

# Web app: the core compiled to WebAssembly (the SvelteKit build runs in the next stage).
RUN cargo build --locked --release -p anywhere-wasm --target wasm32-unknown-unknown \
 && wasm-bindgen --target web --no-typescript --out-dir /out/pkg \
      target/wasm32-unknown-unknown/release/anywhere_wasm.wasm

FROM node:24.11.1-alpine3.22@sha256:2867d550cf9d8bb50059a0fff528741f11a84d985c732e60e19e8e75c7239c43 AS web
WORKDIR /src/web/app
COPY web/app/package.json web/app/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/app/ ./
COPY --from=build /out/pkg static/player/pkg
RUN npm run build

FROM scratch AS out
COPY --from=build /out/Bin /Bin
COPY --from=web /src/web/app/build /web
