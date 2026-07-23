//! Outbox → WhatsApp group notifications (instant + printed-cancel via Wazapin).

use crate::batch::{carrier_display, format_wib, is_urgent_carrier};
use crate::cancel_notify::{self, CancelItem, CancelOrder};
use crate::error::{Error, Result};
use crate::instant_notify::{self, NotifyItem, NotifyOrder};
use crate::product_names::{self, catalog_map_from_pairs};
use crate::store::{self, OutboxEvent};
use crate::wazapin::WazapinClient;
use serde_json::Value;
use sqlx::{PgPool, Row};
use tracing::{info, warn};

/// True when outbox payload (or DB row) is urgent/instant shipping.
pub fn payload_is_urgent(payload: &Value) -> bool {
    let buyer = payload
        .get("buyerShippingCarrier")
        .or_else(|| payload.get("buyer_shipping_carrier"))
        .and_then(|v| v.as_str());
    let ship = payload
        .get("shipmentProvider")
        .or_else(|| payload.get("shipment_provider"))
        .and_then(|v| v.as_str());
    let name = payload
        .get("shippingCarrierName")
        .or_else(|| payload.get("shipping_carrier_name"))
        .and_then(|v| v.as_str());
    is_urgent_carrier(buyer, ship, name)
}

/// Build caption for WA image (agreed copy + deadline).
pub fn instant_caption(order: &NotifyOrder) -> String {
    let when = order.ordered_at_wib.as_deref().unwrap_or("—");
    format!("Instant Gaiss..\nDeadline {when}\nThank youuu")
}

/// Load one order + line items as notify card input.
pub async fn load_notify_order(pool: &PgPool, order_id: i64) -> Result<NotifyOrder> {
    let row = sqlx::query(
        r#"
        SELECT id, platform_order_id, platform,
               buyer_shipping_carrier, shipment_provider, shipping_carrier_name,
               ordered_at, state
        FROM orders
        WHERE id = $1
        "#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| Error::Other(format!("order {order_id} not found")))?;

    let buyer: Option<String> = row.get("buyer_shipping_carrier");
    let ship: Option<String> = row.get("shipment_provider");
    let cname: Option<String> = row.get("shipping_carrier_name");
    let ordered: Option<chrono::DateTime<chrono::Utc>> = row.get("ordered_at");

    let catalog = match sqlx::query(r#"SELECT art, name FROM product_catalog"#)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => catalog_map_from_pairs(
            rows.into_iter()
                .map(|r| (r.get::<String, _>("art"), r.get::<String, _>("name"))),
        ),
        Err(_) => Default::default(),
    };

    let item_rows = sqlx::query(
        r#"
        SELECT sku, variant_attr,
               COALESCE(
                   NULLIF(item_name, ''),
                   NULLIF(payload->>'itemName', ''),
                   NULLIF(payload->>'productName', ''),
                   NULLIF(payload->>'title', '')
               ) AS raw_name,
               COALESCE(
                   NULLIF(image_url, ''),
                   NULLIF(payload->>'imgUrl', ''),
                   NULLIF(payload->>'image', ''),
                   NULLIF(payload->>'cosImage', '')
               ) AS image_url,
               quantity
        FROM order_items
        WHERE order_id = $1
        ORDER BY line_no ASC
        "#,
    )
    .bind(order_id)
    .fetch_all(pool)
    .await?;

    let mut items = Vec::new();
    for r in item_rows {
        let sku: Option<String> = r.get("sku");
        let raw: Option<String> = r.get("raw_name");
        let name = product_names::resolve_display_name(
            sku.as_deref().unwrap_or(""),
            raw.as_deref(),
            &catalog,
        );
        items.push(NotifyItem {
            sku,
            name: Some(name),
            variant_attr: r.get("variant_attr"),
            image_url: r.get("image_url"),
            quantity: r.get::<i32, _>("quantity"),
        });
    }

    Ok(NotifyOrder {
        order_id: Some(row.get("id")),
        platform_order_id: row.get("platform_order_id"),
        platform: row.get("platform"),
        carrier: carrier_display(buyer.as_deref(), ship.as_deref(), cname.as_deref()),
        is_urgent: Some(is_urgent_carrier(
            buyer.as_deref(),
            ship.as_deref(),
            cname.as_deref(),
        )),
        ordered_at_wib: ordered.map(format_wib),
        state: row.get("state"),
        items,
    })
}

