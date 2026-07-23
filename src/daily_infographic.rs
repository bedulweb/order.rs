//! One-card daily rekap PNG for WA group (sore hari-H).
//!
//! Active metrics exclude canceled orders. Comparisons use yesterday's same
//! local clock window: midnight WIB → as-of, and midnight WIB yesterday →
//! as-of minus one day.

use crate::carrier_day_list::{default_logos_dir, logo_data_uri};
use crate::daily_report::{day_bounds_utc, svg_to_png, write_png};
use crate::error::Result;
use crate::product_names::{self, normalize_art};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CARD_W: u32 = 1080;
const PAD: f64 = 36.0;
const TZ_WIB: i32 = 7;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopProduct {
    pub name: String,
    pub qty: i64,
    pub gmv: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarrierStats {
    pub instant: i64,
    pub spx: i64,
    pub jnt: i64,
    pub jne: i64,
    pub sicepat: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformStats {
    pub shopee_n: i64,
    pub shopee_gmv: f64,
    pub tiktok_n: i64,
    pub tiktok_gmv: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStats {
    pub gmv: f64,
    pub order_count: i64,
    pub aov: f64,
    pub qty: i64,
    pub platform: PlatformStats,
    pub carriers: CarrierStats,
    pub fee_est: f64,
    pub ship_est: f64,
    pub cancel_n: i64,
    pub cancel_gmv: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyInfographic {
    pub date: NaiveDate,
    pub as_of: DateTime<Utc>,
    pub tz_offset_hours: i32,
    pub current: WindowStats,
    pub previous: WindowStats,
    pub top: Vec<TopProduct>,
    pub hpp: f64,
    pub gross: f64,
    pub hpp_match_pct: f64,
}

fn f64_cell(row: &sqlx::postgres::PgRow, col: &str) -> f64 {
    row.try_get::<Option<f64>, _>(col)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<String>, _>(col)
                .ok()
                .flatten()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0.0)
}

fn i64_cell(row: &sqlx::postgres::PgRow, col: &str) -> i64 {
    row.try_get::<i64, _>(col)
        .or_else(|_| row.try_get::<Option<i64>, _>(col).map(|o| o.unwrap_or(0)))
        .unwrap_or(0)
}

fn json_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn json_obj_num(obj: &Value, key: &str) -> f64 {
    obj.get(key).and_then(json_num).unwrap_or(0.0)
}

fn platform_fee_est(fee_detail: &Value) -> f64 {
    [
        "commissionFee",
        "serviceFee",
        "orderProcessFee",
        "sellerOrderProcessingFee",
        "newServiceFee",
    ]
    .iter()
    .filter_map(|key| fee_detail.get(*key).and_then(json_num))
    .filter(|v| *v > 0.0)
    .sum()
}

fn shipping_pref(fee_detail: &Value) -> f64 {
    let actual = fee_detail
        .get("otherFeeInfo")
        .map(|other| json_obj_num(other, "actualShippingFee"))
        .unwrap_or(0.0);
    if actual > 0.0 {
        return actual;
    }

    let estimated = json_obj_num(fee_detail, "estimatedShippingFee");
    if estimated > 0.0 {
        estimated
    } else {
        0.0
    }
}

fn pct(current: f64, previous: f64) -> Option<f64> {
    if previous.abs() < f64::EPSILON {
        None
    } else {
        Some((current - previous) / previous * 100.0)
    }
}

fn pct_i64(current: i64, previous: i64) -> Option<f64> {
    pct(current as f64, previous as f64)
}

fn fmt_rp(x: f64) -> String {
    let n = x.round() as i64;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    let body: String = out.chars().rev().collect();
    if n < 0 {
        format!("-Rp {body}")
    } else {
        format!("Rp {body}")
    }
}

fn fmt_pct(p: Option<f64>) -> String {
    match p {
        None => "—".into(),
        Some(v) if v >= 0.0 => format!("▲ {:.0}%", v.abs()),
        Some(v) => format!("▼ {:.0}%", v.abs()),
    }
}

fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            c if c.is_control() => String::new(),
            c => c.to_string(),
        })
        .collect()
}

fn trunc(s: &str, max: usize) -> String {
    let t: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}

fn day_name_id(d: NaiveDate) -> &'static str {
    match d.weekday().number_from_monday() {
        1 => "Senin",
        2 => "Selasa",
        3 => "Rabu",
        4 => "Kamis",
        5 => "Jumat",
        6 => "Sabtu",
        _ => "Minggu",
    }
}

