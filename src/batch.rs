//! Ops batch domain: backlog membership, urgent classification, generate + lock.
//!
//! Source of truth for “already processed” is active `batch_orders` membership
//! (`voided_at IS NULL`), not wall-clock cutoffs or BigSeller print marks.

use crate::error::{Error, Result};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::fmt;
use uuid::Uuid;

/// Asia/Jakarta fixed offset (WIB, no DST).
pub const WIB_OFFSET_SECS: i32 = 7 * 3600;
pub const BATCH_TIMEZONE: &str = "Asia/Jakarta";

const URGENT_KEYWORDS: &[&str] = &[
    "instant",
    "sameday",
    "same day",
    "same-day",
    "prioritas",
    "gojek",
    "gosend",
    "grab",
    "paxel",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchSession {
    Morning,
    Afternoon,
    Urgent,
}

impl BatchSession {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Morning => "morning",
            Self::Afternoon => "afternoon",
            Self::Urgent => "urgent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "morning" => Some(Self::Morning),
            "afternoon" => Some(Self::Afternoon),
            "urgent" => Some(Self::Urgent),
            _ => None,
        }
    }
}

impl fmt::Display for BatchSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Case-insensitive urgent match on concatenated carrier fields.
pub fn is_urgent_carrier(
    buyer_shipping_carrier: Option<&str>,
    shipment_provider: Option<&str>,
    shipping_carrier_name: Option<&str>,
) -> bool {
    let hay = [
        buyer_shipping_carrier.unwrap_or(""),
        shipment_provider.unwrap_or(""),
        shipping_carrier_name.unwrap_or(""),
    ]
    .join(" ")
    .to_ascii_lowercase();
    if hay.trim().is_empty() {
        return false;
    }
    URGENT_KEYWORDS.iter().any(|kw| hay.contains(kw))
}

/// Sort key for session pick lists: urgent first, then oldest ordered_at.
pub fn sort_backlog_orders(rows: &mut [BacklogOrder]) {
    rows.sort_by(|a, b| {
        b.is_urgent
            .cmp(&a.is_urgent)
            .then_with(|| a.ordered_at.cmp(&b.ordered_at))
            .then_with(|| a.order_id.cmp(&b.order_id))
    });
}

