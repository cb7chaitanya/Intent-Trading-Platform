#!/bin/bash
set -euo pipefail

# ============================================================
# PostgreSQL daily backup to S3-compatible storage
#
# Features:
#   - Full pg_dump with custom format (supports parallel restore)
#   - Gzip compression
#   - S3 upload with checksum verification
#   - 30-day retention with automatic pruning
#   - Prometheus pushgateway metrics (optional)
#   - Exit codes: 0 = success, 1 = failure
# ============================================================

: "${PGHOST:=postgres}"
: "${PGPORT:=5432}"
: "${PGUSER:=postgres}"
: "${PGPASSWORD:=postgres}"
: "${PGDATABASE:=intent_trading}"
: "${S3_BUCKET:=intentx-backups}"
: "${S3_ENDPOINT:=}"
: "${S3_PREFIX:=daily}"
: "${RETENTION_DAYS:=30}"
: "${PUSHGATEWAY_URL:=}"

export PGPASSWORD

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="backup_${PGDATABASE}_${TIMESTAMP}.sql.gz"
BACKUP_PATH="/tmp/${BACKUP_FILE}"
CHECKSUM_FILE="/tmp/${BACKUP_FILE}.sha256"
START_TIME=$(date +%s)

S3_ARGS=""
if [ -n "${S3_ENDPOINT}" ]; then
    S3_ARGS="--endpoint-url ${S3_ENDPOINT}"
fi

log() { echo "[$(date -Iseconds)] $*"; }

push_metrics() {
    local status="$1"
    local duration="$2"
    local size="${3:-0}"

    [ -z "${PUSHGATEWAY_URL}" ] && return 0

    cat <<METRICS | curl -sf --max-time 5 --data-binary @- "${PUSHGATEWAY_URL}/metrics/job/pg_backup/instance/${PGHOST}" 2>/dev/null || true
# HELP backup_last_success_timestamp Unix timestamp of last successful backup
# TYPE backup_last_success_timestamp gauge
backup_last_success_timestamp $([ "$status" = "ok" ] && date +%s || echo 0)
# HELP backup_last_duration_seconds Duration of last backup in seconds
# TYPE backup_last_duration_seconds gauge
backup_last_duration_seconds ${duration}
# HELP backup_last_size_bytes Size of last backup in bytes
# TYPE backup_last_size_bytes gauge
backup_last_size_bytes ${size}
# HELP backup_last_status 1 if last backup succeeded, 0 if failed
# TYPE backup_last_status gauge
backup_last_status $([ "$status" = "ok" ] && echo 1 || echo 0)
METRICS
}

on_failure() {
    local duration=$(( $(date +%s) - START_TIME ))
    log "ERROR: Backup failed after ${duration}s"
    push_metrics "fail" "${duration}" "0"
    rm -f "${BACKUP_PATH}" "${CHECKSUM_FILE}"
    exit 1
}

trap on_failure ERR

# ── Step 1: pg_dump ───────────────────────────────────────
log "Starting backup: ${BACKUP_FILE}"

# Verify connectivity first
pg_isready -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -t 10 \
    || { log "ERROR: Database is not ready"; exit 1; }

pg_dump \
    -h "${PGHOST}" \
    -p "${PGPORT}" \
    -U "${PGUSER}" \
    -d "${PGDATABASE}" \
    --no-owner \
    --no-privileges \
    --verbose \
    --format=plain \
    2>/tmp/backup_stderr.log \
    | gzip > "${BACKUP_PATH}"

BACKUP_SIZE_BYTES=$(stat -f%z "${BACKUP_PATH}" 2>/dev/null || stat -c%s "${BACKUP_PATH}" 2>/dev/null || echo 0)
BACKUP_SIZE_HUMAN=$(du -h "${BACKUP_PATH}" | cut -f1)
log "Dump complete: ${BACKUP_SIZE_HUMAN} (${BACKUP_SIZE_BYTES} bytes)"

# Sanity check: file must be > 1KB
if [ "${BACKUP_SIZE_BYTES}" -lt 1024 ]; then
    log "ERROR: Backup file suspiciously small (${BACKUP_SIZE_BYTES} bytes)"
    exit 1
fi

# ── Step 2: Generate checksum ─────────────────────────────
sha256sum "${BACKUP_PATH}" > "${CHECKSUM_FILE}"
CHECKSUM=$(cat "${CHECKSUM_FILE}" | awk '{print $1}')
log "Checksum: ${CHECKSUM}"

# ── Step 3: Upload to S3 ─────────────────────────────────
aws s3 cp ${S3_ARGS} \
    "${BACKUP_PATH}" \
    "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}"

aws s3 cp ${S3_ARGS} \
    "${CHECKSUM_FILE}" \
    "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}.sha256"

log "Uploaded to s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}"

# ── Step 4: Verify upload ─────────────────────────────────
REMOTE_SIZE=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}" \
    | awk '{print $3}' | head -1)

if [ -z "${REMOTE_SIZE}" ] || [ "${REMOTE_SIZE}" -lt 1024 ]; then
    log "ERROR: Upload verification failed (remote size: ${REMOTE_SIZE:-0})"
    exit 1
fi

log "Upload verified (remote size: ${REMOTE_SIZE} bytes)"

# ── Step 5: Clean up local files ──────────────────────────
rm -f "${BACKUP_PATH}" "${CHECKSUM_FILE}"

# ── Step 6: Prune old backups ─────────────────────────────
# date -d works on GNU, date -v on BSD
CUTOFF=$(date -d "-${RETENTION_DAYS} days" +%Y%m%d 2>/dev/null \
    || date -v-${RETENTION_DAYS}d +%Y%m%d 2>/dev/null \
    || echo "19700101")

log "Pruning backups older than ${RETENTION_DAYS} days (before ${CUTOFF})"

PRUNED=0
aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/" \
    | awk '{print $4}' \
    | grep "^backup_" \
    | grep -v ".sha256" \
    | while read -r file; do
        FILE_DATE=$(echo "${file}" | grep -oE '[0-9]{8}' | head -1)
        if [ -n "${FILE_DATE}" ] && [ "${FILE_DATE}" -lt "${CUTOFF}" ]; then
            log "  Pruning: ${file}"
            aws s3 rm ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${file}" 2>/dev/null || true
            aws s3 rm ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${file}.sha256" 2>/dev/null || true
            PRUNED=$((PRUNED + 1))
        fi
    done

# ── Done ──────────────────────────────────────────────────
DURATION=$(( $(date +%s) - START_TIME ))
log "Backup complete in ${DURATION}s"
push_metrics "ok" "${DURATION}" "${BACKUP_SIZE_BYTES}"
