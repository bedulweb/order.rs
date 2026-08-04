use crate::error::{Error, Result};
use std::env;
use std::path::{Path, PathBuf};

/// Runtime configuration for the BigSeller client + API/worker.
#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub account: Option<String>,
    pub password: Option<String>,
    pub model_path: PathBuf,
    pub charset_path: PathBuf,
    pub session_path: PathBuf,
    pub ocr_threads: usize,
    pub login_attempts: usize,
    /// Neon / Postgres connection string.
    pub database_url: Option<String>,
    /// Bearer / x-api-key for public API (optional in dev).
    pub api_token: Option<String>,
    pub api_bind: String,
    pub sync_new_interval_secs: u64,
    /// Max stale orders healed per reconcile cycle (default 15).
    pub reconcile_cap: i64,
    pub cancel_hour_local: u32,
    pub cancel_minute_local: u32,
    /// Working-hours window (WIB minutes since midnight) for outbox delivery.
    /// Default 07:50–17:00 (470–1020). Outside the window, urgent/instant
    /// order notifications are deferred to the next work start.
    pub work_hour_start_min: u32,
    pub work_hour_end_min: u32,
    pub wa_webhook_url: Option<String>,
    pub wa_webhook_token: Option<String>,
    /// Wazapin group notify (instant orders). None if env incomplete.
    pub wazapin: Option<crate::wazapin::WazapinConfig>,
    /// Tenant slug stored on bs_accounts.code (default: "default").
    pub account_code: Option<String>,
    /// Worker re-login on BS auth expiry (default true).
    pub auto_relogin: bool,
    /// Directory of built ops SPA (`web/dist`). Empty/`off` disables static serve.
    pub web_dist: Option<PathBuf>,
}

impl Config {
    /// Load from environment / defaults relative to the current working directory
    /// (or the crate directory when running via `cargo run` from the project root).
    pub fn from_env() -> Result<Self> {
        let root = project_root();
        let model_path = env::var("BS_MODEL_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| root.join("models/common_old.onnx"));
        let charset_path = env::var("BS_CHARSET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| root.join("models/charset.json"));
        let session_path = env::var("BS_SESSION_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| root.join(".session.json"));

        let ocr_threads = env::var("BS_OCR_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4)
            .max(1);

        let login_attempts = env::var("BS_LOGIN_ATTEMPTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8)
            .max(1);

        Ok(Self {
            base_url: env::var("BS_BASE_URL")
                .unwrap_or_else(|_| "https://www.bigseller.com".into()),
            account: env::var("BS_ACCOUNT").ok().filter(|s| !s.is_empty()),
            password: env::var("BS_PASSWORD").ok().filter(|s| !s.is_empty()),
            model_path,
            charset_path,
            session_path,
            ocr_threads,
            login_attempts,
            database_url: env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()),
            api_token: env::var("API_TOKEN").ok().filter(|s| !s.is_empty()),
            api_bind: env::var("API_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            sync_new_interval_secs: env::var("SYNC_NEW_INTERVAL_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60)
                .max(15),
            reconcile_cap: env::var("RECONCILE_CAP")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(15)
                .clamp(1, 500),
            cancel_hour_local: env::var("CANCEL_HOUR_LOCAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(17)
                .min(23),
            cancel_minute_local: env::var("CANCEL_MINUTE_LOCAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
                .min(59),
            work_hour_start_min: env_hhmm("WORK_HOUR_START", crate::batch::WORK_HOUR_START_MIN),
            work_hour_end_min: env_hhmm("WORK_HOUR_END", crate::batch::WORK_HOUR_END_MIN),
            wa_webhook_url: env::var("WA_WEBHOOK_URL").ok().filter(|s| !s.is_empty()),
            wa_webhook_token: env::var("WA_WEBHOOK_TOKEN").ok().filter(|s| !s.is_empty()),
            wazapin: crate::wazapin::WazapinConfig::from_env(),
            account_code: env::var("BS_ACCOUNT_CODE").ok().filter(|s| !s.is_empty()),
            auto_relogin: env::var("AUTO_RELOGIN")
                .ok()
                .map(|s| matches!(s.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(true),
            web_dist: match env::var("WEB_DIST") {
                Ok(s) if matches!(s.to_ascii_lowercase().as_str(), "" | "off" | "0" | "false") => {
                    None
                }
                Ok(s) => Some(PathBuf::from(s)),
                Err(_) => {
                    let default = root.join("web/dist");
                    if default.is_dir() {
                        Some(default)
                    } else {
                        None
                    }
                }
            },
        })
    }

    pub fn require_credentials(&self) -> Result<(&str, &str)> {
        let account = self
            .account
            .as_deref()
            .ok_or_else(|| Error::Config("BS_ACCOUNT is required".into()))?;
        let password = self
            .password
            .as_deref()
            .ok_or_else(|| Error::Config("BS_PASSWORD is required".into()))?;
        Ok((account, password))
    }

    pub fn require_database_url(&self) -> Result<&str> {
        self.database_url
            .as_deref()
            .ok_or_else(|| Error::Config("DATABASE_URL is required".into()))
    }

    pub fn validate_paths(&self) -> Result<()> {
        if !self.model_path.is_file() {
            return Err(Error::Config(format!(
                "ONNX model not found at {} (copy common_old.onnx from ddddocr package)",
                self.model_path.display()
            )));
        }
        if !self.charset_path.is_file() {
            return Err(Error::Config(format!(
                "charset not found at {}",
                self.charset_path.display()
            )));
        }
        Ok(())
    }
}

/// Prefer crate root when CARGO_MANIFEST_DIR is set (cargo run / tests).
fn project_root() -> PathBuf {
    env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn resolve_under_root(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root().join(path)
    }
}

/// Parse an env var as `HH:MM` (24h) into minutes since midnight, falling back
/// to `default_min` when unset/invalid.
fn env_hhmm(key: &str, default_min: u32) -> u32 {
    let raw = match env::var(key) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return default_min,
    };
    let mut parts = raw.split(':');
    let h: u32 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default_min / 60);
    let m: u32 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default_min % 60);
    (h.min(23) * 60 + m.min(59)).min(24 * 60 - 1)
}
