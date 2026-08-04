//! Scheduled ops jobs run by the worker on a WIB clock:
//!
//! - **Batch pagi** (`BATCH_PAGI_HOUR`/`BATCH_PAGI_MINUTE`, default 07:50 WIB):
//!   claim the backlog into a morning batch, render the 2-up Summary List PDF,
//!   auto-mark it printed in BigSeller, and email it to `RESEND_TO` (e.g. the
//!   Epson email-print address). Runs once per WIB day.
//! - **Rekap sore** (`REKAP_HOUR`/`REKAP_MINUTE`, default 17:00 WIB): send the
//!   daily infographic PNG to the WhatsApp group. Runs once per WIB day.
//! - **Cancel printed**: send a cancel card for today's (ordered_at in WIB day)
//!   cancels whose Summary List was already printed. Runs once per WIB day,
//!   right after the cancel sync window.

use crate::batch::{self, BatchSession};
use crate::config::Config;
use crate::email;
use crate::error::Result;
use crate::wazapin::{WazapinClient, WazapinConfig};
use chrono::{Datelike, Duration, FixedOffset, Timelike, Utc};
use sqlx::{PgPool, Row};

pub const WIB_OFFSET: i32 = 7 * 3600;

pub struct Scheduler {
    pub pool: PgPool,
    pub cfg: Config,
    /// True once per WIB day for each job (guards against duplicate runs).
    pub ran_batch_pagi_day: Option<u32>,
    pub ran_rekap_day: Option<u32>,
    pub ran_cancel_printed_day: Option<u32>,
}

/// WIB "yday" (ordinal within the year) for dedupe — changes at WIB midnight.
fn wib_yday() -> u32 {
    let wib = FixedOffset::east_opt(WIB_OFFSET).expect("WIB offset");
    Utc::now().with_timezone(&wib).ordinal()
}

fn wib_now_min() -> u32 {
    let wib = FixedOffset::east_opt(WIB_OFFSET).expect("WIB offset");
    let t = Utc::now().with_timezone(&wib);
    t.hour() * 60 + t.minute()
}

fn due(hour: u32, minute: u32) -> bool {
    let now = wib_now_min();
    let target = (hour * 60 + minute).min(24 * 60 - 1);
    now >= target
}

fn env_hhmm(key: &str, default: (u32, u32)) -> (u32, u32) {
    let raw = match std::env::var(key) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return default,
    };
    let mut parts = raw.split(':');
    let h: u32 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default.0);
    let m: u32 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default.1);
    (h.min(23), m.min(59))
}

/// One worker-tick check: run each job whose WIB clock has passed its target
/// hour and hasn't run yet today. Returns true if anything ran.
pub async fn tick(s: &mut Scheduler) -> Result<bool> {
    let mut ran = false;
    let yday = wib_yday();

    let (bh, bm) = env_hhmm("BATCH_PAGI_HOUR", (7, 50));
    if due(bh, bm) && s.ran_batch_pagi_day != Some(yday) {
        match run_batch_pagi(s).await {
            Ok(()) => {
                s.ran_batch_pagi_day = Some(yday);
                ran = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "batch pagi job failed");
                // Keep retrying later ticks of the same day (e.g. transient DB).
            }
        }
    }

    let (rh, rm) = env_hhmm("REKAP_HOUR", (17, 0));
    if due(rh, rm) && s.ran_rekap_day != Some(yday) {
        match run_rekap_sore(s).await {
            Ok(()) => {
                s.ran_rekap_day = Some(yday);
                ran = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "rekap sore job failed");
            }
        }
    }

    // Cancel-printed follows the cancel sync window (CANCEL_HOUR_LOCAL /
    // CANCEL_MINUTE_LOCAL, default 17:00 WIB). Only cancels with a printed
    // Summary List go out.
    let ch: u32 = std::env::var("CANCEL_HOUR_LOCAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(17)
        .min(23);
    let cm: u32 = std::env::var("CANCEL_MINUTE_LOCAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
        .min(59);
    if due(ch, cm) && s.ran_cancel_printed_day != Some(yday) {
        match run_cancel_printed(s).await {
            Ok(()) => {
                s.ran_cancel_printed_day = Some(yday);
                ran = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "cancel printed job failed");
            }
        }
    }

    Ok(ran)
}

