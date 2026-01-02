//! Detailed OMS (Order Management System) Test Binary
//!
//! Tests the OrderStateStore with detailed status tracking, pending_cancels,
//! and real-time order state updates from WebSocket.
//!
//! This binary helps verify:
//! - WebSocket connection and message handling
//! - Order status transitions (PLACEMENT -> UPDATE -> CANCELLATION)
//! - Pending cancellations tracking
//! - Order counts by status per asset
//! - REST API hydration
//!
//! Usage:
//!   cargo run --bin test_oms_detailed
//!
//! Required environment variables:
//!   - PRIVATE_KEY (or API_KEY, API_SECRET, API_PASSPHRASE)
//!   - CLOB_URL (optional, defaults to https://clob.polymarket.com)

use anyhow::Result;
use chrono::Utc;
use polymarket::infrastructure::client::clob::{RestClient, POLYGON_CHAIN_ID};
use polymarket::infrastructure::client::user::{
    spawn_user_order_tracker, Order, OrderEventCallback, OrderStatus, SharedOrderState,
};
use polymarket::infrastructure::client::PolymarketAuth;
use polymarket::infrastructure::ShutdownManager;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

const CLOB_URL: &str = "https://clob.polymarket.com";

/// Clear terminal and move cursor to top-left
fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
    io::stdout().flush().unwrap();
}

/// Format order status with color codes
fn format_status(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::Open => "\x1B[32mOPEN\x1B[0m",            // Green
        OrderStatus::PartiallyFilled => "\x1B[33mPARTIAL\x1B[0m", // Yellow
        OrderStatus::Filled => "\x1B[36mFILLED\x1B[0m",        // Cyan
        OrderStatus::Cancelled => "\x1B[31mCANCELLED\x1B[0m",  // Red
    }
}

/// Callback to log all OMS events
struct OmsEventLogger;

impl OrderEventCallback for OmsEventLogger {
    fn on_order_placed(&self, order: &Order) {
        info!(
            "\x1B[32m[EVENT] PLACED\x1B[0m: {} {} @ {} (id: {}...)",
            order.side,
            order.outcome,
            order.price,
            &order.order_id[..12.min(order.order_id.len())]
        );
    }

    fn on_order_updated(&self, order: &Order) {
        info!(
            "\x1B[33m[EVENT] UPDATE\x1B[0m: {} {} @ {} ({}/{} matched)",
            order.side,
            order.outcome,
            order.price,
            order.size_matched,
            order.original_size
        );
    }

    fn on_order_cancelled(&self, order: &Order) {
        info!(
            "\x1B[31m[EVENT] CANCELLED\x1B[0m: {} {} @ {} (id: {}...)",
            order.side,
            order.outcome,
            order.price,
            &order.order_id[..12.min(order.order_id.len())]
        );
    }

    fn on_order_filled(&self, order: &Order) {
        info!(
            "\x1B[36m[EVENT] FILLED\x1B[0m: {} {} @ {} (size: {})",
            order.side,
            order.outcome,
            order.price,
            order.original_size
        );
    }

    fn on_trade(&self, fill: &polymarket::infrastructure::client::user::Fill) {
        info!(
            "\x1B[35m[EVENT] TRADE\x1B[0m: {} {} {:.2} @ ${:.4} (status: {})",
            fill.side,
            &fill.asset_id[..8.min(fill.asset_id.len())],
            fill.size,
            fill.price,
            fill.status
        );
    }
}

/// Stats for order counts by status
#[derive(Default, Debug)]
struct AssetStats {
    open: usize,
    partial: usize,
    filled: usize,
    cancelled: usize,
    total_bids: usize,
    total_asks: usize,
}

impl AssetStats {
    fn total(&self) -> usize {
        self.open + self.partial + self.filled + self.cancelled
    }
}

