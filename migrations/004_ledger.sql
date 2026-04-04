DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'entry_type') THEN
        CREATE TYPE entry_type AS ENUM ('DEBIT', 'CREDIT');
    END IF;
END $$;

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'reference_type') THEN
        CREATE TYPE reference_type AS ENUM ('TRADE', 'DEPOSIT', 'WITHDRAWAL', 'FEE');
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS ledger_entries (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id),
    asset asset_type NOT NULL,
    amount BIGINT NOT NULL,
    entry_type entry_type NOT NULL,
    reference_type reference_type NOT NULL,
    reference_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ledger_account_id ON ledger_entries (account_id);
CREATE INDEX IF NOT EXISTS idx_ledger_reference_id ON ledger_entries (reference_id);
CREATE INDEX IF NOT EXISTS idx_ledger_created_at ON ledger_entries (created_at);
