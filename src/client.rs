//! Shared HTTP client for BigSeller (cookie jar + `clienttype` header).

use crate::error::{Error, Result};
use crate::session::SessionData;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use reqwest_cookie_store::{CookieStore, CookieStoreMutex, RawCookie};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct HttpClient {
    pub inner: Client,
    pub base_url: String,
    cookie_store: Arc<CookieStoreMutex>,
}

impl HttpClient {
    pub fn new(base_url: &str) -> Result<Self> {
        Self::build(base_url, CookieStore::default())
    }

    /// Client that restores cookies from a saved session.
    pub fn with_session(base_url: &str, session: &SessionData) -> Result<Self> {
        let mut store = CookieStore::default();
        let origin = origin_url(base_url)?;
        for (name, value) in &session.cookies {
            let line = format!("{name}={value}; Path=/");
            let raw = RawCookie::parse(line)
                .map_err(|e| Error::Other(format!("invalid session cookie {name}: {e}")))?;
            let _ = store.insert_raw(&raw, &origin);
        }
        // Prefer explicit access_token as muc_token when jar has no JWT yet.
        if !session.cookies.contains_key("muc_token") {
            if let Some(tok) = session.access_token.as_deref().filter(|t| !t.is_empty()) {
                let line = format!("muc_token={tok}; Path=/");
                if let Ok(raw) = RawCookie::parse(line) {
                    let _ = store.insert_raw(&raw, &origin);
                }
            }
        }
        Self::build(base_url, store)
    }

    fn build(base_url: &str, store: CookieStore) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert("clienttype", HeaderValue::from_static("1"));
        headers.insert(
            "Accept",
            HeaderValue::from_static("application/json, text/plain, */*"),
        );

        let cookie_store = Arc::new(CookieStoreMutex::new(store));
        let inner = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .cookie_provider(Arc::clone(&cookie_store))
            .build()?;

        Ok(Self {
            inner,
            base_url: base_url.trim_end_matches('/').to_string(),
            cookie_store,
        })
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.base_url, path)
        }
    }

    /// Snapshot name→value for cookies that would be sent to the site origin.
    pub fn snapshot_cookies(&self) -> Result<HashMap<String, String>> {
        let origin = origin_url(&self.base_url)?;
        let guard = self
            .cookie_store
            .lock()
            .map_err(|_| Error::Other("cookie store lock poisoned".into()))?;
        let mut map = HashMap::new();
        for (name, value) in guard.get_request_values(&origin) {
            map.insert(name.to_string(), value.to_string());
        }
        // Also keep any other host cookies under bigseller domains (iter_any).
        for c in guard.iter_any() {
            map.entry(c.name().to_string())
                .or_insert_with(|| c.value().to_string());
        }
        Ok(map)
    }

    /// GET JSON and map BigSeller envelope `{ code, msg, data }`.
    pub async fn get_json(&self, path: &str) -> Result<Value> {
        let resp = self
            .inner
            .get(self.url(path))
            .header("Referer", format!("{}/en_US/login.htm", self.base_url))
            .send()
            .await?;
        parse_envelope(resp).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .inner
            .post(self.url(path))
            .header("Referer", format!("{}/web/order/index.htm", self.base_url))
            .header("Origin", &self.base_url)
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await?;
        parse_envelope(resp).await
    }
}

fn origin_url(base_url: &str) -> Result<Url> {
    Url::parse(base_url.trim_end_matches('/'))
        .map_err(|e| Error::Other(format!("invalid base URL: {e}")))
}

pub async fn parse_envelope(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await?;
    let value: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        serde_json::json!({
            "code": -1,
            "msg": text.chars().take(200).collect::<String>(),
            "data": null
        })
    });

    if !status.is_success() {
        let msg = value
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or(status.as_str());
        return Err(Error::Api {
            code: status.as_u16() as i64,
            message: msg.to_string(),
        });
    }

    Ok(value)
}

/// Extract API business code from envelope (0 = success).
pub fn api_code(v: &Value) -> Option<i64> {
    v.get("code")
        .and_then(|c| c.as_i64().or_else(|| c.as_u64().map(|u| u as i64)))
}

pub fn api_msg(v: &Value) -> String {
    v.get("msg")
        .or_else(|| v.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn ensure_ok(v: &Value) -> Result<()> {
    match api_code(v) {
        Some(0) => Ok(()),
        Some(code) if is_auth_expired_code(code) => {
            Err(Error::AuthExpired(format!("{code}: {}", api_msg(v))))
        }
        Some(code) => Err(Error::Api {
            code,
            message: api_msg(v),
        }),
        None => Ok(()),
    }
}

/// BigSeller codes that mean "re-login required".
pub fn is_auth_expired_code(code: i64) -> bool {
    matches!(code, 2001 | 401 | 401006)
}

pub fn is_auth_error(err: &Error) -> bool {
    match err {
        Error::AuthExpired(_) => true,
        Error::Api { code, .. } => is_auth_expired_code(*code),
        Error::NotAuthenticated => true,
        _ => false,
    }
}
