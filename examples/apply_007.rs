//! Apply docs/sql/007_state_changed_at.sql (no psql on this host).
use orders::config::Config;
use orders::db;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let sql = include_str!("../docs/sql/007_state_changed_at.sql");
    sqlx::raw_sql(sql).execute(&pool).await?;
    println!("007_state_changed_at applied");
    Ok(())
}
