//! Manual: carrier day list PNG → Wazapin group.
//! Default day = yesterday Asia/Jakarta.
//!
//! ```bash
//! cargo run --release --example send_carrier_day_list
//! cargo run --release --example send_carrier_day_list -- 2026-07-22
//! ```

use chrono::{Duration, FixedOffset, NaiveDate, Utc};
use orders::carrier_day_list;
use orders::config::Config;
use orders::db;
use orders::wazapin::{WazapinClient, WazapinConfig};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;

    let wib = FixedOffset::east_opt(7 * 3600).unwrap();
    let today_wib = Utc::now().with_timezone(&wib).date_naive();
    let day = match env::args().nth(1) {
        Some(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d")?,
        None => today_wib - Duration::days(1),
    };

    let list = carrier_day_list::load_carrier_day_list(&pool, day, 7).await?;
    println!(
        "day={day} total={} instant={} spx={} jne={} jnt={} sicepat={} other={}",
        list.total_orders,
        list.instant_orders,
        list.spx_orders,
        list.jne_orders,
        list.jnt_orders,
        list.sicepat_orders,
        list.other_orders
    );

    let png = carrier_day_list::render_carrier_day_list_png(&list).await?;
    let out = std::path::PathBuf::from(format!(
        "logs/carrier-day-{}-i{}-spx{}-jne{}-jnt{}-sc{}.png",
        day,
        list.instant_orders,
        list.spx_orders,
        list.jne_orders,
        list.jnt_orders,
        list.sicepat_orders
    ));
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    carrier_day_list::write_carrier_day_list_png(&out, &png)?;
    println!("wrote {}", out.display());

    let wz_cfg = cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or("WAZAPIN_API_KEY / CHANNEL / GROUP not set")?;
    let client = WazapinClient::new(wz_cfg)?;
    let caption = format!(
        "List pesanan {}\nInstant {} · SPX {} · JNE {} · J&T {} · SiCepat {}\nTotal {}",
        day.format("%d/%m/%Y"),
        list.instant_orders,
        list.spx_orders,
        list.jne_orders,
        list.jnt_orders,
        list.sicepat_orders,
        list.total_orders
    );
    let fname = format!("carrier-day-{day}.png");
    println!("sending …");
    let r = client.send_png_bytes(&png, &fname, &caption).await?;
    println!("ok msg_id={}", r.id);
    Ok(())
}
