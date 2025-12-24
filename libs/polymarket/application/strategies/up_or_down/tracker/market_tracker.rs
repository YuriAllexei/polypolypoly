//! Market tracker for the Up or Down strategy.
//!
//! Handles WebSocket connection, orderbook monitoring, and the main tracking loop.

use crate::application::strategies::up_or_down::services::{
    get_market_oracle_age, get_price_to_beat, log_market_ended,
};
use crate::application::strategies::up_or_down::tracker::{
    check_all_orderbooks, check_risk, guardian_check, place_order, upgrade_order_on_tick_change,
};
use crate::application::strategies::up_or_down::types::{
    MarketTrackerContext, OrderInfo, TrackerState, TrackingLoopExit, MAX_RECONNECT_ATTEMPTS,
    STALENESS_THRESHOLD_SECS,
};
use crate::domain::DbMarket;
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::config::UpOrDownConfig;
use crate::infrastructure::client::user::{SharedOrderState, SharedPositionTracker};
use crate::infrastructure::{
    build_ws_client, decimal_places, handle_client_event, BalanceManager, MarketTrackerConfig,
    RiskManagerHandle, SharedOraclePrices, SharedOrderbooks, SharedPrecisions, TickSizeChangeEvent,
};
use chrono::Utc;
use crossbeam_channel::{unbounded, Receiver};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::time::sleep;
use tracing::{error, info, warn};

// =============================================================================
// WebSocket Client Type
// =============================================================================

/// Type alias for the WebSocket client used in tracking
type WsClient = crate::infrastructure::WebSocketClient<
    crate::infrastructure::SniperRouter,
    crate::infrastructure::SniperMessage,
>;

// =============================================================================
// Connection Result
// =============================================================================

/// Result of creating a WebSocket connection
struct ConnectionResult {
    client: WsClient,
    orderbooks: SharedOrderbooks,
    precisions: SharedPrecisions,
    tick_size_rx: Receiver<TickSizeChangeEvent>,
    first_snapshot_received: Arc<AtomicBool>,
}

// =============================================================================
// Main Entry Point
// =============================================================================

