//! Test Binary for OrderManager
//!
//! Tests the WebSocket connection and order tracking functionality.
//!
//! Usage:
//!   cargo run --bin test_order_manager
//!
//! Required environment variables:
//!   - API_KEY
//!   - API_SECRET
//!   - API_PASSPHRASE

use anyhow::Result;
use polymarket::infrastructure::{init_tracing_with_level, OrderManager, ShutdownManager};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenv::dotenv().ok();

    // Initialize logging
    init_tracing_with_level("info");

    info!("===========================================");
    info!("OrderManager Test");
    info!("===========================================");
    info!("");

    // Create shutdown manager
    let shutdown = Arc::new(ShutdownManager::new());
    shutdown.spawn_signal_handler();

    // Create and start OrderManager
    let mut order_manager = OrderManager::new();
    order_manager.start(shutdown.flag()).await?;

    info!("");
    info!("OrderManager started. Waiting for messages...");
    info!("Press Ctrl+C to stop.");
    info!("");

    // Main loop - print stats periodically
    let mut last_order_count = 0;
    let mut last_fill_count = 0;

    while shutdown.flag().load(std::sync::atomic::Ordering::Acquire) {
        sleep(Duration::from_secs(5)).await;

        let order_count = order_manager.order_count();
        let fill_count = order_manager.fill_count();
        let asset_count = order_manager.asset_count();

        // Only log if counts changed
        if order_count != last_order_count || fill_count != last_fill_count {
            info!(
                "[Stats] Orders: {} (+{}), Fills: {} (+{}), Assets: {}",
                order_count,
                order_count.saturating_sub(last_order_count),
                fill_count,
                fill_count.saturating_sub(last_fill_count),
                asset_count
            );

            // Log per-asset breakdown
            for asset_id in order_manager.asset_ids() {
                let bids = order_manager.get_bids(&asset_id).len();
                let asks = order_manager.get_asks(&asset_id).len();
                let fills = order_manager.get_fills(&asset_id).len();
                let total_bid = order_manager.total_bid_size(&asset_id);
                let total_ask = order_manager.total_ask_size(&asset_id);
                info!(
                    "  Asset {}...: bids={} ({:.2}), asks={} ({:.2}), fills={}",
                    &asset_id[..12.min(asset_id.len())],
                    bids, total_bid,
                    asks, total_ask,
                    fills
                );
            }

            last_order_count = order_count;
            last_fill_count = fill_count;
        }
    }

    info!("");
    info!("Shutting down...");

    // Stop order manager
    order_manager.stop().await;

    info!("");
    info!("===========================================");
    info!("OrderManager test completed");
    info!("===========================================");

    Ok(())
}
