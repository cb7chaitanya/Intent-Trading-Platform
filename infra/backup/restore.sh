#!/bin/bash
set -euo pipefail

# ============================================================
# Restore PostgreSQL from S3 backup
#
# Usage:
#   ./restore.sh                                              # latest
#   ./restore.sh backup_intent_trading_20260401_120000.sql.gz  # specific
#   ./restore.sh --pitr 2026-04-01T15:30:00                   # point-in-time
#
# Options:
#   --skip-verify     Skip post-restore verification
#   --skip-migrations Skip running migrations after restore
#   --pitr TIMESTAMP  Restore to a point-in-time (requires WAL)
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

log() { echo "[$(date -Iseconds)] RESTORE: $*"; }

SKIP_VERIFY=0
SKIP_MIGRATIONS=0
PITR_TARGET=""
BACKUP_FILE=""

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-verify)     SKIP_VERIFY=1; shift ;;
        --skip-migrations) SKIP_MIGRATIONS=1; shift ;;
        --pitr)            PITR_TARGET="$2"; shift 2 ;;
        *)                 BACKUP_FILE="$1"; shift ;;
    esac
done

# ── Step 1: Find backup to restore ───────────────────────
if [ -z "${BACKUP_FILE}" ]; then
    log "Finding latest backup..."
    BACKUP_FILE=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/" \
        | awk '{print $4}' \
        | grep "^backup_" \
        | grep -v ".sha256" \
        | sort -r \
        | head -1)

    if [ -z "${BACKUP_FILE}" ]; then
        log "ERROR: No backups found in s3://${S3_BUCKET}/${S3_PREFIX}/"
        exit 1
    fi
fi

log "Restoring: ${BACKUP_FILE}"
RESTORE_PATH="/tmp/${BACKUP_FILE}"

# ── Step 2: Download ─────────────────────────────────────
aws s3 cp ${S3_ARGS} \
    "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}" \
    "${RESTORE_PATH}"

RESTORE_SIZE=$(du -h "${RESTORE_PATH}" | cut -f1)
log "Downloaded: ${RESTORE_SIZE}"

# ── Step 3: Verify checksum if available ──────────────────
CHECKSUM_EXISTS=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}.sha256" 2>/dev/null | wc -l)
if [ "${CHECKSUM_EXISTS}" -gt 0 ]; then
    aws s3 cp ${S3_ARGS} \
        "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}.sha256" \
        "/tmp/${BACKUP_FILE}.sha256"

    EXPECTED=$(awk '{print $1}' "/tmp/${BACKUP_FILE}.sha256")
    ACTUAL=$(sha256sum "${RESTORE_PATH}" | awk '{print $1}')

    if [ "${EXPECTED}" != "${ACTUAL}" ]; then
        log "ERROR: Checksum mismatch! Backup may be corrupted."
        log "  Expected: ${EXPECTED}"
        log "  Actual:   ${ACTUAL}"
        rm -f "${RESTORE_PATH}" "/tmp/${BACKUP_FILE}.sha256"
        exit 1
    fi
    log "Checksum verified"
fi

# ── Step 4: Pre-restore safety ────────────────────────────
# Count current rows for comparison
CURRENT_TABLES=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -t -c "
    SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public';
" 2>/dev/null | tr -d ' ' || echo "0")
log "Current database has ${CURRENT_TABLES} tables"

# Confirm with user if running interactively
if [ -t 0 ]; then
    echo ""
    echo "  WARNING: This will DROP and RECREATE the '${PGDATABASE}' database."
    echo "  All current data will be lost."
    echo ""
    read -p "  Continue? (yes/no) " CONFIRM
    if [ "${CONFIRM}" != "yes" ]; then
        log "Aborted by user"
        rm -f "${RESTORE_PATH}"
        exit 0
    fi
fi

# ── Step 5: Terminate connections ─────────────────────────
log "Terminating existing connections..."
psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres -c "
    SELECT pg_terminate_backend(pid)
    FROM pg_stat_activity
    WHERE datname = '${PGDATABASE}' AND pid <> pg_backend_pid();
" 2>/dev/null || true

# ── Step 6: Drop and recreate ─────────────────────────────
log "Recreating database..."
psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres -c "
    DROP DATABASE IF EXISTS ${PGDATABASE};
"
psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres -c "
    CREATE DATABASE ${PGDATABASE};
"

# ── Step 7: Restore ──────────────────────────────────────
log "Restoring data..."
RESTORE_START=$(date +%s)

gunzip -c "${RESTORE_PATH}" | psql \
    -h "${PGHOST}" \
    -p "${PGPORT}" \
    -U "${PGUSER}" \
    -d "${PGDATABASE}" \
    --quiet \
    2>/tmp/restore_stderr.log

RESTORE_DURATION=$(( $(date +%s) - RESTORE_START ))
log "SQL restore complete in ${RESTORE_DURATION}s"

# Clean up downloaded file
rm -f "${RESTORE_PATH}" "/tmp/${BACKUP_FILE}.sha256"

# ── Step 8: Apply WAL for PITR (if requested) ────────────
if [ -n "${PITR_TARGET}" ]; then
    log "Point-in-time recovery requested: ${PITR_TARGET}"
    log "NOTE: PITR requires PostgreSQL WAL archiving enabled and recovery.conf."
    log "      This script restores the base backup; WAL replay must be configured"
    log "      in postgresql.conf with recovery_target_time = '${PITR_TARGET}'"
fi

# ── Step 9: Post-restore verification ─────────────────────
if [ "${SKIP_VERIFY}" -eq 0 ]; then
    log "Verifying restore..."

    TABLE_COUNT=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -t -c "
        SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public';
    " | tr -d ' ')

    if [ -z "${TABLE_COUNT}" ] || [ "${TABLE_COUNT}" -lt 5 ]; then
        log "ERROR: Restore verification failed. Only ${TABLE_COUNT} tables found."
        exit 1
    fi

    # Check critical tables
    for table in users intents bids fills markets solvers balances; do
        EXISTS=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -t -c "
            SELECT COUNT(*) FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = '${table}';
        " | tr -d ' ')
        if [ "${EXISTS}" != "1" ]; then
            log "WARNING: Table '${table}' not found after restore"
        fi
    done

    USER_COUNT=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -t -c "SELECT COUNT(*) FROM users;" 2>/dev/null | tr -d ' ' || echo "?")
    log "Restore verified: ${TABLE_COUNT} tables, ${USER_COUNT} users"
fi

# ── Step 10: Run migrations ──────────────────────────────
if [ "${SKIP_MIGRATIONS}" -eq 0 ]; then
    log "NOTE: Run migrations to apply any schema changes newer than the backup:"
    log "  sqlx migrate run --source /app/migrations"
fi

log "Restore complete"
