# Multi-stage build for arcan agent runtime daemon
# Clones all sibling workspace dependencies and builds the binary
# Build bust: 2026-03-25e
# NOTE: WORKDIR must be /arcan — nous-middleware has a hardcoded path dep on /arcan/crates/arcan-core

FROM rust:1.85-bookworm AS builder

# Use /arcan so nous-middleware path dep (/arcan/crates/arcan-core) resolves correctly
WORKDIR /arcan

# Clone sibling dependencies (matches CI checkout pattern)
RUN git clone --depth 1 https://github.com/broomva/aiOS.git ../aiOS && \
    git clone --depth 1 https://github.com/broomva/lago.git ../lago && \
    git clone --depth 1 https://github.com/broomva/praxis.git ../praxis && \
    git clone --depth 1 https://github.com/broomva/autonomic.git ../autonomic && \
    git clone --depth 1 https://github.com/broomva/vigil.git ../vigil && \
    git clone --depth 1 https://github.com/broomva/haima.git ../haima && \
    git clone --depth 1 https://github.com/broomva/nous.git ../nous && \
    git clone --depth 1 https://github.com/broomva/anima.git ../anima

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build release binary
RUN cargo build --release -p arcan

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash arcan

COPY --from=builder /arcan/target/release/arcan /usr/local/bin/arcan

USER arcan
WORKDIR /home/arcan

# ARCAN_JWT_SECRET — shared HMAC secret for JWT auth (same as broomva.tech AUTH_SECRET).
# When set, all API routes except /health and /healthz require a valid Bearer token.
# When unset, auth is disabled (local dev mode).
ENV RUST_LOG=info
EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

ENTRYPOINT ["arcan", "serve"]
