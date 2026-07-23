# Ops batch + web dashboard — design

**Date:** 2026-07-22
**Status:** draft for review
**Product:** `orders` (BigSeller → Postgres → internal API + ops UI)

## Goals

1. Process orders in **two sessions per day** (morning + afternoon), timezone **Asia/Jakarta (WIB)**.
2. Each session produces a **PDF list** of orders to print / take out.
3. Orders **not yet included in any batch PDF** remain in backlog and appear in the **next** session (including carry-over to next morning after afternoon).
4. **Urgent** shipping (instant / sameday / Gojek / Grab / SPX Instant / prioritas / etc.) may be processed **outside** the two session windows and must not double-appear after batched.
5. Internal web UI on **`https://rs.obayito.com`** for triggers, backlog, batch history, PDF download, and later product sales reports.
6. Existing mobile/API consumers keep working on **`/v1/*`** (no `/api` prefix, no forced API move).

## Non-goals (v1)

- Replacing BigSeller packing UI entirely.
- Multi-user RBAC / SSO (token gate is enough).
- Full analytics suite (charts v2 later).
- Migrating API paths to `/api/v1`.
- Yew, Loco, Tokio Topcoat, or server-rendered HTML-only UI.

## Stack (locked)

| Layer | Choice |
|--------|--------|
| Worker / sync / domain | Existing Rust crate `orders` |
| HTTP API | Axum `orders serve` — paths under `/v1` and `/health` |
| Web UI | **React + [coss ui](https://coss.com/ui/docs)** (Base UI + Tailwind; CLI `@coss/style`) |
| App shell | Vite SPA in repo folder `web/` |
| Timezone labels & cron | Asia/Jakarta |
| Auth (v1) | Same `API_TOKEN` as today (`Authorization: Bearer` or `X-Api-Key`) |

### Why not alternatives

- **HTML-only / HTMX:** fine for 3 buttons; weak once product reports and richer ops land.
- **Yew:** full-Rust UI; slower dashboard delivery, no coss-class component set.
- **Loco / Tokio Topcoat:** full-stack frameworks; conflict with existing Axum worker architecture; Topcoat still experimental.
- **Classic shadcn (Radix):** acceptable, but product choice is **coss ui** registry.

### coss setup (when implementing)

```bash
cd web
# new app with Vite + React + TS, Tailwind v4, then:
npx shadcn@latest init @coss/style
```

Docs: [Get Started](https://coss.com/ui/docs/get-started), [Styling](https://coss.com/ui/docs/styling).

## Domain model: processing batches

### Source of truth

**Not** wall-clock cutoffs alone, and **not** BigSeller print marks alone.

BigSeller marks (`print_collect_mark`, etc.) are noisy in our data (many `new` orders already marked). They may be shown as info later; they do **not** decide batch membership.

**Source of truth:** an order is “processed for takeout” when it is a member of an ops **batch** we created (and thus appeared on that batch’s PDF).

```text
order synced → backlog if eligible AND not in any batch
                    │
         ┌──────────┴──────────┐
         ▼                     ▼
   session batch          urgent batch
   (morning/afternoon)    (any time)
         │                     │
         └──────────┬──────────┘
                    ▼
            PDF + batch_orders rows
            order.batch_id set → leaves backlog
```

### Eligible for backlog

- Operational state still needs packing work — v1: `state = 'new'` (exclude canceled).
- `batch_id IS NULL` (never successfully included in a completed batch).
- Account scope: current `BS_ACCOUNT_CODE` / `account_id` as today.

### Sessions

| `session` | When | Contents |
|-----------|------|----------|
| `morning` | Scheduled or manual morning run | All backlog (urgent first, then oldest) |
| `afternoon` | Scheduled or manual afternoon run | Backlog accumulated since last batch |
| `urgent` | Any time (manual or optional auto-notify) | Backlog rows classified urgent only |

There is **no third daily session**. Orders after afternoon stay in backlog → next **morning**.

Default schedule hints (configurable, not membership rules): e.g. 08:00 and 14:00 WIB. Exact hours are config (`BATCH_MORNING_CRON` / env or systemd timers) and can drift; **batch membership is still event-based**.

### Urgent classification

Match case-insensitively against
`buyer_shipping_carrier`, `shipment_provider`, `shipping_carrier_name`:

- Keywords (v1): `instant`, `sameday`, `same day`, `same-day`, `prioritas`, `gojek`, `gosend`, `grab`, `paxel`
- Optional later: `jne yes`, `sicepat best`, etc.

Store `is_urgent` on batch line snapshot at generate time (carrier string frozen on the line).

### Batch entity (conceptual)

- `id` (uuid), `account_id`, `session`, `created_at` (timestamptz), `timezone` = `Asia/Jakarta`
- `order_count`, `urgent_count`, `pdf_path` or stored bytes reference
- `status`: `ready` | `failed`
- Members: `batch_orders(batch_id, order_id, platform_order_id, carrier snapshot, is_urgent, position)`

**Idempotency:** generating twice in the same second must not double-assign; use transaction + row lock on candidate orders (`FOR UPDATE SKIP LOCKED` or equivalent).

**Reprint:** `GET` same batch PDF by id — never re-select backlog for reprint.

**Escape hatches (v1.1 ok):** remove one order from batch back to backlog; admin “force include”.

### PDF content (v1)

- Header: local date/time WIB, session, batch id, counts
- Section **Urgent** first (if any)
- Per order: platform order id, platform, carrier, ordered_at local, item lines (sku, name, qty)
- Optional footer: SKU aggregate pick list for the batch
- Filename: `batch-{session}-{yyyyMMdd-HHmm}-{shortId}.pdf`

## HTTP API (Axum)

All under existing auth. JSON camelCase to match current API style where applicable.

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/v1/batches/backlog` | Eligible orders + urgent flags |
| POST | `/v1/batches` | Body: `{ "session": "morning" \| "afternoon" \| "urgent" }` → create batch + PDF |
| GET | `/v1/batches` | Query: `date=` (WIB day), list batches |
| GET | `/v1/batches/{id}` | Batch detail + members |
| GET | `/v1/batches/{id}/pdf` | `application/pdf` download |

Later (phase reports, not blocking ops v1):

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/v1/reports/top-products` | Query: `from`, `to` (WIB dates), top SKU by qty/revenue |

Keep existing routes unchanged: `/health`, `/v1/app/lookup/*`, events, cancel report, etc.

## Web UI (`web/`)

### Hosting

```text
https://rs.obayito.com/          → static SPA (web/dist)
https://rs.obayito.com/v1/*      → proxy orders :8080
https://rs.obayito.com/health    → proxy orders :8080
```

No `/api` prefix. SPA calls same-origin `/v1/...` with Bearer token.

Nginx: stop blanket `location /` → API only; split static vs `/v1` and `/health` (update `deploy/nginx-rs.obayito.com.conf`).

### Screens (v1)

1. **Login / token gate** — store token in `sessionStorage` (v1); optional HttpOnly cookie later.
2. **Ops home**
   - Backlog counts (total / urgent)
   - Actions: Generate morning | afternoon | urgent
   - Today’s batches table → download PDF / open detail
3. **Backlog table** — platform id, carrier, urgent badge, ordered_at WIB
4. **Batch detail** — members + reprint PDF

### Screens (phase 2)

5. **Products** — top SKU table, date range (WIB), uses `/v1/reports/top-products`

### UI kit

coss components: Sidebar/layout, Button, Card, Table, Badge, Tabs, Dialog (confirm generate), Toast, Skeleton, Empty.

## Worker / cron

- Existing worker continues BigSeller sync (unchanged responsibility).
- Optional: systemd timers or worker ticks at morning/afternoon call the same batch-create logic **in-process** (preferred over HTTP self-call) so PDF generation shares DB pool.
- Web buttons call `POST /v1/batches` for manual runs and catch-up.

## Data migrations

New SQL under `docs/sql/` (or existing migration style):

- `batches`
- `batch_orders`
- `orders.batch_id` nullable FK (or membership only via `batch_orders` — prefer **membership table only** so history survives; backlog = not exists in `batch_orders` for non-voided membership)

Preferred backlog definition:

```sql
NOT EXISTS (
  SELECT 1 FROM batch_orders bo
  WHERE bo.order_id = orders.id AND bo.voided_at IS NULL
)
```

Avoid overwriting a single `orders.batch_id` if we need multi-history; one active membership is enough for v1 with `voided_at` for corrections.

## Deploy

1. `cargo build --release` — API gains batch routes.
2. `cd web && npm ci && npm run build` — artifacts to e.g. `web/dist` or `/var/www/orders-ui`.
3. Reload nginx with static + API split.
4. Apply SQL migration once.
5. `orders-api` / worker units unchanged except env if needed (`BATCH_*`).

## Phased delivery

| Phase | Deliverable |
|-------|-------------|
| **A** | SQL + batch domain + PDF + `/v1/batches*` |
| **B** | `web/` Vite + coss + ops screens + nginx |
| **C** | Optional cron morning/afternoon |
| **D** | Top products report API + UI page |

## Risks

| Risk | Mitigation |
|------|------------|
| Double generate | DB transaction + lock candidates |
| BS print marks misleading | Ignore for membership |
| Large PDF | Cap page size / stream; page_size limits on backlog display |
| Token in sessionStorage | Internal-only site; HTTPS; later cookie |
| coss/CLI churn | Pin versions in lockfile |

## Open config (defaults, adjustable in impl)

- Morning hint: `08:00` WIB
- Afternoon hint: `14:00` WIB
- Urgent keyword list: as above
- Eligible states: `new` only in v1

---

## Approval

Review this spec before implementation plan / coding.

Confirm especially:

1. React + coss ui + Axum `/v1` — yes
2. Batch membership (not clock-only, not BS print marks) — yes
3. Urgent keywords list — enough for v1?
4. Phase A backend before B web — preferred order
