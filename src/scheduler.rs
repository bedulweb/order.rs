//! Scheduled ops jobs run by the worker on a WIB clock:
//!
//! - **Batch pagi** (`BATCH_PAGI_HOUR`/`BATCH_PAGI_MINUTE`, default 08:00 WIB):
//!   claim the backlog and render the 2-up Summary List PDF. Printer email and
//!   automatic BigSeller printed marking are controlled by `AUTO_PRINT=false`
//!   by default; the group notification remains independent. Runs once per WIB
//!   day.
//! - **Batch siang** (`BATCH_SIANG_HOUR`/`BATCH_SIANG_MINUTE`, default 13:05 WIB):
//!   same flow for the afternoon session (backlog accumulated since morning).
//!   Runs once per WIB day.
//! - **Rekap sore** (`REKAP_HOUR`/`REKAP_MINUTE`, default 17:00 WIB): send the
//!   daily infographic PNG to the WhatsApp group. Runs once per WIB day.
//! - **Cancel printed**: send a cancel card for today's (ordered_at in WIB day)
//!   cancels whose Summary List was already printed. Runs once per WIB day,
//!   right after the cancel sync window.
//! - **Instant batch** (`INSTANT_BATCH_INTERVAL_MIN`, default 5): within
//!   working hours, combine pending urgent `order.created` notifications into
//!   one card per interval (instead of one WA message per order).

use crate::batch::{self, BatchSession};
use crate::config::Config;
use crate::email;
use crate::error::Result;
use crate::wazapin::{WazapinClient, WazapinConfig};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Timelike, Utc};
use sqlx::{PgPool, Row};

pub const WIB_OFFSET: i32 = 7 * 3600;

