//! Test binary for orderbook verification
//!
//! Connects to Polymarket WebSocket and logs orderbook updates
//! to verify that orderbook processing is working correctly.
//!
//! Usage:
//!   cargo run --bin test_orderbook -- <token_id_1> <token_id_2> [outcome_1] [outcome_2]
//!
//! Example:
//!   cargo run --bin test_orderbook -- 123456... 789012... "Up" "Down"

use anyhow::Result;
use polymarket::client::clob::spawn_market_tracker;
use polymarket::database::MarketDatabase;
use polymarket::utils::{init_tracing, ShutdownManager};
use std::env;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        println!("Orderbook Test - WebSocket Connection");
        println!("");
        println!("Usage: {} <token_id_1> <token_id_2> [outcome_1] [outcome_2]", args[0]);
        println!("");
        println!("Arguments:");
        println!("  token_id_1   First token ID (e.g., Yes/Up outcome)");
        println!("  token_id_2   Second token ID (e.g., No/Down outcome)");
        println!("  outcome_1    Optional name for first outcome (default: 'Yes')");
        println!("  outcome_2    Optional name for second outcome (default: 'No')");
        println!("");
        println!("Example:");
        println!("  {} 27883770341662... 91848735676346... Up Down", args[0]);
        return Ok(());
    }

    let token_ids = vec![args[1].clone(), args[2].clone()];
    let outcomes = vec![
        args.get(3).cloned().unwrap_or_else(|| "Yes".to_string()),
        args.get(4).cloned().unwrap_or_else(|| "No".to_string()),
    ];

    // Use token IDs as market ID for logging
    let market_id = format!("test_{}", &token_ids[0][..8.min(token_ids[0].len())]);

    info!("Orderbook Test - Connecting to Polymarket WebSocket");
    info!("Press Ctrl+C to stop");
    info!("");
    info!("Token IDs:");
    info!("  [{}] {}", outcomes[0], token_ids[0]);
    info!("  [{}] {}", outcomes[1], token_ids[1]);
    info!("");

    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();

    info!("Connecting to WebSocket and subscribing to orderbook...");
    info!("Watch for orderbook snapshots and updates:");
    info!("");

    // Use a far-future resolution time so it doesn't auto-close
    let resolution_time = "2099-12-31T23:59:59Z".to_string();

    // Create in-memory database for testing (opportunities won't be recorded
    // since we use threshold 1.0 which is unreachable)
    let db = Arc::new(MarketDatabase::new(":memory:").await?);

    spawn_market_tracker(
        market_id,
        token_ids,
        outcomes,
        resolution_time,
        shutdown.flag(),
        db,
        1.0,  // High threshold means no opportunities recorded during test
        None, // No event_id for test
    )
    .await?;

    info!("");
    info!("Orderbook test completed");
    Ok(())
}
