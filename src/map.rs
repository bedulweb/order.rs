//! Map BigSeller `pageList` JSON rows → typed structs for Postgres upsert.

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct MappedShop {
    pub id: i64,
    pub platform: String,
    pub name: String,
    pub site: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MappedItem {
    pub id: i64,
    pub line_no: i32,
    pub sku: Option<String>,
    pub variant_attr: Option<String>,
    pub item_name: Option<String>,
    pub quantity: i32,
    pub amount: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub original_price: Option<Decimal>,
    pub image_url: Option<String>,
    pub product_url: Option<String>,
    pub platform_item_id: Option<String>,
    pub platform_variation_id: Option<String>,
    pub inventory_sku: Option<String>,
    pub is_addition: bool,
    pub product_type: Option<i32>,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct MappedOrder {
    pub id: i64,
    pub shop: MappedShop,
    pub platform: String,
    pub platform_order_id: String,
    pub package_no: Option<String>,
    pub package_index: Option<String>,
    pub state: String,
    pub platform_state: Option<String>,
    pub view_status: Option<String>,
    pub marketplace_state: Option<String>,
    pub last_order_status: Option<String>,
    pub amount: Option<Decimal>,
    pub currency: Option<String>,
    pub payment_method: Option<String>,
    pub buyer_username: Option<String>,
    pub contact_person: Option<String>,
    pub recipient_region: Option<String>,
    pub buyer_message: Option<String>,
    pub seller_note: Option<String>,
    pub tracking_no: Option<String>,
    pub tracking_url: Option<String>,
    pub shipment_provider: Option<String>,
    pub shipping_carrier_id: Option<i64>,
    pub shipping_carrier_name: Option<String>,
    pub buyer_shipping_carrier: Option<String>,
    pub shipping_config_option_id: Option<i32>,
    pub shipping_config_option_name: Option<String>,
    pub warehouse_id: Option<i64>,
    pub warehouse_name: Option<String>,
    pub store_site: Option<String>,
    pub pack_state: Option<i16>,
    pub item_total_num: Option<i32>,
    pub print_label_mark: Option<i16>,
    pub print_bill_mark: Option<i16>,
    pub print_pick_list_mark: Option<i16>,
    pub print_collect_mark: Option<i16>,
    pub has_error: bool,
    pub error_msg: Option<String>,
    pub ordered_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub ship_by_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub timeout_at: Option<DateTime<Utc>>,
    pub printed_collect_at: Option<DateTime<Utc>>,
    pub payload: Value,
    pub payload_hash: Vec<u8>,
    pub items: Vec<MappedItem>,
}

pub fn map_order_row(row: &Value) -> Option<MappedOrder> {
    let id = as_i64(row.get("id"))?;
    let shop_id = as_i64(row.get("shopId"))?;
    let platform = as_string(row.get("platform")).unwrap_or_else(|| "unknown".into());
    let platform_order_id = as_string(row.get("platformOrderId"))?;
    let shop_name = as_string(row.get("shopName")).unwrap_or_else(|| format!("shop-{shop_id}"));

    let err = as_string(row.get("errorMsg"))
        .or_else(|| as_string(row.get("error")))
        .filter(|s| !s.is_empty());

    let items = row
        .get("orderItemList")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .enumerate()
                .filter_map(|(i, it)| map_item(it, i as i32))
                .collect()
        })
        .unwrap_or_default();

    let payload = row.clone();
    let payload_hash = {
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        Sha256::digest(&bytes).to_vec()
    };

    Some(MappedOrder {
        id,
        shop: MappedShop {
            id: shop_id,
            platform: platform.clone(),
            name: shop_name,
            site: as_string(row.get("storeSite")),
        },
        platform,
        platform_order_id,
        package_no: empty_to_none(as_string(row.get("packageNo"))),
        package_index: as_string(row.get("packageIndex")),
        state: as_string(row.get("state")).unwrap_or_else(|| "unknown".into()),
        platform_state: as_string(row.get("platformState")),
        view_status: as_string(row.get("viewStatus")),
        marketplace_state: as_string(row.get("marketPlaceState")),
        last_order_status: as_string(row.get("lastOrderStatus")),
        amount: as_money(row.get("amount")),
        currency: as_string(row.get("amountUnit")),
        payment_method: as_string(row.get("paymentMethod")),
        buyer_username: as_string(row.get("buyerUsername")),
        contact_person: as_string(row.get("contactPerson")),
        recipient_region: as_string(row.get("recipient")),
        buyer_message: as_string(row.get("buyerMessage")),
        seller_note: as_string(row.get("sellerNote")).or_else(|| as_string(row.get("sellerNotes"))),
        tracking_no: empty_to_none(as_string(row.get("trackingNo"))),
        tracking_url: as_string(row.get("trackingUrl")),
        shipment_provider: as_string(row.get("shipmentProvider")),
        shipping_carrier_id: as_i64(row.get("shippingCarrierId")),
        shipping_carrier_name: as_string(row.get("shippingCarrierName")),
        buyer_shipping_carrier: as_string(row.get("buyerShippingCarrier")),
        shipping_config_option_id: as_i64(row.get("shippingConfigOptionId")).map(|n| n as i32),
        shipping_config_option_name: as_string(row.get("shippingConfigOptionName")),
        warehouse_id: as_i64(row.get("warehouseId")),
        warehouse_name: as_string(row.get("shipmentWarehouse")),
        store_site: as_string(row.get("storeSite")),
        pack_state: as_i64(row.get("packState")).map(|n| n as i16),
        item_total_num: as_i64(row.get("itemTotalNum")).map(|n| n as i32),
        print_label_mark: as_i64(row.get("printLabelMark")).map(|n| n as i16),
        print_bill_mark: as_i64(row.get("printBillMark")).map(|n| n as i16),
        print_pick_list_mark: as_i64(row.get("printPickListMark")).map(|n| n as i16),
        print_collect_mark: as_i64(row.get("printCollectMark")).map(|n| n as i16),
        has_error: err.is_some(),
        error_msg: err,
        ordered_at: as_ts(row.get("orderCreateTime")),
        paid_at: as_ts(row.get("payTime")),
        ship_by_at: as_ts(row.get("shippedTime")),
        completed_at: as_ts(row.get("completedTime")),
        deadline_at: as_ts(row.get("deadlineTime")),
        timeout_at: as_ts(row.get("outTime")),
        printed_collect_at: as_ts(row.get("printedCollectTime")),
        payload,
        payload_hash,
        items,
    })
}

