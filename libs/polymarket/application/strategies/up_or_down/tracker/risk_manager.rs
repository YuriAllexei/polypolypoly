//! Risk management and order execution for the Up or Down strategy.
//!
//! Handles pre-order risk checks, post-order risk monitoring, order placement,
//! and order cancellation.

use crate::application::strategies::up_or_down::services::{
    get_oracle_price, log_order_failed, log_order_success, log_placing_order, log_risk_detected,
};
use crate::application::strategies::up_or_down::tracker::calculate_dynamic_threshold;
use crate::application::strategies::up_or_down::types::{
    MarketTrackerContext, OrderInfo, TrackerState, FINAL_SECONDS_BYPASS,
};
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::{BalanceManager, SharedOraclePrices, SharedOrderbooks, SharedPrecisions};
use chrono::Utc;
use std::sync::{Arc, RwLock};
use tracing::{debug, error, info, warn};

// =============================================================================
// Pre-Order Risk Check
// =============================================================================

/// Pre-order risk check based on oracle price proximity.
///
/// Checks if |price_to_beat - oracle_price| in bps < oracle_bps_price_threshold.
/// If the oracle price is too close to price_to_beat, the outcome is uncertain
/// and we should NOT place the order.
///
/// Returns:
/// - `true` if safe to place order (oracle price far enough from price_to_beat)
/// - `false` if risky to place order (oracle price too close)
pub fn pre_order_risk_check(
    ctx: &MarketTrackerContext,
    oracle_prices: &Option<SharedOraclePrices>,
) -> bool {
    // If we don't have price_to_beat or oracle prices, allow the order (no data to check against)
    let (price_to_beat, oracle_prices) = match (ctx.price_to_beat, oracle_prices) {
        (Some(ptb), Some(op)) => (ptb, op),
        _ => {
            debug!(
                "[WS {}] Pre-check PASS: No oracle data available",
                ctx.market_id
            );
            return true;
        }
    };

    // Get current oracle price
    let current_price = match get_oracle_price(ctx.oracle_source, ctx.crypto_asset, oracle_prices) {
        Some(price) => price,
        None => {
            debug!(
                "[WS {}] Pre-check PASS: Could not get oracle price",
                ctx.market_id
            );
            return true;
        }
    };

    // Calculate BPS difference
    let bps_diff = ((price_to_beat - current_price).abs() / price_to_beat) * 10000.0;

    // If oracle price is too close to price_to_beat, it's risky
    if bps_diff < ctx.oracle_bps_price_threshold {
        warn!(
            "[WS {}] Pre-check FAIL: Oracle ${:.2} too close to target ${:.2} ({:.1} bps < {:.1} threshold)",
            ctx.market_id,
            current_price,
            price_to_beat,
            bps_diff,
            ctx.oracle_bps_price_threshold
        );
        return false;
    }

    info!(
        "[WS {}] Pre-check PASS: Oracle ${:.2} vs target ${:.2} ({:.1} bps >= {:.1} threshold)",
        ctx.market_id,
        current_price,
        price_to_beat,
        bps_diff,
        ctx.oracle_bps_price_threshold
    );
    true
}

// =============================================================================
// Post-Order Risk Check
// =============================================================================

