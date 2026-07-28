//! Probe 2: catalog ART shape + prefix-match HPP coverage.
use orders::config::Config;
use orders::db;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;

    println!("=== sample catalog art ===");
    for r in sqlx::query("SELECT art, name, hpp FROM product_catalog WHERE hpp > 0 ORDER BY updated_at DESC LIMIT 12")
        .fetch_all(&pool)
        .await?
    {
        println!(
            "  {:<16} hpp={:<8} {}",
            r.get::<String, _>("art"),
            r.get::<i64, _>("hpp"),
            r.get::<String, _>("name").chars().take(40).collect::<String>(),
        );
    }

    println!("\n=== sample order skus (30d) ===");
    for r in sqlx::query(
        "SELECT DISTINCT btrim(oi.sku) AS sku FROM order_items oi JOIN orders o ON o.id=oi.order_id WHERE o.ordered_at >= now() - interval '30 days' ORDER BY 1 LIMIT 15",
    )
    .fetch_all(&pool)
    .await?
    {
        println!("  {}", r.get::<String, _>("sku"));
    }

    println!("\n=== prefix-match coverage (30d qty) ===");
    // Try matching sku prefixes of length N (split on '-') against catalog art.
    for n in [1, 2, 3] {
        let r = sqlx::query(&format!(
            r#"
            SELECT COALESCE(SUM(oi.quantity),0)::bigint AS qty_total,
                   COALESCE(SUM(oi.quantity) FILTER (WHERE pc.art IS NOT NULL),0)::bigint AS qty_matched
            FROM order_items oi
            JOIN orders o ON o.id = oi.order_id
            LEFT JOIN product_catalog pc
              ON pc.art = array_to_string((string_to_array(btrim(oi.sku), '-'))[1:{n}], '-')
            WHERE o.ordered_at >= now() - interval '30 days'
              AND oi.is_addition IS NOT TRUE
            "#
        ))
        .fetch_one(&pool)
        .await?;
        let total = r.get::<i64, _>("qty_total");
        let matched = r.get::<i64, _>("qty_matched");
        println!(
            "  prefix-{n}: {matched}/{total} qty = {:.1}%",
            100.0 * matched as f64 / total.max(1) as f64
        );
    }

    println!("\n=== exact vs prefix-2 combined (30d) ===");
    let r = sqlx::query(
        r#"
        SELECT COALESCE(SUM(oi.quantity),0)::bigint AS qty_total,
               COALESCE(SUM(oi.quantity) FILTER (WHERE pc.art IS NOT NULL),0)::bigint AS qty_matched
        FROM order_items oi
        JOIN orders o ON o.id = oi.order_id
        LEFT JOIN product_catalog pc
          ON pc.art = COALESCE(
              (SELECT art FROM product_catalog WHERE art = btrim(oi.sku) LIMIT 1),
              (SELECT art FROM product_catalog WHERE art = array_to_string((string_to_array(btrim(oi.sku), '-'))[1:2], '-') LIMIT 1)
          )
        WHERE o.ordered_at >= now() - interval '30 days'
          AND oi.is_addition IS NOT TRUE
        "#,
    )
    .fetch_one(&pool)
    .await?;
    println!(
        "  combined: {}/{}",
        r.get::<i64, _>("qty_matched"),
        r.get::<i64, _>("qty_total")
    );
    Ok(())
}
