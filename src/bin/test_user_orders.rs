//! Test binary for User Order Tracker
//!
//! Connects to Polymarket's user WebSocket channel and displays
//! real-time order and trade updates.
//!
//! Requires environment variables:
//!   - API_KEY
//!   - API_SECRET
//!   - API_PASSPHRASE
//!
//! Usage:
//!   cargo run --bin test_user_orders

use anyhow::Result;
use chrono::Utc;
use polymarket::infrastructure::{spawn_user_order_tracker, OrderStatus, ShutdownManager};
use std::io::{self, Write};
use std::time::Duration;
use tokio::time::sleep;

/// Clear terminal and move cursor to top-left
fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
    io::stdout().flush().unwrap();
}

/// Format order status with color
fn format_status(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::Open => "OPEN",
        OrderStatus::PartiallyFilled => "PARTIAL",
        OrderStatus::Filled => "FILLED",
        OrderStatus::Cancelled => "CANCELLED",
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file (looks for .env in current directory or parent directories)
    dotenv::dotenv().ok();

    // Initialize logging - show info level for order/trade events
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();

    // Show initial message
    clear_screen();
    println!("════════════════════════════════════════════════════════════════");
    println!("User Order Tracker - Connecting...");
    println!("════════════════════════════════════════════════════════════════");
    println!("Press Ctrl+C to stop");
    println!();

    // Check for required env vars (loaded from .env file)
    if std::env::var("API_KEY").is_err() {
        eprintln!("ERROR: API_KEY not found in .env file or environment");
        eprintln!("Please ensure .env file contains API_KEY, API_SECRET, and API_PASSPHRASE");
        return Ok(());
    }

    let orders = spawn_user_order_tracker(shutdown.flag()).await?;

    // Wait a moment for connection
    sleep(Duration::from_secs(2)).await;

    // Main loop: refresh display every second
    while shutdown.is_running() {
        let manager = orders.read().unwrap();
        let order_count = manager.order_count();
        let trade_count = manager.trade_count();
        let asset_count = manager.asset_count();
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

        clear_screen();

        println!("════════════════════════════════════════════════════════════════");
        println!("USER ORDER DASHBOARD");
        println!("════════════════════════════════════════════════════════════════");
        println!("  Last Update: {}", now);
        println!(
            "  Orders: {} | Trades: {} | Assets: {}",
            order_count, trade_count, asset_count
        );
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════");
        println!();

        // Recent Orders
        println!("ORDERS ({} total)", order_count);
        println!("────────────────────────────────────────────────────────────────");
        if order_count == 0 {
            println!("  (no orders yet - place an order to see updates)");
        } else {
            // Show up to 10 most recent orders
            let mut all_orders: Vec<_> = manager.all_orders().collect();
            all_orders.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            for order in all_orders.iter().take(10) {
                println!(
                    "  {} {:>4} {:>3} @ {:>6} | {:>6}/{:>6} | {}",
                    format_status(order.status),
                    order.side,
                    order.outcome,
                    order.price,
                    order.size_matched,
                    order.original_size,
                    &order.order_id[..12.min(order.order_id.len())]
                );
            }
            if order_count > 10 {
                println!("  ... and {} more", order_count - 10);
            }
        }
        println!();

        // Recent Trades
        println!("TRADES ({} total)", trade_count);
        println!("────────────────────────────────────────────────────────────────");
        if trade_count == 0 {
            println!("  (no trades yet)");
        } else {
            // Show up to 10 most recent trades
            let mut all_trades: Vec<_> = manager.all_trades().collect();
            all_trades.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            for trade in all_trades.iter().take(10) {
                println!(
                    "  {:>10} {:>4} {:>3} @ {:>6} (size: {:>6}) | {}",
                    format!("{:?}", trade.status),
                    trade.side,
                    trade.outcome,
                    trade.price,
                    trade.size,
                    &trade.trade_id[..12.min(trade.trade_id.len())]
                );
            }
            if trade_count > 10 {
                println!("  ... and {} more", trade_count - 10);
            }
        }

        println!();
        println!("════════════════════════════════════════════════════════════════");
        println!("  Waiting for order/trade updates from WebSocket...");
        println!("════════════════════════════════════════════════════════════════");

        drop(manager); // Release lock before sleeping
        sleep(Duration::from_secs(1)).await;
    }

    clear_screen();
    println!("Shutting down...");
    Ok(())
}
