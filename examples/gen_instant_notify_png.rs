//! Playground: fixture → PNG for WA instant notify layout (phone-friendly).
//!
//! ```bash
//! cargo run --example gen_instant_notify_png
//! # → logs/instant-notify-sample.png
//! ```

use orders::instant_notify::{
    card_to_svg, default_png_out, default_sample_fixture, load_fixture, render_notify_png,
    write_sample_png, NotifyCard,
};
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut fixture = default_sample_fixture();
    let mut out = default_png_out();
    let mut keep_svg = false;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--fixture" => {
                fixture = PathBuf::from(args.next().ok_or("--fixture needs path")?);
            }
            "--out" => {
                out = PathBuf::from(args.next().ok_or("--out needs path")?);
            }
            "--svg" => keep_svg = true,
            other => {
                eprintln!("unknown arg: {other}");
                eprintln!("usage: gen_instant_notify_png [--fixture PATH] [--out PATH] [--svg]");
                std::process::exit(2);
            }
        }
    }

    let orders = load_fixture(&fixture)?;
    println!("fixture: {} ({} orders)", fixture.display(), orders.len());
    let card = NotifyCard::from_orders(orders.clone());
    println!("card: {} — {}", card.title, card.subtitle);

    let png = render_notify_png(orders).await?;
    write_sample_png(&out, &png)?;
    println!("png: {} ({} bytes)", out.display(), png.len());

    if keep_svg {
        let svg_path = out.with_extension("svg");
        let svg = card_to_svg(&card, &std::collections::HashMap::new());
        std::fs::write(&svg_path, svg.as_bytes())?;
        println!("svg: {} (thumbs empty in svg dump)", svg_path.display());
    }

    Ok(())
}
