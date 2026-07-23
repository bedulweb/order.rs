//! Screenshot OCR (Shopee / WhatsApp order details) via [`ocrs`].
//!
//! Separate from [`crate::ocr::CaptchaOcr`] (ddddocr captcha for BigSeller login).

use crate::error::{Error, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::info;

const DETECTION_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten";
const RECOGNITION_URL: &str =
    "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten";

/// Default cache dir: `$HOME/.cache/ocrs` (same as `ocrs-cli`).
pub fn default_model_cache_dir() -> PathBuf {
    let mut dir = dirs_next_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("ocrs");
    if let Ok(override_dir) = std::env::var("OCRS_CACHE_DIR") {
        if !override_dir.is_empty() {
            dir = PathBuf::from(override_dir);
        }
    }
    dir
}

fn dirs_next_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn ensure_model(cache: &Path, filename: &str, url: &str) -> Result<PathBuf> {
    fs::create_dir_all(cache).map_err(|e| Error::Ocr(format!("ocrs cache dir: {e}")))?;
    let path = cache.join(filename);
    if path.is_file() {
        return Ok(path);
    }
    info!(%url, path = %path.display(), "downloading ocrs model");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Ocr(format!("http client: {e}")))?;
    let bytes = client
        .get(url)
        .send()
        .map_err(|e| Error::Ocr(format!("download {filename}: {e}")))?
        .error_for_status()
        .map_err(|e| Error::Ocr(format!("download {filename}: {e}")))?
        .bytes()
        .map_err(|e| Error::Ocr(format!("download body {filename}: {e}")))?;
    let tmp = path.with_extension("rten.part");
    fs::write(&tmp, &bytes).map_err(|e| Error::Ocr(format!("write model: {e}")))?;
    fs::rename(&tmp, &path).map_err(|e| Error::Ocr(format!("rename model: {e}")))?;
    Ok(path)
}

/// Load detection + recognition models (cached under [`default_model_cache_dir`]).
pub fn load_engine() -> Result<OcrEngine> {
    let cache = default_model_cache_dir();
    let det = ensure_model(&cache, "text-detection.rten", DETECTION_URL)?;
    let rec = ensure_model(&cache, "text-recognition.rten", RECOGNITION_URL)?;
    let detection_model =
        Model::load_file(&det).map_err(|e| Error::Ocr(format!("load detection model: {e}")))?;
    let recognition_model =
        Model::load_file(&rec).map_err(|e| Error::Ocr(format!("load recognition model: {e}")))?;
    OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })
    .map_err(|e| Error::Ocr(format!("ocrs engine: {e}")))
}

fn engine_cached() -> Result<&'static OcrEngine> {
    static ENGINE: OnceLock<std::result::Result<OcrEngine, String>> = OnceLock::new();
    let slot = ENGINE.get_or_init(|| load_engine().map_err(|e| e.to_string()));
    match slot {
        Ok(e) => Ok(e),
        Err(msg) => Err(Error::Ocr(msg.clone())),
    }
}

/// OCR an image file; returns non-empty text lines (order roughly top→bottom).
pub fn ocr_image_lines(path: &Path) -> Result<Vec<String>> {
    let img = image::open(path)
        .map_err(|e| Error::Ocr(format!("open image: {e}")))?
        .into_rgb8();
    ocr_rgb8_lines(&img)
}

/// OCR raw image bytes (JPEG/PNG/WebP/…); returns non-empty text lines.
pub fn ocr_image_bytes(bytes: &[u8]) -> Result<Vec<String>> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| Error::Ocr(format!("decode image: {e}")))?
        .into_rgb8();
    ocr_rgb8_lines(&img)
}

fn ocr_rgb8_lines(img: &image::RgbImage) -> Result<Vec<String>> {
    let engine = engine_cached()?;
    let src = ImageSource::from_bytes(img.as_raw(), img.dimensions())
        .map_err(|e| Error::Ocr(format!("image source: {e}")))?;
    let input = engine
        .prepare_input(src)
        .map_err(|e| Error::Ocr(format!("prepare input: {e}")))?;
    let text = engine
        .get_text(&input)
        .map_err(|e| Error::Ocr(format!("recognize: {e}")))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|s| s.len() > 1)
        .map(str::to_string)
        .collect())
}

/// Best-effort order id candidates from OCR lines (Shopee / marketplace screenshots).
///
/// Patterns from live `platform_order_id` (Postgres backfill sample):
/// - **Shopee** (~99.7%): 14 chars = `YYMMDD` + 8 alnum, uppercase
///   e.g. `2607206K6S67BG`. Rare: 19 chars.
/// - **TikTok** (100% in sample): exactly 18 digits, e.g. `585115262532748712`.
/// - **Not** order id: BigSeller `package_no` like `BS2798049528`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderIdHit {
    pub id: String,
    pub kind: OrderIdKind,
    /// True when found next to a "No. Pesanan" / "Nomor pesanan" label.
    pub labeled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderIdKind {
    /// Shopee-style: `YYMMDD` + alnum tail (usually 14 chars total).
    Alphanumeric,
    /// TikTok-style long numeric (usually exactly 18 digits).
    NumericLong,
}