fn map_item(it: &Value, line_no: i32) -> Option<MappedItem> {
    let id = as_i64(it.get("id"))?;
    Some(MappedItem {
        id,
        line_no,
        sku: as_string(it.get("varSku")).or_else(|| as_string(it.get("sku"))),
        variant_attr: as_string(it.get("varAttr")),
        item_name: as_string(it.get("itemName")).or_else(|| as_string(it.get("productName"))),
        quantity: as_i64(it.get("quantity")).unwrap_or(1) as i32,
        amount: as_money(it.get("amount")),
        unit_price: as_money(it.get("itemPrice")).or_else(|| as_money(it.get("price"))),
        original_price: as_money(it.get("originalPrice")),
        image_url: as_string(it.get("imgUrl")).or_else(|| as_string(it.get("imageUrl"))),
        product_url: as_string(it.get("productUrl")),
        platform_item_id: as_string(it.get("platformItemId")),
        platform_variation_id: as_string(it.get("platformVariationId")),
        inventory_sku: as_string(it.get("inventorySku")),
        is_addition: as_i64(it.get("isAddition")).unwrap_or(0) != 0
            || it
                .get("isAddition")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        product_type: as_i64(it.get("productType")).map(|n| n as i32),
        payload: it.clone(),
    })
}

pub fn as_i64(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    if let Some(n) = v.as_u64() {
        return Some(n as i64);
    }
    if let Some(n) = v.as_f64() {
        return Some(n as i64);
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse().ok();
    }
    None
}

pub fn as_string(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        if s.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    if let Some(n) = v.as_i64() {
        return Some(n.to_string());
    }
    if let Some(n) = v.as_u64() {
        return Some(n.to_string());
    }
    if let Some(b) = v.as_bool() {
        return Some(b.to_string());
    }
    None
}

fn empty_to_none(s: Option<String>) -> Option<String> {
    s.filter(|x| !x.is_empty())
}

/// Parse money from JSON number or string without going through `f64`.
pub fn as_money(v: Option<&Value>) -> Option<Decimal> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        return Decimal::from_str_exact(t)
            .or_else(|_| Decimal::from_str(t))
            .ok();
    }
    if let Some(n) = v.as_i64() {
        return Some(Decimal::from(n));
    }
    if let Some(n) = v.as_u64() {
        return Some(Decimal::from(n));
    }
    // JSON numbers that serde_json only exposes as f64 (no integer form).
    // Prefer string round-trip via Display to reduce binary float artifacts.
    if let Some(n) = v.as_f64() {
        if !n.is_finite() {
            return None;
        }
        return Decimal::from_str(&n.to_string()).ok();
    }
    None
}

/// Bind `Numeric` via string to avoid float drift on the wire.
pub fn money_str(v: Option<Decimal>) -> Option<String> {
    v.map(|d| d.normalize().to_string())
}

/// BigSeller epochs: usually ms; small values treated as seconds.
pub fn as_ts(v: Option<&Value>) -> Option<DateTime<Utc>> {
    let n = as_i64(v)?;
    if n <= 0 {
        return None;
    }
    let ms = if n > 1_000_000_000_000 {
        n
    } else if n > 1_000_000_000 {
        n * 1000
    } else {
        return None;
    };
    Utc.timestamp_millis_opt(ms).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn as_money_integer_string() {
        assert_eq!(as_money(Some(&json!("104848"))), Some(dec("104848")));
    }

    #[test]
    fn as_money_fractional_string() {
        assert_eq!(as_money(Some(&json!("104848.50"))), Some(dec("104848.50")));
    }

    #[test]
    fn as_money_empty_string_is_none() {
        assert_eq!(as_money(Some(&json!(""))), None);
        assert_eq!(as_money(Some(&json!("   "))), None);
    }

    #[test]
    fn as_money_integer_json() {
        assert_eq!(as_money(Some(&json!(104848))), Some(dec("104848")));
    }

    #[test]
    fn money_str_preserves_decimal() {
        // normalize() drops trailing zeros; value is still exact for Postgres numeric.
        assert_eq!(
            money_str(Some(dec("104848.50"))).as_deref(),
            Some("104848.5")
        );
        assert_eq!(money_str(Some(dec("104848"))).as_deref(), Some("104848"));
        assert_eq!(
            Decimal::from_str(money_str(Some(dec("104848.50"))).as_deref().unwrap()).unwrap(),
            dec("104848.50")
        );
    }
}
