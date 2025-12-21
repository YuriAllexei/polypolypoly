use super::super::services::log_winning_token;
use super::winner_analyzer::analyze_orderbooks_for_winner;
use crate::domain::DbMarket;
use crate::infrastructure::client::TradingClient;
use crate::infrastructure::{
    build_ws_client, BalanceManager, FullTimeEvent, MarketTrackerConfig, SharedOrderbooks,
    SharedPrecisions,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

// =============================================================================
// Helper Functions
// =============================================================================

/// Wait for the first orderbook snapshot to be received.
/// Returns true if snapshot received, false if timeout or shutdown.
async fn wait_for_snapshot(
    first_snapshot_received: &Arc<AtomicBool>,
    shutdown_flag: &Arc<AtomicBool>,
    market_id: &str,
) -> bool {
    let start = std::time::Instant::now();

    loop {
        if first_snapshot_received.load(Ordering::Acquire) {
            return true;
        }
        if start.elapsed() > StdDuration::from_secs(10) {
            error!(
                "[Sports Tracker] Timeout waiting for snapshot on market {}",
                market_id
            );
            return false;
        }
        if !shutdown_flag.load(Ordering::Acquire) {
            info!(
                "[Sports Tracker] Shutdown during snapshot wait for market {}",
                market_id
            );
            return false;
        }
        sleep(StdDuration::from_millis(10)).await;
    }
}

// =============================================================================
// Main Entry Point
// =============================================================================

/// Run a market tracker for a single sports market
///
/// Connects to the orderbook WebSocket, waits for snapshot,
/// analyzes orderbooks to find the winning token, and places an order if threshold met.
pub async fn run_sports_market_tracker(
    market: DbMarket,
    event: FullTimeEvent,
    shutdown_flag: Arc<AtomicBool>,
    trading: Arc<TradingClient>,
    balance_manager: Arc<RwLock<BalanceManager>>,
    order_pct: f64,
    bid_threshold: f64,
) -> anyhow::Result<()> {
    // Parse market data
    let token_ids = market.parse_token_ids()?;
    let outcomes = market.parse_outcomes()?;

    if token_ids.len() < 2 {
        warn!(
            "[Sports Tracker] Market {} has fewer than 2 tokens, skipping",
            market.id
        );
        return Ok(());
    }

    info!(
        "[Sports Tracker] Starting tracker for market: {} ({})",
        market.question, market.id
    );

    // Build WebSocket configuration
    let ws_config = MarketTrackerConfig::new(
        market.id.clone(),
        market.question.clone(),
        market.slug.clone(),
        token_ids.clone(),
        outcomes.clone(),
        &market.end_date,
    )?;

    // Create shared orderbooks and precisions
    let orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
    let precisions: SharedPrecisions = Arc::new(RwLock::new(HashMap::new()));
    let first_snapshot_received = Arc::new(AtomicBool::new(false));

    // Connect to WebSocket
    let client = match build_ws_client(
        &ws_config,
        Arc::clone(&orderbooks),
        Arc::clone(&precisions),
        None, // No tick_size_tx needed for now
        Arc::clone(&first_snapshot_received),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(
                "[Sports Tracker] Failed to connect to WS for market {}: {}",
                market.id, e
            );
            return Err(e);
        }
    };

    info!(
        "[Sports Tracker] Connected to orderbook WS for market {}",
        market.id
    );

    // Wait for first snapshot
    if !wait_for_snapshot(&first_snapshot_received, &shutdown_flag, &market.id).await {
        let _ = client.shutdown().await;
        return Ok(());
    }

    // Analyze orderbooks to find winner
    let winner = analyze_orderbooks_for_winner(&orderbooks, &token_ids, &outcomes);

    // Check if best bid meets threshold
    let should_execute = match &winner {
        Some(w) => match w.best_bid {
            Some((price, _)) => price > bid_threshold,
            None => false,
        },
        None => false,
    };

    if !should_execute {
        let bid_info = winner
            .as_ref()
            .and_then(|w| w.best_bid)
            .map(|(p, _)| format!("{:.4}", p))
            .unwrap_or_else(|| "None".to_string());

        debug!(
            "[Sports Tracker] Not executing for market {} - best bid {} is not above threshold {:.2}",
            market.id, bid_info, bid_threshold
        );

        // Cleanup and return
        let _ = client.shutdown().await;
        return Ok(());
    }

    // Log the winner analysis
    log_winning_token(&market, &event, &winner);

    // Place order for the winning token
    if let Some(ref w) = winner {
        // Get precision from SharedPrecisions (default to 2 if not found)
        let precision = precisions.read().get(&w.token_id).copied().unwrap_or(2);

        // Calculate price: 1.0 - 10^(-precision)
        // precision=2 -> 0.99, precision=3 -> 0.999, etc.
        let price = 1.0 - 10_f64.powi(-(precision as i32));

        // Calculate order size from balance percentage
        let current_balance = balance_manager.read().current_balance();
        let size = (current_balance * order_pct).round().max(1.0);

        info!(
            "[Sports Tracker] Placing order for {} @ ${:.4} x ${:.2} (precision: {})",
            w.outcome_name, price, size, precision
        );

        match trading.buy(&w.token_id, price, size).await {
            Ok(response) => {
                info!(
                    "[Sports Tracker] ✅ Order placed successfully for market {}: {:?}",
                    market.id, response
                );
            }
            Err(e) => {
                error!(
                    "[Sports Tracker] ❌ Order failed for market {}: {}",
                    market.id, e
                );
            }
        }
    }

    // Cleanup
    if let Err(e) = client.shutdown().await {
        warn!(
            "[Sports Tracker] Error shutting down WS for market {}: {}",
            market.id, e
        );
    }

    info!(
        "[Sports Tracker] Tracker completed for market {}",
        market.id
    );
    Ok(())
}
