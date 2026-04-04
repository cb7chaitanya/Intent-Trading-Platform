DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'twap_status') THEN
        CREATE TYPE twap_status AS ENUM ('active', 'completed', 'cancelled', 'failed');
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS twap_intents (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    token_in TEXT NOT NULL,
    token_out TEXT NOT NULL,
    total_qty BIGINT NOT NULL,
    filled_qty BIGINT NOT NULL DEFAULT 0,
    min_price BIGINT NOT NULL DEFAULT 0,
    duration_secs BIGINT NOT NULL,
    interval_secs BIGINT NOT NULL,
    slices_total INT NOT NULL,
    slices_completed INT NOT NULL DEFAULT 0,
    status twap_status NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS twap_child_intents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    twap_id UUID NOT NULL REFERENCES twap_intents(id),
    intent_id UUID NOT NULL,
    slice_index INT NOT NULL,
    qty BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_twap_status ON twap_intents (status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_twap_children ON twap_child_intents (twap_id);
CREATE INDEX IF NOT EXISTS idx_twap_children_scheduled ON twap_child_intents (scheduled_at)
    WHERE status = 'pending';
