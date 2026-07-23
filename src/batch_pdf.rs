//! Summary List PDF for ops batches (BigSeller-style pick sheet).
//!
//! Layout:
//! - Header: **Summary List** (left) + `N pesanan | N barang | N SKU` (right)
//! - Rows: thumb | name + qty | `SKU: …` + bold variant | `*XXXX : qty`
//! - Footer every page: `Dicetak: DD/MM/YYYY HH:MM WIB` + `Hal. N`
//!
//! No HPP. Thumbs fetched concurrently (best-effort; missing → empty box).

use crate::batch::{BatchSession, PdfOrderLine};
use crate::error::{Error, Result};
use printpdf::image_crate::{self, DynamicImage};
use printpdf::path::{PaintMode, WindingOrder};
use printpdf::{
    BuiltinFont, Color, Image, ImageTransform, Line, Mm, PdfDocument, PdfDocumentReference,
    PdfLayerReference, Point, Polygon, Rgb,
};
use std::collections::{BTreeMap, HashMap};
use std::io::BufWriter;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN: f32 = 14.0;
const FOOTER_Y: f32 = 12.0;
const CONTENT_BOTTOM: f32 = 20.0;
const QTY_COL_W: f32 = 16.0;
/// Larger thumbs so product photos stay readable on paper (esp. pale / white items).
const THUMB_MM: f32 = 18.0;
const THUMB_GAP: f32 = 3.5;
const THUMB_PAD_MM: f32 = 0.6;
const GAP_AFTER_DIV: f32 = 3.2;
const GAP_BEFORE_DIV: f32 = 2.8;
const THUMB_FETCH_CONCURRENCY: usize = 12;
const THUMB_TIMEOUT: Duration = Duration::from_secs(8);
const THUMB_PX: u32 = 144;

/// One aggregated SKU row on the Summary List.
#[derive(Debug, Clone)]
pub struct SummarySkuRow {
    pub sku: String,
    pub name: String,
    pub variant: String,
    pub qty: i32,
    pub image_url: Option<String>,
    /// package code `*last4` → qty
    pub packages: BTreeMap<String, i32>,
}

/// Aggregate order lines into SKU rows (full SKU key; empty sku falls back to name).
pub fn aggregate_summary_rows(lines: &[PdfOrderLine]) -> Vec<SummarySkuRow> {
    let mut map: BTreeMap<String, SummarySkuRow> = BTreeMap::new();

    for line in lines {
        let code = package_code(&line.platform_order_id);
        for it in &line.items {
            let sku = it
                .sku
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("-")
                .to_string();
            let name = it
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(no name)")
                .to_string();
            let variant = it
                .variant_attr
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let image_url = it
                .image_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let qty = it.quantity.max(0);
            if qty == 0 {
                continue;
            }
            let key = if sku == "-" {
                format!("noname:{name}")
            } else {
                sku.clone()
            };
            let entry = map.entry(key).or_insert_with(|| SummarySkuRow {
                sku: sku.clone(),
                name: name.clone(),
                variant: variant.clone(),
                qty: 0,
                image_url: image_url.clone(),
                packages: BTreeMap::new(),
            });
            entry.qty += qty;
            *entry.packages.entry(code.clone()).or_insert(0) += qty;
            if entry.name == "(no name)" && name != "(no name)" {
                entry.name = name;
            }
            if entry.variant.is_empty() && !variant.is_empty() {
                entry.variant = variant;
            }
            if entry.image_url.is_none() {
                entry.image_url = image_url;
            }
        }
    }

    let mut rows: Vec<SummarySkuRow> = map.into_values().collect();
    rows.sort_by(|a, b| b.qty.cmp(&a.qty).then_with(|| a.sku.cmp(&b.sku)));
    rows
}

pub fn package_code(platform_order_id: &str) -> String {
    let s = platform_order_id.trim();
    let tail = if s.len() >= 4 { &s[s.len() - 4..] } else { s };
    format!("*{tail}")
}

fn approx_text_width_pt(text: &str, size: f32, bold: bool) -> f32 {
    let factor = if bold { 0.55 } else { 0.50 };
    text.chars().count() as f32 * size * factor
}

fn trunc_to_width(text: &str, size: f32, bold: bool, max_mm: f32) -> String {
    let max_pt = max_mm * 72.0 / 25.4;
    if approx_text_width_pt(text, size, bold) <= max_pt {
        return text.to_string();
    }
    let mut t: String = text.chars().collect();
    while !t.is_empty() && approx_text_width_pt(&format!("{t}..."), size, bold) > max_pt {
        t.pop();
    }
    if t.is_empty() {
        "...".into()
    } else {
        format!("{t}...")
    }
}

