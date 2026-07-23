//! Instant-order WA notify card: SVG → PNG (playground / future notifier).
//!
//! Layout inspired by Summary List PDF, sized for phone (1080px wide).
//! Cool blue palette — readable on mobile WhatsApp.

use crate::batch_pdf::package_code;
use crate::daily_report::{self, write_png};
use crate::error::{Error, Result};
use crate::product_names::{self, normalize_art};
use base64::Engine;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Phone-friendly width (WhatsApp full-bleed on most devices).
const CARD_W: u32 = 1080;
const PAD: f64 = 36.0;
const INNER_PAD: f64 = 32.0;
const THUMB: f64 = 108.0;
const THUMB_GAP: f64 = 22.0;
const ITEM_GAP: f64 = 22.0;
const ORDER_GAP: f64 = 20.0;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyItem {
    pub sku: Option<String>,
    pub name: Option<String>,
    pub variant_attr: Option<String>,
    pub image_url: Option<String>,
    pub quantity: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyOrder {
    pub order_id: Option<i64>,
    pub platform_order_id: String,
    pub platform: String,
    pub carrier: Option<String>,
    pub is_urgent: Option<bool>,
    pub ordered_at_wib: Option<String>,
    pub state: Option<String>,
    pub items: Vec<NotifyItem>,
}

#[derive(Debug, Clone)]
pub struct NotifyCard {
    pub title: String,
    pub subtitle: String,
    pub orders: Vec<NotifyOrder>,
    pub footer: String,
}

impl NotifyCard {
    pub fn from_orders(orders: Vec<NotifyOrder>) -> Self {
        let orders = orders
            .into_iter()
            .map(normalize_order_names)
            .collect::<Vec<_>>();
        let n = orders.len();
        let n_items: i32 = orders
            .iter()
            .flat_map(|o| o.items.iter())
            .map(|i| i.quantity.max(0))
            .sum();
        let carriers: Vec<String> = {
            let mut c: Vec<String> = orders.iter().filter_map(|o| o.carrier.clone()).collect();
            c.sort();
            c.dedup();
            c
        };
        let carrier_bit = if carriers.is_empty() {
            "Instant".into()
        } else if carriers.len() == 1 {
            carriers[0].clone()
        } else {
            format!("{} kurir", carriers.len())
        };
        Self {
            title: "Pesanan Instant".into(),
            subtitle: format!("{n} order  ·  {n_items} barang  ·  {carrier_bit}"),
            orders,
            footer: String::new(),
        }
    }
}

/// Re-resolve display names (fixes stale fixture SKU fallbacks / 0B typo).
fn normalize_order_names(mut order: NotifyOrder) -> NotifyOrder {
    let empty = HashMap::new();
    for item in &mut order.items {
        let sku = item.sku.as_deref().unwrap_or("");
        let raw = item.name.as_deref();
        // If name looks like a SKU code (or missing), resolve again.
        let needs = match raw {
            None => true,
            Some(n) => {
                let t = n.trim();
                t.is_empty()
                    || t.eq_ignore_ascii_case(sku)
                    || looks_like_sku_code(t)
                    || t.starts_with("0B-")
                    || t.starts_with("OB-") && t.len() < 18 && !t.contains(' ')
            }
        };
        if needs {
            item.name = Some(product_names::resolve_display_name(sku, raw, &empty));
        }
        if let Some(s) = item.sku.as_ref() {
            let n = normalize_art(s);
            if n != *s {
                item.sku = Some(n);
            }
        }
    }
    order
}

fn looks_like_sku_code(s: &str) -> bool {
    let u = s.trim().to_ascii_uppercase();
    // e.g. 0B-0134-S or OB-0136-LB
    let parts: Vec<&str> = u.split('-').collect();
    parts.len() >= 2
        && parts[0].chars().all(|c| c.is_ascii_alphanumeric())
        && parts[0].len() <= 3
        && parts
            .get(1)
            .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()))
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
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{t}…")
}

fn font() -> &'static str {
    "DejaVu Sans, Liberation Sans, Noto Sans, sans-serif"
}

fn short_wib(s: &str) -> String {
    // "2026-07-21 20:47:04 WIB" → "21/07 20:47"
    if s.len() >= 16 {
        let d = &s[0..10];
        let t = &s[11..16];
        if let Some((_y, rest)) = d.split_once('-') {
            if let Some((m, day)) = rest.split_once('-') {
                return format!("{day}/{m} {t}");
            }
        }
    }
    s.to_string()
}

