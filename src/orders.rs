//! Order list / status-count APIs (authenticated).

use crate::client::{self, HttpClient};
use crate::error::Result;
use crate::session::SessionData;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::debug;

#[derive(Clone)]
pub struct OrdersApi {
    http: HttpClient,
}

impl OrdersApi {
    pub fn new(base_url: &str, session: &SessionData) -> Result<Self> {
        Ok(Self {
            http: HttpClient::with_session(base_url, session)?,
        })
    }

    /// Status counts for the order sidebar (new / packing / …).
    pub async fn status_counts(&self) -> Result<Value> {
        let body = json!({});
        let v = self
            .http
            .post_json("/api/v1/order/getOrderStatusCount.json", &body)
            .await?;
        client::ensure_ok(&v)?;
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }

    /// Probe session: `GET /api/v1/isLogin.json`.
    pub async fn is_login(&self) -> Result<bool> {
        let v = self.http.get_json("/api/v1/isLogin.json").await?;
        client::ensure_ok(&v)?;
        Ok(v.get("data").and_then(|d| d.as_bool()).unwrap_or(false))
    }

    /// Paginated order list for a status bucket (e.g. `"new"`).
    pub async fn page_list(&self, query: &OrderListQuery) -> Result<OrderPage> {
        let body = query.to_json();
        debug!(%body, "order pageList request");
        let v = self
            .http
            .post_json("/api/v1/order/new/pageList.json", &body)
            .await?;
        client::ensure_ok(&v)?;

        let page = v.pointer("/data/page").cloned().unwrap_or(Value::Null);

        let rows = page
            .get("rows")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        // BigSeller uses totalSize (and sometimes total).
        let total = page
            .get("totalSize")
            .or_else(|| page.get("total"))
            .and_then(|t| t.as_u64().or_else(|| t.as_i64().map(|i| i as u64)))
            .unwrap_or(rows.len() as u64);

        Ok(OrderPage {
            total,
            rows,
            raw: v,
        })
    }
}

#[derive(Debug, Clone)]
pub struct OrderListQuery {
    pub status: String,
    pub page_no: u32,
    pub page_size: u32,
    pub order_by: String,
    /// UI "new" tab uses `"0"`. Historical completed/shipped need `None` (null/empty).
    pub pack_state: Option<String>,
    /// Include older BigSeller history rows (completed archive, etc.).
    pub history_order: bool,
    /// When true with empty status, broader listing (use carefully).
    pub all_order: bool,
    /// Marketplace order no / package search (BigSeller `searchType: orderNo`).
    pub search_content: Option<String>,
}

impl Default for OrderListQuery {
    fn default() -> Self {
        Self {
            status: "new".into(),
            page_no: 1,
            page_size: 50,
            order_by: "expireTime".into(),
            pack_state: Some("0".into()),
            history_order: false,
            all_order: false,
            search_content: None,
        }
    }
}

impl OrderListQuery {
    /// Active "To Process / New" style list (default worker path).
    pub fn active(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            ..Default::default()
        }
    }

    /// Historical / completed archive listing.
    ///
    /// BigSeller returns empty pages for `completed`/`shipped` when `packState` is `"0"`.
    /// Use `history_order: true` and no pack filter.
    pub fn historical(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            page_no: 1,
            page_size: 50,
            order_by: "orderCreateTime".into(),
            pack_state: None,
            history_order: true,
            all_order: false,
            search_content: None,
        }
    }

    /// Search one marketplace order number (active + recent tabs).
    ///
    /// Live BigSeller returns hits with `allOrder=true` and `historyOrder=false`.
    /// Historical archive search often returns empty for in-flight shipped orders.
    pub fn search_order_no(order_no: impl Into<String>) -> Self {
        Self {
            status: String::new(),
            page_no: 1,
            page_size: 20,
            order_by: "orderCreateTime".into(),
            pack_state: None,
            history_order: false,
            all_order: true,
            search_content: Some(order_no.into()),
        }
    }

    pub fn to_json(&self) -> Value {
        // Shape aligned with live UI capture (docs/pageList-request-template.json).
        let pack_state = match &self.pack_state {
            Some(s) => json!(s),
            None => Value::Null,
        };
        let search_content = match &self.search_content {
            Some(s) => json!(s),
            None => Value::Null,
        };
        json!({
            "status": self.status,
            "pageNo": self.page_no,
            "pageSize": self.page_size,
            "orderBy": self.order_by,
            "inquireType": 2,
            "searchType": "orderNo",
            "searchContent": search_content,
            "platform": null,
            "shopId": null,
            "warehouseId": null,
            "timeType": 1,
            "days": "",
            "beginDate": "",
            "endDate": "",
            "printStatus": null,
            "printLabelMark": null,
            "printCollectMark": null,
            "packState": pack_state,
            "allOrder": self.all_order,
            "historyOrder": self.history_order,
            "desc": 1,
            "showLogisticsArr": 0,
            "showStoreArr": 0,
        })
    }
}

#[derive(Debug, Clone)]
pub struct OrderPage {
    pub total: u64,
    pub rows: Vec<Value>,
    pub raw: Value,
}

/// Compact row summary for CLI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSummary {
    pub platform_order_id: Option<String>,
    pub platform: Option<String>,
    pub shop_name: Option<String>,
    pub buyer: Option<String>,
    pub status: Option<String>,
    pub amount: Option<String>,
}

impl OrderSummary {
    pub fn from_row(row: &Value) -> Self {
        Self {
            platform_order_id: str_field(
                row,
                &[
                    "platformOrderId",
                    "platform_order_id",
                    "orderId",
                    "ordersn",
                    "orderSn",
                    "orderNo",
                ],
            ),
            platform: str_field(row, &["platform", "platformName", "platform_name"]),
            shop_name: str_field(row, &["shopName", "shop_name", "storeName"]),
            buyer: str_field(
                row,
                &[
                    "buyerUsername",
                    "buyerName",
                    "receiverName",
                    "buyer",
                    "customerName",
                ],
            ),
            status: str_field(
                row,
                &["orderStatus", "status", "packageStatus", "stateName"],
            ),
            amount: str_field(
                row,
                &[
                    "orderAmount",
                    "payAmount",
                    "totalAmount",
                    "amount",
                    "payment",
                ],
            )
            .or_else(|| num_field(row, &["orderAmount", "payAmount", "totalAmount", "amount"])),
        }
    }
}

fn num_field(row: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(n) = v.as_f64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_i64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_u64() {
                return Some(n.to_string());
            }
        }
    }
    None
}

fn str_field(row: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
            if let Some(n) = v.as_i64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_u64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_f64() {
                return Some(n.to_string());
            }
        }
    }
    None
}
