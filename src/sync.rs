//! Pull BigSeller pageList buckets into Postgres.

use crate::accounts::{self, Account};
use crate::auth;
use crate::client;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::map::map_order_row;
use crate::ocr::CaptchaOcr;
use crate::orders::{OrderListQuery, OrdersApi};
use crate::session::SessionData;
use crate::store::{
    begin_sync_run, claim_pending_outbox, finish_sync_run, mark_outbox_failed, mark_outbox_sent,
    set_cursor, upsert_order, UpsertOutcome,
};
use chrono::{Datelike, Local, NaiveTime, Timelike, Utc};
use serde_json::json;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct SyncStats {
    pub kind: String,
    pub pages: i32,
    pub upserted: i32,
    pub created: i32,
    pub state_changed: i32,
}

/// Context for a single BigSeller tenant sync.
#[derive(Debug, Clone, Default)]
pub struct SyncContext {
    pub account_id: Option<i64>,
    pub account_code: Option<String>,
}

/// Options for paginated bucket pull.
#[derive(Debug, Clone)]
pub struct SyncBucketOpts {
    pub page_size: u32,
    pub max_pages: u32,
    /// Use historical pageList flags (no packState filter + historyOrder).
    pub historical: bool,
    /// Sleep between BigSeller page requests (rate limit).
    pub page_delay_ms: u64,
}

impl Default for SyncBucketOpts {
    fn default() -> Self {
        Self {
            page_size: 50,
            max_pages: 80,
            historical: false,
            page_delay_ms: 0,
        }
    }
}

/// Fetch one marketplace order number from BigSeller and upsert into Postgres.
///
/// Tries historical all-order search first, then a few common status buckets.
pub async fn sync_one_order_no(
    pool: &PgPool,
    api: &OrdersApi,
    order_no: &str,
    ctx: &SyncContext,
) -> Result<SyncStats> {
    let kind = match &ctx.account_code {
        Some(c) => format!("orders_search:{c}"),
        None => "orders_search".into(),
    };
    let run_id = begin_sync_run(pool, &kind, ctx.account_id).await?;
    let mut upserted = 0i32;
    let mut created = 0i32;
    let mut state_changed = 0i32;
    let mut pages = 0i32;

    let result = async {
        // Prefer the live search shape that BigSeller actually returns for shipped/active
        // orders. Only fall back to historical buckets if the first hit is empty.
        let mut queries = vec![OrderListQuery::search_order_no(order_no)];
        for status in ["shipped", "completed", "processing", "new", "canceled"] {
            let mut q = OrderListQuery::active(status);
            q.search_content = Some(order_no.to_string());
            q.page_size = 20;
            q.pack_state = None;
            q.all_order = true;
            queries.push(q);
            let mut qh = OrderListQuery::historical(status);
            qh.search_content = Some(order_no.to_string());
            qh.page_size = 20;
            queries.push(qh);
        }

        let mut seen_bs_ids = std::collections::HashSet::new();
        for (i, q) in queries.into_iter().enumerate() {
            if i > 0 {
                // BigSeller rate-limits multi-search bursts.
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            pages += 1;
            let page = match api.page_list(&q).await {
                Ok(p) => p,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("frequent") || msg.contains("try again") || msg.contains("2001")
                    {
                        warn!(%msg, "pageList search retry after backoff");
                        tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                        api.page_list(&q).await?
                    } else {
                        return Err(e);
                    }
                }
            };
            if page.rows.is_empty() {
                continue;
            }
            for row in &page.rows {
                let Some(mapped) = map_order_row(row) else {
                    warn!("skip unmappable order row");
                    continue;
                };
                if !mapped.platform_order_id.eq_ignore_ascii_case(order_no)
                    && !mapped
                        .platform_order_id
                        .to_ascii_uppercase()
                        .contains(&order_no.to_ascii_uppercase())
                {
                    // Keep non-exact hits only when search returned a single row.
                    if page.rows.len() > 1 {
                        continue;
                    }
                }
                if !seen_bs_ids.insert(mapped.id) {
                    continue;
                }
                let outcome: UpsertOutcome = upsert_order(pool, &mapped, ctx.account_id).await?;
                upserted += 1;
                if outcome.is_new {
                    created += 1;
                }
                if outcome.state_changed {
                    state_changed += 1;
                }
                info!(
                    order_no = %mapped.platform_order_id,
                    bs_id = mapped.id,
                    state = %mapped.state,
                    "synced order by search"
                );
            }
            if upserted > 0 {
                break;
            }
        }

        Ok::<_, Error>(SyncStats {
            kind: kind.clone(),
            pages,
            upserted,
            created,
            state_changed,
        })
    }
    .await;

    match result {
        Ok(stats) => {
            finish_sync_run(
                pool,
                run_id,
                "ok",
                stats.pages,
                stats.upserted,
                None,
                json!({
                    "orderNo": order_no,
                    "created": stats.created,
                    "stateChanged": stats.state_changed,
                }),
            )
            .await?;
            Ok(stats)
        }
        Err(e) => {
            let _ = finish_sync_run(
                pool,
                run_id,
                "error",
                pages,
                upserted,
                Some(&e.to_string()),
                json!({ "orderNo": order_no }),
            )
            .await;
            Err(e)
        }
    }
}

