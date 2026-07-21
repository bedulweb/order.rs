-- Outbox for "new order" notifications + lookup helpers
-- Safe to re-run on Neon after 001/002.

BEGIN;

CREATE TABLE IF NOT EXISTS notification_outbox (
    id              bigserial PRIMARY KEY,
    event_type      text NOT NULL,                 -- order.created | order.state_changed
    order_id        bigint REFERENCES orders (id) ON DELETE SET NULL,
    platform_order_id text,
    payload         jsonb NOT NULL DEFAULT '{}'::jsonb,
    status          text NOT NULL DEFAULT 'pending', -- pending | sent | failed | skipped
    attempts        integer NOT NULL DEFAULT 0,
    last_error      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    sent_at         timestamptz,
    available_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS notification_outbox_pending_idx
    ON notification_outbox (status, available_at)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS notification_outbox_created_idx
    ON notification_outbox (created_at DESC);

-- Fast lookup by marketplace order number (may span shops; API disambiguates).
CREATE INDEX IF NOT EXISTS orders_platform_order_id_idx
    ON orders (platform_order_id);

-- Cancel / in-cancel style filters for evening reports.
CREATE INDEX IF NOT EXISTS orders_state_in_cancel_idx
    ON orders (state, ordered_at DESC)
    WHERE state IN ('canceled', 'cancelled')
       OR COALESCE((payload->>'inCancel')::boolean, false) = true
       OR COALESCE(payload->>'inCancel', '') IN ('1', 'true', 'True');

-- Print marks for "sudah dicetak" summaries.
CREATE INDEX IF NOT EXISTS orders_print_collect_idx
    ON orders (print_collect_mark)
    WHERE print_collect_mark IS NOT NULL AND print_collect_mark <> 0;

COMMIT;
