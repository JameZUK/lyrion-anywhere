# Release artefacts: static gateway binaries for each Linux platform, and the browser player.
# Used by tools/build-release.sh:
#   docker build -f tools/release/Dockerfile --target out --output type=local,dest=dist/build .

FROM rust:1.98-alpine3.22 AS build
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

# Browser player: the core compiled to WebAssembly, plus the page.
RUN cargo build --locked --release -p anywhere-wasm --target wasm32-unknown-unknown \
 && wasm-bindgen --target web --no-typescript --out-dir /out/web/pkg \
      target/wasm32-unknown-unknown/release/anywhere_wasm.wasm \
 && cp web/spike/*.html web/spike/*.js web/spike/*.css /out/web/

FROM scratch AS out
COPY --from=build /out /
