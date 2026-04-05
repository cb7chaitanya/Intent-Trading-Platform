#!/bin/bash
set -euo pipefail

# ============================================================
# Backup verification
#
# Downloads the latest backup, checks the checksum, and does a
# test restore into a throwaway database to confirm the dump is
# actually restorable. Drops the test database when done.
# ============================================================

: "${PGHOST:=postgres}"
: "${PGPORT:=5432}"
: "${PGUSER:=postgres}"
: "${PGPASSWORD:=postgres}"
: "${PGDATABASE:=intent_trading}"
: "${S3_BUCKET:=intentx-backups}"
: "${S3_ENDPOINT:=}"
: "${S3_PREFIX:=daily}"
: "${VERIFY_DB:=_backup_verify}"
: "${PUSHGATEWAY_URL:=}"

export PGPASSWORD

S3_ARGS=""
if [ -n "${S3_ENDPOINT}" ]; then
    S3_ARGS="--endpoint-url ${S3_ENDPOINT}"
fi

log() { echo "[$(date -Iseconds)] VERIFY: $*"; }

push_verify_metric() {
    local ok="$1"
    [ -z "${PUSHGATEWAY_URL}" ] && return 0
    cat <<METRICS | curl -sf --max-time 5 --data-binary @- "${PUSHGATEWAY_URL}/metrics/job/pg_backup_verify/instance/${PGHOST}" 2>/dev/null || true
# HELP backup_verify_success 1 if last verification passed, 0 if failed
# TYPE backup_verify_success gauge
backup_verify_success ${ok}
# HELP backup_verify_timestamp Unix timestamp of last verification
# TYPE backup_verify_timestamp gauge
backup_verify_timestamp $(date +%s)
METRICS
}

cleanup() {
    # Always try to drop the test database
    psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres \
        -c "DROP DATABASE IF EXISTS ${VERIFY_DB};" 2>/dev/null || true
    rm -f /tmp/verify_backup.sql.gz /tmp/verify_backup.sql.gz.sha256
}

trap cleanup EXIT

# ── Step 1: Find latest backup ────────────────────────────
log "Finding latest backup..."

BACKUP_FILE=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/" \
    | awk '{print $4}' \
    | grep "^backup_" \
    | grep -v ".sha256" \
    | sort -r \
    | head -1)

if [ -z "${BACKUP_FILE}" ]; then
    log "ERROR: No backups found in s3://${S3_BUCKET}/${S3_PREFIX}/"
    push_verify_metric 0
    exit 1
fi

log "Latest backup: ${BACKUP_FILE}"

# ── Step 2: Download backup + checksum ────────────────────
aws s3 cp ${S3_ARGS} \
    "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}" \
    /tmp/verify_backup.sql.gz

DOWNLOADED_SIZE=$(stat -f%z /tmp/verify_backup.sql.gz 2>/dev/null \
    || stat -c%s /tmp/verify_backup.sql.gz 2>/dev/null || echo 0)
log "Downloaded: ${DOWNLOADED_SIZE} bytes"

if [ "${DOWNLOADED_SIZE}" -lt 1024 ]; then
    log "ERROR: Backup file too small (${DOWNLOADED_SIZE} bytes)"
    push_verify_metric 0
    exit 1
fi

# ── Step 3: Checksum verification ─────────────────────────
CHECKSUM_EXISTS=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}.sha256" 2>/dev/null | wc -l)
if [ "${CHECKSUM_EXISTS}" -gt 0 ]; then
    aws s3 cp ${S3_ARGS} \
        "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}.sha256" \
        /tmp/verify_backup.sql.gz.sha256

    EXPECTED=$(awk '{print $1}' /tmp/verify_backup.sql.gz.sha256)
    ACTUAL=$(sha256sum /tmp/verify_backup.sql.gz | awk '{print $1}')

    if [ "${EXPECTED}" != "${ACTUAL}" ]; then
        log "ERROR: Checksum mismatch!"
        log "  Expected: ${EXPECTED}"
        log "  Actual:   ${ACTUAL}"
        push_verify_metric 0
        exit 1
    fi
    log "Checksum verified: ${ACTUAL}"
else
    log "WARNING: No checksum file found, skipping checksum verification"
fi

# ── Step 4: Test restore into throwaway database ──────────
log "Creating test database ${VERIFY_DB}..."

psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres \
    -c "DROP DATABASE IF EXISTS ${VERIFY_DB};"
psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d postgres \
    -c "CREATE DATABASE ${VERIFY_DB};"

log "Restoring into ${VERIFY_DB}..."
gunzip -c /tmp/verify_backup.sql.gz | psql \
    -h "${PGHOST}" \
    -p "${PGPORT}" \
    -U "${PGUSER}" \
    -d "${VERIFY_DB}" \
    --quiet \
    2>/tmp/verify_stderr.log

# ── Step 5: Validate restored data ───────────────────────
TABLE_COUNT=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${VERIFY_DB}" -t -c "
    SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public';
" | tr -d ' ')

if [ -z "${TABLE_COUNT}" ] || [ "${TABLE_COUNT}" -lt 5 ]; then
    log "ERROR: Restored database has too few tables (${TABLE_COUNT})"
    push_verify_metric 0
    exit 1
fi

# Check critical tables exist
for table in users intents bids fills markets solvers balances; do
    EXISTS=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${VERIFY_DB}" -t -c "
        SELECT COUNT(*) FROM information_schema.tables
        WHERE table_schema = 'public' AND table_name = '${table}';
    " | tr -d ' ')
    if [ "${EXISTS}" != "1" ]; then
        log "ERROR: Critical table '${table}' missing from backup"
        push_verify_metric 0
        exit 1
    fi
done

# Count rows in key tables for sanity
USER_COUNT=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${VERIFY_DB}" -t -c "SELECT COUNT(*) FROM users;" | tr -d ' ')
INTENT_COUNT=$(psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${VERIFY_DB}" -t -c "SELECT COUNT(*) FROM intents;" | tr -d ' ')

log "Verification passed:"
log "  Tables:  ${TABLE_COUNT}"
log "  Users:   ${USER_COUNT}"
log "  Intents: ${INTENT_COUNT}"

push_verify_metric 1

# cleanup() runs via trap to drop the test database
log "Backup verification complete"