/// Filter candidates for a generate session (pure; used by tests + generate path).
pub fn filter_candidates_for_session(
    session: BatchSession,
    mut rows: Vec<BacklogOrder>,
) -> Vec<BacklogOrder> {
    if session == BatchSession::Urgent {
        rows.retain(|r| r.is_urgent);
        rows.sort_by(|a, b| {
            a.ordered_at
                .cmp(&b.ordered_at)
                .then_with(|| a.order_id.cmp(&b.order_id))
        });
    } else {
        sort_backlog_orders(&mut rows);
    }
    rows
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklogOrder {
    pub order_id: i64,
    pub platform_order_id: String,
    pub platform: String,
    pub carrier: Option<String>,
    pub buyer_shipping_carrier: Option<String>,
    pub shipment_provider: Option<String>,
    pub shipping_carrier_name: Option<String>,
    pub is_urgent: bool,
    pub ordered_at: Option<DateTime<Utc>>,
    pub item_total_num: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklogResponse {
    pub total: i64,
    pub urgent_count: i64,
    pub orders: Vec<BacklogOrder>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub id: Uuid,
    pub session: String,
    pub status: String,
    pub timezone: String,
    pub order_count: i32,
    pub urgent_count: i32,
    pub pdf_filename: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_at_wib: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchMember {
    pub order_id: i64,
    pub platform_order_id: String,
    pub platform: Option<String>,
    pub carrier_snapshot: Option<String>,
    pub is_urgent: bool,
    pub position: i32,
    pub ordered_at: Option<DateTime<Utc>>,
    pub items: Vec<BatchLineItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchLineItem {
    pub sku: Option<String>,
    pub name: Option<String>,
    /// Variant label (color/size); shown bold on SKU line — never used as product title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_attr: Option<String>,
    /// Product thumb URL for Summary List PDF (not always shown in UI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub quantity: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchDetail {
    #[serde(flatten)]
    pub summary: BatchSummary,
    pub members: Vec<BatchMember>,
}

#[derive(Debug, Clone)]
pub struct PdfOrderLine {
    pub platform_order_id: String,
    pub platform: String,
    pub carrier: String,
    pub is_urgent: bool,
    pub ordered_at_wib: String,
    pub items: Vec<BatchLineItem>,
}

pub fn wib_offset() -> FixedOffset {
    FixedOffset::east_opt(WIB_OFFSET_SECS).expect("valid WIB offset")
}

pub fn format_wib(dt: DateTime<Utc>) -> String {
    dt.with_timezone(&wib_offset())
        .format("%Y-%m-%d %H:%M:%S WIB")
        .to_string()
}

pub fn wib_day_bounds_utc(date: NaiveDate) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start_local = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| Error::Other(format!("invalid date {date}")))?
        .and_local_timezone(wib_offset())
        .single()
        .ok_or_else(|| Error::Other(format!("ambiguous WIB midnight for {date}")))?;
    let start_utc = start_local.with_timezone(&Utc);
    let end_utc = start_utc + chrono::Duration::days(1);
    Ok((start_utc, end_utc))
}

pub fn carrier_display(
    buyer_shipping_carrier: Option<&str>,
    shipment_provider: Option<&str>,
    shipping_carrier_name: Option<&str>,
) -> Option<String> {
    [
        buyer_shipping_carrier,
        shipment_provider,
        shipping_carrier_name,
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|s| !s.is_empty())
    .map(str::to_string)
}

fn row_to_backlog(row: &sqlx::postgres::PgRow) -> BacklogOrder {
    let buyer: Option<String> = row.get("buyer_shipping_carrier");
    let ship: Option<String> = row.get("shipment_provider");
    let name: Option<String> = row.get("shipping_carrier_name");
    let is_urgent = is_urgent_carrier(buyer.as_deref(), ship.as_deref(), name.as_deref());
    let carrier = carrier_display(buyer.as_deref(), ship.as_deref(), name.as_deref());
    BacklogOrder {
        order_id: row.get("id"),
        platform_order_id: row.get("platform_order_id"),
        platform: row.get("platform"),
        carrier,
        buyer_shipping_carrier: buyer,
        shipment_provider: ship,
        shipping_carrier_name: name,
        is_urgent,
        ordered_at: row.get("ordered_at"),
        item_total_num: row.get("item_total_num"),
    }
}

const BACKLOG_SQL: &str = r#"
    SELECT
        o.id, o.platform_order_id, o.platform,
        o.buyer_shipping_carrier, o.shipment_provider, o.shipping_carrier_name,
        o.ordered_at, o.item_total_num
    FROM orders o
    WHERE o.state = 'new'
      AND ($1::bigint IS NULL OR o.account_id = $1)
      AND NOT EXISTS (
          SELECT 1 FROM batch_orders bo
          WHERE bo.order_id = o.id AND bo.voided_at IS NULL
      )
    ORDER BY o.ordered_at ASC NULLS LAST, o.id ASC
    LIMIT $2
"#;

/// List backlog orders (eligible + not in active batch).
pub async fn list_backlog(
    pool: &PgPool,
    account_id: Option<i64>,
    limit: i64,
) -> Result<BacklogResponse> {
    let limit = limit.clamp(1, 5000);
    let rows = sqlx::query(BACKLOG_SQL)
        .bind(account_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    let mut orders: Vec<BacklogOrder> = rows.iter().map(row_to_backlog).collect();
    // Display: urgent first, then oldest
    sort_backlog_orders(&mut orders);

    // Full counts (not limited to page size) for ops home badges.
    let count_row = sqlx::query(
        r#"
        SELECT
            COUNT(*)::bigint AS total,
            COUNT(*) FILTER (
                WHERE
                    lower(concat_ws(' ',
                        COALESCE(o.buyer_shipping_carrier, ''),
                        COALESCE(o.shipment_provider, ''),
                        COALESCE(o.shipping_carrier_name, '')
                    )) LIKE ANY (ARRAY[
                        '%instant%', '%sameday%', '%same day%', '%same-day%',
                        '%prioritas%', '%gojek%', '%gosend%', '%grab%', '%paxel%'
                    ])
            )::bigint AS urgent_count
        FROM orders o
        WHERE o.state = 'new'
          AND ($1::bigint IS NULL OR o.account_id = $1)
          AND NOT EXISTS (
              SELECT 1 FROM batch_orders bo
              WHERE bo.order_id = o.id AND bo.voided_at IS NULL
          )
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;

    let total: i64 = count_row.get("total");
    let urgent_count: i64 = count_row.get("urgent_count");
    Ok(BacklogResponse {
        total,
        urgent_count,
        orders,
    })
}

async fn load_catalog_name_map(pool: &PgPool) -> Result<std::collections::HashMap<String, String>> {
    // Best-effort: missing table → empty map (series/mimi still work).
    let rows = sqlx::query(r#"SELECT art, name FROM product_catalog"#)
        .fetch_all(pool)
        .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "product_catalog unavailable for batch names");
            return Ok(std::collections::HashMap::new());
        }
    };
    Ok(crate::product_names::catalog_map_from_pairs(
        rows.into_iter().map(|r| {
            let art: String = r.get("art");
            let name: String = r.get("name");
            (art, name)
        }),
    ))
}

fn resolve_line_item(
    sku: Option<String>,
    raw_name: Option<String>,
    variant_attr: Option<String>,
    image_url: Option<String>,
    quantity: i32,
    catalog: &std::collections::HashMap<String, String>,
) -> BatchLineItem {
    let sku_s = sku.as_deref().unwrap_or("");
    let name = crate::product_names::resolve_display_name(sku_s, raw_name.as_deref(), catalog);
    let variant = variant_attr
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let image_url = image_url
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    BatchLineItem {
        sku,
        name: Some(name),
        variant_attr: variant,
        image_url,
        quantity,
    }
}

async fn load_items_for_orders(
    pool: &PgPool,
    order_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<BatchLineItem>>> {
    use std::collections::HashMap;
    let mut map: HashMap<i64, Vec<BatchLineItem>> = HashMap::new();
    if order_ids.is_empty() {
        return Ok(map);
    }
    let catalog = load_catalog_name_map(pool).await?;
    let rows = sqlx::query(
        r#"
        SELECT order_id, sku, variant_attr,
               COALESCE(
                   NULLIF(item_name, ''),
                   NULLIF(payload->>'itemName', ''),
                   NULLIF(payload->>'productName', ''),
                   NULLIF(payload->>'title', '')
               ) AS raw_name,
               COALESCE(
                   NULLIF(image_url, ''),
                   NULLIF(payload->>'imgUrl', ''),
                   NULLIF(payload->>'image', ''),
                   NULLIF(payload->>'cosImage', '')
               ) AS image_url,
               quantity
        FROM order_items
        WHERE order_id = ANY($1)
        ORDER BY order_id, line_no ASC
        "#,
    )
    .bind(order_ids)
    .fetch_all(pool)
    .await?;

    for row in rows {
        let oid: i64 = row.get("order_id");
        map.entry(oid).or_default().push(resolve_line_item(
            row.get("sku"),
            row.get("raw_name"),
            row.get("variant_attr"),
            row.get("image_url"),
            row.get("quantity"),
            &catalog,
        ));
    }
    Ok(map)
}

/// Create a batch: lock candidates, insert members, store PDF bytes.
pub async fn create_batch(
    pool: &PgPool,
    session: BatchSession,
    account_id: Option<i64>,
) -> Result<BatchDetail> {
    let mut tx = pool.begin().await?;

    // Lock eligible order rows so concurrent generates cannot double-assign.
    let lock_rows = sqlx::query(
        r#"
        SELECT
            o.id, o.platform_order_id, o.platform,
            o.buyer_shipping_carrier, o.shipment_provider, o.shipping_carrier_name,
            o.ordered_at, o.item_total_num
        FROM orders o
        WHERE o.state = 'new'
          AND ($1::bigint IS NULL OR o.account_id = $1)
          AND NOT EXISTS (
              SELECT 1 FROM batch_orders bo
              WHERE bo.order_id = o.id AND bo.voided_at IS NULL
          )
        ORDER BY o.ordered_at ASC NULLS LAST, o.id ASC
        FOR UPDATE OF o SKIP LOCKED
        "#,
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;

    let candidates: Vec<BacklogOrder> = lock_rows.iter().map(row_to_backlog).collect();
    let selected = filter_candidates_for_session(session, candidates);

    if selected.is_empty() {
        return Err(Error::Other(
            "no eligible orders in backlog for session".into(),
        ));
    }

    let batch_id = Uuid::new_v4();
    let now = Utc::now();
    let order_ids: Vec<i64> = selected.iter().map(|o| o.order_id).collect();
    let catalog = {
        let rows = sqlx::query(r#"SELECT art, name FROM product_catalog"#)
            .fetch_all(&mut *tx)
            .await;
        match rows {
            Ok(rows) => crate::product_names::catalog_map_from_pairs(rows.into_iter().map(|r| {
                let art: String = r.get("art");
                let name: String = r.get("name");
                (art, name)
            })),
            Err(e) => {
                tracing::warn!(error = %e, "product_catalog unavailable during create_batch");
                std::collections::HashMap::new()
            }
        }
    };

    let items_map = {
        let rows = sqlx::query(
            r#"
            SELECT order_id, sku, variant_attr,
                   COALESCE(
                       NULLIF(item_name, ''),
                       NULLIF(payload->>'itemName', ''),
                       NULLIF(payload->>'productName', ''),
                       NULLIF(payload->>'title', '')
                   ) AS raw_name,
                   COALESCE(
                       NULLIF(image_url, ''),
                       NULLIF(payload->>'imgUrl', ''),
                       NULLIF(payload->>'image', ''),
                       NULLIF(payload->>'cosImage', '')
                   ) AS image_url,
                   quantity
            FROM order_items
            WHERE order_id = ANY($1)
            ORDER BY order_id, line_no ASC
            "#,
        )
        .bind(&order_ids)
        .fetch_all(&mut *tx)
        .await?;
        let mut map: std::collections::HashMap<i64, Vec<BatchLineItem>> =
            std::collections::HashMap::new();
        for row in rows {
            let oid: i64 = row.get("order_id");
            map.entry(oid).or_default().push(resolve_line_item(
                row.get("sku"),
                row.get("raw_name"),
                row.get("variant_attr"),
                row.get("image_url"),
                row.get("quantity"),
                &catalog,
            ));
        }
        map
    };

    let mut members: Vec<BatchMember> = Vec::with_capacity(selected.len());
    let mut pdf_lines: Vec<PdfOrderLine> = Vec::with_capacity(selected.len());
    let mut urgent_count = 0i32;

    for (idx, o) in selected.iter().enumerate() {
        if o.is_urgent {
            urgent_count += 1;
        }
        let items = items_map.get(&o.order_id).cloned().unwrap_or_default();
        let carrier_snap = o.carrier.clone();
        members.push(BatchMember {
            order_id: o.order_id,
            platform_order_id: o.platform_order_id.clone(),
            platform: Some(o.platform.clone()),
            carrier_snapshot: carrier_snap.clone(),
            is_urgent: o.is_urgent,
            position: idx as i32,
            ordered_at: o.ordered_at,
            items: items.clone(),
        });
        pdf_lines.push(PdfOrderLine {
            platform_order_id: o.platform_order_id.clone(),
            platform: o.platform.clone(),
            carrier: carrier_snap.unwrap_or_else(|| "-".into()),
            is_urgent: o.is_urgent,
            ordered_at_wib: o.ordered_at.map(format_wib).unwrap_or_else(|| "-".into()),
            items,
        });
    }

    let order_count = members.len() as i32;
    let short_id: String = batch_id.simple().to_string().chars().take(8).collect();
    let wib_stamp = now
        .with_timezone(&wib_offset())
        .format("%Y%m%d-%H%M")
        .to_string();
    let pdf_filename = format!("batch-{}-{}-{}.pdf", session.as_str(), wib_stamp, short_id);

    // Membership first (short TX). PDF with thumbs is built after commit so
    // image HTTP does not hold row locks.
    sqlx::query(
        r#"
        INSERT INTO batches (
            id, account_id, session, timezone, status,
            order_count, urgent_count, pdf_bytes, pdf_filename, created_at
        ) VALUES (
            $1, $2, $3, $4, 'ready',
            $5, $6, NULL, $7, $8
        )
        "#,
    )
    .bind(batch_id)
    .bind(account_id)
    .bind(session.as_str())
    .bind(BATCH_TIMEZONE)
    .bind(order_count)
    .bind(urgent_count)
    .bind(&pdf_filename)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    for m in &members {
        sqlx::query(
            r#"
            INSERT INTO batch_orders (
                batch_id, order_id, platform_order_id, platform,
                carrier_snapshot, is_urgent, position, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(batch_id)
        .bind(m.order_id)
        .bind(&m.platform_order_id)
        .bind(&m.platform)
        .bind(&m.carrier_snapshot)
        .bind(m.is_urgent)
        .bind(m.position)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    let pdf_bytes = crate::batch_pdf::render_batch_pdf(
        batch_id,
        session,
        &format_wib(now),
        order_count,
        urgent_count,
        &pdf_lines,
    )
    .await?;

    if let Err(e) =
        sqlx::query(r#"UPDATE batches SET pdf_bytes = $2 WHERE id = $1 AND status = 'ready'"#)
            .bind(batch_id)
            .bind(&pdf_bytes)
            .execute(pool)
            .await
    {
        tracing::error!(error = %e, %batch_id, "failed to store batch pdf after membership commit");
        return Err(e.into());
    }

    Ok(BatchDetail {
        summary: BatchSummary {
            id: batch_id,
            session: session.as_str().into(),
            status: "ready".into(),
            timezone: BATCH_TIMEZONE.into(),
            order_count,
            urgent_count,
            pdf_filename: Some(pdf_filename),
            created_at: now,
            created_at_wib: format_wib(now),
        },
        members,
    })
}

pub async fn list_batches_for_wib_date(
    pool: &PgPool,
    date: NaiveDate,
    account_id: Option<i64>,
) -> Result<Vec<BatchSummary>> {
    let (start, end) = wib_day_bounds_utc(date)?;
    let rows = sqlx::query(
        r#"
        SELECT id, session, status, timezone, order_count, urgent_count,
               pdf_filename, created_at
        FROM batches
        WHERE created_at >= $1 AND created_at < $2
          AND ($3::bigint IS NULL OR account_id = $3 OR account_id IS NULL)
        ORDER BY created_at DESC
        "#,
    )
    .bind(start)
    .bind(end)
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let created_at: DateTime<Utc> = row.get("created_at");
            BatchSummary {
                id: row.get("id"),
                session: row.get("session"),
                status: row.get("status"),
                timezone: row.get("timezone"),
                order_count: row.get("order_count"),
                urgent_count: row.get("urgent_count"),
                pdf_filename: row.get("pdf_filename"),
                created_at,
                created_at_wib: format_wib(created_at),
            }
        })
        .collect())
}

pub async fn get_batch(pool: &PgPool, id: Uuid) -> Result<Option<BatchDetail>> {
    let row = sqlx::query(
        r#"
        SELECT id, session, status, timezone, order_count, urgent_count,
               pdf_filename, created_at
        FROM batches WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let created_at: DateTime<Utc> = row.get("created_at");
    let summary = BatchSummary {
        id: row.get("id"),
        session: row.get("session"),
        status: row.get("status"),
        timezone: row.get("timezone"),
        order_count: row.get("order_count"),
        urgent_count: row.get("urgent_count"),
        pdf_filename: row.get("pdf_filename"),
        created_at,
        created_at_wib: format_wib(created_at),
    };

    let mrows = sqlx::query(
        r#"
        SELECT bo.order_id, bo.platform_order_id, bo.platform, bo.carrier_snapshot,
               bo.is_urgent, bo.position, o.ordered_at
        FROM batch_orders bo
        LEFT JOIN orders o ON o.id = bo.order_id
        WHERE bo.batch_id = $1 AND bo.voided_at IS NULL
        ORDER BY bo.position ASC
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let order_ids: Vec<i64> = mrows.iter().map(|r| r.get::<i64, _>("order_id")).collect();
    let items_map = load_items_for_orders(pool, &order_ids).await?;

    let members = mrows
        .into_iter()
        .map(|r| {
            let oid: i64 = r.get("order_id");
            BatchMember {
                order_id: oid,
                platform_order_id: r.get("platform_order_id"),
                platform: r.get("platform"),
                carrier_snapshot: r.get("carrier_snapshot"),
                is_urgent: r.get("is_urgent"),
                position: r.get("position"),
                ordered_at: r.get("ordered_at"),
                items: items_map.get(&oid).cloned().unwrap_or_default(),
            }
        })
        .collect();

    Ok(Some(BatchDetail { summary, members }))
}

pub async fn get_batch_pdf(pool: &PgPool, id: Uuid) -> Result<Option<(String, Vec<u8>)>> {
    let row = sqlx::query(
        r#"
        SELECT pdf_filename, pdf_bytes
        FROM batches WHERE id = $1 AND status = 'ready'
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let bytes: Option<Vec<u8>> = row.get("pdf_bytes");
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let filename: String = row
        .get::<Option<String>, _>("pdf_filename")
        .unwrap_or_else(|| format!("batch-{id}.pdf"));
    Ok(Some((filename, bytes)))
}

/// Rebuild PDF bytes for an existing ready batch (same members; new Summary List layout).
/// Does not change membership. Use when renderer improved after batch was created.
pub async fn regenerate_batch_pdf(pool: &PgPool, id: Uuid) -> Result<BatchDetail> {
    let detail = get_batch(pool, id)
        .await?
        .ok_or_else(|| Error::Other(format!("batch {id} not found")))?;
    if detail.summary.status != "ready" {
        return Err(Error::Other(format!(
            "batch {id} status is {}, expected ready",
            detail.summary.status
        )));
    }

    let session = BatchSession::parse(&detail.summary.session).ok_or_else(|| {
        Error::Other(format!(
            "invalid session on batch: {}",
            detail.summary.session
        ))
    })?;

    let pdf_lines: Vec<PdfOrderLine> = detail
        .members
        .iter()
        .map(|m| PdfOrderLine {
            platform_order_id: m.platform_order_id.clone(),
            platform: m.platform.clone().unwrap_or_else(|| "-".into()),
            carrier: m.carrier_snapshot.clone().unwrap_or_else(|| "-".into()),
            is_urgent: m.is_urgent,
            ordered_at_wib: m.ordered_at.map(format_wib).unwrap_or_else(|| "-".into()),
            items: m.items.clone(),
        })
        .collect();

    let pdf_bytes = crate::batch_pdf::render_batch_pdf(
        detail.summary.id,
        session,
        &detail.summary.created_at_wib,
        detail.summary.order_count,
        detail.summary.urgent_count,
        &pdf_lines,
    )
    .await?;

    let short_id: String = id.simple().to_string().chars().take(8).collect();
    let wib_stamp = detail
        .summary
        .created_at
        .with_timezone(&wib_offset())
        .format("%Y%m%d-%H%M")
        .to_string();
    let pdf_filename = format!("batch-{}-{}-{}.pdf", session.as_str(), wib_stamp, short_id);

    sqlx::query(
        r#"
        UPDATE batches
        SET pdf_bytes = $2, pdf_filename = $3
        WHERE id = $1 AND status = 'ready'
        "#,
    )
    .bind(id)
    .bind(&pdf_bytes)
    .bind(&pdf_filename)
    .execute(pool)
    .await?;

    get_batch(pool, id)
        .await?
        .ok_or_else(|| Error::Other(format!("batch {id} missing after regenerate")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bo(id: i64, urgent: bool, ordered: Option<DateTime<Utc>>) -> BacklogOrder {
        BacklogOrder {
            order_id: id,
            platform_order_id: format!("PO{id}"),
            platform: "shopee".into(),
            carrier: None,
            buyer_shipping_carrier: None,
            shipment_provider: None,
            shipping_carrier_name: None,
            is_urgent: urgent,
            ordered_at: ordered,
            item_total_num: Some(1),
        }
    }

    #[test]
    fn urgent_keywords_match_real_carrier_strings() {
        assert!(is_urgent_carrier(Some("SPX Instant"), None, None));
        assert!(is_urgent_carrier(None, Some("GoSend Same Day"), None));
        assert!(is_urgent_carrier(None, None, Some("GrabExpress")));
        assert!(is_urgent_carrier(Some("JNE REG"), Some("gosend"), None));
        assert!(is_urgent_carrier(Some("Prioritas"), None, None));
        assert!(is_urgent_carrier(Some("paxel same-day"), None, None));
        assert!(is_urgent_carrier(Some("SAME DAY"), None, None));
        assert!(is_urgent_carrier(Some("sameday"), None, None));
        assert!(is_urgent_carrier(Some("gojek"), None, None));
        assert!(!is_urgent_carrier(
            Some("JNE REG"),
            Some("SiCepat REG"),
            None
        ));
        assert!(!is_urgent_carrier(None, None, None));
        assert!(!is_urgent_carrier(Some(""), Some("  "), None));
    }

    #[test]
    fn urgent_session_keeps_only_urgent() {
        let t0 = Utc::now() - chrono::Duration::hours(2);
        let t1 = Utc::now() - chrono::Duration::hours(1);
        let rows = vec![
            bo(1, false, Some(t0)),
            bo(2, true, Some(t1)),
            bo(3, true, Some(t0)),
        ];
        let out = filter_candidates_for_session(BatchSession::Urgent, rows);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|r| r.is_urgent));
        // oldest first among urgent
        assert_eq!(out[0].order_id, 3);
        assert_eq!(out[1].order_id, 2);
    }

    #[test]
    fn morning_puts_urgent_first_then_oldest() {
        let t0 = Utc::now() - chrono::Duration::hours(3);
        let t1 = Utc::now() - chrono::Duration::hours(2);
        let t2 = Utc::now() - chrono::Duration::hours(1);
        let rows = vec![
            bo(1, false, Some(t1)),
            bo(2, true, Some(t2)),
            bo(3, false, Some(t0)),
            bo(4, true, Some(t0)),
        ];
        let out = filter_candidates_for_session(BatchSession::Morning, rows);
        assert_eq!(
            out.iter().map(|r| r.order_id).collect::<Vec<_>>(),
            vec![4, 2, 3, 1]
        );
    }

    #[test]
    fn afternoon_same_ordering_as_morning() {
        let t0 = Utc::now() - chrono::Duration::hours(2);
        let t1 = Utc::now() - chrono::Duration::hours(1);
        let rows = vec![bo(1, false, Some(t1)), bo(2, true, Some(t0))];
        let out = filter_candidates_for_session(BatchSession::Afternoon, rows);
        assert_eq!(out[0].order_id, 2);
        assert_eq!(out[1].order_id, 1);
    }

    #[test]
    fn session_parse_roundtrip() {
        assert_eq!(BatchSession::parse("Morning"), Some(BatchSession::Morning));
        assert_eq!(BatchSession::parse("URGENT"), Some(BatchSession::Urgent));
        assert_eq!(BatchSession::parse("nope"), None);
        assert_eq!(BatchSession::Morning.as_str(), "morning");
    }

    #[test]
    fn wib_day_bounds_are_seventeen_hours_utc_offset() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        let (start, end) = wib_day_bounds_utc(d).unwrap();
        // 2026-07-22 00:00 WIB = 2026-07-21 17:00 UTC
        assert_eq!(start.to_rfc3339(), "2026-07-21T17:00:00+00:00");
        assert_eq!((end - start).num_hours(), 24);
    }
}