/// Extract order-number candidates from already-OCR'd lines.
pub fn extract_order_ids(lines: &[String]) -> Vec<OrderIdHit> {
    let mut hits: Vec<OrderIdHit> = Vec::new();
    let mut push = |id: String, kind: OrderIdKind, labeled: bool| {
        if id.len() < 10 {
            return;
        }
        // drop obvious non-order noise
        let lower = id.to_ascii_lowercase();
        if lower.contains("pesanan") || lower.contains("pengembalian") {
            return;
        }
        if let Some(existing) = hits.iter_mut().find(|h| h.id == id) {
            existing.labeled |= labeled;
            return;
        }
        hits.push(OrderIdHit { id, kind, labeled });
    };

    for (i, line) in lines.iter().enumerate() {
        let labeled = is_order_label(line);
        // same line: label + id
        for (raw, kind) in scan_ids(line) {
            push(normalize_order_id(&raw, kind), kind, labeled);
        }
        if labeled {
            let end = (i + 3).min(lines.len());
            for nearby in lines.iter().take(end).skip(i + 1) {
                for (raw, kind) in scan_ids(nearby) {
                    push(normalize_order_id(&raw, kind), kind, true);
                }
            }
            // label often appears *after* the value in ocrs reading order
            if let Some(prev) = i.checked_sub(1).and_then(|p| lines.get(p)) {
                for (raw, kind) in scan_ids(prev) {
                    push(normalize_order_id(&raw, kind), kind, true);
                }
            }
        }
    }

    // Prefer labeled hits first, then alphanumeric (platform ids), then long numeric.
    hits.sort_by(|a, b| {
        b.labeled
            .cmp(&a.labeled)
            .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
            .then_with(|| b.id.len().cmp(&a.id.len()))
    });
    hits
}

fn kind_rank(k: OrderIdKind) -> u8 {
    match k {
        OrderIdKind::Alphanumeric => 0,
        OrderIdKind::NumericLong => 1,
    }
}

fn is_order_label(s: &str) -> bool {
    let t = s.to_ascii_lowercase().replace(' ', "");
    t.contains("no.pesanan")
        || t.contains("nopesanan")
        || t.contains("nomorpesanan")
        || t.contains("orderid")
        || t.contains("orderno")
}

fn scan_ids(line: &str) -> Vec<(String, OrderIdKind)> {
    let mut out = Vec::new();
    // Prefer whitespace-separated tokens so "5841…779 TT" does not glue into one id.
    for tok in line.split_whitespace() {
        let t = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if t.is_empty() || is_bigseller_package_no(t) {
            continue;
        }
        if is_tiktok_style_id(t) {
            out.push((t.to_string(), OrderIdKind::NumericLong));
        } else if is_shopee_style_id(t) {
            out.push((t.to_string(), OrderIdKind::Alphanumeric));
        }
    }
    // Also scan unbroken line for ids with no spaces (OCR sometimes merges label+value).
    for m in regex_digit_ids(line) {
        out.push((m, OrderIdKind::NumericLong));
    }
    for m in regex_alpha_ids(line) {
        if !is_bigseller_package_no(&m) {
            out.push((m, OrderIdKind::Alphanumeric));
        }
    }
    out
}

/// BigSeller internal package id (`package_no`), not marketplace order no.
fn is_bigseller_package_no(tok: &str) -> bool {
    let b = tok.as_bytes();
    b.len() >= 4
        && b[0].eq_ignore_ascii_case(&b'B')
        && b[1].eq_ignore_ascii_case(&b'S')
        && b[2..].iter().all(|c| c.is_ascii_digit())
}

/// TikTok `platform_order_id`: always 18 digits in backfill (allow 17–19 for OCR slip).
fn is_tiktok_style_id(tok: &str) -> bool {
    let len = tok.len();
    (17..=19).contains(&len) && tok.bytes().all(|c| c.is_ascii_digit())
}

