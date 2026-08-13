//! Manual: kirim PDF Summary List batch ke grup WhatsApp sebagai dokumen
//! (jalur yang sama dengan notifikasi batch otomatis: upload litterbox 72h).
//!
//! ```bash
//! cargo run --release --example send_batch_pdf_group            # batch ready terbaru
//! cargo run --release --example send_batch_pdf_group <batch_id> # batch tertentu
//! ```

use orders::batch;
use orders::config::Config;
use orders::db;
use orders::notify;
use orders::wazapin::{WazapinClient, WazapinConfig};
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let wz_cfg = cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or("WAZAPIN_API_KEY / CHANNEL / GROUP not set")?;
    let client = WazapinClient::new(wz_cfg)?;

    let batch_id = match std::env::args().nth(1) {
        Some(s) => uuid::Uuid::parse_str(&s)?,
        None => {
            let row = sqlx::query(
                "SELECT id FROM batches WHERE status = 'ready' ORDER BY created_at DESC LIMIT 1",
            )
            .fetch_optional(&pool)
            .await?
            .ok_or("tidak ada batch ready di DB")?;
            row.get::<uuid::Uuid, _>("id")
        }
    };

    let (pdf_filename, bytes) = batch::get_batch_pdf(&pool, batch_id)
        .await?
        .ok_or_else(|| format!("batch {batch_id} tidak punya pdf tersimpan"))?;

    let caption = format!("Summary List 2-up — kirim ulang manual (batch {batch_id})");
    println!(
        "sending batch pdf: {} ({} bytes) …",
        pdf_filename,
        bytes.len()
    );
    let msg_id = notify::send_batch_pdf_to_group(&client, &bytes, &pdf_filename, &caption).await?;
    println!("ok msg_id={msg_id}");
    Ok(())
}
