-- Add chain field to markets for multi-chain settlement routing.
ALTER TABLE markets ADD COLUMN IF NOT EXISTS chain TEXT NOT NULL DEFAULT 'ethereum';
