//! Real-time Position Tracker Test Binary
//!
//! Tests the PositionTracker with real-time fill updates from WebSocket.
//!
//! This binary helps verify:
//! - Position tracking from confirmed fills
//! - Average entry price calculation
//! - Realized P&L tracking
//! - Token pair registration
//! - Merge opportunity detection
//! - Only CONFIRMED trades are processed
//!
//! Usage:
//!   cargo run --bin test_position_tracker_realtime -- [token_a] [token_b] [condition_id]
//!
//! Examples:
//!   # Run without token pair (no merge detection)
//!   cargo run --bin test_position_tracker_realtime
//!
//!   # Run with token pair for merge detection
//!   cargo run --bin test_position_tracker_realtime -- \
//!     "123456789..." "987654321..." "condition_abc"
//!
//! Required environment variables:
//!   - PRIVATE_KEY (or API_KEY, API_SECRET, API_PASSPHRASE)

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;
use polymarket::infrastructure::client::clob::{RestClient, POLYGON_CHAIN_ID};
use polymarket::infrastructure::client::user::{
    spawn_user_order_tracker, Fill, Order, OrderEventCallback, Position,
    PositionEvent, PositionEventCallback, PositionTracker, PositionTrackerBridge,
    SharedOrderState, SharedPositionTracker,
};
use polymarket::infrastructure::client::PolymarketAuth;
use polymarket::infrastructure::ShutdownManager;
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

/// Position event logger
struct PositionEventLogger;

impl PositionEventCallback for PositionEventLogger {
    fn on_position_updated(&self, event: &PositionEvent) {
        match event {
            PositionEvent::Updated {
                token_id,
                old_position,
                new_position,
                fill,
            } => {
                let old_size = old_position
                    .as_ref()
                    .map(|p| p.size)
                    .unwrap_or(0.0);
                info!(
                    "\x1B[36m[POSITION] UPDATE\x1B[0m: {}... | {:.2} -> {:.2} | avg ${:.4} | realized P&L: ${:.2}",
                    &token_id[..8.min(token_id.len())],
                    old_size,
                    new_position.size,
                    new_position.avg_entry_price,
                    new_position.realized_pnl
                );
                info!(
                    "  Fill: {} {:.2} @ ${:.4} (status: {})",
                    fill.side, fill.size, fill.price, fill.status
                );
            }
            PositionEvent::MergeOpportunity(merge) => {
                info!(
                    "\x1B[35m[MERGE] OPPORTUNITY\x1B[0m: {:.2} pairs | profit: ${:.2} ({:.1}%)",
                    merge.mergeable_pairs,
                    merge.potential_profit,
                    merge.profit_percentage()
                );
            }
            PositionEvent::NoOp => {
                // Duplicate trade - already processed, no action needed
            }
        }
    }
}

/// Dual callback that forwards to both position tracker bridge and our logger
struct DualCallback {
    bridge: Arc<PositionTrackerBridge>,
    logger: Arc<PositionEventLogger>,
    tracker: SharedPositionTracker,
}

impl DualCallback {
    fn new(tracker: SharedPositionTracker) -> Self {
        let bridge = Arc::new(PositionTrackerBridge::new(tracker.clone()));
        Self {
            bridge,
            logger: Arc::new(PositionEventLogger),
            tracker,
        }
    }
}

impl OrderEventCallback for DualCallback {
    fn on_order_placed(&self, order: &Order) {
        self.bridge.on_order_placed(order);
    }

    fn on_order_updated(&self, order: &Order) {
        self.bridge.on_order_updated(order);
    }

    fn on_order_cancelled(&self, order: &Order) {
        self.bridge.on_order_cancelled(order);
    }

    fn on_order_filled(&self, order: &Order) {
        self.bridge.on_order_filled(order);
    }

    fn on_trade(&self, fill: &Fill) {
        // Forward to bridge (updates position on MATCHED)
        self.bridge.on_trade(fill);

        // Log the position event if this was a MATCHED trade (position-changing)
        if fill.status == polymarket::infrastructure::client::user::TradeStatus::Matched {
            // Get the updated position for logging
            if let Some(pos) = self.tracker.read().get_position(&fill.asset_id) {
                let event = PositionEvent::Updated {
                    token_id: fill.asset_id.clone(),
                    old_position: None,
                    new_position: pos.clone(),
                    fill: fill.clone(),
                };
                self.logger.on_position_updated(&event);
            }
        }
    }
}