fn month_name_id(m: u32) -> &'static str {
    match m {
        1 => "Januari",
        2 => "Februari",
        3 => "Maret",
        4 => "April",
        5 => "Mei",
        6 => "Juni",
        7 => "Juli",
        8 => "Agustus",
        9 => "September",
        10 => "Oktober",
        11 => "November",
        _ => "Desember",
    }
}

const CARRIER_SQL: &str = r#"
CASE
  WHEN hay ~ 'instant' OR hay ~ 'sameday' OR hay ~ 'same day' OR hay ~ 'same-day'
    OR hay ~ 'prioritas' OR hay ~ 'gojek' OR hay ~ 'gosend' OR hay ~ 'grab' OR hay ~ 'paxel'
  THEN 'instant'
  WHEN hay ~ 'spx' OR hay ~ 'shopee express' OR hay ~ 'shopee xpress' OR hay ~ 'shopee-xpress'
  THEN 'spx'
  WHEN hay ~ 'j&t' OR hay ~ 'jnt' OR hay ~ 'j-t' OR hay ~ 'jet express' THEN 'jnt'
  WHEN hay ~ 'jne' THEN 'jne'
  WHEN hay ~ 'sicepat' OR hay ~ 'si cepat' OR hay ~ 'si-cepat' THEN 'sicepat'
  ELSE 'other'
END
"#;

