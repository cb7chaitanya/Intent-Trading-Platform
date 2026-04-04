DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'intent_status') THEN
        CREATE TYPE intent_status AS ENUM (
            'open', 'bidding', 'matched', 'executing', 'completed', 'failed', 'cancelled'
        );
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS intents (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    token_in TEXT NOT NULL,
    token_out TEXT NOT NULL,
    amount_in BIGINT NOT NULL,
    min_amount_out BIGINT NOT NULL,
    deadline BIGINT NOT NULL,
    status intent_status NOT NULL DEFAULT 'open',
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_intents_user_id ON intents (user_id);
CREATE INDEX IF NOT EXISTS idx_intents_status ON intents (status);
CREATE INDEX IF NOT EXISTS idx_intents_created_at ON intents (created_at);
