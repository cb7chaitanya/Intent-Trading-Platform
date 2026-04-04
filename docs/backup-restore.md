# Backup & Restore Guide

## Architecture

```
┌──────────┐   daily 2am    ┌───────────┐    S3 API     ┌─────────────┐
│ Postgres │ ──pg_dump──→   │ pg-backup │ ──upload──→   │ S3 / MinIO  │
│          │                │ container │               │ (30d retain) │
└──────────┘                └───────────┘               └─────────────┘
     │
     │  WAL archiving (optional)
     └──────────────────────────────────────────────────→
```

## Daily Backups

The `pg-backup` container runs `crond` with a daily backup at 02:00 UTC.

### Start (production profile)

```bash
docker compose --profile production up -d pg-backup
```

### Manual backup

```bash
docker compose --profile production exec pg-backup /scripts/backup.sh
```

### What the backup does

1. `pg_dump` full database → gzip
2. Upload to `s3://{bucket}/{prefix}/backup_{db}_{timestamp}.sql.gz`
3. Prune backups older than `RETENTION_DAYS` (default: 30)

## Restore

### From latest backup

```bash
docker compose --profile production run --rm pg-backup /scripts/restore.sh
```

### From specific backup

```bash
docker compose --profile production run --rm pg-backup \
    /scripts/restore.sh backup_intent_trading_20260401_120000.sql.gz
```

### Restore steps

1. Downloads backup from S3
2. Terminates existing connections
3. Drops and recreates the database
4. Restores from dump
5. Prints table count for verification

**After restore, run migrations:**

```bash
sqlx migrate run --database-url postgres://postgres:postgres@localhost:5432/intent_trading
```

## WAL Archiving (Point-in-Time Recovery)

For continuous archiving, add to your `postgresql.conf`:

```ini
archive_mode = on
archive_command = '/scripts/wal-archive.sh %p %f'
```

And mount the WAL archive script into the Postgres container.

## S3 Storage Options

### MinIO (local development)

Add to docker-compose:
```yaml
minio:
  image: minio/minio
  ports:
    - "9000:9000"
    - "9001:9001"
  environment:
    MINIO_ROOT_USER: minioadmin
    MINIO_ROOT_PASSWORD: minioadmin
  command: server /data --console-address ":9001"
```

Create bucket: `mc mb local/intentx-backups`

### AWS S3 (production)

Set environment variables:
```
S3_BUCKET=your-bucket-name
S3_ENDPOINT=             # leave empty for AWS
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=us-east-1
```

## Configuration

| Variable | Default | Description |
|---|---|---|
| `PGHOST` | postgres | Database host |
| `PGDATABASE` | intent_trading | Database name |
| `S3_BUCKET` | intentx-backups | S3 bucket |
| `S3_ENDPOINT` | (empty=AWS) | S3-compatible endpoint |
| `S3_PREFIX` | daily | Key prefix |
| `RETENTION_DAYS` | 30 | Days to keep backups |

## Monitoring

Check backup logs:
```bash
docker compose --profile production logs pg-backup --tail 50
```

Verify backups exist:
```bash
aws s3 ls s3://intentx-backups/daily/ --endpoint-url http://localhost:9000
```
