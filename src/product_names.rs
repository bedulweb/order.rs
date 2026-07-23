//! Display names for packing / Summary List PDFs.
//!
//! Prefer internal series maps and catalog ART over long marketplace titles.
//! HPP is intentionally not used here (dashboard/report only).

use std::collections::HashMap;

/// Piyama / Obytobi series — highest priority for those SKU families.
const SERIES_MAP: &[(&str, &str)] = &[
    ("MB-042", "Miabebo Piyama Classic"),
    ("MB-043", "Miabebo Piyama Outline"),
    ("MB-044", "Miabebo Piyama Inline"),
    ("OB-0124", "Obayito Piyama Kerah"),
    ("OB-0125", "Obayito Piyama Outline"),
    ("OB-0126", "Obayito Piyama Inline"),
    ("OB-0124B", "Obytobi Piyama Kerah"),
    ("OB-0125B", "Obytobi Piyama Outline"),
    ("OB-0126B", "Obytobi Piyama Inline"),
    // Marketplace often stores as `0B-0133-…` (zero) — normalized to OB-.
    ("OB-0133", "Obayito Tencel Piyama Panjang"),
    ("OB-0134", "Obayito Tencel Piyama Panjang"),
];

/// Tencel pillow/bolster SKUs → plain Mimi names (not marketplace TENCEL titles).
const MIMI_T_MAP: &[(&str, &str)] = &[
    ("OB-001T", "Obayito Bantal Mimi Pillow"),
    ("OB-021T", "Obayito Bantal Mimi Pillow"),
    ("OB-003T", "Obayito Guling Mimi Bolster"),
    ("OB-023T", "Obayito Guling Mimi Bolster"),
];

/// Normalize ART / SKU for matching (uppercase, trim).
/// Only collapses Obytobi `…-B` into `…B` (not size letters like `-M` / `-L`).
/// e.g. `OB-0125-B-L-HITM` → `OB-0125B-L-HITM`
///
/// Also fixes marketplace typo `0B-…` → `OB-…` (leading zero instead of letter O).
pub fn normalize_art(raw: &str) -> String {
    let mut s = raw.trim().to_uppercase().replace(' ', "");
    if s.is_empty() {
        return s;
    }
    // Marketplace typo: leading zero instead of letter O (`0B-0133` → `OB-0133`).
    if let Some(rest) = s.strip_prefix("0B") {
        s = format!("OB{rest}");
    }
    let parts: Vec<&str> = s.split('-').collect();
    let mut joined: Vec<String> = Vec::with_capacity(parts.len());
    let mut idx = 0;
    while idx < parts.len() {
        if idx + 1 < parts.len()
            && parts[idx]
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_digit())
            && parts[idx + 1] == "B"
        {
            joined.push(format!("{}B", parts[idx]));
            idx += 2;
        } else {
            joined.push(parts[idx].to_string());
            idx += 1;
        }
    }
    joined.join("-")
}

/// Color is usually the last hyphen segment when it looks like a color code (letters, ≥3).
pub fn strip_color_segment(sku: &str) -> String {
    let s = normalize_art(sku);
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() < 3 {
        return s;
    }
    let last = parts[parts.len() - 1];
    let is_color = last.len() >= 3
        && last.chars().all(|c| c.is_ascii_alphabetic())
        && !last.chars().any(|c| c.is_ascii_digit());
    if is_color {
        parts[..parts.len() - 1].join("-")
    } else {
        s
    }
}

fn prefix_match<'a>(sku_n: &str, key: &'a str) -> bool {
    sku_n == key || sku_n.starts_with(&format!("{key}-"))
}

fn series_name(sku_n: &str) -> Option<&'static str> {
    // Prefer longer keys first (OB-0125B before OB-0125)
    let mut keys: Vec<_> = SERIES_MAP.iter().collect();
    keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (k, name) in keys {
        if prefix_match(sku_n, k) {
            return Some(*name);
        }
    }
    None
}

fn mimi_t_name(sku_n: &str) -> Option<&'static str> {
    for (k, name) in MIMI_T_MAP {
        if prefix_match(sku_n, k) {
            return Some(*name);
        }
    }
    None
}

