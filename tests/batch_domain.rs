//! Integration tests for batch membership + generate locking.
//! Requires `DATABASE_URL`. Synthetic orders only (platform_order_id prefix `TEST-BATCH-`).
//! Scoped to a dedicated `bs_accounts.code = test-batch-ops` so live backlog is never touched.
//! Skips cleanly when DB env is missing.

use orders::batch::{create_batch, get_batch_pdf, list_backlog, BatchSession};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

const PREFIX: &str = "TEST-BATCH-";
const ACCOUNT_CODE: &str = "test-batch-ops";

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

async fn ensure_tables(pool: &sqlx::PgPool) -> bool {
    sqlx::query("SELECT 1 FROM batches LIMIT 1")
        .fetch_optional(pool)
        .await
        .is_ok()
}

async fn ensure_test_account(pool: &sqlx::PgPool) -> sqlx::Result<i64> {
    if let Some(row) = sqlx::query("SELECT id FROM bs_accounts WHERE code = $1")
        .bind(ACCOUNT_CODE)
        .fetch_optional(pool)
        .await?
    {
        return Ok(row.get("id"));
    }
    // Prefer reusing any existing account so we never create tenant noise.
    if let Some(row) = sqlx::query("SELECT id FROM bs_accounts ORDER BY id LIMIT 1")
        .fetch_optional(pool)
        .await?
    {
        // Still scope by account_id of a dedicated synthetic account when possible.
        let id: i64 = row.get("id");
        // Insert a disposable account with required login_account.
        let ins = sqlx::query(
            r#"
            INSERT INTO bs_accounts (login_account, display_name, code, created_at, updated_at)
            VALUES ($1, 'synthetic batch tests', $2, now(), now())
            RETURNING id
            "#,
        )
        .bind(format!("__{ACCOUNT_CODE}__"))
        .bind(ACCOUNT_CODE)
        .fetch_one(pool)
        .await;
        match ins {
            Ok(r) => Ok(r.get("id")),
            Err(_) => Ok(id), // fallback: existing account (tests still use PREFIX ids)
        }
    } else {
        let r = sqlx::query(
            r#"
            INSERT INTO bs_accounts (login_account, display_name, code)
            VALUES ($1, 'synthetic batch tests', $2)
            RETURNING id
            "#,
        )
        .bind(format!("__{ACCOUNT_CODE}__"))
        .bind(ACCOUNT_CODE)
        .fetch_one(pool)
        .await?;
        Ok(r.get("id"))
    }
}

