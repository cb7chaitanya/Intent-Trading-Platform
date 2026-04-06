-- Multi-chain settlement fields on markets.
ALTER TABLE markets ADD COLUMN IF NOT EXISTS settlement_contract TEXT;
ALTER TABLE markets ADD COLUMN IF NOT EXISTS base_token_mint TEXT;
ALTER TABLE markets ADD COLUMN IF NOT EXISTS quote_token_mint TEXT;
ALTER TABLE markets ADD COLUMN IF NOT EXISTS base_decimals SMALLINT NOT NULL DEFAULT 18;
ALTER TABLE markets ADD COLUMN IF NOT EXISTS quote_decimals SMALLINT NOT NULL DEFAULT 6;