/// Run the WebSocket market tracker for a single market.
///
/// Connects to Polymarket WebSocket, subscribes to orderbook updates,
/// and monitors for trading signals until shutdown or market ends.
pub async fn run_market_tracker(
    market: DbMarket,
    shutdown_flag: Arc<AtomicBool>,
    config: UpOrDownConfig,
    trading: Arc<TradingClient>,
    oracle_prices: Option<SharedOraclePrices>,
    balance_manager: Arc<RwLock<BalanceManager>>,
    _position_tracker: Option<SharedPositionTracker>,
    order_state: Option<SharedOrderState>,
    risk_manager: Option<RiskManagerHandle>,
) -> anyhow::Result<()> {
    // Initialize context and state
    let outcomes = market.parse_outcomes()?;
    let mut ctx = MarketTrackerContext::new(&market, &config, outcomes.clone())?;
    let mut state = TrackerState::new();

    // Build WebSocket configuration
    let ws_config = MarketTrackerConfig::new(
        ctx.market_id.clone(),
        ctx.market_question.clone(),
        market.slug.clone(),
        ctx.token_ids.clone(),
        outcomes,
        &market.end_date,
    )?;

    // Fetch the price to beat for this market
    fetch_and_set_price_to_beat(&mut ctx, &market).await;

    // Log startup info
    log_tracker_startup(&ctx, &ws_config);

    // Track reconnection attempts
    let mut reconnect_attempts: u32 = 0;

    // Outer reconnection loop - handles WebSocket reconnection on staleness
    'reconnect: loop {
        // Check shutdown before attempting connection
        if !shutdown_flag.load(Ordering::Acquire) {
            info!(
                "[WS {}] Shutdown signal received before connect",
                ctx.market_id
            );
            break 'reconnect;
        }

        // Check if market has ended before attempting connection
        if Utc::now() > ctx.market_end_time {
            info!(
                "[WS {}] Market already ended, not connecting",
                ctx.market_id
            );
            break 'reconnect;
        }

        // Handle reconnection delay
        if reconnect_attempts > 0 {
            info!(
                "[WS {}] Reconnection attempt {} of {}",
                ctx.market_id, reconnect_attempts, MAX_RECONNECT_ATTEMPTS
            );
            sleep(StdDuration::from_secs(2)).await;
        }

        // Create WebSocket connection
        let conn_result = match create_ws_connection(&ws_config, &ctx.market_id).await {
            Ok(result) => result,
            Err(e) => {
                error!("[WS {}] Failed to connect: {}", ctx.market_id, e);
                reconnect_attempts += 1;
                continue 'reconnect;
            }
        };

        // Wait for first snapshot
        if !wait_for_snapshot(&conn_result.first_snapshot_received, &shutdown_flag, &ctx.market_id).await {
            let _ = conn_result.client.shutdown().await;
            if !shutdown_flag.load(Ordering::Acquire) {
                break 'reconnect; // Shutdown requested
            }
            reconnect_attempts += 1;
            continue 'reconnect;
        }

        // Validate all expected tokens have orderbooks
        if !validate_orderbooks(&conn_result.orderbooks, &ctx) {
            let _ = conn_result.client.shutdown().await;
            reconnect_attempts += 1;
            continue 'reconnect;
        }

        // Run the main tracking loop
        let (exit_reason, connection_start) = run_tracking_loop(
            &conn_result,
            &mut state,
            &ctx,
            &shutdown_flag,
            &oracle_prices,
            &trading,
            &balance_manager,
            &order_state,
            &risk_manager,
        )
        .await;

        // Close current WebSocket connection
        info!(
            "[WS {}] Closing connection (reason: {:?})",
            ctx.market_id,
            exit_reason.as_str()
        );
        if let Err(e) = conn_result.client.shutdown().await {
            warn!("[WS {}] Error during shutdown: {}", ctx.market_id, e);
        }

        // Handle reconnection or exit
        if !handle_reconnection(
            &exit_reason,
            &mut reconnect_attempts,
            &mut state,
            &ctx.market_id,
            connection_start,
        ) {
            break 'reconnect;
        }
    }

    // Log final state
    if !state.order_placed.is_empty() {
        info!(
            "[WS {}] Tracker stopping with {} orders still open (leaving them for potential fills)",
            ctx.market_id,
            state.order_placed.len()
        );
    }

    info!("[WS {}] Tracker stopped", ctx.market_id);
    Ok(())
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Fetch the price to beat and set it in the context
async fn fetch_and_set_price_to_beat(ctx: &mut MarketTrackerContext, market: &DbMarket) {
    let price_to_beat = match get_price_to_beat(ctx.timeframe, ctx.crypto_asset, market).await {
        Ok(price) => {
            info!("[WS {}] Price to beat: ${:.2}", ctx.market_id, price);
            Some(price)
        }
        Err(e) => {
            warn!(
                "[WS {}] Failed to fetch price to beat: {}",
                ctx.market_id, e
            );
            None
        }
    };
    ctx.set_price_to_beat(price_to_beat);
}

/// Log tracker startup information
fn log_tracker_startup(ctx: &MarketTrackerContext, ws_config: &MarketTrackerConfig) {
    info!("[WS {}] Connecting to orderbook stream...", ctx.market_id);
    info!("[WS {}] Market: {}", ctx.market_id, ctx.market_question);
    info!("[WS {}] Oracle: {}", ctx.market_id, ctx.oracle_source);
    info!("[WS {}] Asset: {}", ctx.market_id, ctx.crypto_asset);
    info!("[WS {}] Timeframe: {}", ctx.market_id, ctx.timeframe);
    info!(
        "[WS {}] Resolution time: {}",
        ctx.market_id, ws_config.resolution_time
    );
}

