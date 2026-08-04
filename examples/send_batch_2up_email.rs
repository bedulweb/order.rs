//! Generate a 2-up (two pages per A4 sheet) Summary List PDF from the real
//! order backlog in Postgres and email it as an attachment.
//!
//! Recipient comes from `RESEND_TO` (env) or the first CLI arg.
//!
//! ```bash
//! cargo run --release --example send_batch_2up_email
//! cargo run --release --example send_batch_2up_email -- someone@example.com
//! ```

use orders::batch::{self, BatchSession, BacklogOrder, PdfOrderLine};
use orders::batch_pdf::render_batch_pdf_2up;
use orders::config::Config;
use orders::db;
use orders::email;
use sqlx::Row;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    // Recipient = first positional (non-flag) argument, else RESEND_TO env.
    // `--count N` is consumed so its value is never mistaken for the address.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let count = args
        .iter()
        .position(|a| a == "--count")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<i64>().ok());
    let to = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with("--")
                && !(*i > 0 && args[*i - 1] == "--count")
        })
        .map(|(_, a)| a.clone())
        .next()
        .or_else(|| cfg.smtp.as_ref().and_then(|e| e.to.clone()))
        .ok_or("no recipient: set RESEND_TO or pass <to> arg")?;
    let pool = db::connect(cfg.require_database_url()?).await?;

    // Real backlog: eligible orders (state=new, not in any active batch).
    let backlog = batch::list_backlog(&pool, None, 200).await?;
    let mut orders: Vec<BacklogOrder> = backlog.orders.clone();
    if let Some(n) = count {
        if (orders.len() as i64) < n {
            // Pull the newest `new` orders straight from the DB so the PDF has
            // ~n rows (real data, no placeholders).
            let rows = sqlx::query(
                r#"
                SELECT o.id, o.platform_order_id, o.platform,
                       o.buyer_shipping_carrier, o.shipment_provider, o.shipping_carrier_name,
                       o.ordered_at, o.item_total_num
                FROM orders o
                WHERE o.state IN ('new', 'processing')
                ORDER BY o.ordered_at DESC NULLS LAST, o.id DESC
                LIMIT $1
                "#,
            )
            .bind(n)
            .fetch_all(&pool)
            .await?;
            orders = rows
                .iter()
                .map(|r| {
                    let buyer: Option<String> = r.get("buyer_shipping_carrier");
                    let ship: Option<String> = r.get("shipment_provider");
                    let name: Option<String> = r.get("shipping_carrier_name");
                    let carrier = batch::carrier_display(buyer.as_deref(), ship.as_deref(), name.as_deref());
                    let is_urgent = batch::is_urgent_carrier(buyer.as_deref(), ship.as_deref(), name.as_deref());
                    BacklogOrder {
                        order_id: r.get("id"),
                        platform_order_id: r.get("platform_order_id"),
                        platform: r.get("platform"),
                        carrier,
                        buyer_shipping_carrier: buyer,
                        shipment_provider: ship,
                        shipping_carrier_name: name,
                        is_urgent,
                        ordered_at: r.get("ordered_at"),
                        item_total_num: r.get("item_total_num"),
                    }
                })
                .collect();
        }
    }
    if orders.is_empty() {
        return Err("no orders found".into());
    }
    println!("orders used={} (backlog total={} urgent={})", orders.len(), backlog.total, backlog.urgent_count);

    // Load line items for those orders, then build the PDF lines like
    // batch.rs::finalize_batch does.
    let ids: Vec<i64> = orders.iter().map(|o| o.order_id).collect();
    let items_map = batch::load_items_for_orders(&pool, &ids).await?;

    let pdf_lines: Vec<PdfOrderLine> = orders
        .iter()
        .map(|o| PdfOrderLine {
            platform_order_id: o.platform_order_id.clone(),
            platform: o.platform.clone(),
            carrier: o.carrier.clone().unwrap_or_else(|| "-".into()),
            is_urgent: o.is_urgent,
            ordered_at_wib: o.ordered_at.map(batch::format_wib).unwrap_or_else(|| "-".into()),
            items: items_map.get(&o.order_id).cloned().unwrap_or_default(),
        })
        .collect();

    let order_count = pdf_lines.len() as i32;
    let urgent_count = orders.iter().filter(|o| o.is_urgent).count() as i32;
    let n_sku: i32 = pdf_lines.iter().flat_map(|l| &l.items).count() as i32;

    let bytes = render_batch_pdf_2up(
        Uuid::new_v4(),
        BatchSession::Morning,
        &batch::format_wib(chrono::Utc::now()),
        order_count,
        urgent_count,
        &pdf_lines,
    )
    .await?;

    std::fs::create_dir_all("logs")?;
    let path = "logs/batch-2up-live.pdf";
    std::fs::write(path, &bytes)?;
    println!("wrote {path} ({} bytes, {order_count} orders)", bytes.len());

    let email_cfg = cfg.smtp.as_ref().ok_or("RESEND_API_KEY not set")?;
    let subject = format!("Summary List 2-up — {order_count} pesanan · {n_sku} item");
    // Empty body: Epson Email Print prints the attachment, not the body text.
    let msg_id = email::send_pdf_only(email_cfg, &to, &subject, &bytes, "summary-list-2up.pdf").await?;
    println!("email sent msg_id={msg_id}");
    Ok(())
}