fn wrap_parts(parts: &[String], size: f32, max_mm: f32) -> Vec<String> {
    if parts.is_empty() {
        return Vec::new();
    }
    let max_pt = max_mm * 72.0 / 25.4;
    let mut lines = Vec::new();
    let mut cur = String::new();
    for p in parts {
        let piece = if cur.is_empty() {
            p.clone()
        } else {
            format!("{cur}    {p}")
        };
        if approx_text_width_pt(&piece, size, false) <= max_pt {
            cur = piece;
        } else {
            if !cur.is_empty() {
                lines.push(cur);
            }
            cur = p.clone();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() {
                c
            } else if c == '—' || c == '–' || c == '·' {
                '-'
            } else if c == '“' || c == '”' || c == '„' {
                '"'
            } else if c == '‘' || c == '’' {
                '\''
            } else if c == '…' {
                '.'
            } else {
                '?'
            }
        })
        .collect()
}

pub fn footer_print_stamp(created_wib: &str) -> String {
    let s = created_wib.trim();
    if s.len() >= 16 {
        let date = &s[0..10];
        let time = &s[11..16];
        if let Some((y, rest)) = date.split_once('-') {
            if let Some((m, d)) = rest.split_once('-') {
                return format!("{d}/{m}/{y} {time} WIB");
            }
        }
    }
    s.to_string()
}

/// Decode image bytes to a small RGB thumb suitable for PDF embed
/// (uses printpdf's bundled `image` 0.24).
fn decode_thumb(bytes: &[u8]) -> Option<DynamicImage> {
    let img = image_crate::load_from_memory(bytes).ok()?;
    let thumb = img.thumbnail(THUMB_PX, THUMB_PX);
    Some(DynamicImage::ImageRgb8(thumb.to_rgb8()))
}

/// Concurrent best-effort fetch of unique image URLs → RGB thumbs.
async fn fetch_thumbs(urls: &[String]) -> HashMap<String, DynamicImage> {
    let client = match reqwest::Client::builder()
        .timeout(THUMB_TIMEOUT)
        .user_agent("orders-summary-list/1.0")
        .build()
    {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::warn!(error = %e, "thumb http client");
            return HashMap::new();
        }
    };

    let mut unique: Vec<String> = urls
        .iter()
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .cloned()
        .collect();
    unique.sort();
    unique.dedup();

    let mut map = HashMap::new();
    let mut ok = 0usize;
    // Chunked concurrency without extra futures deps.
    for chunk in unique.chunks(THUMB_FETCH_CONCURRENCY) {
        let mut handles = Vec::with_capacity(chunk.len());
        for url in chunk {
            let client = Arc::clone(&client);
            let url = url.clone();
            handles.push(tokio::spawn(async move {
                let img = match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                        Ok(b) => decode_thumb(&b),
                        Err(_) => None,
                    },
                    _ => None,
                };
                (url, img)
            }));
        }
        for h in handles {
            if let Ok((url, Some(img))) = h.await {
                map.insert(url, img);
                ok += 1;
            }
        }
    }
    tracing::info!(fetched = ok, "summary list thumbs");
    map
}

struct PageState {
    page: printpdf::PdfPageIndex,
    layer: printpdf::PdfLayerIndex,
    y: f32,
}

fn layer_of<'a>(doc: &'a PdfDocumentReference, st: &PageState) -> PdfLayerReference {
    doc.get_page(st.page).get_layer(st.layer)
}

fn hline(layer: &PdfLayerReference, x0: f32, x1: f32, y: f32, gray: f32) {
    layer.set_outline_color(printpdf::Color::Rgb(printpdf::Rgb::new(
        gray, gray, gray, None,
    )));
    layer.set_outline_thickness(0.5);
    let line = Line {
        points: vec![
            (Point::new(Mm(x0), Mm(y)), false),
            (Point::new(Mm(x1), Mm(y)), false),
        ],
        is_closed: false,
    };
    layer.add_line(line);
}

fn text_at(
    layer: &PdfLayerReference,
    font: &printpdf::IndirectFontRef,
    s: &str,
    size: f32,
    x: f32,
    y: f32,
) {
    layer.use_text(sanitize(s), size, Mm(x), Mm(y), font);
}

