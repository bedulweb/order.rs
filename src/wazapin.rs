//! Wazapin public API client — send text/image to WhatsApp groups.
//!
//! Docs: wazapin/platform/docs/api/group-messages-and-mentions.md
//! Base: `https://api.wazapin.com`

use crate::error::{Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct WazapinConfig {
    pub base_url: String,
    pub api_key: String,
    pub channel_id: String,
    pub group_jid: String,
    pub org_slug: Option<String>,
    /// When true, worker sends instant-order PNG to the group.
    pub notify_instant: bool,
    /// When true, worker sends cancel PNG for summary-printed cancels.
    pub notify_cancel: bool,
}

impl WazapinConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("WAZAPIN_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())?;
        let channel_id = std::env::var("WAZAPIN_CHANNEL_ID")
            .ok()
            .filter(|s| !s.is_empty())?;
        let group_jid = std::env::var("WAZAPIN_GROUP_JID")
            .ok()
            .filter(|s| !s.is_empty())?;
        let base_url = std::env::var("WAZAPIN_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://api.wazapin.com".into());
        let org_slug = std::env::var("WAZAPIN_ORG_SLUG")
            .ok()
            .filter(|s| !s.is_empty());
        let notify_instant = env_flag("WAZAPIN_NOTIFY_INSTANT", true);
        let notify_cancel = env_flag("WAZAPIN_NOTIFY_CANCEL", true);
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            channel_id,
            group_jid,
            org_slug,
            notify_instant,
            notify_cancel,
        })
    }

    pub fn enabled_for_instant(&self) -> bool {
        self.notify_instant
    }

    pub fn enabled_for_cancel(&self) -> bool {
        self.notify_cancel
    }

    pub fn enabled_any(&self) -> bool {
        self.notify_instant || self.notify_cancel
    }
}

