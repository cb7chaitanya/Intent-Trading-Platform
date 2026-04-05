#!/bin/bash
set -euo pipefail

# ============================================================
# WAL segment cleanup
#
# Removes WAL segments from S3 that are older than the latest
# full backup. WALs before the latest backup are not needed
# for recovery since we can restore from the backup instead.
# ============================================================

: "${S3_BUCKET:=intentx-backups}"
: "${S3_ENDPOINT:=}"
: "${S3_PREFIX_BACKUP:=daily}"
: "${S3_PREFIX_WAL:=wal}"
: "${WAL_RETENTION_DAYS:=7}"

S3_ARGS=""
if [ -n "${S3_ENDPOINT}" ]; then
    S3_ARGS="--endpoint-url ${S3_ENDPOINT}"
fi

log() { echo "[$(date -Iseconds)] WAL-CLEANUP: $*"; }

# Find the date of the latest backup
LATEST_BACKUP=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX_BACKUP}/" \
    | awk '{print $4}' \
    | grep "^backup_" \
    | grep -v ".sha256" \
    | sort -r \
    | head -1)

if [ -z "${LATEST_BACKUP}" ]; then
    log "No backups found, skipping WAL cleanup"
    exit 0
fi

BACKUP_DATE=$(echo "${LATEST_BACKUP}" | grep -oE '[0-9]{8}' | head -1)
log "Latest backup date: ${BACKUP_DATE}"

# Also enforce a hard retention: remove WALs older than WAL_RETENTION_DAYS
CUTOFF=$(date -d "-${WAL_RETENTION_DAYS} days" +%Y%m%d 2>/dev/null \
    || date -v-${WAL_RETENTION_DAYS}d +%Y%m%d 2>/dev/null \
    || echo "19700101")

# List all WAL files and remove old ones
CLEANED=0
aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX_WAL}/" 2>/dev/null \
    | while read -r line; do
        FILE_DATE=$(echo "${line}" | awk '{print $1}' | tr -d '-')
        FILE_NAME=$(echo "${line}" | awk '{print $4}')
        if [ -n "${FILE_DATE}" ] && [ "${FILE_DATE}" -lt "${CUTOFF}" ]; then
            aws s3 rm ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX_WAL}/${FILE_NAME}" 2>/dev/null || true
            CLEANED=$((CLEANED + 1))
        fi
    done

log "WAL cleanup complete (retention: ${WAL_RETENTION_DAYS} days)"
