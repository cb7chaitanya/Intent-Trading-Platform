-- Add PartiallyFilled status to intent_status enum
ALTER TYPE intent_status ADD VALUE IF NOT EXISTS 'partiallyfilled';

-- Recreate fills table to support multiple fills per intent
DROP TABLE IF EXISTS fills;

CREATE TABLE fills (
    id UUID PRIMARY KEY,
    intent_id UUID NOT NULL REFERENCES intents(id),
    solver_id TEXT NOT NULL,
    price BIGINT NOT NULL,
    qty BIGINT NOT NULL,
    filled_qty BIGINT NOT NULL,
    tx_hash TEXT NOT NULL DEFAULT '',
    timestamp BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_fills_intent_id ON fills (intent_id);
CREATE INDEX IF NOT EXISTS idx_fills_solver_id ON fills (solver_id);
