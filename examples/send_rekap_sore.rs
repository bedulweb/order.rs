//! Manual: daily infographic PNG → optional Wazapin group.
//!
//! ```bash
//! cargo run --release --example send_rekap_sore
//! cargo run --release --example send_rekap_sore -- --send
//! cargo run --release --example send_rekap_sore -- --out logs/rekap-sore-test.png
//! ```

use orders::config::Config;
use orders::daily_infographic;
use orders::db;
use orders::wazapin::{WazapinClient, WazapinConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;

    let mut out: Option<PathBuf> = None;
    let mut send = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--send" => send = true,
            "--out" => {
                let Some(path) = args.next() else {
                    return Err("--out requires a path".into());
                };
                out = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let report = daily_infographic::load_daily_infographic(&pool, None, None).await?;
    println!(
        "date={} orders={} qty={} gmv={} cancel={} fee_est={} ship_est={}",
        report.date,
        report.current.order_count,
        report.current.qty,
        report.current.gmv,
        report.current.cancel_n,
        report.current.fee_est,
        report.current.ship_est
    );

    let png = daily_infographic::render_png(&report)?;
    let out = out.unwrap_or_else(|| daily_infographic::default_png_path(report.date));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &png)?;
    println!("wrote {}", out.display());

    if send {
        let wz_cfg = cfg
            .wazapin
            .clone()
            .or_else(WazapinConfig::from_env)
            .ok_or("WAZAPIN_API_KEY / CHANNEL / GROUP not set")?;
        let client = WazapinClient::new(wz_cfg)?;
        let caption = format!(
            "Rekap hari ini {}\nOmset {} · Order {} · Qty {} · Cancel {}",
            report.date.format("%d/%m/%Y"),
            report.current.gmv.round() as i64,
            report.current.order_count,
            report.current.qty,
            report.current.cancel_n
        );
        let filename = out
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("rekap-sore.png");
        println!("sending …");
        let r = client.send_png_bytes(&png, filename, &caption).await?;
        println!("ok msg_id={}", r.id);
    }

    Ok(())
}
