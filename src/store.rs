//! Postgres upserts + public read queries.

use crate::error::{Error, Result};
use crate::map::{money_str, MappedOrder};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct UpsertOutcome {
    pub order_id: i64,
    pub is_new: bool,
    pub state_changed: bool,
    pub previous_state: Option<String>,
}

fn is_cancel_state(state: &str) -> bool {
    matches!(
        state.to_ascii_lowercase().as_str(),
        "canceled" | "cancelled"
    )
}

/// True when Summary List was already printed for this order:
/// ops `batch_orders` membership and/or BigSeller collect/pick print marks.
pub async fn order_summary_was_printed(pool: &PgPool, order_id: i64) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT
            COALESCE(o.print_collect_mark, 0)::int AS print_collect_mark,
            COALESCE(o.print_pick_list_mark, 0)::int AS print_pick_list_mark,
            EXISTS(
                SELECT 1 FROM batch_orders bo WHERE bo.order_id = o.id
            ) AS in_batch
        FROM orders o
        WHERE o.id = $1
        "#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| Error::Other(format!("order {order_id} not found")))?;

    let collect: i32 = row.get("print_collect_mark");
    let pick: i32 = row.get("print_pick_list_mark");
    let in_batch: bool = row.get("in_batch");
    Ok(in_batch || collect != 0 || pick != 0)
}