/// Render instant card PNG + send to Wazapin group.
pub async fn send_instant_notify(
    pool: &PgPool,
    wazapin: &WazapinClient,
    order_id: i64,
) -> Result<String> {
    let order = load_notify_order(pool, order_id).await?;
    if order.items.is_empty() {
        warn!(order_id, "instant notify: no line items");
    }
    let caption = instant_caption(&order);
    let png = instant_notify::render_notify_png(vec![order.clone()]).await?;
    let fname = format!(
        "instant-{}.png",
        order
            .platform_order_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(24)
            .collect::<String>()
    );
    let r = wazapin.send_png_bytes(&png, &fname, &caption).await?;
    info!(
        order_id,
        platform_order_id = %order.platform_order_id,
        msg_id = %r.id,
        "instant notify sent"
    );
    Ok(r.id)
}

/// Caption for cancel card / daily cancel list (no @mention).
pub fn cancel_caption(_order: &CancelOrder) -> String {
    cancel_list_caption(1)
}

/// Caption when the PNG lists one or more cancels for the day.
pub fn cancel_list_caption(order_count: usize) -> String {
    let _ = order_count;
    "list cancel hari ini gais".into()
}

/// Load one canceled order as cancel-card input.
pub async fn load_cancel_order(pool: &PgPool, order_id: i64) -> Result<CancelOrder> {
    let row = sqlx::query(
        r#"
        SELECT id, platform_order_id, platform,
               buyer_shipping_carrier, shipment_provider, shipping_carrier_name,
               ordered_at, synced_at, state, view_status,
               payload->>'cancelReason' AS cancel_reason,
               payload->>'cancel_reason' AS cancel_reason_snake,
               payload->'cancelInfo'->>'reason' AS cancel_info_reason
        FROM orders
        WHERE id = $1
        "#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| Error::Other(format!("order {order_id} not found")))?;

    let buyer: Option<String> = row.get("buyer_shipping_carrier");
    let ship: Option<String> = row.get("shipment_provider");
    let cname: Option<String> = row.get("shipping_carrier_name");
    let ordered: Option<chrono::DateTime<chrono::Utc>> = row.get("ordered_at");
    let synced: Option<chrono::DateTime<chrono::Utc>> = row.get("synced_at");

    let reason = ["cancel_reason", "cancel_reason_snake", "cancel_info_reason"]
        .into_iter()
        .find_map(|k| {
            row.try_get::<Option<String>, _>(k)
                .ok()
                .flatten()
                .filter(|s| !s.trim().is_empty())
        });

    let catalog = match sqlx::query(r#"SELECT art, name FROM product_catalog"#)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => catalog_map_from_pairs(
            rows.into_iter()
                .map(|r| (r.get::<String, _>("art"), r.get::<String, _>("name"))),
        ),
        Err(_) => Default::default(),
    };

    let item_rows = sqlx::query(
        r#"
        SELECT sku, variant_attr,
               COALESCE(
                   NULLIF(item_name, ''),
                   NULLIF(payload->>'itemName', ''),
                   NULLIF(payload->>'productName', ''),
                   NULLIF(payload->>'title', '')
               ) AS raw_name,
               COALESCE(
                   NULLIF(image_url, ''),
                   NULLIF(payload->>'imgUrl', ''),
                   NULLIF(payload->>'image', ''),
                   NULLIF(payload->>'cosImage', '')
               ) AS image_url,
               quantity
        FROM order_items
        WHERE order_id = $1
        ORDER BY line_no ASC
        "#,
    )
    .bind(order_id)
    .fetch_all(pool)
    .await?;

    let mut items = Vec::new();
    for r in item_rows {
        let sku: Option<String> = r.get("sku");
        let raw: Option<String> = r.get("raw_name");
        let name = product_names::resolve_display_name(
            sku.as_deref().unwrap_or(""),
            raw.as_deref(),
            &catalog,
        );
        items.push(CancelItem {
            sku,
            name: Some(name),
            variant_attr: r.get("variant_attr"),
            image_url: r.get("image_url"),
            quantity: r.get::<i32, _>("quantity"),
        });
    }

    Ok(CancelOrder {
        order_id: Some(row.get("id")),
        platform_order_id: row.get("platform_order_id"),
        platform: row.get("platform"),
        carrier: carrier_display(buyer.as_deref(), ship.as_deref(), cname.as_deref()),
        ordered_at_wib: ordered.map(format_wib),
        canceled_at_wib: synced.map(format_wib),
        state: row.get("state"),
        cancel_reason: reason,
        items,
    })
}

/// Render cancel card PNG + send to Wazapin group.
/// Only allowed when Summary List was already printed (batch and/or collect mark).
pub async fn send_cancel_notify(
    pool: &PgPool,
    wazapin: &WazapinClient,
    order_id: i64,
) -> Result<String> {
    if !store::order_summary_was_printed(pool, order_id).await? {
        return Err(Error::Other(format!(
            "order {order_id}: cancel notify skipped — summary not printed yet"
        )));
    }
    let order = load_cancel_order(pool, order_id).await?;
    send_cancel_orders(wazapin, vec![order]).await
}

