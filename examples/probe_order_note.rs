//! Probe: tampilkan kolom note/message + payload mentah untuk satu order.
//! Usage: cargo run --example probe_order_note -- <platform_order_id>

use sqlx::Row;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "BS2798050543".into());
    let url = std::env::var("DATABASE_URL")?;
    let pool = sqlx::PgPool::connect(&url).await?;

    let rows = sqlx::query(
        r#"
        SELECT o.id, o.platform_order_id, o.package_no, o.state, o.view_status,
               o.buyer_message, o.seller_note, o.payload::text AS payload
        FROM orders o
        WHERE o.platform_order_id = $1
           OR o.package_no = $1
           OR o.payload::text LIKE '%' || $1 || '%'
        "#,
    )
    .bind(&id)
    .fetch_all(&pool)
    .await?;

    if rows.is_empty() {
        println!("no rows for {id} (platform_order_id / package_no / payload)");
        return Ok(());
    }

    for r in &rows {
        let oid: i64 = r.get("id");
        let payload: String = r.get("payload");
        let buyer_msg: Option<String> = r.get("buyer_message");
        let seller_note: Option<String> = r.get("seller_note");
        println!("id={oid}");
        println!(
            "platform_order_id={}",
            r.get::<String, _>("platform_order_id")
        );
        println!("package_no={:?}", r.get::<Option<String>, _>("package_no"));
        println!(
            "state={:?} view_status={:?}",
            r.get::<Option<String>, _>("state"),
            r.get::<Option<String>, _>("view_status")
        );
        println!("buyer_message={buyer_msg:?}");
        println!("seller_note={seller_note:?}");

        // Keys payload yang terlihat seperti note/message/remark.
        let v: serde_json::Value = serde_json::from_str(&payload)?;
        let mut interesting = Vec::new();
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                let kl = k.to_ascii_lowercase();
                if kl.contains("note")
                    || kl.contains("message")
                    || kl.contains("remark")
                    || kl.contains("buyer")
                {
                    interesting.push((k.clone(), val.to_string()));
                }
            }
        }
        if interesting.is_empty() {
            println!("payload: tidak ada key note/message/remark");
        } else {
            println!("payload keys menarik:");
            for (k, val) in interesting {
                println!("  {k} = {val}");
            }
        }

        let items = sqlx::query(
            "SELECT sku, item_name, variant_attr, quantity FROM order_items WHERE order_id = $1 ORDER BY line_no",
        )
        .bind(oid)
        .fetch_all(&pool)
        .await?;
        println!("items:");
        for it in items {
            println!(
                "  sku={:?} name={:?} variant={:?} qty={}",
                it.get::<Option<String>, _>("sku"),
                it.get::<Option<String>, _>("item_name"),
                it.get::<Option<String>, _>("variant_attr"),
                it.get::<i32, _>("quantity")
            );
        }
        println!("---");
    }
    Ok(())
}
