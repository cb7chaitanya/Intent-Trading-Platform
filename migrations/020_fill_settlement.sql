ALTER TABLE fills ADD COLUMN IF NOT EXISTS settled BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE fills ADD COLUMN IF NOT EXISTS settled_at TIMESTAMPTZ;

-- Allow failed_settlements to reference a fill_id instead of only trade_id
ALTER TABLE failed_settlements ADD COLUMN IF NOT EXISTS fill_id UUID;

CREATE INDEX IF NOT EXISTS idx_fills_unsettled
    ON fills (intent_id) WHERE settled = FALSE;