fn text_right(
    layer: &PdfLayerReference,
    font: &printpdf::IndirectFontRef,
    s: &str,
    size: f32,
    right_x: f32,
    y: f32,
    bold: bool,
) {
    let w_mm = approx_text_width_pt(s, size, bold) * 25.4 / 72.0;
    text_at(layer, font, s, size, right_x - w_mm, y);
}

fn thumb_rect_points(x: f32, bottom: f32) -> Vec<(Point, bool)> {
    vec![
        (Point::new(Mm(x), Mm(bottom)), false),
        (Point::new(Mm(x + THUMB_MM), Mm(bottom)), false),
        (Point::new(Mm(x + THUMB_MM), Mm(bottom + THUMB_MM)), false),
        (Point::new(Mm(x), Mm(bottom + THUMB_MM)), false),
    ]
}

/// Light plate + border so pale product photos don't disappear on white paper.
fn draw_thumb_plate(layer: &PdfLayerReference, x: f32, bottom: f32) {
    layer.set_fill_color(Color::Rgb(Rgb::new(0.93, 0.93, 0.94, None)));
    layer.set_outline_color(Color::Rgb(Rgb::new(0.55, 0.55, 0.58, None)));
    layer.set_outline_thickness(0.7);
    layer.add_polygon(Polygon {
        rings: vec![thumb_rect_points(x, bottom)],
        mode: PaintMode::FillStroke,
        winding_order: WindingOrder::NonZero,
    });
}

fn draw_thumb_placeholder(layer: &PdfLayerReference, x: f32, bottom: f32) {
    draw_thumb_plate(layer, x, bottom);
}

/// Fit RGB thumb into the plate (letterbox), keeping aspect ratio.
fn place_thumb(layer: &PdfLayerReference, dyn_img: &DynamicImage, x: f32, bottom: f32) {
    draw_thumb_plate(layer, x, bottom);

    let pdf_img = Image::from_dynamic_image(dyn_img);
    let px_w = dyn_img.width().max(1) as f32;
    let px_h = dyn_img.height().max(1) as f32;
    // printpdf: at dpi=72, 1px → 1pt before scale_x/y.
    let inner = (THUMB_MM - 2.0 * THUMB_PAD_MM).max(4.0);
    let target_pt = inner * 72.0 / 25.4;
    let scale = (target_pt / px_w).min(target_pt / px_h);
    let drawn_w_mm = px_w * scale * 25.4 / 72.0;
    let drawn_h_mm = px_h * scale * 25.4 / 72.0;
    let ox = x + (THUMB_MM - drawn_w_mm) / 2.0;
    let oy = bottom + (THUMB_MM - drawn_h_mm) / 2.0;
    pdf_img.add_to_layer(
        layer.clone(),
        ImageTransform {
            translate_x: Some(Mm(ox)),
            translate_y: Some(Mm(oy)),
            rotate: None,
            scale_x: Some(scale),
            scale_y: Some(scale),
            dpi: Some(72.0),
        },
    );
}