pub async fn upsert_order(
    pool: &PgPool,
    m: &MappedOrder,
    account_id: Option<i64>,
) -> Result<UpsertOutcome> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO shops (id, account_id, platform, name, site, payload, synced_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, now(), now())
        ON CONFLICT (id) DO UPDATE SET
            account_id = COALESCE(EXCLUDED.account_id, shops.account_id),
            platform = EXCLUDED.platform,
            name = EXCLUDED.name,
            site = COALESCE(EXCLUDED.site, shops.site),
            synced_at = now(),
            updated_at = now()
        "#,
    )
    .bind(m.shop.id)
    .bind(account_id)
    .bind(&m.shop.platform)
    .bind(&m.shop.name)
    .bind(&m.shop.site)
    .execute(&mut *tx)
    .await?;

    // Canonical row key for app lookup is platform_order_id. BigSeller may reuse the same
    // (shop_id, platform_order_id) with a different internal id across list buckets /
    // multi-package — unique index orders_shop_platform_order_uid would fail on plain
    // ON CONFLICT (id). Prefer the existing id when the marketplace key already exists.
    let existing_by_key = sqlx::query(
        r#"
        SELECT id, state FROM orders
        WHERE shop_id = $1 AND platform_order_id = $2
        LIMIT 1
        "#,
    )
    .bind(m.shop.id)
    .bind(&m.platform_order_id)
    .fetch_optional(&mut *tx)
    .await?;

    let order_id = if let Some(ref row) = existing_by_key {
        row.get::<i64, _>("id")
    } else {
        m.id
    };

    let prev = sqlx::query(r#"SELECT state FROM orders WHERE id = $1"#)
        .bind(order_id)
        .fetch_optional(&mut *tx)
        .await?;

    let (is_new, previous_state, state_changed) = match prev {
        None => (true, None, false),
        Some(row) => {
            let old: String = row.get("state");
            let changed = old != m.state;
            (false, Some(old), changed)
        }
    };

    let amount = money_str(m.amount);
    sqlx::query(
        r#"
        INSERT INTO orders (
            id, account_id, shop_id, platform, platform_order_id, package_no, package_index,
            state, platform_state, view_status, marketplace_state, last_order_status,
            amount, currency, payment_method,
            buyer_username, contact_person, recipient_region, buyer_message, seller_note,
            tracking_no, tracking_url, shipment_provider,
            shipping_carrier_id, shipping_carrier_name, buyer_shipping_carrier,
            shipping_config_option_id, shipping_config_option_name,
            warehouse_id, warehouse_name, store_site,
            pack_state, item_total_num,
            print_label_mark, print_bill_mark, print_pick_list_mark, print_collect_mark,
            has_error, error_msg,
            ordered_at, paid_at, ship_by_at, completed_at, deadline_at, timeout_at, printed_collect_at,
            payload, payload_hash, first_seen_at, synced_at, updated_at
        ) VALUES (
            $1,$2,$3,$4,$5,$6,$7,
            $8,$9,$10,$11,$12,
            $13::numeric,$14,$15,
            $16,$17,$18,$19,$20,
            $21,$22,$23,
            $24,$25,$26,
            $27,$28,
            $29,$30,$31,
            $32,$33,
            $34,$35,$36,$37,
            $38,$39,
            $40,$41,$42,$43,$44,$45,$46,
            $47,$48, now(), now(), now()
        )
        ON CONFLICT (id) DO UPDATE SET
            account_id = COALESCE(EXCLUDED.account_id, orders.account_id),
            shop_id = EXCLUDED.shop_id,
            platform = EXCLUDED.platform,
            platform_order_id = EXCLUDED.platform_order_id,
            package_no = EXCLUDED.package_no,
            package_index = EXCLUDED.package_index,
            state = EXCLUDED.state,
            platform_state = EXCLUDED.platform_state,
            view_status = EXCLUDED.view_status,
            marketplace_state = EXCLUDED.marketplace_state,
            last_order_status = EXCLUDED.last_order_status,
            amount = EXCLUDED.amount,
            currency = EXCLUDED.currency,
            payment_method = EXCLUDED.payment_method,
            buyer_username = EXCLUDED.buyer_username,
            contact_person = EXCLUDED.contact_person,
            recipient_region = EXCLUDED.recipient_region,
            buyer_message = EXCLUDED.buyer_message,
            seller_note = EXCLUDED.seller_note,
            tracking_no = EXCLUDED.tracking_no,
            tracking_url = EXCLUDED.tracking_url,
            shipment_provider = EXCLUDED.shipment_provider,
            shipping_carrier_id = EXCLUDED.shipping_carrier_id,
            shipping_carrier_name = EXCLUDED.shipping_carrier_name,
            buyer_shipping_carrier = EXCLUDED.buyer_shipping_carrier,
            shipping_config_option_id = EXCLUDED.shipping_config_option_id,
            shipping_config_option_name = EXCLUDED.shipping_config_option_name,
            warehouse_id = EXCLUDED.warehouse_id,
            warehouse_name = EXCLUDED.warehouse_name,
            store_site = EXCLUDED.store_site,
            pack_state = EXCLUDED.pack_state,
            item_total_num = EXCLUDED.item_total_num,
            print_label_mark = EXCLUDED.print_label_mark,
            print_bill_mark = EXCLUDED.print_bill_mark,
            print_pick_list_mark = EXCLUDED.print_pick_list_mark,
            print_collect_mark = EXCLUDED.print_collect_mark,
            has_error = EXCLUDED.has_error,
            error_msg = EXCLUDED.error_msg,
            ordered_at = EXCLUDED.ordered_at,
            paid_at = EXCLUDED.paid_at,
            ship_by_at = EXCLUDED.ship_by_at,
            completed_at = EXCLUDED.completed_at,
            deadline_at = EXCLUDED.deadline_at,
            timeout_at = EXCLUDED.timeout_at,
            printed_collect_at = EXCLUDED.printed_collect_at,
            payload = EXCLUDED.payload,
            payload_hash = EXCLUDED.payload_hash,
            synced_at = now(),
            updated_at = now()
        "#,
    )
    .bind(order_id)
    .bind(account_id)
    .bind(m.shop.id)
    .bind(&m.platform)
    .bind(&m.platform_order_id)
    .bind(&m.package_no)
    .bind(&m.package_index)
    .bind(&m.state)
    .bind(&m.platform_state)
    .bind(&m.view_status)
    .bind(&m.marketplace_state)
    .bind(&m.last_order_status)
    .bind(&amount)
    .bind(&m.currency)
    .bind(&m.payment_method)
    .bind(&m.buyer_username)
    .bind(&m.contact_person)
    .bind(&m.recipient_region)
    .bind(&m.buyer_message)
    .bind(&m.seller_note)
    .bind(&m.tracking_no)
    .bind(&m.tracking_url)
    .bind(&m.shipment_provider)
    .bind(m.shipping_carrier_id)
    .bind(&m.shipping_carrier_name)
    .bind(&m.buyer_shipping_carrier)
    .bind(m.shipping_config_option_id)
    .bind(&m.shipping_config_option_name)
    .bind(m.warehouse_id)
    .bind(&m.warehouse_name)
    .bind(&m.store_site)
    .bind(m.pack_state)
    .bind(m.item_total_num)
    .bind(m.print_label_mark)
    .bind(m.print_bill_mark)
    .bind(m.print_pick_list_mark)
    .bind(m.print_collect_mark)
    .bind(m.has_error)
    .bind(&m.error_msg)
    .bind(m.ordered_at)
    .bind(m.paid_at)
    .bind(m.ship_by_at)
    .bind(m.completed_at)
    .bind(m.deadline_at)
    .bind(m.timeout_at)
    .bind(m.printed_collect_at)
    .bind(&m.payload)
    .bind(&m.payload_hash)
    .execute(&mut *tx)
    .await?;

    if state_changed {
        if let Some(ref from) = previous_state {
            sqlx::query(
                r#"
                INSERT INTO order_status_history (order_id, from_state, to_state, source)
                VALUES ($1, $2, $3, 'sync')
                "#,
            )
            .bind(order_id)
            .bind(from)
            .bind(&m.state)
            .execute(&mut *tx)
            .await?;
        }

        // Cancel WA notify only when Summary List was already printed
        // (ops batch membership and/or BigSeller collect print mark).
        if is_cancel_state(&m.state) && !previous_state.as_deref().is_some_and(is_cancel_state) {
            let in_batch: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM batch_orders
                    WHERE order_id = $1
                )
                "#,
            )
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await?;
            let collect_printed =
                m.print_collect_mark.unwrap_or(0) != 0 || m.print_pick_list_mark.unwrap_or(0) != 0;
            if in_batch || collect_printed {
                let cancel_payload = json!({
                    "orderId": order_id,
                    "platformOrderId": m.platform_order_id,
                    "platform": m.platform,
                    "shopId": m.shop.id,
                    "shopName": m.shop.name,
                    "state": m.state,
                    "previousState": previous_state,
                    "printCollectMark": m.print_collect_mark,
                    "printPickListMark": m.print_pick_list_mark,
                    "summaryPrinted": true,
                    "inBatch": in_batch,
                    "buyerShippingCarrier": m.buyer_shipping_carrier,
                    "shipmentProvider": m.shipment_provider,
                    "shippingCarrierName": m.shipping_carrier_name,
                });
                sqlx::query(
                    r#"
                    INSERT INTO notification_outbox (event_type, order_id, platform_order_id, payload, status, account_id)
                    VALUES ('order.canceled', $1, $2, $3, 'pending', $4)
                    "#,
                )
                .bind(order_id)
                .bind(&m.platform_order_id)
                .bind(&cancel_payload)
                .bind(account_id)
                .execute(&mut *tx)
                .await?;
                debug!(
                    order_id,
                    in_batch, collect_printed, "enqueued order.canceled"
                );
            }
        }
    }

    sqlx::query(r#"DELETE FROM order_items WHERE order_id = $1"#)
        .bind(order_id)
        .execute(&mut *tx)
        .await?;

    for it in &m.items {
        let amt = money_str(it.amount);
        let unit = money_str(it.unit_price);
        let orig = money_str(it.original_price);
        sqlx::query(
            r#"
            INSERT INTO order_items (
                id, order_id, line_no, sku, variant_attr, item_name, quantity,
                amount, unit_price, original_price,
                image_url, product_url, platform_item_id, platform_variation_id,
                inventory_sku, is_addition, product_type, payload, synced_at
            ) VALUES (
                $1,$2,$3,$4,$5,$6,$7,
                $8::numeric,$9::numeric,$10::numeric,
                $11,$12,$13,$14,
                $15,$16,$17,$18, now()
            )
            ON CONFLICT (id) DO UPDATE SET
                order_id = EXCLUDED.order_id,
                line_no = EXCLUDED.line_no,
                sku = EXCLUDED.sku,
                variant_attr = EXCLUDED.variant_attr,
                item_name = EXCLUDED.item_name,
                quantity = EXCLUDED.quantity,
                amount = EXCLUDED.amount,
                unit_price = EXCLUDED.unit_price,
                original_price = EXCLUDED.original_price,
                image_url = EXCLUDED.image_url,
                product_url = EXCLUDED.product_url,
                platform_item_id = EXCLUDED.platform_item_id,
                platform_variation_id = EXCLUDED.platform_variation_id,
                inventory_sku = EXCLUDED.inventory_sku,
                is_addition = EXCLUDED.is_addition,
                product_type = EXCLUDED.product_type,
                payload = EXCLUDED.payload,
                synced_at = now()
            "#,
        )
        .bind(it.id)
        .bind(order_id)
        .bind(it.line_no)
        .bind(&it.sku)
        .bind(&it.variant_attr)
        .bind(&it.item_name)
        .bind(it.quantity)
        .bind(&amt)
        .bind(&unit)
        .bind(&orig)
        .bind(&it.image_url)
        .bind(&it.product_url)
        .bind(&it.platform_item_id)
        .bind(&it.platform_variation_id)
        .bind(&it.inventory_sku)
        .bind(it.is_addition)
        .bind(it.product_type)
        .bind(&it.payload)
        .execute(&mut *tx)
        .await?;
    }

    if is_new {
        let notify_payload = json!({
            "orderId": order_id,
            "platformOrderId": m.platform_order_id,
            "platform": m.platform,
            "shopId": m.shop.id,
            "shopName": m.shop.name,
            "amount": m.amount,
            "currency": m.currency,
            "state": m.state,
            "buyerUsername": m.buyer_username,
            "itemTotalNum": m.item_total_num,
            "buyerShippingCarrier": m.buyer_shipping_carrier,
            "shipmentProvider": m.shipment_provider,
            "shippingCarrierName": m.shipping_carrier_name,
        });
        sqlx::query(
            r#"
            INSERT INTO notification_outbox (event_type, order_id, platform_order_id, payload, status, account_id)
            VALUES ('order.created', $1, $2, $3, 'pending', $4)
            "#,
        )
        .bind(order_id)
        .bind(&m.platform_order_id)
        .bind(&notify_payload)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
        debug!(order_id, "enqueued order.created");
    }

    tx.commit().await?;

    Ok(UpsertOutcome {
        order_id,
        is_new,
        state_changed,
        previous_state,
    })
}

