# Internal order HTTP API

Reads Postgres only. Never calls BigSeller.

Auth: `Authorization: Bearer <API_TOKEN>` or `X-Api-Key: <API_TOKEN>`.

Default base: `http://127.0.0.1:8080` (`API_BIND`).

---

## Freshness (new orders)

```text
orders worker  --every SYNC_NEW_INTERVAL_SECS-->
  BigSeller pageList status=new
  -> UPSERT shops / orders / order_items
  -> if first_seen: notification_outbox (order.created)
  -> optional POST WA_WEBHOOK_URL
```

Consumers either:

1. Lookup when the user pastes a platform order id, or
2. Poll `GET /v1/orders/events?since=...`

---

## Cancel report (evening)

```text
orders worker  --once per local day at CANCEL_HOUR_LOCAL-->
  pageList canceled (+ platformProcessing)
  -> UPSERT Neon

Consumer:
  GET /v1/reports/in-cancel/daily?date=YYYY-MM-DD
```

Manual: `orders sync --status cancel`

---

## Endpoints

### GET /health

```json
{ "ok": true }
```

### GET /v1/sync/status

Recent `sync_runs` and cached order count.

### GET /v1/orders/by-platform-id/{platformOrderId}

Query (optional):

- `shopId` -- BigSeller shop id
- `platform` -- e.g. `shopee`, `tiktok`
- `account` -- `bs_accounts.code` tenant slug (e.g. `default`)

**200**

```json
{
  "found": true,
  "count": 1,
  "order": {
    "id": 14459756009,
    "shopId": 2001903,
    "shopName": "SP Obayito",
    "platform": "shopee",
    "platformOrderId": "2607206K6S67BG",
    "amount": "104848",
    "currency": "IDR",
    "state": "new",
    "items": [
      {
        "id": 10539301694,
        "sku": "OB-0136-3",
        "variantAttr": "3",
        "itemName": null,
        "quantity": 1,
        "amount": "115000"
      }
    ]
  },
  "matches": []
}
```

**404** -- not in cache yet; wait for worker sync and retry.

### GET /v1/orders/events?since=0&limit=50

Cursor feed from `notification_outbox` (`order.created`, ...).

```json
{
  "events": [
    {
      "id": 1,
      "eventType": "order.created",
      "orderId": 14459756009,
      "platformOrderId": "2607206K6S67BG",
      "payload": {},
      "status": "pending",
      "createdAt": "2026-07-21T10:00:00Z"
    }
  ],
  "nextCursor": 1
}
```

Next page: `since=<nextCursor>`.

### GET /v1/reports/in-cancel/daily?date=2026-07-21&tzOffsetHours=7

```json
{
  "date": "2026-07-21",
  "total": 12,
  "printedCollect": 8,
  "printedLabel": 5,
  "printedAny": 9,
  "notPrinted": 3,
  "orders": []
}
```

---

## Process commands

```bash
orders sync --status new
orders sync --status cancel
orders sync --status all

orders worker
orders serve
```

Env template: `.env.example`. Put real values in local `.env` (gitignored). See [secrets.md](secrets.md).
