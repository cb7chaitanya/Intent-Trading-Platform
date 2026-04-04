FROM postgres:16-alpine

RUN apk add --no-cache aws-cli bash gzip

COPY backup.sh /scripts/backup.sh
COPY restore.sh /scripts/restore.sh
COPY wal-archive.sh /scripts/wal-archive.sh
RUN chmod +x /scripts/*.sh

# Default: run daily backup at 2 AM UTC via crond
RUN echo "0 2 * * * /scripts/backup.sh >> /var/log/backup.log 2>&1" > /etc/crontabs/root

CMD ["crond", "-f", "-d", "8"]
