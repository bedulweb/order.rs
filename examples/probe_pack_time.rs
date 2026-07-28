//! Probe: does the payload carry a pack/processing timestamp?
use orders::config::Config;
use orders::db;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let rows = sqlx::query(
        "SELECT platform_order_id, payload FROM orders WHERE state IN ('processing','pickup') ORDER BY synced_at DESC LIMIT 3",
    )
    .fetch_all(&pool)
    .await?;
    for r in rows {
        let id: String = r.get("platform_order_id");
        let p: Option<serde_json::Value> = r.get("payload");
        if let Some(p) = p {
            let keys: Vec<&String> = p.as_object().map(|o| o.keys().collect()).unwrap_or_default();
            let time_keys: Vec<String> = keys.iter().filter(|k| k.to_lowercase().contains("time") || k.to_lowercase().contains("pack") || k.to_lowercase().contains("date")).map(|k| format!("{k}={}", &p[*k].to_string()[..40.min(p[*k].to_string().len())])).collect();
            println!("=== {id}");
            for t in time_keys { println!("  {t}"); }
        }
    }
    Ok(())
}
