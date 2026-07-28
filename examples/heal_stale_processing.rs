//! One-off: heal rows still state=processing whose state_changed_at predates
//! today (or is NULL) — they left BigSeller's processing bucket without us
//! ever seeing the transition. Search each one and upsert its true state.
use orders::config::Config;
use orders::db;
use orders::sync::{self, SyncContext};
use orders::{OrdersApi, SessionData};
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let session = SessionData::load(&cfg.session_path)?;
    let api = OrdersApi::new(&cfg.base_url, &session)?;
    let account = sync::bootstrap_account(&pool, &cfg).await?;
    let ctx = SyncContext {
        account_id: Some(account.id),
        account_code: Some(account.code.clone()),
    };

    let rows = sqlx::query(
        r#"
        SELECT platform_order_id
        FROM orders
        WHERE state IN ('processing', 'pickup', 'platformProcessing')
          AND (state_changed_at IS NULL
               OR state_changed_at < ((timezone('Asia/Jakarta', now()))::date)::timestamp AT TIME ZONE 'Asia/Jakarta')
        "#,
    )
    .fetch_all(&pool)
    .await?;
    let total = rows.len();
    println!("stale processing rows: {total}");

    let mut healed = 0usize;
    let mut failed = 0usize;
    for (i, r) in rows.iter().enumerate() {
        let no: String = r.get("platform_order_id");
        match sync::sync_one_order_no(&pool, &api, &no, &ctx).await {
            Ok(s) => {
                if s.state_changed > 0 {
                    healed += 1;
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("  {no}: {e}");
            }
        }
        if (i + 1) % 25 == 0 {
            println!("  progress {}/{} (healed {healed}, failed {failed})", i + 1, total);
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    println!("DONE: {total} searched, {healed} healed, {failed} failed");
    Ok(())
}