/// Best-effort fetch image URL → PNG data-URI for embedding in SVG.
async fn fetch_thumb_data_uris(urls: &[String]) -> HashMap<String, String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("orders-instant-notify/0.1")
        .build()
    {
        Ok(c) => Arc::new(c),
        Err(_) => return HashMap::new(),
    };

    let mut unique: Vec<String> = urls
        .iter()
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .cloned()
        .collect();
    unique.sort();
    unique.dedup();

    let mut out = HashMap::new();
    for chunk in unique.chunks(8) {
        let mut handles = Vec::new();
        for url in chunk {
            let client = Arc::clone(&client);
            let url = url.clone();
            handles.push(tokio::spawn(async move {
                let bytes = match client.get(&url).send().await {
                    Ok(r) if r.status().is_success() => r.bytes().await.ok(),
                    _ => None,
                };
                let data_uri = bytes.and_then(|b| {
                    let img = image::load_from_memory(&b).ok()?;
                    let thumb = img.thumbnail(160, 160);
                    let mut png = Vec::new();
                    thumb
                        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                        .ok()?;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
                    Some(format!("data:image/png;base64,{b64}"))
                });
                (url, data_uri)
            }));
        }
        for h in handles {
            if let Ok((url, Some(uri))) = h.await {
                out.insert(url, uri);
            }
        }
    }
    out
}

