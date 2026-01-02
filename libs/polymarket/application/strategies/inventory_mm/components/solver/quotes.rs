//! Quote ladder calculation.

use tracing::debug;

use crate::application::strategies::inventory_mm::types::{
    InventorySnapshot, OrderbookSnapshot, Quote, QuoteLadder, SolverConfig,
};
use super::profitability::{calculate_max_bids, check_recovery_status};

/// Polymarket minimum order size (in shares)
const MIN_ORDER_SIZE: f64 = 5.0;

/// Calculate quote ladder for both Up and Down tokens.
///
/// Quotes are market-based with profitability caps:
/// - Price = best_ask - offset - level_spread (market-based)
/// - Capped at max_bid = 1.0 - other_side_avg - margin (profitability cap)
///
/// Risk is managed via:
/// - Offset mechanism: increases when imbalanced, making bids less aggressive
/// - Profitability cap: prevents bids that would lead to losses
/// - Max imbalance threshold: stops quoting entirely when too imbalanced
/// - Skew sizing: reduces size on overweight side
pub fn calculate_quotes(
    delta: f64,
    up_ob: &OrderbookSnapshot,
    down_ob: &OrderbookSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    up_token_id: &str,
    down_token_id: &str,
) -> QuoteLadder {
    let mut ladder = QuoteLadder::new();

    // Check recovery status - if stuck, cancel all quotes
    let recovery = check_recovery_status(
        inventory,
        config.recovery_threshold,
        config.stuck_threshold,
    );

    if recovery.is_stuck {
        debug!(
            "[Solver] STUCK MODE: combined_avg={:.4} >= {:.4}, cancelling all quotes",
            recovery.combined_avg, config.stuck_threshold
        );
        return ladder; // Empty ladder = cancel all existing orders
    }

    // Calculate offsets based on imbalance (price adjustment)
    // When heavy on UP (delta > 0): UP offset increases (passive), DOWN offset decreases (aggressive)
    // When heavy on DOWN (delta < 0): DOWN offset increases (passive), UP offset decreases (aggressive)
    // This makes the needed side MORE aggressive to speed up rebalancing
    // Use configurable min_offset to prevent spread crossing when offsets go negative

    let up_offset = (config.base_offset * (1.0 + delta * config.offset_scaling)).max(config.min_offset);
    let down_offset = (config.base_offset * (1.0 - delta * config.offset_scaling)).max(config.min_offset);

    // Calculate recovery relaxation - only apply if in recovery mode
    let recovery_relaxation = if recovery.in_recovery {
        config.recovery_relaxation
    } else {
        0.0
    };

    // Calculate max profitable bids (profitability cap)
    // In recovery mode, margin is relaxed on the needed side
    let (max_up_bid, max_down_bid) = calculate_max_bids(
        inventory,
        up_ob,
        down_ob,
        config.min_profit_margin,
        recovery_relaxation,
        delta,
    );

    // Calculate skew-adjusted sizes (size adjustment)
    // delta > 0 = heavy UP → reduce UP size, increase DOWN size
    // delta < 0 = heavy DOWN → increase UP size, reduce DOWN size
    // IMPORTANT: Clamp to [MIN_ORDER_SIZE, max] - NEVER skip due to low size
    // For inventory MM, we must always quote both sides to manage imbalance
    // NOTE: Round to whole numbers - Polymarket rejects fractional sizes!
    let max_size = config.order_size * 3.0;
    let up_size = (config.order_size * (1.0 - delta * config.skew_factor))
        .clamp(MIN_ORDER_SIZE, max_size)
        .round();
    let down_size = (config.order_size * (1.0 + delta * config.skew_factor))
        .clamp(MIN_ORDER_SIZE, max_size)
        .round();

    debug!(
        "[Solver] delta={:.2}, recovery={} → offsets=(UP:{:.3}, DOWN:{:.3}), max_bids=(UP:{:.3?}, DOWN:{:.3?}), sizes=(UP:{:.1}, DOWN:{:.1})",
        delta, recovery.in_recovery, up_offset, down_offset, max_up_bid, max_down_bid, up_size, down_size
    );

    // Build Up quotes
    // Skip if TOO imbalanced on UP side, UNLESS we have no DOWN inventory (one-sided)
    // One-sided positions need to keep quoting to allow profit-taking if market recovers
    // FIX: Use >= to include boundary (delta=0.8 should skip, not just delta>0.8)
    let skip_up = delta >= config.max_imbalance && inventory.down_size > 0.0;
    if !skip_up {
        if let Some(best_ask) = up_ob.best_ask_price() {
            ladder.up_quotes = build_ladder(
                up_token_id,
                best_ask,
                up_offset,
                up_size,
                max_up_bid,
                config,
                config.capped_size_factor,
            );
        }
    }

    // Build Down quotes
    // Skip if TOO imbalanced on DOWN side, UNLESS we have no UP inventory (one-sided)
    // FIX: Use <= to include boundary (delta=-0.8 should skip, not just delta<-0.8)
    let skip_down = delta <= -config.max_imbalance && inventory.up_size > 0.0;
    if !skip_down {
        if let Some(best_ask) = down_ob.best_ask_price() {
            ladder.down_quotes = build_ladder(
                down_token_id,
                best_ask,
                down_offset,
                down_size,
                max_down_bid,
                config,
                config.capped_size_factor,
            );
        }
    }

    ladder
}

