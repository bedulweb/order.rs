//! Read-only probe for BigSeller's server-side label preparation flow.
//!
//! Usage:
//!   cargo run --example probe_resi_print -- 123 456
//!
//! The probe calls checkPrintInfo and getPuidNew, but never calls
//! confirmLabelPrint and never updates the local database.

use orders::{config::Config, resi::probe_resi_print};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let order_ids = std::env::args()
        .skip(1)
        .map(|arg| arg.parse::<i64>())
        .collect::<Result<Vec<_>, _>>()?;

    if order_ids.is_empty() {
        return Err("pass at least one internal BigSeller order id".into());
    }

    let result = probe_resi_print(&cfg.base_url, &cfg.session_path, &order_ids).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
