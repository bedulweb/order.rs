# Urgent Order Feed Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent Urgent toggle to the Order Masuk feed and apply it server-side across every status tab with correct pagination.

**Architecture:** Extend the existing `/v1/orders/new` request with an optional `urgent` query flag. Pass that flag through `api.rs` into `store::list_orders_feed`, where the SQL predicate uses the existing urgent carrier fields before count/pagination. The React page persists the toggle in `localStorage`, sends it with each request, resets pagination and selection when changed, and preserves current refresh/search behavior.

**Tech Stack:** Rust, Axum, SQLx/PostgreSQL, React 19, TypeScript, Vite, Tailwind, existing coss UI components.

## Global Constraints

- Keep existing `/v1/orders/new` status, search, pagination, and response contracts compatible.
- Use the existing `is_urgent_carrier` classification semantics; do not duplicate a divergent keyword list.
- Persist only the Urgent preference in `localStorage`; auth remains in `sessionStorage`.
- No new dependencies.
- Do not modify generated `web/dist` assets.

---

### Task 1: Add backend urgent query plumbing and SQL filtering

**Files:**
- Modify: `src/api.rs:647-683`
- Modify: `src/store.rs:764-845`
- Test: `src/store.rs` existing unit-test module, if a focused SQL contract test is appropriate

**Interfaces:**
- `NewOrdersQuery.urgent: Option<bool>` is accepted from `GET /v1/orders/new?urgent=true`.
- `list_orders_feed(pool, account_id, status, q, urgent, limit, offset)` receives the optional filter.
- `urgent = Some(true)` restricts rows to the existing urgent carrier classification; `None`/`Some(false)` leaves the feed unfiltered.

- [ ] **Step 1: Add a failing source/behavior test** that asserts the feed query contract includes an urgent predicate and that the API forwards the query option.
- [ ] **Step 2: Run the focused Rust test and confirm it fails because the query/API contract is absent.**
- [ ] **Step 3: Add `urgent: Option<bool>` to `NewOrdersQuery`, pass `q.urgent` into `list_orders_feed`, and update its call signature.**
- [ ] **Step 4: Add a bound SQL condition before search/order/pagination. Use a nullable boolean bind, with the urgent condition matching the three carrier columns through the existing `is_urgent_carrier` keyword semantics.**
- [ ] **Step 5: Ensure the total-count SQL uses the same condition and bind ordering as the select query.**
- [ ] **Step 6: Run `cargo fmt --check`, the focused tests, and `cargo check`.**
- [ ] **Step 7: Commit the backend change with `git commit -m "feat: filter order feed by urgent status"`.**

### Task 2: Persist the Urgent toggle and send it from the web page

**Files:**
- Modify: `web/src/lib/api.ts:192-205`
- Modify: `web/src/App.tsx:960-1050` and the existing controls section in `NewOrdersPage`

**Interfaces:**
- `fetchOrdersFeed` accepts `urgent?: boolean` and sends `urgent=true` only when enabled.
- `NewOrdersPage` owns `urgentOnly` state initialized from `localStorage`.
- Clicking the control persists the new value, resets `page` to `0`, clears `selectedOrders`, and causes a feed reload.

- [ ] **Step 1: Add a failing TypeScript/API contract test or a focused static assertion for `fetchOrdersFeed` query construction, if the repository's test setup supports it; otherwise use the existing build as the contract check.**
- [ ] **Step 2: Extend `fetchOrdersFeed` options with `urgent?: boolean` and append `urgent=true` when enabled.**
- [ ] **Step 3: Initialize `urgentOnly` defensively from `localStorage`, treating only the exact string `"true"` as enabled.**
- [ ] **Step 4: Include `urgentOnly` in `load`'s request and dependencies. Restrict new-order flash detection to `!urgentOnly` as well as the existing unfiltered conditions.**
- [ ] **Step 5: Add the Urgent toggle beside the existing search/refresh controls, with an accessible label and a visibly distinct active state using existing `Button`/`Zap` patterns.**
- [ ] **Step 6: Persist toggle changes under a dedicated key, reset pagination, and clear selected orders.**
- [ ] **Step 7: Run `npm run lint` and `npm run build` from `web/`.**
- [ ] **Step 8: Commit the web change with `git commit -m "feat: persist urgent order feed filter"`.**

### Task 3: Verify integration and review the diff

**Files:**
- No planned source changes.

- [ ] **Step 1: Run `cargo fmt --check`.**
- [ ] **Step 2: Run `cargo check` and relevant Rust tests.**
- [ ] **Step 3: Run `npm run lint` and `npm run build` from `web/`.**
- [ ] **Step 4: Inspect `git diff HEAD~2..HEAD` and `git status --short` for scope, generated files, and accidental changes.**
- [ ] **Step 5: Confirm the final behavior: localStorage persistence, all status tabs, server-side total/pagination, and selection reset.**
