# Proposed architecture — `orders` crate

Design target: a **deep BigSeller client** with a thin CLI, driven by the API map in `api-map.md`.  
Not implemented yet — this is the blueprint after Playwright mapping.

---

## Goals

1. **Correctness first** — match browser request shapes (envelope, cookies, `clienttype`, pageList body).
2. **Domain-shaped API** — callers think in `Order`, `Status`, `Shop`, not raw paths.
3. **Session lifecycle** — login → persist → validate → auto re-auth on `2001`.
4. **Extensible** — orders P0 now; products/inventory later without rewriting HTTP core.
5. **Testable** — mock `HttpTransport`; golden JSON fixtures from live captures.

Non-goals (for now): full packing/print workflow UI, multi-account pooling, reverse of every marketing endpoint.

---

## Recommended approach

### Option A — Layered domain client (recommended)

```text
cli (main)
  → app services (LoginService, OrderService)
    → domain (Order, Status, Session)
      → api modules (auth_api, order_api, account_api)
        → transport (HttpClient + envelope + cookies)
          → infra (ocr, crypto, config, session file)
```

**Why:** Matches observed API groups; keeps HTTP noise out of domain; easy to grow P1/P2.

### Option B — One fat `BigSellerClient` with many methods

Faster short-term, becomes unreadable past ~30 endpoints. Rejected for this map size.

### Option C — Codegen OpenAPI from catalog

No official OpenAPI; JS is minified. Manual domain types from fixtures are cheaper and more accurate.

---

## Module layout (target)

```text
src/
  lib.rs                 # public facade
  main.rs                # clap CLI only

  config.rs              # env / .env
  error.rs               # Error + ApiCode mapping

  transport/
    mod.rs
    client.rs            # reqwest + cookie_store + clienttype
    envelope.rs          # parse {code,msg,data}, ensure_ok
    retry.rs             # auth-expired → callback

  infra/
    crypto.rs            # password blob
    ocr.rs               # CaptchaOcr
    session_store.rs     # .session.json

  domain/
    mod.rs
    order.rs             # Order, OrderItem, OrderId, Money
    status.rs            # OrderStatus enum ↔ API strings
    shop.rs
    account.rs
    page.rs              # Page<T> { total, page_no, rows }

  api/
    mod.rs
    auth.rs              # genVerifyCode, loginsub, is_login
    account.rs           # index, shops, rights
    orders.rs            # page_list, status_counts, …
    # later: products.rs, inventory.rs

  services/
    mod.rs
    auth_service.rs      # OCR loop + save session
    order_service.rs     # high-level list/filter helpers

  cli/                   # optional split from main
    login.rs
    list.rs
    …
```

### Mapping current → target

| Now | Becomes |
|-----|---------|
| `client.rs` | `transport/{client,envelope}.rs` |
| `auth.rs` | `api/auth.rs` + `services/auth_service.rs` |
| `orders.rs` | `api/orders.rs` + `domain/order.rs` + `services/order_service.rs` |
| `session.rs` | `infra/session_store.rs` |
| `crypto.rs` / `ocr.rs` | `infra/*` |

---

## Domain model (P0)

```rust
// conceptual — not final names

enum OrderStatus {
    New,           // "new"
    Shipped,       // "shipped"
    Completed,     // "completed" — may need history flags
    Canceled,      // "canceled"
    PlatformProcessing, // "platformProcessing"
    ReturnRefund,  // "Return refund"
    ToBeSupplemented,   // "toBeSupplemented"
    // Unknown(String) for forward-compat
}

struct OrderListParams {
    status: OrderStatus,
    page_no: u32,
    page_size: u32,
    order_by: OrderSort,     // ExpireTime, …
    shop_id: Option<i64>,
    platform: Option<String>,
    search: Option<OrderSearch>, // type + content
    // escape hatch:
    extra: serde_json::Map<String, Value>,
}

struct Order {
    id: i64,
    platform_order_id: String,
    package_no: Option<String>,
    shop_id: i64,
    shop_name: String,
    platform: String,
    state: String,
    view_status: Option<String>,
    amount: String,
    amount_unit: String,
    buyer_username: Option<String>,
    tracking_no: Option<String>,
    shipment_provider: Option<String>,
    payment_method: Option<String>,
    created_at_ms: Option<i64>,
    items: Vec<OrderItem>,
    raw: Value,  // full row for fields we don't model yet
}

struct StatusCounts {
    by_status: HashMap<String, u64>,  // from statusCountMap
    // optional: shops, platforms from same payload
    raw: Value,
}
```

