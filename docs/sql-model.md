# PostgreSQL model (long-term) — BigSeller orders

Based on live `pageList` captures (44 rows × **214** top-level fields), nested
`orderItemList` (~61 fields), `feeDetail` + `otherFeeInfo`, plus
`getOrderStatusCount` / `shopsAndPlatforms` shapes from API map.

**Consumers:** internal web app (read orders) + Rust sync worker.  
**Not for:** public GraphQL / multi-tenant SaaS yet.

---

## 1. API → storage principles

| Principle | Why |
|-----------|-----|
| **Hybrid columns + JSONB** | UI/list/filter need typed columns; BigSeller adds/renames fields often — full row stays in `payload`. |
| **BigSeller `id` is PK** | Stable int64 on every row (`14459756009`). |
| **Business unique** | `(shop_id, platform_order_id)` — marketplace order number is unique per shop. |
| **Money as `numeric`** | API sends amounts as **strings** (`"104848"`, `"0.00"`). Never float. |
| **Time as `timestamptz`** | API uses **epoch milliseconds** (`orderCreateTime: 1784552788000`). Store UTC; keep `*_str` only inside JSON if needed. |
| **Raw always wins on sync** | UPSERT replaces typed columns **and** `payload` from latest API row. |
| **App reads Postgres** | Web app never talks to BigSeller directly; only sync worker does. |

```text
BigSeller REST  ──sync worker──►  PostgreSQL  ◄──  Web app (read)
     (SoT BS)                      (SoT app)
```

---

## 2. Identity & relationships (from API)

```text
Account (BS login)
  └── Shop[]                    shopsAndPlatforms / statusCount.shopLists
        └── Order[]             pageList.rows[]  id = BigSeller order id
              ├── OrderItem[]   orderItemList[]  id = line id
              ├── FeeDetail     feeDetail (1:1, same id as order)
              ├── Label[]       labelList[]
              └── Remark[]      orderRemarksList[]
```

| API field | Role |
|-----------|------|
| `id` | Internal BigSeller order id (PK) |
| `platformOrderId` | Shopee/TikTok order no. |
| `packageNo` | BigSeller package id (`BS2798049528`) |
| `shopId` + `shopName` + `platform` | Shop denormalized on order row |
| `state` | BS bucket: `new`, `shipped`, `canceled`, … |
| `platformState` | Marketplace raw: `READY_TO_SHIP`, … |
| `orderItemList[].id` | Line id (unique globally in samples) |
| `orderItemList[].platformItemId` / `varSku` | Catalog refs (often null on TikTok name fields) |

**Observed quirks (model must tolerate):**

- `amount` / many fees = **string**, not number  
- `viewPlatfrom` typo in API (keep in payload; expose `platform` typed)  
- `itemName` often **null**; use `varSku` + `varAttr` + image  
- `shippedTime` present even on `state=new` (deadline-ish, not always “already shipped”)  
- Address mostly masked: `contactPerson`, `recipient` (region only) — no full street in list API  
- `feeDetail.id` == order `id`

---

## 3. Table map (long-term)

```text
bs_accounts          # login identity (optional multi-account later)
bs_sessions          # muc_token cookies (restricted access)
shops
orders               # typed + payload jsonb
order_items
order_fees           # optional normalized fee; or only inside orders.payload
order_labels         # optional
order_remarks        # optional
order_status_history # append-only when state changes on sync
sync_runs            # observability
sync_cursors         # per-status / per-shop high-water marks
```

**v1 minimum to ship web “cek order”:**  
`shops`, `orders`, `order_items`, `sync_runs`, `bs_sessions` (or env session file first).

---

## 4. Column mapping (orders)

### 4.1 Always promote to real columns (query / index / UI)

