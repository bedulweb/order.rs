-- Ops pick/list batches (membership is source of truth for "already processed").
-- Apply once:
--   psql "$DATABASE_URL" -f docs/sql/005_batches.sql
--
-- Backlog = orders.state = 'new' AND NOT EXISTS (
--   SELECT 1 FROM batch_orders bo
--   WHERE bo.order_id = orders.id AND bo.voided_at IS NULL
-- )

CREATE TABLE IF NOT EXISTS batches (
    id              UUID PRIMARY KEY,
    account_id      BIGINT REFERENCES bs_accounts (id),
    session         TEXT NOT NULL
                    CHECK (session IN ('morning', 'afternoon', 'urgent')),
    timezone        TEXT NOT NULL DEFAULT 'Asia/Jakarta',
    status          TEXT NOT NULL DEFAULT 'ready'
                    CHECK (status IN ('ready', 'failed')),
    order_count     INT NOT NULL DEFAULT 0,
    urgent_count    INT NOT NULL DEFAULT 0,
    pdf_bytes       BYTEA,
    pdf_filename    TEXT,
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS batches_created_at_idx ON batches (created_at DESC);
CREATE INDEX IF NOT EXISTS batches_account_created_idx
    ON batches (account_id, created_at DESC);

CREATE TABLE IF NOT EXISTS batch_orders (
    batch_id            UUID NOT NULL REFERENCES batches (id) ON DELETE CASCADE,
    order_id            BIGINT NOT NULL REFERENCES orders (id),
    platform_order_id   TEXT NOT NULL,
    platform            TEXT,
    carrier_snapshot    TEXT,
    is_urgent           BOOLEAN NOT NULL DEFAULT false,
    position            INT NOT NULL DEFAULT 0,
    voided_at           TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (batch_id, order_id)
);

CREATE INDEX IF NOT EXISTS batch_orders_order_id_idx ON batch_orders (order_id);

-- At most one active (non-voided) membership per order.
CREATE UNIQUE INDEX IF NOT EXISTS batch_orders_active_order_uid
    ON batch_orders (order_id)
    WHERE voided_at IS NULL;
