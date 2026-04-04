-- ============================================================
-- Consolidate executions timestamp columns
--
-- Before: created_at BIGINT, created_ts TIMESTAMPTZ
-- After:  created_at TIMESTAMPTZ
--
-- Strategy:
--   1. Backfill created_ts from created_at epoch where NULL
--   2. Drop the BIGINT column
--   3. Rename created_ts → created_at
--   4. Recreate indexes on new column name
--
-- For partitioned tables, ALTER TABLE on the parent propagates
-- to all partitions automatically.
-- ============================================================

-- 1. Backfill any rows where created_ts is NULL
UPDATE executions
SET created_ts = to_timestamp(created_at)
WHERE created_ts IS NULL;

-- 2. Drop the BIGINT column
ALTER TABLE executions DROP COLUMN IF EXISTS created_at;

-- 3. Rename created_ts → created_at
ALTER TABLE executions RENAME COLUMN created_ts TO created_at;

-- 4. Recreate indexes (old ones referenced created_ts)
DROP INDEX IF EXISTS idx_executions_p_intent;
DROP INDEX IF EXISTS idx_executions_p_status;

CREATE INDEX IF NOT EXISTS idx_executions_intent_created
    ON executions (intent_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_executions_status_created
    ON executions (status, created_at);