/// Sync one status bucket (all pages).
pub async fn sync_status_bucket(
    pool: &PgPool,
    api: &OrdersApi,
    status: &str,
    page_size: u32,
    max_pages: u32,
    ctx: &SyncContext,
) -> Result<SyncStats> {
    sync_status_bucket_with(
        pool,
        api,
        status,
        ctx,
        SyncBucketOpts {
            page_size,
            max_pages,
            historical: false,
            page_delay_ms: 0,
        },
    )
    .await
}

/// Sync one status bucket with full options (historical backfill, delays).
pub async fn sync_status_bucket_with(
    pool: &PgPool,
    api: &OrdersApi,
    status: &str,
    ctx: &SyncContext,
    opts: SyncBucketOpts,
) -> Result<SyncStats> {
    let kind = match &ctx.account_code {
        Some(c) if opts.historical => format!("orders_hist:{status}:{c}"),
        Some(c) => format!("orders_status:{status}:{c}"),
        None if opts.historical => format!("orders_hist:{status}"),
        None => format!("orders_status:{status}"),
    };
    let run_id = begin_sync_run(pool, &kind, ctx.account_id).await?;
    let mut pages = 0i32;
    let mut upserted = 0i32;
    let mut created = 0i32;
    let mut state_changed = 0i32;
    let page_size = opts.page_size;
    let max_pages = opts.max_pages;

    let result = async {
        let mut page_no = 1u32;
        loop {
            if page_no > max_pages {
                warn!(status, max_pages, "hit max_pages cap");
                break;
            }
            if page_no > 1 && opts.page_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(opts.page_delay_ms)).await;
            }

            let mut q = if opts.historical {
                OrderListQuery::historical(status)
            } else {
                let mut q = OrderListQuery::active(status);
                // packState="0" means UNPACKED — only the New Orders bucket
                // uses it. Packed buckets (processing/shipped/…) return zero
                // rows with that filter (verified live: processing totalSize=0
                // with packState="0" vs 31 with null), which silently made
                // those bucket syncs no-ops and left stale states behind.
                if status != "new" {
                    q.pack_state = None;
                }
                q
            };
            q.page_no = page_no;
            q.page_size = page_size;

            let page = match api.page_list(&q).await {
                Ok(p) => p,
                Err(e) => {
                    // One retry after backoff on rate limit / transient errors.
                    let msg = e.to_string();
                    if msg.contains("frequent") || msg.contains("try again") || msg.contains("2001")
                    {
                        warn!(%msg, page_no, "pageList retry after backoff");
                        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                        api.page_list(&q).await?
                    } else {
                        return Err(e);
                    }
                }
            };
            pages += 1;

            if page.rows.is_empty() {
                break;
            }

            for row in &page.rows {
                let Some(mapped) = map_order_row(row) else {
                    warn!("skip unmappable order row");
                    continue;
                };
                let outcome: UpsertOutcome = upsert_order(pool, &mapped, ctx.account_id).await?;
                upserted += 1;
                if outcome.is_new {
                    created += 1;
                }
                if outcome.state_changed {
                    state_changed += 1;
                }
            }

            info!(
                status,
                page_no,
                rows = page.rows.len(),
                total = page.total,
                upserted,
                created,
                "bucket page"
            );

            let got = page.rows.len() as u32;
            if got < page_size {
                break;
            }
            if page.total > 0 && (page_no as u64) * (page_size as u64) >= page.total {
                break;
            }
            page_no += 1;
        }

        let cursor_key = match &ctx.account_code {
            Some(c) => format!("last_sync:{status}:{c}"),
            None => format!("last_sync:{status}"),
        };
        set_cursor(
            pool,
            &cursor_key,
            json!({
                "at": Utc::now().to_rfc3339(),
                "pages": pages,
                "upserted": upserted,
                "created": created,
                "historical": opts.historical,
                "accountId": ctx.account_id,
            }),
        )
        .await?;

        Ok::<(), Error>(())
    }
    .await;

    match result {
        Ok(()) => {
            finish_sync_run(
                pool,
                run_id,
                "ok",
                pages,
                upserted,
                None,
                json!({ "created": created, "stateChanged": state_changed }),
            )
            .await?;
            info!(
                status,
                pages, upserted, created, state_changed, "sync bucket done"
            );
            Ok(SyncStats {
                kind,
                pages,
                upserted,
                created,
                state_changed,
            })
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = finish_sync_run(
                pool,
                run_id,
                "error",
                pages,
                upserted,
                Some(&msg),
                json!({}),
            )
            .await;
            Err(e)
        }
    }
}