/// Render Summary List PDF bytes (async: fetches product thumbs).
pub async fn render_batch_pdf(
    batch_id: Uuid,
    session: BatchSession,
    created_wib: &str,
    order_count: i32,
    _urgent_count: i32,
    lines: &[PdfOrderLine],
) -> Result<Vec<u8>> {
    let rows = aggregate_summary_rows(lines);
    let urls: Vec<String> = rows.iter().filter_map(|r| r.image_url.clone()).collect();
    let thumbs = fetch_thumbs(&urls).await;

    let n_sku = rows.len() as i32;
    let n_barang: i32 = rows.iter().map(|r| r.qty).sum();
    let print_stamp = footer_print_stamp(created_wib);
    let meta = format!("{order_count} pesanan  |  {n_barang} barang  |  {n_sku} SKU");
    let title = "Summary List";
    let _ = (batch_id, session);

    let (doc, page1, layer1) = PdfDocument::new("Summary List", Mm(PAGE_W), Mm(PAGE_H), "Layer 1");
    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| Error::Other(format!("pdf font: {e}")))?;
    let font_bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| Error::Other(format!("pdf font bold: {e}")))?;

    let mut pages: Vec<PageState> = vec![PageState {
        page: page1,
        layer: layer1,
        y: PAGE_H - MARGIN,
    }];

    let text_right_x = PAGE_W - MARGIN - QTY_COL_W;
    let tx = MARGIN + THUMB_MM + THUMB_GAP;
    let max_name_w = text_right_x - tx - 2.0;

    let draw_header = |doc: &PdfDocumentReference,
                       st: &mut PageState,
                       font: &printpdf::IndirectFontRef,
                       font_bold: &printpdf::IndirectFontRef,
                       title: &str,
                       meta: &str| {
        let layer = layer_of(doc, st);
        text_at(&layer, font_bold, title, 14.0, MARGIN, st.y);
        text_right(&layer, font, meta, 9.0, PAGE_W - MARGIN, st.y, false);
        st.y -= 5.5;
        hline(&layer, MARGIN, PAGE_W - MARGIN, st.y, 0.45);
        st.y -= GAP_AFTER_DIV + 1.0;
    };

    let draw_footer = |doc: &PdfDocumentReference,
                       st: &PageState,
                       page_no: usize,
                       font: &printpdf::IndirectFontRef,
                       stamp: &str| {
        let layer = layer_of(doc, st);
        hline(&layer, MARGIN, PAGE_W - MARGIN, FOOTER_Y + 5.0, 0.75);
        text_at(
            &layer,
            font,
            &format!("Dicetak: {stamp}"),
            8.0,
            MARGIN,
            FOOTER_Y,
        );
        text_right(
            &layer,
            font,
            &format!("Hal. {page_no}"),
            8.0,
            PAGE_W - MARGIN,
            FOOTER_Y,
            false,
        );
    };

    {
        let st = pages.last_mut().unwrap();
        draw_header(&doc, st, &font, &font_bold, title, &meta);
    }

    for row in &rows {
        let pkg_parts: Vec<String> = row
            .packages
            .iter()
            .map(|(code, q)| format!("{code} : {q}"))
            .collect();
        let pkg_lines = wrap_parts(&pkg_parts, 7.5, max_name_w);
        let n_pkg = pkg_lines.len();

        let text_h = 3.6 + 3.8 + (n_pkg as f32 * 3.2) + GAP_BEFORE_DIV;
        let content_h = text_h.max(THUMB_MM + 1.0);
        let row_h = content_h + GAP_AFTER_DIV;

        if pages.last().unwrap().y - row_h < CONTENT_BOTTOM {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Layer 1");
            pages.push(PageState {
                page: p,
                layer: l,
                y: PAGE_H - MARGIN,
            });
            let st = pages.last_mut().unwrap();
            draw_header(&doc, st, &font, &font_bold, title, &meta);
        }

        let st = pages.last_mut().unwrap();
        let layer = layer_of(&doc, st);
        let row_top = st.y;
        let img_bottom = row_top - THUMB_MM;
        let name_y = row_top - 2.5;

        // thumb (plate + photo; pale products need the plate or they vanish on white paper)
        let mut drew_img = false;
        if let Some(url) = row.image_url.as_ref() {
            if let Some(dyn_img) = thumbs.get(url) {
                place_thumb(&layer, dyn_img, MARGIN, img_bottom);
                drew_img = true;
            }
        }
        if !drew_img {
            draw_thumb_placeholder(&layer, MARGIN, img_bottom);
        }

        // name + qty
        let name = trunc_to_width(&row.name, 9.0, true, max_name_w);
        text_at(&layer, &font_bold, &name, 9.0, tx, name_y);
        text_right(
            &layer,
            &font_bold,
            &row.qty.to_string(),
            11.0,
            PAGE_W - MARGIN,
            name_y,
            true,
        );

        let mut y_text = name_y - 3.8;
        let sku_prefix = format!("SKU: {}", row.sku);
        if row.variant.is_empty() {
            let line = trunc_to_width(&sku_prefix, 8.0, false, max_name_w);
            text_at(&layer, &font, &line, 8.0, tx, y_text);
        } else {
            let prefix = format!("{sku_prefix}   ");
            let prefix_w = approx_text_width_pt(&prefix, 8.0, false) * 25.4 / 72.0;
            text_at(&layer, &font, &prefix, 8.0, tx, y_text);
            let var_max = (max_name_w - prefix_w).max(10.0);
            let var = trunc_to_width(&row.variant, 8.0, true, var_max);
            text_at(&layer, &font_bold, &var, 8.0, tx + prefix_w, y_text);
        }

        if !pkg_lines.is_empty() {
            y_text -= 3.6;
            for pl in &pkg_lines {
                text_at(&layer, &font, pl, 7.5, tx, y_text);
                y_text -= 3.2;
            }
            y_text += 3.2;
        }

        let content_bottom = y_text.min(img_bottom) - 1.5;
        let div_y = content_bottom - GAP_BEFORE_DIV;
        hline(&layer, MARGIN, PAGE_W - MARGIN, div_y, 0.88);
        st.y = div_y - GAP_AFTER_DIV;
    }

    for (i, st) in pages.iter().enumerate() {
        draw_footer(&doc, st, i + 1, &font, &print_stamp);
    }

    let mut buf = BufWriter::new(Vec::new());
    doc.save(&mut buf)
        .map_err(|e| Error::Other(format!("pdf save: {e}")))?;
    buf.into_inner()
        .map_err(|e| Error::Other(format!("pdf buffer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::BatchLineItem;

    #[test]
    fn package_code_last4() {
        assert_eq!(package_code("260715PS7HRGC0"), "*RGC0");
        assert_eq!(package_code("AB"), "*AB");
    }

    #[test]
    fn footer_stamp_formats() {
        assert_eq!(
            footer_print_stamp("2026-07-22 08:15:00 WIB"),
            "22/07/2026 08:15 WIB"
        );
    }

    #[test]
    fn aggregate_sums_by_sku_and_packages() {
        let lines = vec![
            PdfOrderLine {
                platform_order_id: "AAAABBBB".into(),
                platform: "shopee".into(),
                carrier: "JNE".into(),
                is_urgent: false,
                ordered_at_wib: "-".into(),
                items: vec![BatchLineItem {
                    sku: Some("SKU-1".into()),
                    name: Some("Widget".into()),
                    variant_attr: Some("Merah,M".into()),
                    image_url: Some("https://example.com/a.jpg".into()),
                    quantity: 2,
                }],
            },
            PdfOrderLine {
                platform_order_id: "XXXXYYYY".into(),
                platform: "tiktok".into(),
                carrier: "JNE".into(),
                is_urgent: false,
                ordered_at_wib: "-".into(),
                items: vec![BatchLineItem {
                    sku: Some("SKU-1".into()),
                    name: Some("Widget".into()),
                    variant_attr: Some("Merah,M".into()),
                    image_url: None,
                    quantity: 1,
                }],
            },
        ];
        let rows = aggregate_summary_rows(&lines);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].qty, 3);
        assert_eq!(rows[0].packages.get("*BBBB"), Some(&2));
        assert_eq!(rows[0].packages.get("*YYYY"), Some(&1));
        assert_eq!(rows[0].variant, "Merah,M");
        assert_eq!(
            rows[0].image_url.as_deref(),
            Some("https://example.com/a.jpg")
        );
    }

    #[tokio::test]
    async fn render_pdf_is_nonempty_pdf_header() {
        let lines = vec![
            PdfOrderLine {
                platform_order_id: "260715PS7HRGC0".into(),
                platform: "shopee".into(),
                carrier: "SPX Instant".into(),
                is_urgent: true,
                ordered_at_wib: "2026-07-22 08:00:00 WIB".into(),
                items: vec![BatchLineItem {
                    sku: Some("MB-043-3XL-PBLP".into()),
                    name: Some("Miabebo Piyama Outline".into()),
                    variant_attr: Some("3XL, Peach Blush Pink".into()),
                    image_url: None,
                    quantity: 2,
                }],
            },
            PdfOrderLine {
                platform_order_id: "TT123456789012345678".into(),
                platform: "tiktok".into(),
                carrier: "JNE REG".into(),
                is_urgent: false,
                ordered_at_wib: "2026-07-22 07:00:00 WIB".into(),
                items: vec![BatchLineItem {
                    sku: Some("OB-023T-2M-KMRI".into()),
                    name: Some("Obayito Guling Mimi Bolster".into()),
                    variant_attr: Some("2M, Kamari".into()),
                    image_url: None,
                    quantity: 1,
                }],
            },
        ];
        let bytes = render_batch_pdf(
            Uuid::nil(),
            BatchSession::Morning,
            "2026-07-22 08:15:00 WIB",
            2,
            1,
            &lines,
        )
        .await
        .expect("pdf");
        assert!(bytes.len() > 200);
        assert!(bytes.starts_with(b"%PDF"));
        let as_str = String::from_utf8_lossy(&bytes);
        assert!(
            as_str.contains("Summary") || bytes.windows(7).any(|w| w == b"Summary"),
            "expected Summary List title in PDF"
        );
    }
}
