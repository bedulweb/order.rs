-- Complete the 007 backfill: processing/shipped/completed rows that changed
-- state before state_changed_at existed and were never batch-claimed stayed
-- NULL and vanished from the "today" tabs. For rows ORDERED today (WIB),
-- ordered_at is a safe lower bound — an order cannot be processed before it
-- is placed — so they belong in today's tabs.
--
-- Note the explicit ::timestamp cast: on this database `date AT TIME ZONE`
-- resolves through timestamptz and shifts the boundary +7h (verified:
-- boundary came out 07:00 UTC instead of 2026-07-27 17:00 UTC).
--
-- Apply: psql "$DATABASE_URL" -f docs/sql/008_backfill_state_changed_today.sql
--    or: cargo run --example apply_008

UPDATE orders
SET state_changed_at = ordered_at
WHERE state_changed_at IS NULL
  AND ordered_at IS NOT NULL
  AND ordered_at >= ((timezone('Asia/Jakarta', now()))::date)::timestamp AT TIME ZONE 'Asia/Jakarta'
  AND state IN ('processing', 'pickup', 'platformProcessing', 'shipped', 'completed');
