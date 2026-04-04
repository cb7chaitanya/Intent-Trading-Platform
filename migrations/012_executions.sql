DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'execution_status') THEN
        CREATE TYPE execution_status AS ENUM ('pending', 'executing', 'completed', 'failed');
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS executions (
    id UUID PRIMARY KEY,
    intent_id UUID NOT NULL REFERENCES intents(id),
    solver_id TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    status execution_status NOT NULL DEFAULT 'pending',
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_executions_intent_id ON executions (intent_id);
CREATE INDEX IF NOT EXISTS idx_executions_status ON executions (status);
