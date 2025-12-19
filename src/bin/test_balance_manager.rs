//! Test binary for BalanceManager
//!
//! Tests the balance monitoring and halt functionality.
//!
//! Requires environment variables in `.env`:
//!   - PRIVATE_KEY (with 0x prefix)
//!   - PROXY_WALLET (optional)
//!   - API_KEY, API_SECRET, API_PASSPHRASE (optional)
//!
//! Usage:
//!   cargo run --bin test_balance_manager

use anyhow::Result;
use polymarket::application::BalanceManager;
use polymarket::infrastructure::client::clob::TradingClient;
use polymarket::infrastructure::shutdown::ShutdownManager;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("BALANCE MANAGER TEST");
    println!("════════════════════════════════════════════════════════════════");
    println!();

    // Initialize trading client
    println!("Initializing trading client...");
    let trading = Arc::new(TradingClient::from_env().await?);
    println!("  Signer: {:?}", trading.signer_address());
    println!("  Maker:  {:?}", trading.maker_address());
    println!();

    // Initialize shutdown manager
    let shutdown = Arc::new(ShutdownManager::new());
    shutdown.spawn_signal_handler();

    // Create and start balance manager (10% halt threshold)
    println!("Starting balance manager with 10% halt threshold...");
    let mut balance_manager = BalanceManager::new(0.99);
    balance_manager
        .start(Arc::clone(&trading), shutdown.flag())
        .await?;

    println!();
    println!("Balance manager started! Monitoring for 30 seconds...");
    println!("Press Ctrl+C to stop early.");
    println!();
    println!("────────────────────────────────────────────────────────────────");

    // Monitor for 30 seconds, printing status every 2 seconds
    for i in 0..500 {
        if !shutdown.is_running() {
            println!("Shutdown requested, stopping...");
            break;
        }

        let current = balance_manager.current_balance();
        let pivot = balance_manager.pivot_balance();
        let halted = balance_manager.is_halted();
        let threshold = pivot * balance_manager.halt_threshold();

        println!(
            "[{:2}s] Current: ${:.2} | Pivot: ${:.2} | Threshold: ${:.2} | Halted: {}",
            i * 2,
            current,
            pivot,
            threshold,
            if halted { "YES" } else { "NO" }
        );

        sleep(Duration::from_secs(2)).await;
    }

    println!("────────────────────────────────────────────────────────────────");
    println!();

    // Stop balance manager
    println!("Stopping balance manager...");
    balance_manager.stop().await;

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("Test complete!");
    println!("════════════════════════════════════════════════════════════════");

    Ok(())
}
