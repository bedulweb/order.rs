# Rust Hygiene Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring `orders` (order.rs) from pragmatic v0.1 up to solid internal-service Rust quality: typed money, clean errors, clippy CI, real tests, no production unwraps.

**Architecture:** Keep one crate + binary. Improve in place: `error`/`map`/`store` first, then tests + CI, then light splits of fat modules. No multi-crate split unless a later task requires it.

**Tech Stack:** Rust 2021, Tokio, sqlx 0.8, axum 0.8, thiserror, rust_decimal, chrono, clap, cargo clippy, GitHub Actions.

**Repo:** `/home/ujang/projects/apps/orders` · remote `https://github.com/bedulweb/order.rs`

## Global Constraints

- Do not commit `.env`, `.session.json`, or `models/*.onnx`.
- Do not touch `loka-points` or skill trees under `.agents/`.
- Prefer small commits after each task.
- `cargo test` and `cargo clippy -- -D warnings` must pass before merge of the hygiene branch.
- Money must never go through `f64` after Task 2.
- Public HTTP JSON field names (camelCase) stay stable for consumers.

---

## File map (target)

| Path | Responsibility |
|------|----------------|
| `src/error.rs` | Library errors; `From<sqlx::Error>` |
| `src/map.rs` | JSON → typed `MappedOrder`; `Decimal` money |
| `src/store.rs` | Upserts/reads; bind `Decimal` |
| `src/sync.rs` | Worker; no dead branches / unwrap |
| `src/api.rs` | HTTP; thin |
| `src/main.rs` | CLI only; slim over time |
| `tests/fixtures/` | Redacted pageList row JSON |
| `tests/map_order.rs` | Mapper unit/integration |
| `.github/workflows/ci.yml` | fmt, clippy -D, test |
| `Cargo.toml` | lints + rust_decimal |

---

## Task 1: Clippy baseline + kill dead code

**Files:**
- Modify: `src/sync.rs` (Default derive, remove identical if/else on session load)
- Modify: `src/store.rs` (~line 578 `unwrap` on date → `ok_or` / expect with path that returns `Result`)
- Modify: `Cargo.toml` (optional workspace lints section at end)

**Produces:** `cargo clippy --all-targets -- -D warnings` clean on current tree (before money change).

- [x] **Step 1:** Run `cargo clippy --all-targets -- -W clippy::all 2>&1 | tee /tmp/clippy.txt` and list every warning.
- [x] **Step 2:** `SyncContext` — replace manual `Default` with `#[derive(Default)]`.
- [x] **Step 3:** Worker session load — delete the `or_else` that returns `Err(e)` in both branches; use plain `SessionData::load(...)` + handle `NotAuthenticated` once.
- [x] **Step 4:** Replace remaining production `unwrap()`/`expect()` outside `#[cfg(test)]` (grep: `rg '\.unwrap\(|\.expect\(' src`).
- [x] **Step 5:** `cargo clippy --all-targets -- -D warnings` must exit 0.
- [x] **Step 6:** Commit: `chore: clippy-clean baseline before money/error refactors`

---

## Task 2: Money as Decimal (no f64)

**Files:**
- Modify: `Cargo.toml` — add `rust_decimal = { version = "1", features = ["serde-with-str"] }` and `sqlx` feature if needed (`rust_decimal` via string bind is OK)
- Modify: `src/map.rs` — `as_money` → `Option<Decimal>`; `MappedOrder.amount` / item amounts `Option<Decimal>`; delete `money_str(f64)`
- Modify: `src/store.rs` — bind amount as `Option<String>` from `decimal.to_string()` **or** enable sqlx rust_decimal; drop f64 path
- Test: `src/map.rs` module tests or `tests/map_money.rs`

**Produces:** No `f64` in money path; parse `"104848"` and `"104848.50"`.

- [x] **Step 1:** Add failing test: `as_money(Some(&json!("104848"))) == Some(dec!(104848))`.
- [x] **Step 2:** Implement `as_money` with `Decimal::from_str_exact` / `from_str`; reject empty.
- [x] **Step 3:** Change `MappedOrder` / `MappedItem` fields from `Option<f64>` to `Option<Decimal>`.
- [x] **Step 4:** Update `store::upsert_order` binds; outbox JSON can use string amount.
- [x] **Step 5:** `rg 'as_money|money_str|f64' src/map.rs src/store.rs` — no money f64 left.
- [x] **Step 6:** `cargo test` + clippy -D; commit: `fix: store money as Decimal, not f64`

---

## Task 3: Typed errors (sqlx + less stringly Db)