pub async fn sync_new_orders(
    pool: &PgPool,
    api: &OrdersApi,
    ctx: &SyncContext,
) -> Result<SyncStats> {
    sync_status_bucket(pool, api, "new", 50, 40, ctx).await
}

/// One reconciliation cycle: how many stale rows were examined / healed.
#[derive(Debug, Clone, Default)]
pub struct ReconcileStats {
    pub candidates: i32,
    /// Found in BigSeller and state actually changed (healed).
    pub refreshed: i32,
    /// Not found anywhere and older than 30 days → moved to `archived`.
    pub archived: i32,
    /// Not found but recent — left for a retry next cycle.
    pub not_found: i32,
}

/// Absence-based state reconciliation.
///
/// The worker re-upserts the whole BigSeller `new` bucket every cycle, so a
/// row still `state = 'new'` whose `synced_at` fell behind the last passes
/// was *not seen* — the order left the New Orders bucket (shipped,
/// completed, canceled, …) without ever being re-pulled. Look each such
/// order up through the all-order search and upsert its true current state.
///
/// Capped per cycle and newest-stale first so the one-time backlog drains
/// gently and unfindable ancients sink to the bottom. Only call this after
/// a successful new-bucket pass — absence is only meaningful when the
/// bucket was actually pulled.
pub async fn reconcile_stale_new_orders(
    pool: &PgPool,
    api: &OrdersApi,
    ctx: &SyncContext,
    stale_after_secs: u64,
    cap: i64,
) -> Result<ReconcileStats> {
    let window = format!("{stale_after_secs} seconds");
    let rows = sqlx::query(
        r#"
        SELECT platform_order_id
        FROM orders
        WHERE state = 'new'
          AND synced_at < now() - $2::interval
          AND synced_at > now() - interval '30 days'
          AND ($1::bigint IS NULL OR account_id = $1)
        GROUP BY platform_order_id
        ORDER BY max(synced_at) DESC
        LIMIT $3
        "#,
    )
    .bind(ctx.account_id)
    .bind(&window)
    .bind(cap)
    .fetch_all(pool)
    .await?;

    let mut stats = ReconcileStats::default();
    if rows.is_empty() {
        return Ok(stats);
    }
    stats.candidates = rows.len() as i32;

    let kind = match &ctx.account_code {
        Some(c) => format!("reconcile:{c}"),
        None => "reconcile".into(),
    };
    let run_id = begin_sync_run(pool, &kind, ctx.account_id).await?;

    let result = async {
        for row in rows {
            let pid: String = row.get("platform_order_id");
            // Gentle pacing — BigSeller throttles search bursts
            // ("too frequent") which returns empty results.
            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
            let q = OrderListQuery::search_order_no(&pid);
            let page = match api.page_list(&q).await {
                Ok(p) => p,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("frequent") || msg.contains("try again") || msg.contains("2001")
                    {
                        warn!(%msg, "reconcile search retry after backoff");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        api.page_list(&q).await?
                    } else {
                        return Err(e);
                    }
                }
            };

            let mut found = false;
            for r in &page.rows {
                let Some(mapped) = map_order_row(r) else {
                    continue;
                };
                if !mapped.platform_order_id.eq_ignore_ascii_case(&pid) {
                    continue;
                }
                found = true;
                let outcome = upsert_order(pool, &mapped, ctx.account_id).await?;
                if outcome.state_changed {
                    stats.refreshed += 1;
                    info!(
                        order_no = %pid,
                        from = ?outcome.previous_state,
                        to = %mapped.state,
                        "reconciled stale order"
                    );
                }
            }
            if !found {
                stats.not_found += 1;
                // Orders older than 30 days that BigSeller's search no longer
                // indexes will never be found — archive them instead of
                // retrying every cycle (recent misses stay for retry).
                let archived = sqlx::query(
                    r#"
                    UPDATE orders
                    SET state = 'archived', updated_at = now()
                    WHERE platform_order_id = $1
                      AND state = 'new'
                      AND ordered_at < now() - interval '30 days'
                    "#,
                )
                .bind(&pid)
                .execute(pool)
                .await?;
                if archived.rows_affected() > 0 {
                    stats.archived += 1;
                    stats.not_found -= 1;
                    info!(order_no = %pid, "archived unfindable old order");
                } else {
                    warn!(order_no = %pid, "reconcile: order not found in BigSeller search");
                }
            }
        }
        Ok::<_, Error>(())
    }
    .await;

    match result {
        Ok(()) => {
            finish_sync_run(
                pool,
                run_id,
                "ok",
                stats.candidates,
                stats.refreshed,
                None,
                json!({
                    "candidates": stats.candidates,
                    "refreshed": stats.refreshed,
                    "archived": stats.archived,
                    "notFound": stats.not_found,
                }),
            )
            .await?;
            Ok(stats)
        }
        Err(e) => {
            finish_sync_run(
                pool,
                run_id,
                "error",
                stats.candidates,
                stats.refreshed,
                Some(&e.to_string()),
                json!({}),
            )
            .await
            .ok();
            Err(e)
        }
    }
}

