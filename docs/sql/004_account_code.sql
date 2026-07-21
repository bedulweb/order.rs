-- Multi-account ready: stable code slug on bs_accounts + indexes.
-- Single-tenant today still uses one row (code=default or BS_ACCOUNT_CODE).

BEGIN;

ALTER TABLE bs_accounts
    ADD COLUMN IF NOT EXISTS code text;

-- Backfill existing rows
UPDATE bs_accounts
SET code = 'default'
WHERE code IS NULL OR btrim(code) = '';

-- Unique code (nullable only for legacy; we force non-null after backfill)
ALTER TABLE bs_accounts
    ALTER COLUMN code SET DEFAULT 'default';

UPDATE bs_accounts SET code = 'default' WHERE code IS NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bs_accounts_code_key'
    ) THEN
        ALTER TABLE bs_accounts ADD CONSTRAINT bs_accounts_code_key UNIQUE (code);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS orders_account_id_idx ON orders (account_id);
CREATE INDEX IF NOT EXISTS orders_account_platform_order_idx
    ON orders (account_id, platform_order_id);
CREATE INDEX IF NOT EXISTS shops_account_id_idx ON shops (account_id);

ALTER TABLE notification_outbox
    ADD COLUMN IF NOT EXISTS account_id bigint REFERENCES bs_accounts (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS notification_outbox_account_idx
    ON notification_outbox (account_id, id);

COMMENT ON COLUMN bs_accounts.code IS
    'Stable slug for API/config (e.g. bs-a, default). Not the BigSeller login email.';

COMMIT;
