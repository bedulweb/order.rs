//! Dump feeDetail keys for today's active orders (WIB).
use orders::config::Config;
use orders::db;
use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let pool = db::connect(cfg.require_database_url()?).await?;
    let rows = sqlx::query(
        r#"
        SELECT id, platform, amount::text AS amount, state,
               payload->'feeDetail' AS fee
        FROM orders
        WHERE coalesce(ordered_at, first_seen_at) >= (timezone('Asia/Jakarta', now()))::date AT TIME ZONE 'Asia/Jakarta'
          AND coalesce(ordered_at, first_seen_at) < ((timezone('Asia/Jakarta', now()))::date + 1) AT TIME ZONE 'Asia/Jakarta'
          AND lower(coalesce(state,'')) NOT IN ('canceled','cancelled')
        ORDER BY coalesce(ordered_at, first_seen_at)
        "#,
    )
    .fetch_all(&pool)
    .await?;

    let mut key_stats: std::collections::BTreeMap<String, (i64, f64, f64, f64, i64)> =
        std::collections::BTreeMap::new();
    let mut both = 0i64;
    let mut sum_actual = 0.0f64;
    let mut sum_est = 0.0f64;
    let mut sum_pref = 0.0f64;
    let mut fee_sum_naive = 0.0f64;
    let mut fee_sum_safe = 0.0f64;

    for (i, r) in rows.iter().enumerate() {
        let id: i64 = r.get("id");
        let platform: Option<String> = r.try_get("platform").ok();
        let amount: String = r.get("amount");
        let fee: Option<serde_json::Value> = r.try_get("fee").ok().flatten();
        if i < 3 {
            println!("=== {id} {platform:?} amount={amount}");
            if let Some(f) = &fee {
                println!("{}", serde_json::to_string_pretty(f)?);
            } else {
                println!("(no feeDetail)");
            }
        }
        let Some(f) = fee else { continue };

        let num = |v: &serde_json::Value| -> Option<f64> {
            v.as_f64()
                .or_else(|| v.as_i64().map(|n| n as f64))
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        };
        let walk = |obj: &serde_json::Value, prefix: &str, stats: &mut std::collections::BTreeMap<String, (i64, f64, f64, f64, i64)>| {
            let Some(map) = obj.as_object() else { return };
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                if let Some(n) = num(v) {
                    let e = stats.entry(key).or_insert((0, 0.0, f64::MAX, f64::MIN, 0));
                    e.0 += 1;
                    e.1 += n;
                    e.2 = e.2.min(n);
                    e.3 = e.3.max(n);
                    if n < 0.0 { e.4 += 1; }
                } else if v.is_object() {
                    // one level deeper only for otherFeeInfo etc handled below
                }
            }
        };
        walk(&f, "", &mut key_stats);
        if let Some(o) = f.get("otherFeeInfo") {
            walk(o, "otherFeeInfo", &mut key_stats);
        }

        let actual = f
            .get("otherFeeInfo")
            .and_then(|o| o.get("actualShippingFee"))
            .and_then(num)
            .unwrap_or(0.0);
        let est = f.get("estimatedShippingFee").and_then(num).unwrap_or(0.0);
        sum_actual += actual.max(0.0);
        sum_est += est.max(0.0);
        if actual > 0.0 && est > 0.0 {
            both += 1;
        }
        if actual > 0.0 {
            sum_pref += actual;
        } else if est > 0.0 {
            sum_pref += est;
        }

        for k in [
            "serviceFee",
            "commissionFee",
            "totalPlatformFee",
            "sellerTransactionFee",
            "orderProcessFee",
            "sellerOrderProcessingFee",
            "newServiceFee",
        ] {
            if let Some(v) = f.get(k).and_then(num) {
                fee_sum_naive += v; // includes negatives
                if v > 0.0 {
                    fee_sum_safe += v;
                }
            }
        }
    }

    println!("\n=== KEY STATS (n,sum,min,max,neg) ===");
    for (k, (n, sum, min, max, neg)) in &key_stats {
        println!("{k}: n={n} sum={sum:.0} min={min} max={max} neg={neg}");
    }
    println!("\norders={}", rows.len());
    println!("both_actual_and_est={both}");
    println!(
        "sum_actual={sum_actual:.0} sum_est={sum_est:.0} sum_pref={sum_pref:.0} double={:.0}",
        sum_actual + sum_est
    );
    println!("fee_naive_all_signed={fee_sum_naive:.0} fee_pos_only={fee_sum_safe:.0}");
    Ok(())
}
