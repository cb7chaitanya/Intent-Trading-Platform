CREATE TABLE IF NOT EXISTS fills (
    intent_id UUID PRIMARY KEY REFERENCES intents(id),
    solver_id TEXT NOT NULL,
    price BIGINT NOT NULL,
    qty BIGINT NOT NULL,
    tx_hash TEXT NOT NULL DEFAULT '',
    timestamp BIGINT NOT NULL
);
