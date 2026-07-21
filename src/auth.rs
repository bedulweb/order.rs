//! Login flow: genVerifyCode → OCR → AES password → loginsub.

use crate::client::{self, HttpClient};
use crate::config::Config;
use crate::crypto::encrypt_password;
use crate::error::{Error, Result};
use crate::ocr::CaptchaOcr;
use crate::session::SessionData;
use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct LoginResult {
    pub session: SessionData,
    pub attempts: usize,
    pub captcha_used: String,
}

/// Full login with captcha OCR retries.
pub async fn login(cfg: &Config, ocr: &CaptchaOcr) -> Result<LoginResult> {
    let (account, password) = cfg.require_credentials()?;
    let http = HttpClient::new(&cfg.base_url)?;

    // Warm locale / session cookies from the login page.
    let _ = http.inner.get(http.url("/en_US/login.htm")).send().await?;

    let mut last_message = String::from("no attempt");

    for attempt in 1..=cfg.login_attempts {
        let (access_code, captcha_png) = match fetch_captcha(&http).await {
            Ok(v) => v,
            Err(e) => {
                last_message = e.to_string();
                warn!(attempt, error = %last_message, "captcha fetch failed");
                // Rate-limit / IP block: back off harder.
                let backoff = if last_message.to_lowercase().contains("limit") {
                    Duration::from_secs(3)
                } else {
                    Duration::from_millis(500)
                };
                tokio::time::sleep(backoff).await;
                continue;
            }
        };

        let captcha_text = match ocr.classify_bytes(&captcha_png) {
            Ok(t) => t,
            Err(e) => {
                last_message = e.to_string();
                warn!(attempt, error = %last_message, "OCR failed");
                continue;
            }
        };

        if captcha_text.is_empty() {
            last_message = "empty OCR result".into();
            warn!(attempt, "empty captcha OCR");
            continue;
        }

        info!(attempt, captcha = %captcha_text, "submitting login");

        let pwd_blob = encrypt_password(password)?;
        let body = json!({
            "account": account,
            "password": pwd_blob,
            "accessCode": access_code,
            "picVerificationCode": captcha_text,
            "fingerPrint": fingerprint(),
            "authType": "email",
            "phoneAccountCode": "",
            "bsMetrics": Value::Null,
        });

        let resp = http
            .inner
            .post(http.url("/api_v2/api/v3/auth/loginsub.json"))
            .header("Referer", http.url("/en_US/login.htm"))
            .header("Origin", &http.base_url)
            .header("content-type", "application/json")
            .header("clienttype", "1")
            .json(&body)
            .send()
            .await?;

        let value = client::parse_envelope(resp).await?;

        match client::api_code(&value) {
            Some(0) => {
                let mut cookies = http.snapshot_cookies()?;

                let access_token = value
                    .pointer("/data/accessToken")
                    .or_else(|| value.pointer("/data/token"))
                    .or_else(|| value.pointer("/data/mucToken"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());

                if let Some(ref tok) = access_token {
                    cookies
                        .entry("muc_token".into())
                        .or_insert_with(|| tok.clone());
                }

                if !cookies.contains_key("muc_token") && access_token.is_none() {
                    warn!(attempt, body = %value, "login code=0 but no token found");
                }

                let session = SessionData {
                    cookies,
                    access_token,
                    account: Some(account.to_string()),
                    saved_at: Some(now_unix()),
                };
                session.save(&cfg.session_path)?;
                info!(
                    attempt,
                    path = %cfg.session_path.display(),
                    cookie_count = session.cookies.len(),
                    "login ok, session saved"
                );
                return Ok(LoginResult {
                    session,
                    attempts: attempt,
                    captcha_used: captcha_text,
                });
            }
            Some(code) => {
                last_message = format!("{code}: {}", client::api_msg(&value));
                warn!(attempt, %last_message, captcha = %captcha_text, "login rejected");
            }
            None => {
                last_message = format!("unexpected response: {value}");
                warn!(attempt, %last_message);
            }
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    Err(Error::LoginExhausted {
        attempts: cfg.login_attempts,
        last_message,
    })
}

async fn fetch_captcha(http: &HttpClient) -> Result<(String, Vec<u8>)> {
    let v = http.get_json("/api_v2/api/v2/genVerifyCode.json").await?;

    if client::api_code(&v) != Some(0) {
        return Err(Error::Api {
            code: client::api_code(&v).unwrap_or(-1),
            message: client::api_msg(&v),
        });
    }

    let data = v
        .get("data")
        .ok_or_else(|| Error::Other("genVerifyCode: missing data".into()))?;
    let access = data
        .get("accessCode")
        .and_then(|a| a.as_str())
        .ok_or_else(|| Error::Other("genVerifyCode: missing accessCode".into()))?
        .to_string();
    let b64 = data
        .get("base64Image")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Other("genVerifyCode: missing base64Image".into()))?;
    let b64 = b64.split(',').next_back().unwrap_or(b64).trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| Error::Other(format!("base64 captcha: {e}")))?;
    Ok((access, bytes))
}

fn fingerprint() -> String {
    format!("orders-rs-{}", &uuid_like()[..16])
}

fn uuid_like() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

fn now_unix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}