pub struct Scheduler {
    pub pool: PgPool,
    pub cfg: Config,
    /// True once per WIB day for each job (guards against duplicate runs).
    pub ran_batch_pagi_day: Option<u32>,
    pub ran_batch_siang_day: Option<u32>,
    pub ran_rekap_day: Option<u32>,
    pub ran_cancel_printed_day: Option<u32>,
    /// Last instant-batch run (periodic within working hours).
    pub last_instant_batch_at: Option<DateTime<Utc>>,
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

/// True when the periodic instant batch should fire: never run before
/// (`None`) or at least `interval_min` minutes after the last run.
fn instant_batch_due(last: Option<DateTime<Utc>>, now: DateTime<Utc>, interval_min: u64) -> bool {
    match last {
        None => true,
        Some(t) => now >= t + Duration::minutes(interval_min as i64),
    }
}

/// One worker-tick check: run each job whose WIB clock has passed its target
/// hour and hasn't run yet today. Returns true if anything ran.
pub async fn tick(s: &mut Scheduler) -> Result<bool> {
    let mut ran = false;
    let yday = wib_yday();

    let (bh, bm) = env_hhmm("BATCH_PAGI_HOUR", (8, 0));
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

    let (sh, sm) = env_hhmm("BATCH_SIANG_HOUR", (13, 5));
    if due(sh, sm) && s.ran_batch_siang_day != Some(yday) {
        match run_batch_siang(s).await {
            Ok(()) => {
                s.ran_batch_siang_day = Some(yday);
                ran = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "batch siang job failed");
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

    // Instant batch: combine pending urgent order.created notifications into
    // one card per interval (default 5 min), only within working hours — so a
    // morning rush produces a single combined card instead of one WA message
    // per order (same pattern as the daily cancel list).
    let ib_interval: u64 = std::env::var("INSTANT_BATCH_INTERVAL_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
        .max(1);
    let now = Utc::now();
    if crate::batch::is_within_working_hours(
        now,
        crate::batch::WORK_HOUR_START_MIN,
        crate::batch::WORK_HOUR_END_MIN,
    ) && instant_batch_due(s.last_instant_batch_at, now, ib_interval)
    {
        match run_instant_batch(s).await {
            Ok(()) => {
                s.last_instant_batch_at = Some(now);
                ran = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "instant batch job failed");
                // Keep timer unset → retry on the next tick (~60s).
            }
        }
    }

    Ok(ran)
}

/// Batch pagi: claim backlog and render the 2-up PDF; printer delivery is
/// controlled by `AUTO_PRINT`.
pub async fn run_batch_pagi(s: &Scheduler) -> Result<()> {
    run_batch_email(s, BatchSession::Morning).await
}

/// Batch siang: same flow for the afternoon session (backlog since morning).
pub async fn run_batch_siang(s: &Scheduler) -> Result<()> {
    run_batch_email(s, BatchSession::Afternoon).await
}

/// Shared morning/afternoon batch flow: claim backlog and render the 2-up
/// Summary List PDF. Automatic printer email + BigSeller `printed` marking are
/// controlled by `AUTO_PRINT` (disabled by default); the group notification is
/// independent and remains enabled when configured.
pub async fn run_batch_email(s: &Scheduler, session: BatchSession) -> Result<()> {
    let email_target = if s.cfg.auto_print {
        let email_cfg = s.cfg.smtp.as_ref().ok_or_else(|| {
            crate::error::Error::Other("RESEND_API_KEY not set for batch email".into())
        })?;
        let to = email_cfg.to.as_deref().ok_or_else(|| {
            crate::error::Error::Other("RESEND_TO not set for batch email".into())
        })?;
        Some((email_cfg, to))
    } else {
        None
    };

    // Empty backlog → treat the session as done (no retry storm, no empty PDF).
    if !batch::has_backlog(&s.pool, None).await? {
        tracing::info!(session = %session.as_str(), "batch: tidak ada backlog, skip email");
        return Ok(());
    }

    let detail = batch::create_batch(&s.pool, session, None).await?;

    // Reuse the stored (2-up) PDF bytes — same file the web UI downloads and
    // the resend tool emails — instead of rendering a second copy.
    let (pdf_filename, bytes) = batch::get_batch_pdf(&s.pool, detail.summary.id)
        .await?
        .ok_or_else(|| crate::error::Error::Other("batch pdf not stored".into()))?;

    let subject = format!(
        "Summary List 2-up ({}) — {} pesanan · {} urgent",
        session.as_str(),
        detail.summary.order_count,
        detail.summary.urgent_count
    );

    if let Some((email_cfg, to)) = email_target {
        // Only the automatic printer path marks orders as printed. Manual print
        // endpoints keep their existing behavior and are independent of this flag.
        let ids: Vec<i64> = detail.members.iter().map(|m| m.order_id).collect();
        if let Err(e) =
            batch::mark_summary_printed(&s.cfg.base_url, &s.cfg.session_path, &ids).await
        {
            tracing::warn!(error = %e, count = ids.len(), "batch auto-print mark failed (email sent anyway)");
        }

        let msg_id = email::send_pdf_only(email_cfg, to, &subject, &bytes, &pdf_filename).await?;
        tracing::info!(
            batch_id = %detail.summary.id,
            session = %session.as_str(),
            orders = detail.summary.order_count,
            msg_id = %msg_id,
            "batch printer email sent"
        );
    } else {
        tracing::info!(
            batch_id = %detail.summary.id,
            session = %session.as_str(),
            orders = detail.summary.order_count,
            "batch auto-print disabled; PDF remains available for manual printing"
        );
    }

    // Send the actual Summary List PDF to the WhatsApp group as a document.
    // This notification is independent of the automatic printer setting.
    if let Some(wz_cfg) = s.cfg.wazapin.clone().or_else(WazapinConfig::from_env) {
        if wz_cfg.enabled_for_batch() {
            match WazapinClient::new(wz_cfg) {
                Ok(client) => {
                    if let Err(e) = crate::notify::send_batch_pdf_to_group(
                        &client,
                        &bytes,
                        &pdf_filename,
                        &subject,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "batch pdf to group failed (email sent anyway)");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "batch wazapin client init failed"),
            }
        }
    }

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
    let r = client.send_png_bytes(&png, &fname, &caption, false).await?;
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
    if !client.config().enabled_for_cancel() {
        return Ok(());
    }

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

/// Instant batch: combine all pending urgent `order.created` notifications
/// into ONE combined card (default every 5 min within working hours) instead
/// of one WA message per order — same "query + one card" pattern as cancel.
pub async fn run_instant_batch(s: &Scheduler) -> Result<()> {
    let wz_cfg = s
        .cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or_else(|| crate::error::Error::Other("WAZAPIN not set for instant batch".into()))?;
    let client = WazapinClient::new(wz_cfg)?;
    if !client.config().enabled_for_instant() {
        return Ok(());
    }

    // Cap per card so a huge rush does not produce a giant PNG; the remainder
    // stays pending and goes out on the next interval.
    const MAX_ORDERS: usize = 50;
    let events = crate::store::list_pending_instant_outbox(&s.pool, 200).await?;
    let urgent: Vec<_> = events
        .into_iter()
        .filter(|ev| crate::notify::payload_is_urgent(&ev.payload))
        .take(MAX_ORDERS)
        .collect();

    if urgent.is_empty() {
        tracing::info!("instant batch: tidak ada pesanan instant menunggu");
        return Ok(());
    }

    let mut orders = Vec::new();
    let mut sent_ids = Vec::new();
    for ev in &urgent {
        let Some(oid) = ev.order_id else {
            continue;
        };
        match crate::notify::load_notify_order(&s.pool, oid).await {
            Ok(o) => {
                orders.push(o);
                sent_ids.push(ev.id);
            }
            Err(e) => {
                tracing::warn!(
                    outbox_id = ev.id,
                    order_id = oid,
                    error = %e,
                    "instant batch: load order failed"
                );
                crate::store::mark_outbox_failed(&s.pool, ev.id, &e.to_string()).await?;
            }
        }
    }

    if orders.is_empty() {
        return Ok(());
    }
    let msg_id = crate::notify::send_instant_orders(&client, orders).await?;
    for id in &sent_ids {
        crate::store::mark_outbox_sent(&s.pool, *id).await?;
    }
    tracing::info!(
        msg_id = %msg_id,
        count = sent_ids.len(),
        "instant batch sent"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn instant_batch_due_never_run_is_due() {
        assert!(instant_batch_due(None, Utc::now(), 5));
    }

    #[test]
    fn instant_batch_due_respects_interval() {
        let now = Utc.with_ymd_and_hms(2026, 8, 5, 1, 0, 0).unwrap();
        let last = now - Duration::minutes(4);
        assert!(!instant_batch_due(Some(last), now, 5));
        let last = now - Duration::minutes(5);
        assert!(instant_batch_due(Some(last), now, 5));
        let last = now - Duration::minutes(30);
        assert!(instant_batch_due(Some(last), now, 5));
    }

    #[test]
    fn instant_batch_due_handles_day_rollover() {
        // Last run yesterday 23:59, now today 00:01 → elapsed > interval.
        let last = Utc.with_ymd_and_hms(2026, 8, 4, 16, 59, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 5, 0, 1, 0).unwrap();
        assert!(instant_batch_due(Some(last), now, 5));
    }
}
