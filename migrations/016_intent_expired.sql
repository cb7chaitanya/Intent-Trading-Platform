ALTER TYPE intent_status ADD VALUE IF NOT EXISTS 'expired';

CREATE INDEX IF NOT EXISTS idx_intents_deadline_active
    ON intents (deadline)
    WHERE status IN ('open', 'bidding');
