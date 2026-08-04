//! Run the scheduler's jobs once (for testing). All three jobs run regardless
//! of their WIB clock time.
//!
//! ```bash
//! cargo run --release --example run_scheduler_test
//! ```

use orders::config::Config;
use orders::db;
use orders::scheduler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;

    let s = scheduler::Scheduler {
        pool,
        cfg,
        ran_batch_pagi_day: None,
        ran_rekap_day: None,
        ran_cancel_printed_day: None,
    };
    scheduler::run_batch_pagi(&s).await?;
    scheduler::run_rekap_sore(&s).await?;
    scheduler::run_cancel_printed(&s).await?;
    println!("scheduler jobs ok");
    Ok(())
}