/// Build SVG string for the notify card (thumbs as data-URIs when available).
pub fn card_to_svg(card: &NotifyCard, thumbs: &HashMap<String, String>) -> String {
    let w = CARD_W as f64;
    let content_x = PAD + INNER_PAD;
    let content_w = w - 2.0 * PAD - 2.0 * INNER_PAD;
    let text_x = content_x + THUMB + THUMB_GAP;
    let qty_x = content_x + content_w;

    let header_h = 156.0;
    let mut body_h = 20.0f64;
    for order in &card.orders {
        body_h += 64.0;
        body_h += order.items.len() as f64 * (THUMB + ITEM_GAP);
        body_h += ORDER_GAP;
    }
    let footer_h = 24.0;
    let card_h = header_h + body_h + footer_h + 16.0;
    let h = (PAD * 2.0 + card_h).ceil() as u32;
    let inner_w = w - 2.0 * PAD;
    let inner_h = h as f64 - 2.0 * PAD;

    let mut body = String::new();
    let mut y = PAD + header_h + 12.0;

    for (oi, order) in card.orders.iter().enumerate() {
        let carrier = order
            .carrier
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("Instant");
        let plat = order.platform.to_ascii_lowercase();
        let code = package_code(&order.platform_order_id);
        let when = order
            .ordered_at_wib
            .as_deref()
            .map(short_wib)
            .unwrap_or_else(|| "—".into());

        // Order meta row — plain text only (no second colored header chip).
        let chip_y = y;
        body.push_str(&format!(
            r##"
  <text x="{tx}" y="{ty1}" font-family="{ff}" font-size="23" font-weight="700" fill="#0F172A">{oid}</text>
  <text x="{tx}" y="{ty2}" font-family="{ff}" font-size="16" fill="#64748B">{meta}</text>
  <text x="{qx}" y="{ty1}" text-anchor="end" font-family="{ff}" font-size="24" font-weight="700" fill="#2563EB">{code}</text>
"##,
            tx = content_x,
            ty1 = chip_y + 22.0,
            ty2 = chip_y + 46.0,
            qx = qty_x,
            ff = font(),
            oid = esc(&order.platform_order_id),
            meta = esc(&format!("{plat}  ·  {carrier}  ·  {when}")),
            code = esc(&code),
        ));
        y = chip_y + 58.0 + 14.0;

        for item in &order.items {
            let name = item
                .name
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("(tanpa nama)");
            let sku_disp = item
                .sku
                .as_deref()
                .map(normalize_art)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".into());
            let variant = item
                .variant_attr
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let qty = item.quantity.max(0);
            let img_bottom = y;
            let img_x = content_x;

            let mut drew = false;
            if let Some(url) = item.image_url.as_ref() {
                if let Some(uri) = thumbs.get(url) {
                    // clip via nested: rounded rect + image (resvg supports clipPath)
                    let clip_id =
                        format!("c{}", (img_bottom as u32).wrapping_mul(31) ^ (img_x as u32));
                    body.push_str(&format!(
                        r##"  <defs><clipPath id="{cid}"><rect x="{x}" y="{iy}" width="{tw}" height="{th}" rx="14"/></clipPath></defs>
  <image href="{uri}" x="{x}" y="{iy}" width="{tw}" height="{th}" clip-path="url(#{cid})" preserveAspectRatio="xMidYMid slice"/>
  <rect x="{x}" y="{iy}" width="{tw}" height="{th}" rx="14" fill="none" stroke="#DBEAFE" stroke-width="2"/>
"##,
                        cid = clip_id,
                        uri = uri,
                        x = img_x,
                        iy = img_bottom,
                        tw = THUMB,
                        th = THUMB,
                    ));
                    drew = true;
                }
            }
            if !drew {
                body.push_str(&format!(
                    r##"  <rect x="{x}" y="{iy}" width="{tw}" height="{th}" rx="14" fill="#F1F5F9" stroke="#E2E8F0" stroke-width="1.5"/>
  <text x="{cx}" y="{cy}" text-anchor="middle" font-family="{ff}" font-size="15" fill="#94A3B8">foto</text>
"##,
                    x = img_x,
                    iy = img_bottom,
                    tw = THUMB,
                    th = THUMB,
                    cx = img_x + THUMB / 2.0,
                    cy = img_bottom + THUMB / 2.0 + 5.0,
                    ff = font(),
                ));
            }

            // Name (primary) · variant (secondary) · SKU (muted tertiary)
            let name_y = img_bottom + 30.0;
            let var_y = img_bottom + 58.0;
            let sku_y = img_bottom + 84.0;

            body.push_str(&format!(
                r##"
  <text x="{tx}" y="{ny}" font-family="{ff}" font-size="26" font-weight="700" fill="#0F172A">{name}</text>
"##,
                tx = text_x,
                ny = name_y,
                ff = font(),
                name = esc(&trunc(name, 34)),
            ));

            if let Some(v) = variant {
                body.push_str(&format!(
                    r##"  <text x="{tx}" y="{vy}" font-family="{ff}" font-size="20" font-weight="600" fill="#334155">{var}</text>
"##,
                    tx = text_x,
                    vy = var_y,
                    ff = font(),
                    var = esc(&trunc(v, 36)),
                ));
                body.push_str(&format!(
                    r##"  <text x="{tx}" y="{sy}" font-family="{ff}" font-size="16" fill="#94A3B8">{sku}</text>
"##,
                    tx = text_x,
                    sy = sku_y,
                    ff = font(),
                    sku = esc(&trunc(&sku_disp, 40)),
                ));
            } else {
                body.push_str(&format!(
                    r##"  <text x="{tx}" y="{vy}" font-family="{ff}" font-size="16" fill="#94A3B8">{sku}</text>
"##,
                    tx = text_x,
                    vy = var_y,
                    ff = font(),
                    sku = esc(&trunc(&sku_disp, 40)),
                ));
            }

            body.push_str(&format!(
                r##"  <text x="{qx}" y="{ny}" text-anchor="end" font-family="{ff}" font-size="32" font-weight="700" fill="#1D4ED8">×{qty}</text>
"##,
                qx = qty_x,
                ny = name_y,
                ff = font(),
                qty = qty,
            ));

            y = img_bottom + THUMB + ITEM_GAP;
        }

        if oi + 1 < card.orders.len() {
            // soft section rule between orders
            let ry = y - ITEM_GAP / 2.0 + 4.0;
            body.push_str(&format!(
                r##"  <line x1="{x0}" y1="{ry}" x2="{x1}" y2="{ry}" stroke="#E2E8F0" stroke-width="2"/>
"##,
                x0 = content_x,
                x1 = qty_x,
                ry = ry,
            ));
            y += ORDER_GAP * 0.4;
        }
    }

    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <linearGradient id="hero" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#3B82F6"/>
      <stop offset="50%" stop-color="#2563EB"/>
      <stop offset="100%" stop-color="#1D4ED8"/>
    </linearGradient>
    <filter id="shadow" x="-4%" y="-4%" width="108%" height="112%">
      <feDropShadow dx="0" dy="8" stdDeviation="16" flood-color="#0F172A" flood-opacity="0.12"/>
    </filter>
    <clipPath id="cardTop">
      <rect x="{pad}" y="{pad}" width="{iw}" height="{ih}" rx="28"/>
    </clipPath>
  </defs>

  <rect width="100%" height="100%" fill="#F8FAFC"/>
  <rect x="{pad}" y="{pad}" width="{iw}" height="{ih}" rx="28" fill="#FFFFFF" filter="url(#shadow)"/>
  <!-- Single top header only (clip so bottom edge is straight, no second strip). -->
  <g clip-path="url(#cardTop)">
    <rect x="{pad}" y="{pad}" width="{iw}" height="136" fill="url(#hero)"/>
  </g>

  <text x="{cx}" y="{t1}" text-anchor="middle" font-family="{ff}" font-size="15" font-weight="600" fill="#BFDBFE" letter-spacing="3.5">INSTANT · SEGERA PROSES</text>
  <text x="{cx}" y="{t2}" text-anchor="middle" font-family="{ff}" font-size="38" font-weight="700" fill="#FFFFFF">{title}</text>
  <text x="{cx}" y="{t3}" text-anchor="middle" font-family="{ff}" font-size="18" fill="#DBEAFE">{sub}</text>

{body}
</svg>
"##,
        w = CARD_W,
        h = h,
        pad = PAD,
        iw = inner_w,
        ih = inner_h,
        cx = w / 2.0,
        t1 = PAD + 38.0,
        t2 = PAD + 82.0,
        t3 = PAD + 116.0,
        ff = font(),
        title = esc(&card.title),
        sub = esc(&card.subtitle),
        body = body,
    )
}

