//! Inbound Wazapin webhook → perintah dari grup WhatsApp (ops).
//!
//! Member grup bisa memicu notifikasi dengan mengetik kata kunci:
//! - `instant` → kirim kartu pesanan instant ke grup yang sama (pakai channel
//!   yang menerima pesan tersebut, jadi tidak perlu konfigurasi tambahan).
//!
//! Endpoint: `POST /v1/wazapin/webhook` (balas 200 cepat, kerjaan di task).

use crate::api::ApiState;
use crate::error::Error;
use crate::instant_notify::NotifyOrder;
use crate::notify::{load_notify_order, send_instant_orders};
use crate::wazapin::{WazapinClient, WazapinConfig};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;
use sqlx::{PgPool, Row};
use tracing::{info, warn};

/// Kata kunci grup → kartu pesanan instant.
const CMD_INSTANT: &str = "instant";
/// Maks order per kartu trigger (sama dengan instant batch).
const MAX_CARD_ORDERS: usize = 50;

/// Event `message.new` Wazapin (raw atau bagian `data` dari envelope Svix).
#[derive(Debug, Clone, serde::Deserialize)]
struct InboundMessage {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    msg_type: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
}

/// POST /v1/wazapin/webhook — terima event pesan masuk, proses di task.
pub async fn wazapin_webhook(State(st): State<ApiState>, body: Bytes) -> Response {
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "wazapin webhook: json invalid");
            return (StatusCode::BAD_REQUEST, "invalid json").into_response();
        }
    };

    // Envelope Svix (`{ type, data }`) vs event mentah (payload flat).
    let inner = payload
        .get("data")
        .filter(|d| d.is_object())
        .unwrap_or(&payload);
    let ev: InboundMessage = match serde_json::from_value(inner.clone()) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "wazapin webhook: event tidak dikenali");
            return (StatusCode::OK, "ignored").into_response();
        }
    };

    // Hanya teks inbound dari grup WhatsApp.
    let Some(chat) = ev.chat_id.as_deref().filter(|c| c.ends_with("@g.us")) else {
        return (StatusCode::OK, "ignored").into_response();
    };
    let is_text = ev.msg_type.as_deref().unwrap_or("text") == "text";
    let is_inbound = ev.direction.as_deref().unwrap_or("inbound") == "inbound";
    if !is_text || !is_inbound {
        return (StatusCode::OK, "ignored").into_response();
    }
    let Some(raw_cmd) = ev.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) else {
        return (StatusCode::OK, "ignored").into_response();
    };

    let pool = st.pool.clone();
    let cmd = raw_cmd.to_ascii_lowercase();
    let chat = chat.to_string();
    let channel = ev.channel_id.clone();
    tokio::spawn(async move {
        if let Err(e) = run_command(&pool, chat, channel, &cmd).await {
            warn!(error = %e, cmd, "wazapin webhook command failed");
        }
    });

    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

async fn run_command(
    pool: &PgPool,
    chat_id: String,
    channel_id: Option<String>,
    cmd: &str,
) -> crate::error::Result<()> {
    match cmd {
        CMD_INSTANT => {
            let orders = load_urgent_orders(pool).await?;
            let mut cfg = WazapinConfig::from_env()
                .ok_or_else(|| Error::Other("WAZAPIN_API_KEY / CHANNEL / GROUP not set".into()))?;
            // Balas di grup yang sama, lewat channel yang menerima pesan.
            cfg.group_jid = chat_id.clone();
            if let Some(c) = channel_id.as_deref().filter(|c| !c.is_empty()) {
                cfg.channel_id = c.to_string();
            }
            let client = WazapinClient::new(cfg)?;

            if orders.is_empty() {
                let r = client
                    .send_text("Tidak ada pesanan instant menunggu 😊")
                    .await?;
                info!(msg_id = %r.id, %chat_id, "trigger instant: kosong");
                return Ok(());
            }
            let msg_id = send_instant_orders(&client, orders).await?;
            info!(msg_id, %chat_id, "trigger instant sent");
            Ok(())
        }
        _ => {
            info!(cmd, %chat_id, "wazapin webhook: perintah tidak dikenal");
            Ok(())
        }
    }
}

/// Order urgent yang masih terbuka hari ini (WIB) — sumber kartu trigger.
/// Keyword sama dengan feed urgent ops UI + `anteraja` (konsisten dengan batch).
async fn load_urgent_orders(pool: &PgPool) -> crate::error::Result<Vec<NotifyOrder>> {
    let wib = chrono::FixedOffset::east_opt(crate::batch::WIB_OFFSET_SECS).expect("WIB offset");
    let day_start = chrono::Utc::now()
        .with_timezone(&wib)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight")
        .and_local_timezone(wib)
        .single()
        .expect("WIB has no DST")
        .with_timezone(&chrono::Utc);

    let rows = sqlx::query(
        r#"
        SELECT o.id
        FROM orders o
        WHERE o.state IN ('new', 'processing', 'pickup')
          AND COALESCE(o.ordered_at, o.first_seen_at) >= $1
          AND LOWER(CONCAT_WS(' ', o.buyer_shipping_carrier, o.shipment_provider, o.shipping_carrier_name)) LIKE ANY(ARRAY[
              '%instant%', '%sameday%', '%same day%', '%same-day%', '%prioritas%',
              '%gojek%', '%gosend%', '%grab%', '%paxel%', '%anteraja%'
          ])
        ORDER BY o.ordered_at DESC NULLS LAST, o.id DESC
        LIMIT 200
        "#,
    )
    .bind(day_start)
    .fetch_all(pool)
    .await?;

    let mut orders = Vec::new();
    for r in rows {
        let id: i64 = r.get("id");
        match load_notify_order(pool, id).await {
            Ok(o) => {
                orders.push(o);
                if orders.len() >= MAX_CARD_ORDERS {
                    break;
                }
            }
            Err(e) => warn!(order_id = id, error = %e, "trigger instant: load order gagal"),
        }
    }
    Ok(orders)
}
