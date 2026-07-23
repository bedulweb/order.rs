# Implementation plan: Ops batch + React coss UI

**Date:** 2026-07-22
**Spec:** [2026-07-22-ops-batch-web-design.md](../specs/2026-07-22-ops-batch-web-design.md)

## Approach

1. **SQL** — `docs/sql/005_batches.sql`: `batches` + `batch_orders` with `voided_at`. Membership-only backlog (no `orders.batch_id`).
2. **Rust domain** — `src/batch.rs` pure urgent/session helpers + store I/O; `src/batch_pdf.rs` PDF bytes; wire `/v1/batches*` in `api.rs`.
3. **Generate** — single transaction, `FOR UPDATE SKIP LOCKED` on candidate orders, insert batch + members + PDF bytes, commit.
4. **Web** — `web/` Vite React TS + Tailwind v4 + `@coss/style`; token gate, ops home, backlog, batch detail + PDF.
5. **Nginx** — SPA static `/`, proxy `/v1` and `/health` to `:8080`.

## Task checklist

- [x] SQL migration + apply/document
- [x] Batch domain + PDF + API routes + tests
- [x] Web scaffold + screens + build
- [x] Nginx conf
- [x] Smoke / verification evidence

## Out of scope (v1)

- Phase-2 top-products
- systemd timer / worker auto-batch (optional)
- void/remove-order escape hatches
