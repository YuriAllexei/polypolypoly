//! Test binary for Binance Direct Price Feed
//!
//! Connects directly to Binance WebSocket for real-time crypto prices.
//! Shows latency stats to compare with Polymarket's relay (~50-100ms slower).
//!
//! Usage:
//!   cargo run --bin test_binance

use anyhow::Result;
use chrono::Utc;
use polymarket::infrastructure::{spawn_binance_tracker, BinanceAsset, ShutdownManager};
use std::io::{self, Write};
use std::time::Duration;
use tokio::time::sleep;

/// Clear terminal and move cursor to top-left
fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
    io::stdout().flush().unwrap();
}

/// Format latency with color coding
fn format_latency(latency_ms: i64) -> String {
    if latency_ms < 0 {
        format!("\x1B[33m{:>6}ms\x1B[0m", latency_ms) // Yellow for negative (clock issue)
    } else if latency_ms < 50 {
        format!("\x1B[32m{:>6}ms\x1B[0m", latency_ms) // Green for excellent
    } else if latency_ms < 100 {
        format!("\x1B[33m{:>6}ms\x1B[0m", latency_ms) // Yellow for okay
    } else {
        format!("\x1B[31m{:>6}ms\x1B[0m", latency_ms) // Red for slow
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Minimal logging - only warnings/errors
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();

    // Show initial message
    clear_screen();
    println!("════════════════════════════════════════════════════════════════════════");
    println!("  BINANCE DIRECT PRICE FEED - Connecting...");
    println!("════════════════════════════════════════════════════════════════════════");
    println!("  URL: wss://stream.binance.com:9443/stream");
    println!("  Assets: BTC, ETH, SOL, XRP");
    println!("  Press Ctrl+C to stop");
    println!("════════════════════════════════════════════════════════════════════════");

    let prices = spawn_binance_tracker(shutdown.flag()).await?;

    // Wait a moment for initial data
    sleep(Duration::from_secs(2)).await;

    // Main loop: refresh display
    while shutdown.is_running() {
        let manager = prices.read();
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let trade_count = manager.trade_count();
        let avg_latency = manager.avg_latency_ms();
        let min_latency = manager.min_latency_ms();
        let max_latency = manager.max_latency_ms();
        let age_ms = manager.age().as_millis();

        clear_screen();

        println!("════════════════════════════════════════════════════════════════════════");
        println!("  BINANCE DIRECT PRICE FEED");
        println!("════════════════════════════════════════════════════════════════════════");
        println!("  Time: {}    Feed Age: {}ms", now, age_ms);
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════════════");
        println!();

        // Price table header
        println!("  {:>6}  {:>14}  {:>10}  {:>12}  {:>10}",
                 "ASSET", "PRICE", "LATENCY", "TRADE ID", "SIDE");
        println!("  ──────────────────────────────────────────────────────────────────");

        // Display prices for each asset
        for asset in BinanceAsset::all() {
            if let Some(entry) = manager.get_price_by_asset(*asset) {
                let side = if entry.is_sell { "SELL" } else { "BUY" };
                let side_color = if entry.is_sell { "\x1B[31m" } else { "\x1B[32m" };
                println!(
                    "  {:>6}  ${:>13.2}  {}  {:>12}  {}{:>10}\x1B[0m",
                    asset.symbol(),
                    entry.value,
                    format_latency(entry.latency_ms),
                    entry.trade_id,
                    side_color,
                    side
                );
            } else {
                println!("  {:>6}  {:>14}  {:>10}  {:>12}  {:>10}",
                         asset.symbol(), "(waiting...)", "-", "-", "-");
            }
        }

        println!();
        println!("  ══════════════════════════════════════════════════════════════════");
        println!("  LATENCY STATS");
        println!("  ──────────────────────────────────────────────────────────────────");

        if trade_count > 0 {
            println!("  Trades Received: {:>10}", trade_count);
            println!("  Avg Latency:     {:>10.1}ms", avg_latency);
            if min_latency != i64::MAX {
                println!("  Min Latency:     {:>10}ms", min_latency);
            }
            if max_latency != i64::MIN {
                println!("  Max Latency:     {:>10}ms", max_latency);
            }
        } else {
            println!("  (waiting for trades...)");
        }

        println!();
        println!("  ══════════════════════════════════════════════════════════════════");
        println!("  LATENCY LEGEND");
        println!("  ──────────────────────────────────────────────────────────────────");
        println!("  \x1B[32m<50ms\x1B[0m = Excellent  \x1B[33m50-100ms\x1B[0m = OK  \x1B[31m>100ms\x1B[0m = Slow");
        println!("  Compare with Polymarket relay: typically 50-100ms slower");
        println!("════════════════════════════════════════════════════════════════════════");

        drop(manager); // Release lock before sleeping
        sleep(Duration::from_millis(100)).await;
    }

    clear_screen();
    println!("Shutting down...");
    Ok(())
}
