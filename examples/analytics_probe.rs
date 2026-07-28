//! Read-only probe: what data exists for the analytics page.
use orders::config::Config;
use orders::db;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;

    println!("=== ORDERS: volume + range ===");
    let r = sqlx::query(
        r#"
        SELECT count(*)::bigint AS n,
               min(ordered_at) AS min_at, max(ordered_at) AS max_at,
               count(*) FILTER (WHERE amount IS NULL)::bigint AS amount_null,
               count(*) FILTER (WHERE ordered_at IS NULL)::bigint AS ordered_null
        FROM orders
        "#,
    )
    .fetch_one(&pool)
    .await?;
    println!(
        "n={} range={:?}..{:?} amount_null={} ordered_null={}",
        r.get::<i64, _>("n"),
        r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("min_at"),
        r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("max_at"),
        r.get::<i64, _>("amount_null"),
        r.get::<i64, _>("ordered_null"),
    );

    println!("\n=== STATE distribution ===");
    for r in sqlx::query("SELECT state, count(*)::bigint AS n FROM orders GROUP BY 1 ORDER BY 2 DESC")
        .fetch_all(&pool)
        .await?
    {
        println!("  {:<20} {}", r.get::<Option<String>, _>("state").unwrap_or_default(), r.get::<i64, _>("n"));
    }

    println!("\n=== PLATFORM x CURRENCY ===");
    for r in sqlx::query(
        "SELECT platform, currency, count(*)::bigint AS n FROM orders GROUP BY 1,2 ORDER BY 3 DESC",
    )
    .fetch_all(&pool)
    .await?
    {
        println!(
            "  {:<12} {:<6} {}",
            r.get::<Option<String>, _>("platform").unwrap_or_default(),
            r.get::<Option<String>, _>("currency").unwrap_or_default(),
            r.get::<i64, _>("n"),
        );
    }

    println!("\n=== ITEMS: coverage ===");
    let r = sqlx::query(
        r#"
        SELECT count(*)::bigint AS n,
               count(*) FILTER (WHERE unit_price IS NULL)::bigint AS price_null,
               count(*) FILTER (WHERE amount IS NULL)::bigint AS amount_null,
               count(*) FILTER (WHERE sku IS NULL OR btrim(sku) = '')::bigint AS sku_empty,
               count(*) FILTER (WHERE is_addition)::bigint AS additions,
               count(*) FILTER (WHERE quantity IS NULL)::bigint AS qty_null
        FROM order_items
        "#,
    )
    .fetch_one(&pool)
    .await?;
    println!(
        "n={} price_null={} amount_null={} sku_empty={} additions={} qty_null={}",
        r.get::<i64, _>("n"),
        r.get::<i64, _>("price_null"),
        r.get::<i64, _>("amount_null"),
        r.get::<i64, _>("sku_empty"),
        r.get::<i64, _>("additions"),
        r.get::<i64, _>("qty_null"),
    );

    println!("\n=== CATALOG: HPP coverage (items last 30d by qty) ===");
    let r = sqlx::query(
        r#"
        SELECT count(DISTINCT oi.sku)::bigint AS skus,
               COALESCE(SUM(oi.quantity),0)::bigint AS qty_total,
               COALESCE(SUM(oi.quantity) FILTER (WHERE pc.art IS NOT NULL),0)::bigint AS qty_with_hpp,
               count(DISTINCT oi.sku) FILTER (WHERE pc.art IS NOT NULL)::bigint AS skus_with_hpp
        FROM order_items oi
        JOIN orders o ON o.id = oi.order_id
        LEFT JOIN product_catalog pc ON pc.art = btrim(oi.sku)
        WHERE o.ordered_at >= now() - interval '30 days'
          AND oi.is_addition IS NOT TRUE
        "#,
    )
    .fetch_one(&pool)
    .await?;
    println!(
        "skus={} qty_total={} qty_with_hpp={} skus_with_hpp={}",
        r.get::<i64, _>("skus"),
        r.get::<i64, _>("qty_total"),
        r.get::<i64, _>("qty_with_hpp"),
        r.get::<i64, _>("skus_with_hpp"),
    );
    let cat = sqlx::query("SELECT count(*)::bigint AS n, count(*) FILTER (WHERE hpp > 0)::bigint AS hpp_pos FROM product_catalog")
        .fetch_one(&pool)
        .await?;
    println!(
        "catalog rows={} hpp>0={}",
        cat.get::<i64, _>("n"),
        cat.get::<i64, _>("hpp_pos"),
    );

    println!("\n=== MARGIN sample (top 5 sku by revenue, 30d) ===");
    for r in sqlx::query(
        r#"
        SELECT COALESCE(NULLIF(btrim(oi.sku),''), oi.item_name, '?') AS sku,
               COALESCE(SUM(oi.quantity),0)::bigint AS qty,
               COALESCE(SUM(oi.amount),0)::text AS revenue,
               COALESCE(SUM(pc.hpp::numeric * oi.quantity),0)::text AS cost,
               count(*) FILTER (WHERE pc.art IS NULL)::bigint AS lines_no_hpp
        FROM order_items oi
        JOIN orders o ON o.id = oi.order_id
        LEFT JOIN product_catalog pc ON pc.art = btrim(oi.sku)
        WHERE o.ordered_at >= now() - interval '30 days'
          AND o.state NOT IN ('canceled','cancelled','archived')
          AND oi.is_addition IS NOT TRUE
        GROUP BY 1
        ORDER BY revenue DESC
        LIMIT 5
        "#,
    )
    .fetch_all(&pool)
    .await?
    {
        println!(
            "  {:<16} qty={:<4} rev={:<12} cost={:<12} lines_no_hpp={}",
            r.get::<Option<String>, _>("sku").unwrap_or_default(),
            r.get::<i64, _>("qty"),
            r.get::<String, _>("revenue"),
            r.get::<String, _>("cost"),
            r.get::<i64, _>("lines_no_hpp"),
        );
    }

    println!("\n=== feeDetail presence (30d) ===");
    let r = sqlx::query(
        r#"
        SELECT count(*)::bigint AS n,
               count(*) FILTER (WHERE payload ? 'feeDetail')::bigint AS with_fee,
               count(*) FILTER (WHERE payload->'feeDetail' IS NOT NULL AND payload->'feeDetail' != 'null'::jsonb)::bigint AS fee_nonnull
        FROM orders
        WHERE ordered_at >= now() - interval '30 days'
        "#,
    )
    .fetch_one(&pool)
    .await?;
    println!(
        "orders={} with_fee_key={} fee_nonnull={}",
        r.get::<i64, _>("n"),
        r.get::<i64, _>("with_fee"),
        r.get::<i64, _>("fee_nonnull"),
    );
    if let Some(r) = sqlx::query(
        "SELECT payload->'feeDetail' AS fee FROM orders WHERE payload->'feeDetail' IS NOT NULL AND payload->'feeDetail' != 'null'::jsonb LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?
    {
        let fee: Option<serde_json::Value> = r.get("fee");
        println!("sample feeDetail: {}", serde_json::to_string(&fee.unwrap_or_default())?.chars().take(400).collect::<String>());
    }

    println!("\n=== DAILY volume sample (last 7d) ===");
    for r in sqlx::query(
        r#"
        SELECT to_char(timezone('Asia/Jakarta', ordered_at), 'MM-DD') AS d,
               count(*)::bigint AS n,
               COALESCE(SUM(amount),0)::text AS revenue
        FROM orders
        WHERE ordered_at >= now() - interval '7 days'
          AND state NOT IN ('canceled','cancelled','archived')
        GROUP BY 1 ORDER BY 1
        "#,
    )
    .fetch_all(&pool)
    .await?
    {
        println!(
            "  {} orders={} revenue={}",
            r.get::<String, _>("d"),
            r.get::<i64, _>("n"),
            r.get::<String, _>("revenue"),
        );
    }

    Ok(())
}
