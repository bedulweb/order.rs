//! Apply docs/sql/008_backfill_state_changed_today.sql.
use orders::config::Config;
use orders::db;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let sql = include_str!("../docs/sql/008_backfill_state_changed_today.sql");
    let res = sqlx::raw_sql(sql).execute(&pool).await?;
    println!("008 applied");
    let _ = res;
    Ok(())
}