pub async fn sync_cancel_related(
    pool: &PgPool,
    api: &OrdersApi,
    ctx: &SyncContext,
) -> Result<Vec<SyncStats>> {
    let mut out = Vec::new();
    for status in ["canceled", "platformProcessing"] {
        match sync_status_bucket(pool, api, status, 50, 80, ctx).await {
            Ok(s) => out.push(s),
            Err(e) => {
                warn!(status, error = %e, "cancel-related sync failed for bucket");
            }
        }
    }
    Ok(out)
}

/// Full historical backfill across main BigSeller status buckets.
///
/// Uses `historyOrder=true` and no `packState` filter so completed/shipped
/// archives are included (live counts can be thousands of rows).
pub async fn sync_historical_all(
    pool: &PgPool,
    api: &OrdersApi,
    ctx: &SyncContext,
) -> Result<Vec<SyncStats>> {
    // Order: smaller / hot buckets first, huge completed last.
    let buckets = [
        "new",
        "unpaid",
        "platformProcessing",
        "shipped",
        "canceled",
        "completed",
    ];
    let mut out = Vec::new();
    for status in buckets {
        let max_pages = match status {
            "completed" => 500, // ~25k rows @ 50/page
            "canceled" => 120,
            "shipped" => 80,
            _ => 40,
        };
        info!(status, max_pages, "historical bucket start");
        match sync_status_bucket_with(
            pool,
            api,
            status,
            ctx,
            SyncBucketOpts {
                page_size: 50,
                max_pages,
                historical: true,
                page_delay_ms: 1200,
            },
        )
        .await
        {
            Ok(s) => {
                info!(
                    kind = %s.kind,
                    pages = s.pages,
                    upserted = s.upserted,
                    created = s.created,
                    "historical bucket done"
                );
                out.push(s);
            }
            Err(e) => {
                warn!(status, error = %e, "historical bucket failed");
            }
        }
        // pause between buckets
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Worker loops
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub new_interval_secs: u64,
    pub reconcile_cap: i64,
    pub cancel_hour_local: u32,
    pub cancel_minute_local: u32,
    /// Working-hours window (WIB minutes since midnight) for outbox delivery.
    pub work_hour_start_min: u32,
    pub work_hour_end_min: u32,
    pub wa_webhook_url: Option<String>,
    pub wa_webhook_token: Option<String>,
    pub wazapin: Option<crate::wazapin::WazapinConfig>,
    /// Auto re-login when BS returns auth-expired.
    pub auto_relogin: bool,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            new_interval_secs: 60,
            reconcile_cap: 15,
            cancel_hour_local: 17,
            cancel_minute_local: 0,
            work_hour_start_min: crate::batch::WORK_HOUR_START_MIN,
            work_hour_end_min: crate::batch::WORK_HOUR_END_MIN,
            wa_webhook_url: None,
            wa_webhook_token: None,
            wazapin: None,
            auto_relogin: true,
        }
    }
}