fn calculate_asset_stats(state: &SharedOrderState, asset_id: &str) -> AssetStats {
    let store = state.read();
    let bids = store.get_bids(asset_id);
    let asks = store.get_asks(asset_id);

    let mut stats = AssetStats::default();
    stats.total_bids = bids.len();
    stats.total_asks = asks.len();

    for order in bids.iter().chain(asks.iter()) {
        match order.status {
            OrderStatus::Open => stats.open += 1,
            OrderStatus::PartiallyFilled => stats.partial += 1,
            OrderStatus::Filled => stats.filled += 1,
            OrderStatus::Cancelled => stats.cancelled += 1,
        }
    }

    stats
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenv::dotenv().ok();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let shutdown = Arc::new(ShutdownManager::new());
    shutdown.spawn_signal_handler();

    clear_screen();
    println!("════════════════════════════════════════════════════════════════");
    println!("DETAILED OMS TEST");
    println!("════════════════════════════════════════════════════════════════");
    println!("Press Ctrl+C to stop");
    println!();

    // Setup authentication
    let private_key = std::env::var("PRIVATE_KEY").ok();
    let api_key = std::env::var("API_KEY").ok();
    let api_secret = std::env::var("API_SECRET").ok();
    let api_passphrase = std::env::var("API_PASSPHRASE").ok();

    let auth = if let Some(pk) = private_key {
        info!("Using PRIVATE_KEY for authentication");
        let mut auth = PolymarketAuth::new(&pk, POLYGON_CHAIN_ID)?;

        // If API credentials are set, use them; otherwise derive
        if let (Some(key), Some(secret), Some(pass)) = (api_key, api_secret, api_passphrase) {
            info!("Using provided API credentials");
            auth.set_api_key(polymarket::infrastructure::client::clob::ApiCredentials {
                key,
                secret,
                passphrase: pass,
            });
        } else {
            info!("Deriving API credentials from private key...");
            let rest_client = RestClient::new(CLOB_URL);
            let creds = rest_client.get_or_create_api_creds(&auth).await?;
            auth.set_api_key(creds);
        }
        auth
    } else if let (Some(key), Some(secret), Some(pass)) = (api_key, api_secret, api_passphrase) {
        info!("Using API credentials only (L2 auth)");
        PolymarketAuth::from_api_credentials(
            polymarket::infrastructure::client::clob::ApiCredentials {
                key,
                secret,
                passphrase: pass,
            },
        )
    } else {
        return Err(anyhow::anyhow!(
            "No authentication configured. Set PRIVATE_KEY or API_KEY/API_SECRET/API_PASSPHRASE"
        ));
    };

    // Create REST client for hydration
    let rest_client = RestClient::new(CLOB_URL);

    // Create callback for event logging
    let callback = Arc::new(OmsEventLogger);

    // Spawn user order tracker with REST hydration
    info!("Starting OMS tracker with REST hydration...");
    let state = spawn_user_order_tracker(
        shutdown.flag(),
        &rest_client,
        &auth,
        Some(callback),
    )
    .await?;

    // Wait for initial hydration
    sleep(Duration::from_secs(2)).await;

    // Main display loop
    let mut iteration = 0u64;
    let mut last_asset_stats: HashMap<String, AssetStats> = HashMap::new();

    while shutdown.flag().load(Ordering::Acquire) {
        iteration += 1;
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

        // Only clear screen every 10 iterations to reduce flicker
        if iteration % 10 == 1 {
            clear_screen();
        } else {
            // Move cursor to top
            print!("\x1B[1;1H");
        }

        println!("════════════════════════════════════════════════════════════════");
        println!("DETAILED OMS TEST - Real-time Order State");
        println!("════════════════════════════════════════════════════════════════");
        println!("  Last Update: {} (tick #{})", now, iteration);
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════");
        println!();

        // Global stats
        let store = state.read();
        let order_count = store.order_count();
        let fill_count = store.fill_count();
        let asset_count = store.asset_count();
        let asset_ids = store.asset_ids();

        // Count orders by status globally
        let mut global_open = 0;
        let mut global_partial = 0;
        let mut global_filled = 0;
        let mut global_cancelled = 0;

        for asset_id in &asset_ids {
            for order in store.get_bids(asset_id).iter().chain(store.get_asks(asset_id).iter()) {
                match order.status {
                    OrderStatus::Open => global_open += 1,
                    OrderStatus::PartiallyFilled => global_partial += 1,
                    OrderStatus::Filled => global_filled += 1,
                    OrderStatus::Cancelled => global_cancelled += 1,
                }
            }
        }
        drop(store); // Release lock before printing

        println!("GLOBAL STATS");
        println!("────────────────────────────────────────────────────────────────");
        println!(
            "  Total Orders: {} | Fills: {} | Assets: {}",
            order_count, fill_count, asset_count
        );
        println!(
            "  By Status: {} OPEN | {} PARTIAL | {} FILLED | {} CANCELLED",
            global_open, global_partial, global_filled, global_cancelled
        );
        println!();

        // Per-asset breakdown (only show non-empty assets, limit to 5)
        println!("PER-ASSET BREAKDOWN");
        println!("────────────────────────────────────────────────────────────────");

        let mut shown = 0;
        for asset_id in asset_ids.iter() {
            let stats = calculate_asset_stats(&state, asset_id);

            // Only show assets with active orders
            if stats.open > 0 || stats.partial > 0 {
                shown += 1;
                if shown > 5 {
                    println!("  ... and {} more assets", asset_ids.len() - 5);
                    break;
                }

                let short_id = &asset_id[..12.min(asset_id.len())];
                println!(
                    "  {}...: {} open, {} partial, {} filled, {} cancelled (bids={}, asks={})",
                    short_id,
                    stats.open,
                    stats.partial,
                    stats.filled,
                    stats.cancelled,
                    stats.total_bids,
                    stats.total_asks
                );

                // Show open orders for this asset
                let store = state.read();
                let open_orders = store.get_open_orders(asset_id);
                for order in open_orders.iter().take(3) {
                    println!(
                        "    {} {} {:>4} @ {:>6} | {:>6}/{:>6} | {}",
                        format_status(order.status),
                        order.side,
                        order.outcome,
                        format!("${:.4}", order.price),
                        format!("{:.2}", order.size_matched),
                        format!("{:.2}", order.original_size),
                        &order.order_id[..12.min(order.order_id.len())]
                    );
                }
                if open_orders.len() > 3 {
                    println!("    ... and {} more open orders", open_orders.len() - 3);
                }
                println!();

                // Check for status changes
                if let Some(prev_stats) = last_asset_stats.get(asset_id) {
                    if prev_stats.open != stats.open || prev_stats.cancelled != stats.cancelled {
                        info!(
                            "Status change for {}...: open {} -> {}, cancelled {} -> {}",
                            short_id,
                            prev_stats.open,
                            stats.open,
                            prev_stats.cancelled,
                            stats.cancelled
                        );
                    }
                }
                last_asset_stats.insert(asset_id.clone(), stats);
            }
        }

        if shown == 0 {
            println!("  (no active orders - place an order to see updates)");
            println!();
        }

        println!("════════════════════════════════════════════════════════════════");
        println!("  Watching for WebSocket updates...");
        println!("════════════════════════════════════════════════════════════════");

        sleep(Duration::from_millis(500)).await;
    }

    println!();
    println!("Shutting down...");

    Ok(())
}