/// Load fixture JSON (array of orders).
pub fn load_fixture(path: &Path) -> Result<Vec<NotifyOrder>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::Other(format!("read fixture {}: {e}", path.display())))?;
    serde_json::from_str(&raw).map_err(|e| Error::Other(format!("parse fixture: {e}")))
}

/// Render notify PNG from orders (fetches thumbs).
pub async fn render_notify_png(orders: Vec<NotifyOrder>) -> Result<Vec<u8>> {
    let card = NotifyCard::from_orders(orders);
    let urls: Vec<String> = card
        .orders
        .iter()
        .flat_map(|o| o.items.iter())
        .filter_map(|i| i.image_url.clone())
        .collect();
    let thumbs = fetch_thumb_data_uris(&urls).await;
    let svg = card_to_svg(&card, &thumbs);
    daily_report::svg_to_png(&svg)
}

pub fn default_sample_fixture() -> PathBuf {
    PathBuf::from("examples/fixtures/instant-notify-sample.json")
}

pub fn default_png_out() -> PathBuf {
    PathBuf::from("logs/instant-notify-sample.png")
}

pub fn write_sample_png(path: &Path, png: &[u8]) -> Result<()> {
    write_png(path, png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_svg_renders_blue_and_names() {
        let orders = vec![NotifyOrder {
            order_id: Some(1),
            platform_order_id: "26072195X9S7EJ".into(),
            platform: "shopee".into(),
            carrier: Some("SPX Instant".into()),
            is_urgent: Some(true),
            ordered_at_wib: Some("2026-07-21 20:47:04 WIB".into()),
            state: Some("new".into()),
            items: vec![
                NotifyItem {
                    sku: Some("OB-0136-LB".into()),
                    name: Some("Obayito Singlet".into()),
                    variant_attr: Some("LB".into()),
                    image_url: None,
                    quantity: 2,
                },
                NotifyItem {
                    sku: Some("0B-0134-S-KMRI".into()),
                    name: Some("0B-0134-S".into()),
                    variant_attr: Some("Nemuri,S".into()),
                    image_url: None,
                    quantity: 1,
                },
            ],
        }];
        let card = NotifyCard::from_orders(orders);
        let svg = card_to_svg(&card, &HashMap::new());
        assert!(svg.contains("Pesanan Instant"));
        assert!(svg.contains("#2563EB") || svg.contains("#1D4ED8"));
        assert!(svg.contains("Tencel Piyama") || svg.contains("Piyama Panjang"));
        assert!(svg.contains("Obayito Singlet"));
        // primary name should not be the bare SKU code as title for tencel line
        assert!(!svg.contains(">0B-0134-S<"));
        let png = daily_report::svg_to_png(&svg).expect("png");
        assert!(png.starts_with(b"\x89PNG"));
    }
}
