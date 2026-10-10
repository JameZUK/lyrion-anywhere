# Lyrion Anywhere gateway, with the web app.
#
#   docker build -t lyrion-anywhere .
#   docker run --network host -v anywhere-data:/data lyrion-anywhere \
#       serve --lms lms.example.lan --allowed-origin https://music.example.com
#
# See docs/DEPLOY.md. The image runs as a non-root user with no shell.

# --- gateway: a static musl binary -------------------------------------------------------
FROM rust:1.98-alpine3.22 AS gateway
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p anywhere-gateway \
 && install -D target/release/anywhere-gateway /out/anywhere-gateway \
 && install -d -m 0700 /out/data

# --- web app: the core compiled to WebAssembly, then the SvelteKit app ---------------------
FROM rust:1.98-alpine3.22 AS wasm
RUN apk add --no-cache musl-dev \
 && rustup target add wasm32-unknown-unknown \
 && cargo install --locked wasm-bindgen-cli --version 0.2.129
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p anywhere-wasm --target wasm32-unknown-unknown \
 && wasm-bindgen --target web --no-typescript --out-dir /out/pkg \
      target/wasm32-unknown-unknown/release/anywhere_wasm.wasm

FROM node:24.11.1-alpine3.22 AS web
WORKDIR /src/web/app
COPY web/app/package.json web/app/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/app/ ./
COPY --from=wasm /out/pkg static/player/pkg
RUN npm run build

# --- runtime ------------------------------------------------------------------------------
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=gateway /out/anywhere-gateway /usr/local/bin/anywhere-gateway
COPY --from=gateway --chown=nonroot:nonroot /out/data /data
COPY --from=web /src/web/app/build /web
VOLUME /data
EXPOSE 9443
USER nonroot
ENTRYPOINT ["/usr/local/bin/anywhere-gateway"]
CMD ["serve", "--data-dir", "/data", "--static-dir", "/web"]
