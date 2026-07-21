-- Optional helpers for sync worker (PostgreSQL)

-- Parse BigSeller epoch that may be ms or seconds.
CREATE OR REPLACE FUNCTION bs_ts(n bigint)
RETURNS timestamptz
LANGUAGE sql
IMMUTABLE
AS $$
  SELECT CASE
    WHEN n IS NULL THEN NULL
    WHEN n > 1000000000000 THEN to_timestamp(n / 1000.0)  -- ms
    WHEN n > 1000000000    THEN to_timestamp(n)           -- sec
    ELSE NULL
  END;
$$;

-- Parse money strings from API ("104848", "0.00", "").
CREATE OR REPLACE FUNCTION bs_money(s text)
RETURNS numeric
LANGUAGE sql
IMMUTABLE
AS $$
  SELECT CASE
    WHEN s IS NULL OR btrim(s) = '' THEN NULL
    ELSE s::numeric
  END;
$$;