struct WorkerState {
    cfg: Config,
    app_cfg: WorkerConfig,
    pool: PgPool,
    api: OrdersApi,
    account: Account,
    ocr: Option<Arc<CaptchaOcr>>,
}

impl WorkerState {
    async fn ensure_api(&mut self) -> Result<()> {
        match self.api.is_login().await {
            Ok(true) => {
                let _ = accounts::mark_session_checked(&self.pool, self.account.id, true).await;
                return Ok(());
            }
            Ok(false) => {
                warn!("isLogin=false — will re-login if enabled");
            }
            Err(e) if client::is_auth_error(&e) => {
                warn!(error = %e, "session probe auth error");
            }
            Err(e) => {
                warn!(error = %e, "isLogin probe failed (continuing)");
                return Ok(());
            }
        }
        self.relogin().await
    }

    async fn relogin(&mut self) -> Result<()> {
        if !self.app_cfg.auto_relogin {
            return Err(Error::AuthExpired(
                "session expired; auto_relogin disabled — run `orders login`".into(),
            ));
        }
        let ocr = match &self.ocr {
            Some(o) => o.clone(),
            None => {
                self.cfg.validate_paths()?;
                let o = CaptchaOcr::load(
                    &self.cfg.model_path,
                    &self.cfg.charset_path,
                    self.cfg.ocr_threads,
                )?;
                o.warmup()?;
                let o = Arc::new(o);
                self.ocr = Some(o.clone());
                o
            }
        };
        info!(account = %self.account.code, "auto re-login starting");
        let result = auth::login(&self.cfg, ocr.as_ref()).await?;
        self.api = OrdersApi::new(&self.cfg.base_url, &result.session)?;
        accounts::save_session_row(&self.pool, self.account.id, &result.session).await?;
        info!(account = %self.account.code, attempts = result.attempts, "auto re-login ok");
        Ok(())
    }

    async fn run_new_sync(&mut self, ctx: &SyncContext) -> Result<SyncStats> {
        match sync_new_orders(&self.pool, &self.api, ctx).await {
            Ok(v) => Ok(v),
            Err(e) if client::is_auth_error(&e) && self.app_cfg.auto_relogin => {
                warn!(error = %e, "auth expired mid-sync — re-login once");
                self.relogin().await?;
                sync_new_orders(&self.pool, &self.api, ctx).await
            }
            Err(e) => Err(e),
        }
    }

    /// Sync a packed bucket (processing/shipped) so orders that move on
    /// (packed elsewhere, handed to the carrier) leave their stale state.
    /// The new-bucket pass + reconcile only heal rows still `state='new'`.
    async fn run_bucket_sync(
        &mut self,
        ctx: &SyncContext,
        status: &'static str,
        max_pages: u32,
    ) -> Result<SyncStats> {
        match sync_status_bucket(&self.pool, &self.api, status, 50, max_pages, ctx).await {
            Ok(v) => Ok(v),
            Err(e) if client::is_auth_error(&e) && self.app_cfg.auto_relogin => {
                warn!(error = %e, "auth expired mid-sync — re-login once");
                self.relogin().await?;
                sync_status_bucket(&self.pool, &self.api, status, 50, max_pages, ctx).await
            }
            Err(e) => Err(e),
        }
    }

