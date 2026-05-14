# syntax=docker/dockerfile:1.7
# ---- builder ------------------------------------------------------------
FROM rust:1.90-slim-bookworm AS builder

# symbolica → rug → gmp-mpfr-sys needs m4 + system math libs at build time.
# Headers + lib archives go into the final binary; runtime image doesn't
# need these.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        m4 \
        libgmp-dev \
        libmpfr-dev \
        libmpc-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Pre-cache dependency builds: copy manifests + the font asset that
# `include_bytes!` references, then build a stub, then drop it in.
COPY Cargo.toml Cargo.lock ./
COPY assets ./assets
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release --locked \
    && cargo clean --release -p mallard-bot \
    && rm -rf src

COPY . .
RUN cargo build --release --locked

# ---- runtime ------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

ARG TYPST_VERSION=0.14.2

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        ffmpeg \
        xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL -o /tmp/typst.tar.xz \
        "https://github.com/typst/typst/releases/download/v${TYPST_VERSION}/typst-x86_64-unknown-linux-musl.tar.xz" \
    && tar -xJf /tmp/typst.tar.xz -C /tmp \
    && mv /tmp/typst-x86_64-unknown-linux-musl/typst /usr/local/bin/typst \
    && rm -rf /tmp/typst.tar.xz /tmp/typst-x86_64-unknown-linux-musl

WORKDIR /app
COPY --from=builder /build/target/release/mallard-bot /usr/local/bin/mallard-bot
COPY content ./content
# Baked voice library — initContainer seeds these into the PVC-mounted
# /app/voices on first start (cp -rn, so user-added clips win on conflict).
COPY voices ./default-voices

ENV RUST_LOG=info
ENV TYPST_PACKAGE_CACHE_PATH=/app/data/typst-cache
CMD ["mallard-bot"]
