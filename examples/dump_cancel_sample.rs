//! Dump N latest canceled orders as JSON fixture for cancel-notify playground.
use orders::batch::{carrier_display, format_wib};
use orders::config::Config;
use orders::db;
use orders::product_names::{self, catalog_map_from_pairs};
use serde_json::json;
use sqlx::Row;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let mut limit: i64 = 3;
    let mut out = "examples/fixtures/cancel-notify-sample.json".to_string();

    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--limit" => {
                limit = args
                    .next()
                    .ok_or("--limit needs N")?
                    .parse()
                    .map_err(|_| "bad --limit")?;
            }
            "--out" => {
                out = args.next().ok_or("--out needs path")?;
            }
            other => {
                eprintln!("unknown arg: {other}");
                eprintln!("usage: dump_cancel_sample [--limit N] [--out PATH]");
                std::process::exit(2);
            }
        }
    }

    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.database_url.as_deref().ok_or("DATABASE_URL")?).await?;

    let rows = sqlx::query(
        r#"
        SELECT
            o.id, o.platform_order_id, o.platform,
            o.buyer_shipping_carrier, o.shipment_provider, o.shipping_carrier_name,
            o.ordered_at, o.updated_at, o.state,
            o.payload->>'cancelReason' AS cancel_reason,
            o.payload->>'buyerCancelReason' AS buyer_cancel_reason,
            o.payload->>'cancel_reason' AS cancel_reason2
        FROM orders o
        WHERE lower(coalesce(o.state,'')) IN ('canceled', 'cancelled')
        ORDER BY coalesce(o.updated_at, o.ordered_at) DESC NULLS LAST, o.id DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    println!("canceled orders: {}", rows.len());
    if rows.is_empty() {
        std::fs::create_dir_all("examples/fixtures")?;
        std::fs::write(&out, "[]\n")?;
        println!("wrote {out} (0)");
        return Ok(());
    }

    let catalog = match sqlx::query(r#"SELECT art, name FROM product_catalog"#)
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => catalog_map_from_pairs(
            rows.into_iter()
                .map(|r| (r.get::<String, _>("art"), r.get::<String, _>("name"))),
        ),
        Err(_) => Default::default(),
    };

    let ids: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
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

    let mut by_oid: std::collections::HashMap<i64, Vec<serde_json::Value>> = Default::default();
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

    let mut out_orders = Vec::new();
    for r in &rows {
        let buyer: Option<String> = r.get("buyer_shipping_carrier");
        let ship: Option<String> = r.get("shipment_provider");
        let name: Option<String> = r.get("shipping_carrier_name");
        let oid: i64 = r.get("id");
        let ordered: Option<chrono::DateTime<chrono::Utc>> = r.get("ordered_at");
        let updated: Option<chrono::DateTime<chrono::Utc>> = r.get("updated_at");
        let reason = r
            .get::<Option<String>, _>("cancel_reason")
            .or_else(|| r.get("buyer_cancel_reason"))
            .or_else(|| r.get("cancel_reason2"));
        out_orders.push(json!({
            "orderId": oid,
            "platformOrderId": r.get::<String, _>("platform_order_id"),
            "platform": r.get::<String, _>("platform"),
            "carrier": carrier_display(buyer.as_deref(), ship.as_deref(), name.as_deref()),
            "orderedAtWib": ordered.map(format_wib),
            "canceledAtWib": updated.map(format_wib),
            "state": r.get::<String, _>("state"),
            "cancelReason": reason,
            "items": by_oid.get(&oid).cloned().unwrap_or_default(),
        }));
        println!(
            "  {} | {:?} | items={}",
            out_orders.last().unwrap()["platformOrderId"],
            out_orders.last().unwrap()["carrier"],
            out_orders.last().unwrap()["items"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0)
        );
    }

    std::fs::create_dir_all("examples/fixtures")?;
    std::fs::write(&out, serde_json::to_string_pretty(&out_orders)?)?;
    println!("wrote {out} ({} orders)", out_orders.len());
    Ok(())
}
