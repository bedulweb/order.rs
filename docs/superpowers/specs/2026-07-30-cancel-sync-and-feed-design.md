# Cancel sync and Order Masuk feed — design

**Date:** 2026-07-30  
**Status:** approved design; awaiting written-spec review  
**Product:** `orders` worker, on-demand sync, and Order Masuk feed

## Goal

Keep the local order state aligned when BigSeller cancels an order. Every normal new-order sync must also pull BigSeller's cancel bucket. An order that was previously shown in **Baru** must move to **Cancel** as soon as the next sync sees its canceled state.

The Order Masuk feed adds a **Cancel** tab. It shows orders whose state changed to `canceled` today in WIB, matching the time scope of **Diproses**, **Dikirim**, and **Selesai**.

## Scope

- Worker ticks synchronize `new` and `canceled` in the same cycle.
- UI-triggered Refresh uses the same sequence: `new`, `canceled`, then reconciliation.
- Reconciliation checks every eligible local `new` order that disappeared from the latest BigSeller new bucket, rather than an arbitrary candidate cap.
- Feed/API support a `canceled` status and return its count.
- UI renders the Cancel tab after Semua.

No cancellation action is introduced. The change only imports and displays BigSeller's existing cancellation state.

## Synchronization behavior

### Worker

Each successful worker tick will:

1. Pull and upsert BigSeller's `new` bucket.
2. Pull and upsert BigSeller's `canceled` bucket.
3. Reconcile every account-scoped, recent local order still in state `new` whose `synced_at` predates the successful new-bucket pass.
4. Continue the existing `processing`/`shipped` schedules and outbox handling.

The dedicated evening cancel sync is removed because cancel synchronization is now part of every regular cycle. `platformProcessing` remains on its existing cancel-related path unless its cadence is separately changed.

The all-candidate reconciliation keeps the existing safety boundaries:

- It runs only after a successful pull of the `new` bucket, so absence has meaning.
- It only considers orders from the last 30 days.
- It retains BigSeller request pacing and retry/backoff behavior.
- An order not found remains retryable unless it is old enough for the existing archive behavior.

This is intentionally exhaustive rather than capped: a cancellation must not wait behind unrelated stale orders. At the expected small active-new-order volume, paced individual lookups are acceptable. Sync progress and worker logs report candidate/refreshed counts so a slow or unusually large run is visible.

### Refresh from the UI

The on-demand Refresh flow will report separate steps:

1. Check BigSeller session.
2. Pull order Baru.
3. Pull order Cancel.
4. Reconcile state order.

A Refresh success means both bucket pulls and reconciliation have completed. If Cancel pull fails, the Refresh reports an error rather than claiming the data is current.

## Feed behavior

### Status contract

`FeedStatus` and `GET /v1/orders/new` accept:

`new | processing | shipped | completed | all | canceled`

`canceled` maps to `o.state = 'canceled' AND o.state_changed_at >= current WIB midnight`. It uses the same search, account, urgent, unprinted-summary, ordering, total-count, and pagination semantics as the existing tabs.

`FeedCounts` gains `canceled`, calculated with the same WIB-day boundary and account/filter constraints as the other per-tab counts. The All tab continues to include canceled orders, since it means every non-archived order.

### UI

The Order Masuk tabs are:

**Baru · Diproses · Dikirim · Selesai · Semua · Cancel**

Cancel is read-only: it does not expose selection, packing, summary printing, or resi printing controls. Empty-state copy says that canceled orders appear after synchronization.

## Error handling and compatibility

- Existing status values and omitted status query behavior remain unchanged.
- An invalid status response lists `canceled` among the permitted values.
- Sync errors are surfaced by existing worker logs and the on-demand progress dialog; no failed pull is silently treated as current data.
- Existing user-owned uncommitted `resi` probe work remains untouched.

## Testing and verification

1. Add focused Rust tests for status parsing and SQL/feed contract so canceled is accepted and scoped to orders canceled today.
2. Add a reconciliation test seam or extracted candidate-query test proving no cap omits eligible stale new orders.
3. Run focused Rust tests, `cargo fmt --check`, `cargo check`, and `cargo clippy --all-targets --all-features --locked -- -D warnings` where the project supports it.
4. Run `npm run lint` and `npm run build` in `web/`.
5. Manually verify with a fixture or controlled database state: an order starts as `new`; a regular sync pulls it from BigSeller canceled; its local state becomes `canceled`; it disappears from Baru and appears in Cancel with the correct count.
6. Review the final diff and `git status --short` to confirm no generated assets or unrelated user changes are included.