/// Create a WebSocket connection and return the client with shared state
async fn create_ws_connection(
    ws_config: &MarketTrackerConfig,
    market_id: &str,
) -> anyhow::Result<ConnectionResult> {
    let orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
    let precisions: SharedPrecisions = Arc::new(RwLock::new(HashMap::new()));
    let first_snapshot_received = Arc::new(AtomicBool::new(false));

    // Create channel for tick_size_change events
    let (tick_size_tx, tick_size_rx) = unbounded::<TickSizeChangeEvent>();

    let client = build_ws_client(
        ws_config,
        Arc::clone(&orderbooks),
        Arc::clone(&precisions),
        Some(tick_size_tx),
        Arc::clone(&first_snapshot_received),
    )
    .await?;

    info!("[WS {}] Connected and subscribed", market_id);

    Ok(ConnectionResult {
        client,
        orderbooks,
        precisions,
        tick_size_rx,
        first_snapshot_received,
    })
}

/// Wait for the first orderbook snapshot to be received.
/// Returns true if snapshot received, false if timeout or shutdown.
async fn wait_for_snapshot(
    first_snapshot_received: &Arc<AtomicBool>,
    shutdown_flag: &Arc<AtomicBool>,
    market_id: &str,
) -> bool {
    let start = Instant::now();

    loop {
        if first_snapshot_received.load(Ordering::Acquire) {
            return true;
        }
        if start.elapsed() > StdDuration::from_secs(10) {
            error!("[WS {}] Timeout waiting for first snapshot", market_id);
            return false;
        }
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[WS {}] Shutdown during snapshot wait", market_id);
            return false;
        }
        sleep(StdDuration::from_millis(10)).await;
    }
}

/// Validate that all expected tokens have orderbooks.
/// Returns true if all tokens have orderbooks, false otherwise.
fn validate_orderbooks(orderbooks: &SharedOrderbooks, ctx: &MarketTrackerContext) -> bool {
    let obs = orderbooks.read();
    let missing_count = ctx
        .token_ids
        .iter()
        .filter(|t| !obs.contains_key(*t))
        .count();
    if missing_count > 0 {
        error!(
            "[WS {}] Missing orderbooks for {} tokens",
            ctx.market_id, missing_count
        );
        return false;
    }
    true
}

