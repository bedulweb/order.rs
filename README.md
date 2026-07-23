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
  secrets.md        credentials (.env)
  sql/              Postgres migrations
scripts/
  fetch-model.sh
```

## Prerequisites

- Rust 1.75+
- PostgreSQL (Neon URL)
- OCR model: `models/common_old.onnx` (see `scripts/fetch-model.sh`)
- Local **`.env`** (gitignored) — see [.env.example](.env.example) and [docs/secrets.md](docs/secrets.md)

## Setup

```bash
git clone https://github.com/bedulweb/order.rs.git
cd order.rs

./scripts/fetch-model.sh
cp .env.example .env
# fill BS_ACCOUNT, BS_PASSWORD, DATABASE_URL, API_TOKEN

# apply SQL once (Neon)
psql "$DATABASE_URL" -f docs/sql/001_init.sql
psql "$DATABASE_URL" -f docs/sql/002_helpers.sql
psql "$DATABASE_URL" -f docs/sql/003_outbox_and_indexes.sql
psql "$DATABASE_URL" -f docs/sql/004_account_code.sql
psql "$DATABASE_URL" -f docs/sql/005_batches.sql

cargo build --release

# Ops web UI (optional local)
cd web && npm ci && npm run build
# deploy dist to /var/www/orders-ui (see deploy/nginx-rs.obayito.com.conf)
```

## Usage

```bash
./target/release/orders doctor
./target/release/orders login
./target/release/orders sync --status new
./target/release/orders sync --status cancel
./target/release/orders sync --status all
# full historical backfill (completed/shipped/canceled/… → Postgres)
./target/release/orders sync --status history

# long-running (two processes)
./target/release/orders worker
./target/release/orders serve

# debug against BigSeller
./target/release/orders list --status new
./target/release/orders counts
./target/release/orders status

# screenshot → marketplace order id (ocrs; models cached in ~/.cache/ocrs)
./target/release/orders extract-order-id path/to/shopee.jpeg
# captcha debug only (ddddocr ONNX — not for screenshots)
./target/release/orders ocr path/to/captcha.png
```

## HTTP API (summary)

Base (local): `http://127.0.0.1:8080` (`API_BIND`).
Public (nginx): `https://rs.obayito.com` — see [`deploy/`](deploy/) (`setup-rs.obayito.com.sh`).
Auth: `Authorization: Bearer <API_TOKEN>` or `X-Api-Key: <API_TOKEN>`.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Liveness + DB ping |
| GET | `/v1/sync/status` | Recent sync runs, order count |
| GET | `/v1/orders/by-platform-id/{id}` | Lookup by marketplace order id |
| **POST** | **`/v1/app/lookup/text`** | App: nomor pesanan (JSON) |
| **POST** | **`/v1/app/lookup/photo`** | App: upload screenshot → OCR → lookup |
| GET | `/v1/orders/events` | Outbox cursor (`?since=&limit=`) |
| GET | `/v1/reports/in-cancel/daily` | Daily cancel + print summary |
| GET | `/v1/batches/backlog` | Ops: eligible pick backlog + urgent flags |
| POST | `/v1/batches` | Ops: `{ "session": "morning"\|"afternoon"\|"urgent" }` → batch + PDF |
| GET | `/v1/batches?date=YYYY-MM-DD` | Ops: list batches for a WIB day |
| GET | `/v1/batches/{id}` | Ops: batch detail + members |
| GET | `/v1/batches/{id}/pdf` | Ops: `application/pdf` reprint |

Auth: `Authorization: Bearer <API_TOKEN>` or `X-Api-Key: <API_TOKEN>`.

Public site: SPA on `/` (ops UI), API on `/v1/*` and `/health` — see [`deploy/nginx-rs.obayito.com.conf`](deploy/nginx-rs.obayito.com.conf).

### App lookup (text)

```bash
curl -sS -X POST https://rs.obayito.com/v1/app/lookup/text \
  -H "Authorization: Bearer $API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"platformOrderId":"260715PS7HRGC0"}'
```

### App lookup (photo)

```bash
curl -sS -X POST https://rs.obayito.com/v1/app/lookup/photo \
  -H "Authorization: Bearer $API_TOKEN" \
  -F "image=@/path/to/screenshot.jpeg"
```

**Found** (`found: true`): `order` includes `items` (what they bought), amounts, platform, state.
**Not found** (`found: false`, HTTP 200):

```json
{
  "ok": true,
  "found": false,
  "message": "Maaf nomor pesanan tidak ditemukan harap periksa kembali."
}
```

Show `message` as-is in the app UI.

## Worker behaviour

- Every `SYNC_NEW_INTERVAL_SECS` (default 60): pull `status=new`, upsert, enqueue `order.created` on first see.
- Once per local day at `CANCEL_HOUR_LOCAL`:`CANCEL_MINUTE_LOCAL` (default 17:00): pull cancel-related buckets.
- On BigSeller auth expiry (code `2001`): auto re-login when `AUTO_RELOGIN=true`.
- Optional: POST outbox events to `WA_WEBHOOK_URL`.

## Notes

- CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
- Captcha rate limits apply; space out logins.
- **Two OCR stacks:** captcha login uses `models/common_old.onnx` (`orders ocr` / login); screenshot order ids use Rust **ocrs** (`orders extract-order-id`, models in `~/.cache/ocrs`).
- Session file (`.session.json`) is local only; also mirrored to `bs_sessions` when DB is configured.
- Money fields are stored as `numeric`; timestamps as `timestamptz`.
- Do not commit `.env`, `.session.json`, or `models/*.onnx`.
