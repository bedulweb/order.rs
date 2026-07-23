//! Catalog parse / lookup / idempotent import tests.
//! Pure parse tests always run. DB tests skip cleanly without DATABASE_URL.

use orders::catalog::{
    dedupe_by_art, get_product, import_from_bytes, lookup_by_art, normalize_art,
    normalize_product_row, parse_hpp, parse_produk_normalisasi_bytes,
    parse_produk_normalisasi_path, upsert_products, Product, DEFAULT_WORKBOOK, PRIMARY_SHEET,
};
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use std::time::Duration;

fn workbook_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_WORKBOOK)
}

fn database_url() -> Option<String> {
    dotenvy::dotenv().ok();
    std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty())
}

async fn pool() -> Option<sqlx::PgPool> {
    let url = database_url()?;
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()
}

async fn ensure_catalog_table(pool: &sqlx::PgPool) -> bool {
    orders::catalog::ensure_schema(pool).await.is_ok()
}

#[test]
fn shipped_normalize_known_art_hpp() {
    // Representative rows from MARKETPLACE_PRICE_2026_NORMALIZED.xlsx sheet Produk Normalisasi.
    let p =
        normalize_product_row("OB-001", "Obayito Sarung Bantal Bayi", "39900").expect("OB-001 row");
    assert_eq!(p.art, "OB-001");
    assert_eq!(p.hpp, 39_900);

    let p2 = normalize_product_row("OB-021T-1XL", "Obayito Mimi Pillow Tencel", "329900")
        .expect("pillow row");
    assert_eq!(p2.hpp, 329_900);
}

#[test]
fn shipped_normalize_unknown_and_invalid() {
    assert!(normalize_product_row("", "x", "100").is_none());
    assert!(normalize_product_row("ART-X", "x", "-").is_none());
    assert!(normalize_product_row("ART-X", "x", "not-a-number").is_none());
    assert_eq!(normalize_art("  ZZ-9  ").as_deref(), Some("ZZ-9"));
    assert_eq!(parse_hpp("46900.0"), Some(46_900));
}

#[test]
fn shipped_dedupe_last_wins() {
    let out = dedupe_by_art(vec![
        Product {
            art: "OB-010".into(),
            name: "a".into(),
            hpp: 1,
        },
        Product {
            art: "OB-010".into(),
            name: "b".into(),
            hpp: 69_900,
        },
    ]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].hpp, 69_900);
}

#[test]
fn parse_real_workbook_known_pair() {
    let path = workbook_path();
    assert!(
        path.is_file(),
        "fixture workbook missing: {}",
        path.display()
    );
    let (products, _skipped) =
        parse_produk_normalisasi_path(&path).expect("parse shipped workbook");
    assert!(
        products.len() > 100,
        "expected large catalog, got {}",
        products.len()
    );
    let hit = products
        .iter()
        .find(|p| p.art == "OB-001")
        .expect("OB-001 in workbook");
    assert_eq!(hit.hpp, 39_900);
    assert!(hit.name.to_ascii_lowercase().contains("sarung"));

    // Unknown ART is simply absent — lookup miss is not a crash.
    assert!(products.iter().all(|p| p.art != "___NO_SUCH_ART___"));

    // Unique after dedupe
    let mut arts: Vec<&str> = products.iter().map(|p| p.art.as_str()).collect();
    let before = arts.len();
    arts.sort_unstable();
    arts.dedup();
    assert_eq!(arts.len(), before);

    // Sheet constant is the real sheet name used by parser.
    assert_eq!(PRIMARY_SHEET, "Produk Normalisasi");
}

#[test]
fn parse_bytes_drives_shipped_entry() {
    let path = workbook_path();
    let bytes = std::fs::read(&path).expect("read workbook");
    let (products, skipped) = parse_produk_normalisasi_bytes(&bytes).expect("parse bytes");
    assert!(!products.is_empty());
    assert!(skipped >= 0);
    let pillow = products
        .iter()
        .find(|p| p.art == "OB-021T-1XL")
        .expect("OB-021T-1XL");
    // After last-wins dedupe, HPP is one of the sheet values (integer IDR).
    assert!(pillow.hpp > 0);
}

