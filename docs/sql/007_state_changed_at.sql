-- Track WHEN an order last changed state, so the ops feed tabs can scope
-- Diproses/Dikirim/Selesai to "moved today" instead of the cumulative pool.
-- Apply once:
--   psql "$DATABASE_URL" -f docs/sql/007_state_changed_at.sql
-- (or: cargo run --example apply_007)

ALTER TABLE orders ADD COLUMN IF NOT EXISTS state_changed_at timestamptz;

-- Backfill best known transition times:
-- 1) processing orders claimed by a pick-list batch: batch creation is the
--    closest recorded moment to "processed" (BigSeller exposes no packTime);
UPDATE orders o
SET state_changed_at = bo.first_claim
FROM (
    SELECT order_id, min(created_at) AS first_claim
    FROM batch_orders
    WHERE voided_at IS NULL
    GROUP BY order_id
) bo
WHERE o.id = bo.order_id
  AND o.state_changed_at IS NULL
  AND o.state IN ('processing', 'pickup', 'platformProcessing');

-- 2) completed orders carry their completion time;
UPDATE orders
SET state_changed_at = completed_at
WHERE state_changed_at IS NULL
  AND state = 'completed'
  AND completed_at IS NOT NULL;

-- 3) new orders: last pull is a fine proxy;
UPDATE orders
SET state_changed_at = synced_at
WHERE state_changed_at IS NULL
  AND state = 'new';

-- 4) everything else (shipped without a timestamp, canceled, ...) stays
--    NULL: excluded from the "today" tabs until a real transition is seen.

CREATE INDEX IF NOT EXISTS orders_state_state_changed_at_idx
    ON orders (state, state_changed_at);
