# Lyrion Anywhere gateway, with the browser player page.
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

# --- browser player: the core compiled to WebAssembly -------------------------------------
FROM rust:1.98-alpine3.22 AS web
RUN apk add --no-cache musl-dev \
 && rustup target add wasm32-unknown-unknown \
 && cargo install --locked wasm-bindgen-cli --version 0.2.129
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p anywhere-wasm --target wasm32-unknown-unknown \
 && wasm-bindgen --target web --no-typescript --out-dir web/spike/pkg \
      target/wasm32-unknown-unknown/release/anywhere_wasm.wasm \
 && mkdir -p /out/web \
 && cp web/spike/*.html web/spike/*.js web/spike/*.css /out/web/ \
 && cp -r web/spike/pkg /out/web/pkg

# --- runtime ------------------------------------------------------------------------------
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=gateway /out/anywhere-gateway /usr/local/bin/anywhere-gateway
COPY --from=gateway --chown=nonroot:nonroot /out/data /data
COPY --from=web /out/web /web
VOLUME /data
EXPOSE 9443
USER nonroot
ENTRYPOINT ["/usr/local/bin/anywhere-gateway"]
CMD ["serve", "--data-dir", "/data", "--static-dir", "/web"]
