-- Add stop_side to distinguish stop-loss from stop-buy
-- sell = trigger when price <= stop_price (stop-loss)
-- buy  = trigger when price >= stop_price (stop-buy)
ALTER TABLE intents ADD COLUMN IF NOT EXISTS stop_side TEXT DEFAULT 'sell';

-- Add triggered_at to track when a stop was triggered (trigger-once guarantee)
ALTER TABLE intents ADD COLUMN IF NOT EXISTS triggered_at TIMESTAMPTZ;

-- Stop-limit: if limit_price is set alongside stop_price, convert to limit on trigger
-- (no schema change needed — limit_price column already exists)

-- Improve stop order index
DROP INDEX IF EXISTS idx_intents_stop_pending;
CREATE INDEX IF NOT EXISTS idx_intents_stop_pending
    ON intents (stop_price, stop_side)
    WHERE order_type = 'stop' AND status = 'open' AND triggered_at IS NULL;
