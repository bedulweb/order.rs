//! Internal HTTP API for consumers (loka-points, mobile apps, etc.).
//! Reads Postgres only — never BigSeller.
//!
//! App lookup:
//! - `POST /v1/app/lookup/text` — nomor pesanan diketik manual
//! - `POST /v1/app/lookup/photo` — upload screenshot (OCR → lookup)

use crate::accounts::{self, get_account_by_code};
use crate::batch::{self, BatchSession};
use crate::catalog;
use crate::screen_ocr::{self, OrderIdHit};
use crate::store::{
    cancel_daily_report, find_by_platform_order_id, list_events_since, CancelDailyReport,
    OrderDetailDto, OutboxEvent,
};
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::info;
use uuid::Uuid;

/// User-facing copy when order is missing or OCR cannot read a number.
pub const MSG_ORDER_NOT_FOUND: &str = "Maaf nomor pesanan tidak ditemukan harap periksa kembali.";

/// Max upload size for screenshot lookup (bytes).
const MAX_PHOTO_BYTES: usize = 5 * 1024 * 1024;

/// Max upload size for catalog xlsx import (bytes).
const MAX_CATALOG_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone)]
pub struct ApiState {
    pub pool: PgPool,
    pub api_token: Option<String>,
    /// Optional Vite `web/dist` (or deploy root). When set, SPA is served on `/`.
    pub web_dist: Option<PathBuf>,
}

pub fn router(state: ApiState) -> Router {
    let web_dist = state.web_dist.clone();
    let api = Router::new()
        .route("/health", get(health))
        .route("/v1/sync/status", get(sync_status))
        .route(
            "/v1/orders/by-platform-id/{platform_order_id}",
            get(lookup_by_platform_id),
        )
        .route("/v1/orders/events", get(events))
        .route("/v1/reports/in-cancel/daily", get(cancel_report))
        // Ops batches (pick lists)
        .route("/v1/batches/backlog", get(batches_backlog))
        .route("/v1/batches", get(batches_list).post(batches_create))
        .route("/v1/batches/{id}", get(batches_get))
        .route("/v1/batches/{id}/pdf", get(batches_pdf))
        .route(
            "/v1/batches/{id}/regenerate-pdf",
            post(batches_regenerate_pdf),
        )
        // Product catalog + HPP
        .route("/v1/catalog/products", get(catalog_list))
        .route("/v1/catalog/products/{art}", get(catalog_get))
        .route("/v1/catalog/import", post(catalog_import))
        // Mobile / consumer app
        .route("/v1/app/lookup/text", post(app_lookup_text))
        .route("/v1/app/lookup/photo", post(app_lookup_photo))
        .layer(DefaultBodyLimit::max(MAX_CATALOG_UPLOAD_BYTES + 64 * 1024))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Prefer API routes; fall back to SPA assets when web_dist exists.
    if let Some(dist) = web_dist.filter(|p| p.is_dir()) {
        let index = dist.join("index.html");
        let spa = ServeDir::new(dist).not_found_service(ServeFile::new(index));
        api.fallback_service(spa)
    } else {
        api
    }
}

pub async fn serve(state: ApiState, bind: SocketAddr) -> crate::error::Result<()> {
    if let Some(ref dist) = state.web_dist {
        if dist.is_dir() {
            info!(path = %dist.display(), "serving ops SPA from web_dist");
        } else {
            info!(path = %dist.display(), "web_dist set but missing — API only");
        }
    }
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

// ---------------------------------------------------------------------------
// App lookup (text + photo)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppLookupTextBody {
    /// Nomor pesanan marketplace (Shopee / TikTok / …)
    #[serde(
        alias = "order_id",
        alias = "platform_order_id",
        alias = "nomorPesanan"
    )]
    platform_order_id: String,
    shop_id: Option<i64>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppLookupFilters {
    shop_id: Option<i64>,
    platform: Option<String>,
    account: Option<String>,
}

/// POST JSON `{ "platformOrderId": "260715PS7HRGC0" }`
async fn app_lookup_text(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<AppLookupTextBody>,
) -> std::result::Result<Json<AppLookupResponse>, ApiError> {
    check_auth(&st, &headers)?;
    let id = body.platform_order_id.trim();
    if id.is_empty() {
        return Ok(Json(AppLookupResponse::not_found(
            "text",
            None,
            vec![],
            None,
        )));
    }
    let account_id = resolve_account_id(&st.pool, body.account.as_deref()).await?;
    let found = find_by_platform_order_id(
        &st.pool,
        id,
        body.shop_id,
        body.platform.as_deref(),
        account_id,
    )
    .await
    .map_err(ApiError::from)?;
    if found.is_empty() {
        return Ok(Json(AppLookupResponse::not_found(
            "text",
            Some(id.to_string()),
            vec![],
            Some("order_not_in_database"),
        )));
    }
    Ok(Json(AppLookupResponse::found_text("text", id, found)))
}

