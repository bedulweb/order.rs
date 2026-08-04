//! Dump the daily infographic SVG to stdout for debugging.
use orders::config::Config;
use orders::daily_infographic;
use orders::db;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let report = daily_infographic::load_daily_infographic(&pool, None, None).await?;
    let svg = daily_infographic::to_svg(&report)?;
    println!("{svg}");
    Ok(())
}
