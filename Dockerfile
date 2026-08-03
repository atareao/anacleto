# =============================================================================
# Stage 1 — Builder
# =============================================================================
FROM rust:1.85-slim-bookworm AS builder

# Build-time dependencies (SQLite bindings need pkg-config + libsqlite3-dev)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 1. Copy manifests first — layer caching means this only re-runs when deps change
COPY Cargo.toml Cargo.lock ./

# 2. Create a dummy main.rs so `cargo build` can fetch & compile dependencies
#    without needing the real source.  We remove it before the real build.
RUN mkdir src && echo "fn main() {}" > src/main.rs

# 3. Build dependencies (this layer is cached as long as Cargo.lock is unchanged)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    rm -rf src

# 4. Copy the real source code
COPY src/ src/

# 5. Build the real binary (only recompiles changed source files)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --offline && \
    cp target/release/anacleto /app/anacleto

# =============================================================================
# Stage 2 — Runtime
# =============================================================================
FROM debian:bookworm-slim AS runtime

# Runtime dependencies: TLS certificates (for reqwest) + SQLite shared library
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libsqlite3-1 \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user for security
RUN groupadd --gid 1000 anacleto && \
    useradd --uid 1000 --gid anacleto --shell /bin/false --create-home anacleto

WORKDIR /home/anacleto

COPY --from=builder --chown=anacleto:anacleto /app/anacleto /usr/local/bin/anacleto

USER anacleto

# Metadata — https://github.com/opencontainers/image-spec/blob/main/annotations.md
LABEL org.opencontainers.image.title="anacleto" \
      org.opencontainers.image.description="Agent orchestration engine in Rust — agents, subagents, skills, and MCPs" \
      org.opencontainers.image.version="0.1.0" \
      org.opencontainers.image.source="https://github.com/atareao/anacleto"

EXPOSE 8080

CMD ["anacleto"]