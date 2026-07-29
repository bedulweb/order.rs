# Urgent order feed filter — design

**Date:** 2026-07-29
**Status:** draft for review
**Product:** `orders` Order Masuk feed

## Goal

Add an Urgent toggle to `NewOrdersPage` so operators can view only urgent orders in every feed tab: Baru, Diproses, Dikirim, Selesai, and Semua.

## Behavior

- The toggle is rendered with the existing Order Masuk controls.
- Urgent filtering is off by default unless a previous choice exists in the browser's `localStorage`.
- Clicking the toggle flips the filter and immediately persists the new boolean value.
- When enabled, the API returns only orders classified urgent by the existing carrier classification logic.
- The filter applies together with the selected status tab, search query, pagination, and auto-refresh.
- Changing the filter resets pagination to the first page and clears selected orders so hidden orders cannot remain selected for bulk actions.
- Existing behavior is unchanged when the filter is disabled.

## API and data flow

The web API request gains an optional `urgent=true|false` query parameter. The frontend sends the parameter on every feed request. The Axum query type parses it as an optional boolean; omitted/false means no urgent restriction.

`list_orders_feed` receives the filter and adds an SQL predicate based on the same carrier fields used by `is_urgent_carrier`:

- `buyer_shipping_carrier`
- `shipment_provider`
- `shipping_carrier_name`

The predicate is applied before `ORDER BY`, `LIMIT`, and `OFFSET`, and the `total` count uses the same predicate. This keeps pagination and result counts correct for all status tabs and searches. No new order field or endpoint is required.

## Frontend design

- Add a `LOCAL_STORAGE` key dedicated to this preference.
- Initialize React state defensively from `localStorage`, treating invalid values as disabled.
- Use a compact toggle/button matching existing controls, with an accessible label and visible active state.
- Include `urgent` in the feed loader dependencies and request options.
- Keep the existing newly-arrived flash behavior limited to the unfiltered Baru view.

## Testing and verification

- Add/extend Rust tests for urgent classification/filter query contracts where practical.
- Run the web TypeScript/Vite build and lint.
- Run `cargo fmt --check`, `cargo check`, and relevant Rust tests.
- Review the final diff to confirm no unrelated files or generated artifacts changed.