    /// Persist the live (rotated) cookies to `.session.json` + `bs_sessions`
    /// so CLI tools and recovery paths never read a stale jar. BigSeller
    /// rotates cookies per response; only this long-lived client sees them.
    async fn persist_session(&self) {
        let cookies = match self.api.current_cookies() {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => return,
            Err(e) => {
                warn!(error = %e, "cookie snapshot failed — session not persisted");
                return;
            }
        };
        let mut session = SessionData::load(&self.cfg.session_path).unwrap_or_default();
        session.cookies = cookies;
        session.saved_at = Some(Utc::now().to_rfc3339());
        if let Err(e) = session.save(&self.cfg.session_path) {
            warn!(error = %e, "persist rotated session file failed");
            return;
        }
        if let Err(e) = accounts::save_session_row(&self.pool, self.account.id, &session).await {
            warn!(error = %e, "persist bs_sessions row failed");
        }
    }

    /// Heal stale `state = 'new'` rows after a successful new-bucket pass.
    /// Window scales with the sync interval (3×, clamped to 3–60 minutes);
    /// per-cycle cap is configurable (RECONCILE_CAP) so a one-time backlog
    /// can be drained fast, then turned back down.
    async fn run_reconcile(&mut self, ctx: &SyncContext) -> Result<ReconcileStats> {
        let cap = self.app_cfg.reconcile_cap;
        let stale_after = (self.app_cfg.new_interval_secs.saturating_mul(3)).clamp(180, 3600);
        match reconcile_stale_new_orders(&self.pool, &self.api, ctx, stale_after, cap).await {
            Ok(v) => Ok(v),
            Err(e) if client::is_auth_error(&e) && self.app_cfg.auto_relogin => {
                warn!(error = %e, "auth expired during reconcile — re-login once");
                self.relogin().await?;
                reconcile_stale_new_orders(&self.pool, &self.api, ctx, stale_after, cap).await
            }
            Err(e) => Err(e),
        }
    }

    async fn run_cancel_sync(&mut self, ctx: &SyncContext) -> Result<Vec<SyncStats>> {
        match sync_cancel_related(&self.pool, &self.api, ctx).await {
            Ok(v) => Ok(v),
            Err(e) if client::is_auth_error(&e) && self.app_cfg.auto_relogin => {
                warn!(error = %e, "auth expired mid-cancel — re-login once");
                self.relogin().await?;
                sync_cancel_related(&self.pool, &self.api, ctx).await
            }
            Err(e) => Err(e),
        }
    }
}

/// Bootstrap account row from config + optional existing session file.
pub async fn bootstrap_account(pool: &PgPool, cfg: &Config) -> Result<Account> {
    let login_owned: String = if let Some(a) = cfg.account.clone().filter(|s| !s.is_empty()) {
        a
    } else if let Ok(s) = SessionData::load(&cfg.session_path) {
        s.account
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "unknown".into())
    } else {
        return Err(Error::Config(
            "BS_ACCOUNT required (or session with account field)".into(),
        ));
    };

    let code = cfg
        .account_code
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".into());

    let acct = accounts::ensure_account(
        pool,
        &code,
        &login_owned,
        Some(&format!("BigSeller ({code})")),
    )
    .await?;

    if let Ok(session) = SessionData::load(&cfg.session_path) {
        let _ = accounts::save_session_row(pool, acct.id, &session).await;
    }

    Ok(acct)
}

