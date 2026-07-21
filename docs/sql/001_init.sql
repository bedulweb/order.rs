-- BigSeller orders — PostgreSQL schema v1 (long-term hybrid model)
-- See docs/sql-model.md for field mapping rationale.
--
-- Requires: PostgreSQL 14+

BEGIN;

CREATE EXTENSION IF NOT EXISTS pgcrypto; -- gen_random_uuid if needed later

-- ---------------------------------------------------------------------------
-- Account / session (sync worker only)
-- ---------------------------------------------------------------------------

CREATE TABLE bs_accounts (
    id              bigserial PRIMARY KEY,
    login_account   text NOT NULL UNIQUE,          -- email / phone used at login
    display_name    text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE bs_sessions (
    account_id      bigint NOT NULL REFERENCES bs_accounts (id) ON DELETE CASCADE,
    cookies         jsonb NOT NULL DEFAULT '{}'::jsonb,  -- muc_token, JSESSIONID, ...
    access_token    text,
    is_valid        boolean NOT NULL DEFAULT true,
    last_login_at   timestamptz,
    last_check_at   timestamptz,                   -- last isLogin probe
    updated_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id)
);

COMMENT ON TABLE bs_sessions IS 'Restricted: sync worker role only; never expose via web API';

-- ---------------------------------------------------------------------------
-- Shops
-- ---------------------------------------------------------------------------

