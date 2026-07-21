# order.rs

Rust service that syncs [BigSeller](https://www.bigseller.com) orders into PostgreSQL (Neon) and exposes a small internal HTTP API for consumers (for example a loyalty app).

BigSeller is only contacted by the **worker**. The HTTP API reads Postgres only.

## Architecture

```text
BigSeller REST
      |
      v
 orders worker  ----UPSERT---->  Neon Postgres
                                      |
                                      v
                               orders serve (HTTP)
                                      |
                                      v
                               internal consumers
```

| Process | Command | Role |
|---------|---------|------|
| Worker | `orders worker` | Login/session, pull `pageList`, upsert rows, outbox / optional webhook |
| API | `orders serve` | Lookup, events, cancel report (Postgres only) |
| One-shot | `orders sync` | Manual cache fill |
| Health | `orders doctor` | Env, DB, session, `isLogin` |

Multi-account: one Neon database; tenant slug is `bs_accounts.code` (`BS_ACCOUNT_CODE`, default `default`). Orders store `account_id`.

## Repository layout

```text
src/
  main.rs       CLI
  api.rs        HTTP (axum)
  sync.rs       worker + pageList pull
  store.rs      SQL upsert / reads
  map.rs        JSON row -> columns
  accounts.rs   bs_accounts / bs_sessions
  orders.rs     BigSeller client
  auth.rs       captcha login
  ocr.rs        ONNX CTC captcha
  ...
docs/
  public-api.md     HTTP contract
  api-map.md        BigSeller reverse notes
  sql/              Postgres migrations
  secrets.md        Infisical / credentials
scripts/
  fetch-model.sh
  push-secrets-infisical.sh
```

## Prerequisites

- Rust 1.75+
- PostgreSQL (Neon URL)
- OCR model: `models/common_old.onnx` (see `scripts/fetch-model.sh`)
- Credentials via **Infisical** (preferred) or local `.env` (gitignored)

## Secrets (Infisical)

Do not commit `.env` or session files. Store secrets in Infisical and inject at runtime:

```bash
# once: link project (creates .infisical.json)
infisical init

# upload current local .env into Infisical (dev)
./scripts/push-secrets-infisical.sh

# run with secrets injected
infisical run --env=dev -- ./target/release/orders doctor
infisical run --env=dev -- ./target/release/orders worker
infisical run --env=dev -- ./target/release/orders serve
```

Details: [docs/secrets.md](docs/secrets.md). Template keys: [.env.example](.env.example).

Required secret names:

| Name | Purpose |
|------|---------|
| `BS_ACCOUNT` | BigSeller login |
| `BS_PASSWORD` | BigSeller password |
| `DATABASE_URL` | Neon Postgres |
| `API_TOKEN` | Bearer token for HTTP API |
| `BS_ACCOUNT_CODE` | Tenant slug (default `default`) |

Optional: `API_BIND`, `SYNC_NEW_INTERVAL_SECS`, `CANCEL_HOUR_LOCAL`, `AUTO_RELOGIN`, `WA_WEBHOOK_URL`, `WA_WEBHOOK_TOKEN`.

## Setup

```bash
git clone https://github.com/bedulweb/order.rs.git
cd order.rs

./scripts/fetch-model.sh
cp .env.example .env   # or: infisical export --env=dev > .env

# apply SQL once (Neon)
psql "$DATABASE_URL" -f docs/sql/001_init.sql
psql "$DATABASE_URL" -f docs/sql/002_helpers.sql
psql "$DATABASE_URL" -f docs/sql/003_outbox_and_indexes.sql
psql "$DATABASE_URL" -f docs/sql/004_account_code.sql

cargo build --release
```

## Usage

```bash
./target/release/orders doctor
./target/release/orders login
./target/release/orders sync --status new
./target/release/orders sync --status cancel
./target/release/orders sync --status all

# long-running (two processes)
./target/release/orders worker
./target/release/orders serve

# debug against BigSeller
./target/release/orders list --status new
./target/release/orders counts
./target/release/orders status
```

## HTTP API (summary)

Base: `http://127.0.0.1:8080` (`API_BIND`).  
Auth: `Authorization: Bearer <API_TOKEN>` or `X-Api-Key: <API_TOKEN>`.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Liveness + DB ping |
| GET | `/v1/sync/status` | Recent sync runs, order count |
| GET | `/v1/orders/by-platform-id/{id}` | Lookup by marketplace order id |
| GET | `/v1/orders/events` | Outbox cursor (`?since=&limit=`) |
| GET | `/v1/reports/in-cancel/daily` | Daily cancel + print summary |

Full contract: [docs/public-api.md](docs/public-api.md).

Example:

```http
GET /v1/orders/by-platform-id/2607206K6S67BG?account=default
Authorization: Bearer <API_TOKEN>
```

## Worker behaviour

- Every `SYNC_NEW_INTERVAL_SECS` (default 60): pull `status=new`, upsert, enqueue `order.created` on first see.
- Once per local day at `CANCEL_HOUR_LOCAL`:`CANCEL_MINUTE_LOCAL` (default 17:00): pull cancel-related buckets.
- On BigSeller auth expiry (code `2001`): auto re-login when `AUTO_RELOGIN=true`.
- Optional: POST outbox events to `WA_WEBHOOK_URL`.

## Notes

- Captcha rate limits apply; space out logins.
- Session file (`.session.json`) is local only; also mirrored to `bs_sessions` when DB is configured.
- Money fields are stored as `numeric`; timestamps as `timestamptz`.
- Do not commit `.env`, `.session.json`, or `models/*.onnx`.
