-- Solver onboarding: add registration fields to solvers table
ALTER TABLE solvers ADD COLUMN IF NOT EXISTS email TEXT;
ALTER TABLE solvers ADD COLUMN IF NOT EXISTS api_key TEXT;
ALTER TABLE solvers ADD COLUMN IF NOT EXISTS webhook_url TEXT;
ALTER TABLE solvers ADD COLUMN IF NOT EXISTS active BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE solvers ADD COLUMN IF NOT EXISTS total_fills BIGINT NOT NULL DEFAULT 0;
ALTER TABLE solvers ADD COLUMN IF NOT EXISTS failed_fills BIGINT NOT NULL DEFAULT 0;

-- Unique constraints
CREATE UNIQUE INDEX IF NOT EXISTS idx_solvers_api_key ON solvers (api_key) WHERE api_key IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_solvers_email ON solvers (email) WHERE email IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_solvers_name ON solvers (name);

-- Backfill total_fills from successful_trades for existing rows
UPDATE solvers SET total_fills = successful_trades WHERE total_fills = 0 AND successful_trades > 0;
UPDATE solvers SET failed_fills = failed_trades WHERE failed_fills = 0 AND failed_trades > 0;
