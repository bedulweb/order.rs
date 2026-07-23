//! Product catalog: ART (SKU) → normalized name + HPP (IDR cost).
//!
//! Pure row normalize is separate from DB/HTTP so unit tests need no Postgres.
//! Import source of truth: workbook sheet `Produk Normalisasi`
//! (columns No, Nama Produk Normalisasi, ART, HPP).

use crate::error::{Error, Result};
use calamine::{open_workbook_auto_from_rs, Data, Reader, Sheets};
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

/// Primary import sheet name in the normalized marketplace workbook.
pub const PRIMARY_SHEET: &str = "Produk Normalisasi";

/// Default workbook path relative to process CWD / repo root.
pub const DEFAULT_WORKBOOK: &str = "MARKETPLACE_PRICE_2026_NORMALIZED.xlsx";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub art: String,
    pub name: String,
    /// Whole IDR (rupiah), no fractional part.
    pub hpp: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub inserted: i64,
    pub updated: i64,
    pub skipped: i64,
    pub total_rows: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductListResponse {
    pub total: i64,
    pub products: Vec<Product>,
}

/// Trim ART; empty after trim → None.
pub fn normalize_art(raw: &str) -> Option<String> {
    let art = raw.trim();
    if art.is_empty() {
        None
    } else {
        Some(art.to_string())
    }
}

/// Parse HPP as non-negative whole IDR.
/// Accepts plain integers, digit strings, optional trailing `.0` / `,0`.
pub fn parse_hpp(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() || s == "-" {
        return None;
    }
    // Strip thousand separators and currency noise.
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    // "329900.0" or "329900,0" → take integer part before decimal sep.
    let int_part = cleaned
        .split_once(['.', ','])
        .map(|(a, _)| a)
        .unwrap_or(cleaned.as_str());
    if int_part.is_empty() {
        return None;
    }
    let n: i64 = int_part.parse().ok()?;
    if n < 0 {
        None
    } else {
        Some(n)
    }
}

/// Normalize one logical catalog row. Returns None when ART or HPP invalid.
pub fn normalize_product_row(art: &str, name: &str, hpp: &str) -> Option<Product> {
    let art = normalize_art(art)?;
    let hpp = parse_hpp(hpp)?;
    Some(Product {
        art,
        name: name.trim().to_string(),
        hpp,
    })
}

/// Collapse rows by ART: last valid product wins (primary-sheet policy).
pub fn dedupe_by_art(rows: Vec<Product>) -> Vec<Product> {
    let mut map: HashMap<String, Product> = HashMap::new();
    for p in rows {
        map.insert(p.art.clone(), p);
    }
    let mut out: Vec<Product> = map.into_values().collect();
    out.sort_by(|a, b| a.art.cmp(&b.art));
    out
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            // Prefer integer display when value is whole.
            if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{e:?}"),
    }
}

/// Parse workbook bytes: sheet `Produk Normalisasi` only.
/// Returns (valid products after per-row normalize + ART dedupe, skipped invalid data rows).
pub fn parse_produk_normalisasi_bytes(bytes: &[u8]) -> Result<(Vec<Product>, i64)> {
    let cursor = Cursor::new(bytes);
    let mut workbook: Sheets<_> =
        open_workbook_auto_from_rs(cursor).map_err(|e| Error::Other(format!("open xlsx: {e}")))?;

    let range = workbook
        .worksheet_range(PRIMARY_SHEET)
        .map_err(|e| Error::Other(format!("sheet '{PRIMARY_SHEET}': {e}")))?;

    let mut products = Vec::new();
    let mut skipped = 0i64;
    let mut header_seen = false;
    let mut col_art: Option<usize> = None;
    let mut col_name: Option<usize> = None;
    let mut col_hpp: Option<usize> = None;

    for row in range.rows() {
        let cells: Vec<String> = row.iter().map(cell_to_string).collect();
        if !header_seen {
            // Detect header by column titles.
            for (i, c) in cells.iter().enumerate() {
                let t = c.trim().to_ascii_lowercase();
                if t == "art" {
                    col_art = Some(i);
                } else if (t.contains("nama") && t.contains("normalisasi"))
                    || t == "nama produk normalisasi"
                {
                    col_name = Some(i);
                } else if t == "hpp" {
                    col_hpp = Some(i);
                }
            }
            if col_art.is_some() && col_hpp.is_some() {
                header_seen = true;
                if col_name.is_none() {
                    col_name = Some(1); // B column default
                }
                continue;
            }
            // Not a header row — try fixed A=No B=Name C=ART D=HPP if first cells look like titles.
            continue;
        }

        let art_i = col_art.unwrap_or(2);
        let name_i = col_name.unwrap_or(1);
        let hpp_i = col_hpp.unwrap_or(3);
        let art = cells.get(art_i).map(|s| s.as_str()).unwrap_or("");
        let name = cells.get(name_i).map(|s| s.as_str()).unwrap_or("");
        let hpp = cells.get(hpp_i).map(|s| s.as_str()).unwrap_or("");

        // Skip fully empty trailing rows.
        if art.trim().is_empty() && name.trim().is_empty() && hpp.trim().is_empty() {
            continue;
        }

        match normalize_product_row(art, name, hpp) {
            Some(p) => products.push(p),
            None => skipped += 1,
        }
    }

    if !header_seen {
        return Err(Error::Other(format!(
            "sheet '{PRIMARY_SHEET}': header row with ART/HPP not found"
        )));
    }

    let products = dedupe_by_art(products);
    Ok((products, skipped))
}

