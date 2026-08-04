//! Send a test email via SMTP (Resend) to a recipient address.
//! Recipient from `RESEND_TO` (env) or the first CLI arg.
//!
//! ```bash
//! cargo run --release --example send_test_email
//! cargo run --release --example send_test_email -- ujangas1908@gmail.com
//! ```

use orders::config::Config;
use orders::email;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cfg = Config::from_env()?;
    let email_cfg = cfg
        .smtp
        .as_ref()
        .ok_or("RESEND_API_KEY / RESEND_FROM not set")?;
    let to = match std::env::args().nth(1) {
        Some(t) => t,
        None => email_cfg
            .to
            .clone()
            .ok_or("no recipient: set RESEND_TO or pass <to> arg")?,
    };
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
