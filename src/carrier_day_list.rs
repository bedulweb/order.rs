//! Daily carrier list card (Instant / SPX / JNE / J&T / SiCepat) with brand logos → SVG → PNG.
//! Canceled orders are excluded from counts (not shown on the card).

use crate::daily_report::write_png;
use crate::error::{Error, Result};
use base64::Engine;
use chrono::{Datelike, NaiveDate};
use sqlx::{PgPool, Row};
use std::path::{Path, PathBuf};

const CARD_W: u32 = 1080;
const PAD: f64 = 36.0;

#[derive(Debug, Clone)]
pub struct CarrierDayList {
    pub date: NaiveDate,
    /// SPX Instant + GoSend / Grab / same-day / prioritas / paxel / gojek, etc.
    pub instant_orders: i64,
    /// SPX non-instant (Standard, Hemat, …).
    pub spx_orders: i64,
    pub jne_orders: i64,
    pub jnt_orders: i64,
    pub sicepat_orders: i64,
    pub other_orders: i64,
    pub total_orders: i64,
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

/// Resolve logo dir: `LOGOS_DIR` env, else `logs/logos-logistics` under cwd / crate root.
pub fn default_logos_dir() -> PathBuf {
    if let Ok(p) = std::env::var("LOGOS_DIR") {
        return PathBuf::from(p);
    }
    let candidates = [
        PathBuf::from("logs/logos-logistics"),
        PathBuf::from("/home/ujang/projects/apps/orders/logs/logos-logistics"),
    ];
    for c in candidates {
        if c.is_dir() {
            return c;
        }
    }
    PathBuf::from("logs/logos-logistics")
}

pub fn logo_data_uri(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::Other(format!("read logo {}: {e}", path.display())))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

/// Load order counts for a WIB calendar day.
/// Exclusive buckets (priority): Instant → SPX → J&T → JNE → SiCepat → other.
/// Canceled/cancelled states are omitted from the system counts entirely.
pub async fn load_carrier_day_list(
    pool: &PgPool,
    day_wib: NaiveDate,
    tz_offset_hours: i32,
) -> Result<CarrierDayList> {
    let (start_utc, end_utc) = crate::daily_report::day_bounds_utc(day_wib, tz_offset_hours)?;
    let row = sqlx::query(
        r#"
        WITH base AS (
            SELECT
                lower(
                    coalesce(buyer_shipping_carrier, '') || ' ' ||
                    coalesce(shipment_provider, '') || ' ' ||
                    coalesce(shipping_carrier_name, '')
                ) AS hay
            FROM orders
            WHERE coalesce(ordered_at, first_seen_at) >= $1
              AND coalesce(ordered_at, first_seen_at) < $2
              AND lower(coalesce(state, '')) NOT IN ('canceled', 'cancelled')
        ),
        tagged AS (
            SELECT
                CASE
                    -- Instant first (includes SPX Instant; separate from SPX Standard/Hemat)
                    WHEN hay ~ 'instant'
                      OR hay ~ 'sameday'
                      OR hay ~ 'same day'
                      OR hay ~ 'same-day'
                      OR hay ~ 'prioritas'
                      OR hay ~ 'gojek'
                      OR hay ~ 'gosend'
                      OR hay ~ 'grab'
                      OR hay ~ 'paxel'
                    THEN 'instant'
                    WHEN hay ~ 'spx'
                      OR hay ~ 'shopee express'
                      OR hay ~ 'shopee xpress'
                      OR hay ~ 'shopee-xpress'
                    THEN 'spx'
                    WHEN hay ~ 'j&t'
                      OR hay ~ 'jnt'
                      OR hay ~ 'j-t'
                      OR hay ~ 'jet express'
                    THEN 'jnt'
                    WHEN hay ~ 'jne'
                    THEN 'jne'
                    WHEN hay ~ 'sicepat'
                      OR hay ~ 'si cepat'
                      OR hay ~ 'si-cepat'
                    THEN 'sicepat'
                    ELSE 'other'
                END AS bucket
            FROM base
        )
        SELECT
            count(*)::bigint AS total,
            count(*) FILTER (WHERE bucket = 'instant')::bigint AS instant,
            count(*) FILTER (WHERE bucket = 'spx')::bigint AS spx,
            count(*) FILTER (WHERE bucket = 'jne')::bigint AS jne,
            count(*) FILTER (WHERE bucket = 'jnt')::bigint AS jnt,
            count(*) FILTER (WHERE bucket = 'sicepat')::bigint AS sicepat,
            count(*) FILTER (WHERE bucket = 'other')::bigint AS other
        FROM tagged
        "#,
    )
    .bind(start_utc)
    .bind(end_utc)
    .fetch_one(pool)
    .await?;

    Ok(CarrierDayList {
        date: day_wib,
        instant_orders: row.get("instant"),
        spx_orders: row.get("spx"),
        jne_orders: row.get("jne"),
        jnt_orders: row.get("jnt"),
        sicepat_orders: row.get("sicepat"),
        other_orders: row.get("other"),
        total_orders: row.get("total"),
    })
}

struct CarrierVisual {
    key: &'static str,
    label: &'static str,
    subtitle: &'static str,
    count: i64,
    bg: &'static str,
    accent: &'static str,
    logo_file: &'static str,
}

pub fn carrier_day_list_to_svg(list: &CarrierDayList, logos_dir: &Path) -> Result<String> {
    let carriers = [
        CarrierVisual {
            key: "instant",
            label: "Instant",
            subtitle: "SPX Instant · GoSend · Grab · Same Day",
            count: list.instant_orders,
            bg: "#FEF3C7",
            accent: "#B45309",
            // Stand-in brand mark for the instant bucket (category, not one courier).
            logo_file: "gosend.png",
        },
        CarrierVisual {
            key: "spx",
            label: "SPX",
            subtitle: "Standard · Hemat",
            count: list.spx_orders,
            bg: "#F5F3FF",
            accent: "#6D28D9",
            logo_file: "spx-express.png",
        },
        CarrierVisual {
            key: "jne",
            label: "JNE",
            subtitle: "Reguler / ekspres",
            count: list.jne_orders,
            bg: "#EFF6FF",
            accent: "#1D4ED8",
            logo_file: "jne.png",
        },
        CarrierVisual {
            key: "jnt",
            label: "J&T",
            subtitle: "Express",
            count: list.jnt_orders,
            bg: "#FFF7ED",
            accent: "#C2410C",
            logo_file: "j-t-express.png",
        },
        CarrierVisual {
            key: "sicepat",
            label: "SiCepat",
            subtitle: "REG · Halu · BEST",
            count: list.sicepat_orders,
            bg: "#ECFDF5",
            accent: "#047857",
            logo_file: "sicepat.png",
        },
    ];

    let mut logo_uris = Vec::new();
    for c in &carriers {
        let p = logos_dir.join(c.logo_file);
        logo_uris.push(logo_data_uri(&p)?);
    }

    let n_rows = carriers.len() as f64;
    let w = CARD_W as f64;
    let header_h = 148.0;
    let row_h = 128.0;
    let row_gap = 14.0;
    let body_pad_top = 26.0;
    let total_h = 80.0;
    let body_h = body_pad_top + row_h * n_rows + row_gap * (n_rows - 1.0) + 22.0 + total_h + 24.0;
    let card_h = header_h + body_h;
    let h = (PAD * 2.0 + card_h).ceil() as u32;
    let inner_w = w - 2.0 * PAD;
    let inner_h = h as f64 - 2.0 * PAD;
    let cx = w / 2.0;
    let content_x = PAD + 36.0;
    let content_w = inner_w - 72.0;

    let date_label = format!(
        "{}, {} {} {}",
        day_name_id(list.date),
        list.date.day(),
        month_name_id(list.date.month()),
        list.date.year()
    );

    let logo_box = 96.0f64;
    let mut rows = String::new();
    for (i, c) in carriers.iter().enumerate() {
        let y = PAD + header_h + body_pad_top + (i as f64) * (row_h + row_gap);
        let uri = &logo_uris[i];
        let logo_x = content_x + 20.0;
        let logo_y = y + (row_h - logo_box) / 2.0;
        let text_x = logo_x + logo_box + 24.0;
        let num_x = content_x + content_w - 28.0;
        let clip = format!("logo{}", c.key);

        rows.push_str(&format!(
            r##"
  <rect x="{rx}" y="{y}" width="{rw}" height="{rh}" rx="22" fill="{bg}"/>
  <rect x="{lx}" y="{ly}" width="{lb}" height="{lb}" rx="18" fill="#FFFFFF"/>
  <defs>
    <clipPath id="{clip}">
      <rect x="{lx}" y="{ly}" width="{lb}" height="{lb}" rx="18"/>
    </clipPath>
  </defs>
  <image href="{uri}" x="{lx}" y="{ly}" width="{lb}" height="{lb}" clip-path="url(#{clip})" preserveAspectRatio="xMidYMid meet"/>
  <text x="{tx}" y="{ty1}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="26" font-weight="700" fill="#0F172A">{label}</text>
  <text x="{tx}" y="{ty2}" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="15" fill="#64748B">{sub}</text>
  <text x="{nx}" y="{ny1}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="48" font-weight="700" fill="{accent}">{count}</text>
  <text x="{nx}" y="{ny2}" text-anchor="end" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="14" fill="#64748B">pesanan</text>
"##,
            rx = content_x,
            y = y,
            rw = content_w,
            rh = row_h,
            bg = c.bg,
            lx = logo_x,
            ly = logo_y,
            lb = logo_box,
            clip = clip,
            uri = uri,
            tx = text_x,
            ty1 = y + 52.0,
            ty2 = y + 80.0,
            nx = num_x,
            ny1 = y + 62.0,
            ny2 = y + 90.0,
            label = esc(c.label),
            sub = esc(c.subtitle),
            accent = c.accent,
            count = c.count,
        ));
    }

    let y_total = PAD + header_h + body_pad_top + n_rows * (row_h + row_gap) - row_gap + 12.0;

    // Plain total only — no cancel wording. Other bucket only if non-zero.
    let total_line = if list.other_orders > 0 {
        format!(
            "{} pesanan · lain-lain {}",
            list.total_orders, list.other_orders
        )
    } else {
        format!("{} pesanan", list.total_orders)
    };

    Ok(format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <linearGradient id="hero" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#0EA5E9"/>
      <stop offset="50%" stop-color="#0284C7"/>
      <stop offset="100%" stop-color="#0369A1"/>
    </linearGradient>
    <filter id="shadow" x="-4%" y="-4%" width="108%" height="112%">
      <feDropShadow dx="0" dy="8" stdDeviation="16" flood-color="#0F172A" flood-opacity="0.12"/>
    </filter>
    <clipPath id="cardTop">
      <rect x="{pad}" y="{pad}" width="{iw}" height="{ih}" rx="28"/>
    </clipPath>
  </defs>

  <rect width="100%" height="100%" fill="#F0F9FF"/>
  <rect x="{pad}" y="{pad}" width="{iw}" height="{ih}" rx="28" fill="#FFFFFF" filter="url(#shadow)"/>
  <g clip-path="url(#cardTop)">
    <rect x="{pad}" y="{pad}" width="{iw}" height="{hh}" fill="url(#hero)"/>
  </g>

  <text x="{cx}" y="{t1}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="15" font-weight="600" fill="#BAE6FD" letter-spacing="3.5">LIST PESANAN HARI INI</text>
  <text x="{cx}" y="{t2}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="28" font-weight="700" fill="#FFFFFF">Instant · SPX · JNE · J&amp;T · SiCepat</text>
  <text x="{cx}" y="{t3}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" fill="#E0F2FE">{date}</text>

{rows}

  <rect x="{rx}" y="{yt}" width="{rw}" height="72" rx="20" fill="#0F172A"/>
  <text x="{cx}" y="{yt1}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="18" fill="#94A3B8">TOTAL</text>
  <text x="{cx}" y="{yt2}" text-anchor="middle" font-family="DejaVu Sans, Liberation Sans, sans-serif" font-size="26" font-weight="700" fill="#FFFFFF">{total_line}</text>
</svg>
"##,
        w = CARD_W,
        h = h,
        pad = PAD,
        iw = inner_w,
        ih = inner_h,
        hh = header_h,
        cx = cx,
        t1 = PAD + 42.0,
        t2 = PAD + 88.0,
        t3 = PAD + 122.0,
        date = esc(&date_label),
        rows = rows,
        rx = content_x,
        rw = content_w,
        yt = y_total,
        yt1 = y_total + 28.0,
        yt2 = y_total + 56.0,
        total_line = esc(&total_line),
    ))
}

pub async fn render_carrier_day_list_png(list: &CarrierDayList) -> Result<Vec<u8>> {
    let logos = default_logos_dir();
    let svg = carrier_day_list_to_svg(list, &logos)?;
    crate::daily_report::svg_to_png(&svg)
}

pub fn write_carrier_day_list_png(path: &Path, png: &[u8]) -> Result<()> {
    write_png(path, png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_separates_instant_and_spx() {
        let logos = default_logos_dir();
        if !logos.join("spx-express.png").exists() || !logos.join("gosend.png").exists() {
            eprintln!("skip: logos not on disk at {}", logos.display());
            return;
        }
        let list = CarrierDayList {
            date: NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
            instant_orders: 3,
            spx_orders: 10,
            jne_orders: 2,
            jnt_orders: 11,
            sicepat_orders: 7,
            other_orders: 0,
            total_orders: 33,
        };
        let svg = carrier_day_list_to_svg(&list, &logos).expect("svg");
        assert!(svg.contains("Instant"));
        assert!(svg.contains("SPX"));
        assert!(svg.contains("33 pesanan"));
        assert!(!svg.to_ascii_lowercase().contains("cancel"));
        assert!(!svg.contains("lain-lain"));
    }
}
