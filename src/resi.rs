//! Shipping-label (resi) bulk printing via the BigSeller print plugin.
//!
//! BigSeller's "Auto High-speed Printing" flow, reproduced from the live web
//! app and the plugin wire protocol (plugin v1.2.5.x, ws://localhost:21319):
//!
//! 1. `checkPrintInfo.json` (orderIds, printType=0, isCross=1) validates the
//!    orders and buffers their platform shipping labels server-side.
//! 2. `getPuidNew.json` returns the encrypted user id (`encryptId` / `uid`).
//! 3. The *browser* (on the machine with the plugin installed) handshakes
//!    the plugin: `getPrinter` -> `setPuid [encryptId, uid]` -> `getVersion`;
//!    the plugin then pulls the buffered labels from BigSeller itself and
//!    prints them, reporting `printProcess` progress frames.
//!
//! This module implements steps 1–2 (backend); step 3 lives in the frontend
//! because the WebSocket must originate from the user's machine.

use crate::error::{Error, Result};
use crate::session::SessionData;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResiLabel {
    pub order_id: i64,
    pub platform_order_id: String,
    pub platform: String,
    pub package_no: Option<String>,
    pub tracking_no: Option<String>,
    pub shipping_carrier_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResiPrep {
    /// Encrypted puid for the plugin `setPuid` handshake.
    pub encrypt_id: String,
    pub uid: String,
    /// True when the account uses buffered (auto high-speed) printing — the
    /// plugin fetches labels by puid instead of receiving them inline.
    pub buffer_print_user: bool,
    /// Orders BigSeller buffered labels for (with tracking numbers).
    pub labels: Vec<ResiLabel>,
    /// Requested orders BigSeller refused (canceled / not printable).
    pub not_printable: Vec<i64>,
    pub ship_provider: Option<String>,
}

/// Validate the orders in BigSeller, buffer their shipping labels, and fetch
/// the plugin handshake material (encryptId/uid).
pub async fn prepare_resi_print(
    base_url: &str,
    session_path: &Path,
    order_ids: &[i64],
) -> Result<ResiPrep> {
    if order_ids.is_empty() {
        return Err(Error::Other("no orders selected".into()));
    }
    let session = SessionData::load(session_path)?;
    let client = web_client(base_url, &session)?;

    // 1) Validate + buffer the platform labels server-side. isCross=1 allows
    //    mixed carriers in one print job (matches the live web app).
    let csv = order_ids
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let check: serde_json::Value = client
        .post(format!("{base_url}/api/v1/print/print/checkPrintInfo.json"))
        .form(&[
            ("orderIds", csv.as_str()),
            ("printType", "0"),
            ("isInventory", "0"),
            ("isCross", "1"),
        ])
        .send()
        .await
        .map_err(|e| Error::Other(e.to_string()))?
        .json()
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    let data = check.get("data").cloned().unwrap_or_default();
    let labels: Vec<ResiLabel> = data
        .get("printedListMap")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    Some(ResiLabel {
                        order_id: r.get("orderId")?.as_i64()?,
                        platform_order_id: r.get("platformOrderId")?.as_str()?.to_string(),
                        platform: r
                            .get("platform")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        package_no: r
                            .get("packageNo")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        tracking_no: r
                            .get("trackingNo")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        shipping_carrier_name: r
                            .get("shippingCarrierName")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let not_printable: Vec<i64> = data
        .get("noPrintOrders")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default();

    // 2) Encrypted puid for the plugin handshake.
    let puid: serde_json::Value = client
        .post(format!("{base_url}/api/v1/print/getPuidNew.json"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| Error::Other(e.to_string()))?
        .json()
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    let pdata = puid.get("data").cloned().unwrap_or_default();
    let encrypt_id = pdata
        .get("encryptId")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let uid = pdata
        .get("uid")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let buffer_print_user = pdata
        .get("bufferPrintUser")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if encrypt_id.is_empty() {
        return Err(Error::Other(
            "BigSeller tidak mengembalikan print uid (getPuidNew) — cek sesi login".into(),
        ));
    }
    tracing::info!(
        requested = order_ids.len(),
        labels = labels.len(),
        not_printable = not_printable.len(),
        "resi print prepared (labels buffered in BigSeller)"
    );

    Ok(ResiPrep {
        encrypt_id,
        uid,
        buffer_print_user,
        labels,
        not_printable,
        ship_provider: data
            .get("shipProviderName")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

/// Register the print job so the connected plugin picks it up. This is the
/// step the live web app performs on its print page (`confirmLabelPrint`)
/// after the plugin handshake — without it the buffered labels are never
/// handed to the plugin and nothing prints.
pub async fn confirm_resi_print(
    pool: &sqlx::PgPool,
    base_url: &str,
    session_path: &Path,
    order_ids: &[i64],
) -> Result<serde_json::Value> {
    if order_ids.is_empty() {
        return Err(Error::Other("no orders selected".into()));
    }
    let session = SessionData::load(session_path)?;
    let client = web_client(base_url, &session)?;
    let csv = order_ids
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let v: serde_json::Value = client
        .post(format!("{base_url}/api/v1/order/confirmLabelPrint.json"))
        .form(&[("orderIds", csv.as_str())])
        .send()
        .await
        .map_err(|e| Error::Other(e.to_string()))?
        .json()
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    tracing::info!(orders = order_ids.len(), response = %v, "resi print confirmed (job registered for plugin)");
    // Reflect BigSeller's label mark in our DB immediately (the worker's
    // next processing sync would catch up within minutes anyway).
    if v.get("code").and_then(|c| c.as_i64()) == Some(0) {
        sqlx::query("UPDATE orders SET print_label_mark = 1 WHERE id = ANY($1)")
            .bind(order_ids.to_vec())
            .execute(pool)
            .await?;
    }
    Ok(v)
}

/// HTTP client that looks like the BigSeller browser app — the print APIs
/// only queue/confirm jobs for the plugin when the request carries the
/// web app's origin/referer/x-requested-with headers.
fn web_client(base_url: &str, session: &SessionData) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    let mut put = |k: &'static str, v: String| {
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&v) {
            headers.insert(k, val);
        }
    };
    put("clienttype", "1".into());
    put("origin", base_url.to_string());
    put("referer", format!("{base_url}/web/order/index.htm"));
    put("x-requested-with", "XMLHttpRequest".into());
    put("sec-fetch-dest", "empty".into());
    put("sec-fetch-mode", "cors".into());
    put("sec-fetch-site", "same-origin".into());
    put("accept", "application/json, text/plain, */*".into());
    put(
        "user-agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into(),
    );
    put("cookie", session.cookie_header());
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Other(e.to_string()))
}
