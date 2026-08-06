//! Email an existing batch's Summary List PDF (resend) — e.g. after changing
//! the printer email address. Recipient defaults to `RESEND_TO`; pass an
//! address after `--` to override.
//!
//! ```bash
//! # today's (WIB) latest morning batch → RESEND_TO
//! cargo run --release --example resend_batch_email
//! # a specific batch → RESEND_TO
//! cargo run --release --example resend_batch_email <batch-uuid>
//! # a specific batch → explicit address
//! cargo run --release --example resend_batch_email <batch-uuid> -- someone@example.com
//! ```

use orders::batch;
use orders::config::Config;
use orders::db;
use orders::email;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;

    // First positional arg = batch id (optional); after "--" = recipient.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut batch_arg: Option<String> = None;
    let mut to_arg: Option<String> = None;
    let mut after_sep = false;
    for a in args {
        if a == "--" {
            after_sep = true;
            continue;
        }
        if !after_sep && a.starts_with("--") {
            continue;
        }
        if !after_sep && batch_arg.is_none() {
            batch_arg = Some(a);
            continue;
        }
        if to_arg.is_none() {
            to_arg = Some(a);
        }
    }
    let to = to_arg
        .or_else(|| cfg.smtp.as_ref().and_then(|e| e.to.clone()))
        .ok_or("no recipient: set RESEND_TO or pass <to> arg")?;

    let pool = db::connect(cfg.require_database_url()?).await?;

    let id: Uuid = if let Some(s) = batch_arg {
        Uuid::parse_str(&s)?
    } else {
        // Today's (WIB) latest morning batch.
        let today = chrono::Utc::now().with_timezone(&batch::wib_offset()).date_naive();
        let batches = batch::list_batches_for_wib_date(&pool, today, None).await?;
        batches
            .iter()
            .find(|b| b.session == "morning")
            .map(|b| b.id)
            .ok_or("no morning batch today")?
    };

    let (filename, bytes) = batch::get_batch_pdf(&pool, id)
        .await?
        .ok_or("batch pdf not found / not ready")?;

    let email_cfg = cfg.smtp.as_ref().ok_or("RESEND_API_KEY not set")?;
    let subject = format!("Summary List 2-up (resend) — batch {id}");
    let msg_id = email::send_pdf_only(email_cfg, &to, &subject, &bytes, &filename).await?;
    println!("resent batch {id} -> {to} msg_id={msg_id}");
    Ok(())
}