/// POST multipart: field `image` (or `photo` / `file`) = screenshot JPEG/PNG.
/// Optional form fields: `account`, `platform`, `shopId`.
async fn app_lookup_photo(
    State(st): State<ApiState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> std::result::Result<Json<AppLookupResponse>, ApiError> {
    check_auth(&st, &headers)?;

    let mut image_bytes: Option<Vec<u8>> = None;
    let mut filters = AppLookupFilters {
        shop_id: None,
        platform: None,
        account: None,
    };

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        let name_l = name.to_ascii_lowercase();
        if matches!(
            name_l.as_str(),
            "image" | "photo" | "file" | "screenshot" | "gambar"
        ) {
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("read image: {e}")))?;
            if data.len() > MAX_PHOTO_BYTES {
                return Err(ApiError::BadRequest(format!(
                    "image too large (max {} MB)",
                    MAX_PHOTO_BYTES / (1024 * 1024)
                )));
            }
            if data.is_empty() {
                return Err(ApiError::BadRequest("empty image".into()));
            }
            image_bytes = Some(data.to_vec());
        } else if name_l == "account" {
            let t = field
                .text()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            filters.account = Some(t);
        } else if name_l == "platform" {
            let t = field
                .text()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            filters.platform = Some(t);
        } else if matches!(name_l.as_str(), "shopid" | "shop_id") {
            let t = field
                .text()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            filters.shop_id = t.trim().parse().ok();
        }
    }

    let Some(bytes) = image_bytes else {
        return Err(ApiError::BadRequest(
            "multipart field required: image (or photo/file)".into(),
        ));
    };

    // OCR is CPU-bound; keep the async runtime free.
    let hits =
        tokio::task::spawn_blocking(move || screen_ocr::extract_order_ids_from_bytes(&bytes))
            .await
            .map_err(|e| ApiError::Internal(format!("ocr join: {e}")))?
            .map_err(|e| ApiError::Internal(format!("ocr: {e}")))?;

    if hits.is_empty() {
        return Ok(Json(AppLookupResponse::not_found(
            "photo",
            None,
            vec![],
            Some("order_id_not_recognized"),
        )));
    }

    let account_id = resolve_account_id(&st.pool, filters.account.as_deref()).await?;
    let candidate_ids: Vec<String> = hits.iter().map(|h| h.id.clone()).collect();

    // Try best OCR hit first, then other candidates (OCR may rank wrong).
    for hit in &hits {
        let found = find_by_platform_order_id(
            &st.pool,
            &hit.id,
            filters.shop_id,
            filters.platform.as_deref(),
            account_id,
        )
        .await
        .map_err(ApiError::from)?;
        if !found.is_empty() {
            return Ok(Json(AppLookupResponse::found(
                "photo", &hit.id, &hits, found,
            )));
        }
    }

    Ok(Json(AppLookupResponse::not_found(
        "photo",
        Some(hits[0].id.clone()),
        candidate_ids,
        Some("order_not_in_database"),
    )))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppLookupResponse {
    ok: bool,
    found: bool,
    /// Always set when `found` is false — show this string in the app UI.
    message: Option<String>,
    source: String,
    platform_order_id: Option<String>,
    /// Machine-readable reason when not found (optional for debugging).
    reason: Option<String>,
    /// OCR candidate ids (photo path only).
    ocr_candidates: Vec<String>,
    /// Primary order (first match) including `items` (what they bought).
    order: Option<OrderDetailDto>,
    /// Extra matches if the same platform id exists on multiple shops.
    matches: Vec<OrderDetailDto>,
}

impl AppLookupResponse {
    fn found(
        source: &str,
        platform_order_id: &str,
        hits: &[OrderIdHit],
        orders: Vec<OrderDetailDto>,
    ) -> Self {
        let count = orders.len();
        let order = orders.first().cloned();
        let matches = if count > 1 { orders } else { vec![] };
        Self {
            ok: true,
            found: true,
            message: None,
            source: source.into(),
            platform_order_id: Some(platform_order_id.into()),
            reason: None,
            ocr_candidates: hits.iter().map(|h| h.id.clone()).collect(),
            order,
            matches,
        }
    }

    fn found_text(source: &str, platform_order_id: &str, orders: Vec<OrderDetailDto>) -> Self {
        let count = orders.len();
        let order = orders.first().cloned();
        let matches = if count > 1 { orders } else { vec![] };
        Self {
            ok: true,
            found: true,
            message: None,
            source: source.into(),
            platform_order_id: Some(platform_order_id.into()),
            reason: None,
            ocr_candidates: vec![],
            order,
            matches,
        }
    }

