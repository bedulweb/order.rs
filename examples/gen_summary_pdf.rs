use orders::batch::{BatchLineItem, BatchSession, PdfOrderLine};
use orders::batch_pdf::render_batch_pdf;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let lines = vec![
        PdfOrderLine {
            platform_order_id: "260715PS7HRGC0".into(),
            platform: "shopee".into(),
            carrier: "SPX Instant".into(),
            is_urgent: true,
            ordered_at_wib: "2026-07-22 08:00:00 WIB".into(),
            items: vec![
                BatchLineItem {
                    sku: Some("MB-043-3XL-PBLP".into()),
                    name: Some("Miabebo Piyama Outline".into()),
                    variant_attr: Some("3XL, Peach Blush Pink".into()),
                    image_url: None,
                    quantity: 2,
                },
                BatchLineItem {
                    sku: Some("OB-023T-2M-KMRI".into()),
                    name: Some("Obayito Guling Mimi Bolster".into()),
                    variant_attr: Some("2M, Kamari".into()),
                    image_url: None,
                    quantity: 1,
                },
            ],
        },
        PdfOrderLine {
            platform_order_id: "TT9999ABCDEF12".into(),
            platform: "tiktok".into(),
            carrier: "JNE REG".into(),
            is_urgent: false,
            ordered_at_wib: "2026-07-22 07:00:00 WIB".into(),
            items: vec![BatchLineItem {
                sku: Some("MB-043-3XL-PBLP".into()),
                name: Some("Miabebo Piyama Outline".into()),
                variant_attr: Some("3XL, Peach Blush Pink".into()),
                image_url: None,
                quantity: 1,
            }],
        },
    ];
    let bytes = render_batch_pdf(
        Uuid::nil(),
        BatchSession::Morning,
        "2026-07-22 14:30:00 WIB",
        2,
        1,
        &lines,
    )
    .await
    .unwrap();
    std::fs::write("logs/production-summary-list-sample.pdf", &bytes).unwrap();
    println!("wrote {} bytes", bytes.len());
}
