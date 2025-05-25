# Build stage
FROM python:3.12-slim as builder

# Set build arguments
ARG VERSION=0.1.0

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /build

# Copy package files
COPY pyproject.toml README.md ./
COPY arcan ./arcan

# Install build tools and create wheel
RUN pip install --no-cache-dir build && \
    python -m build --wheel

# Runtime stage
FROM python:3.12-slim

# Set environment variables
ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

# Create non-root user
RUN useradd -m -u 1000 arcan && \
    mkdir -p /app && \
    chown -R arcan:arcan /app

# Set working directory
WORKDIR /app

# Copy wheel from builder
COPY --from=builder /build/dist/*.whl /tmp/

# Install the package
RUN pip install --no-cache-dir /tmp/*.whl && \
    rm -rf /tmp/*.whl

# Switch to non-root user
USER arcan

# Set the entrypoint
ENTRYPOINT ["arcan"]
CMD ["--help"]

# Add labels
LABEL org.opencontainers.image.title="Arcan" \
      org.opencontainers.image.description="A powerful spellbook for modern development" \
      org.opencontainers.image.authors="broomva@gmail.com" \
      org.opencontainers.image.source="https://github.com/broomva/arcan" \
      org.opencontainers.image.version="${VERSION}" 