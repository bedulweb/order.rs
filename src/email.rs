//! Email notifications via the Resend HTTP API (`api.resend.com/emails`).
//!
//! HTTPS is used instead of SMTP because outbound SMTP ports (465/587) are
//! firewalled on the deployment host. Sender must be a domain verified in the
//! Resend dashboard (e.g. `noreply@obayito.com`).

use crate::error::{Error, Result};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub api_key: String,
    pub from: String,
    /// Default recipient (e.g. the Epson email-print address). Used by the
    /// scheduled batch-pagi email workflow; examples may override.
    pub to: Option<String>,
}

impl SmtpConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("RESEND_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())?;
        let from = std::env::var("RESEND_FROM")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Order.rs <noreply@obayito.com>".into());
        let to = std::env::var("RESEND_TO")
            .ok()
            .filter(|s| !s.is_empty());
        Some(Self { api_key, from, to })
    }
}

/// Send a plain-text email via Resend. Returns the Resend message id.
pub async fn send_text(cfg: &SmtpConfig, to: &str, subject: &str, body: &str) -> Result<String> {
    send(cfg, to, subject, body, None).await
}

/// Send an email with a PDF attachment (base64 inline in the Resend JSON).
pub async fn send_pdf(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    body: &str,
    pdf_bytes: &[u8],
    filename: &str,
) -> Result<String> {
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        pdf_bytes,
    );
    let attachment = json!({
        "filename": filename,
        "content": b64,
    });
    send(cfg, to, subject, body, Some(attachment)).await
}

/// Send an email that carries **only** the PDF attachment — no body text.
/// Used for Epson Email Print where a non-empty body would print instead of
/// (or alongside) the attachment. Resend requires a `text`/`html` field, so a
/// single space is sent (nothing printable).
pub async fn send_pdf_only(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    pdf_bytes: &[u8],
    filename: &str,
) -> Result<String> {
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        pdf_bytes,
    );
    let attachment = json!({
        "filename": filename,
        "content": b64,
    });
    send(cfg, to, subject, " ", Some(attachment)).await
}

async fn send(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    body: &str,
    attachment: Option<serde_json::Value>,
) -> Result<String> {
    let client = reqwest::Client::new();
    let mut payload = json!({
        "from": cfg.from,
        "to": [to],
        "subject": subject,
        "text": body,
    });
    if let Some(a) = attachment {
        payload["attachments"] = serde_json::json!([a]);
    }
    let resp = client
        .post("https://api.resend.com/emails")
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Other(format!("resend http: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| Error::Other(format!("resend body: {e}")))?;
    if !status.is_success() {
        return Err(Error::Other(format!(
            "resend HTTP {status}: {}",
            text.chars().take(400).collect::<String>()
        )));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Other(format!("resend json: {e}")))?;
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| Error::Other(format!("resend missing id: {text}")))?;
    tracing::info!(to, subject, msg_id = %id, "email sent");
    Ok(id.to_string())
}

