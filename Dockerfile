# Lyrion Anywhere gateway, with the web app.
#
#   docker build -t lyrion-anywhere .
#   docker run --network host -v anywhere-data:/data lyrion-anywhere \
#       serve --lms lms.example.lan --allowed-origin https://music.example.com
#
# See docs/DEPLOY.md. The image runs as a non-root user with no shell.
#
# Base images are pinned by digest (tag kept for reading); Dependabot proposes updates.

# --- gateway: a static musl binary -------------------------------------------------------
FROM rust:1.98-alpine3.22@sha256:a1796ca6fa216d6727b5f61c69e4c665b120b1a4dcb969639e2f25f1ed309456 AS gateway
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p anywhere-gateway \
 && install -D target/release/anywhere-gateway /out/anywhere-gateway \
 && install -d -m 0700 /out/data

# --- web app: the core compiled to WebAssembly, then the SvelteKit app ---------------------
FROM rust:1.98-alpine3.22@sha256:a1796ca6fa216d6727b5f61c69e4c665b120b1a4dcb969639e2f25f1ed309456 AS wasm
RUN apk add --no-cache musl-dev \
 && rustup target add wasm32-unknown-unknown \
 && cargo install --locked wasm-bindgen-cli --version 0.2.129
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p anywhere-wasm --target wasm32-unknown-unknown \
 && wasm-bindgen --target web --no-typescript --out-dir /out/pkg \
      target/wasm32-unknown-unknown/release/anywhere_wasm.wasm

FROM node:24.11.1-alpine3.22@sha256:2867d550cf9d8bb50059a0fff528741f11a84d985c732e60e19e8e75c7239c43 AS web
WORKDIR /src/web/app
COPY web/app/package.json web/app/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/app/ ./
COPY --from=wasm /out/pkg static/player/pkg
RUN npm run build

# --- runtime ------------------------------------------------------------------------------
FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
COPY --from=gateway /out/anywhere-gateway /usr/local/bin/anywhere-gateway
COPY --from=gateway --chown=nonroot:nonroot /out/data /data
COPY --from=web /src/web/app/build /web
VOLUME /data
EXPOSE 9443
USER nonroot
ENTRYPOINT ["/usr/local/bin/anywhere-gateway"]
CMD ["serve", "--data-dir", "/data", "--static-dir", "/web"]
