//! PostgreSQL pool (Neon).

use crate::error::{Error, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

pub async fn connect(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15))
        .connect(database_url)
        .await
        .map_err(|e| Error::Db(e.to_string()))
}

pub async fn ping(pool: &PgPool) -> Result<()> {
    sqlx::query("SELECT 1")
        .execute(pool)
        .await
        .map_err(|e| Error::Db(e.to_string()))?;
    Ok(())
}
