//! Manual: send a plain text message to a WhatsApp group via Wazapin.
//!
//! ```bash
//! cargo run --release --example send_test_text -- "test gitu"
//! cargo run --release --example send_test_text -- "test gitu" 120363346311683269@g.us
//! ```

use orders::wazapin::{WazapinClient, WazapinConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let msg = args.first().cloned().unwrap_or_else(|| "test gitu".into());
    let to = args
        .get(1)
        .cloned()
        .or_else(|| {
            std::env::var("WAZAPIN_GROUP_JID")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .ok_or("no group jid: pass <to> arg or set WAZAPIN_GROUP_JID")?;

    let mut cfg =
        WazapinConfig::from_env().ok_or("WAZAPIN_API_KEY / WAZAPIN_CHANNEL_ID not set")?;
    cfg.group_jid = to.clone();

    let client = WazapinClient::new(cfg)?;
    let r = client.send_text(&msg).await?;
    println!("sent msg_id={} to={to} text={msg}", r.id);
    Ok(())
}
