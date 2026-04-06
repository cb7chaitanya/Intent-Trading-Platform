-- Hash Time-Locked Contracts for cross-chain atomic swaps.
--
-- Flow:
-- 1. Platform generates secret + hash
-- 2. User's funds locked on source chain with hash (HTLC-lock)
-- 3. Solver claims on destination chain by revealing secret
-- 4. Platform uses revealed secret to unlock source chain funds
-- 5. If timeout expires before claim, user gets refund

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'htlc_status') THEN
        CREATE TYPE htlc_status AS ENUM (
            'created',
            'source_locked',
            'dest_claimed',
            'source_unlocked',
            'refunded',
            'expired',
            'failed'
        );
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS htlc_swaps (
    id UUID PRIMARY KEY,
    fill_id UUID NOT NULL,
    intent_id UUID NOT NULL REFERENCES intents(id),

    -- Secret management
    secret_hash BYTEA NOT NULL,          -- SHA-256(secret), 32 bytes
    secret BYTEA,                        -- revealed after dest claim, 32 bytes

    -- Source chain (user locks funds here)
    source_chain TEXT NOT NULL,
    source_sender TEXT NOT NULL,
    source_receiver TEXT NOT NULL,        -- solver's address on source chain
    source_token TEXT,
    source_amount BIGINT NOT NULL,
    source_lock_tx TEXT,                  -- tx that locked the HTLC on source
    source_unlock_tx TEXT,               -- tx that unlocked with the secret
    source_timelock TIMESTAMPTZ NOT NULL, -- after this, user can refund

    -- Destination chain (solver releases funds here)
    dest_chain TEXT NOT NULL,
    dest_sender TEXT NOT NULL,            -- solver's address on dest chain
    dest_receiver TEXT NOT NULL,          -- user's address on dest chain
    dest_token TEXT,
    dest_amount BIGINT NOT NULL,
    dest_lock_tx TEXT,                   -- solver's HTLC lock on dest chain
    dest_claim_tx TEXT,                  -- user/platform claims with secret

    -- Lifecycle
    status htlc_status NOT NULL DEFAULT 'created',
    solver_id TEXT NOT NULL,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_at TIMESTAMPTZ,
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,

    UNIQUE (fill_id)
);

CREATE INDEX IF NOT EXISTS idx_htlc_status ON htlc_swaps (status)
    WHERE status NOT IN ('source_unlocked', 'refunded', 'expired');
CREATE INDEX IF NOT EXISTS idx_htlc_timelock ON htlc_swaps (source_timelock)
    WHERE status IN ('created', 'source_locked', 'dest_claimed');
CREATE INDEX IF NOT EXISTS idx_htlc_intent ON htlc_swaps (intent_id);