/// Format position for display
fn format_position(pos: &Position) -> String {
    let direction = if pos.is_long() {
        "\x1B[32mLONG\x1B[0m"
    } else if pos.is_short() {
        "\x1B[31mSHORT\x1B[0m"
    } else {
        "\x1B[90mFLAT\x1B[0m"
    };

    format!(
        "{:>6} {:>8.2} @ ${:.4} | Cost: ${:>8.2} | P&L: ${:>8.2} | Fees: ${:.2}",
        direction,
        pos.size,
        pos.avg_entry_price,
        pos.cost_basis,
        pos.realized_pnl,
        pos.total_fees
    )
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

    // Parse command line args for token pair registration
    let args: Vec<String> = std::env::args().collect();
    let token_pair = if args.len() >= 4 {
        Some((args[1].clone(), args[2].clone(), args[3].clone()))
    } else {
        None
    };

    let shutdown = Arc::new(ShutdownManager::new());
    shutdown.spawn_signal_handler();

    clear_screen();
    println!("════════════════════════════════════════════════════════════════");
    println!("REAL-TIME POSITION TRACKER TEST");
    println!("════════════════════════════════════════════════════════════════");
    println!("Press Ctrl+C to stop");
    if let Some((ref a, ref b, ref c)) = token_pair {
        println!("Token Pair: {}... <-> {}...", &a[..8.min(a.len())], &b[..8.min(b.len())]);
        println!("Condition: {}...", &c[..20.min(c.len())]);
    } else {
        println!("No token pair registered (merge detection disabled)");
    }
    println!();

    // Setup authentication
    let private_key = std::env::var("PRIVATE_KEY").ok();
    let api_key = std::env::var("API_KEY").ok();
    let api_secret = std::env::var("API_SECRET").ok();
    let api_passphrase = std::env::var("API_PASSPHRASE").ok();

    let auth = if let Some(pk) = private_key {
        info!("Using PRIVATE_KEY for authentication");
        let mut auth = PolymarketAuth::new(&pk, POLYGON_CHAIN_ID)?;

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

    // Create position tracker
    let position_tracker: SharedPositionTracker = Arc::new(RwLock::new(PositionTracker::new()));

    // Register token pair if provided
    if let Some((ref token_a, ref token_b, ref condition_id)) = token_pair {
        position_tracker
            .write()
            .register_token_pair(token_a, token_b, condition_id);
        info!("Registered token pair for merge detection");
    }

    // Create REST client for hydration
    let rest_client = RestClient::new(CLOB_URL);

    // Get proxy wallet address (where positions are held)
    let proxy_wallet: ethers::types::Address = std::env::var("PROXY_WALLET")
        .map_err(|_| anyhow::anyhow!("PROXY_WALLET not set"))?
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid PROXY_WALLET address"))?;

    // Hydrate positions from REST API
    info!("Fetching initial positions from REST API...");
    match rest_client.get_positions(proxy_wallet).await {
        Ok(positions) => {
            let mut tracker = position_tracker.write();
            let mut count = 0;
            for pos in positions {
                if let Ok(size) = pos.size.parse::<f64>() {
                    if size.abs() > 0.001 {
                        tracker.hydrate_position(&pos.asset_id, size, 0.0);
                        count += 1;
                    }
                }
            }
            info!("Hydrated {} positions from REST API", count);
        }
        Err(e) => {
            info!("Could not fetch positions: {} (will build from fills)", e);
        }
    }

    // Create dual callback
    let callback = Arc::new(DualCallback::new(position_tracker.clone()));

    // Spawn user order tracker
    info!("Starting order tracker with position bridge...");
    let _state: SharedOrderState = spawn_user_order_tracker(
        shutdown.flag(),
        &rest_client,
        &auth,
        Some(callback),
    )
    .await?;

    // Wait for initial connection
    sleep(Duration::from_secs(2)).await;

    // Main display loop
    let mut iteration = 0u64;

    while shutdown.flag().load(Ordering::Acquire) {
        iteration += 1;
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

        // Only clear screen every 10 iterations
        if iteration % 10 == 1 {
            clear_screen();
        } else {
            print!("\x1B[1;1H");
        }

        println!("════════════════════════════════════════════════════════════════");
        println!("REAL-TIME POSITION TRACKER TEST");
        println!("════════════════════════════════════════════════════════════════");
        println!("  Last Update: {} (tick #{})", now, iteration);
        println!("  Press Ctrl+C to stop");
        println!("════════════════════════════════════════════════════════════════");
        println!();

        // Get position stats
        let tracker = position_tracker.read();
        let positions = tracker.get_all_positions();
        let total_realized_pnl = tracker.get_total_realized_pnl();
        let total_fees = tracker.get_total_fees();
        let merge_opportunities = tracker.get_merge_opportunities();

        println!("SUMMARY");
        println!("────────────────────────────────────────────────────────────────");
        println!(
            "  Positions: {} | Total Realized P&L: ${:.2} | Total Fees: ${:.2}",
            positions.len(),
            total_realized_pnl,
            total_fees
        );
        println!();

        // Show positions
        println!("POSITIONS (only showing non-flat)");
        println!("────────────────────────────────────────────────────────────────");

        let mut shown = 0;
        for pos in positions.iter() {
            if !pos.is_flat() {
                shown += 1;
                let short_id = &pos.token_id[..12.min(pos.token_id.len())];
                println!("  {}...: {}", short_id, format_position(pos));
            }
        }

        if shown == 0 {
            println!("  (no open positions - place and fill an order to see updates)");
        }
        println!();

        // Show merge opportunities
        if !merge_opportunities.is_empty() {
            println!("MERGE OPPORTUNITIES");
            println!("────────────────────────────────────────────────────────────────");
            for merge in &merge_opportunities {
                let status = if merge.is_profitable() {
                    "\x1B[32mPROFITABLE\x1B[0m"
                } else {
                    "\x1B[31mUNPROFITABLE\x1B[0m"
                };
                println!(
                    "  {} pairs: {:.2} | value: ${:.2} | cost: ${:.2} | profit: ${:.2} ({})",
                    status,
                    merge.mergeable_pairs,
                    merge.merge_value,
                    merge.total_cost,
                    merge.potential_profit,
                    format!("{:.1}%", merge.profit_percentage())
                );
            }
            println!();
        }

        drop(tracker);

        println!("════════════════════════════════════════════════════════════════");
        println!("  Watching for CONFIRMED fills from WebSocket...");
        println!("  (MATCHED/MINED/RETRYING fills are logged but not applied)");
        println!("════════════════════════════════════════════════════════════════");

        sleep(Duration::from_millis(500)).await;
    }

    println!();
    println!("Shutting down...");

    Ok(())
}