/// Run the main tracking loop.
/// Returns the exit reason and when the connection started.
async fn run_tracking_loop(
    conn: &ConnectionResult,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
    shutdown_flag: &Arc<AtomicBool>,
    oracle_prices: &Option<SharedOraclePrices>,
    trading: &Arc<TradingClient>,
    balance_manager: &Arc<RwLock<BalanceManager>>,
    order_state: &Option<SharedOrderState>,
    risk_manager: &Option<RiskManagerHandle>,
) -> (TrackingLoopExit, Instant) {
    let connection_start = Instant::now();
    let mut seen_updates_since_connect = false;
    let mut last_oracle_warning: Option<Instant> = None;

    let exit_reason = loop {
        // Check shutdown flag (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[WS {}] Shutdown signal received", ctx.market_id);
            break TrackingLoopExit::Shutdown;
        }

        // Check if we're too late - market ended but no orders placed
        if Utc::now() > ctx.market_end_time && state.order_placed.is_empty() {
            info!(
                "[WS {}] Too late - market ended ({}) with no orders placed",
                ctx.market_id,
                ctx.market_end_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
            break TrackingLoopExit::TooLate;
        }

        // Check if market resolved: time passed AND we have high-confidence order ($0.999+)
        if Utc::now() > ctx.market_end_time && state.has_high_confidence_order() {
            info!(
                "[WS {}] Market resolved: time passed ({}) with $0.999+ order placed",
                ctx.market_id,
                ctx.market_end_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
            break TrackingLoopExit::MarketEnded;
        }

        // Handle WebSocket events
        if let Some(event) = conn.client.try_recv_event() {
            if !handle_client_event(event, &ctx.market_id) {
                break TrackingLoopExit::WebSocketDisconnected;
            }
        }

        // Handle tick_size_change events
        while let Ok(event) = conn.tick_size_rx.try_recv() {
            let new_precision = decimal_places(&event.new_tick_size);
            info!(
                "[WS {}] Tick size changed for {}: {} -> {} (precision: {})",
                ctx.market_id,
                ctx.get_outcome_name(&event.asset_id),
                event.old_tick_size,
                event.new_tick_size,
                new_precision
            );

            // Check if we have an order for this token that could be upgraded
            if let Some(current_order) = state.order_placed.get(&event.asset_id).cloned() {
                // Only proceed if upgrade is actually needed (new precision is higher)
                if new_precision > current_order.precision {
                    // Skip order state check for recently-placed orders (WebSocket has slight delay)
                    // This prevents removing orders that were just placed but not yet indexed
                    if !current_order.is_recently_placed(2) {
                        let order_exists = order_state
                            .as_ref()
                            .map(|s| s.read().get_order(&current_order.order_id).is_some())
                            .unwrap_or(true); // Default to true if no order_state (conservative)

                        if !order_exists {
                            info!(
                                "[WS {}] Order {} not found in order state for {}, removing from local state",
                                ctx.market_id,
                                current_order.order_id,
                                ctx.get_outcome_name(&event.asset_id)
                            );
                            state.order_placed.remove(&event.asset_id);
                            continue;
                        }
                    }

                    // Check if trading is halted
                    if balance_manager.read().is_halted() {
                        info!(
                            "[WS {}] Order upgrade blocked - trading halted",
                            ctx.market_id
                        );
                        continue;
                    }

                    if let Some(new_order_info) = upgrade_order_on_tick_change(
                        trading,
                        &event.asset_id,
                        &current_order,
                        new_precision,
                        ctx,
                        balance_manager,
                    )
                    .await
                    {
                        state.order_placed.insert(event.asset_id.clone(), new_order_info);
                    } else {
                        // Upgrade failed - remove from tracking and reset timer state
                        // so a fresh order can be attempted on next no-asks detection
                        state.order_placed.remove(&event.asset_id);
                        state.threshold_triggered.remove(&event.asset_id);
                        state.no_asks_timers.remove(&event.asset_id);
                        warn!(
                            "[WS {}] Order upgrade failed for {}, reset state for fresh order attempt",
                            ctx.market_id,
                            ctx.get_outcome_name(&event.asset_id)
                        );
                    }
                }
            }
        }

        // Check for stale orderbooks
        let (is_stale, market_has_activity) =
            check_orderbook_staleness(&conn.orderbooks, ctx, connection_start, seen_updates_since_connect);

        if market_has_activity {
            seen_updates_since_connect = true;
        }

        if is_stale {
            break TrackingLoopExit::StaleOrderbook;
        }

        // Check oracle health (logs warnings for stale data)
        // Note: This just logs warnings - pre_order_risk_check blocks orders at 10s
        check_oracle_health(oracle_prices, ctx, &mut last_oracle_warning);

        // Check orderbooks and get tokens needing orders
        let (tokens_to_order, all_empty) =
            check_all_orderbooks(&conn.orderbooks, state, ctx).await;

        // Exit if market has ended (all orderbooks empty)
        if all_empty {
            log_market_ended(ctx);
            break TrackingLoopExit::AllOrderbooksEmpty;
        }

        // Process tokens that exceeded threshold
        process_order_candidates(
            tokens_to_order,
            &conn.orderbooks,
            &conn.precisions,
            state,
            ctx,
            oracle_prices,
            trading,
            balance_manager,
            order_state.as_ref(),
            risk_manager,
        )
        .await;

        // Monitor for risk on placed orders
        check_risk(&conn.orderbooks, state, ctx, oracle_prices, trading).await;

        // Guardian safety net - runs ALWAYS, never bypassed
        guardian_check(state, ctx, oracle_prices, trading).await;

        // Brief sleep before next iteration
        sleep(StdDuration::from_millis(10)).await;
    };

    (exit_reason, connection_start)
}

/// Oracle staleness warning threshold (seconds)
const ORACLE_STALENESS_WARNING_SECS: u64 = 15;

/// Oracle staleness critical threshold (seconds)
const ORACLE_STALENESS_CRITICAL_SECS: u64 = 30;

/// Check this market's specific oracle health and log warnings.
///
/// Returns true if oracle is healthy enough for trading, false if critically stale.
fn check_oracle_health(
    oracle_prices: &Option<SharedOraclePrices>,
    ctx: &MarketTrackerContext,
    last_oracle_warning: &mut Option<Instant>,
) -> bool {
    let Some(age) = get_market_oracle_age(oracle_prices, ctx.oracle_source) else {
        // Unknown oracle source - skip health check
        return true;
    };

    let age_secs = age.as_secs();

    // Rate-limit warnings to once every 5 seconds
    let should_log = match last_oracle_warning {
        Some(last) => last.elapsed() > StdDuration::from_secs(5),
        None => true,
    };

    if age_secs < ORACLE_STALENESS_WARNING_SECS {
        return true; // Healthy
    } else if age_secs < ORACLE_STALENESS_CRITICAL_SECS {
        if should_log {
            warn!(
                "[WS {}] {} oracle STALE: {:.1}s since last update (warning threshold: {}s)",
                ctx.market_id, ctx.oracle_source, age.as_secs_f64(), ORACLE_STALENESS_WARNING_SECS
            );
            *last_oracle_warning = Some(Instant::now());
        }
        return true; // Warning but allow trading (pre_order_risk_check will block)
    } else {
        if should_log {
            error!(
                "[WS {}] {} oracle CRITICAL: {:.1}s since last update - new orders blocked",
                ctx.market_id, ctx.oracle_source, age.as_secs_f64()
            );
            *last_oracle_warning = Some(Instant::now());
        }
        return false; // Critical - block trading
    }
}

/// Check if any orderbooks are stale (haven't received updates recently).
/// Returns (is_stale, has_activity).
fn check_orderbook_staleness(
    orderbooks: &SharedOrderbooks,
    ctx: &MarketTrackerContext,
    connection_start: Instant,
    seen_updates: bool,
) -> (bool, bool) {
    let obs = orderbooks.read();
    let connection_age = connection_start.elapsed().as_secs_f64();
    let mut stale = false;
    let mut has_activity = seen_updates;

    for (token_id, orderbook) in obs.iter() {
        let staleness = orderbook.seconds_since_update();

        // Check if we've received updates beyond the initial snapshot
        if !has_activity && staleness < (connection_age - 5.0) {
            has_activity = true;
        }

        // Only consider staleness if we've seen activity
        if has_activity && staleness > STALENESS_THRESHOLD_SECS {
            warn!(
                "[WS {}] Orderbook for {} is stale ({:.1}s since last update) - triggering reconnection",
                ctx.market_id,
                ctx.get_outcome_name(token_id),
                staleness
            );
            stale = true;
            break;
        }
    }
    (stale, has_activity)
}

/// Process tokens that are candidates for order placement.
async fn process_order_candidates(
    tokens_to_order: Vec<(String, String, f64)>,
    orderbooks: &SharedOrderbooks,
    precisions: &SharedPrecisions,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
    _oracle_prices: &Option<SharedOraclePrices>,
    trading: &Arc<TradingClient>,
    balance_manager: &Arc<RwLock<BalanceManager>>,
    order_state: Option<&SharedOrderState>,
    risk_manager: &Option<RiskManagerHandle>,
) {
    for (token_id, outcome_name, elapsed) in tokens_to_order {
        // Re-check orderbook and capture liquidity before placing order
        let (still_no_asks, best_bid, liq_at_99) = {
            let obs = orderbooks.read();
            match obs.get(&token_id) {
                Some(ob) => (
                    ob.asks.is_empty(),
                    ob.best_bid(),
                    ob.bid_liquidity_at_price(0.99),
                ),
                None => (false, None, 0.0),
            }
        };

        if !still_no_asks {
            info!(
                "[WS {}] Skipping order for {} - asks appeared during processing",
                ctx.market_id, outcome_name
            );
            state.threshold_triggered.remove(&token_id);
            state.no_asks_timers.remove(&token_id);
            continue;
        }

        // Log liquidity at entry
        let top_bid_str = best_bid
            .map(|(p, s)| format!("{:.2} @ ${:.2}", s, p))
            .unwrap_or_else(|| "none".to_string());
        info!(
            "[WS {}] Bid Liquidity: Top: {} | At $0.99: {:.2}",
            ctx.market_id, top_bid_str, liq_at_99
        );

        // Pre-order risk check using RiskManager - ALWAYS check, no bypass
        let risk_check_passed = match (risk_manager, ctx.price_to_beat) {
            (Some(rm), Some(ptb)) => rm.pre_placement_check(ptb, ctx.oracle_source, ctx.crypto_asset),
            _ => true, // No risk manager or no price_to_beat - allow order
        };

        if !risk_check_passed {
            info!(
                "[WS {}] Skipping order for {} - pre-placement risk check failed",
                ctx.market_id, outcome_name
            );
            state.threshold_triggered.remove(&token_id);
            state.no_asks_timers.remove(&token_id);
            continue;
        }

        // Check if trading is halted due to balance drop
        if balance_manager.read().is_halted() {
            info!(
                "[WS {}] Order blocked - trading halted due to balance drop",
                ctx.market_id
            );
            state.threshold_triggered.remove(&token_id);
            state.no_asks_timers.remove(&token_id);
            continue;
        }

        // Place the order
        if let Some((order_id, precision)) =
            place_order(trading, &token_id, &outcome_name, elapsed, ctx, precisions, balance_manager, order_state).await
        {
            state.order_placed.insert(token_id.clone(), OrderInfo::new(order_id, precision));

            // Register market with risk manager for continuous monitoring now that we have an order
            if let (Some(rm), Some(price_to_beat)) = (risk_manager, ctx.price_to_beat) {
                if ctx.token_ids.len() >= 2 {
                    if let Err(e) = rm.register_market(
                        ctx.market_id.clone(),
                        price_to_beat,
                        ctx.oracle_source,
                        ctx.crypto_asset,
                        ctx.market_end_time,
                        [ctx.token_ids[0].clone(), ctx.token_ids[1].clone()],
                    ) {
                        warn!("[WS {}] Failed to register with risk manager: {}", ctx.market_id, e);
                    } else {
                        info!("[WS {}] Registered with risk manager after order placement", ctx.market_id);
                    }
                }
            }
        }
    }
}

/// Handle reconnection logic.
/// Returns true if should reconnect, false if should exit.
fn handle_reconnection(
    exit_reason: &TrackingLoopExit,
    reconnect_attempts: &mut u32,
    state: &mut TrackerState,
    market_id: &str,
    connection_start: Instant,
) -> bool {
    if !exit_reason.should_reconnect() {
        return false;
    }

    // Check if connection was stable (ran longer than staleness threshold)
    let connection_duration = connection_start.elapsed().as_secs_f64();
    if connection_duration > STALENESS_THRESHOLD_SECS * 2.0 {
        *reconnect_attempts = 0;
        info!(
            "[WS {}] Connection was stable for {:.1}s, resetting reconnect counter",
            market_id, connection_duration
        );
    }

    *reconnect_attempts += 1;

    // Check if we've exceeded max attempts
    if *reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
        error!(
            "[WS {}] Exceeded max reconnection attempts ({}) due to repeated staleness/disconnects, giving up",
            market_id, MAX_RECONNECT_ATTEMPTS
        );
        return false;
    }

    info!(
        "[WS {}] Will attempt reconnection (attempt {} of {})",
        market_id, reconnect_attempts, MAX_RECONNECT_ATTEMPTS
    );

    // Clear timer state on reconnect
    state.clear_timers();

    true
}
