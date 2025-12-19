//! Test binary for canceling all open orders
//!
//! Cancels all open orders via Polymarket CLOB API.
//!
//! Requires environment variables in `.env`:
//!   - PRIVATE_KEY (with 0x prefix)
//!   - PROXY_WALLET (optional)
//!   - API_KEY, API_SECRET, API_PASSPHRASE (optional)
//!
//! Usage:
//!   cargo run --bin test_cancel_all

use anyhow::Result;
use polymarket::infrastructure::client::clob::TradingClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("CANCEL ALL ORDERS");
    println!("════════════════════════════════════════════════════════════════");
    println!();

    println!("Initializing trading client...");
    let client = TradingClient::from_env().await?;

    println!("  Signer:  {:?}", client.signer_address());
    println!("  Maker:   {:?}", client.maker_address());
    println!();

    println!("Canceling all open orders...");
    let response = client.cancel_all().await?;

    println!();
    println!("RESULT:");
    println!("────────────────────────────────────────────────────────────────");
    println!("  Canceled: {} order(s)", response.canceled.len());
    for order_id in &response.canceled {
        println!("    - {}", order_id);
    }

    if !response.not_canceled.is_empty() {
        println!();
        println!("  Failed to cancel: {} order(s)", response.not_canceled.len());
        for (order_id, reason) in &response.not_canceled {
            println!("    - {}: {}", order_id, reason);
        }
    }

    println!();
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}
