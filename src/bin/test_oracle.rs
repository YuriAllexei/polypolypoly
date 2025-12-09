//! Test binary for Oracle Price Manager
//!
//! Connects to Polymarket's live data WebSocket and displays
//! real-time crypto prices from ChainLink and Binance oracles.
//!
//! Usage:
//!   cargo run --bin test_oracle

use anyhow::Result;
use chrono::Utc;
use polymarket::infrastructure::{spawn_oracle_trackers, OracleType, ShutdownManager};
use std::io::{self, Write};
use std::time::Duration;
use tokio::time::sleep;

/// Clear terminal and move cursor to top-left
fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
    io::stdout().flush().unwrap();
}

#[tokio::main]
async fn main() -> Result<()> {
    // Minimal logging - only errors
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();

    // Show initial message
    clear_screen();
    println!("════════════════════════════════════════════════════════════════");
    println!("🔮 Oracle Price Manager - Connecting...");
    println!("════════════════════════════════════════════════════════════════");
    println!("Press Ctrl+C to stop");

    let prices = spawn_oracle_trackers(shutdown.flag()).await?;

    // Wait a moment for initial data
    sleep(Duration::from_secs(2)).await;

    // Main loop: refresh display every second
    while shutdown.is_running() {
        if !shutdown.is_running() {
            break;
        }

        let manager = prices.read().unwrap();
        let chainlink_count = manager.symbol_count(OracleType::ChainLink);
        let binance_count = manager.symbol_count(OracleType::Binance);
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

        clear_screen();

        println!("════════════════════════════════════════════════════════════════");
        println!("🔮 ORACLE PRICE DASHBOARD");
        println!("════════════════════════════════════════════════════════════════");
        println!("  Last Update: {}", now);
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════");
        println!();

        // ChainLink prices
        println!("📈 CHAINLINK ({} symbols)", chainlink_count);
        println!("────────────────────────────────────────");
        if chainlink_count == 0 {
            println!("  (waiting for data...)");
        } else {
            let mut chainlink_prices: Vec<_> = manager
                .get_all_prices(OracleType::ChainLink)
                .iter()
                .collect();
            chainlink_prices.sort_by(|a, b| a.0.cmp(b.0));
            for (symbol, entry) in chainlink_prices {
                println!("  {:>6}  ${:>12.2}", symbol, entry.value);
            }
        }
        println!();

        // Binance prices
        println!("📊 BINANCE ({} symbols)", binance_count);
        println!("────────────────────────────────────────");
        if binance_count == 0 {
            println!("  (waiting for data...)");
        } else {
            let mut binance_prices: Vec<_> =
                manager.get_all_prices(OracleType::Binance).iter().collect();
            binance_prices.sort_by(|a, b| a.0.cmp(b.0));
            for (symbol, entry) in binance_prices {
                println!("  {:>6}  ${:>12.2}", symbol, entry.value);
            }
        }

        println!();
        println!("════════════════════════════════════════════════════════════════");

        drop(manager); // Release lock before sleeping
        sleep(Duration::from_millis(5)).await;
    }

    clear_screen();
    println!("Shutting down...");
    Ok(())
}
