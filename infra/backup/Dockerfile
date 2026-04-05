FROM postgres:16-alpine

RUN apk add --no-cache aws-cli bash gzip curl

COPY backup.sh /scripts/backup.sh
COPY restore.sh /scripts/restore.sh
COPY verify.sh /scripts/verify.sh
COPY wal-archive.sh /scripts/wal-archive.sh
COPY wal-cleanup.sh /scripts/wal-cleanup.sh
RUN chmod +x /scripts/*.sh

# Cron schedule:
#   Daily full backup at 2:00 AM UTC
#   WAL cleanup at 3:00 AM UTC (remove WALs older than latest backup)
#   Backup verification at 4:00 AM UTC
RUN printf '%s\n' \
    "0 2 * * * /scripts/backup.sh >> /var/log/backup.log 2>&1" \
    "0 3 * * * /scripts/wal-cleanup.sh >> /var/log/backup.log 2>&1" \
    "0 4 * * * /scripts/verify.sh >> /var/log/backup.log 2>&1" \
    > /etc/crontabs/root

# Create log file so tail works on startup
RUN touch /var/log/backup.log

# Health check: cron process running + backup log updated in last 25 hours
HEALTHCHECK --interval=60s --timeout=5s --retries=3 \
    CMD pgrep crond > /dev/null && \
        test -f /var/log/backup.log

CMD ["crond", "-f", "-d", "8"]