/// Check for risk on tokens with placed orders and cancel if risk detected.
///
/// Two signals must both be active to indicate risk:
/// 1. Average of other bids (excluding top) < 0.85
/// 2. |price_to_beat - oracle_price| in bps < oracle_bps_price_threshold
///
/// Returns false early if no orders are placed or if the market has ended.
/// Only cancels the specific token(s) where risk is detected, not all orders.
pub async fn check_risk(
    orderbooks: &SharedOrderbooks,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
    oracle_prices: &Option<SharedOraclePrices>,
    trading: &TradingClient,
) -> bool {
    if state.order_placed.is_empty() {
        return false;
    }

    // Skip risk check if market ended OR in final seconds before end
    let now = Utc::now();
    let time_remaining = ctx
        .market_end_time
        .signed_duration_since(now)
        .num_milliseconds() as f64
        / 1000.0;
    let market_ended = time_remaining <= 0.0;
    let in_final_seconds = time_remaining > 0.0 && time_remaining <= FINAL_SECONDS_BYPASS;

    if market_ended || in_final_seconds {
        return false;
    }

    // Signal 2: Check oracle price difference (applies to whole market)
    let mut signal_2_active = false;
    let mut bps_diff = 0.0;
    let mut oracle_price = 0.0;

    if let (Some(price_to_beat), Some(oracle_prices)) = (ctx.price_to_beat, oracle_prices) {
        if let Some(current_price) =
            get_oracle_price(ctx.oracle_source, ctx.crypto_asset, oracle_prices)
        {
            oracle_price = current_price;
            bps_diff = ((price_to_beat - current_price).abs() / price_to_beat) * 10000.0;
            if bps_diff < ctx.oracle_bps_price_threshold {
                signal_2_active = true;
            }
        }
    }

    // If oracle signal not active, no risk
    if !signal_2_active {
        return false;
    }

    // Signal 1: Check bid levels per token
    let bid_data: Vec<(String, Vec<f64>)> = {
        let obs = orderbooks.read().unwrap();
        state
            .order_placed
            .keys()
            .filter_map(|token_id| {
                obs.get(token_id).and_then(|orderbook| {
                    let bid_levels = orderbook.bids.levels();
                    if bid_levels.len() > 1 {
                        let other_bids: Vec<f64> = bid_levels
                            .iter()
                            .skip(1)
                            .take(4)
                            .map(|(price, _)| *price)
                            .collect();
                        if !other_bids.is_empty() {
                            return Some((token_id.clone(), other_bids));
                        }
                    }
                    None
                })
            })
            .collect()
    };

    let mut tokens_at_risk: Vec<(String, f64, Vec<f64>)> = Vec::new();
    for (token_id, other_bids) in bid_data {
        let avg_bid_price = other_bids.iter().sum::<f64>() / other_bids.len() as f64;
        if avg_bid_price < 0.85 {
            tokens_at_risk.push((token_id, avg_bid_price, other_bids));
        }
    }

    if tokens_at_risk.is_empty() {
        return false;
    }

    // Cancel only the specific tokens at risk
    for (token_id, avg_bid_price, other_bids) in tokens_at_risk {
        let outcome_name = ctx.get_outcome_name(&token_id);
        log_risk_detected(
            ctx,
            &token_id,
            &outcome_name,
            avg_bid_price,
            &other_bids,
            bps_diff,
            oracle_price,
        );

        // Only remove from state if cancellation succeeds
        if let Some(order_info) = state.order_placed.get(&token_id) {
            let cancelled = cancel_order(trading, &order_info.order_id, &token_id, ctx).await;
            if cancelled {
                state.order_placed.remove(&token_id);
            } else {
                warn!(
                    "[WS {}] Failed to cancel order for {} - keeping in state for retry",
                    ctx.market_id, outcome_name
                );
            }
        }
    }

    true
}

// =============================================================================
// Order Placement
// =============================================================================

/// Place a buy order for a token.
///
/// Returns (order_id, precision) if successful, None if failed.
pub async fn place_order(
    trading: &TradingClient,
    token_id: &str,
    outcome_name: &str,
    elapsed: f64,
    ctx: &MarketTrackerContext,
    precisions: &SharedPrecisions,
    balance_manager: &Arc<RwLock<BalanceManager>>,
) -> Option<(String, u8)> {
    let dynamic_threshold = calculate_dynamic_threshold(ctx);
    log_placing_order(ctx, token_id, outcome_name, elapsed, dynamic_threshold);

    // Get precision for this token (default to 2)
    let precision = {
        let precs = precisions.read().unwrap();
        *precs.get(token_id).unwrap_or(&2)
    };

    // Calculate price: 0.99 for precision 2, 0.999 for precision 3, etc.
    let price = 1.0 - 10_f64.powi(-(precision as i32));

    // Calculate order size from current balance
    let current_balance = balance_manager.read().unwrap().current_balance();
    let order_size = (current_balance * ctx.order_pct_of_collateral).round();
    // Ensure minimum order size of 1
    let order_size = order_size.max(1.0);

    info!(
        "[WS {}] Order size: ${:.0} ({:.0}% of ${:.2} balance)",
        ctx.market_id, order_size, ctx.order_pct_of_collateral * 100.0, current_balance
    );

    match trading.buy(token_id, price, order_size).await {
        Ok(response) => {
            log_order_success(ctx, token_id, outcome_name, &response);
            response.order_id.map(|id| (id, precision))
        }
        Err(e) => {
            log_order_failed(ctx, token_id, outcome_name, &e);
            None
        }
    }
}

// =============================================================================
// Order Cancellation
// =============================================================================