| Column | API source | PG type | Notes |
|--------|------------|---------|-------|
| `id` | `id` | `bigint` PK | |
| `shop_id` | `shopId` | `bigint` NOT NULL | FK → shops |
| `platform` | `platform` | `text` NOT NULL | `shopee`, `tiktok`, … |
| `platform_order_id` | `platformOrderId` | `text` NOT NULL | |
| `package_no` | `packageNo` | `text` | BS package |
| `state` | `state` | `text` NOT NULL | list bucket |
| `platform_state` | `platformState` | `text` | |
| `view_status` | `viewStatus` | `text` | |
| `marketplace_state` | `marketPlaceState` | `text` | |
| `amount` | `amount` | `numeric(18,2)` | parse string |
| `currency` | `amountUnit` | `char(3)` | `IDR` |
| `buyer_username` | `buyerUsername` | `text` | |
| `contact_person` | `contactPerson` | `text` | masked |
| `recipient_region` | `recipient` | `text` | not full address |
| `payment_method` | `paymentMethod` | `text` | Prepaid / COD |
| `tracking_no` | `trackingNo` | `text` | empty string → null |
| `shipment_provider` | `shipmentProvider` | `text` | |
| `shipping_carrier_name` | `shippingCarrierName` | `text` | |
| `shipping_carrier_id` | `shippingCarrierId` | `bigint` | |
| `warehouse_id` | `warehouseId` | `bigint` | |
| `warehouse_name` | `shipmentWarehouse` | `text` | |
| `pack_state` | `packState` | `smallint` | |
| `item_total_num` | `itemTotalNum` | `int` | |
| `print_label_mark` | `printLabelMark` | `smallint` | 0/1 flags |
| `print_collect_mark` | `printCollectMark` | `smallint` | |
| `has_error` | derived | `boolean` | `error`/`errorMsg` non-empty |
| `error_msg` | `errorMsg` | `text` | |
| `ordered_at` | `orderCreateTime` | `timestamptz` | ms/1000 |
| `paid_at` | `payTime` | `timestamptz` | nullable |
| `ship_by_at` | `shippedTime` | `timestamptz` | **name carefully** — often SLA deadline on new |
| `completed_at` | `completedTime` | `timestamptz` | |
| `deadline_at` | `deadline` | `timestamptz` | |
| `timeout_at` | `timeoutSeconds` | `timestamptz` | API sometimes seconds not ms — detect magnitude |
| `store_site` | `storeSite` / `addressSite` | `text` | `ID` |
| `payload` | full row | `jsonb` NOT NULL | **source of truth blob** |
| `payload_hash` | sha256(payload) | `bytea` | skip write if unchanged |
| `synced_at` | worker | `timestamptz` | last successful UPSERT |
| `first_seen_at` | worker | `timestamptz` | insert only |
| `updated_at` | worker | `timestamptz` | |

### 4.2 Keep only in `payload` (unless product needs later)

Print timestamps matrix, TikTok split tags, blacklist, serial flags, voucher blobs,
agent logistics, digital delivery, multi-order links, most `*Str` display strings,
`feeDetail` full tree (or promote later into `order_fees`).

### 4.3 Money / time helpers (Rust or SQL)

```text
parse_money(s): null if s in (None,""); else Numeric
parse_bs_time(n):
  if n is null → null
  if n > 1e12 → timestamptz from ms   # orderCreateTime style
  if n > 1e9  → timestamptz from sec  # timeoutSeconds style (~1784653199)
  else → null / log warning
```

---

## 5. order_items

| Column | API | PG |
|--------|-----|-----|
| `id` | `orderItemList[].id` | `bigint` PK |
| `order_id` | parent `id` | `bigint` FK |
| `line_no` | array index | `int` |
| `sku` | `varSku` | `text` |
| `variant_attr` | `varAttr` | `text` |
| `quantity` | `quantity` | `int` |
| `amount` | `amount` | `numeric(18,2)` |
| `unit_price` | `varDiscountedPrice` | `numeric(18,2)` |
| `original_price` | `varOriginalPrice` | `numeric(18,2)` |
| `image_url` | `image` | `text` |
| `product_url` | `link` | `text` |
| `platform_item_id` | `platformItemId` | `text` |
| `platform_variation_id` | `platformVariationId` | `text` |
| `item_name` | `itemName` / `vName` | `text` | often null |
| `is_addition` | `isAddition` | `boolean` |
| `payload` | full line | `jsonb` |
| `synced_at` | | `timestamptz` |

On sync: **replace strategy** — delete items for `order_id` not in latest list, UPSERT current lines (simplest, correct for list API).

---

## 6. shops

From `statusCount.shopLists` / `shopsAndPlatforms`:

