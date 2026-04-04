-- ============================================================
-- Postgres RANGE partitioning for high-volume tables
--
-- Strategy:
--   - Partition by created_at month
--   - PK includes partition key: (id, created_at)
--   - Foreign keys TO partitioned tables are dropped
--     (application layer enforces referential integrity)
--   - Indexes are created per-partition automatically
--   - A function + trigger auto-creates future partitions
-- ============================================================

-- ============================================================
-- 1. trades → trades_partitioned
-- ============================================================

-- Rename old table
ALTER TABLE IF EXISTS trades RENAME TO trades_old;

-- Create partitioned table
CREATE TABLE trades (
    id UUID NOT NULL,
    buyer_account_id UUID NOT NULL,
    seller_account_id UUID NOT NULL,
    solver_account_id UUID NOT NULL,
    asset_in asset_type NOT NULL,
    asset_out asset_type NOT NULL,
    amount_in BIGINT NOT NULL,
    amount_out BIGINT NOT NULL,
    platform_fee BIGINT NOT NULL DEFAULT 0,
    solver_fee BIGINT NOT NULL DEFAULT 0,
    status trade_status NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at TIMESTAMPTZ,
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_trades_p_buyer ON trades (buyer_account_id, created_at);
CREATE INDEX idx_trades_p_seller ON trades (seller_account_id, created_at);
CREATE INDEX idx_trades_p_status ON trades (status, created_at);

-- Migrate data
INSERT INTO trades SELECT * FROM trades_old ON CONFLICT DO NOTHING;
DROP TABLE IF EXISTS trades_old CASCADE;

-- ============================================================
-- 2. ledger_entries → partitioned
-- ============================================================

ALTER TABLE IF EXISTS ledger_entries RENAME TO ledger_entries_old;

CREATE TABLE ledger_entries (
    id UUID NOT NULL,
    account_id UUID NOT NULL,
    asset asset_type NOT NULL,
    amount BIGINT NOT NULL,
    entry_type entry_type NOT NULL,
    reference_type reference_type NOT NULL,
    reference_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_ledger_p_account ON ledger_entries (account_id, created_at);
CREATE INDEX idx_ledger_p_reference ON ledger_entries (reference_id);

INSERT INTO ledger_entries SELECT * FROM ledger_entries_old ON CONFLICT DO NOTHING;
DROP TABLE IF EXISTS ledger_entries_old CASCADE;

-- ============================================================
-- 3. market_trades → partitioned
-- ============================================================

ALTER TABLE IF EXISTS market_trades RENAME TO market_trades_old;

CREATE TABLE market_trades (
    id UUID NOT NULL,
    market_id UUID NOT NULL,
    buyer_account_id UUID NOT NULL,
    seller_account_id UUID NOT NULL,
    price BIGINT NOT NULL,
    qty BIGINT NOT NULL,
    fee BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_market_trades_p_market ON market_trades (market_id, created_at);

INSERT INTO market_trades SELECT * FROM market_trades_old ON CONFLICT DO NOTHING;
DROP TABLE IF EXISTS market_trades_old CASCADE;

-- ============================================================
-- 4. fills → partitioned (uses timestamp column as BIGINT,
--    convert to TIMESTAMPTZ for partitioning)
-- ============================================================

ALTER TABLE IF EXISTS fills RENAME TO fills_old;

CREATE TABLE fills (
    id UUID NOT NULL,
    intent_id UUID NOT NULL,
    solver_id TEXT NOT NULL,
    price BIGINT NOT NULL,
    qty BIGINT NOT NULL,
    filled_qty BIGINT NOT NULL,
    tx_hash TEXT NOT NULL DEFAULT '',
    timestamp BIGINT NOT NULL,
    settled BOOLEAN NOT NULL DEFAULT FALSE,
    settled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_fills_p_intent ON fills (intent_id, created_at);
CREATE INDEX idx_fills_p_unsettled ON fills (intent_id) WHERE settled = FALSE;

INSERT INTO fills (id, intent_id, solver_id, price, qty, filled_qty, tx_hash, timestamp, settled, settled_at, created_at)
    SELECT id, intent_id, solver_id, price, qty, filled_qty, tx_hash, timestamp, settled, settled_at,
           COALESCE(settled_at, to_timestamp(timestamp))
    FROM fills_old ON CONFLICT DO NOTHING;
DROP TABLE IF EXISTS fills_old CASCADE;

-- ============================================================
-- 5. executions → partitioned (uses created_at as BIGINT,
--    add TIMESTAMPTZ column for partitioning)
-- ============================================================

ALTER TABLE IF EXISTS executions RENAME TO executions_old;

CREATE TABLE executions (
    id UUID NOT NULL,
    intent_id UUID NOT NULL,
    solver_id TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    status execution_status NOT NULL DEFAULT 'pending',
    created_at BIGINT NOT NULL,
    created_ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_ts)
) PARTITION BY RANGE (created_ts);

CREATE INDEX idx_executions_p_intent ON executions (intent_id, created_ts);
CREATE INDEX idx_executions_p_status ON executions (status, created_ts);

INSERT INTO executions (id, intent_id, solver_id, tx_hash, status, created_at, created_ts)
    SELECT id, intent_id, solver_id, tx_hash, status, created_at, to_timestamp(created_at)
    FROM executions_old ON CONFLICT DO NOTHING;
DROP TABLE IF EXISTS executions_old CASCADE;

-- ============================================================
-- 6. Create initial partitions (current + next 3 months)
-- ============================================================

DO $$
DECLARE
    tbl TEXT;
    m INT;
    y INT;
    start_date DATE;
    end_date DATE;
    part_name TEXT;
BEGIN
    FOR tbl IN SELECT unnest(ARRAY['trades', 'ledger_entries', 'market_trades', 'fills', 'executions'])
    LOOP
        FOR m IN 0..3 LOOP
            start_date := date_trunc('month', NOW()) + (m || ' months')::interval;
            end_date := start_date + '1 month'::interval;
            y := EXTRACT(YEAR FROM start_date);
            part_name := tbl || '_y' || y || 'm' || LPAD(EXTRACT(MONTH FROM start_date)::TEXT, 2, '0');

            IF NOT EXISTS (
                SELECT 1 FROM pg_class WHERE relname = part_name
            ) THEN
                EXECUTE format(
                    'CREATE TABLE %I PARTITION OF %I FOR VALUES FROM (%L) TO (%L)',
                    part_name, tbl, start_date, end_date
                );
                RAISE NOTICE 'Created partition: %', part_name;
            END IF;
        END LOOP;
    END LOOP;
END $$;

-- ============================================================
-- 7. Auto-partition creation function
--    Call monthly via pg_cron or application worker
-- ============================================================

CREATE OR REPLACE FUNCTION create_monthly_partitions(months_ahead INT DEFAULT 3)
RETURNS void AS $$
DECLARE
    tbl TEXT;
    m INT;
    y INT;
    start_date DATE;
    end_date DATE;
    part_name TEXT;
BEGIN
    FOR tbl IN SELECT unnest(ARRAY['trades', 'ledger_entries', 'market_trades', 'fills', 'executions'])
    LOOP
        FOR m IN 0..months_ahead LOOP
            start_date := date_trunc('month', NOW()) + (m || ' months')::interval;
            end_date := start_date + '1 month'::interval;
            y := EXTRACT(YEAR FROM start_date);
            part_name := tbl || '_y' || y || 'm' || LPAD(EXTRACT(MONTH FROM start_date)::TEXT, 2, '0');

            IF NOT EXISTS (
                SELECT 1 FROM pg_class WHERE relname = part_name
            ) THEN
                EXECUTE format(
                    'CREATE TABLE %I PARTITION OF %I FOR VALUES FROM (%L) TO (%L)',
                    part_name, tbl, start_date, end_date
                );
                RAISE NOTICE 'Created partition: %', part_name;
            END IF;
        END LOOP;
    END LOOP;
END;
$$ LANGUAGE plpgsql;