/// Cancel a single order and log the result.
/// Returns true if cancellation succeeded, false otherwise.
pub async fn cancel_order(
    trading: &TradingClient,
    order_id: &str,
    token_id: &str,
    ctx: &MarketTrackerContext,
) -> bool {
    let outcome_name = ctx.get_outcome_name(token_id);
    info!(
        "[WS {}] 🚨 Cancelling order {} for {}",
        ctx.market_id, order_id, outcome_name
    );

    match trading.cancel_order(order_id).await {
        Ok(response) => {
            if !response.canceled.is_empty() {
                info!(
                    "[WS {}] ✅ Cancelled order for {}",
                    ctx.market_id, outcome_name
                );
                return true;
            }
            if !response.not_canceled.is_empty() {
                warn!(
                    "[WS {}] ⚠️ Failed to cancel order {}: {:?}",
                    ctx.market_id, order_id, response.not_canceled
                );
            }
            false
        }
        Err(e) => {
            error!(
                "[WS {}] ❌ Failed to cancel order {}: {}",
                ctx.market_id, order_id, e
            );
            false
        }
    }
}

/// Cancel multiple orders and log the result
pub async fn cancel_orders(trading: &TradingClient, order_ids: &[String], ctx: &MarketTrackerContext) {
    info!(
        "[WS {}] 🚨 CANCELLING {} orders due to risk detection",
        ctx.market_id,
        order_ids.len()
    );

    match trading.cancel_orders(order_ids).await {
        Ok(response) => {
            if !response.canceled.is_empty() {
                info!(
                    "[WS {}] ✅ Successfully cancelled {} orders: {:?}",
                    ctx.market_id,
                    response.canceled.len(),
                    response.canceled
                );
            }
            if !response.not_canceled.is_empty() {
                warn!(
                    "[WS {}] ⚠️ Failed to cancel {} orders: {:?}",
                    ctx.market_id,
                    response.not_canceled.len(),
                    response.not_canceled
                );
            }
        }
        Err(e) => {
            error!("[WS {}] ❌ Failed to cancel orders: {}", ctx.market_id, e);
        }
    }
}

// =============================================================================
// Order Upgrade (Tick Size Change)
// =============================================================================

/// Upgrade an existing order when tick size changes to a higher precision.
///
/// This allows us to bid higher (e.g., $0.999 instead of $0.99) when the market
/// allows more decimal places.
///
/// Returns the new OrderInfo if upgrade successful, None otherwise.
pub async fn upgrade_order_on_tick_change(
    trading: &TradingClient,
    token_id: &str,
    current_order: &OrderInfo,
    new_precision: u8,
    ctx: &MarketTrackerContext,
    balance_manager: &Arc<RwLock<BalanceManager>>,
) -> Option<OrderInfo> {
    let outcome_name = ctx.get_outcome_name(token_id);

    // Only upgrade if new precision is higher
    if new_precision <= current_order.precision {
        return None;
    }

    let old_price = 1.0 - 10_f64.powi(-(current_order.precision as i32));
    let new_price = 1.0 - 10_f64.powi(-(new_precision as i32));

    info!(
        "[WS {}] Upgrading order for {}: ${:.3} -> ${:.4} (precision {} -> {})",
        ctx.market_id, outcome_name, old_price, new_price, current_order.precision, new_precision
    );

    // Cancel existing order - only proceed if cancelled successfully
    if !cancel_order(trading, &current_order.order_id, token_id, ctx).await {
        warn!(
            "[WS {}] Failed to cancel old order for upgrade, keeping existing order for {}",
            ctx.market_id, outcome_name
        );
        // Return the current order so caller keeps tracking it
        return Some(current_order.clone());
    }

    // Place new order at higher precision
    let current_balance = balance_manager.read().unwrap().current_balance();
    let order_size = (current_balance * ctx.order_pct_of_collateral).round().max(1.0);

    match trading.buy(token_id, new_price, order_size).await {
        Ok(response) => {
            if let Some(order_id) = response.order_id {
                info!(
                    "[WS {}] Upgraded order placed for {}: {}",
                    ctx.market_id, outcome_name, order_id
                );
                Some(OrderInfo::new(order_id, new_precision))
            } else {
                warn!(
                    "[WS {}] Upgrade order placed but no order_id returned for {}",
                    ctx.market_id, outcome_name
                );
                None
            }
        }
        Err(e) => {
            error!(
                "[WS {}] Failed to place upgraded order for {}: {}",
                ctx.market_id, outcome_name, e
            );
            None
        }
    }
}
