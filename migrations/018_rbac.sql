CREATE TABLE IF NOT EXISTS roles (
    id UUID PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS user_roles (
    user_id UUID NOT NULL REFERENCES users(id),
    role_id UUID NOT NULL REFERENCES roles(id),
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, role_id)
);

CREATE TABLE IF NOT EXISTS permissions (
    id UUID PRIMARY KEY,
    role_id UUID NOT NULL REFERENCES roles(id),
    resource TEXT NOT NULL,
    action TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (role_id, resource, action)
);

CREATE INDEX IF NOT EXISTS idx_user_roles_user ON user_roles (user_id);
CREATE INDEX IF NOT EXISTS idx_permissions_role ON permissions (role_id);

-- Seed default roles
INSERT INTO roles (id, name, description) VALUES
    ('10000000-0000-0000-0000-000000000001', 'admin', 'Full platform access'),
    ('10000000-0000-0000-0000-000000000002', 'trader', 'Can create intents, deposit, withdraw'),
    ('10000000-0000-0000-0000-000000000003', 'solver', 'Can submit bids and view auctions'),
    ('10000000-0000-0000-0000-000000000004', 'read_only', 'Can only view data')
ON CONFLICT (name) DO NOTHING;

-- Seed default permissions
INSERT INTO permissions (id, role_id, resource, action) VALUES
    -- admin: everything
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000001', '*', '*'),
    -- trader
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'intents', 'create'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'intents', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'intents', 'cancel'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'balances', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'balances', 'deposit'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'balances', 'withdraw'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'markets', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000002', 'trades', 'read'),
    -- solver
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'bids', 'create'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'bids', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'intents', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'markets', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000003', 'auctions', 'read'),
    -- read_only
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'intents', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'markets', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'trades', 'read'),
    (gen_random_uuid(), '10000000-0000-0000-0000-000000000004', 'balances', 'read')
ON CONFLICT (role_id, resource, action) DO NOTHING;
