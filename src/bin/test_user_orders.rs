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
use polymarket::infrastructure::{OrderManager, OrderStatus, ShutdownManager};
use std::io::{self, Write};
use std::sync::Arc;
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

    let shutdown = Arc::new(ShutdownManager::new());
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

    // Create and start OrderManager
    let mut order_manager = OrderManager::new();
    order_manager.start(shutdown.flag()).await?;

    // Wait a moment for connection
    sleep(Duration::from_secs(2)).await;

    // Main loop: refresh display every second
    while shutdown.is_running() {
        let order_count = order_manager.order_count();
        let fill_count = order_manager.fill_count();
        let asset_count = order_manager.asset_count();
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

        clear_screen();

        println!("════════════════════════════════════════════════════════════════");
        println!("USER ORDER DASHBOARD");
        println!("════════════════════════════════════════════════════════════════");
        println!("  Last Update: {}", now);
        println!(
            "  Orders: {} | Fills: {} | Assets: {}",
            order_count, fill_count, asset_count
        );
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════");
        println!();

        // Per-asset breakdown
        let asset_ids = order_manager.asset_ids();
        for asset_id in asset_ids.iter().take(5) {
            let short_id = &asset_id[..12.min(asset_id.len())];
            let bids = order_manager.get_bids(asset_id);
            let asks = order_manager.get_asks(asset_id);
            let fills = order_manager.get_fills(asset_id);
            let total_bid = order_manager.total_bid_size(asset_id);
            let total_ask = order_manager.total_ask_size(asset_id);

            println!("ASSET: {}...", short_id);
            println!(
                "  Bids: {} (size: {:.2}) | Asks: {} (size: {:.2}) | Fills: {}",
                bids.len(),
                total_bid,
                asks.len(),
                total_ask,
                fills.len()
            );

            // Show open orders
            let open_orders = order_manager.get_open_orders(asset_id);
            for order in open_orders.iter().take(3) {
                println!(
                    "    {} {} {:>4} @ {:>6} | {:>6}/{:>6} | {}",
                    format_status(order.status),
                    order.side,
                    order.outcome,
                    order.price,
                    order.size_matched,
                    order.original_size,
                    &order.order_id[..12.min(order.order_id.len())]
                );
            }
            println!();
        }

        if asset_ids.is_empty() {
            println!("  (no orders yet - place an order to see updates)");
            println!();
        }

        println!("════════════════════════════════════════════════════════════════");
        println!("  Waiting for order/trade updates from WebSocket...");
        println!("════════════════════════════════════════════════════════════════");

        sleep(Duration::from_secs(1)).await;
    }

    clear_screen();
    println!("Shutting down...");

    // Stop order manager
    order_manager.stop().await;

    Ok(())
}
