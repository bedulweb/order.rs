//! Offline mapper coverage — no network, no Neon.

use orders::map::{as_money, as_ts, map_order_row};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;

fn fixture_row() -> Value {
    let raw = include_str!("fixtures/order_row_min.json");
    serde_json::from_str(raw).expect("fixture JSON parses")
}

#[test]
fn map_order_row_fixture_happy_path() {
    let row = fixture_row();
    let m = map_order_row(&row).expect("mappable fixture row");

    assert_eq!(m.id, 14459756009);
    assert_eq!(m.shop.id, 2001903);
    assert_eq!(m.platform_order_id, "2607206K6S67BG");
    assert_eq!(m.platform, "shopee");
    assert_eq!(m.amount, Some(Decimal::from_str("104848").unwrap()));
    assert_eq!(m.currency.as_deref(), Some("IDR"));
    assert!(!m.items.is_empty());
    let item = &m.items[0];
    assert_eq!(item.quantity, 1);
    assert!(item.sku.is_some() || item.item_name.is_some());
}

#[test]
fn map_order_row_missing_id_is_none() {
    let mut row = fixture_row();
    if let Value::Object(map) = &mut row {
        map.remove("id");
    }
    assert!(map_order_row(&row).is_none());
}

#[test]
fn as_money_accepts_integer_and_fractional_strings() {
    assert_eq!(
        as_money(Some(&json!("104848"))),
        Some(Decimal::from_str("104848").unwrap())
    );
    assert_eq!(
        as_money(Some(&json!("104848.50"))),
        Some(Decimal::from_str("104848.50").unwrap())
    );
}

#[test]
fn as_ts_ms_vs_seconds_boundary() {
    // milliseconds epoch → Some
    let ms = as_ts(Some(&json!(1_700_000_000_000i64))).expect("ms");
    assert_eq!(ms.timestamp_millis(), 1_700_000_000_000);

    // seconds epoch (just above 1e9) → scaled to ms
    let sec = as_ts(Some(&json!(1_700_000_000i64))).expect("seconds");
    assert_eq!(sec.timestamp_millis(), 1_700_000_000_000);

    // too small → None
    assert!(as_ts(Some(&json!(999_999_999i64))).is_none());
    assert!(as_ts(Some(&json!(0))).is_none());
    assert!(as_ts(Some(&json!(-1))).is_none());
}
