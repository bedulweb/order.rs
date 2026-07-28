//! Are there processing orders whose label is not yet printed?
use orders::config::Config;
use orders::db;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let r = sqlx::query(
        r#"
        SELECT
          count(*)::bigint AS total,
          count(*) FILTER (WHERE coalesce(print_label_mark, 0) = 0)::bigint AS unprinted,
          count(*) FILTER (WHERE coalesce(print_label_mark, 0) <> 0)::bigint AS printed
        FROM orders
        WHERE state IN ('processing', 'pickup', 'platformProcessing')
        "#,
    )
    .fetch_one(&pool)
    .await?;
    println!("processing: total={} unprinted={} printed={}",
        r.get::<i64,_>("total"), r.get::<i64,_>("unprinted"), r.get::<i64,_>("printed"));
    for r in sqlx::query(
        r#"
        SELECT platform_order_id, platform,
               coalesce(nullif(btrim(buyer_shipping_carrier),''), nullif(btrim(shipment_provider),''), nullif(btrim(shipping_carrier_name),''), '?') AS carrier
        FROM orders
        WHERE state IN ('processing','pickup','platformProcessing')
          AND coalesce(print_label_mark, 0) = 0
        LIMIT 8
        "#,
    ).fetch_all(&pool).await? {
        println!("  belum cetak: {} [{}] {}",
            r.get::<String,_>("platform_order_id"),
            r.get::<Option<String>,_>("platform").unwrap_or_default(),
            r.get::<String,_>("carrier"));
    }
    Ok(())
}
