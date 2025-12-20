//! Orderbook checking logic for the Up or Down strategy.
//!
//! Handles monitoring orderbooks for the no-asks condition and managing timers.

use crate::application::strategies::up_or_down::services::log_no_asks_started;
use crate::application::strategies::up_or_down::types::{
    MarketTrackerContext, OrderbookCheckResult, TrackerState, FINAL_SECONDS_BYPASS,
};
use crate::infrastructure::SharedOrderbooks;
use chrono::Utc;
use std::time::Instant;
use tracing::{debug, info};

// =============================================================================
// Dynamic Threshold Calculation
// =============================================================================

/// Calculate dynamic no-ask threshold based on time remaining until market end.
///
/// Uses exponential decay formula:
/// threshold = min + (max - min) * (1 - exp(-time_remaining / tau))
///
/// - When far from market end (large time_remaining): threshold approaches max (conservative)
/// - When close to market end (small time_remaining): threshold approaches min (aggressive)
pub fn calculate_dynamic_threshold(ctx: &MarketTrackerContext) -> f64 {
    let now = Utc::now();
    let time_remaining = ctx
        .market_end_time
        .signed_duration_since(now)
        .num_milliseconds() as f64
        / 1000.0;

    // If past market end or at market end, use minimum threshold
    if time_remaining <= 0.0 {
        return ctx.threshold_min;
    }

    // Exponential decay formula
    ctx.threshold_min
        + (ctx.threshold_max - ctx.threshold_min)
            * (1.0 - (-time_remaining / ctx.threshold_tau).exp())
}

// =============================================================================
// Single Token Orderbook Check
// =============================================================================

/// Check a single token's orderbook and update timer state
pub fn check_token_orderbook(
    token_id: &str,
    has_asks: bool,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
) -> OrderbookCheckResult {
    let outcome_name = ctx.get_outcome_name(token_id);

    if has_asks {
        // Asks exist - reset timer and threshold state
        if state.no_asks_timers.remove(token_id).is_some() {
            state.threshold_triggered.remove(token_id);
            debug!(
                "⏹️  Timer RESET for {} ({}) - asks appeared in orderbook",
                token_id, outcome_name
            );
        }
        return OrderbookCheckResult::HasAsks;
    }

    // Check if we're in final seconds - bypass all waits
    let now = Utc::now();
    let time_remaining = ctx
        .market_end_time
        .signed_duration_since(now)
        .num_milliseconds() as f64
        / 1000.0;
    let in_final_seconds = time_remaining > 0.0 && time_remaining <= FINAL_SECONDS_BYPASS;

    // If in final seconds and no order placed yet, immediately trigger
    if in_final_seconds && !state.order_placed.contains_key(token_id) {
        info!(
            "[WS {}] Final {:.1}s - bypassing threshold wait for {}",
            ctx.market_id, time_remaining, outcome_name
        );
        state.threshold_triggered.insert(token_id.to_string());
        return OrderbookCheckResult::ThresholdExceeded {
            elapsed_secs: 0.0,
        };
    }

    // No asks - start timer if not running
    // Only log "STARTING TIMER" if:
    // 1. Timer doesn't exist yet
    // 2. We haven't already triggered threshold (prevents spam after order cycle)
    // 3. We don't already have an order placed (prevents spam after order placed)
    if !state.no_asks_timers.contains_key(token_id) {
        // Only log if this is truly a new detection (not a restart after order cycle)
        if !state.threshold_triggered.contains(token_id) && !state.order_placed.contains_key(token_id)
        {
            log_no_asks_started(ctx, token_id, &outcome_name);
        }
        state
            .no_asks_timers
            .insert(token_id.to_string(), Instant::now());
    }

    // Check if threshold exceeded using dynamic threshold
    if !state.threshold_triggered.contains(token_id) {
        if let Some(timer_start) = state.no_asks_timers.get(token_id) {
            let elapsed = timer_start.elapsed().as_secs_f64();
            let dynamic_threshold = calculate_dynamic_threshold(ctx);
            if elapsed >= dynamic_threshold {
                // Check if order already placed for this token
                if state.order_placed.contains_key(token_id) {
                    // Order already exists - silently skip (don't remove timer to prevent restart)
                    return OrderbookCheckResult::NoAsks;
                }
                state.threshold_triggered.insert(token_id.to_string());
                return OrderbookCheckResult::ThresholdExceeded {
                    elapsed_secs: elapsed,
                };
            }
        }
    }

    OrderbookCheckResult::NoAsks
}

// =============================================================================
// All Orderbooks Check
// =============================================================================

/// Check all orderbooks and return tokens that need orders placed.
///
/// Returns a tuple of:
/// - Vec of (token_id, outcome_name, elapsed_secs) for tokens that exceeded threshold
/// - bool indicating if all orderbooks are empty (market ended)
pub async fn check_all_orderbooks(
    orderbooks: &SharedOrderbooks,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
) -> (Vec<(String, String, f64)>, bool) {
    use crate::application::strategies::up_or_down::services::log_threshold_exceeded;

    let mut tokens_to_order = Vec::new();
    let mut all_empty = true;

    let token_data: Vec<(String, bool, bool)> = {
        let obs = orderbooks.read();
        ctx.token_ids
            .iter()
            .filter_map(|token_id| {
                obs.get(token_id).map(|orderbook| {
                    (
                        token_id.clone(),
                        !orderbook.asks.is_empty(),
                        !orderbook.bids.is_empty(),
                    )
                })
            })
            .collect()
    };

    for (token_id, has_asks, has_bids) in token_data {
        if has_asks || has_bids {
            all_empty = false;
        }

        match check_token_orderbook(&token_id, has_asks, state, ctx) {
            OrderbookCheckResult::ThresholdExceeded { elapsed_secs } => {
                let outcome_name = ctx.get_outcome_name(&token_id);
                let dynamic_threshold = calculate_dynamic_threshold(ctx);
                log_threshold_exceeded(
                    ctx,
                    &token_id,
                    &outcome_name,
                    elapsed_secs,
                    dynamic_threshold,
                );
                tokens_to_order.push((token_id.clone(), outcome_name, elapsed_secs));
            }
            _ => {}
        }
    }

    (tokens_to_order, all_empty)
}
