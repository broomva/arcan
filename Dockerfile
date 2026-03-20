# Multi-stage build for arcan agent runtime daemon
# Clones all sibling workspace dependencies and builds the binary

FROM rust:1-bookworm AS builder

WORKDIR /build

# Clone sibling dependencies (matches CI checkout pattern)
RUN git clone --depth 1 https://github.com/broomva/aiOS.git ../aiOS && \
    git clone --depth 1 https://github.com/broomva/lago.git ../lago && \
    git clone --depth 1 https://github.com/broomva/praxis.git ../praxis && \
    git clone --depth 1 https://github.com/broomva/autonomic.git ../autonomic && \
    git clone --depth 1 https://github.com/broomva/vigil.git ../vigil && \
    git clone --depth 1 https://github.com/broomva/haima.git ../haima

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

COPY --from=builder /build/target/release/arcan /usr/local/bin/arcan

USER arcan
WORKDIR /home/arcan

ENV RUST_LOG=info
EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

ENTRYPOINT ["arcan"]
