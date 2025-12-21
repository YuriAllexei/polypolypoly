//! Quick test script for ActiveOrderManager
//!
//! Run with: cargo run --bin test-active-orders

use polymarket::infrastructure::client::TradingClient;
use polymarket::infrastructure::ActiveOrderManager;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    dotenv::dotenv().ok();

    println!("\n=== ActiveOrderManager Test ===\n");

    // Create trading client from env
    println!("Creating TradingClient from env...");
    let trading = Arc::new(TradingClient::from_env().await?);
    println!("  ✓ TradingClient created\n");

    // Test direct fetch first
    println!("Testing direct order fetch via TradingClient...");
    let orders = trading.get_orders(None).await?;
    println!("  ✓ Found {} active orders via direct fetch\n", orders.len());

    // Collect order IDs for later
    let mut order_ids: Vec<String> = Vec::new();

    if !orders.is_empty() {
        println!("Orders (direct fetch):");
        println!("{:-<80}", "");
        for order in &orders {
            let id = order.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let market = order.get("market").and_then(|v| v.as_str()).unwrap_or("?");
            let side = order.get("side").and_then(|v| v.as_str()).unwrap_or("?");
            let price = order.get("price").and_then(|v| v.as_str()).unwrap_or("?");
            let original_size = order.get("original_size").and_then(|v| v.as_str()).unwrap_or("?");
            let size_matched = order.get("size_matched").and_then(|v| v.as_str()).unwrap_or("0");

            order_ids.push(id.to_string());

            println!("  ID: {}", id);
            println!("    Market: {}", market);
            println!("    Side: {} | Price: {} | Size: {} (matched: {})", side, price, original_size, size_matched);
            println!();
        }
        println!("{:-<80}\n", "");
    }

    // Now test ActiveOrderManager
    println!("Starting ActiveOrderManager...");
    let shutdown_flag = Arc::new(AtomicBool::new(true));
    let mut manager = ActiveOrderManager::new();
    manager.start(Arc::clone(&trading), Arc::clone(&shutdown_flag)).await?;
    println!("  ✓ ActiveOrderManager started with {} orders\n", manager.order_count());

    // Let it run for a few seconds
    println!("Waiting 3 seconds to observe polling...\n");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Query the manager
    println!("Querying ActiveOrderManager state:");
    println!("  Total orders: {}", manager.order_count());

    // Show orders from manager
    if manager.order_count() > 0 {
        println!("\nOrders (from ActiveOrderManager):");
        println!("{:-<80}", "");

        for order_id in &order_ids {
            if let Some(active_order) = manager.get_order(order_id) {
                println!(
                    "  ID: {} | {} | ${} x {} (remaining: {:.2})",
                    active_order.order_id,
                    active_order.side,
                    active_order.price,
                    active_order.original_size,
                    active_order.remaining_size()
                );
            }
        }
        println!("{:-<80}", "");
    }

    // Shutdown
    println!("\nShutting down...");
    shutdown_flag.store(false, Ordering::Release);
    manager.stop().await;
    println!("  ✓ Done\n");

    Ok(())
}
