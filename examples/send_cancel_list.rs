//! Manual: send multi-order cancel list PNG to Wazapin group.
//!
//! ```bash
//! cargo run --release --example send_cancel_list -- 12736681502 12736708427 ...
//! ```

use orders::config::Config;
use orders::db;
use orders::notify;
use orders::wazapin::{WazapinClient, WazapinConfig};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let ids: Vec<i64> = env::args()
        .skip(1)
        .map(|s| s.parse::<i64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "usage: send_cancel_list <order_id> [order_id…]")?;
    if ids.is_empty() {
        return Err("usage: send_cancel_list <order_id> [order_id…]".into());
    }

    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let wz_cfg = cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or("WAZAPIN_API_KEY / CHANNEL / GROUP not set")?;
    let client = WazapinClient::new(wz_cfg)?;

    let mut orders = Vec::new();
    for id in &ids {
        let o = notify::load_cancel_order(&pool, *id).await?;
        println!(
            "  + {} · {} ({} items)",
            o.platform_order_id,
            o.platform,
            o.items.len()
        );
        orders.push(o);
    }

    println!("sending cancel list n={} …", orders.len());
    let msg_id = notify::send_cancel_orders(&client, orders).await?;
    println!("ok msg_id={msg_id}");
    Ok(())
}
