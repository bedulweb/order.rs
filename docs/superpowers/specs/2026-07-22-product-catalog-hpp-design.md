# Product catalog + HPP (ART/SKU cost) — design

**Date:** 2026-07-22
**Status:** implemented
**Product:** `orders` (Postgres catalog + `/v1` API + ops UI)

## Goals

1. Durable **product catalog** keyed by **ART** (SKU code), with normalized product name and **HPP** (harga pokok penjualan / cost) in IDR.
2. **Idempotent import** from the normalized marketplace workbook (`MARKETPLACE_PRICE_2026_NORMALIZED.xlsx`).
3. Authenticated **HTTP API** under existing `/v1` + `API_TOKEN` for list/get/import.
4. Ops **web UI** screen to browse catalog (ART, name, HPP) and trigger import from the repo workbook path (or upload).
5. Reliable **lookup by ART** for joining to order line SKUs (`order_items.sku` ↔ catalog ART).

## Non-goals (v1)

- Full margin/profit analytics dashboards or charts.
- Replacing BigSeller inventory or live marketplace price sync.
- Auto-resolving **Perlu Review** duplicate-ART conflicts beyond documenting/skipping that sheet.
- Multi-currency, historical HPP versioning UI.
- RBAC beyond existing `API_TOKEN`; public unauthenticated catalog.
- Changing batch PDF / backlog membership rules (batch v1 stays as-is).

## Source workbook

| Item | Value |
|------|--------|
| File | `MARKETPLACE_PRICE_2026_NORMALIZED.xlsx` (repo root) |
| Primary sheet | **`Produk Normalisasi`** |
| Columns (A–D) | `No`, `Nama Produk Normalisasi`, `ART`, `HPP` |
| Conflict sheet | **`Perlu Review`** — **not imported** in v1 (documentation only) |

Import policy: **primary sheet only**. Rows on `Perlu Review` are human review leftovers (duplicate ARTs with conflicting HPP/names, blank names, etc.). They must not be double-inserted.

Within the primary sheet, the same ART may appear more than once. **Last valid row wins** for that ART in a single import pass (after normalize). Re-import upserts on ART — no duplicate ART rows in Postgres.

## Domain model

### Product (catalog row)

| Field | Type | Notes |
|-------|------|--------|
| `art` | text PK | SKU code; unique; join key to `order_items.sku` |
| `name` | text | Normalized product name |
| `hpp` | bigint | Cost in whole IDR (no decimals stored) |
| `updated_at` | timestamptz | Last upsert time |
| `created_at` | timestamptz | First insert time |

### Normalize rules (pure; tested without DB)

- **ART:** trim whitespace; reject empty after trim. Case preserved as in source (codes are typically uppercase). Lookup query ART is trimmed the same way; match is **exact** (case-sensitive) against stored `art`.
- **Name:** trim; empty name is allowed only if ART+HPP valid (prefer non-empty; empty still upserts with `""`).
- **HPP:** parse as integer IDR. Accept plain digits (`39900`), digit strings with spaces, optional trailing `.0`. Reject blank, `-`, non-numeric text. Values are whole rupiah (no fractional cents).

### Lookup for order lines

```text
order_items.sku  --trim-->  catalog.art
  hit  → Product { art, name, hpp }
  miss → explicit None / 404 (never panic)
```

## Persistence

SQL migration: `docs/sql/006_product_catalog.sql`

```sql
CREATE TABLE IF NOT EXISTS product_catalog (
    art         TEXT PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT '',
    hpp         BIGINT NOT NULL CHECK (hpp >= 0),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Upsert: `INSERT … ON CONFLICT (art) DO UPDATE SET name, hpp, updated_at` in one transaction per import batch. Counts: `inserted` / `updated` / `skipped` (invalid rows).

## HTTP API (Axum, same auth as batches)

All routes require `API_TOKEN` when set (`Authorization: Bearer` or `X-Api-Key`).

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/v1/catalog/products` | List products. Query: `q` (search ART or name, optional), `limit` (default 500, max 2000), `offset` (default 0). |
| `GET` | `/v1/catalog/products/{art}` | Get one product by ART. 404 if missing. |
| `POST` | `/v1/catalog/import` | Import. Body JSON `{ "path": "…" }` optional (default repo workbook), **or** multipart field `file` xlsx upload. Response: `{ inserted, updated, skipped, totalRows }`. |

JSON field names: **camelCase** (`hpp`, `art`, `name`, `inserted`, …).

Does not break existing `/v1/batches/*`, `/v1/app/*`, lookup, or `/health`.

## CLI (optional)

```bash
orders catalog-import [--path MARKETPLACE_PRICE_2026_NORMALIZED.xlsx]
```

Prints the same counts as the API (JSON line or human summary).

## Ops web UI (`web/`)

- Token gate unchanged (sessionStorage + same Bearer helpers).
- Nav entry **Products** → catalog table: ART, name, HPP (formatted IDR).
- Search box hits list `q`.
- **Import** button calls `POST /v1/catalog/import` (default path) and refreshes the table; shows inserted/updated/skipped notice.
- Same-origin `/v1` fetches like batches.

## Stack

| Layer | Choice |
|--------|--------|
| Parse xlsx | Rust `calamine` (no Python in production path) |
| Domain | `src/catalog.rs` — pure normalize + store upsert/lookup |
| API | `src/api.rs` routes |
| SQL | `docs/sql/006_product_catalog.sql` |
| UI | `web/src` Products view + `lib/api.ts` |

## Tests (in-repo)

1. Pure parse/normalize of representative workbook-shaped rows: known ART → expected HPP; empty ART / bad HPP → skip.
2. Lookup hit (`OB-001` → 39900) and miss (unknown ART → None) on shipped functions.
3. Optional DB integration: double import → no duplicate ARTs; second run reports updates/skips.
4. Static route wiring asserts `/v1/catalog/products` paths exist in `api.rs`.

## Verification notes

- Apply migration when DB available: `psql "$DATABASE_URL" -f docs/sql/006_product_catalog.sql`
- Import twice against real xlsx; assert unique ART count stable.
- Authenticated GET list/detail for an imported ART returns numeric `hpp`.