pub fn parse_produk_normalisasi_path(path: &Path) -> Result<(Vec<Product>, i64)> {
    let bytes =
        std::fs::read(path).map_err(|e| Error::Other(format!("read {}: {e}", path.display())))?;
    parse_produk_normalisasi_bytes(&bytes)
}

/// Create `product_catalog` if missing (idempotent). Safe to call on every import/serve.
pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS product_catalog (
            art         TEXT PRIMARY KEY,
            name        TEXT NOT NULL DEFAULT '',
            hpp         BIGINT NOT NULL CHECK (hpp >= 0),
            created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(r#"CREATE INDEX IF NOT EXISTS product_catalog_name_idx ON product_catalog (name)"#)
        .execute(pool)
        .await?;
    sqlx::query(
        r#"CREATE INDEX IF NOT EXISTS product_catalog_updated_at_idx ON product_catalog (updated_at DESC)"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Upsert products in one transaction. Counts inserted vs updated by prior existence.
pub async fn upsert_products(pool: &PgPool, products: &[Product]) -> Result<ImportResult> {
    ensure_schema(pool).await?;
    let mut tx = pool.begin().await?;
    let mut inserted = 0i64;
    let mut updated = 0i64;

    for p in products {
        let existed: bool =
            sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM product_catalog WHERE art = $1)"#)
                .bind(&p.art)
                .fetch_one(&mut *tx)
                .await?;

        sqlx::query(
            r#"
            INSERT INTO product_catalog (art, name, hpp, created_at, updated_at)
            VALUES ($1, $2, $3, now(), now())
            ON CONFLICT (art) DO UPDATE SET
                name = EXCLUDED.name,
                hpp = EXCLUDED.hpp,
                updated_at = now()
            "#,
        )
        .bind(&p.art)
        .bind(&p.name)
        .bind(p.hpp)
        .execute(&mut *tx)
        .await?;

        if existed {
            updated += 1;
        } else {
            inserted += 1;
        }
    }

    tx.commit().await?;
    Ok(ImportResult {
        inserted,
        updated,
        skipped: 0,
        total_rows: products.len() as i64,
    })
}

/// Parse workbook + upsert. `skipped` includes invalid data rows from the sheet.
pub async fn import_from_bytes(pool: &PgPool, bytes: &[u8]) -> Result<ImportResult> {
    let (products, skipped) = parse_produk_normalisasi_bytes(bytes)?;
    let mut result = upsert_products(pool, &products).await?;
    result.skipped = skipped;
    result.total_rows = products.len() as i64 + skipped;
    Ok(result)
}

pub async fn import_from_path(pool: &PgPool, path: &Path) -> Result<ImportResult> {
    let bytes =
        std::fs::read(path).map_err(|e| Error::Other(format!("read {}: {e}", path.display())))?;
    import_from_bytes(pool, &bytes).await
}

/// Exact ART match after trim. Unknown ART → None (never error).
pub async fn lookup_by_art(pool: &PgPool, art: &str) -> Result<Option<Product>> {
    ensure_schema(pool).await?;
    let Some(art) = normalize_art(art) else {
        return Ok(None);
    };
    let row = sqlx::query(r#"SELECT art, name, hpp FROM product_catalog WHERE art = $1"#)
        .bind(&art)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|r| Product {
        art: r.get("art"),
        name: r.get("name"),
        hpp: r.get("hpp"),
    }))
}

pub async fn get_product(pool: &PgPool, art: &str) -> Result<Option<Product>> {
    lookup_by_art(pool, art).await
}

pub async fn list_products(
    pool: &PgPool,
    q: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<ProductListResponse> {
    ensure_schema(pool).await?;
    let limit = limit.clamp(1, 2000);
    let offset = offset.max(0);
    let q = q.map(str::trim).filter(|s| !s.is_empty());

    let (total, products) = if let Some(term) = q {
        let pattern = format!("%{}%", term);
        let total: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)::bigint FROM product_catalog
            WHERE art ILIKE $1 OR name ILIKE $1
            "#,
        )
        .bind(&pattern)
        .fetch_one(pool)
        .await?;

        let rows = sqlx::query(
            r#"
            SELECT art, name, hpp FROM product_catalog
            WHERE art ILIKE $1 OR name ILIKE $1
            ORDER BY art
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let products = rows
            .into_iter()
            .map(|r| Product {
                art: r.get("art"),
                name: r.get("name"),
                hpp: r.get("hpp"),
            })
            .collect();
        (total, products)
    } else {
        let total: i64 = sqlx::query_scalar(r#"SELECT COUNT(*)::bigint FROM product_catalog"#)
            .fetch_one(pool)
            .await?;

        let rows = sqlx::query(
            r#"
            SELECT art, name, hpp FROM product_catalog
            ORDER BY art
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let products = rows
            .into_iter()
            .map(|r| Product {
                art: r.get("art"),
                name: r.get("name"),
                hpp: r.get("hpp"),
            })
            .collect();
        (total, products)
    };

    Ok(ProductListResponse { total, products })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_art_trims_and_rejects_empty() {
        assert_eq!(normalize_art("  OB-001  ").as_deref(), Some("OB-001"));
        assert_eq!(normalize_art(""), None);
        assert_eq!(normalize_art("   "), None);
    }

    #[test]
    fn parse_hpp_accepts_workbook_shapes() {
        assert_eq!(parse_hpp("39900"), Some(39900));
        assert_eq!(parse_hpp("329900"), Some(329900));
        assert_eq!(parse_hpp("329900.0"), Some(329900));
        assert_eq!(parse_hpp(" 46900 "), Some(46900));
        assert_eq!(parse_hpp("-"), None);
        assert_eq!(parse_hpp(""), None);
        assert_eq!(parse_hpp("n/a"), None);
    }

    #[test]
    fn normalize_product_row_known_art_hpp() {
        let p = normalize_product_row("OB-001", "Obayito Sarung Bantal Bayi", "39900")
            .expect("valid row");
        assert_eq!(p.art, "OB-001");
        assert_eq!(p.hpp, 39900);
        assert!(p.name.contains("Sarung"));
    }

    #[test]
    fn normalize_rejects_empty_art() {
        assert!(normalize_product_row("", "x", "100").is_none());
        assert!(normalize_product_row("  ", "x", "100").is_none());
    }

    #[test]
    fn dedupe_last_art_wins() {
        let rows = vec![
            Product {
                art: "OB-010".into(),
                name: "first".into(),
                hpp: 1,
            },
            Product {
                art: "OB-010".into(),
                name: "second".into(),
                hpp: 69900,
            },
        ];
        let out = dedupe_by_art(rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].hpp, 69900);
        assert_eq!(out[0].name, "second");
    }

    #[test]
    fn parse_real_workbook_shape_when_present() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("MARKETPLACE_PRICE_2026_NORMALIZED.xlsx");
        if !path.is_file() {
            eprintln!("skip: workbook missing at {}", path.display());
            return;
        }
        let (products, skipped) = parse_produk_normalisasi_path(&path).expect("parse workbook");
        assert!(
            products.len() > 100,
            "expected many products, got {}",
            products.len()
        );
        // Known fixture from sheet row: OB-001 → 39900
        let ob001 = products.iter().find(|p| p.art == "OB-001");
        assert!(ob001.is_some(), "OB-001 must be present");
        assert_eq!(ob001.unwrap().hpp, 39900);
        // Unique ARTs after dedupe
        let mut arts: Vec<_> = products.iter().map(|p| p.art.as_str()).collect();
        arts.sort();
        arts.dedup();
        assert_eq!(arts.len(), products.len());
        // skipped may be > 0 for bad rows; must not panic
        assert!(skipped >= 0);
    }
}