/// Long-running worker: poll new orders + evening cancel + outbox + auto re-login.
pub async fn run_worker(pool: PgPool, cfg: Config, app_cfg: WorkerConfig) -> Result<()> {
    let account = bootstrap_account(&pool, &cfg).await?;
    info!(
        account_id = account.id,
        code = %account.code,
        new_interval_secs = app_cfg.new_interval_secs,
        cancel_hour = app_cfg.cancel_hour_local,
        auto_relogin = app_cfg.auto_relogin,
        "worker starting"
    );

    let api = match SessionData::load(&cfg.session_path) {
        Ok(s) => OrdersApi::new(&cfg.base_url, &s)?,
        Err(_) if app_cfg.auto_relogin => {
            // Will login on first ensure_api
            let empty = SessionData::default();
            OrdersApi::new(&cfg.base_url, &empty)?
        }
        Err(e) => return Err(e),
    };

    let mut state = WorkerState {
        cfg: cfg.clone(),
        app_cfg: app_cfg.clone(),
        pool: pool.clone(),
        api,
        account: account.clone(),
        ocr: None,
    };

    // Scheduled ops jobs (batch pagi/siang → email printer, rekap sore, cancel
    // printed, instant batch).
    let mut scheduler = crate::scheduler::Scheduler {
        pool: pool.clone(),
        cfg,
        ran_batch_pagi_day: None,
        ran_batch_siang_day: None,
        ran_rekap_day: None,
        ran_cancel_printed_day: None,
        last_instant_batch_at: None,
    };

    let ctx = SyncContext {
        account_id: Some(account.id),
        account_code: Some(account.code.clone()),
    };

    let mut last_cancel_day: Option<u32> = None;
    let mut tick_n: u64 = 0;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(app_cfg.new_interval_secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tick.tick().await;
        tick_n += 1;

        // Sync hanya jalan di jam kerja (default 07:50–17:00 WIB). Di luar itu
        // worker tidur untuk BigSeller (tidak menarik order/reconcile/bucket),
        // tapi scheduler harian (batch pagi, rekap sore, cancel printed) dan
        // cancel-sync malam tetap berjalan sesuai jadwal masing-masing.
        let in_work_hours = crate::batch::is_within_working_hours(
            Utc::now(),
            app_cfg.work_hour_start_min,
            app_cfg.work_hour_end_min,
        );

        if in_work_hours {
            if let Err(e) = state.ensure_api().await {
                warn!(error = %e, "ensure session failed — skip tick");
                continue;
            }

            match state.run_new_sync(&ctx).await {
                Ok(s) => {
                    if s.created > 0 {
                        info!(created = s.created, "new orders detected");
                    }
                    // Keep the saved session fresh (cookies rotate per response).
                    state.persist_session().await;
                    // Absence is only meaningful after a successful bucket pass.
                    match state.run_reconcile(&ctx).await {
                        Ok(r) if r.candidates > 0 => {
                            info!(
                                candidates = r.candidates,
                                refreshed = r.refreshed,
                                archived = r.archived,
                                not_found = r.not_found,
                                "reconciled stale new orders"
                            );
                        }
                        Err(e) => warn!(error = %e, "reconcile failed"),
                        _ => {}
                    }
                }
                Err(e) => warn!(error = %e, "sync new failed"),
            }

            // Packed buckets: processing is tiny (1–2 pages) every tick; shipped
            // is heavier (~9 pages at 50/page), so every 6th tick (~30 min).
            // Without these, orders packed outside our UI or handed to the
            // carrier keep their stale state forever (reconcile only heals
            // rows still state='new').
            match state.run_bucket_sync(&ctx, "processing", 2).await {
                Ok(s) if s.state_changed > 0 => {
                    info!(
                        upserted = s.upserted,
                        state_changed = s.state_changed,
                        "processing bucket: states caught up"
                    );
                }
                Ok(_) => {}
                Err(e) => warn!(error = %e, "sync processing failed"),
            }
            if tick_n.is_multiple_of(6) {
                match state.run_bucket_sync(&ctx, "shipped", 12).await {
                    Ok(s) if s.state_changed > 0 => {
                        info!(
                            upserted = s.upserted,
                            state_changed = s.state_changed,
                            "shipped bucket: states caught up"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "sync shipped failed"),
                }
            }

            if let Err(e) = drain_outbox(&state.pool, &state.app_cfg).await {
                warn!(error = %e, "outbox drain failed");
            }
        }

        // Scheduled ops jobs (batch pagi, rekap sore, cancel printed) — WIB clock.
        if let Err(e) = crate::scheduler::tick(&mut scheduler).await {
            warn!(error = %e, "scheduler tick failed");
        }

        let now = Local::now();
        let yday = now.ordinal();
        let due = now.time()
            >= NaiveTime::from_hms_opt(app_cfg.cancel_hour_local, app_cfg.cancel_minute_local, 0)
                .or_else(|| NaiveTime::from_hms_opt(17, 0, 0))
                .unwrap_or(NaiveTime::MIN);
        if due && last_cancel_day != Some(yday) {
            info!("running evening cancel-related sync");
            // Outside work hours ensure_api isn't called every tick — make sure
            // the session is fresh before the evening cancel pull.
            if let Err(e) = state.ensure_api().await {
                warn!(error = %e, "cancel sync: session not ready, skip");
            } else {
                match state.run_cancel_sync(&ctx).await {
                    Ok(stats) => {
                        for s in stats {
                            info!(kind = %s.kind, upserted = s.upserted, "cancel sync ok");
                        }
                        last_cancel_day = Some(yday);
                        let _ = set_cursor(
                            &state.pool,
                            &format!("last_cancel_evening:{}", account.code),
                            json!({ "at": Utc::now().to_rfc3339(), "localHour": now.hour() }),
                        )
                        .await;
                    }
                    Err(e) => warn!(error = %e, "evening cancel sync failed"),
                }
            }
        }
    }
}

async fn drain_outbox(pool: &PgPool, cfg: &WorkerConfig) -> Result<()> {
    let events = claim_pending_outbox(pool, 20).await?;
    if events.is_empty() {
        return Ok(());
    }

    let wazapin = match cfg.wazapin.as_ref() {
        Some(w) if w.enabled_any() => match crate::wazapin::WazapinClient::new(w.clone()) {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(error = %e, "wazapin client init failed");
                None
            }
        },
        _ => None,
    };

    let webhook_url = cfg.wa_webhook_url.as_deref();
    if wazapin.is_none() && webhook_url.is_none() {
        // Nothing to deliver — leave pending so enabling env later still drains.
        return Ok(());
    }

    let http = reqwest::Client::new();
    for ev in events {
        // Urgent/instant "order.created" notifications are batched by the
        // scheduled instant-batch job (every INSTANT_BATCH_INTERVAL_MIN within
        // working hours) into one combined card — no more one message per order
        // during a morning rush. Leave them pending here; the job marks them
        // sent. The job is gated to working hours, so nothing pings overnight.
        if ev.event_type == "order.created"
            && crate::notify::payload_is_urgent(&ev.payload)
        {
            continue;
        }

        // 1) Cancel → Wazapin group (per-event card, gated by summary-printed)
        if let Some(ref wz) = wazapin {
            match crate::notify::try_handle_outbox_wazapin(pool, wz, &ev).await {
                Ok(true) => {
                    mark_outbox_sent(pool, ev.id).await?;
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    warn!(
                        outbox_id = ev.id,
                        error = %e,
                        "wazapin notify failed"
                    );
                    mark_outbox_failed(pool, ev.id, &e.to_string()).await?;
                    continue;
                }
            }
        }

        // 2) Optional generic webhook for non-instant (or all if no wazapin)
        let Some(url) = webhook_url else {
            // Non-instant with only wazapin configured: mark sent so outbox doesn't pile up.
            if wazapin.is_some() {
                mark_outbox_sent(pool, ev.id).await?;
            }
            continue;
        };

        let mut req = http.post(url).json(&json!({
            "id": ev.id,
            "eventType": ev.event_type,
            "orderId": ev.order_id,
            "platformOrderId": ev.platform_order_id,
            "payload": ev.payload,
            "createdAt": ev.created_at,
        }));
        if let Some(tok) = cfg.wa_webhook_token.as_deref() {
            req = req.bearer_auth(tok);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                mark_outbox_sent(pool, ev.id).await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                mark_outbox_failed(pool, ev.id, &format!("HTTP {status}: {body}")).await?;
            }
            Err(e) => {
                mark_outbox_failed(pool, ev.id, &e.to_string()).await?;
            }
        }
    }
    Ok(())
}