**Rule:** always keep `raw: Value` on large entities so CLI `--json` never loses fields while types stay stable.

---

## Transport design

```rust
trait Transport {
    async fn get_json(&self, path: &str) -> Result<Value>;
    async fn post_json(&self, path: &str, body: &Value) -> Result<Value>;
    fn snapshot_cookies(&self) -> Result<HashMap<String, String>>;
}

struct Envelope<T> {
    code: i64,
    msg: String,
    data: T,
}

// ensure_ok: code==0 else Error::Api { code, message, reauth: code indicates auth }
```

Session:

```rust
struct Session {
    cookies: HashMap<String, String>,
    account: Option<String>,
    saved_at: Option<String>,
}

impl Session {
    fn is_authenticated(&self) -> bool; // muc_token present
}
```

`AuthService::ensure_session()`:

1. Load session file  
2. `GET isLogin`  
3. If false → full login (OCR) → save  

`OrderService` always goes through `ensure_session` or receives a live `Client`.

---

## API module style

Thin, path-faithful, typed where cheap:

```rust
// api/orders.rs
impl OrdersApi {
    pub async fn status_counts(&self) -> Result<StatusCounts>;
    pub async fn page_list(&self, p: &OrderListParams) -> Result<Page<Order>>;
}

// Builds JSON from OrderListParams + merges pageList-request-template defaults
fn page_list_body(p: &OrderListParams) -> Value;
```

Constants:

```rust
pub mod paths {
    pub const PAGE_LIST: &str = "/api/v1/order/new/pageList.json";
    pub const STATUS_COUNT: &str = "/api/v1/order/getOrderStatusCount.json";
    pub const IS_LOGIN: &str = "/api/v1/isLogin.json";
    pub const GEN_CAPTCHA: &str = "/api_v2/api/v2/genVerifyCode.json";
    pub const LOGIN: &str = "/api_v2/api/v3/auth/loginsub.json";
    // …
}
```

---

## CLI surface (stable)

```text
orders login
orders status                 # session + isLogin
orders counts                 # statusCountMap table
orders list --status new [--page N] [--json] [--shop-id]
orders me                     # index.json summary (P1)
orders shops                  # shopsAndPlatforms (P1)
```

---

## Error model

```text
Config / Ocr / Crypto / Http / Io / Json
Api { code, message, reauth: bool }
NotAuthenticated
LoginExhausted { attempts, last_message }
```

Map known auth failures (`2001`, msg contains `401`) → `reauth = true`.

---

## Testing strategy

| Layer | How |
|-------|-----|
| crypto | unit (blob shape) |
| ocr | optional file fixture |
| envelope | unit on sample JSON |
| order parse | golden: slice of `pageList` row → `Order` |
| api | `wiremock` or recorded fixtures under `tests/fixtures/` |
| e2e | `#[ignore]` live test needing `BS_*` |

Fixtures should be **redacted** (no tokens, mask buyer names if sharing).

---

## Implementation phases

### Phase 1 — structure only (no behavior change)

- Move files into `transport/`, `infra/`, `api/`, `domain/`
- Keep public CLI behavior identical
- Add `paths` constants + full pageList default body from template

### Phase 2 — domain types

- `OrderStatus`, `Order`, `OrderItem`, `StatusCounts`, `Page<T>`
- Fix `OrderSummary` field mapping from real row keys (`platformOrderId`, `amount`, …)
- `list` CLI uses typed parse; `--json` prints `raw` page

### Phase 3 — session lifecycle

- `isLogin` probe before list
- Optional `--relogin` / auto re-login once on auth error

### Phase 4 — P1 account context

- `me`, `shops` commands
- Cache shops in memory for filter validation

### Phase 5 — mutations (after more Playwright)

- Pack / print / ship endpoints once mapped by click capture

---

## What we deliberately do *not* do

- Mirror all 50+ shell endpoints as first-class methods  
- Depend on broken crates.io `ddddocr`  
- Store passwords in session file (cookies only)  
- Commit `.session.json` / ONNX / `.env`

---

## Decision needed from you

1. **Approve this layout (Option A)?**  
2. **Phase 1+2 now** (refactor + typed orders), or map **mutation APIs** with Playwright first?  
3. Scope freeze for v0.2: **list + counts + login** only, or also **me/shops**?