/// Render + send a multi-order cancel list card (skips per-order summary gate).
/// Used for daily digest / simulations.
pub async fn send_cancel_orders(
    wazapin: &WazapinClient,
    orders: Vec<CancelOrder>,
) -> Result<String> {
    if orders.is_empty() {
        return Err(Error::Other("cancel notify: no orders".into()));
    }
    for o in &orders {
        if o.items.is_empty() {
            warn!(
                platform_order_id = %o.platform_order_id,
                "cancel notify: no line items"
            );
        }
    }
    let caption = cancel_list_caption(orders.len());
    let n = orders.len();
    let png = cancel_notify::render_cancel_png(orders).await?;
    let fname = format!("cancel-list-{n}.png");
    let r = wazapin.send_png_bytes(&png, &fname, &caption).await?;
    info!(order_count = n, msg_id = %r.id, "cancel list notify sent");
    Ok(r.id)
}

/// Handle one outbox row for Wazapin (instant created or printed cancel).
/// Returns true if this event was fully handled here (caller should mark sent/failed).
pub async fn try_handle_outbox_wazapin(
    pool: &PgPool,
    wazapin: &WazapinClient,
    ev: &OutboxEvent,
) -> Result<bool> {
    match ev.event_type.as_str() {
        "order.created" => {
            if !wazapin.config().enabled_for_instant() {
                return Ok(false);
            }
            let mut urgent = payload_is_urgent(&ev.payload);
            if !urgent {
                if let Some(oid) = ev.order_id {
                    if let Ok(o) = load_notify_order(pool, oid).await {
                        urgent = o.is_urgent.unwrap_or(false);
                    }
                }
            }
            if !urgent {
                return Ok(false);
            }
            let oid = ev
                .order_id
                .ok_or_else(|| Error::Other("outbox order.created missing order_id".into()))?;
            send_instant_notify(pool, wazapin, oid).await?;
            Ok(true)
        }
        "order.canceled" => {
            if !wazapin.config().enabled_for_cancel() {
                return Ok(false);
            }
            let oid = ev
                .order_id
                .ok_or_else(|| Error::Other("outbox order.canceled missing order_id".into()))?;
            // Defense in depth: only if still marked summary-printed.
            if !store::order_summary_was_printed(pool, oid).await? {
                info!(
                    order_id = oid,
                    "order.canceled outbox skipped — not summary-printed"
                );
                return Ok(true); // mark sent; no WA spam for unprinted cancels
            }
            send_cancel_notify(pool, wazapin, oid).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_spx_instant_payload() {
        let p = json!({
            "buyerShippingCarrier": "SPX Instant",
            "platformOrderId": "x"
        });
        assert!(payload_is_urgent(&p));
        let p2 = json!({ "buyerShippingCarrier": "J&T Express" });
        assert!(!payload_is_urgent(&p2));
    }

    #[test]
    fn caption_has_order_id() {
        let o = NotifyOrder {
            order_id: Some(1),
            platform_order_id: "26072195X9S7EJ".into(),
            platform: "shopee".into(),
            carrier: Some("SPX Instant".into()),
            is_urgent: Some(true),
            ordered_at_wib: Some("2026-07-21 20:47:04 WIB".into()),
            state: Some("new".into()),
            items: vec![NotifyItem {
                sku: Some("A".into()),
                name: Some("X".into()),
                variant_attr: None,
                image_url: None,
                quantity: 2,
            }],
        };
        let c = instant_caption(&o);
        assert!(c.contains("Instant Gaiss"));
        assert!(c.contains("Deadline"));
        assert!(c.contains("2026-07-21 20:47:04 WIB"));
        assert!(c.contains("Thank youuu"));
    }

    #[test]
    fn cancel_caption_marks_summary() {
        let o = CancelOrder {
            order_id: Some(1),
            platform_order_id: "2607207F868BUW".into(),
            platform: "shopee".into(),
            carrier: Some("J&T Express".into()),
            ordered_at_wib: Some("2026-07-21 04:28:19 WIB".into()),
            canceled_at_wib: Some("2026-07-21 10:00:00 WIB".into()),
            state: Some("canceled".into()),
            cancel_reason: Some("Buyer cancel".into()),
            items: vec![CancelItem {
                sku: Some("A".into()),
                name: Some("X".into()),
                variant_attr: None,
                image_url: None,
                quantity: 1,
            }],
        };
        let c = cancel_caption(&o);
        assert_eq!(c, "list cancel hari ini gais");
        assert_eq!(cancel_list_caption(7), "list cancel hari ini gais");
    }
}
