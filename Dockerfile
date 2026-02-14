# ============================================================
# Stage 1: Build
# ============================================================
FROM rust:1.80-bookworm AS builder

WORKDIR /app

# Cache dependencies: copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/arcan-core/Cargo.toml crates/arcan-core/Cargo.toml
COPY crates/arcan-harness/Cargo.toml crates/arcan-harness/Cargo.toml
COPY crates/arcan-store/Cargo.toml crates/arcan-store/Cargo.toml
COPY crates/arcan-provider/Cargo.toml crates/arcan-provider/Cargo.toml
COPY crates/arcand/Cargo.toml crates/arcand/Cargo.toml
COPY crates/arcan-lago/Cargo.toml crates/arcan-lago/Cargo.toml
COPY crates/arcan/Cargo.toml crates/arcan/Cargo.toml

# Create dummy source files to build dependencies
RUN mkdir -p crates/arcan-core/src && echo "" > crates/arcan-core/src/lib.rs && \
    mkdir -p crates/arcan-harness/src && echo "" > crates/arcan-harness/src/lib.rs && \
    mkdir -p crates/arcan-store/src && echo "" > crates/arcan-store/src/lib.rs && \
    mkdir -p crates/arcan-provider/src && echo "" > crates/arcan-provider/src/lib.rs && \
    mkdir -p crates/arcand/src && echo "" > crates/arcand/src/lib.rs && \
    mkdir -p crates/arcan-lago/src && echo "" > crates/arcan-lago/src/lib.rs && \
    mkdir -p crates/arcan/src && echo "fn main() {}" > crates/arcan/src/main.rs

# Build dependencies (cached layer)
RUN cargo build --release --workspace 2>/dev/null || true

# Copy real source code
COPY crates/ crates/

# Touch source files to invalidate the dummy builds
RUN find crates -name "*.rs" -exec touch {} +

# Build the production binary
RUN cargo build --release -p arcan

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash arcan

COPY --from=builder /app/target/release/arcan /usr/local/bin/arcan

USER arcan
WORKDIR /home/arcan

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

ENTRYPOINT ["arcan"]
