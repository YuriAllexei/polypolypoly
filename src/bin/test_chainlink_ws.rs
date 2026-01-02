//! Test binary for ChainLink Data Streams WebSocket
//!
//! Connects directly to ChainLink's Data Streams WebSocket API and displays
//! real-time crypto prices with HMAC authentication.
//!
//! Usage:
//!   cargo run --bin test_chainlink_ws
//!
//! Required environment variables:
//!   CHAINLINK_CLIENT_ID - Your ChainLink client ID (UUID)
//!   STREAMS_SECRET - Your ChainLink streams secret

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;
use polymarket::infrastructure::client::oracle::{
    spawn_chainlink_tracker, FeedIdMap, OraclePriceManager, OracleType, SharedOraclePrices,
};
use polymarket::infrastructure::ShutdownManager;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Clear terminal and move cursor to top-left
fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
    io::stdout().flush().unwrap();
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenv::dotenv().ok();

    // Initialize logging with debug level for chainlink
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("polymarket::infrastructure::client::oracle::chainlink_ws=debug".parse().unwrap())
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    // Check required env vars
    if std::env::var("CHAINLINK_CLIENT_ID").is_err() {
        eprintln!("Error: CHAINLINK_CLIENT_ID environment variable not set");
        eprintln!("Please set CHAINLINK_CLIENT_ID and STREAMS_SECRET in your .env file");
        std::process::exit(1);
    }
    if std::env::var("STREAMS_SECRET").is_err() {
        eprintln!("Error: STREAMS_SECRET environment variable not set");
        eprintln!("Please set CHAINLINK_CLIENT_ID and STREAMS_SECRET in your .env file");
        std::process::exit(1);
    }

    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();

    // Show initial message
    clear_screen();
    println!("════════════════════════════════════════════════════════════════");
    println!("ChainLink Data Streams WebSocket - Connecting...");
    println!("════════════════════════════════════════════════════════════════");

    // Show feed IDs being tracked
    let feed_map = FeedIdMap::new();
    println!("\nTracking feeds:");
    for symbol in feed_map.symbols() {
        if let Some(feed_id) = feed_map.get_feed_id(symbol) {
            println!("  {} -> {}", symbol, feed_id);
        }
    }
    println!("\nPress Ctrl+C to stop\n");

    // Create shared price manager
    let prices: SharedOraclePrices = Arc::new(RwLock::new(OraclePriceManager::new()));

    // Spawn ChainLink tracker
    let tracker_prices = Arc::clone(&prices);
    let tracker_shutdown = shutdown.flag();
    tokio::spawn(async move {
        if let Err(e) = spawn_chainlink_tracker(tracker_prices, tracker_shutdown).await {
            eprintln!("ChainLink tracker error: {}", e);
        }
    });

    // Wait a moment for initial connection
    sleep(Duration::from_secs(3)).await;

    // Main loop: refresh display every second
    while shutdown.is_running() {
        let manager = prices.read();
        let chainlink_count = manager.symbol_count(OracleType::ChainLink);
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let oracle_age = manager.oracle_age(OracleType::ChainLink);
        let msg_count = manager.oracle_message_count(OracleType::ChainLink);

        clear_screen();

        println!("════════════════════════════════════════════════════════════════");
        println!("CHAINLINK DATA STREAMS - LIVE PRICES");
        println!("════════════════════════════════════════════════════════════════");
        println!("  Last Update: {}", now);
        println!("  Oracle Age:  {:.1}s", oracle_age.as_secs_f64());
        println!("  Messages:    {}", msg_count);
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════");
        println!();

        // ChainLink prices
        println!("PRICES ({} symbols)", chainlink_count);
        println!("────────────────────────────────────────");
        if chainlink_count == 0 {
            println!("  (waiting for data...)");
            println!();
            println!("  If no data appears, check:");
            println!("  1. CHAINLINK_CLIENT_ID is set correctly");
            println!("  2. STREAMS_SECRET is set correctly");
            println!("  3. Network connectivity to ws.dataengine.chain.link");
        } else {
            let mut chainlink_prices: Vec<_> = manager
                .get_all_prices(OracleType::ChainLink)
                .iter()
                .collect();
            chainlink_prices.sort_by(|a, b| a.0.cmp(b.0));
            for (symbol, entry) in chainlink_prices {
                let age_ms = entry.age().as_millis();
                println!(
                    "  {:>6}  ${:>12.2}  (age: {}ms)",
                    symbol, entry.value, age_ms
                );
            }
        }

        println!();
        println!("════════════════════════════════════════════════════════════════");

        drop(manager); // Release lock before sleeping
        sleep(Duration::from_millis(100)).await;
    }

    clear_screen();
    println!("Shutting down...");
    Ok(())
}
