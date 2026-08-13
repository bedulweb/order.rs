//! Probe: cari order cancel yang summary list-nya sudah di-print (untuk tes
//! kartu cancel). Usage: cargo run --example probe_printed_cancel [limit]

use sqlx::Row;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let limit: i64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let url = std::env::var("DATABASE_URL")?;
    let pool = sqlx::PgPool::connect(&url).await?;

    let rows = sqlx::query(
        r#"
        SELECT o.id, o.platform_order_id, o.platform, o.state, o.ordered_at,
               o.print_collect_mark, o.print_pick_list_mark,
               EXISTS(SELECT 1 FROM batch_orders bo WHERE bo.order_id = o.id AND bo.voided_at IS NULL) AS in_batch
        FROM orders o
        WHERE (
            o.state IN ('canceled', 'cancelled')
            OR COALESCE((o.payload->>'inCancel')::boolean, false) = true
            OR COALESCE(o.payload->>'inCancel', '') IN ('1', 'true', 'True')
            OR o.view_status ILIKE '%cancel%'
            OR o.marketplace_state ILIKE '%cancel%'
        )
        AND (
            o.print_collect_mark <> 0
            OR o.print_pick_list_mark <> 0
            OR EXISTS(SELECT 1 FROM batch_orders bo WHERE bo.order_id = o.id AND bo.voided_at IS NULL)
        )
        ORDER BY o.ordered_at DESC NULLS LAST
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    if rows.is_empty() {
        println!("tidak ada cancel dengan summary printed");
        return Ok(());
    }
    for r in &rows {
        println!(
            "id={} order={} platform={} state={:?} ordered={:?} collect={} pick={} in_batch={}",
            r.get::<i64, _>("id"),
            r.get::<String, _>("platform_order_id"),
            r.get::<String, _>("platform"),
            r.get::<Option<String>, _>("state"),
            r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("ordered_at"),
            r.get::<i16, _>("print_collect_mark"),
            r.get::<i16, _>("print_pick_list_mark"),
            r.get::<bool, _>("in_batch"),
        );
    }
    Ok(())
}
