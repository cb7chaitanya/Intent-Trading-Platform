CREATE TABLE IF NOT EXISTS bids (
    id UUID PRIMARY KEY,
    intent_id UUID NOT NULL REFERENCES intents(id),
    solver_id TEXT NOT NULL,
    amount_out BIGINT NOT NULL,
    fee BIGINT NOT NULL,
    timestamp BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_bids_intent_id ON bids (intent_id);
CREATE INDEX IF NOT EXISTS idx_bids_solver_id ON bids (solver_id);
