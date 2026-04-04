#!/bin/bash
set -euo pipefail

# ============================================================
# End-to-end test runner
#
# Starts services, waits for ready, runs tests, stops services.
# ============================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

echo "=== Starting test infrastructure ==="
docker compose up -d postgres redis

echo "=== Waiting for Postgres..."
for i in $(seq 1 30); do
    if docker compose exec -T postgres pg_isready -U postgres 2>/dev/null; then
        break
    fi
    sleep 1
done

echo "=== Waiting for Redis..."
for i in $(seq 1 30); do
    if docker compose exec -T redis redis-cli ping 2>/dev/null | grep -q PONG; then
        break
    fi
    sleep 1
done

echo "=== Starting platform..."
docker compose up -d intent-trading

echo "=== Waiting for platform health..."
for i in $(seq 1 60); do
    if curl -sf http://localhost:3000/health/live >/dev/null 2>&1; then
        echo "Platform ready"
        break
    fi
    if [ "$i" -eq 60 ]; then
        echo "ERROR: Platform did not become ready"
        docker compose logs intent-trading --tail 50
        exit 1
    fi
    sleep 1
done

echo "=== Running E2E tests ==="
E2E_BASE_URL=http://localhost:3000 \
    cargo test --test e2e_test --features e2e -- --test-threads=1 2>&1

EXIT_CODE=$?

echo "=== Stopping services ==="
docker compose down

exit $EXIT_CODE
