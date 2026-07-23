//! Daily order rekap → SVG → PNG (resvg), optional upload to temp.sh.
//!
//! Same SKU + variant rows are merged (qty summed). Canceled orders excluded by default.

use crate::error::{Error, Result};
use chrono::{Datelike, Local, NaiveDate};
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CARD_W: u32 = 1080;
const PAD_X: f64 = 48.0;
const ROW_H: f64 = 56.0;
/// Y where the SKU table header starts (after hero + stats + meta).
const TABLE_TOP: f64 = 380.0;
const FOOTER_H: f64 = 88.0;
const MAX_SKU_ROWS: usize = 40;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailySkuLine {
    pub sku: String,
    pub variant: Option<String>,
    pub item_name: Option<String>,
    pub qty: i64,
    pub order_count: i64,
    pub amount: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyRekap {
    pub date: NaiveDate,
    pub tz_offset_hours: i32,
    pub order_count: i64,
    pub item_qty: i64,
    pub sku_lines: i64,
    pub gmv: Option<String>,
    pub by_state: Vec<(String, i64)>,
    pub by_platform: Vec<(String, i64)>,
    pub skus: Vec<DailySkuLine>,
}

/// Calendar-day bounds in a fixed UTC offset (Asia/Jakarta = +7).
pub fn day_bounds_utc(
    date: NaiveDate,
    tz_offset_hours: i32,
) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    let start_local = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| Error::Other(format!("invalid date {date}")))?;
    let start_utc = start_local.and_utc() - chrono::Duration::hours(tz_offset_hours as i64);
    let end_utc = start_utc + chrono::Duration::days(1);
    Ok((start_utc, end_utc))
}

