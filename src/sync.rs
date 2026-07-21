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
use sqlx::PgPool;
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
                OrderListQuery::active(status)
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
    pub cancel_hour_local: u32,
    pub cancel_minute_local: u32,
    pub wa_webhook_url: Option<String>,
    pub wa_webhook_token: Option<String>,
    /// Auto re-login when BS returns auth-expired.
    pub auto_relogin: bool,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            new_interval_secs: 60,
            cancel_hour_local: 17,
            cancel_minute_local: 0,
            wa_webhook_url: None,
            wa_webhook_token: None,
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
        cfg,
        app_cfg: app_cfg.clone(),
        pool: pool.clone(),
        api,
        account: account.clone(),
        ocr: None,
    };

    let ctx = SyncContext {
        account_id: Some(account.id),
        account_code: Some(account.code.clone()),
    };

    let mut last_cancel_day: Option<u32> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(app_cfg.new_interval_secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tick.tick().await;

        if let Err(e) = state.ensure_api().await {
            warn!(error = %e, "ensure session failed — skip tick");
            continue;
        }

        match state.run_new_sync(&ctx).await {
            Ok(s) => {
                if s.created > 0 {
                    info!(created = s.created, "new orders detected");
                }
            }
            Err(e) => warn!(error = %e, "sync new failed"),
        }

        if let Err(e) = drain_outbox(&state.pool, &state.app_cfg).await {
            warn!(error = %e, "outbox drain failed");
        }

        let now = Local::now();
        let yday = now.ordinal();
        let due = now.time()
            >= NaiveTime::from_hms_opt(app_cfg.cancel_hour_local, app_cfg.cancel_minute_local, 0)
                .or_else(|| NaiveTime::from_hms_opt(17, 0, 0))
                .unwrap_or(NaiveTime::MIN);
        if due && last_cancel_day != Some(yday) {
            info!("running evening cancel-related sync");
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

async fn drain_outbox(pool: &PgPool, cfg: &WorkerConfig) -> Result<()> {
    let events = claim_pending_outbox(pool, 20).await?;
    if events.is_empty() {
        return Ok(());
    }

    let Some(url) = cfg.wa_webhook_url.as_deref() else {
        return Ok(());
    };

    let client = reqwest::Client::new();
    for ev in events {
        let mut req = client.post(url).json(&json!({
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
