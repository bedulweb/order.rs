//! Dump instant/urgent orders for a WIB calendar day (default: today) → JSON fixture.
use chrono::{FixedOffset, Local, NaiveDate, Utc};
use orders::batch::{carrier_display, format_wib, is_urgent_carrier};
use orders::config::Config;
use orders::db;
use orders::product_names::{self, catalog_map_from_pairs};
use serde_json::json;
use sqlx::Row;
use std::env;

fn wib() -> FixedOffset {
    FixedOffset::east_opt(7 * 3600).expect("WIB")
}

fn parse_day(s: &str) -> Result<NaiveDate, Box<dyn std::error::Error>> {
    Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d")?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let mut day = Local::now().with_timezone(&wib()).date_naive();
    let mut out = format!(
        "examples/fixtures/instant-notify-{}.json",
        day.format("%Y-%m-%d")
    );

    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--date" => {
                day = parse_day(&args.next().ok_or("--date needs YYYY-MM-DD")?)?;
                out = format!(
                    "examples/fixtures/instant-notify-{}.json",
                    day.format("%Y-%m-%d")
                );
            }
            "--out" => {
                out = args.next().ok_or("--out needs path")?;
            }
            other => {
                eprintln!("unknown arg: {other}");
                eprintln!("usage: dump_instant_today [--date YYYY-MM-DD] [--out PATH]");
                std::process::exit(2);
            }
        }
    }

    let start_local = day
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(wib())
        .single()
        .ok_or("ambiguous WIB midnight")?;
    let start_utc = start_local.with_timezone(&Utc);
    let end_utc = start_utc + chrono::Duration::days(1);

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
        WHERE o.ordered_at >= $1 AND o.ordered_at < $2
          AND (
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
          AND lower(coalesce(o.state,'')) NOT IN ('canceled', 'cancelled')
        ORDER BY o.ordered_at DESC NULLS LAST, o.id DESC
        "#,
    )
    .bind(start_utc)
    .bind(end_utc)
    .fetch_all(&pool)
    .await?;

    println!(
        "day={} WIB  window={} .. {}  hits={}",
        day,
        start_utc.to_rfc3339(),
        end_utc.to_rfc3339(),
        rows.len()
    );

    if rows.is_empty() {
        eprintln!("no non-canceled instant/urgent orders for this day");
        // still write empty array for clarity
        std::fs::create_dir_all("examples/fixtures")?;
        std::fs::write(&out, "[]\n")?;
        println!("wrote {out} (0 orders)");
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

    let mut out_orders = Vec::new();
    for r in &rows {
        let buyer: Option<String> = r.get("buyer_shipping_carrier");
        let ship: Option<String> = r.get("shipment_provider");
        let name: Option<String> = r.get("shipping_carrier_name");
        let oid: i64 = r.get("id");
        let ordered: Option<chrono::DateTime<chrono::Utc>> = r.get("ordered_at");
        out_orders.push(json!({
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

    std::fs::create_dir_all("examples/fixtures")?;
    let body = serde_json::to_string_pretty(&out_orders)?;
    std::fs::write(&out, &body)?;
    println!("wrote {out} ({} orders)", out_orders.len());
    for o in &out_orders {
        println!(
            "  {}  {}  items={}",
            o["platformOrderId"].as_str().unwrap_or("?"),
            o["carrier"].as_str().unwrap_or("?"),
            o["items"].as_array().map(|a| a.len()).unwrap_or(0)
        );
    }
    Ok(())
}
