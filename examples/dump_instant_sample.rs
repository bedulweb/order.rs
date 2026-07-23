//! One-shot: dump 3 latest instant/urgent orders as JSON fixture seed.
use orders::batch::{carrier_display, format_wib, is_urgent_carrier};
use orders::config::Config;
use orders::db;
use orders::product_names::{self, catalog_map_from_pairs};
use serde_json::json;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let url = cfg.database_url.as_deref().ok_or("DATABASE_URL required")?;
    let pool = db::connect(url).await?;

    let rows = sqlx::query(
        r#"
        SELECT
            o.id, o.platform_order_id, o.platform,
            o.buyer_shipping_carrier, o.shipment_provider, o.shipping_carrier_name,
            o.ordered_at, o.state, o.item_total_num
        FROM orders o
        WHERE (
            lower(coalesce(o.buyer_shipping_carrier,'')) LIKE ANY (ARRAY[
                '%instant%','%sameday%','%same day%','%same-day%',
                '%prioritas%','%gojek%','%gosend%','%grab%','%paxel%'
            ])
            OR lower(coalesce(o.shipment_provider,'')) LIKE ANY (ARRAY[
                '%instant%','%sameday%','%same day%','%same-day%',
                '%prioritas%','%gojek%','%gosend%','%grab%','%paxel%'
            ])
            OR lower(coalesce(o.shipping_carrier_name,'')) LIKE ANY (ARRAY[
                '%instant%','%sameday%','%same day%','%same-day%',
                '%prioritas%','%gojek%','%gosend%','%grab%','%paxel%'
            ])
        )
        ORDER BY o.ordered_at DESC NULLS LAST, o.id DESC
        LIMIT 3
        "#,
    )
    .fetch_all(&pool)
    .await?;

    if rows.is_empty() {
        eprintln!("no instant/urgent orders found");
        return Ok(());
    }

    let catalog = match sqlx::query(r#"SELECT art, name FROM product_catalog"#)
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => catalog_map_from_pairs(rows.into_iter().map(|r| {
            let art: String = r.get("art");
            let name: String = r.get("name");
            (art, name)
        })),
        Err(_) => std::collections::HashMap::new(),
    };

    let ids: Vec<i64> = rows.iter().map(|r| r.get::<i64, _>("id")).collect();
    let item_rows = sqlx::query(
        r#"
        SELECT order_id, sku, variant_attr,
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
        WHERE order_id = ANY($1)
        ORDER BY order_id, line_no ASC
        "#,
    )
    .bind(&ids)
    .fetch_all(&pool)
    .await?;

    let mut by_oid: std::collections::HashMap<i64, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for r in item_rows {
        let oid: i64 = r.get("order_id");
        let sku: Option<String> = r.get("sku");
        let raw: Option<String> = r.get("raw_name");
        let display = product_names::resolve_display_name(
            sku.as_deref().unwrap_or(""),
            raw.as_deref(),
            &catalog,
        );
        by_oid.entry(oid).or_default().push(json!({
            "sku": sku,
            "name": display,
            "variantAttr": r.get::<Option<String>, _>("variant_attr"),
            "imageUrl": r.get::<Option<String>, _>("image_url"),
            "quantity": r.get::<i32, _>("quantity"),
        }));
    }

    let mut out = Vec::new();
    for r in &rows {
        let buyer: Option<String> = r.get("buyer_shipping_carrier");
        let ship: Option<String> = r.get("shipment_provider");
        let name: Option<String> = r.get("shipping_carrier_name");
        let oid: i64 = r.get("id");
        let ordered: Option<chrono::DateTime<chrono::Utc>> = r.get("ordered_at");
        out.push(json!({
            "orderId": oid,
            "platformOrderId": r.get::<String, _>("platform_order_id"),
            "platform": r.get::<String, _>("platform"),
            "carrier": carrier_display(buyer.as_deref(), ship.as_deref(), name.as_deref()),
            "isUrgent": is_urgent_carrier(buyer.as_deref(), ship.as_deref(), name.as_deref()),
            "orderedAtWib": ordered.map(|d| format_wib(d)),
            "state": r.get::<String, _>("state"),
            "items": by_oid.get(&oid).cloned().unwrap_or_default(),
        }));
    }

    let path = "examples/fixtures/instant-notify-sample.json";
    std::fs::create_dir_all("examples/fixtures")?;
    let body = serde_json::to_string_pretty(&out)?;
    std::fs::write(path, &body)?;
    println!("wrote {path} ({} orders)", out.len());
    println!("{body}");
    Ok(())
}
