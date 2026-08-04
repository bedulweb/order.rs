//! Send a test email via SMTP (Resend) to a recipient address.
//!
//! ```bash
//! cargo run --release --example send_test_email -- ujangas1908@gmail.com
//! ```

use orders::config::Config;
use orders::email;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let to = std::env::args()
        .nth(1)
        .ok_or("usage: send_test_email <to>")?;
    let cfg = Config::from_env()?;
    let email_cfg = cfg
        .smtp
        .as_ref()
        .ok_or("RESEND_API_KEY / RESEND_FROM not set")?;
    println!("sending to {to} from {} …", email_cfg.from);
    let msg_id = email::send_text(
        email_cfg,
        &to,
        "Test email dari order.rs",
        "Halo! Ini email test dari order.rs via Resend.",
    )
    .await?;
    println!("ok msg_id={msg_id}");
    Ok(())
}
