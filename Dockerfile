# syntax=docker/dockerfile:1
# One image: the Rust server + the built web client it serves.

# --- Rust: server binary + wasm client ---------------------------------------------------------
FROM rust:1-bookworm AS rust
WORKDIR /src
RUN rustup target add wasm32-unknown-unknown \
 && cargo install wasm-bindgen-cli --version 0.2.130 --locked
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY migrations migrations
COPY contract contract
COPY scripts/build-wasm.mjs scripts/build-wasm.mjs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p gs-server --bin gs-server \
 && cp target/release/gs-server /gs-server \
 && cargo build -p gs-client --target wasm32-unknown-unknown --profile wasm-release \
 && mkdir -p /wasm \
 && wasm-bindgen --target web --out-dir /wasm --out-name gs_client target/wasm32-unknown-unknown/wasm-release/gs_client.wasm

# --- Web: Vite build ---------------------------------------------------------------------------
FROM node:24-bookworm-slim AS web
WORKDIR /src
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json apps/web/package.json
COPY packages/engine/package.json packages/engine/package.json
RUN pnpm install --frozen-lockfile
COPY apps apps
COPY packages packages
COPY --from=rust /wasm packages/engine/src/wasm
RUN pnpm --filter web build

# --- Runtime -----------------------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=rust /gs-server /app/gs-server
COPY --from=web /src/apps/web/dist /app/web
ENV GS_HTTP_ADDR=0.0.0.0:8080 GS_UDP_PORT=40000 GS_WEB_DIR=/app/web
EXPOSE 8080/tcp 40000/udp
CMD ["/app/gs-server"]
