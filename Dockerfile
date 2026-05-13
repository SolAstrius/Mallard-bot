# syntax=docker/dockerfile:1.7
# ---- builder ------------------------------------------------------------
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /build

# Pre-cache dependency builds: copy manifests + the font asset that
# `include_bytes!` references, then build a stub, then drop it in.
COPY Cargo.toml Cargo.lock ./
COPY assets ./assets
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/deps/mallard_bot* target/release/deps/mallard-bot*

COPY . .
RUN cargo build --release --locked

# ---- runtime ------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        ffmpeg \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /build/target/release/mallard-bot /usr/local/bin/mallard-bot
COPY content ./content
COPY voices ./voices

ENV RUST_LOG=info
CMD ["mallard-bot"]
