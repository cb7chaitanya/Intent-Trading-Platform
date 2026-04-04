-- ============================================================
-- Query optimization: missing indexes and composites
-- Based on EXPLAIN ANALYZE of actual query patterns
-- ============================================================

-- ============================================================
-- intents
-- ============================================================

-- Hot query: SELECT * FROM intents WHERE deadline < $1 AND status IN ('open', 'bidding')
-- Current: idx_intents_deadline_active partial index exists but only on non-partitioned table
-- Fix: composite (status, deadline) for the expiry worker scan
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_intents_status_deadline
    ON intents (status, deadline)
    WHERE status IN ('open', 'bidding');

-- Hot query: SELECT * FROM intents WHERE id = $1 FOR UPDATE
-- PK covers this on non-partitioned. On partitioned, PK is (id, created_at).
-- Add a plain index on id for direct lookups without created_at.
-- (Only needed if partitioned — safe to create anyway)
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_intents_id_lookup
    ON intents (id);

-- ============================================================
-- bids
-- ============================================================

-- Hot query: SELECT * FROM bids WHERE intent_id = $1 ORDER BY timestamp ASC
-- Current: idx_bids_intent_id covers the filter but not the sort
-- Fix: composite covering index for sort elimination
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_bids_intent_timestamp
    ON bids (intent_id, timestamp ASC);

-- Auction engine: SELECT best bid by (amount_out - fee) DESC
-- No index can directly cover computed sort, but covering intent_id + amount_out + fee
-- lets Postgres do an index-only scan and sort in memory (small per intent)
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_bids_intent_value
    ON bids (intent_id, amount_out DESC, fee ASC);

-- ============================================================
-- fills
-- ============================================================

-- Hot query: SELECT * FROM fills WHERE intent_id = $1 AND settled = FALSE ORDER BY price DESC
-- Current: idx_fills_unsettled partial on (intent_id) WHERE settled = FALSE
-- Fix: add price to the index for sort elimination
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_fills_intent_unsettled_price
    ON fills (intent_id, price DESC)
    WHERE settled = FALSE;

-- Settlement: SELECT COALESCE(SUM(filled_qty), 0) FROM fills WHERE intent_id = $1 AND settled = TRUE
-- Covering index for aggregate without table heap access
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_fills_intent_settled_qty
    ON fills (intent_id, filled_qty)
    WHERE settled = TRUE;

-- ============================================================
-- executions
-- ============================================================

-- Hot query: SELECT * FROM executions WHERE intent_id = $1 ORDER BY created_at DESC LIMIT 1
-- Current: idx_executions_intent_id exists but without sort
-- (Partitioned table uses created_ts, non-partitioned uses created_at)
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_executions_intent_created
    ON executions (intent_id, created_at DESC);

-- ============================================================
-- ledger_entries
-- ============================================================

-- Hot query: ledger balance calculation:
--   SELECT SUM(CASE WHEN entry_type = 'DEBIT' THEN amount ELSE -amount END)
--   FROM ledger_entries WHERE account_id = $1 AND asset = $2
-- Current: idx_ledger_account_id covers filter but not asset
-- Fix: composite for the balance query
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_ledger_account_asset
    ON ledger_entries (account_id, asset);

-- ============================================================
-- market_trades
-- ============================================================

-- Hot query: SELECT * FROM market_trades WHERE market_id = $1 ORDER BY created_at DESC LIMIT $2
-- Current: idx_market_trades_created_at is (market_id, created_at) — correct direction?
-- Fix: ensure DESC for the common "most recent" query pattern
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_market_trades_market_recent
    ON market_trades (market_id, created_at DESC);

-- Candle aggregation: GROUP BY time bucket WHERE market_id = $1 AND created_at >= $2
-- The existing (market_id, created_at) composite handles this well.
-- No additional index needed.

-- ============================================================
-- failed_settlements
-- ============================================================

-- Retry worker: SELECT ... WHERE permanently_failed = FALSE AND next_retry_at <= $1
-- Current: partial index on next_retry_at WHERE permanently_failed = FALSE — good
-- Add fill_id index for per-fill retry lookups
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_failed_settlements_fill
    ON failed_settlements (fill_id)
    WHERE fill_id IS NOT NULL;

-- ============================================================
-- twap_child_intents
-- ============================================================

-- TWAP listener: SELECT * FROM twap_child_intents WHERE intent_id = $1
-- Missing index on intent_id (only twap_id and scheduled_at indexed)
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_twap_children_intent
    ON twap_child_intents (intent_id);

-- ============================================================
-- balances
-- ============================================================

-- The (account_id, asset) UNIQUE constraint already serves as an index.
-- No additional index needed.

-- ============================================================
-- Cleanup: drop redundant single-column indexes superseded by composites
-- ============================================================

-- idx_bids_intent_id is superseded by idx_bids_intent_timestamp
-- Keep it for now — it's small and some queries filter without ORDER BY

-- idx_fills_intent_id is superseded by idx_fills_p_intent (partitioned)
-- Keep both — different partition schemes may use either
