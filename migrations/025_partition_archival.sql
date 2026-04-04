-- Track archived partitions for audit trail
CREATE TABLE IF NOT EXISTS partition_archive_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    table_name TEXT NOT NULL,
    partition_name TEXT NOT NULL,
    row_count BIGINT NOT NULL DEFAULT 0,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Function: detach and drop partitions older than retention_months.
-- Returns number of partitions archived.
CREATE OR REPLACE FUNCTION archive_old_partitions(retention_months INT DEFAULT 6)
RETURNS INT AS $$
DECLARE
    tbl TEXT;
    part RECORD;
    cutoff DATE;
    archived INT := 0;
    cnt BIGINT;
BEGIN
    cutoff := date_trunc('month', NOW()) - (retention_months || ' months')::interval;

    FOR tbl IN SELECT unnest(ARRAY['trades', 'fills', 'executions', 'ledger_entries', 'market_trades'])
    LOOP
        FOR part IN
            SELECT c.relname AS partition_name,
                   pg_get_expr(c.relpartbound, c.oid) AS bound_expr
            FROM pg_inherits i
            JOIN pg_class c ON c.oid = i.inhrelid
            WHERE i.inhparent = tbl::regclass
            ORDER BY c.relname
        LOOP
            -- Extract the FROM date from the partition bound
            -- Format: "FOR VALUES FROM ('2025-01-01') TO ('2025-02-01')"
            DECLARE
                from_date DATE;
            BEGIN
                from_date := (regexp_match(part.bound_expr, 'FROM \(''([^'']+)''\)'))[1]::DATE;

                IF from_date < cutoff THEN
                    -- Count rows before archival
                    EXECUTE format('SELECT COUNT(*) FROM %I', part.partition_name) INTO cnt;

                    -- Log the archival
                    INSERT INTO partition_archive_log (table_name, partition_name, row_count)
                    VALUES (tbl, part.partition_name, cnt);

                    -- Detach the partition (instant, no lock on parent)
                    EXECUTE format('ALTER TABLE %I DETACH PARTITION %I', tbl, part.partition_name);

                    -- Drop the detached table
                    EXECUTE format('DROP TABLE %I', part.partition_name);

                    RAISE NOTICE 'Archived: % (% rows)', part.partition_name, cnt;
                    archived := archived + 1;
                END IF;
            EXCEPTION
                WHEN OTHERS THEN
                    RAISE WARNING 'Failed to archive %: %', part.partition_name, SQLERRM;
            END;
        END LOOP;
    END LOOP;

    RETURN archived;
END;
$$ LANGUAGE plpgsql;
