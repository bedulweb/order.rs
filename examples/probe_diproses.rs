//! What are today's Diproses orders and when were they really packed?
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
        SELECT platform_order_id, platform, state,
               coalesce(nullif(btrim(buyer_shipping_carrier),''), nullif(btrim(shipment_provider),''), '?') AS carrier,
               to_char(timezone('Asia/Jakarta', state_changed_at), 'DD/MM HH24:MI') AS changed_wib,
               payload->>'packTimeStr' AS pack_time,
               coalesce(print_label_mark, 0) AS label_mark
        FROM orders
        WHERE state IN ('processing','pickup','platformProcessing')
          AND state_changed_at >= ((timezone('Asia/Jakarta', now()))::date)::timestamp AT TIME ZONE 'Asia/Jakarta'
        ORDER BY state_changed_at ASC
        "#,
    ).fetch_all(&pool).await?;
    println!("total: {}", rows.len());
    for r in &rows {
        println!("{} [{}] {} carrier={} changed={} packTime={} label={}",
            r.get::<String,_>("platform_order_id"),
            r.get::<Option<String>,_>("platform").unwrap_or_default(),
            r.get::<String,_>("state"),
            r.get::<String,_>("carrier"),
            r.get::<Option<String>,_>("changed_wib").unwrap_or_default(),
            r.get::<Option<String>,_>("pack_time").unwrap_or_else(|| "-".into()),
            r.get::<i32,_>("label_mark"));
    }
    Ok(())
}