fn regex_digit_ids(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (17..=19).contains(&len) {
                out.push(s[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

fn regex_alpha_ids(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let tok = &s[start..i];
            if is_shopee_style_id(tok) {
                out.push(tok.to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Shopee `platform_order_id` from DB (~5k sample):
/// - **14 chars** (dominant): `^\d{6}[A-Z0-9]{8}$`
/// - **19 chars** (rare): `^\d{6}[A-Z0-9]{13}$`
///
/// Tail may be letter-light (e.g. `250911833646S0`); length + prefix is enough.
fn is_shopee_style_id(tok: &str) -> bool {
    if is_bigseller_package_no(tok) {
        return false;
    }
    let b = tok.as_bytes();
    let n = b.len();
    // Canonical 14 / rare 19; allow 13–15 for single OCR insert/delete on the common form.
    if !matches!(n, 13 | 14 | 15 | 19) {
        return false;
    }
    if !b.iter().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    if n < 6 || !b[..6].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let letters = b.iter().filter(|c| c.is_ascii_alphabetic()).count();
    let digits = b.iter().filter(|c| c.is_ascii_digit()).count();
    // Must contain at least one letter (pure numeric → TikTok path).
    if letters == 0 {
        return false;
    }
    // Exact canonical lengths: trust structure (covers letter-light real ids).
    if n == 14 || n == 19 {
        return true;
    }
    // Off-by-one OCR: still require a bit of letter mix.
    if letters < 2 {
        return false;
    }
    if digits * 100 / n > 85 {
        return false;
    }
    true
}

/// Fix common OCR confusions on Shopee-style ids.
pub fn normalize_order_id(raw: &str, kind: OrderIdKind) -> String {
    let mut s = raw.trim().to_string();
    if kind == OrderIdKind::Alphanumeric {
        // DB stores uppercase; OCR often mixes case.
        s = s
            .chars()
            .map(|c| {
                if c.is_ascii_alphabetic() {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            })
            .collect();
        // Trailing O → 0 (e.g. …HRGCO → …HRGC0). Real trailing O is extremely rare in DB.
        if s.ends_with('O') {
            let head = &s[..s.len() - 1];
            if head.chars().filter(|c| c.is_ascii_digit()).count() >= 6 {
                s.pop();
                s.push('0');
            }
        }
    }
    s
}

/// OCR image + extract best order id (if any).
pub fn extract_order_id_from_image(path: &Path) -> Result<Option<OrderIdHit>> {
    let lines = ocr_image_lines(path)?;
    Ok(extract_order_ids(&lines).into_iter().next())
}

/// OCR image bytes + all order-id candidates (best first).
pub fn extract_order_ids_from_bytes(bytes: &[u8]) -> Result<Vec<OrderIdHit>> {
    let lines = ocr_image_bytes(bytes)?;
    Ok(extract_order_ids(&lines))
}

/// OCR image bytes + best order id (if any).
pub fn extract_order_id_from_bytes(bytes: &[u8]) -> Result<Option<OrderIdHit>> {
    Ok(extract_order_ids_from_bytes(bytes)?.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_shopee_platform_id_near_label() {
        let lines = vec![
            "Rincian Pesanan".into(),
            "260715PS7HRGC0".into(),
            "No. Pesanan".into(),
            "Salin".into(),
        ];
        let hits = extract_order_ids(&lines);
        assert_eq!(hits[0].id, "260715PS7HRGC0");
        assert!(hits[0].labeled);
        assert_eq!(hits[0].kind, OrderIdKind::Alphanumeric);
    }

    #[test]
    fn extracts_long_numeric_nomor_pesanan() {
        let lines = vec![
            "584161174119548779".into(),
            "Nomor pesanan".into(),
            "Total: Rp13.950".into(),
        ];
        let hits = extract_order_ids(&lines);
        assert_eq!(hits[0].id, "584161174119548779");
        assert!(hits[0].labeled);
        assert_eq!(hits[0].kind, OrderIdKind::NumericLong);
    }

    #[test]
    fn normalizes_trailing_o_to_zero() {
        assert_eq!(
            normalize_order_id("260715PS7HRGCO", OrderIdKind::Alphanumeric),
            "260715PS7HRGC0"
        );
    }

    #[test]
    fn ocr_typo_from_ocrs_cli_is_fixed() {
        let lines = vec!["260715PS7HRGCO".into(), "No. Pesanan".into()];
        let hits = extract_order_ids(&lines);
        assert_eq!(hits[0].id, "260715PS7HRGC0");
    }

    #[test]
    fn accepts_letter_light_shopee_from_db() {
        // Real row: only one letter in the tail — old rule (letters>=3) rejected these.
        let lines = vec!["250911833646S0".into(), "No. Pesanan".into()];
        let hits = extract_order_ids(&lines);
        assert_eq!(hits[0].id, "250911833646S0");
        assert_eq!(hits[0].kind, OrderIdKind::Alphanumeric);
    }

    #[test]
    fn accepts_tiktok_18_digit() {
        let lines = vec!["585115262532748712".into(), "Nomor pesanan".into()];
        let hits = extract_order_ids(&lines);
        assert_eq!(hits[0].id, "585115262532748712");
        assert_eq!(hits[0].kind, OrderIdKind::NumericLong);
    }

    #[test]
    fn rejects_bigseller_package_no() {
        let lines = vec!["BS2798049528".into(), "No. Pesanan".into()];
        let hits = extract_order_ids(&lines);
        assert!(hits.is_empty());
    }

    #[test]
    fn accepts_rare_shopee_19() {
        let lines = vec!["260305AASCRSCB374UU".into()];
        let hits = extract_order_ids(&lines);
        assert_eq!(hits[0].id, "260305AASCRSCB374UU");
    }
}