#[tokio::test]
async fn db_lookup_hit_and_miss_and_idempotent_import() {
    let Some(pool) = pool().await else {
        eprintln!("skip: DATABASE_URL unavailable");
        return;
    };
    if !ensure_catalog_table(&pool).await {
        eprintln!("skip: product_catalog table unavailable");
        return;
    }

    // Isolate with synthetic ART prefix so we never collide with real catalog keys long-term.
    let art = format!("TEST-CAT-{}", uuid::Uuid::new_v4());
    let product = Product {
        art: art.clone(),
        name: "synthetic catalog row".into(),
        hpp: 12_345,
    };

    let r1 = upsert_products(&pool, &[product.clone()])
        .await
        .expect("first upsert");
    assert_eq!(r1.inserted, 1);
    assert_eq!(r1.updated, 0);

    let hit = lookup_by_art(&pool, &art).await.expect("lookup");
    assert!(hit.is_some());
    let hit = hit.unwrap();
    assert_eq!(hit.art, art);
    assert_eq!(hit.hpp, 12_345);

    let miss = lookup_by_art(&pool, "___NO_SUCH_ART___")
        .await
        .expect("miss lookup");
    assert!(miss.is_none());

    let empty = lookup_by_art(&pool, "   ").await.expect("empty art");
    assert!(empty.is_none());

    // Idempotent second upsert → update, not duplicate.
    let mut product2 = product.clone();
    product2.hpp = 54_321;
    product2.name = "updated name".into();
    let r2 = upsert_products(&pool, &[product2])
        .await
        .expect("second upsert");
    assert_eq!(r2.inserted, 0);
    assert_eq!(r2.updated, 1);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM product_catalog WHERE art = $1")
            .bind(&art)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(count, 1, "must not create duplicate ART rows");

    let got = get_product(&pool, &format!("  {art}  "))
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(got.hpp, 54_321);
    assert_eq!(got.name, "updated name");

    // Cleanup synthetic row
    let _ = sqlx::query("DELETE FROM product_catalog WHERE art = $1")
        .bind(&art)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn db_import_real_workbook_twice_no_duplicate_arts() {
    let Some(pool) = pool().await else {
        eprintln!("skip: DATABASE_URL unavailable");
        return;
    };
    if !ensure_catalog_table(&pool).await {
        eprintln!("skip: product_catalog table unavailable");
        return;
    }
    let path = workbook_path();
    if !path.is_file() {
        eprintln!("skip: workbook missing");
        return;
    }
    let bytes = std::fs::read(&path).expect("read");

    let first = import_from_bytes(&pool, &bytes).await.expect("import 1");
    assert!(first.inserted + first.updated > 0);

    let count_after_1: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM product_catalog")
        .fetch_one(&pool)
        .await
        .expect("count1");

    let second = import_from_bytes(&pool, &bytes).await.expect("import 2");
    // Second pass should not insert new unique ARTs (all updates or zero inserts).
    assert_eq!(
        second.inserted, 0,
        "second import must not insert new ARTs: {:?}",
        second
    );
    assert!(second.updated > 0);

    let count_after_2: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM product_catalog")
        .fetch_one(&pool)
        .await
        .expect("count2");
    assert_eq!(
        count_after_1, count_after_2,
        "ART row count must be stable across re-import"
    );

    // Known pair still correct via shipped lookup.
    let ob = lookup_by_art(&pool, "OB-001")
        .await
        .expect("lookup OB-001")
        .expect("OB-001 present");
    assert_eq!(ob.hpp, 39_900);
    assert!(ob.hpp > 0);

    // Sanity: unique constraint holds
    let dup: Option<(String, i64)> = sqlx::query_as(
        r#"
        SELECT art, COUNT(*)::bigint AS c
        FROM product_catalog
        GROUP BY art
        HAVING COUNT(*) > 1
        LIMIT 1
        "#,
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    assert!(dup.is_none(), "duplicate ART rows: {dup:?}");
}
