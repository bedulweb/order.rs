//! Pack orders in BigSeller from our UI (mirror of the web "Pack" button).
//!
//! Flow mirrors BigSeller's own web app: `batchInternalVerify` (which of the
//! requested orders are still packable) → `batchPack` (async submit) → a
//! background pass that re-searches each packed order so our Postgres state
//! catches up within seconds instead of waiting for the reconcile window.

use crate::client;
use crate::error::{Error, Result};
use crate::map::map_order_row;
use crate::orders::{OrderListQuery, OrdersApi};
use crate::session::SessionData;
use crate::store::upsert_order;
use serde::Serialize;
use sqlx::PgPool;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackResult {
    pub requested: usize,
    /// Orders BigSeller accepted for packing (validIds from the verify step).
    pub packed: Vec<i64>,
    /// Requested but no longer packable (already moved / packed / canceled).
    pub skipped: Vec<i64>,
    pub ok: bool,
    pub message: String,
}

fn csv(ids: &[i64]) -> String {
    ids.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn api_from_session(base_url: &str, session_path: &Path) -> Result<OrdersApi> {
    let session = SessionData::load(session_path)?;
    OrdersApi::new(base_url, &session)
}

/// Verify + pack the given BigSeller order ids, then spawn a background
/// state refresh so the ops page reflects the move almost immediately.
pub async fn pack_orders(
    pool: &PgPool,
    base_url: &str,
    session_path: &Path,
    account_id: Option<i64>,
    order_ids: &[i64],
) -> Result<PackResult> {
    if order_ids.is_empty() {
        return Err(Error::Other("no orders selected".into()));
    }

    let api = api_from_session(base_url, session_path)?;
    let ids_csv = csv(order_ids);

    // 1) Verify: which requested orders are still packable?
    let verify = match api
        .post_form(
            "/api/v1/order/batchInternalVerify.json",
            &[("orderIds", ids_csv.clone())],
        )
        .await
    {
        Ok(v) => v,
        Err(e) if client::is_auth_error(&e) => {
            // The worker keeps the session file fresh — reload once and retry.
            warn!(error = %e, "pack verify auth expired — reloading session");
            let api2 = api_from_session(base_url, session_path)?;
            api2.post_form(
                "/api/v1/order/batchInternalVerify.json",
                &[("orderIds", ids_csv.clone())],
            )
            .await?
        }
        Err(e) => return Err(e),
    };

    let data = verify.get("data").cloned().unwrap_or_default();
    let valid: Vec<i64> = data
        .get("validIds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
                .collect()
        })
        .unwrap_or_default();
    let skipped: Vec<i64> = order_ids
        .iter()
        .copied()
        .filter(|id| !valid.contains(id))
        .collect();

    if valid.is_empty() {
        return Ok(PackResult {
            requested: order_ids.len(),
            packed: vec![],
            skipped,
            ok: true,
            message: "tidak ada order yang bisa di-pack (sudah pindah status)".into(),
        });
    }

    // 2) Pack (async submit on BigSeller's side).
    let pack = api
        .post_form("/api/v1/order/batchPack.json", &[("orderIds", csv(&valid))])
        .await?;
    let code = client::api_code(&pack).unwrap_or(-1);
    let ok = code == 0;
    let message = client::api_msg(&pack);

    if ok {
        info!(
            count = valid.len(),
            skipped = skipped.len(),
            "packed orders via BigSeller"
        );
        // 3) Background refresh: re-search each packed order so our state
        //    follows within seconds (the page polls every 30s).
        let pool2 = pool.clone();
        let base_url2 = base_url.to_string();
        let session_path2 = session_path.to_path_buf();
        let valid2 = valid.clone();
        tokio::spawn(async move {
            // Give BigSeller a moment to process the async pack submit.
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            let api = match api_from_session(&base_url2, &session_path2) {
                Ok(a) => a,
                Err(e) => {
                    warn!(error = %e, "pack refresh: session load failed");
                    return;
                }
            };
            let mut refreshed = 0i32;
            for id in &valid2 {
                // Search needs the marketplace order number, not the
                // BigSeller id — fetch it from our DB.
                let pid: Option<String> =
                    match sqlx::query_scalar("SELECT platform_order_id FROM orders WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&pool2)
                        .await
                    {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                let Some(pid) = pid else { continue };
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                let page = match api.page_list(&OrderListQuery::search_order_no(&pid)).await {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, order_no = %pid, "pack refresh search failed");
                        continue;
                    }
                };
                for row in &page.rows {
                    let Some(mapped) = map_order_row(row) else {
                        continue;
                    };
                    if !mapped.platform_order_id.eq_ignore_ascii_case(&pid) {
                        continue;
                    }
                    match upsert_order(&pool2, &mapped, account_id).await {
                        Ok(o) if o.state_changed => refreshed += 1,
                        Ok(_) => {}
                        Err(e) => {
                            warn!(error = %e, order_no = %pid, "pack refresh upsert failed")
                        }
                    }
                }
            }
            info!(refreshed, total = valid2.len(), "pack refresh done");
        });
    }

    Ok(PackResult {
        requested: order_ids.len(),
        packed: valid,
        skipped,
        ok,
        message,
    })
}
