# ============================================================
# Multi-stage Rust build for all platform binaries
# ============================================================

# Stage 1: Build
FROM rust:1.86-bookworm AS builder

WORKDIR /app

# Cache dependencies by building with empty src first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    mkdir -p src/bin/load_test && \
    echo "fn main() {}" > src/bin/solver_bot.rs && \
    echo "fn main() {}" > src/bin/api_gateway.rs && \
    echo "fn main() {}" > src/bin/load_test/main.rs && \
    echo "pub mod gateway { pub mod router; pub mod auth; pub mod proxy; pub mod rate_limit; pub mod metrics_middleware; }" > src/lib.rs && \
    cargo build --release 2>/dev/null || true

# Copy actual source and migrations
COPY src/ src/
COPY migrations/ migrations/

# Build all binaries
RUN cargo build --release --bin intent-trading && \
    cargo build --release --bin api-gateway && \
    cargo build --release --bin solver-bot && \
    cargo build --release --bin load-test

# ============================================================
# Stage 2: Runtime images
# ============================================================

# --- intent-trading ---
FROM debian:bookworm-slim AS intent-trading
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/intent-trading /usr/local/bin/
COPY --from=builder /app/migrations /app/migrations
WORKDIR /app
ENV DATABASE_URL=postgres://postgres:postgres@postgres:5432/intent_trading \
    REDIS_URL=redis://redis:6379 \
    SERVER_ADDR=0.0.0.0:3000 \
    LOG_LEVEL=info \
    ENVIRONMENT=production
EXPOSE 3000
CMD ["intent-trading"]

# --- api-gateway ---
FROM debian:bookworm-slim AS api-gateway
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/api-gateway /usr/local/bin/
WORKDIR /app
ENV REDIS_URL=redis://redis:6379 \
    GATEWAY_ADDR=0.0.0.0:4000 \
    UPSTREAM_URL=http://intent-trading:3000 \
    LOG_LEVEL=info \
    ENVIRONMENT=production
EXPOSE 4000
CMD ["api-gateway"]

# --- solver-bot ---
FROM debian:bookworm-slim AS solver-bot
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/solver-bot /usr/local/bin/
WORKDIR /app
CMD ["solver-bot"]

# --- load-test ---
FROM debian:bookworm-slim AS load-test
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/load-test /usr/local/bin/
WORKDIR /app
CMD ["load-test"]