fn longest_catalog_prefix<'a>(
    sku_n: &str,
    catalog: &'a HashMap<String, String>,
) -> Option<&'a str> {
    let candidates = [sku_n.to_string(), strip_color_segment(sku_n)];
    let mut best: Option<(&str, usize)> = None;
    for probe in &candidates {
        for (art, name) in catalog {
            // Catalog map keys are normalized at insert time.
            if probe == art || probe.starts_with(&format!("{art}-")) {
                let len = art.len();
                if best.map(|(_, l)| len > l).unwrap_or(true) {
                    best = Some((name.as_str(), len));
                }
            }
        }
    }
    best.map(|(n, _)| n)
}

fn is_bad_marketplace_title(s: &str) -> bool {
    let u = s.to_ascii_uppercase();
    u.contains("TENCEL") || s.len() > 55 || (u.contains("PREMIUM") && s.len() > 40)
}

/// Resolve packing display name for one line.
pub fn resolve_display_name(
    sku: &str,
    item_name: Option<&str>,
    catalog: &HashMap<String, String>,
) -> String {
    let sku_n = normalize_art(sku);
    if sku_n.is_empty() {
        return item_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(no sku)")
            .to_string();
    }

    if let Some(n) = series_name(&sku_n) {
        return n.to_string();
    }
    if let Some(n) = mimi_t_name(&sku_n) {
        return n.to_string();
    }
    if let Some(n) = longest_catalog_prefix(&sku_n, catalog) {
        return n.to_string();
    }

    if let Some(raw) = item_name.map(str::trim).filter(|s| !s.is_empty()) {
        if !is_bad_marketplace_title(raw) {
            return raw.to_string();
        }
    }

    let stripped = strip_color_segment(&sku_n);
    if !stripped.is_empty() {
        return stripped;
    }
    sku_n
}

/// Build catalog lookup from ART → name rows (keys normalized).
pub fn catalog_map_from_pairs(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (art, name) in pairs {
        let k = normalize_art(&art);
        if !k.is_empty() && !name.trim().is_empty() {
            m.insert(k, name.trim().to_string());
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        catalog_map_from_pairs(pairs.iter().map(|(a, n)| (a.to_string(), n.to_string())))
    }

    #[test]
    fn piyama_series_and_obytobi_b() {
        let c = cat(&[]);
        assert_eq!(
            resolve_display_name("MB-043-3XL-PBLP", None, &c),
            "Miabebo Piyama Outline"
        );
        assert_eq!(
            resolve_display_name("OB-0125B-L-HITM", None, &c),
            "Obytobi Piyama Outline"
        );
        assert_eq!(
            resolve_display_name("OB-0125-B-L-HITM", None, &c),
            "Obytobi Piyama Outline"
        );
    }

    #[test]
    fn mimi_t_not_tencel_title() {
        let c = cat(&[]);
        let long = "Bantal Bayi TENCEL™ 100% Premium Soft";
        assert_eq!(
            resolve_display_name("OB-023T-2M-KMRI", Some(long), &c),
            "Obayito Guling Mimi Bolster"
        );
        assert_eq!(
            resolve_display_name("OB-021T-1M-ABU", Some(long), &c),
            "Obayito Bantal Mimi Pillow"
        );
    }

    #[test]
    fn catalog_prefix_beats_long_item_name() {
        let c = cat(&[("OB-099", "Obayito Special Catalog")]);
        assert_eq!(
            resolve_display_name("OB-099-M-MERH", Some("Long marketplace junk TENCEL"), &c),
            "Obayito Special Catalog"
        );
    }

    #[test]
    fn never_use_empty_as_sku_fallback() {
        let c = cat(&[]);
        assert_eq!(resolve_display_name("XYZ-1", None, &c), "XYZ-1");
    }

    #[test]
    fn zero_b_typo_and_tencel_piyama() {
        let c = cat(&[]);
        assert_eq!(normalize_art("0B-0134-S-KMRI"), "OB-0134-S-KMRI");
        assert_eq!(
            resolve_display_name("0B-0134-S-KMRI", None, &c),
            "Obayito Tencel Piyama Panjang"
        );
        assert_eq!(
            resolve_display_name("0B-0133-S-KMRI", None, &c),
            "Obayito Tencel Piyama Panjang"
        );
    }
}