**Files:**
- Modify: `src/error.rs`
- Modify: `src/store.rs`, `src/db.rs`, `src/accounts.rs` — use `?` instead of `map_err(|e| Error::Db(e.to_string()))` where possible

**Produces:**

```rust
#[error(transparent)]
Db(#[from] sqlx::Error),
```

Keep `Config(String)`, `Ocr(String)`, etc. Do not break `AuthExpired` / `Api` variants.

- [x] **Step 1:** Add `Db(#[from] sqlx::Error)` (or rename carefully if both String and From needed — prefer only `#[from]`).
- [x] **Step 2:** Replace bulk `.map_err(|e| Error::Db(e.to_string()))?` with `?` in `store.rs` / `db.rs` / `accounts.rs`.
- [x] **Step 3:** Ensure `api.rs` `ApiError::from` still maps internal errors to 500 without leaking connection strings (display ok; no Debug dump of URL).
- [x] **Step 4:** `cargo build --release` + clippy -D; commit: `refactor: propagate sqlx::Error via thiserror`

---

## Task 4: Fixture + mapper tests

**Files:**
- Create: `tests/fixtures/order_row_min.json` (copy from `/tmp/bs-api-map/one_order_full.json` **redacted**: strip long tokens if any; keep id, shopId, amounts, one item)
- Create: `tests/map_order.rs`
- Modify: `src/map.rs` only if public API of helpers needs `pub use`

**Produces:** Tests that do not need network or Neon.

- [x] **Step 1:** Write redacted fixture (platformOrderId, amount strings, orderItemList[0]).
- [x] **Step 2:** Test `map_order_row` returns Some; assert `id`, `shop.id`, `platform_order_id`, `amount`, item `sku`/`quantity`.
- [x] **Step 3:** Test `as_ts` ms vs seconds boundary with small table-driven cases.
- [x] **Step 4:** Test unmappable row (missing id) → None.
- [x] **Step 5:** `cargo test`; commit: `test: map_order_row fixture coverage`

---

## Task 5: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`
- Modify: `Cargo.toml` — `[lints.rust]` / `[lints.clippy]` if desired (optional; CI `-D warnings` is enough)

**Produces:** PR checks: fmt, clippy -D, test.

```yaml
# outline only — fill versions in implementation
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - dtolnay/rust-toolchain (stable, rustfmt, clippy)
      - cargo fmt --check
      - cargo clippy --all-targets -- -D warnings
      - cargo test
```

No Neon secrets in CI for unit tests. Integration DB tests = later optional job with secrets.

- [x] **Step 1:** Add workflow file; push to branch and confirm green (or act locally).
- [x] **Step 2:** Commit: `ci: fmt clippy test on push/PR`

---

## Task 6: Light module splits (only if still fat)

**Files (only if after Tasks 1–5 `store.rs` or `main.rs` still painful):**
- Create: `src/cli/` or keep functions in `main` extracted to `src/bin_support.rs`
- Create: `src/store/upsert.rs` + `src/store/read.rs` if `store.rs` > ~600 lines and hard to edit

**Produces:** No behavior change; smaller files.

- [ ] **Step 1:** Measure line counts; skip this task if both under ~400 and readable.
- [ ] **Step 2:** If split: move upsert vs read; `mod store { mod upsert; mod read; pub use ... }`.
- [ ] **Step 3:** `cargo test` + clippy; commit: `refactor: split store/cli modules`

---

## Task 7 (optional later): sqlx offline / query macros

**Not required for “hygiene done”.** Only if you want compile-time SQL:

- Enable `SQLX_OFFLINE=true`, `cargo sqlx prepare` against Neon (secrets local only).
- Convert hottest queries in `store.rs` to `query_as!`.

Skip unless Task 1–5 already merged and stable.

---

## Definition of done

- [x] `cargo clippy --all-targets -- -D warnings` = 0
- [x] `cargo test` = 0 (includes fixture mapper tests)
- [x] No money `f64` in `map`/`store`
- [x] `sqlx::Error` via `?` / `#[from]`
- [x] CI file on `main`
- [x] No production `unwrap` outside tests
- [x] Docs: one line in `README.md` under Notes: “CI runs clippy -D and tests”

## Out of scope (explicit)

- Multi-account worker loop (10 BS accounts)
- GraphQL / loka-points changes
- Full typed serde of all 214 BS fields
- Packing/print mutation APIs

## Suggested execution order

1 → 2 → 3 → 4 → 5 → (6 if needed) → stop. Task 7 only on demand.

## Estimate

| Task | Size |
|------|------|
| 1 Clippy | S |
| 2 Decimal | M |
| 3 Errors | S–M |
| 4 Tests | M |
| 5 CI | S |
| 6 Split | S optional |
