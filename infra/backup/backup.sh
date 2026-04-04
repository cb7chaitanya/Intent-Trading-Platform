#!/bin/bash
set -euo pipefail

# ============================================================
# Postgres daily backup to S3-compatible storage
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

export PGPASSWORD

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="backup_${PGDATABASE}_${TIMESTAMP}.sql.gz"
BACKUP_PATH="/tmp/${BACKUP_FILE}"

echo "[$(date -Iseconds)] Starting backup: ${BACKUP_FILE}"

# Full database dump with custom format, compressed
pg_dump \
    -h "${PGHOST}" \
    -p "${PGPORT}" \
    -U "${PGUSER}" \
    -d "${PGDATABASE}" \
    --no-owner \
    --no-privileges \
    --verbose \
    2>/tmp/backup_stderr.log \
    | gzip > "${BACKUP_PATH}"

BACKUP_SIZE=$(du -h "${BACKUP_PATH}" | cut -f1)
echo "[$(date -Iseconds)] Dump complete: ${BACKUP_SIZE}"

# Upload to S3
S3_ARGS=""
if [ -n "${S3_ENDPOINT}" ]; then
    S3_ARGS="--endpoint-url ${S3_ENDPOINT}"
fi

aws s3 cp ${S3_ARGS} \
    "${BACKUP_PATH}" \
    "s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}"

echo "[$(date -Iseconds)] Uploaded to s3://${S3_BUCKET}/${S3_PREFIX}/${BACKUP_FILE}"

# Clean up local file
rm -f "${BACKUP_PATH}"

# Prune old backups beyond retention period
CUTOFF=$(date -d "-${RETENTION_DAYS} days" +%Y%m%d 2>/dev/null || date -v-${RETENTION_DAYS}d +%Y%m%d)

echo "[$(date -Iseconds)] Pruning backups older than ${RETENTION_DAYS} days (before ${CUTOFF})"

aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/" \
    | awk '{print $4}' \
    | grep "^backup_" \
    | while read -r file; do
        # Extract date from filename: backup_intent_trading_20260101_120000.sql.gz
        FILE_DATE=$(echo "${file}" | grep -oE '[0-9]{8}' | head -1)
        if [ -n "${FILE_DATE}" ] && [ "${FILE_DATE}" -lt "${CUTOFF}" ]; then
            echo "  Deleting: ${file}"
            aws s3 rm ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${file}"
        fi
    done

echo "[$(date -Iseconds)] Backup complete"
