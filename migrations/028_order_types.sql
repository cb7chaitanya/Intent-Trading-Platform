DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'order_type') THEN
        CREATE TYPE order_type AS ENUM ('market', 'limit', 'stop');
    END IF;
END $$;

ALTER TABLE intents ADD COLUMN IF NOT EXISTS order_type order_type NOT NULL DEFAULT 'market';
ALTER TABLE intents ADD COLUMN IF NOT EXISTS limit_price BIGINT;
ALTER TABLE intents ADD COLUMN IF NOT EXISTS stop_price BIGINT;

-- Pending stop orders for the monitor worker
CREATE INDEX IF NOT EXISTS idx_intents_stop_pending
    ON intents (stop_price)
    WHERE order_type = 'stop' AND status = 'open';