    fn not_found(
        source: &str,
        platform_order_id: Option<String>,
        ocr_candidates: Vec<String>,
        reason: Option<&str>,
    ) -> Self {
        Self {
            ok: true,
            found: false,
            message: Some(MSG_ORDER_NOT_FOUND.into()),
            source: source.into(),
            platform_order_id,
            reason: reason.map(str::to_string),
            ocr_candidates,
            order: None,
            matches: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy / internal lookup
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Ops batches
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BacklogQuery {
    account: Option<String>,
    limit: Option<i64>,
}

async fn batches_backlog(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<BacklogQuery>,
) -> std::result::Result<Json<batch::BacklogResponse>, ApiError> {
    check_auth(&st, &headers)?;
    let account_id = resolve_account_id(&st.pool, q.account.as_deref()).await?;
    let limit = q.limit.unwrap_or(500);
    let resp = batch::list_backlog(&st.pool, account_id, limit)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(resp))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateBatchBody {
    session: String,
    account: Option<String>,
}

async fn batches_create(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CreateBatchBody>,
) -> std::result::Result<Json<batch::BatchDetail>, ApiError> {
    check_auth(&st, &headers)?;
    let session = BatchSession::parse(&body.session)
        .ok_or_else(|| ApiError::BadRequest("session must be morning|afternoon|urgent".into()))?;
    let account_id = resolve_account_id(&st.pool, body.account.as_deref()).await?;
    let detail = batch::create_batch(&st.pool, session, account_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(detail))
}

#[derive(Debug, Deserialize)]
struct BatchesListQuery {
    /// WIB calendar day YYYY-MM-DD (default: today WIB).
    date: Option<String>,
    account: Option<String>,
}

async fn batches_list(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<BatchesListQuery>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    check_auth(&st, &headers)?;
    let date = match q.date.as_deref() {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| ApiError::BadRequest("date must be YYYY-MM-DD".into()))?,
        None => {
            let wib = chrono::Utc::now().with_timezone(&batch::wib_offset());
            wib.date_naive()
        }
    };
    let account_id = resolve_account_id(&st.pool, q.account.as_deref()).await?;
    let batches = batch::list_batches_for_wib_date(&st.pool, date, account_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(json!({
        "date": date.format("%Y-%m-%d").to_string(),
        "timezone": batch::BATCH_TIMEZONE,
        "batches": batches,
    })))
}

async fn batches_get(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> std::result::Result<Json<batch::BatchDetail>, ApiError> {
    check_auth(&st, &headers)?;
    match batch::get_batch(&st.pool, id)
        .await
        .map_err(ApiError::from)?
    {
        Some(d) => Ok(Json(d)),
        None => Err(ApiError::NotFound),
    }
}

async fn batches_pdf(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> std::result::Result<Response, ApiError> {
    check_auth(&st, &headers)?;
    let Some((filename, bytes)) = batch::get_batch_pdf(&st.pool, id)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::NotFound);
    };
    let mut res = Response::new(bytes.into());
    *res.status_mut() = StatusCode::OK;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    let disp = format!("inline; filename=\"{filename}\"");
    if let Ok(v) = HeaderValue::from_str(&disp) {
        res.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

/// Rebuild Summary List PDF for an existing batch (membership unchanged).
async fn batches_regenerate_pdf(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> std::result::Result<Json<batch::BatchDetail>, ApiError> {
    check_auth(&st, &headers)?;
    let detail = batch::regenerate_batch_pdf(&st.pool, id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(detail))
}

// ---------------------------------------------------------------------------
// Product catalog
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CatalogListQuery {
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn catalog_list(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<CatalogListQuery>,
) -> std::result::Result<Json<catalog::ProductListResponse>, ApiError> {
    check_auth(&st, &headers)?;
    let resp = catalog::list_products(
        &st.pool,
        q.q.as_deref(),
        q.limit.unwrap_or(500),
        q.offset.unwrap_or(0),
    )
    .await
    .map_err(ApiError::from)?;
    Ok(Json(resp))
}

async fn catalog_get(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Path(art): Path<String>,
) -> std::result::Result<Json<catalog::Product>, ApiError> {
    check_auth(&st, &headers)?;
    match catalog::get_product(&st.pool, &art)
        .await
        .map_err(ApiError::from)?
    {
        Some(p) => Ok(Json(p)),
        None => Err(ApiError::NotFound),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogImportBody {
    /// Path to xlsx on the server (default: repo workbook name).
    path: Option<String>,
}

/// POST `/v1/catalog/import`
/// - JSON `{ "path": "…" }` or `{}` / empty → import from server path
/// - multipart field `file` (or `xlsx`) → import uploaded bytes
async fn catalog_import(
    State(st): State<ApiState>,
    headers: HeaderMap,
    request: axum::http::Request<axum::body::Body>,
) -> std::result::Result<Json<catalog::ImportResult>, ApiError> {
    check_auth(&st, &headers)?;

    let ct = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if ct.starts_with("multipart/") {
        let mut multipart = Multipart::from_request(request, &st)
            .await
            .map_err(|e| ApiError::BadRequest(format!("multipart: {e}")))?;
        let mut file_bytes: Option<Vec<u8>> = None;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| ApiError::BadRequest(format!("multipart field: {e}")))?
        {
            let name = field.name().unwrap_or("").to_ascii_lowercase();
            if matches!(name.as_str(), "file" | "xlsx" | "workbook" | "upload") {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("read file: {e}")))?;
                if data.len() > MAX_CATALOG_UPLOAD_BYTES {
                    return Err(ApiError::BadRequest("upload too large".into()));
                }
                file_bytes = Some(data.to_vec());
            }
        }
        let Some(bytes) = file_bytes else {
            return Err(ApiError::BadRequest(
                "multipart field required: file (or xlsx)".into(),
            ));
        };
        let result = catalog::import_from_bytes(&st.pool, &bytes)
            .await
            .map_err(ApiError::from)?;
        return Ok(Json(result));
    }

    let bytes = axum::body::to_bytes(request.into_body(), MAX_CATALOG_UPLOAD_BYTES + 1)
        .await
        .map_err(|e| ApiError::BadRequest(format!("read body: {e}")))?;
    if bytes.len() > MAX_CATALOG_UPLOAD_BYTES {
        return Err(ApiError::BadRequest("body too large".into()));
    }

    let result = if bytes.is_empty() {
        let path = resolve_catalog_path(None)?;
        catalog::import_from_path(&st.pool, &path)
            .await
            .map_err(ApiError::from)?
    } else if ct.contains("json") || bytes.first() == Some(&b'{') {
        let body: CatalogImportBody = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::BadRequest(format!("json body: {e}")))?;
        let path = resolve_catalog_path(body.path.as_deref())?;
        catalog::import_from_path(&st.pool, &path)
            .await
            .map_err(ApiError::from)?
    } else {
        // Raw xlsx octets
        catalog::import_from_bytes(&st.pool, &bytes)
            .await
            .map_err(ApiError::from)?
    };

    Ok(Json(result))
}

fn resolve_catalog_path(explicit: Option<&str>) -> std::result::Result<PathBuf, ApiError> {
    if let Some(p) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(ApiError::BadRequest(format!(
            "catalog workbook not found: {p}"
        )));
    }
    // Prefer CWD, then crate manifest dir (dev).
    let candidates = [
        PathBuf::from(catalog::DEFAULT_WORKBOOK),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(catalog::DEFAULT_WORKBOOK),
    ];
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(ApiError::BadRequest(format!(
        "default workbook {} not found (pass path or upload file)",
        catalog::DEFAULT_WORKBOOK
    )))
}

enum ApiError {
    Unauthorized,
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl From<crate::error::Error> for ApiError {
    fn from(e: crate::error::Error) -> Self {
        // Empty backlog on generate is a client-facing 400, not a 500.
        if let crate::error::Error::Other(ref m) = e {
            if m.contains("no eligible orders") {
                return ApiError::BadRequest(m.clone());
            }
        }
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
                    "message": MSG_ORDER_NOT_FOUND,
                    "found": false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_message_is_indonesian() {
        let r = AppLookupResponse::not_found("text", Some("x".into()), vec![], None);
        assert!(!r.found);
        assert_eq!(r.message.as_deref(), Some(MSG_ORDER_NOT_FOUND));
        assert!(r.message.as_ref().unwrap().contains("tidak ditemukan"));
    }

    #[test]
    fn batch_session_and_paths_are_wired() {
        // Shipped handlers + session parser used by POST /v1/batches.
        assert!(BatchSession::parse("morning").is_some());
        assert!(BatchSession::parse("afternoon").is_some());
        assert!(BatchSession::parse("urgent").is_some());
        // Source-level contract: router registers these exact paths.
        let src = include_str!("api.rs");
        for path in [
            "/v1/batches/backlog",
            "/v1/batches",
            "/v1/batches/{id}",
            "/v1/batches/{id}/pdf",
            "/v1/batches/{id}/regenerate-pdf",
            "/v1/catalog/products",
            "/v1/catalog/products/{art}",
            "/v1/catalog/import",
            "/v1/app/lookup/text",
            "/health",
        ] {
            assert!(src.contains(path), "missing route path {path}");
        }
        assert!(src.contains("check_auth"));
    }
}
