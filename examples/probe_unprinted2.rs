use orders::config::Config;
use orders::db;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let rows = sqlx::query(
        r#"
        SELECT platform_order_id, platform,
               coalesce(nullif(btrim(buyer_shipping_carrier),''), nullif(btrim(shipment_provider),''), nullif(btrim(shipping_carrier_name),''), '?') AS carrier,
               coalesce(print_label_mark, 0) AS mark
        FROM orders
        WHERE state IN ('processing','pickup','platformProcessing')
          AND state_changed_at >= ((timezone('Asia/Jakarta', now()))::date)::timestamp AT TIME ZONE 'Asia/Jakarta'
        ORDER BY print_label_mark, platform_order_id
        "#,
    ).fetch_all(&pool).await?;
    let mut unprinted = 0;
    for r in &rows {
        let mark: i32 = r.get("mark");
        if mark == 0 {
            unprinted += 1;
            println!("  BELUM: {} [{}] {}", r.get::<String,_>("platform_order_id"), r.get::<Option<String>,_>("platform").unwrap_or_default(), r.get::<String,_>("carrier"));
        }
    }
    println!("hari ini: total={} belum_cetak={}", rows.len(), unprinted);
    Ok(())
}
