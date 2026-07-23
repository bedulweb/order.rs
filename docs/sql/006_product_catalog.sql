-- Product catalog (ART/SKU → name + HPP IDR).
-- Apply once:
--   psql "$DATABASE_URL" -f docs/sql/006_product_catalog.sql
--
-- Join key: order_items.sku (trimmed) = product_catalog.art

CREATE TABLE IF NOT EXISTS product_catalog (
    art         TEXT PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT '',
    hpp         BIGINT NOT NULL CHECK (hpp >= 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS product_catalog_name_idx
    ON product_catalog (name);

CREATE INDEX IF NOT EXISTS product_catalog_updated_at_idx
    ON product_catalog (updated_at DESC);
