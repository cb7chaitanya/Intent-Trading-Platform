# Postgres Partitioning Strategy

## Overview

Five high-volume tables are partitioned by month using Postgres RANGE partitioning on `created_at`:

| Table | Partition Key | Notes |
|---|---|---|
| `trades` | `created_at TIMESTAMPTZ` | PK: `(id, created_at)` |
| `ledger_entries` | `created_at TIMESTAMPTZ` | PK: `(id, created_at)` |
| `market_trades` | `created_at TIMESTAMPTZ` | PK: `(id, created_at)` |
| `fills` | `created_at TIMESTAMPTZ` | Added column; PK: `(id, created_at)` |
| `executions` | `created_ts TIMESTAMPTZ` | Added column; PK: `(id, created_ts)` |

## Partition Naming

Format: `{table}_y{YYYY}m{MM}`

Examples:
- `trades_y2026m04`
- `ledger_entries_y2026m05`

## Index Strategy

Per-partition indexes (inherited from parent):

| Table | Indexes |
|---|---|
| trades | `(buyer_account_id, created_at)`, `(seller_account_id, created_at)`, `(status, created_at)` |
| ledger_entries | `(account_id, created_at)`, `(reference_id)` |
| market_trades | `(market_id, created_at)` |
| fills | `(intent_id, created_at)`, partial on `intent_id WHERE settled = FALSE` |
| executions | `(intent_id, created_ts)`, `(status, created_ts)` |

## Automatic Partition Creation

### Via Application Worker

The `partition_manager` worker runs daily and calls:

```sql
SELECT create_monthly_partitions(3);
```

This creates partitions for the current month + 3 months ahead.

### Via pg_cron (Alternative)

```sql
SELECT cron.schedule('create-partitions', '0 0 1 * *',
    $$SELECT create_monthly_partitions(3)$$);
```

### Manual

```sql
CREATE TABLE trades_y2026m07
    PARTITION OF trades
    FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');
```

## Query Patterns

Queries that include `created_at` in the WHERE clause benefit from partition pruning:

```sql
-- Fast: scans only relevant partition(s)
SELECT * FROM trades WHERE created_at >= '2026-04-01' AND created_at < '2026-05-01';

-- Also fast: Postgres prunes based on range
SELECT * FROM ledger_entries WHERE account_id = $1 AND created_at >= NOW() - INTERVAL '30 days';

-- Slower: scans all partitions (no created_at filter)
SELECT * FROM trades WHERE id = $1;
```

**Best practice**: Always include a time range in queries against partitioned tables.

## Foreign Keys

Partitioned tables cannot be targets of foreign keys in Postgres. References from `fills.intent_id → intents.id` and `executions.intent_id → intents.id` are enforced at the application layer after partitioning.

## Data Retention

To drop old data efficiently:

```sql
-- Detach partition (instant, no row-level locking)
ALTER TABLE trades DETACH PARTITION trades_y2025m01;

-- Archive or drop
DROP TABLE trades_y2025m01;
```

## Monitoring

```sql
-- List all partitions for a table
SELECT inhrelid::regclass AS partition
FROM pg_inherits
WHERE inhparent = 'trades'::regclass
ORDER BY partition;

-- Partition sizes
SELECT pg_size_pretty(pg_total_relation_size(inhrelid)) AS size,
       inhrelid::regclass AS partition
FROM pg_inherits
WHERE inhparent = 'trades'::regclass
ORDER BY pg_total_relation_size(inhrelid) DESC;
```