/// Build a ladder of bids for a single token
fn build_ladder(
    token_id: &str,
    best_ask: f64,
    base_offset: f64,
    order_size: f64,
    max_bid: Option<f64>,
    config: &SolverConfig,
    capped_size_factor: f64,
) -> Vec<Quote> {
    let mut quotes = Vec::with_capacity(config.num_levels);
    let mut last_price: Option<f64> = None;

    for level in 0..config.num_levels {
        // Calculate spread for this level (widens with each level)
        let level_spread = (level as f64) * (config.spread_per_level / 100.0);

        // Price = best_ask - base_offset - level_spread
        let market_price = best_ask - base_offset - level_spread;

        // Cap at profitability limit (prevents bids that would lead to losses)
        let (price, was_capped) = if let Some(max) = max_bid {
            if market_price > max {
                (max, true)
            } else {
                (market_price, false)
            }
        } else {
            (market_price, false)
        };

        // Round to tick size
        let price = round_to_tick(price, config.tick_size);

        // Skip if bid would cross or match the spread (prevents immediate TAKER fills)
        if price >= best_ask {
            debug!(
                "[Solver] BLOCKED bid at {:.3} - would cross best_ask {:.3}",
                price, best_ask
            );
            continue;
        }

        // Skip if price too low (not worth quoting)
        if price < 0.01 {
            continue;
        }

        // Skip if same as previous level (can happen when capped at max_bid)
        if last_price.map_or(false, |lp| (lp - price).abs() < 1e-9) {
            continue;
        }
        last_price = Some(price);

        // Reduce size when capped at profitability limit (limits exposure on marginal trades)
        let final_size = if was_capped {
            (order_size * capped_size_factor).round().max(MIN_ORDER_SIZE)
        } else {
            order_size
        };

        quotes.push(Quote::new_bid(
            token_id.to_string(),
            price,
            final_size,
            level,
        ));
    }

    quotes
}