| Column | Source |
|--------|--------|
| `id` | `shopId` / `id` |
| `name` | `name` / `shopName` |
| `platform` | `platform` |
| `site` | `site` (`ID`) |
| `status` | `status` |
| `payload` | full shop object |
| `synced_at` | |

Synthetic shops in count API: Manual (`id=0`), POS (`1`), Messenger (`2`) — still storeable.

---

## 7. Sync metadata

### `sync_runs`

```text
id, kind ('orders_full'|'orders_status'|'shops'),
status ('running'|'ok'|'error'),
started_at, finished_at,
pages_fetched, rows_upserted, error_text, meta jsonb
```

### `sync_cursors`

```text
key text PK   -- e.g. 'orders:new', 'orders:canceled'
value jsonb   -- { "last_page": 1, "last_sync_at": "..." }
updated_at
```

### `order_status_history` (long-term gold)

When UPSERT sees `OLD.state IS DISTINCT FROM NEW.state`:

```text
order_id, from_state, to_state, changed_at (synced_at), source ('sync')
```

Web app can show timeline without re-hitting BigSeller.

---

## 8. Indexes (web “cek order”)

```text
orders (state, ordered_at DESC)           -- inbox tabs
orders (shop_id, ordered_at DESC)
orders (platform, platform_order_id)      -- unique + lookup
orders (tracking_no) WHERE tracking_no IS NOT NULL
orders USING gin (payload jsonb_path_ops) -- optional power search later
order_items (order_id)
order_items (sku)
order_items (platform_item_id)
```

Full-text (optional v2):

```sql
generated tsvector on (platform_order_id, buyer_username, package_no, sku join)
```

---

## 9. Sync algorithm (recommended)

```text
for status in [new, shipped, canceled, ...known working...]:
  page = 1
  loop:
    rows = BS.pageList(status, page, pageSize=100)
    if empty: break
    for row in rows:
      upsert shop (from row.shopId/Name/platform)
      upsert order (typed + payload)
      replace order_items
      if state changed: insert history
    page += 1
  record sync_run
```

**Web read path:**

```text
GET /api/orders?state=new&q=2607206
  → SQL only
POST /api/orders/sync   (admin)
  → enqueue worker → BigSeller → PG
GET /api/orders/:id
  → PG; optional ?refresh=1 → single-order fetch when we map detail API
```

Do **not** implement “if missing in PG then fetch BigSeller” per list row (N+1 + rate limits).  
Missing detail after sync window → explicit refresh.

---

## 10. Multi-tenancy / security

Even for one seller now:

| Table | Access |
|-------|--------|
| `orders` / items | web app role `orders_read` |
| `bs_sessions` | **only** sync worker role — never expose to web API |
| PII columns | buyer_username, contact_person — audit logs on export |

Future multi-account: add `account_id` FK on shops/orders; RLS by account.

---

## 11. What we deliberately don’t normalize in v1

- Full `feeDetail.otherFeeInfo` (29 keys, platform-specific) → JSONB  
- Print/pack matrix of marks+times → payload until packing UI  
- Status count snapshots every minute → optional `stats_snapshots` later  
- GraphQL  

---

## 12. Example row (mental model)

```text
orders.id                 = 14459756009
platform                  = shopee
platform_order_id         = 2607206K6S67BG
package_no                = BS2798049528
state                     = new
amount / currency         = 104848 / IDR
buyer_username            = dini.nurramdani
ordered_at                = 2026-07-20 06:06:28+00  (from ms)
payload                   = { ... entire API object ... }

order_items
  id=10539301694  sku=OB-0136-3  qty=1  amount=115000
```

---

## 13. Files

| File | Purpose |
|------|---------|
| `docs/sql/001_init.sql` | DDL v1 |
| `docs/sql-model.md` | this document |
| `docs/api-map.md` | HTTP endpoints |
| `docs/pageList-request-template.json` | sync request body |

---

## 14. Implementation order (when you say go)

1. Apply `001_init.sql`  
2. Rust: `domain` types + `UPSERT` from `pageList` row  
3. `orders sync` CLI command  
4. Thin read API / web queries against PG  

No GraphQL until the web app has 3+ complex nested screens that hurt with REST.