CREATE TABLE shops (
    id              bigint PRIMARY KEY,            -- BigSeller shopId
    account_id      bigint REFERENCES bs_accounts (id) ON DELETE SET NULL,
    platform        text NOT NULL,                 -- shopee, tiktok, manual, ...
    name            text NOT NULL,
    site            text,                          -- ID, ...
    status          integer,
    payload         jsonb NOT NULL DEFAULT '{}'::jsonb,
    synced_at       timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX shops_platform_idx ON shops (platform);
CREATE INDEX shops_account_idx ON shops (account_id);

-- ---------------------------------------------------------------------------
-- Orders (typed columns + full API row in payload)
-- ---------------------------------------------------------------------------

CREATE TABLE orders (
    id                  bigint PRIMARY KEY,        -- BigSeller order id
    account_id          bigint REFERENCES bs_accounts (id) ON DELETE SET NULL,
    shop_id             bigint NOT NULL REFERENCES shops (id),

    platform            text NOT NULL,
    platform_order_id   text NOT NULL,
    package_no          text,
    package_index       text,

    state               text NOT NULL,             -- new, shipped, canceled, ...
    platform_state      text,
    view_status         text,
    marketplace_state   text,
    last_order_status   text,

    amount              numeric(18, 2),
    currency            text,                      -- IDR
    payment_method      text,

    buyer_username      text,
    contact_person      text,
    recipient_region    text,
    buyer_message       text,
    seller_note         text,

    tracking_no         text,
    tracking_url        text,
    shipment_provider   text,
    shipping_carrier_id bigint,
    shipping_carrier_name text,
    buyer_shipping_carrier text,
    shipping_config_option_id integer,
    shipping_config_option_name text,

    warehouse_id        bigint,
    warehouse_name      text,
    store_site          text,

    pack_state          smallint,
    item_total_num      integer,
    print_label_mark    smallint,
    print_bill_mark     smallint,
    print_pick_list_mark smallint,
    print_collect_mark  smallint,

    has_error           boolean NOT NULL DEFAULT false,
    error_msg           text,

    ordered_at          timestamptz,
    paid_at             timestamptz,
    ship_by_at          timestamptz,               -- from shippedTime (often SLA on new)
    completed_at        timestamptz,
    deadline_at         timestamptz,
    timeout_at          timestamptz,
    printed_collect_at  timestamptz,

    -- full BigSeller row (214+ fields); source blob for forward-compat
    payload             jsonb NOT NULL,
    payload_hash        bytea,

    first_seen_at       timestamptz NOT NULL DEFAULT now(),
    synced_at           timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT orders_shop_platform_order_uid
        UNIQUE (shop_id, platform_order_id)
);

CREATE INDEX orders_state_ordered_idx
    ON orders (state, ordered_at DESC NULLS LAST);

CREATE INDEX orders_shop_ordered_idx
    ON orders (shop_id, ordered_at DESC NULLS LAST);

CREATE INDEX orders_platform_ordered_idx
    ON orders (platform, ordered_at DESC NULLS LAST);

CREATE INDEX orders_tracking_idx
    ON orders (tracking_no)
    WHERE tracking_no IS NOT NULL AND tracking_no <> '';

CREATE INDEX orders_buyer_idx
    ON orders (buyer_username);

CREATE INDEX orders_package_idx
    ON orders (package_no);

CREATE INDEX orders_synced_idx
    ON orders (synced_at DESC);

-- Optional power-user search later:
-- CREATE INDEX orders_payload_gin ON orders USING gin (payload jsonb_path_ops);

COMMENT ON COLUMN orders.ship_by_at IS
    'Mapped from API shippedTime; on state=new this is often ship-by deadline, not actual ship time';
COMMENT ON COLUMN orders.payload IS
    'Complete pageList row JSON from BigSeller; typed columns are projections';

-- ---------------------------------------------------------------------------
-- Order line items
-- ---------------------------------------------------------------------------

CREATE TABLE order_items (
    id                      bigint PRIMARY KEY,    -- BigSeller line id
    order_id                bigint NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    line_no                 integer NOT NULL DEFAULT 0,

    sku                     text,
    variant_attr            text,
    item_name               text,
    quantity                integer NOT NULL DEFAULT 1,
    amount                  numeric(18, 2),
    unit_price              numeric(18, 2),
    original_price          numeric(18, 2),

    image_url               text,
    product_url             text,
    platform_item_id        text,
    platform_variation_id   text,
    inventory_sku           text,
    is_addition             boolean NOT NULL DEFAULT false,
    product_type            integer,

    payload                 jsonb NOT NULL DEFAULT '{}'::jsonb,
    synced_at               timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT order_items_order_line_uid UNIQUE (order_id, line_no)
);

CREATE INDEX order_items_order_idx ON order_items (order_id);
CREATE INDEX order_items_sku_idx ON order_items (sku);
CREATE INDEX order_items_platform_item_idx ON order_items (platform_item_id);

-- ---------------------------------------------------------------------------
-- Optional: normalized fees (v1 can skip and read payload->feeDetail)
-- ---------------------------------------------------------------------------

CREATE TABLE order_fees (
    order_id                bigint PRIMARY KEY REFERENCES orders (id) ON DELETE CASCADE,
    total_amount            numeric(18, 2),
    total_product_price     numeric(18, 2),
    estimated_profit        numeric(18, 2),
    commission_fee          numeric(18, 2),
    service_fee             numeric(18, 2),
    estimated_shipping_fee  numeric(18, 2),
    shipping_rebate         numeric(18, 2),
    voucher_from            numeric(18, 2),
    profit_rate             numeric(8, 4),
    other_fee_info          jsonb NOT NULL DEFAULT '{}'::jsonb,
    payload                 jsonb NOT NULL DEFAULT '{}'::jsonb,
    synced_at               timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- Status history (long-term timeline for web UI)
-- ---------------------------------------------------------------------------

CREATE TABLE order_status_history (
    id              bigserial PRIMARY KEY,
    order_id        bigint NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    from_state      text,
    to_state        text NOT NULL,
    changed_at      timestamptz NOT NULL DEFAULT now(),
    source          text NOT NULL DEFAULT 'sync'  -- sync | manual | api
);

CREATE INDEX order_status_history_order_idx
    ON order_status_history (order_id, changed_at DESC);

-- ---------------------------------------------------------------------------
-- Sync control plane
-- ---------------------------------------------------------------------------

CREATE TABLE sync_runs (
    id              bigserial PRIMARY KEY,
    account_id      bigint REFERENCES bs_accounts (id) ON DELETE SET NULL,
    kind            text NOT NULL,                 -- orders_status:new | shops | full
    status          text NOT NULL DEFAULT 'running', -- running | ok | error
    started_at      timestamptz NOT NULL DEFAULT now(),
    finished_at     timestamptz,
    pages_fetched   integer NOT NULL DEFAULT 0,
    rows_upserted   integer NOT NULL DEFAULT 0,
    error_text      text,
    meta            jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX sync_runs_started_idx ON sync_runs (started_at DESC);

CREATE TABLE sync_cursors (
    key             text PRIMARY KEY,
    value           jsonb NOT NULL DEFAULT '{}'::jsonb,
    updated_at      timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- Convenience view for web list screens
-- ---------------------------------------------------------------------------

CREATE OR REPLACE VIEW v_orders_list AS
SELECT
    o.id,
    o.shop_id,
    s.name AS shop_name,
    o.platform,
    o.platform_order_id,
    o.package_no,
    o.state,
    o.view_status,
    o.marketplace_state,
    o.amount,
    o.currency,
    o.payment_method,
    o.buyer_username,
    o.tracking_no,
    o.shipment_provider,
    o.item_total_num,
    o.has_error,
    o.ordered_at,
    o.paid_at,
    o.ship_by_at,
    o.synced_at
FROM orders o
JOIN shops s ON s.id = o.shop_id;

COMMIT;