async fn cleanup(pool: &sqlx::PgPool, account_id: i64) {
    let _ = sqlx::query(
        r#"
        DELETE FROM batch_orders
        WHERE platform_order_id LIKE $1
           OR order_id IN (SELECT id FROM orders WHERE platform_order_id LIKE $1)
           OR batch_id IN (SELECT id FROM batches WHERE account_id = $2)
        "#,
    )
    .bind(format!("{PREFIX}%"))
    .bind(account_id)
    .execute(pool)
    .await;

    let _ = sqlx::query("DELETE FROM batches WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;

    let _ = sqlx::query(
        "DELETE FROM order_items WHERE order_id IN (SELECT id FROM orders WHERE platform_order_id LIKE $1)",
    )
    .bind(format!("{PREFIX}%"))
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM orders WHERE platform_order_id LIKE $1 OR account_id = $2")
        .bind(format!("{PREFIX}%"))
        .bind(account_id)
        .execute(pool)
        .await;
}

async fn seed_order(
    pool: &sqlx::PgPool,
    account_id: i64,
    id: i64,
    platform_order_id: &str,
    carrier: &str,
) -> sqlx::Result<()> {
    let shop_id: i64 = match sqlx::query("SELECT id FROM shops ORDER BY id LIMIT 1")
        .fetch_optional(pool)
        .await?
    {
        Some(r) => r.get("id"),
        None => {
            sqlx::query(
                r#"
                INSERT INTO shops (id, platform, name, site, payload, synced_at, updated_at)
                VALUES ($1, 'test', 'test-shop', 'ID', '{}'::jsonb, now(), now())
                ON CONFLICT (id) DO NOTHING
                "#,
            )
            .bind(9_000_001_i64)
            .execute(pool)
            .await?;
            9_000_001
        }
    };

    sqlx::query(
        r#"
        INSERT INTO orders (
            id, account_id, shop_id, platform, platform_order_id, state,
            buyer_shipping_carrier, shipment_provider, shipping_carrier_name,
            amount, currency, has_error, payload, payload_hash,
            first_seen_at, synced_at, updated_at, ordered_at
        ) VALUES (
            $1, $2, $3, 'shopee', $4, 'new',
            $5, $5, $5,
            1000, 'IDR', false, '{}'::jsonb, 'test',
            now(), now(), now(), now() - ($6::int * interval '1 minute')
        )
        ON CONFLICT (id) DO UPDATE SET
            account_id = EXCLUDED.account_id,
            state = 'new',
            platform_order_id = EXCLUDED.platform_order_id,
            buyer_shipping_carrier = EXCLUDED.buyer_shipping_carrier,
            shipment_provider = EXCLUDED.shipment_provider,
            shipping_carrier_name = EXCLUDED.shipping_carrier_name,
            updated_at = now()
        "#,
    )
    .bind(id)
    .bind(account_id)
    .bind(shop_id)
    .bind(platform_order_id)
    .bind(carrier)
    .bind(((id % 100) as i32).abs())
    .execute(pool)
    .await?;

    sqlx::query(r#"DELETE FROM order_items WHERE order_id = $1"#)
        .bind(id)
        .execute(pool)
        .await?;
    let item_id = id * 10 + 1;
    sqlx::query(
        r#"
        INSERT INTO order_items (id, order_id, line_no, sku, item_name, quantity, payload)
        VALUES ($1, $2, 1, $3, $4, 1, '{}'::jsonb)
        "#,
    )
    .bind(item_id)
    .bind(id)
    .bind(format!("SKU-{id}"))
    .bind(format!("Item {id}"))
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
async fn backlog_excludes_active_membership_and_generate_is_exclusive() {
    let Some(pool) = pool().await else {
        eprintln!("skip: DATABASE_URL unavailable");
        return;
    };
    if !ensure_tables(&pool).await {
        eprintln!("skip: batches tables missing — apply docs/sql/005_batches.sql");
        return;
    }

    let account_id = match ensure_test_account(&pool).await {
        Ok(id) => id,
        Err(e) => {
            eprintln!("skip: cannot create test account: {e}");
            return;
        }
    };

    cleanup(&pool, account_id).await;

    let base = 8_800_000_000_i64;
    let id_std = base + 1;
    let id_urg = base + 2;
    let id_std2 = base + 3;

    seed_order(
        &pool,
        account_id,
        id_std,
        &format!("{PREFIX}STD-1"),
        "JNE Reguler",
    )
    .await
    .expect("seed std");
    seed_order(
        &pool,
        account_id,
        id_urg,
        &format!("{PREFIX}URG-1"),
        "SPX Instant",
    )
    .await
    .expect("seed urg");
    seed_order(
        &pool,
        account_id,
        id_std2,
        &format!("{PREFIX}STD-2"),
        "SiCepat REG",
    )
    .await
    .expect("seed std2");

    let bl = list_backlog(&pool, Some(account_id), 5000)
        .await
        .expect("backlog");
    assert_eq!(
        bl.orders.len(),
        3,
        "synthetic backlog only for test account"
    );
    let urg = bl
        .orders
        .iter()
        .find(|o| o.order_id == id_urg)
        .expect("urg in backlog");
    assert!(urg.is_urgent, "SPX Instant must classify urgent");
    let std = bl
        .orders
        .iter()
        .find(|o| o.order_id == id_std)
        .expect("std");
    assert!(!std.is_urgent);

    let detail = create_batch(&pool, BatchSession::Urgent, Some(account_id))
        .await
        .expect("create urgent");
    assert_eq!(detail.summary.order_count, 1);
    assert!(detail.members.iter().any(|m| m.order_id == id_urg));
    assert!(!detail.members.iter().any(|m| m.order_id == id_std));

    let (fname, pdf) = get_batch_pdf(&pool, detail.summary.id)
        .await
        .expect("pdf query")
        .expect("pdf present");
    assert!(fname.ends_with(".pdf"));
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 200);

    let (_, pdf2) = get_batch_pdf(&pool, detail.summary.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pdf, pdf2, "reprint returns same stored PDF bytes");

    let bl2 = list_backlog(&pool, Some(account_id), 5000)
        .await
        .expect("backlog2");
    let ids: Vec<_> = bl2.orders.iter().map(|o| o.order_id).collect();
    assert!(!ids.contains(&id_urg), "batched urgent must leave backlog");
    assert!(ids.contains(&id_std));
    assert!(ids.contains(&id_std2));

    let morning = create_batch(&pool, BatchSession::Morning, Some(account_id))
        .await
        .expect("morning");
    assert_eq!(morning.summary.order_count, 2);
    assert!(morning.members.iter().any(|m| m.order_id == id_std));
    assert!(morning.members.iter().any(|m| m.order_id == id_std2));
    assert!(!morning.members.iter().any(|m| m.order_id == id_urg));

    match create_batch(&pool, BatchSession::Morning, Some(account_id)).await {
        Ok(second) => {
            panic!(
                "expected empty backlog error, got {} members",
                second.members.len()
            );
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("no eligible orders"),
                "unexpected error: {msg}"
            );
        }
    }

    let ghost = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO batches (id, account_id, session, timezone, status, order_count, urgent_count)
        VALUES ($1, $2, 'urgent', 'Asia/Jakarta', 'ready', 0, 0)
        "#,
    )
    .bind(ghost)
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("ghost batch");
    let bad = sqlx::query(
        r#"
        INSERT INTO batch_orders (batch_id, order_id, platform_order_id, is_urgent, position)
        VALUES ($1, $2, $3, false, 0)
        "#,
    )
    .bind(ghost)
    .bind(id_std)
    .bind(format!("{PREFIX}STD-1"))
    .execute(&pool)
    .await;
    assert!(
        bad.is_err(),
        "unique active membership must reject double insert"
    );
    let _ = sqlx::query("DELETE FROM batches WHERE id = $1")
        .bind(ghost)
        .execute(&pool)
        .await;

    cleanup(&pool, account_id).await;
}
