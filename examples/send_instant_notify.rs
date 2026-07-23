//! Manual: send instant notify PNG for one order id to Wazapin group.
//!
//! ```bash
//! cargo run --example send_instant_notify -- 14479354211
//! ```

use orders::config::Config;
use orders::db;
use orders::notify;
use orders::wazapin::{WazapinClient, WazapinConfig};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let order_id: i64 = env::args()
        .nth(1)
        .ok_or("usage: send_instant_notify <order_id>")?
        .parse()?;

    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let wz_cfg = cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or("WAZAPIN_API_KEY / CHANNEL / GROUP not set")?;
    let client = WazapinClient::new(wz_cfg)?;

    println!("sending instant notify for order_id={order_id} …");
    let msg_id = notify::send_instant_notify(&pool, &client, order_id).await?;
    println!("ok msg_id={msg_id}");
    Ok(())
}
