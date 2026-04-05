#!/bin/bash
set -euo pipefail

# ============================================================
# WAL archiving to S3 for point-in-time recovery
#
# Configure in postgresql.conf:
#   archive_mode = on
#   archive_command = '/scripts/wal-archive.sh %p %f'
#   archive_timeout = 60
#
# Parameters:
#   $1 = full path to WAL file (%p)
#   $2 = WAL file name (%f)
# ============================================================

: "${S3_BUCKET:=intentx-backups}"
: "${S3_ENDPOINT:=}"
: "${S3_PREFIX:=wal}"

WAL_PATH="$1"
WAL_FILE="$2"

S3_ARGS=""
if [ -n "${S3_ENDPOINT}" ]; then
    S3_ARGS="--endpoint-url ${S3_ENDPOINT}"
fi

# Check if already archived (idempotent)
EXISTS=$(aws s3 ls ${S3_ARGS} "s3://${S3_BUCKET}/${S3_PREFIX}/${WAL_FILE}.gz" 2>/dev/null | wc -l)
if [ "${EXISTS}" -gt 0 ]; then
    echo "[$(date -Iseconds)] WAL ${WAL_FILE} already archived, skipping"
    exit 0
fi

# Compress and upload
gzip -c "${WAL_PATH}" | aws s3 cp ${S3_ARGS} \
    - "s3://${S3_BUCKET}/${S3_PREFIX}/${WAL_FILE}.gz"

echo "[$(date -Iseconds)] WAL archived: ${WAL_FILE}"
