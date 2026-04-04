#!/bin/bash
set -euo pipefail

# ============================================================
# Restore Postgres from S3 backup
#
# Usage:
#   ./restore.sh                        # restore latest backup
#   ./restore.sh backup_intent_trading_20260401_120000.sql.gz  # restore specific
# ============================================================

: "${PGHOST:=postgres}"
: "${PGPORT:=5432}"
: "${PGUSER:=postgres}"
: "${PGPASSWORD:=postgres}"
: "${PGDATABASE:=intent_trading}"
: "${S3_BUCKET:=intentx-backups}"
: "${S3_ENDPOINT:=}"
: "${S3_PREFIX:=daily}"

export PGPASSWORD

S3_ARGS=""
if [ -n "${S3_ENDPOINT}" ]; then
    S3_ARGS="--endpoint-url ${S3_ENDPOINT}"
fi

# Determine which backup to restore
if [ -n "${1:-}" ]; then
    BACKUP_FILE="$1"
else
    echo "Finding latest backup..."
    BACKUP_FILE=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/" \
        | awk '{print $4}' \
        | grep "^backup_" \
        | sort -r \
        | head -1)

    if [ -z "${BACKUP_FILE}" ]; then
        echo "ERROR: No backups found in s3://${S3_BUCKET}/${S3_PREFIX}/"
        exit 1
    fi
fi

echo "[$(date -Iseconds)] Restoring: ${BACKUP_FILE}"
RESTORE_PATH="/tmp/${BACKUP_FILE}"

# Download
aws s3 cp ${S3_ARGS} \
    "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}" \
    "${RESTORE_PATH}"

echo "[$(date -Iseconds)] Downloaded, size: $(du -h "${RESTORE_PATH}" | cut -f1)"

# Terminate existing connections
echo "[$(date -Iseconds)] Terminating existing connections..."
psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres -c "
    SELECT pg_terminate_backend(pid)
    FROM pg_stat_activity
    WHERE datname = '${PGDATABASE}' AND pid <> pg_backend_pid();
" 2>/dev/null || true

# Drop and recreate database
echo "[$(date -Iseconds)] Recreating database..."
psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres -c "
    DROP DATABASE IF EXISTS ${PGDATABASE};
    CREATE DATABASE ${PGDATABASE};
"

# Restore
echo "[$(date -Iseconds)] Restoring data..."
gunzip -c "${RESTORE_PATH}" | psql \
    -h "${PGHOST}" \
    -p "${PGPORT}" \
    -U "${PGUSER}" \
    -d "${PGDATABASE}" \
    --quiet \
    2>/tmp/restore_stderr.log

# Clean up
rm -f "${RESTORE_PATH}"

# Verify
TABLE_COUNT=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -t -c "
    SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public';
")

echo "[$(date -Iseconds)] Restore complete. Tables: ${TABLE_COUNT}"
echo ""
echo "IMPORTANT: Run migrations after restore to apply any newer schema changes:"
echo "  sqlx migrate run --database-url postgres://${PGUSER}:***@${PGHOST}:${PGPORT}/${PGDATABASE}"