/// Batch pagi: claim backlog → 2-up PDF → auto-mark printed → email to RESEND_TO.
pub async fn run_batch_pagi(s: &Scheduler) -> Result<()> {
    let email_cfg = s.cfg.smtp.as_ref().ok_or_else(|| {
        crate::error::Error::Other("RESEND_API_KEY not set for batch pagi".into())
    })?;
    let to = email_cfg
        .to
        .as_deref()
        .ok_or_else(|| crate::error::Error::Other("RESEND_TO not set for batch pagi".into()))?;

    let detail = batch::create_batch(&s.pool, BatchSession::Morning, None).await?;
    let session = BatchSession::parse(&detail.summary.session)
        .ok_or_else(|| crate::error::Error::Other("invalid session".into()))?;

    let pdf_lines: Vec<batch::PdfOrderLine> = detail
        .members
        .iter()
        .map(|m| batch::PdfOrderLine {
            platform_order_id: m.platform_order_id.clone(),
            platform: m.platform.clone().unwrap_or_else(|| "-".into()),
            carrier: m.carrier_snapshot.clone().unwrap_or_else(|| "-".into()),
            is_urgent: m.is_urgent,
            ordered_at_wib: m
                .ordered_at
                .map(batch::format_wib)
                .unwrap_or_else(|| "-".into()),
            items: m.items.clone(),
        })
        .collect();

    let bytes = crate::batch_pdf::render_batch_pdf_2up(
        detail.summary.id,
        session,
        &detail.summary.created_at_wib,
        detail.summary.order_count,
        detail.summary.urgent_count,
        &pdf_lines,
    )
    .await?;

    // Auto-mark Summary List printed in BigSeller (same as the web flow).
    let ids: Vec<i64> = detail.members.iter().map(|m| m.order_id).collect();
    if let Err(e) = batch::mark_summary_printed(&s.cfg.base_url, &s.cfg.session_path, &ids).await {
        tracing::warn!(error = %e, count = ids.len(), "batch pagi auto-mark failed (email sent anyway)");
    }

    let subject = format!(
        "Summary List 2-up — {} pesanan · {} urgent",
        detail.summary.order_count, detail.summary.urgent_count
    );
    let msg_id = email::send_pdf_only(
        email_cfg,
        to,
        &subject,
        &bytes,
        &detail
            .summary
            .pdf_filename
            .clone()
            .unwrap_or_else(|| "summary-list-2up.pdf".into()),
    )
    .await?;
    tracing::info!(
        batch_id = %detail.summary.id,
        orders = detail.summary.order_count,
        msg_id = %msg_id,
        "batch pagi emailed"
    );
    Ok(())
}

/// Rekap sore: daily infographic PNG → WhatsApp group.
pub async fn run_rekap_sore(s: &Scheduler) -> Result<()> {
    let wz_cfg = s
        .cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or_else(|| crate::error::Error::Other("WAZAPIN not set for rekap sore".into()))?;
    let client = WazapinClient::new(wz_cfg)?;

    let report = crate::daily_infographic::load_daily_infographic(&s.pool, None, None).await?;
    let png = crate::daily_infographic::render_png(&report)?;
    let caption = format!(
        "Rekap {} WIB — {} pesanan · {} barang · GMV {}",
        report.date,
        report.current.order_count,
        report.current.qty,
        crate::daily_infographic::fmt_rp(report.current.gmv),
    );
    let fname = format!("rekap-{}.png", report.date);
    let r = client.send_png_bytes(&png, &fname, &caption).await?;
    tracing::info!(date = %report.date, msg_id = %r.id, "rekap sore sent");
    Ok(())
}

/// Cancel printed: today's (ordered_at WIB) cancels whose Summary List was
/// already printed → cancel card to WhatsApp group. Mirrors the worker's
/// cancel-notify gate so unprinted cancels never spam the group.
pub async fn run_cancel_printed(s: &Scheduler) -> Result<()> {
    let wz_cfg = s
        .cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or_else(|| crate::error::Error::Other("WAZAPIN not set for cancel printed".into()))?;
    let client = WazapinClient::new(wz_cfg)?;

    let wib = FixedOffset::east_opt(WIB_OFFSET).expect("WIB offset");
    let today_wib = Utc::now().with_timezone(&wib).date_naive();
    let day_start = today_wib
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight")
        .and_local_timezone(wib)
        .single()
        .expect("WIB has no DST");
    let start_utc = day_start.with_timezone(&Utc);
    let end_utc = start_utc + Duration::days(1);

    let rows = sqlx::query(
        r#"
        SELECT o.id, o.platform_order_id, o.platform
        FROM orders o
        WHERE (
            o.state IN ('canceled', 'cancelled')
            OR COALESCE((o.payload->>'inCancel')::boolean, false) = true
            OR COALESCE(o.payload->>'inCancel', '') IN ('1', 'true', 'True')
            OR o.view_status ILIKE '%cancel%'
            OR o.marketplace_state ILIKE '%cancel%'
        )
        AND COALESCE(o.ordered_at, o.first_seen_at) >= $1
        AND COALESCE(o.ordered_at, o.first_seen_at) < $2
        ORDER BY o.ordered_at DESC NULLS LAST
        "#,
    )
    .bind(start_utc)
    .bind(end_utc)
    .fetch_all(&s.pool)
    .await?;

    let mut orders = Vec::new();
    for r in &rows {
        let id: i64 = r.get("id");
        if !crate::store::order_summary_was_printed(&s.pool, id).await? {
            continue;
        }
        let o = crate::notify::load_cancel_order(&s.pool, id).await?;
        tracing::info!(
            platform_order_id = %o.platform_order_id,
            "cancel printed queued for card"
        );
        orders.push(o);
    }

    if orders.is_empty() {
        tracing::info!("cancel printed: tidak ada cancel hari ini dengan summary sudah di-print");
        return Ok(());
    }
    let msg_id = crate::notify::send_cancel_orders(&client, orders).await?;
    tracing::info!(msg_id = %msg_id, "cancel printed sent");
    Ok(())
}
