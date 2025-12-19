//! Test binary for checking collateral balance
//!
//! Fetches the USDC balance from Polymarket CLOB API.
//!
//! Requires environment variables in `.env`:
//!   - PRIVATE_KEY (with 0x prefix)
//!   - PROXY_WALLET (optional)
//!   - API_KEY, API_SECRET, API_PASSPHRASE (optional)
//!
//! Usage:
//!   cargo run --bin test_balance

use anyhow::Result;
use polymarket::infrastructure::client::clob::TradingClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("POLYMARKET BALANCE CHECK");
    println!("════════════════════════════════════════════════════════════════");
    println!();

    println!("Initializing trading client...");
    let client = TradingClient::from_env().await?;

    println!("  Signer:  {:?}", client.signer_address());
    println!("  Maker:   {:?}", client.maker_address());
    println!();

    println!("Fetching balance...");
    let balance = client.get_usd_balance().await?;

    println!();
    println!("BALANCE INFO:");
    println!("────────────────────────────────────────────────────────────────");
    println!("  Balance: ${:.2} USD", balance);
    println!();
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}
