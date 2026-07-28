//! On-demand "order masuk" sync behind the ops UI Refresh button.
//!
//! Reuses the session file the worker keeps fresh and never re-logs in from
//! the API process — BigSeller only allows one active session, so a login
//! here would kill the worker's. When the session is expired the run fails
//! fast with a clear message instead. Progress is a small step list that the
//! UI polls while the dialog is open.

use crate::error::{Error, Result};
use crate::orders::OrdersApi;
use crate::session::SessionData;
use crate::sync::{self, SyncContext};
use serde::Serialize;
use sqlx::PgPool;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStep {
    pub key: &'static str,
    pub label: &'static str,
    pub state: StepState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRunResult {
    pub pages: i32,
    pub upserted: i32,
    pub created: i32,
    pub state_changed: i32,
    pub healed: i32,
    pub archived: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub running: bool,
    /// Epoch ms of the run start; 0 when nothing has run since API boot.
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub steps: Vec<SyncStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<SyncRunResult>,
}

fn fresh_steps() -> Vec<SyncStep> {
    vec![
        SyncStep {
            key: "session",
            label: "Cek sesi BigSeller",
            state: StepState::Pending,
            detail: None,
        },
        SyncStep {
            key: "pull",
            label: "Tarik order baru",
            state: StepState::Pending,
            detail: None,
        },
        SyncStep {
            key: "reconcile",
            label: "Rekonsiliasi state order",
            state: StepState::Pending,
            detail: None,
        },
    ]
}

impl SyncProgress {
    pub fn idle() -> Self {
        Self {
            running: false,
            started_at_ms: 0,
            finished_at_ms: None,
            ok: None,
            error: None,
            steps: fresh_steps(),
            result: None,
        }
    }

    fn set_step(&mut self, key: &str, state: StepState, detail: Option<String>) {
        if let Some(s) = self.steps.iter_mut().find(|s| s.key == key) {
            s.state = state;
            if let Some(d) = detail {
                s.detail = Some(d);
            }
        }
    }
}

pub type ProgressHandle = Arc<Mutex<SyncProgress>>;

pub fn progress_handle() -> ProgressHandle {
    Arc::new(Mutex::new(SyncProgress::idle()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Mark a run as in flight and spawn it. Returns `false` when a run is
/// already in flight (the caller can still poll progress).
pub async fn start(
    handle: &ProgressHandle,
    pool: PgPool,
    base_url: String,
    session_path: std::path::PathBuf,
    account_id: Option<i64>,
) -> bool {
    {
        let mut p = handle.lock().await;
        if p.running {
            return false;
        }
        *p = SyncProgress {
            running: true,
            started_at_ms: now_ms(),
            ..SyncProgress::idle()
        };
    }
    let h = Arc::clone(handle);
    tokio::spawn(async move {
        run(&h, &pool, &base_url, &session_path, account_id).await;
    });
    true
}

async fn set_step(handle: &ProgressHandle, key: &str, state: StepState, detail: Option<String>) {
    handle.lock().await.set_step(key, state, detail);
}

/// Mark the step failed and hand the error back for the run outcome.
async fn fail_step(handle: &ProgressHandle, key: &str, e: Error) -> Error {
    set_step(handle, key, StepState::Error, Some(e.to_string())).await;
    e
}

async fn run(
    handle: &ProgressHandle,
    pool: &PgPool,
    base_url: &str,
    session_path: &Path,
    account_id: Option<i64>,
) {
    let ctx = SyncContext {
        account_id,
        account_code: None,
    };
    let outcome = run_steps(handle, pool, base_url, session_path, &ctx).await;
    let mut p = handle.lock().await;
    p.running = false;
    p.finished_at_ms = Some(now_ms());
    match outcome {
        Ok(res) => {
            p.ok = Some(true);
            p.result = Some(res);
        }
        Err(e) => {
            p.ok = Some(false);
            p.error = Some(e.to_string());
        }
    }
}

async fn run_steps(
    handle: &ProgressHandle,
    pool: &PgPool,
    base_url: &str,
    session_path: &Path,
    ctx: &SyncContext,
) -> Result<SyncRunResult> {
    // 1) Session — reuse what the worker maintains; never re-login here.
    set_step(handle, "session", StepState::Running, None).await;
    let session = match SessionData::load(session_path) {
        Ok(s) => s,
        Err(e) => return Err(fail_step(handle, "session", e).await),
    };
    let api = match OrdersApi::new(base_url, &session) {
        Ok(a) => a,
        Err(e) => return Err(fail_step(handle, "session", e).await),
    };
    match api.is_login().await {
        Ok(true) => {
            set_step(handle, "session", StepState::Ok, Some("sesi aktif".into())).await;
        }
        Ok(false) => {
            let msg = "sesi kedaluwarsa — worker akan re-login otomatis pada siklus berikutnya";
            set_step(handle, "session", StepState::Error, Some(msg.into())).await;
            return Err(Error::Other(msg.into()));
        }
        Err(e) => return Err(fail_step(handle, "session", e).await),
    }

    // 2) Pull the whole BigSeller `new` bucket.
    set_step(handle, "pull", StepState::Running, None).await;
    let stats = match sync::sync_new_orders(pool, &api, ctx).await {
        Ok(s) => s,
        Err(e) => return Err(fail_step(handle, "pull", e).await),
    };
    set_step(
        handle,
        "pull",
        StepState::Ok,
        Some(format!(
            "{} halaman · {} order disinkronkan · {} baru",
            stats.pages, stats.upserted, stats.created
        )),
    )
    .await;

    // 3) Heal orders that left the new bucket since the last pass. Same
    //    freshness window as the feed (15 min); cap mirrors RECONCILE_CAP.
    set_step(handle, "reconcile", StepState::Running, None).await;
    let cap = std::env::var("RECONCILE_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
        .clamp(1, 500);
    let rec = match sync::reconcile_stale_new_orders(pool, &api, ctx, 900, cap).await {
        Ok(r) => r,
        Err(e) => return Err(fail_step(handle, "reconcile", e).await),
    };
    set_step(
        handle,
        "reconcile",
        StepState::Ok,
        Some(format!(
            "{} state dipulihkan · {} diarsipkan",
            rec.refreshed, rec.archived
        )),
    )
    .await;

    Ok(SyncRunResult {
        pages: stats.pages,
        upserted: stats.upserted,
        created: stats.created,
        state_changed: stats.state_changed,
        healed: rec.refreshed,
        archived: rec.archived,
    })
}
