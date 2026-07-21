//! PostgreSQL pool (Neon).

use crate::error::Result;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

pub async fn connect(database_url: &str) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15))
        .connect(database_url)
        .await?)
}

pub async fn ping(pool: &PgPool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