async fn load_window_stats(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<WindowStats> {
    let sql = format!(
        r#"
        WITH base AS (
          SELECT o.id, o.platform, o.state, o.amount::float8 AS amount,
            lower(coalesce(o.buyer_shipping_carrier,'')||' '||coalesce(o.shipment_provider,'')||' '||coalesce(o.shipping_carrier_name,'')) AS hay
          FROM orders o
          WHERE coalesce(o.ordered_at, o.first_seen_at) >= $1
            AND coalesce(o.ordered_at, o.first_seen_at) < $2
        ),
        active AS (
          SELECT * FROM base WHERE lower(coalesce(state,'')) NOT IN ('canceled','cancelled')
        ),
        tagged AS (
          SELECT *, {CARRIER_SQL} AS bucket FROM active
        )
        SELECT
          coalesce(sum(amount),0)::float8 AS gmv,
          count(*)::bigint AS n,
          coalesce((SELECT sum(oi.quantity)::bigint FROM order_items oi WHERE oi.order_id IN (SELECT id FROM active)),0)::bigint AS qty,
          coalesce(sum(amount) FILTER (WHERE lower(coalesce(platform,''))='shopee'),0)::float8 AS shopee_gmv,
          count(*) FILTER (WHERE lower(coalesce(platform,''))='shopee')::bigint AS shopee_n,
          coalesce(sum(amount) FILTER (WHERE lower(coalesce(platform,''))='tiktok'),0)::float8 AS tiktok_gmv,
          count(*) FILTER (WHERE lower(coalesce(platform,''))='tiktok')::bigint AS tiktok_n,
          count(*) FILTER (WHERE bucket='instant')::bigint AS instant,
          count(*) FILTER (WHERE bucket='spx')::bigint AS spx,
          count(*) FILTER (WHERE bucket='jnt')::bigint AS jnt,
          count(*) FILTER (WHERE bucket='jne')::bigint AS jne,
          count(*) FILTER (WHERE bucket='sicepat')::bigint AS sicepat,
          (SELECT count(*)::bigint FROM base WHERE lower(coalesce(state,'')) IN ('canceled','cancelled')) AS cancel_n,
          coalesce((SELECT sum(amount)::float8 FROM base WHERE lower(coalesce(state,'')) IN ('canceled','cancelled')),0)::float8 AS cancel_gmv
        FROM tagged
        "#
    );
    let row = sqlx::query(&sql)
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await?;
    let gmv = f64_cell(&row, "gmv");
    let order_count = i64_cell(&row, "n");
    let aov = if order_count > 0 {
        gmv / order_count as f64
    } else {
        0.0
    };

    let fee_rows = sqlx::query(
        r#"
        SELECT payload->'feeDetail' AS fee
        FROM orders
        WHERE coalesce(ordered_at, first_seen_at) >= $1
          AND coalesce(ordered_at, first_seen_at) < $2
          AND lower(coalesce(state,'')) NOT IN ('canceled','cancelled')
        "#,
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;

    let mut fee_est = 0.0;
    let mut ship_est = 0.0;
    for row in fee_rows {
        let fee_detail: Option<Value> = row.try_get("fee").ok().flatten();
        let Some(fee_detail) = fee_detail else {
            continue;
        };
        fee_est += platform_fee_est(&fee_detail);
        ship_est += shipping_pref(&fee_detail);
    }

    Ok(WindowStats {
        gmv,
        order_count,
        aov,
        qty: i64_cell(&row, "qty"),
        platform: PlatformStats {
            shopee_n: i64_cell(&row, "shopee_n"),
            shopee_gmv: f64_cell(&row, "shopee_gmv"),
            tiktok_n: i64_cell(&row, "tiktok_n"),
            tiktok_gmv: f64_cell(&row, "tiktok_gmv"),
        },
        carriers: CarrierStats {
            instant: i64_cell(&row, "instant"),
            spx: i64_cell(&row, "spx"),
            jnt: i64_cell(&row, "jnt"),
            jne: i64_cell(&row, "jne"),
            sicepat: i64_cell(&row, "sicepat"),
        },
        fee_est,
        ship_est,
        cancel_n: i64_cell(&row, "cancel_n"),
        cancel_gmv: f64_cell(&row, "cancel_gmv"),
    })
}

async fn load_catalog(pool: &PgPool) -> Result<(HashMap<String, String>, HashMap<String, i64>)> {
    let rows = sqlx::query(r#"SELECT art, name, hpp FROM product_catalog"#)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    let mut names = HashMap::new();
    let mut hpp = HashMap::new();
    for row in rows {
        let art: String = row.get("art");
        let art = normalize_art(&art);
        if art.is_empty() {
            continue;
        }
        let name: String = row.get("name");
        if !name.trim().is_empty() {
            names.insert(art.clone(), name.trim().to_string());
        }
        hpp.insert(art, row.get::<i64, _>("hpp"));
    }

    Ok((names, hpp))
}

fn catalog_hpp_for_sku(sku: &str, hpp_by_art: &HashMap<String, i64>) -> Option<i64> {
    let sku_n = normalize_art(sku);
    if sku_n.is_empty() {
        return None;
    }

    let probes = [sku_n.clone(), product_names::strip_color_segment(&sku_n)];
    let mut best: Option<(usize, i64)> = None;
    for probe in probes {
        for (art, hpp) in hpp_by_art {
            if probe == *art || probe.starts_with(&format!("{art}-")) {
                let len = art.len();
                if best.map(|(best_len, _)| len > best_len).unwrap_or(true) {
                    best = Some((len, *hpp));
                }
            }
        }
    }
    best.map(|(_, hpp)| hpp)
}

async fn load_products_and_hpp(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    gmv: f64,
) -> Result<(Vec<TopProduct>, f64, f64, f64)> {
    let (catalog_names, hpp_by_art) = load_catalog(pool).await?;
    let rows = sqlx::query(
        r#"
        SELECT
          coalesce(nullif(trim(oi.sku), ''), '') AS sku,
          nullif(trim(oi.item_name), '') AS item_name,
          coalesce(sum(oi.quantity), 0)::bigint AS qty,
          coalesce(sum(oi.amount)::float8, 0)::float8 AS gmv
        FROM order_items oi
        JOIN orders o ON o.id = oi.order_id
        WHERE coalesce(o.ordered_at, o.first_seen_at) >= $1
          AND coalesce(o.ordered_at, o.first_seen_at) < $2
          AND lower(coalesce(o.state,'')) NOT IN ('canceled','cancelled')
        GROUP BY 1, 2
        "#,
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;

    let mut by_name: HashMap<String, TopProduct> = HashMap::new();
    let mut hpp_total = 0.0;
    let mut qty_total = 0i64;
    let mut qty_matched = 0i64;

    for row in rows {
        let sku: String = row.get("sku");
        let item_name: Option<String> = row.get("item_name");
        let qty = i64_cell(&row, "qty");
        let line_gmv = f64_cell(&row, "gmv");
        qty_total += qty;

        if let Some(hpp) = catalog_hpp_for_sku(&sku, &hpp_by_art) {
            hpp_total += qty as f64 * hpp as f64;
            qty_matched += qty;
        }

        let name = product_names::resolve_display_name(&sku, item_name.as_deref(), &catalog_names);
        let entry = by_name.entry(name.clone()).or_insert_with(|| TopProduct {
            name,
            qty: 0,
            gmv: 0.0,
        });
        entry.qty += qty;
        entry.gmv += line_gmv;
    }

    let mut top: Vec<TopProduct> = by_name.into_values().collect();
    top.sort_by(|a, b| b.qty.cmp(&a.qty).then_with(|| b.gmv.total_cmp(&a.gmv)));
    top.truncate(5);

    let match_pct = if qty_total > 0 {
        qty_matched as f64 / qty_total as f64 * 100.0
    } else {
        0.0
    };

    Ok((top, hpp_total, gmv - hpp_total, match_pct))
}

/// Load the infographic model.
///
/// `as_of=None` means current time. `tz_offset_hours=None` means WIB/UTC+7.
pub async fn load_daily_infographic(
    pool: &PgPool,
    as_of: Option<DateTime<Utc>>,
    tz_offset_hours: Option<i32>,
) -> Result<DailyInfographic> {
    let as_of = as_of.unwrap_or_else(Utc::now);
    let tz_offset_hours = tz_offset_hours.unwrap_or(TZ_WIB);
    let local_as_of = as_of + Duration::hours(tz_offset_hours as i64);
    let date = local_as_of.date_naive();
    let (start, _end_of_day) = day_bounds_utc(date, tz_offset_hours)?;
    let end = as_of;
    let prev_start = start - Duration::days(1);
    let prev_end = end - Duration::days(1);

    let current = load_window_stats(pool, start, end).await?;
    let previous = load_window_stats(pool, prev_start, prev_end).await?;
    let (top, hpp, gross, hpp_match_pct) =
        load_products_and_hpp(pool, start, end, current.gmv).await?;

    Ok(DailyInfographic {
        date,
        as_of,
        tz_offset_hours,
        current,
        previous,
        top,
        hpp,
        gross,
        hpp_match_pct,
    })
}

fn carrier_logo(logos_dir: &Path, filename: &str) -> Option<String> {
    logo_data_uri(&logos_dir.join(filename)).ok()
}

fn carrier_row(
    y: f64,
    label: &str,
    logo_uri: Option<&str>,
    count: i64,
    prev: i64,
    accent: &str,
) -> String {
    let icon = if let Some(uri) = logo_uri {
        format!(
            r##"<image href="{uri}" x="72" y="{iy}" width="52" height="52" preserveAspectRatio="xMidYMid meet"/>"##,
            iy = y + 10.0
        )
    } else {
        format!(
            r##"<circle cx="98" cy="{cy}" r="20" fill="{accent}" opacity="0.14"/>"##,
            cy = y + 36.0
        )
    };

    format!(
        r##"
  <rect x="56" y="{y}" width="456" height="72" rx="18" fill="#FFFFFF" stroke="#E5E7EB"/>
  {icon}
  <text x="144" y="{ty1}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="20" font-weight="700" fill="#111827">{label}</text>
  <text x="144" y="{ty2}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="15" fill="#6B7280">vs kemarin {pct}</text>
  <text x="482" y="{ty1}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="30" font-weight="800" fill="{accent}">{count}</text>
"##,
        y = y,
        icon = icon,
        ty1 = y + 32.0,
        ty2 = y + 56.0,
        label = esc(label),
        pct = esc(&fmt_pct(pct_i64(count, prev))),
        accent = accent,
        count = count,
    )
}

fn stat_box(x: f64, y: f64, title: &str, value: &str, delta: &str, accent: &str) -> String {
    format!(
        r##"
  <rect x="{x}" y="{y}" width="234" height="126" rx="22" fill="#FFFFFF" stroke="#E5E7EB"/>
  <text x="{tx}" y="{t1}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" font-weight="700" fill="#6B7280">{title}</text>
  <text x="{tx}" y="{t2}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="28" font-weight="800" fill="{accent}">{value}</text>
  <text x="{tx}" y="{t3}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#6B7280">vs same-hour {delta}</text>
"##,
        x = x,
        y = y,
        tx = x + 22.0,
        t1 = y + 34.0,
        t2 = y + 76.0,
        t3 = y + 104.0,
        title = esc(title),
        value = esc(value),
        delta = esc(delta),
        accent = accent,
    )
}

/// Build an SVG using the existing daily report visual language: single card,
/// teal header, white stat tiles, courier rows, product list, internal estimate
/// footer, and red cancel callout.
pub fn to_svg(report: &DailyInfographic) -> Result<String> {
    let w = CARD_W as f64;
    let h = 1560u32;
    let cx = w / 2.0;
    let inner_w = w - 2.0 * PAD;
    let local_as_of = report.as_of + Duration::hours(report.tz_offset_hours as i64);
    let date_label = format!(
        "{}, {} {} {}",
        day_name_id(report.date),
        report.date.day(),
        month_name_id(report.date.month()),
        report.date.year()
    );
    let as_of_label = format!(
        "s.d. {:02}:{:02} WIB · vs kemarin jam sama",
        local_as_of.hour(),
        local_as_of.minute()
    );

    let c = &report.current;
    let p = &report.previous;
    let logos = default_logos_dir();
    let gosend = carrier_logo(&logos, "gosend.png");
    let spx = carrier_logo(&logos, "spx-express.png");
    let jnt = carrier_logo(&logos, "j-t-express.png");
    let jne = carrier_logo(&logos, "jne.png");
    let sicepat = carrier_logo(&logos, "sicepat.png");

    let stat_y = 230.0;
    let stats = [
        stat_box(
            56.0,
            stat_y,
            "OMSET",
            &fmt_rp(c.gmv),
            &fmt_pct(pct(c.gmv, p.gmv)),
            "#0F766E",
        ),
        stat_box(
            306.0,
            stat_y,
            "ORDER",
            &c.order_count.to_string(),
            &fmt_pct(pct_i64(c.order_count, p.order_count)),
            "#134E4A",
        ),
        stat_box(
            556.0,
            stat_y,
            "AOV",
            &fmt_rp(c.aov),
            &fmt_pct(pct(c.aov, p.aov)),
            "#0F766E",
        ),
        stat_box(
            806.0,
            stat_y,
            "QTY",
            &c.qty.to_string(),
            &fmt_pct(pct_i64(c.qty, p.qty)),
            "#134E4A",
        ),
    ]
    .join("");

    let mut courier_rows = String::new();
    courier_rows.push_str(&carrier_row(
        550.0,
        "Instant",
        gosend.as_deref(),
        c.carriers.instant,
        p.carriers.instant,
        "#B45309",
    ));
    courier_rows.push_str(&carrier_row(
        636.0,
        "SPX",
        spx.as_deref(),
        c.carriers.spx,
        p.carriers.spx,
        "#6D28D9",
    ));
    courier_rows.push_str(&carrier_row(
        722.0,
        "J&T",
        jnt.as_deref(),
        c.carriers.jnt,
        p.carriers.jnt,
        "#C2410C",
    ));
    courier_rows.push_str(&carrier_row(
        808.0,
        "JNE",
        jne.as_deref(),
        c.carriers.jne,
        p.carriers.jne,
        "#1D4ED8",
    ));
    courier_rows.push_str(&carrier_row(
        894.0,
        "SiCepat",
        sicepat.as_deref(),
        c.carriers.sicepat,
        p.carriers.sicepat,
        "#047857",
    ));

    let platform_chips = format!(
        r##"
  <rect x="56" y="398" width="470" height="104" rx="22" fill="#FFF7ED" stroke="#FED7AA"/>
  <text x="82" y="432" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" font-weight="800" fill="#9A3412">SHOPEE</text>
  <text x="82" y="470" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="28" font-weight="800" fill="#C2410C">{shopee_n} · {shopee_gmv}</text>
  <text x="82" y="492" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#9A3412">{shopee_pct} vs kemarin</text>

  <rect x="554" y="398" width="470" height="104" rx="22" fill="#F8FAFC" stroke="#CBD5E1"/>
  <text x="580" y="432" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" font-weight="800" fill="#334155">TIKTOK</text>
  <text x="580" y="470" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="28" font-weight="800" fill="#0F172A">{tiktok_n} · {tiktok_gmv}</text>
  <text x="580" y="492" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#64748B">{tiktok_pct} vs kemarin</text>
"##,
        shopee_n = c.platform.shopee_n,
        shopee_gmv = esc(&fmt_rp(c.platform.shopee_gmv)),
        shopee_pct = esc(&fmt_pct(pct(c.platform.shopee_gmv, p.platform.shopee_gmv))),
        tiktok_n = c.platform.tiktok_n,
        tiktok_gmv = esc(&fmt_rp(c.platform.tiktok_gmv)),
        tiktok_pct = esc(&fmt_pct(pct(c.platform.tiktok_gmv, p.platform.tiktok_gmv))),
    );

    let mut top_rows = String::new();
    for (idx, item) in report.top.iter().enumerate() {
        let y = 586.0 + idx as f64 * 78.0;
        top_rows.push_str(&format!(
            r##"
  <text x="582" y="{ty}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="19" font-weight="700" fill="#111827">{rank}. {name}</text>
  <text x="982" y="{ty}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" font-weight="800" fill="#0F766E">×{qty}</text>
  <text x="582" y="{sy}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#6B7280">{gmv}</text>
"##,
            rank = idx + 1,
            name = esc(&trunc(&item.name, 34)),
            qty = item.qty,
            gmv = esc(&fmt_rp(item.gmv)),
            ty = y,
            sy = y + 24.0,
        ));
    }
    if top_rows.is_empty() {
        top_rows.push_str(
            r##"<text x="582" y="620" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" fill="#6B7280">Belum ada produk aktif di window ini.</text>"##,
        );
    }

    let fee_line = if c.fee_est.abs() < 0.5 {
        String::new()
    } else {
        format!(" · Fee plat. ~{}", fmt_rp(c.fee_est))
    };
    let est_line = format!(
        "HPP ~{} · Gross ~{} · match {:.0}%{} · Ongkir ~{}",
        fmt_rp(report.hpp),
        fmt_rp(report.gross),
        report.hpp_match_pct,
        fee_line,
        fmt_rp(c.ship_est)
    );

    Ok(format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#0F766E"/>
      <stop offset="55%" stop-color="#115E59"/>
      <stop offset="100%" stop-color="#134E4A"/>
    </linearGradient>
    <filter id="shadow" x="-5%" y="-5%" width="110%" height="120%">
      <feDropShadow dx="0" dy="10" stdDeviation="18" flood-color="#0F172A" flood-opacity="0.18"/>
    </filter>
  </defs>

  <rect width="100%" height="100%" fill="#ECFDF5"/>
  <rect x="{pad}" y="{pad}" width="{inner_w}" height="{inner_h}" rx="30" fill="#FFFFFF" filter="url(#shadow)"/>
  <rect x="{pad}" y="{pad}" width="{inner_w}" height="166" rx="30" fill="url(#bg)"/>
  <rect x="{pad}" y="168" width="{inner_w}" height="34" fill="url(#bg)"/>

  <text x="{cx}" y="86" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" font-weight="700" fill="#99F6E4" letter-spacing="3">REKAP HARI INI</text>
  <text x="{cx}" y="128" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="34" font-weight="800" fill="#FFFFFF">{date}</text>
  <text x="{cx}" y="164" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="17" fill="#CCFBF1">{as_of}</text>

{stats}
{platform_chips}

  <text x="56" y="526" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="20" font-weight="800" fill="#134E4A">KURIR</text>
  <text x="554" y="526" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="20" font-weight="800" fill="#134E4A">TOP 5 PRODUK</text>
{courier_rows}
{top_rows}

  <rect x="56" y="1036" width="968" height="116" rx="24" fill="#F0FDFA" stroke="#99F6E4"/>
  <text x="82" y="1074" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" font-weight="800" fill="#0F766E">EST INTERNAL</text>
  <text x="82" y="1118" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="24" font-weight="800" fill="#134E4A">{est_line}</text>
  <text x="82" y="1144" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#0F766E">HPP, gross, fee, dan ongkir adalah estimasi operasional; bukan accounting final.</text>

  <rect x="56" y="1184" width="968" height="112" rx="24" fill="#FEF2F2" stroke="#FCA5A5"/>
  <text x="82" y="1226" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" font-weight="800" fill="#B91C1C">CANCEL</text>
  <text x="82" y="1266" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="28" font-weight="800" fill="#991B1B">{cancel_n} order · {cancel_gmv} · {cancel_pct} vs kemarin</text>

  <text x="{cx}" y="1508" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#9CA3AF">orders · generated same-hour window · exclude cancel untuk metrik aktif</text>
</svg>
"##,
        w = CARD_W,
        h = h,
        pad = PAD,
        inner_w = inner_w,
        inner_h = h as f64 - 2.0 * PAD,
        cx = cx,
        date = esc(&date_label),
        as_of = esc(&as_of_label),
        stats = stats,
        platform_chips = platform_chips,
        courier_rows = courier_rows,
        top_rows = top_rows,
        est_line = esc(&est_line),
        cancel_n = c.cancel_n,
        cancel_gmv = esc(&fmt_rp(c.cancel_gmv)),
        cancel_pct = esc(&fmt_pct(pct_i64(c.cancel_n, p.cancel_n))),
    ))
}

pub fn render_png(report: &DailyInfographic) -> Result<Vec<u8>> {
    let svg = to_svg(report)?;
    svg_to_png(&svg)
}

pub fn write_daily_infographic_png(path: &Path, report: &DailyInfographic) -> Result<()> {
    let png = render_png(report)?;
    write_png(path, &png)
}

pub fn default_png_path(date: NaiveDate) -> PathBuf {
    PathBuf::from(format!("logs/rekap-sore-{}.png", date.format("%Y-%m-%d")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_num_reads_number() {
        assert_eq!(json_num(&json!(12500.5)), Some(12500.5));
    }

    #[test]
    fn json_num_reads_string() {
        assert_eq!(json_num(&json!("12500")), Some(12500.0));
    }

    #[test]
    fn platform_fee_est_ignores_negative_and_total_platform_fee() {
        let fee = json!({
            "commissionFee": 5000,
            "serviceFee": "-2500",
            "totalPlatformFee": 99_999,
            "sellerTransactionFee": 88_888,
            "orderProcessFee": 1500,
            "sellerOrderProcessingFee": -100,
            "newServiceFee": "2000"
        });

        assert_eq!(platform_fee_est(&fee), 8500.0);
    }

    #[test]
    fn platform_fee_est_zero_when_only_negative() {
        let fee = json!({
            "commissionFee": -5000,
            "serviceFee": "-2500",
            "totalPlatformFee": -12500
        });

        assert_eq!(platform_fee_est(&fee), 0.0);
    }

    #[test]
    fn shipping_pref_uses_actual_instead_of_double_counting_estimate() {
        let fee = json!({
            "otherFeeInfo": { "actualShippingFee": 12000 },
            "estimatedShippingFee": 9000
        });

        assert_eq!(shipping_pref(&fee), 12000.0);
    }

    #[test]
    fn shipping_pref_reads_actual_only() {
        let fee = json!({
            "otherFeeInfo": { "actualShippingFee": "7000" }
        });

        assert_eq!(shipping_pref(&fee), 7000.0);
    }

    #[test]
    fn shipping_pref_reads_estimate_when_actual_missing() {
        let fee = json!({
            "estimatedShippingFee": "9000"
        });

        assert_eq!(shipping_pref(&fee), 9000.0);
    }

    #[test]
    fn svg_renders_to_png() {
        let report = DailyInfographic {
            date: NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
            as_of: DateTime::parse_from_rfc3339("2026-07-23T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            tz_offset_hours: 7,
            current: WindowStats {
                gmv: 8_866_646.0,
                order_count: 120,
                aov: 73_888.0,
                qty: 154,
                platform: PlatformStats {
                    shopee_n: 100,
                    shopee_gmv: 7_500_000.0,
                    tiktok_n: 20,
                    tiktok_gmv: 1_366_646.0,
                },
                carriers: CarrierStats {
                    instant: 5,
                    spx: 40,
                    jnt: 35,
                    jne: 20,
                    sicepat: 10,
                },
                fee_est: 123_000.0,
                ship_est: 456_000.0,
                cancel_n: 3,
                cancel_gmv: 210_000.0,
            },
            previous: WindowStats {
                gmv: 7_000_000.0,
                order_count: 100,
                aov: 70_000.0,
                qty: 130,
                platform: PlatformStats {
                    shopee_n: 90,
                    shopee_gmv: 6_200_000.0,
                    tiktok_n: 10,
                    tiktok_gmv: 800_000.0,
                },
                carriers: CarrierStats {
                    instant: 4,
                    spx: 30,
                    jnt: 28,
                    jne: 18,
                    sicepat: 9,
                },
                fee_est: 100_000.0,
                ship_est: 300_000.0,
                cancel_n: 2,
                cancel_gmv: 140_000.0,
            },
            top: vec![TopProduct {
                name: "Obayito Tencel Piyama Panjang".into(),
                qty: 12,
                gmv: 840_000.0,
            }],
            hpp: 4_200_000.0,
            gross: 4_666_646.0,
            hpp_match_pct: 92.0,
        };

        let png = render_png(&report).expect("png");
        assert!(png.starts_with(b"\x89PNG"));
    }
}
