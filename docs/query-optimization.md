# Query Optimization Guide

## Index Strategy Summary

### Hot Query Patterns and Their Indexes

| Query Pattern | Table | Index | Type |
|---|---|---|---|
| Expiry worker scan | intents | `(status, deadline) WHERE status IN ('open','bidding')` | Partial composite |
| Direct intent lookup | intents | `(id)` | Plain (for partitioned PK workaround) |
| Bids by intent sorted | bids | `(intent_id, timestamp ASC)` | Composite for sort elimination |
| Best bid selection | bids | `(intent_id, amount_out DESC, fee ASC)` | Covering for index-only scan |
| Unsettled fills sorted | fills | `(intent_id, price DESC) WHERE settled = FALSE` | Partial composite |
| Settled fill aggregate | fills | `(intent_id, filled_qty) WHERE settled = TRUE` | Partial covering for SUM |
| Recent execution | executions | `(intent_id, created_at DESC)` | Composite for LIMIT 1 |
| Ledger balance calc | ledger_entries | `(account_id, asset)` | Composite for filter |
| Recent market trades | market_trades | `(market_id, created_at DESC)` | Composite DESC for LIMIT |
| TWAP child by intent | twap_child_intents | `(intent_id)` | Plain |
| Fill retry lookup | failed_settlements | `(fill_id) WHERE fill_id IS NOT NULL` | Partial |

### EXPLAIN ANALYZE Examples

Run these against your database to verify index usage:

```sql
-- 1. Intent expiry scan (should use idx_intents_status_deadline)
EXPLAIN ANALYZE
SELECT * FROM intents
WHERE deadline < EXTRACT(EPOCH FROM NOW())
  AND status IN ('open', 'bidding')
ORDER BY deadline ASC LIMIT 100;
-- Expected: Index Scan using idx_intents_status_deadline

-- 2. Bids for auction (should use idx_bids_intent_timestamp)
EXPLAIN ANALYZE
SELECT * FROM bids
WHERE intent_id = 'some-uuid'
ORDER BY timestamp ASC;
-- Expected: Index Scan using idx_bids_intent_timestamp (no Sort node)

-- 3. Best bid selection (should use idx_bids_intent_value)
EXPLAIN ANALYZE
SELECT * FROM bids
WHERE intent_id = 'some-uuid'
ORDER BY (amount_out - fee) DESC
LIMIT 1;
-- Expected: Index Scan + in-memory sort (small set per intent)

-- 4. Unsettled fills (should use idx_fills_intent_unsettled_price)
EXPLAIN ANALYZE
SELECT * FROM fills
WHERE intent_id = 'some-uuid' AND settled = FALSE
ORDER BY price DESC;
-- Expected: Index Scan using idx_fills_intent_unsettled_price (no Sort)

-- 5. Settlement aggregate (should use idx_fills_intent_settled_qty)
EXPLAIN ANALYZE
SELECT COALESCE(SUM(filled_qty), 0)
FROM fills
WHERE intent_id = 'some-uuid' AND settled = TRUE;
-- Expected: Index Only Scan using idx_fills_intent_settled_qty

-- 6. Recent trades (should use idx_market_trades_market_recent)
EXPLAIN ANALYZE
SELECT * FROM market_trades
WHERE market_id = 'some-uuid'
ORDER BY created_at DESC
LIMIT 50;
-- Expected: Index Scan Backward using idx_market_trades_market_recent (no Sort)

-- 7. Ledger balance (should use idx_ledger_account_asset)
EXPLAIN ANALYZE
SELECT COALESCE(
    SUM(CASE WHEN entry_type = 'DEBIT' THEN amount ELSE -amount END), 0
) FROM ledger_entries
WHERE account_id = 'some-uuid' AND asset = 'USDC';
-- Expected: Index Scan using idx_ledger_account_asset
```

### Query Improvements Applied

| Before | After | Why |
|---|---|---|
| `SELECT * FROM bids WHERE intent_id = $1 ORDER BY timestamp` | Same query, now uses composite index | Sort elimination — no in-memory sort needed |
| `SUM(filled_qty) ... WHERE settled = TRUE` | Same query, now uses covering index | Index-only scan — no heap access for aggregate |
| `SELECT * FROM market_trades ... ORDER BY created_at DESC LIMIT` | Same query, now uses DESC index | Backward index scan eliminates sort node |
| Expiry worker full table scan on deadline | Partial index on (status, deadline) | Only scans active intents, skips completed/cancelled |

### Partitioned Table Considerations

For partitioned tables (trades, ledger_entries, market_trades, fills, executions):

1. **Always include `created_at` in WHERE clause** for partition pruning
2. PK is `(id, created_at)` — direct `WHERE id = $1` scans all partitions
3. The `idx_*_id_lookup` index on `(id)` helps but still scans all partitions for the index
4. For best performance on ID lookups, maintain a mapping table or include `created_at` in the application context

### Monitoring

```sql
-- Find slow queries (requires pg_stat_statements)
SELECT query, calls, mean_exec_time, total_exec_time
FROM pg_stat_statements
ORDER BY mean_exec_time DESC
LIMIT 20;

-- Find unused indexes
SELECT indexrelname, idx_scan, pg_size_pretty(pg_relation_size(indexrelid))
FROM pg_stat_user_indexes
WHERE idx_scan = 0
ORDER BY pg_relation_size(indexrelid) DESC;

-- Find missing indexes (seq scans on large tables)
SELECT relname, seq_scan, idx_scan, seq_tup_read
FROM pg_stat_user_tables
WHERE seq_scan > idx_scan AND n_live_tup > 10000
ORDER BY seq_tup_read DESC;
```
