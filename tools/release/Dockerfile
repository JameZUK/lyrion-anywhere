# Release artefacts: static gateway binaries for each Linux platform, and the web app.
# Used by tools/build-release.sh:
#   docker build -f tools/release/Dockerfile --target out --output type=local,dest=dist/build .
#
# Base images are pinned by digest (tag kept for reading); Dependabot proposes updates.

FROM rust:1.98-alpine3.22@sha256:a1796ca6fa216d6727b5f61c69e4c665b120b1a4dcb969639e2f25f1ed309456 AS build
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

FROM node:26.3.0-alpine3.22@sha256:c7932b9e5e337b0e733d6e16abc1b0e104759e8b05e59ed56586cce967d26dfe AS web
WORKDIR /src/web/app
COPY web/app/package.json web/app/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/app/ ./
# The version the app shows is the plugin's (vite.config.ts reads ../../plugin/.../install.xml).
COPY plugin/Plugins/LyrionAnywhere/install.xml /src/plugin/Plugins/LyrionAnywhere/install.xml
COPY --from=build /out/pkg static/player/pkg
RUN npm run build

FROM scratch AS out
COPY --from=build /out/Bin /Bin
COPY --from=web /src/web/app/build /web
