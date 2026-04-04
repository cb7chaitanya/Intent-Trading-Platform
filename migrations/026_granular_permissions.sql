-- Add granular permissions for handler-level enforcement.
-- Existing broad permissions (intents:create etc) remain valid;
-- these add the colon-separated format used by require_perm().

INSERT INTO permissions (id, role_id, resource, action) VALUES
    -- trader: intent + balance + market read + trade read
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'intent', 'create'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'intent', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'balance', 'deposit'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'balance', 'withdraw'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'balance', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'market', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'trade', 'read'),
    -- solver: bid + intent read + market read
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'bid', 'create'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'intent', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'market', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'trade', 'read'),
    -- read_only: read everything
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'intent', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'market', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'trade', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'balance', 'read')
ON CONFLICT (role_id, resource, action) DO NOTHING;
