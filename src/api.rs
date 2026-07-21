//! Internal HTTP API for consumers (loka-points, etc.).
//! Reads Postgres only — never BigSeller.

use crate::accounts::{self, get_account_by_code};
use crate::store::{
    cancel_daily_report, find_by_platform_order_id, list_events_since, CancelDailyReport,
    OrderDetailDto, OutboxEvent,
};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Clone)]
pub struct ApiState {
    pub pool: PgPool,
    pub api_token: Option<String>,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/sync/status", get(sync_status))
        .route(
            "/v1/orders/by-platform-id/{platform_order_id}",
            get(lookup_by_platform_id),
        )
        .route("/v1/orders/events", get(events))
        .route("/v1/reports/in-cancel/daily", get(cancel_report))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn serve(state: ApiState, bind: SocketAddr) -> crate::error::Result<()> {
    let app = router(state);
    info!(%bind, "public API listening");
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(crate::error::Error::Io)?;
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::Error::Other(e.to_string()))?;
    Ok(())
}

async fn health(State(st): State<ApiState>) -> impl IntoResponse {
    match crate::db::ping(&st.pool).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "ok": false, "error": "database unavailable" })),
        ),
    }
}

async fn sync_status(
    State(st): State<ApiState>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    check_auth(&st, &headers)?;
    let summary = accounts::latest_sync_summary(&st.pool)
        .await
        .map_err(ApiError::from)?;
    let order_count = accounts::count_orders(&st.pool, None)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(json!({
        "ok": true,
        "orderCount": order_count,
        "recentRuns": summary.get("recentRuns").cloned().unwrap_or(json!([])),
    })))
}

fn check_auth(st: &ApiState, headers: &HeaderMap) -> std::result::Result<(), ApiError> {
    let Some(expected) = st.api_token.as_deref().filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    let got = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or(Some(s)))
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        .unwrap_or("");
    if got == expected {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

async fn resolve_account_id(
    pool: &PgPool,
    account_code: Option<&str>,
) -> std::result::Result<Option<i64>, ApiError> {
    let Some(code) = account_code.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    match get_account_by_code(pool, code)
        .await
        .map_err(ApiError::from)?
    {
        Some(a) => Ok(Some(a.id)),
        None => Err(ApiError::BadRequest(format!(
            "unknown account code: {code}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookupQuery {
    shop_id: Option<i64>,
    platform: Option<String>,
    /// Tenant slug (`bs_accounts.code`), e.g. `default` / `bs-a`
    account: Option<String>,
}

async fn lookup_by_platform_id(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Path(platform_order_id): Path<String>,
    Query(q): Query<LookupQuery>,
) -> std::result::Result<Json<LookupResponse>, ApiError> {
    check_auth(&st, &headers)?;
    let id = platform_order_id.trim();
    if id.is_empty() {
        return Err(ApiError::BadRequest("platform_order_id required".into()));
    }
    let account_id = resolve_account_id(&st.pool, q.account.as_deref()).await?;
    let found =
        find_by_platform_order_id(&st.pool, id, q.shop_id, q.platform.as_deref(), account_id)
            .await
            .map_err(ApiError::from)?;

    if found.is_empty() {
        return Err(ApiError::NotFound);
    }

    let count = found.len();
    let order = found.first().cloned();
    let matches = if count > 1 { found } else { vec![] };
    Ok(Json(LookupResponse {
        found: true,
        count,
        order,
        matches,
    }))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupResponse {
    found: bool,
    count: usize,
    order: Option<OrderDetailDto>,
    matches: Vec<OrderDetailDto>,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    since: Option<i64>,
    limit: Option<i64>,
}

async fn events(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> std::result::Result<Json<EventsResponse>, ApiError> {
    check_auth(&st, &headers)?;
    let since = q.since.unwrap_or(0);
    let limit = q.limit.unwrap_or(50);
    let events = list_events_since(&st.pool, since, limit)
        .await
        .map_err(ApiError::from)?;
    let next_cursor = events.last().map(|e| e.id).unwrap_or(since);
    Ok(Json(EventsResponse {
        events,
        next_cursor,
    }))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EventsResponse {
    events: Vec<OutboxEvent>,
    next_cursor: i64,
}

#[derive(Debug, Deserialize)]
struct CancelReportQuery {
    date: Option<String>,
    #[serde(default = "default_tz")]
    tz_offset_hours: i32,
}

fn default_tz() -> i32 {
    7
}

async fn cancel_report(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CancelReportQuery>,
) -> std::result::Result<Json<CancelDailyReport>, ApiError> {
    check_auth(&st, &headers)?;
    let date = match q.date.as_deref() {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| ApiError::BadRequest("date must be YYYY-MM-DD".into()))?,
        None => {
            let utc = chrono::Utc::now() + chrono::Duration::hours(q.tz_offset_hours as i64);
            utc.date_naive()
        }
    };
    let report = cancel_daily_report(&st.pool, date, q.tz_offset_hours)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(report))
}

enum ApiError {
    Unauthorized,
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl From<crate::error::Error> for ApiError {
    fn from(e: crate::error::Error) -> Self {
        // Never echo raw sqlx/DB diagnostics (may include connection URLs).
        let message = match &e {
            crate::error::Error::Db(_) => "database error".into(),
            other => other.to_string(),
        };
        ApiError::Internal(message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, json!({ "error": "unauthorized" }))
            }
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                json!({
                    "error": "not_found",
                    "message": "order not in cache yet; wait for worker sync"
                }),
            ),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, json!({ "error": m })),
            ApiError::Internal(m) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "internal", "message": m }),
            ),
        };
        (status, Json(body)).into_response()
    }
}