pub async fn load_daily_rekap(
    pool: &PgPool,
    date: NaiveDate,
    tz_offset_hours: i32,
    include_canceled: bool,
) -> Result<DailyRekap> {
    let (start_utc, end_utc) = day_bounds_utc(date, tz_offset_hours)?;

    let cancel_sql = if include_canceled {
        ""
    } else {
        "AND o.state NOT IN ('canceled', 'cancelled')"
    };

    let order_sql = format!(
        r#"
        SELECT
            count(*)::bigint AS order_count,
            coalesce(sum(o.amount), 0)::text AS gmv
        FROM orders o
        WHERE coalesce(o.ordered_at, o.first_seen_at) >= $1
          AND coalesce(o.ordered_at, o.first_seen_at) < $2
          {cancel_sql}
        "#
    );
    let order_row = sqlx::query(&order_sql)
        .bind(start_utc)
        .bind(end_utc)
        .fetch_one(pool)
        .await?;
    let order_count: i64 = order_row.get("order_count");
    let gmv: String = order_row.get("gmv");

    let state_sql = format!(
        r#"
        SELECT o.state, count(*)::bigint AS n
        FROM orders o
        WHERE coalesce(o.ordered_at, o.first_seen_at) >= $1
          AND coalesce(o.ordered_at, o.first_seen_at) < $2
          {cancel_sql}
        GROUP BY o.state
        ORDER BY n DESC
        "#
    );
    let state_rows = sqlx::query(&state_sql)
        .bind(start_utc)
        .bind(end_utc)
        .fetch_all(pool)
        .await?;
    let by_state: Vec<(String, i64)> = state_rows
        .into_iter()
        .map(|r| (r.get::<String, _>("state"), r.get("n")))
        .collect();

    let plat_sql = format!(
        r#"
        SELECT coalesce(nullif(o.platform, ''), 'unknown') AS platform, count(*)::bigint AS n
        FROM orders o
        WHERE coalesce(o.ordered_at, o.first_seen_at) >= $1
          AND coalesce(o.ordered_at, o.first_seen_at) < $2
          {cancel_sql}
        GROUP BY 1
        ORDER BY n DESC
        "#
    );
    let plat_rows = sqlx::query(&plat_sql)
        .bind(start_utc)
        .bind(end_utc)
        .fetch_all(pool)
        .await?;
    let by_platform: Vec<(String, i64)> = plat_rows
        .into_iter()
        .map(|r| (r.get::<String, _>("platform"), r.get("n")))
        .collect();

    let sku_sql = format!(
        r#"
        SELECT
            coalesce(nullif(trim(oi.sku), ''), '(tanpa sku)') AS sku,
            nullif(trim(oi.variant_attr), '') AS variant,
            nullif(trim(oi.item_name), '') AS item_name,
            coalesce(sum(oi.quantity), 0)::bigint AS qty,
            count(distinct o.id)::bigint AS order_count,
            coalesce(sum(oi.amount), 0)::text AS amount
        FROM order_items oi
        JOIN orders o ON o.id = oi.order_id
        WHERE coalesce(o.ordered_at, o.first_seen_at) >= $1
          AND coalesce(o.ordered_at, o.first_seen_at) < $2
          {cancel_sql}
        GROUP BY 1, 2, 3
        ORDER BY qty DESC, sku ASC
        LIMIT 200
        "#
    );
    let sku_rows = sqlx::query(&sku_sql)
        .bind(start_utc)
        .bind(end_utc)
        .fetch_all(pool)
        .await?;

    let mut skus = Vec::with_capacity(sku_rows.len());
    let mut item_qty = 0i64;
    for r in sku_rows {
        let qty: i64 = r.get("qty");
        item_qty += qty;
        skus.push(DailySkuLine {
            sku: r.get("sku"),
            variant: r.get("variant"),
            item_name: r.get("item_name"),
            qty,
            order_count: r.get("order_count"),
            amount: Some(r.get("amount")),
        });
    }
    let sku_lines = skus.len() as i64;

    Ok(DailyRekap {
        date,
        tz_offset_hours,
        order_count,
        item_qty,
        sku_lines,
        gmv: Some(gmv),
        by_state,
        by_platform,
        skus,
    })
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

fn fmt_idr(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let whole = cleaned.split('.').next().unwrap_or("0");
    let digits: String = whole.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return "Rp 0".into();
    }
    let mut out = String::new();
    for (i, ch) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    let grouped: String = out.chars().rev().collect();
    format!("Rp {grouped}")
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

/// Build a clean card-style SVG for WhatsApp (render to PNG before send).
pub fn rekap_to_svg(rekap: &DailyRekap) -> String {
    let show = rekap.skus.len().min(MAX_SKU_ROWS);
    let extra = rekap.skus.len().saturating_sub(show);
    let table_h = 44.0 + (show as f64) * ROW_H + if extra > 0 { 36.0 } else { 0.0 };
    let h = (TABLE_TOP + table_h + FOOTER_H).ceil() as u32;
    let w = CARD_W;

    let date_label = format!(
        "{}, {} {} {}",
        day_name_id(rekap.date),
        rekap.date.day(),
        month_name_id(rekap.date.month()),
        rekap.date.year()
    );
    let gmv = rekap
        .gmv
        .as_deref()
        .map(fmt_idr)
        .unwrap_or_else(|| "Rp 0".into());
    let state_bits: String = rekap
        .by_state
        .iter()
        .take(5)
        .map(|(s, n)| format!("{} {}", esc(s), n))
        .collect::<Vec<_>>()
        .join("  ·  ");
    let plat_bits: String = rekap
        .by_platform
        .iter()
        .map(|(s, n)| format!("{} {}", esc(&s.to_ascii_lowercase()), n))
        .collect::<Vec<_>>()
        .join("  ·  ");

    let mut rows_svg = String::new();
    for (i, line) in rekap.skus.iter().take(show).enumerate() {
        let y = TABLE_TOP + 44.0 + (i as f64) * ROW_H;
        let bg = if i % 2 == 0 { "#FFFFFF" } else { "#F7F8FA" };
        let title = line
            .item_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| trunc(s, 42))
            .unwrap_or_else(|| trunc(&line.sku, 42));
        let sub = match &line.variant {
            Some(v) if !v.is_empty() => format!("{} · {}", trunc(&line.sku, 28), trunc(v, 28)),
            _ => trunc(&line.sku, 48),
        };
        let amt = line
            .amount
            .as_deref()
            .map(fmt_idr)
            .unwrap_or_else(|| "—".into());
        rows_svg.push_str(&format!(
            r##"
  <rect x="{px}" y="{y}" width="{rw}" height="{rh}" fill="{bg}"/>
  <text x="{tx}" y="{ty1}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="22" font-weight="700" fill="#111827">{title}</text>
  <text x="{tx}" y="{ty2}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" fill="#6B7280">{sub}</text>
  <text x="{qx}" y="{ty1}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="24" font-weight="700" fill="#0F766E">×{qty}</text>
  <text x="{ax}" y="{ty1}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" fill="#374151">{amt}</text>
"##,
            px = PAD_X,
            y = y,
            rw = w as f64 - PAD_X * 2.0,
            rh = ROW_H,
            bg = bg,
            tx = PAD_X + 20.0,
            ty1 = y + 24.0,
            ty2 = y + 46.0,
            qx = w as f64 - PAD_X - 200.0,
            ax = w as f64 - PAD_X - 20.0,
            title = esc(&title),
            sub = esc(&sub),
            qty = line.qty,
            amt = esc(&amt),
        ));
    }
    if extra > 0 {
        let y = TABLE_TOP + 44.0 + (show as f64) * ROW_H + 22.0;
        rows_svg.push_str(&format!(
            r##"  <text x="{cx}" y="{y}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" fill="#9CA3AF">+{extra} SKU lain digabung (tidak ditampilkan)</text>
"##,
            cx = w as f64 / 2.0,
            y = y,
            extra = extra,
        ));
    }

    let table_top = TABLE_TOP;
    let footer_y = h as f64 - 36.0;
    let gen = Local::now().format("%H:%M").to_string();

    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#0F766E"/>
      <stop offset="55%" stop-color="#115E59"/>
      <stop offset="100%" stop-color="#134E4A"/>
    </linearGradient>
    <filter id="shadow" x="-5%" y="-5%" width="110%" height="120%">
      <feDropShadow dx="0" dy="8" stdDeviation="16" flood-color="#0F172A" flood-opacity="0.18"/>
    </filter>
  </defs>

  <rect width="100%" height="100%" fill="#ECFDF5"/>
  <rect x="24" y="24" width="{inner_w}" height="{inner_h}" rx="28" fill="#FFFFFF" filter="url(#shadow)"/>
  <rect x="24" y="24" width="{inner_w}" height="168" rx="28" fill="url(#bg)"/>
  <rect x="24" y="160" width="{inner_w}" height="32" fill="url(#bg)"/>

  <text x="{cx}" y="78" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" font-weight="600" fill="#99F6E4" letter-spacing="3">REKAP HARIAN ORDER</text>
  <text x="{cx}" y="122" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="36" font-weight="700" fill="#FFFFFF">{date}</text>
  <text x="{cx}" y="158" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="16" fill="#CCFBF1">SKU sama digabung · exclude cancel · UTC{tz:+}</text>

  <!-- stat pills -->
  <rect x="{p1x}" y="208" width="300" height="92" rx="18" fill="#F0FDFA" stroke="#99F6E4" stroke-width="1.5"/>
  <text x="{p1cx}" y="242" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#0F766E">ORDER</text>
  <text x="{p1cx}" y="278" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="34" font-weight="700" fill="#134E4A">{orders}</text>

  <rect x="{p2x}" y="208" width="300" height="92" rx="18" fill="#F0FDFA" stroke="#99F6E4" stroke-width="1.5"/>
  <text x="{p2cx}" y="242" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#0F766E">QTY ITEM</text>
  <text x="{p2cx}" y="278" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="34" font-weight="700" fill="#134E4A">{qty}</text>

  <rect x="{p3x}" y="208" width="300" height="92" rx="18" fill="#F0FDFA" stroke="#99F6E4" stroke-width="1.5"/>
  <text x="{p3cx}" y="242" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#0F766E">GMV</text>
  <text x="{p3cx}" y="278" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="26" font-weight="700" fill="#134E4A">{gmv}</text>

  <text x="{px}" y="{meta_y}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="15" fill="#6B7280">{state}</text>
  <text x="{px}" y="{meta_y2}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="15" fill="#6B7280">{plat}</text>

  <!-- table header -->
  <rect x="{px}" y="{tt}" width="{tw}" height="40" rx="12" fill="#134E4A"/>
  <text x="{tx}" y="{tty}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" font-weight="700" fill="#CCFBF1">PRODUK (SKU DIGABUNG)</text>
  <text x="{qx}" y="{tty}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" font-weight="700" fill="#CCFBF1">QTY</text>
  <text x="{ax}" y="{tty}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" font-weight="700" fill="#CCFBF1">NOMINAL</text>

{rows}

  <text x="{cx}" y="{fy}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#9CA3AF">order.rs · digenerate {gen} · {sku_n} baris SKU unik</text>
</svg>
"##,
        w = w,
        h = h,
        inner_w = w as f64 - 48.0,
        inner_h = h as f64 - 48.0,
        cx = w as f64 / 2.0,
        date = esc(&date_label),
        tz = rekap.tz_offset_hours,
        p1x = PAD_X,
        p1cx = PAD_X + 150.0,
        p2x = PAD_X + 320.0,
        p2cx = PAD_X + 470.0,
        p3x = PAD_X + 640.0,
        p3cx = PAD_X + 790.0,
        orders = rekap.order_count,
        qty = rekap.item_qty,
        gmv = esc(&gmv),
        px = PAD_X,
        meta_y = 328.0,
        meta_y2 = 350.0,
        state = if state_bits.is_empty() {
            "—".into()
        } else {
            format!("Status: {state_bits}")
        },
        plat = if plat_bits.is_empty() {
            "—".into()
        } else {
            format!("Platform: {plat_bits}")
        },
        tt = table_top,
        tw = w as f64 - PAD_X * 2.0,
        tx = PAD_X + 20.0,
        tty = table_top + 26.0,
        qx = w as f64 - PAD_X - 200.0,
        ax = w as f64 - PAD_X - 20.0,
        rows = rows_svg,
        fy = footer_y,
        gen = esc(&gen),
        sku_n = rekap.sku_lines,
    )
}