pub async fn begin_sync_run(pool: &PgPool, kind: &str, account_id: Option<i64>) -> Result<i64> {
    let row = sqlx::query(
        r#"INSERT INTO sync_runs (kind, status, account_id) VALUES ($1, 'running', $2) RETURNING id"#,
    )
    .bind(kind)
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn finish_sync_run(
    pool: &PgPool,
    id: i64,
    status: &str,
    pages: i32,
    rows: i32,
    error_text: Option<&str>,
    meta: Value,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE sync_runs
        SET status = $2,
            finished_at = now(),
            pages_fetched = $3,
            rows_upserted = $4,
            error_text = $5,
            meta = $6
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(pages)
    .bind(rows)
    .bind(error_text)
    .bind(meta)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public read models (loka-points consumer)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderItemDto {
    pub id: i64,
    pub sku: Option<String>,
    pub variant_attr: Option<String>,
    pub item_name: Option<String>,
    pub quantity: i32,
    pub amount: Option<String>,
    pub unit_price: Option<String>,
    pub image_url: Option<String>,
    pub platform_item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderDetailDto {
    pub id: i64,
    pub shop_id: i64,
    pub shop_name: Option<String>,
    pub platform: String,
    pub platform_order_id: String,
    pub package_no: Option<String>,
    pub state: String,
    pub platform_state: Option<String>,
    pub view_status: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub payment_method: Option<String>,
    pub buyer_username: Option<String>,
    pub contact_person: Option<String>,
    pub recipient_region: Option<String>,
    pub tracking_no: Option<String>,
    pub shipment_provider: Option<String>,
    pub item_total_num: Option<i32>,
    pub print_label_mark: Option<i16>,
    pub print_bill_mark: Option<i16>,
    pub print_collect_mark: Option<i16>,
    pub print_pick_list_mark: Option<i16>,
    pub has_error: bool,
    pub ordered_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub ship_by_at: Option<DateTime<Utc>>,
    pub first_seen_at: DateTime<Utc>,
    pub synced_at: DateTime<Utc>,
    pub items: Vec<OrderItemDto>,
}

fn opt_numeric(row: &sqlx::postgres::PgRow, col: &str) -> Option<String> {
    let v: Option<String> = row.try_get(col).ok().flatten();
    v
}

pub async fn find_by_platform_order_id(
    pool: &PgPool,
    platform_order_id: &str,
    shop_id: Option<i64>,
    platform: Option<&str>,
    account_id: Option<i64>,
) -> Result<Vec<OrderDetailDto>> {
    let rows = sqlx::query(
        r#"
        SELECT
            o.id, o.shop_id, s.name AS shop_name, o.platform, o.platform_order_id,
            o.package_no, o.state, o.platform_state, o.view_status,
            o.amount::text AS amount, o.currency, o.payment_method,
            o.buyer_username, o.contact_person, o.recipient_region,
            o.tracking_no, o.shipment_provider, o.item_total_num,
            o.print_label_mark, o.print_bill_mark, o.print_collect_mark, o.print_pick_list_mark,
            o.has_error, o.ordered_at, o.paid_at, o.ship_by_at,
            o.first_seen_at, o.synced_at
        FROM orders o
        LEFT JOIN shops s ON s.id = o.shop_id
        WHERE o.platform_order_id = $1
          AND ($2::bigint IS NULL OR o.shop_id = $2)
          AND ($3::text IS NULL OR o.platform = $3)
          AND ($4::bigint IS NULL OR o.account_id = $4)
        ORDER BY o.ordered_at DESC NULLS LAST
        LIMIT 20
        "#,
    )
    .bind(platform_order_id)
    .bind(shop_id)
    .bind(platform)
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.get("id");
        let items = load_items(pool, id).await?;
        out.push(OrderDetailDto {
            id,
            shop_id: row.get("shop_id"),
            shop_name: row.get("shop_name"),
            platform: row.get("platform"),
            platform_order_id: row.get("platform_order_id"),
            package_no: row.get("package_no"),
            state: row.get("state"),
            platform_state: row.get("platform_state"),
            view_status: row.get("view_status"),
            amount: opt_numeric(&row, "amount"),
            currency: row.get("currency"),
            payment_method: row.get("payment_method"),
            buyer_username: row.get("buyer_username"),
            contact_person: row.get("contact_person"),
            recipient_region: row.get("recipient_region"),
            tracking_no: row.get("tracking_no"),
            shipment_provider: row.get("shipment_provider"),
            item_total_num: row.get("item_total_num"),
            print_label_mark: row.get("print_label_mark"),
            print_bill_mark: row.get("print_bill_mark"),
            print_collect_mark: row.get("print_collect_mark"),
            print_pick_list_mark: row.get("print_pick_list_mark"),
            has_error: row.get("has_error"),
            ordered_at: row.get("ordered_at"),
            paid_at: row.get("paid_at"),
            ship_by_at: row.get("ship_by_at"),
            first_seen_at: row.get("first_seen_at"),
            synced_at: row.get("synced_at"),
            items,
        });
    }
    Ok(out)
}

async fn load_items(pool: &PgPool, order_id: i64) -> Result<Vec<OrderItemDto>> {
    let rows = sqlx::query(
        r#"
        SELECT id, sku, variant_attr, item_name, quantity,
               amount::text AS amount, unit_price::text AS unit_price,
               COALESCE(
                   NULLIF(image_url, ''),
                   NULLIF(payload->>'image', ''),
                   NULLIF(payload->>'imgUrl', ''),
                   NULLIF(payload->>'imageUrl', '')
               ) AS image_url,
               platform_item_id,
               COALESCE(
                   NULLIF(item_name, ''),
                   NULLIF(payload->>'itemName', ''),
                   NULLIF(payload->>'productName', ''),
                   NULLIF(payload->>'title', ''),
                   sku
               ) AS display_name
        FROM order_items
        WHERE order_id = $1
        ORDER BY line_no ASC
        "#,
    )
    .bind(order_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| OrderItemDto {
            id: row.get("id"),
            sku: row.get("sku"),
            variant_attr: row.get("variant_attr"),
            item_name: row.get("display_name"),
            quantity: row.get("quantity"),
            amount: opt_numeric(&row, "amount"),
            unit_price: opt_numeric(&row, "unit_price"),
            image_url: row.get("image_url"),
            platform_item_id: row.get("platform_item_id"),
        })
        .collect())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelReportOrder {
    pub id: i64,
    pub platform: String,
    pub platform_order_id: String,
    pub shop_name: Option<String>,
    pub state: String,
    pub view_status: Option<String>,
    pub amount: Option<String>,
    pub print_label_mark: Option<i16>,
    pub print_collect_mark: Option<i16>,
    pub print_bill_mark: Option<i16>,
    pub print_pick_list_mark: Option<i16>,
    pub printed_any: bool,
    pub ordered_at: Option<DateTime<Utc>>,
    pub synced_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelDailyReport {
    pub date: NaiveDate,
    pub total: i64,
    pub printed_collect: i64,
    pub printed_label: i64,
    pub printed_any: i64,
    pub not_printed: i64,
    pub orders: Vec<CancelReportOrder>,
}

/// In-cancel / canceled orders for a calendar day (Asia/Jakarta by default via date bounds UTC).
///
/// Includes:
/// - `state` in (canceled, cancelled)
/// - or payload.inCancel truthy
/// - filtered by ordered_at (or first_seen_at) falling on `date` in the given timezone offset hours.
pub async fn cancel_daily_report(
    pool: &PgPool,
    date: NaiveDate,
    tz_offset_hours: i32,
) -> Result<CancelDailyReport> {
    let start_utc = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| Error::Other(format!("invalid calendar date bounds for {date}")))?
        .and_utc()
        - chrono::Duration::hours(tz_offset_hours as i64);
    let end_utc = start_utc + chrono::Duration::days(1);

    let rows = sqlx::query(
        r#"
        SELECT
            o.id, o.platform, o.platform_order_id, s.name AS shop_name,
            o.state, o.view_status, o.amount::text AS amount,
            o.print_label_mark, o.print_collect_mark, o.print_bill_mark, o.print_pick_list_mark,
            o.ordered_at, o.synced_at
        FROM orders o
        LEFT JOIN shops s ON s.id = o.shop_id
        WHERE (
            o.state IN ('canceled', 'cancelled')
            OR COALESCE((o.payload->>'inCancel')::boolean, false) = true
            OR COALESCE(o.payload->>'inCancel', '') IN ('1', 'true', 'True')
            OR o.view_status ILIKE '%cancel%'
            OR o.marketplace_state ILIKE '%cancel%'
        )
        AND COALESCE(o.ordered_at, o.first_seen_at) >= $1
        AND COALESCE(o.ordered_at, o.first_seen_at) < $2
        ORDER BY o.ordered_at DESC NULLS LAST
        LIMIT 5000
        "#,
    )
    .bind(start_utc)
    .bind(end_utc)
    .fetch_all(pool)
    .await?;

    let mut orders = Vec::with_capacity(rows.len());
    let mut printed_collect = 0i64;
    let mut printed_label = 0i64;
    let mut printed_any = 0i64;

    for row in rows {
        let pl: Option<i16> = row.get("print_label_mark");
        let pc: Option<i16> = row.get("print_collect_mark");
        let pb: Option<i16> = row.get("print_bill_mark");
        let pp: Option<i16> = row.get("print_pick_list_mark");
        let any = [pl, pc, pb, pp].into_iter().flatten().any(|m| m != 0);
        if pc.unwrap_or(0) != 0 {
            printed_collect += 1;
        }
        if pl.unwrap_or(0) != 0 {
            printed_label += 1;
        }
        if any {
            printed_any += 1;
        }
        orders.push(CancelReportOrder {
            id: row.get("id"),
            platform: row.get("platform"),
            platform_order_id: row.get("platform_order_id"),
            shop_name: row.get("shop_name"),
            state: row.get("state"),
            view_status: row.get("view_status"),
            amount: opt_numeric(&row, "amount"),
            print_label_mark: pl,
            print_collect_mark: pc,
            print_bill_mark: pb,
            print_pick_list_mark: pp,
            printed_any: any,
            ordered_at: row.get("ordered_at"),
            synced_at: row.get("synced_at"),
        });
    }

    let total = orders.len() as i64;
    Ok(CancelDailyReport {
        date,
        total,
        printed_collect,
        printed_label,
        printed_any,
        not_printed: total - printed_any,
        orders,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxEvent {
    pub id: i64,
    pub event_type: String,
    pub order_id: Option<i64>,
    pub platform_order_id: Option<String>,
    pub payload: Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
}

pub async fn list_events_since(
    pool: &PgPool,
    since_id: i64,
    limit: i64,
) -> Result<Vec<OutboxEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT id, event_type, order_id, platform_order_id, payload, status, created_at, sent_at
        FROM notification_outbox
        WHERE id > $1
        ORDER BY id ASC
        LIMIT $2
        "#,
    )
    .bind(since_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| OutboxEvent {
            id: row.get("id"),
            event_type: row.get("event_type"),
            order_id: row.get("order_id"),
            platform_order_id: row.get("platform_order_id"),
            payload: row.get("payload"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            sent_at: row.get("sent_at"),
        })
        .collect())
}

pub async fn claim_pending_outbox(pool: &PgPool, limit: i64) -> Result<Vec<OutboxEvent>> {
    // No FOR UPDATE without a long-lived txn — simple poll is enough for single worker.
    let rows = sqlx::query(
        r#"
        SELECT id, event_type, order_id, platform_order_id, payload, status, created_at, sent_at
        FROM notification_outbox
        WHERE status = 'pending' AND available_at <= now()
        ORDER BY id ASC
        LIMIT $1
        "#,
    )
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| OutboxEvent {
            id: row.get("id"),
            event_type: row.get("event_type"),
            order_id: row.get("order_id"),
            platform_order_id: row.get("platform_order_id"),
            payload: row.get("payload"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            sent_at: row.get("sent_at"),
        })
        .collect())
}

pub async fn mark_outbox_sent(pool: &PgPool, id: i64) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE notification_outbox
        SET status = 'sent', sent_at = now(), attempts = attempts + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_outbox_failed(pool: &PgPool, id: i64, err: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE notification_outbox
        SET status = CASE WHEN attempts + 1 >= 10 THEN 'failed' ELSE 'pending' END,
            attempts = attempts + 1,
            last_error = $2,
            available_at = now() + (interval '1 minute' * LEAST(attempts + 1, 30))
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(err)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_cursor(pool: &PgPool, key: &str) -> Result<Option<Value>> {
    let row = sqlx::query(r#"SELECT value FROM sync_cursors WHERE key = $1"#)
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

pub async fn set_cursor(pool: &PgPool, key: &str, value: Value) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO sync_cursors (key, value, updated_at)
        VALUES ($1, $2, now())
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
