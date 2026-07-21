//! Persisted auth cookies after a successful login.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionData {
    /// Cookie name → value (typically `muc_token`, `JSESSIONID`, locale cookies).
    pub cookies: HashMap<String, String>,
    /// JWT access token when present in the login response body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
}

impl SessionData {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::NotAuthenticated);
        }
        let raw = std::fs::read_to_string(path)?;
        let data: Self = serde_json::from_str(&raw)?;
        if data.cookies.is_empty() && data.access_token.is_none() {
            return Err(Error::NotAuthenticated);
        }
        Ok(data)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn has_auth(&self) -> bool {
        self.cookies.contains_key("muc_token")
            || self
                .access_token
                .as_ref()
                .map(|t| !t.is_empty())
                .unwrap_or(false)
    }
}
