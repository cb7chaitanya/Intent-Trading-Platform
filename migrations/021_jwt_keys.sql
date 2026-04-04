CREATE TABLE IF NOT EXISTS jwt_keys (
    id UUID PRIMARY KEY,
    key_secret TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_jwt_keys_active ON jwt_keys (active) WHERE active = TRUE;
