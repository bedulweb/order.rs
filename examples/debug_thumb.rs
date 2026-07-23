use orders::batch::{BatchLineItem, BatchSession, PdfOrderLine};
use orders::batch_pdf::render_batch_pdf;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    // download a real shopee thumb
    let url = "https://cf.shopee.co.id/file/id-11134207-81ztd-mfjkoulyliq5a9_tn";
    let lines = vec![PdfOrderLine {
        platform_order_id: "TESTORDER1234".into(),
        platform: "shopee".into(),
        carrier: "JNE".into(),
        is_urgent: false,
        ordered_at_wib: "2026-07-22 08:00:00 WIB".into(),
        items: vec![BatchLineItem {
            sku: Some("TEST-SKU".into()),
            name: Some("Test Product With Thumb".into()),
            variant_attr: Some("Red,M".into()),
            image_url: Some(url.into()),
            quantity: 3,
        }],
    }];
    let bytes = render_batch_pdf(
        Uuid::nil(),
        BatchSession::Morning,
        "2026-07-22 08:15:00 WIB",
        1,
        0,
        &lines,
    )
    .await
    .unwrap();
    std::fs::write("/tmp/debug-thumb.pdf", &bytes).unwrap();
    println!("wrote {} bytes", bytes.len());
}
