CREATE TABLE IF NOT EXISTS markets (
    id UUID PRIMARY KEY,
    base_asset asset_type NOT NULL,
    quote_asset asset_type NOT NULL,
    tick_size BIGINT NOT NULL,
    min_order_size BIGINT NOT NULL,
    fee_rate DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (base_asset, quote_asset)
);