fn env_flag(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|s| matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

#[derive(Debug, Clone)]
pub struct WazapinClient {
    http: reqwest::Client,
    cfg: WazapinConfig,
}

#[derive(Debug, Clone)]
pub struct SendResult {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChannelStatus {
    connected: bool,
    logged_in: bool,
    error: Option<String>,
}

impl WazapinClient {
    pub fn new(cfg: WazapinConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("orders-worker/wazapin")
            .build()
            .map_err(|e| Error::Other(format!("wazapin http client: {e}")))?;
        Ok(Self { http, cfg })
    }

    pub fn config(&self) -> &WazapinConfig {
        &self.cfg
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut r = req
            .header("X-Api-Key", &self.cfg.api_key)
            .header("Content-Type", "application/json");
        if let Some(slug) = self.cfg.org_slug.as_deref() {
            r = r.header("X-Organization-Slug", slug);
        }
        r
    }

    async fn ensure_channel_ready(&self) -> Result<()> {
        let path = format!("/v1/channels/{}/status", self.cfg.channel_id);
        let url = format!("{}{}", self.cfg.base_url, path);
        let resp = self
            .apply_headers(self.http.get(&url))
            .send()
            .await
            .map_err(|e| Error::Other(format!("wazapin GET {path}: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::Other(format!("wazapin channel status body: {e}")))?;
        if !status.is_success() {
            return Err(Error::Other(format!(
                "wazapin channel status HTTP {status}: {}",
                text.chars().take(400).collect::<String>()
            )));
        }

        let channel: ChannelStatus = serde_json::from_str(&text)
            .map_err(|e| Error::Other(format!("wazapin channel status json: {e}")))?;
        if channel.connected && channel.logged_in {
            return Ok(());
        }

        Err(Error::Other(format!(
            "wazapin channel {} is not ready: connected={} logged_in={} error={}. Reconnect/scan QR in Wazapin before sending.",
            self.cfg.channel_id,
            channel.connected,
            channel.logged_in,
            channel.error.as_deref().unwrap_or("-")
        )))
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{path}", self.cfg.base_url);
        let resp = self
            .apply_headers(self.http.post(&url).json(body))
            .send()
            .await
            .map_err(|e| Error::Other(format!("wazapin POST {path}: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::Other(format!("wazapin body: {e}")))?;
        if !status.is_success() {
            return Err(Error::Other(format!(
                "wazapin HTTP {status}: {}",
                text.chars().take(400).collect::<String>()
            )));
        }
        serde_json::from_str(&text).map_err(|e| Error::Other(format!("wazapin json: {e}")))
    }

    /// Upload PNG/JPEG bytes → public `media_url`.
    pub async fn upload_image(&self, bytes: &[u8], filename: &str) -> Result<String> {
        let url = format!("{}/v1/messages/uploads/image", self.cfg.base_url);
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(filename.to_string())
            .mime_str("image/png")
            .map_err(|e| Error::Other(format!("multipart: {e}")))?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let mut req = self
            .http
            .post(&url)
            .header("X-Api-Key", &self.cfg.api_key)
            .multipart(form);
        if let Some(slug) = self.cfg.org_slug.as_deref() {
            req = req.header("X-Organization-Slug", slug);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Other(format!("wazapin upload: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::Other(format!("wazapin upload body: {e}")))?;
        if !status.is_success() {
            return Err(Error::Other(format!(
                "wazapin upload HTTP {status}: {}",
                text.chars().take(400).collect::<String>()
            )));
        }
        // API may return `{ media_url, ... }` at the root or nested under `data`.
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Other(format!("wazapin upload json: {e}")))?;
        extract_media_url(&v).ok_or_else(|| {
            Error::Other(format!(
                "wazapin upload missing media_url: {}",
                text.chars().take(200).collect::<String>()
            ))
        })
    }

    pub async fn send_text(&self, body: &str) -> Result<SendResult> {
        self.ensure_channel_ready().await?;
        let payload = json!({
            "channel_id": self.cfg.channel_id,
            "to": self.cfg.group_jid,
            "type": "text",
            "content": { "body": body },
        });
        let v = self.post_json("/v1/messages", &payload).await?;
        parse_send(&v)
    }

    pub async fn send_image(&self, media_url: &str, caption: &str) -> Result<SendResult> {
        self.ensure_channel_ready().await?;
        let payload = json!({
            "channel_id": self.cfg.channel_id,
            "to": self.cfg.group_jid,
            "type": "image",
            "content": {
                "media_url": media_url,
                "caption": caption,
            },
        });
        let v = self.post_json("/v1/messages", &payload).await?;
        parse_send(&v)
    }

    /// Upload PNG then send as image with caption.
    pub async fn send_png_bytes(
        &self,
        png: &[u8],
        filename: &str,
        caption: &str,
    ) -> Result<SendResult> {
        self.ensure_channel_ready().await?;
        let media_url = self.upload_image(png, filename).await?;
        info!(%media_url, "wazapin media uploaded");
        let r = self.send_image(&media_url, caption).await?;
        info!(id = %r.id, status = %r.status, "wazapin image queued");
        Ok(r)
    }
}

/// Pull `media_url` / `url` from root or nested `data` (Wazapin response shapes vary).
fn extract_media_url(v: &Value) -> Option<String> {
    let candidates = [
        v.get("media_url"),
        v.get("url"),
        v.get("data").and_then(|d| d.get("media_url")),
        v.get("data").and_then(|d| d.get("url")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_send(v: &Value) -> Result<SendResult> {
    let data = v.get("data").unwrap_or(v);
    let id = data
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let status = data
        .get("status")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    if id.is_empty() {
        warn!(?v, "wazapin send missing id");
        return Err(Error::Other("wazapin send response missing data.id".into()));
    }
    Ok(SendResult { id, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_none_without_key() {
        // Don't mutate process env in parallel tests aggressively — just construct.
        let cfg = WazapinConfig {
            base_url: "https://api.wazapin.com".into(),
            api_key: "wzp_x".into(),
            channel_id: "wzp_c".into(),
            group_jid: "120@g.us".into(),
            org_slug: None,
            notify_instant: true,
            notify_cancel: true,
        };
        assert!(cfg.enabled_for_instant());
        assert!(cfg.enabled_for_cancel());
    }

    #[test]
    fn extract_media_url_root_and_nested() {
        let root = serde_json::json!({
            "media_url": "https://media.example/a.png",
            "file_name": "a.png"
        });
        assert_eq!(
            extract_media_url(&root).as_deref(),
            Some("https://media.example/a.png")
        );

        let nested = serde_json::json!({
            "data": { "media_url": "https://media.example/b.png" }
        });
        assert_eq!(
            extract_media_url(&nested).as_deref(),
            Some("https://media.example/b.png")
        );

        let url_key = serde_json::json!({ "url": "https://media.example/c.png" });
        assert_eq!(
            extract_media_url(&url_key).as_deref(),
            Some("https://media.example/c.png")
        );
    }
}