fn load_fontdb() -> resvg::usvg::fontdb::Database {
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_system_fonts();
    // Prefer common Linux fonts for clean Latin + numbers.
    for path in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    ] {
        if Path::new(path).exists() {
            db.load_font_file(path).ok();
        }
    }
    db.set_sans_serif_family("DejaVu Sans");
    db
}

/// Render SVG string to PNG bytes via resvg.
pub fn svg_to_png(svg: &str) -> Result<Vec<u8>> {
    let opt = resvg::usvg::Options {
        fontdb: std::sync::Arc::new(load_fontdb()),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_str(svg, &opt)
        .map_err(|e| Error::Other(format!("svg parse: {e}")))?;

    let size = tree.size().to_int_size();
    let w = size.width().max(1);
    let h = size.height().max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| Error::Other("pixmap alloc failed".into()))?;
    // Soft mint background under transparent areas.
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(236, 253, 245, 255));
    let ts = resvg::tiny_skia::Transform::identity();
    resvg::render(&tree, ts, &mut pixmap.as_mut());

    pixmap
        .encode_png()
        .map_err(|e| Error::Other(format!("png encode: {e}")))
}

pub fn write_png(path: &Path, png: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, png)?;
    Ok(())
}

/// Upload PNG to temp.sh (`multipart file=`). Returns public URL.
pub async fn upload_temp_sh(png: &[u8], filename: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("order.rs-daily-rekap/0.1")
        .build()?;
    let part = reqwest::multipart::Part::bytes(png.to_vec())
        .file_name(filename.to_string())
        .mime_str("image/png")
        .map_err(|e| Error::Other(format!("multipart: {e}")))?;
    let form = reqwest::multipart::Form::new().part("file", part);
    let resp = client
        .post("https://temp.sh/upload")
        .multipart(form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(Error::Other(format!(
            "temp.sh HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    let url = body.trim();
    if !url.starts_with("http") {
        return Err(Error::Other(format!(
            "temp.sh unexpected body: {}",
            url.chars().take(200).collect::<String>()
        )));
    }
    Ok(url.to_string())
}

pub fn default_png_path(date: NaiveDate) -> PathBuf {
    PathBuf::from(format!("logs/rekap-harian-{}.png", date.format("%Y-%m-%d")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_idr_groups() {
        assert_eq!(fmt_idr("84650"), "Rp 84.650");
        assert_eq!(fmt_idr("8866646.00"), "Rp 8.866.646");
    }

    #[test]
    fn svg_renders_png() {
        let rekap = DailyRekap {
            date: NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            tz_offset_hours: 7,
            order_count: 5,
            item_qty: 12,
            sku_lines: 2,
            gmv: Some("150000".into()),
            by_state: vec![("shipped".into(), 3), ("new".into(), 2)],
            by_platform: vec![("shopee".into(), 4), ("tiktok".into(), 1)],
            skus: vec![
                DailySkuLine {
                    sku: "OB-0136-NB".into(),
                    variant: Some("NB".into()),
                    item_name: Some("Sample Product".into()),
                    qty: 3,
                    order_count: 2,
                    amount: Some("195000".into()),
                },
                DailySkuLine {
                    sku: "MB-001".into(),
                    variant: None,
                    item_name: None,
                    qty: 1,
                    order_count: 1,
                    amount: Some("50000".into()),
                },
            ],
        };
        let svg = rekap_to_svg(&rekap);
        assert!(svg.contains("REKAP HARIAN"));
        let png = svg_to_png(&svg).expect("render");
        assert!(png.starts_with(b"\x89PNG"));
        assert!(png.len() > 5_000);
    }
}
