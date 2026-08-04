//! Kirim kartu cancel list — HANYA order cancel yang summary list-nya sudah
//! di-print (batch_orders aktif dan/atau print collect/pick mark di BigSeller).
//! Ini aturan yang sama dengan notifikasi cancel otomatis di worker
//! (`send_cancel_notify`): cancel tanpa print summary tidak dikirim.

use orders::config::Config;
use orders::db;
use orders::notify;
use orders::store;
use orders::wazapin::{WazapinClient, WazapinConfig};
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let wz_cfg = cfg
        .wazapin
        .clone()
        .or_else(WazapinConfig::from_env)
        .ok_or("WAZAPIN not set")?;
    let client = WazapinClient::new(wz_cfg)?;

    // "Cancel hari ini" = yang ordered_at-nya jatuh di hari kalender WIB ini
    // (sama seperti daily in-cancel report), BUKAN synced_at (yang menangkap
    // re-sync batch lama dalam jumlah banyak).
    let wib = chrono::FixedOffset::east_opt(7 * 3600).expect("WIB offset");
    let today_wib = chrono::Utc::now().with_timezone(&wib).date_naive();
    let day_start = today_wib
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight")
        .and_local_timezone(wib)
        .single()
        .expect("WIB has no DST");
    let day_start_utc = day_start.with_timezone(&chrono::Utc);
    let day_end_utc = day_start_utc + chrono::Duration::days(1);
    println!("window ordered_at: {day_start_utc} .. {day_end_utc} (hari {today_wib} WIB)");

    let rows = sqlx::query(
        r#"
        SELECT o.id, o.platform_order_id, o.platform,
               o.print_collect_mark, o.print_pick_list_mark,
               EXISTS(SELECT 1 FROM batch_orders bo WHERE bo.order_id=o.id AND bo.voided_at IS NULL) AS in_batch,
               o.ordered_at
        FROM orders o
        WHERE (
            o.state IN ('canceled', 'cancelled')
            OR COALESCE((o.payload->>'inCancel')::boolean, false) = true
            OR COALESCE(o.payload->>'inCancel', '') IN ('1', 'true', 'True')
            OR o.view_status ILIKE '%cancel%'
            OR o.marketplace_state ILIKE '%cancel%'
        )
        AND COALESCE(o.ordered_at, o.first_seen_at) >= $1
        AND COALESCE(o.ordered_at, o.first_seen_at) < $2
        ORDER BY o.ordered_at DESC NULLS LAST
        "#,
    )
    .bind(day_start_utc)
    .bind(day_end_utc)
    .fetch_all(&pool)
    .await?;

    let mut sent = Vec::new();
    for r in &rows {
        let id: i64 = r.get("id");
        if !store::order_summary_was_printed(&pool, id).await? {
            continue;
        }
        let o = notify::load_cancel_order(&pool, id).await?;
        println!(
            "  + {} · {} · printed ({} items)",
            o.platform_order_id,
            o.platform,
            o.items.len()
        );
        sent.push(o);
    }

    if sent.is_empty() {
        println!("tidak ada cancel dengan summary list sudah di-print");
        return Ok(());
    }
    println!("sending cancel list (printed) n={} …", sent.len());
    let msg_id = notify::send_cancel_orders(&client, sent).await?;
    println!("ok msg_id={msg_id}");
    Ok(())
}