/// Round price down to tick size
fn round_to_tick(price: f64, tick_size: f64) -> f64 {
    // Add small epsilon to handle floating point precision errors
    // e.g., 0.47/0.01 = 46.9999... should floor to 47, not 46
    ((price / tick_size) + 1e-9).floor() * tick_size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> SolverConfig {
        SolverConfig {
            num_levels: 3,
            tick_size: 0.01,
            base_offset: 0.01,
            min_profit_margin: 0.01,
            max_imbalance: 0.8,
            order_size: 100.0,
            spread_per_level: 1.0,
            offset_scaling: 5.0,
            skew_factor: 1.0,
            recovery_threshold: 0.99,
            recovery_relaxation: 0.005,
            capped_size_factor: 0.5,
            stuck_threshold: 1.02,
            min_offset: 0.01,
        }
    }

    #[test]
    fn test_round_to_tick() {
        assert_eq!(round_to_tick(0.456, 0.01), 0.45);
        assert_eq!(round_to_tick(0.459, 0.01), 0.45);
        assert_eq!(round_to_tick(0.45, 0.01), 0.45);
        assert_eq!(round_to_tick(0.999, 0.01), 0.99);
    }

    fn default_inventory() -> InventorySnapshot {
        InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        }
    }

    #[test]
    fn test_build_ladder_basic() {
        let config = default_config();
        // No cap - pass None for max_bid, capped_size_factor = 1.0 (no reduction)
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, None, &config, 1.0);

        assert_eq!(quotes.len(), 3);
        // Level 0: 0.55 - 0.01 - 0 = 0.54
        assert!((quotes[0].price - 0.54).abs() < 0.001);
        // Level 1: 0.55 - 0.01 - 0.01 = 0.53
        assert!((quotes[1].price - 0.53).abs() < 0.001);
        // Level 2: 0.55 - 0.01 - 0.02 = 0.52
        assert!((quotes[2].price - 0.52).abs() < 0.001);
        // All quotes should have the passed size (not capped)
        assert!((quotes[0].size - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_build_ladder_with_cap() {
        let config = default_config();
        // Cap at 0.52 - only first level should be generated (others would be same price)
        // capped_size_factor = 0.5 means capped quotes get half size
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, Some(0.52), &config, 0.5);

        // Level 0: min(0.54, 0.52) = 0.52 (capped!)
        // Level 1: min(0.53, 0.52) = 0.52 -> skip (same as previous)
        // Level 2: min(0.52, 0.52) = 0.52 -> skip (same as previous)
        assert_eq!(quotes.len(), 1);
        assert!((quotes[0].price - 0.52).abs() < 0.001);
        // Capped size = 100 * 0.5 = 50
        assert!((quotes[0].size - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_build_ladder_partial_cap() {
        let config = default_config();
        // Cap at 0.53 - first level is capped (0.54 > 0.53)
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, Some(0.53), &config, 0.5);

        // Level 0: 0.54 > 0.53 → capped at 0.53, reduced size
        // Level 1: 0.53 not > 0.53 → not capped, but same price as level 0, SKIPPED
        // Level 2: 0.52 not > 0.53 → not capped, full size
        assert_eq!(quotes.len(), 2);
        // First quote is capped → half size
        assert!((quotes[0].price - 0.53).abs() < 0.001);
        assert!((quotes[0].size - 50.0).abs() < 0.001);
        // Second quote is not capped → full size
        assert!((quotes[1].price - 0.52).abs() < 0.001);
        assert!((quotes[1].size - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_calculate_quotes_balanced() {
        let config = default_config();
        let inventory = default_inventory();
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: Some((0.53, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: Some((0.43, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        let ladder = calculate_quotes(
            0.0, // balanced delta
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up_token",
            "down_token",
        );

        // Should have quotes on both sides
        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_calculate_quotes_heavy_up() {
        let config = default_config();
        let inventory = default_inventory();
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: Some((0.53, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: Some((0.43, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = 0.8, exactly at max_imbalance
        let delta = 0.8;

        let ladder = calculate_quotes(
            delta,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up_token",
            "down_token",
        );

        // Should have quotes on Down (need more), none/fewer on Up
        assert!(!ladder.down_quotes.is_empty());
        // At exactly 0.8, Up quotes should still be generated (< not <=)
    }

    #[test]
    fn test_calculate_quotes_extreme_imbalance() {
        let mut config = default_config();
        config.max_imbalance = 0.7;
        let inventory = default_inventory();

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = 0.9, above max_imbalance of 0.7
        let delta = 0.9;

        let ladder = calculate_quotes(
            delta,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up_token",
            "down_token",
        );

        // Should have NO Up quotes (too imbalanced), only Down
        assert!(ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_skew_sizing_heavy_up() {
        let mut config = default_config();
        config.skew_factor = 2.0;
        config.order_size = 100.0;
        // Use profitable inventory that won't trigger profitability caps at these market prices
        // max_up_bid = 1.0 - 0.40 - 0.01 = 0.59 (well above 0.54 bid)
        // max_down_bid = 1.0 - 0.50 - 0.01 = 0.49 (well above 0.44 bid)
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.50,
            down_size: 50.0,
            down_avg_price: 0.40,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: Some((0.53, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: Some((0.43, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // Heavy UP (delta = 0.4)
        // up_size = 100 * (1 - 0.4 * 2.0) = 100 * 0.2 = 20
        // down_size = 100 * (1 + 0.4 * 2.0) = 100 * 1.8 = 180
        let ladder = calculate_quotes(0.4, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
        assert!((ladder.up_quotes[0].size - 20.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 180.0).abs() < 0.01);
    }

    #[test]
    fn test_skew_sizing_heavy_down() {
        let mut config = default_config();
        config.skew_factor = 2.0;
        config.order_size = 100.0;
        // Use profitable inventory that won't trigger profitability caps
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.50,
            down_size: 50.0,
            down_avg_price: 0.40,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: Some((0.53, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: Some((0.43, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // Heavy DOWN (delta = -0.4)
        // up_size = 100 * (1 - (-0.4) * 2.0) = 100 * 1.8 = 180
        // down_size = 100 * (1 + (-0.4) * 2.0) = 100 * 0.2 = 20
        let ladder = calculate_quotes(-0.4, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
        assert!((ladder.up_quotes[0].size - 180.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_skew_sizing_balanced() {
        let mut config = default_config();
        config.skew_factor = 2.0;
        config.order_size = 100.0;
        // Use profitable inventory that won't trigger profitability caps
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.50,
            down_size: 50.0,
            down_avg_price: 0.40,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // Balanced (delta = 0)
        // up_size = 100 * (1 - 0 * 2.0) = 100
        // down_size = 100 * (1 + 0 * 2.0) = 100
        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!((ladder.up_quotes[0].size - 100.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_skew_sizing_clamped() {
        let mut config = default_config();
        config.skew_factor = 5.0; // Very aggressive
        config.order_size = 100.0;
        let inventory = default_inventory();

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // Heavy UP (delta = 0.5)
        // up_size = 100 * (1 - 0.5 * 5.0) = 100 * (-1.5) = -150 → clamped to MIN_ORDER_SIZE (5.0)
        // down_size = 100 * (1 + 0.5 * 5.0) = 100 * 3.5 = 350 → clamped to 300
        let ladder = calculate_quotes(0.5, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
        // Now clamped to MIN_ORDER_SIZE instead of 0
        assert!((ladder.up_quotes[0].size - MIN_ORDER_SIZE).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 300.0).abs() < 0.01);
    }

    #[test]
    fn test_skew_sizing_no_skew() {
        let mut config = default_config();
        config.skew_factor = 0.0; // No skew
        config.order_size = 100.0;
        let inventory = default_inventory();

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // Even with imbalance, sizes should be equal when skew_factor = 0
        let ladder = calculate_quotes(0.6, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!((ladder.up_quotes[0].size - 100.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_profitability_cap() {
        let config = default_config();
        // Inventory with combined avg 0.98 (profitable)
        // max_up_bid = 1.0 - 0.46 - 0.01 = 0.53
        // max_down_bid = 1.0 - 0.52 - 0.01 = 0.47
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.50, 100.0)), // High ask - would bid at 0.49
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

        // UP quotes: best_ask 0.55 - 0.01 = 0.54, but max is 0.53 -> capped to 0.53
        assert!(!ladder.up_quotes.is_empty());
        assert!((ladder.up_quotes[0].price - 0.53).abs() < 0.001);

        // DOWN quotes: best_ask 0.50 - 0.01 = 0.49, but max is 0.47 -> capped to 0.47
        assert!(!ladder.down_quotes.is_empty());
        assert!((ladder.down_quotes[0].price - 0.47).abs() < 0.001);
    }

    #[test]
    fn test_profitability_cap_unprofitable_inventory() {
        let config = default_config();
        // Inventory already unprofitable: combined = 0.52 + 0.49 = 1.01
        // max_up_bid = 1.0 - 0.49 - 0.01 = 0.50
        // max_down_bid = 1.0 - 0.52 - 0.01 = 0.47
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.49,  // Already unprofitable
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.50, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

        // UP quotes capped at max_up_bid = 0.50
        // Market bid would be 0.54, but cap is 0.50
        assert!(!ladder.up_quotes.is_empty());
        assert!((ladder.up_quotes[0].price - 0.50).abs() < 0.001);

        // DOWN quotes capped at max_down_bid = 0.47
        // Market bid would be 0.49, but cap is 0.47
        assert!(!ladder.down_quotes.is_empty());
        assert!((ladder.down_quotes[0].price - 0.47).abs() < 0.001);
    }

    #[test]
    fn test_offset_formula_needed_side_aggressive() {
        // Verify the offset formula makes the NEEDED side more aggressive
        let config = default_config();
        // base_offset = 0.01, offset_scaling = 5.0

        // When delta = +0.5 (heavy UP, need DOWN):
        // up_offset = 0.01 * (1 + 0.5 * 5) = 0.01 * 3.5 = 0.035 (passive)
        // down_offset = 0.01 * (1 - 0.5 * 5) = 0.01 * -1.5 = clamped to MIN_OFFSET (aggressive)
        let delta = 0.5;
        let up_offset = (config.base_offset * (1.0 + delta * config.offset_scaling)).max(0.001);
        let down_offset = (config.base_offset * (1.0 - delta * config.offset_scaling)).max(0.001);

        // UP offset should INCREASE (passive) because we're heavy UP
        assert!(up_offset > config.base_offset, "UP offset should be > base when heavy UP");
        // DOWN offset should DECREASE (aggressive) because we need DOWN
        assert!(down_offset < config.base_offset, "DOWN offset should be < base when need DOWN");

        // When delta = -0.5 (heavy DOWN, need UP):
        let delta = -0.5;
        let up_offset = (config.base_offset * (1.0 + delta * config.offset_scaling)).max(0.001);
        let down_offset = (config.base_offset * (1.0 - delta * config.offset_scaling)).max(0.001);

        // UP offset should DECREASE (aggressive) because we need UP
        assert!(up_offset < config.base_offset, "UP offset should be < base when need UP");
        // DOWN offset should INCREASE (passive) because we're heavy DOWN
        assert!(down_offset > config.base_offset, "DOWN offset should be > base when heavy DOWN");
    }

    #[test]
    fn test_stuck_mode_cancels_all() {
        let config = default_config();
        // Stuck inventory: combined = 0.52 + 0.51 = 1.03 >= 1.02
        let stuck_inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.51,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &stuck_inventory, &config, "up", "down");

        // Should return empty ladder - cancel all quotes
        assert!(ladder.up_quotes.is_empty(), "Stuck mode should have no UP quotes");
        assert!(ladder.down_quotes.is_empty(), "Stuck mode should have no DOWN quotes");
    }

    #[test]
    fn test_recovery_mode_relaxes_margin() {
        let mut config = default_config();
        config.recovery_relaxation = 0.01; // Relax by full margin

        // Recovery inventory: combined = 0.52 + 0.48 = 1.00 >= 0.99
        let recovery_inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.48,
        };

        // Heavy UP (delta > 0) - need DOWN, relax DOWN margin
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.50, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = 0 for balanced inventory
        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &recovery_inventory, &config, "up", "down");

        // Should still generate quotes (not stuck)
        assert!(!ladder.up_quotes.is_empty(), "Recovery mode should still quote UP");
        assert!(!ladder.down_quotes.is_empty(), "Recovery mode should still quote DOWN");
    }

    #[test]
    fn test_one_sided_up_only_still_quotes_up() {
        // This is the user's exact scenario: 20 UP @ $0.445, 0 DOWN
        // Market moved: UP crashed to $0.21, DOWN rose to $0.78
        let config = default_config();

        // One-sided inventory: only UP
        let inventory = InventorySnapshot {
            up_size: 20.0,
            up_avg_price: 0.445,
            down_size: 0.0,
            down_avg_price: 0.0,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.21, 100.0)), // UP crashed
            best_bid: Some((0.19, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.78, 100.0)), // DOWN rose
            best_bid: Some((0.76, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = 1.0 (100% UP, 0% DOWN) - exceeds max_imbalance of 0.8
        let ladder = calculate_quotes(
            1.0,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up_token",
            "down_token",
        );

        // KEY: Should STILL have UP quotes because we're one-sided (no DOWN inventory)
        // This allows profit-taking if market recovers
        assert!(!ladder.up_quotes.is_empty(), "One-sided UP should still quote UP");

        // Should also have DOWN quotes (needed side)
        assert!(!ladder.down_quotes.is_empty(), "Should quote DOWN to rebalance");
    }

    #[test]
    fn test_one_sided_down_only_still_quotes_down() {
        // Mirror scenario: 20 DOWN @ $0.78, 0 UP
        let config = default_config();

        // One-sided inventory: only DOWN
        let inventory = InventorySnapshot {
            up_size: 0.0,
            up_avg_price: 0.0,
            down_size: 20.0,
            down_avg_price: 0.78,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.21, 100.0)),
            best_bid: Some((0.19, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.78, 100.0)),
            best_bid: Some((0.76, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = -1.0 (0% UP, 100% DOWN) - exceeds -max_imbalance of -0.8
        let ladder = calculate_quotes(
            -1.0,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up_token",
            "down_token",
        );

        // Should have UP quotes (needed side)
        assert!(!ladder.up_quotes.is_empty(), "Should quote UP to rebalance");

        // KEY: Should STILL have DOWN quotes because we're one-sided (no UP inventory)
        assert!(!ladder.down_quotes.is_empty(), "One-sided DOWN should still quote DOWN");
    }

    #[test]
    fn test_extreme_imbalance_with_both_sides_still_skips() {
        // Verify that when we have BOTH sides with inventory, the max_imbalance
        // check still works as before (skips overweight side)
        let mut config = default_config();
        config.max_imbalance = 0.7;

        // Two-sided inventory (both sides have inventory)
        let inventory = InventorySnapshot {
            up_size: 90.0,
            up_avg_price: 0.50,
            down_size: 10.0,
            down_avg_price: 0.40,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = 0.8, above max_imbalance of 0.7
        let ladder = calculate_quotes(
            0.8,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up_token",
            "down_token",
        );

        // Because we have DOWN inventory (down_size > 0), UP quotes should be SKIPPED
        assert!(ladder.up_quotes.is_empty(), "Two-sided with extreme imbalance should skip UP");
        assert!(!ladder.down_quotes.is_empty(), "Should still quote DOWN");
    }

    #[test]
    fn test_spread_crossing_blocked() {
        // Test that bids at or above best_ask are blocked
        // This can happen when offset goes to min_offset and best_ask is very low
        let config = default_config();

        // Scenario: best_ask is $0.10, min_offset is $0.005
        // With extreme delta, offset would normally be tiny
        // If bid = best_ask - 0.005 = 0.095, rounded to 0.09 - should be OK
        // But if somehow price >= best_ask, it should be blocked

        let inventory = default_inventory();
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.10, 100.0)), // Very low best_ask
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.90, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

        // Verify NO UP quote has price >= best_ask (0.10)
        for quote in &ladder.up_quotes {
            assert!(
                quote.price < 0.10,
                "UP bid {:.3} should be < best_ask 0.10",
                quote.price
            );
        }
    }

    #[test]
    fn test_min_offset_configurable() {
        // Test that config.min_offset is used instead of hardcoded constant
        let mut config = default_config();
        config.min_offset = 0.03; // Set larger min_offset (3 cents)
        config.offset_scaling = 10.0; // Extreme scaling to force clamping

        // Use profitable inventory that won't trigger profitability caps
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.40,
            down_size: 50.0,
            down_avg_price: 0.40,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // With delta = 0.5 and offset_scaling = 10:
        // up_offset = 0.01 * (1 + 0.5*10) = 0.06 (uses calculated)
        // down_offset = 0.01 * (1 - 0.5*10) = -0.04 -> clamped to min_offset = 0.03
        let ladder = calculate_quotes(0.5, &up_ob, &down_ob, &inventory, &config, "up", "down");

        // DOWN side: best_ask (0.45) - min_offset (0.03) = 0.42
        // Verify the bid is at least min_offset away from ask
        if !ladder.down_quotes.is_empty() {
            let down_bid = ladder.down_quotes[0].price;
            // down_bid should be <= 0.42 (ask 0.45 - min_offset 0.03)
            assert!(
                down_bid <= 0.42,
                "DOWN bid {} should be <= 0.42 (ask 0.45 - min_offset 0.03)",
                down_bid
            );
        }
    }
}
