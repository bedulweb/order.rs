use orders::{config::Config, db};
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let rows = sqlx::query(
        "SELECT id FROM orders WHERE tracking_no IS NOT NULL ORDER BY updated_at DESC NULLS LAST LIMIT 3",
    )
    .fetch_all(&pool)
    .await?;
    for row in rows {
        println!("{}", row.get::<i64, _>("id"));
    }
    Ok(())
}
