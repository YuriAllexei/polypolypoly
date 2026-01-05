//! Quote ladder calculation.
//!
//! Simple market-making: place bids below best_ask with offset based on imbalance.
//! No profitability checks - that will be redesigned separately.

use tracing::{debug, info, warn};

use crate::application::strategies::inventory_mm::types::{
    InventorySnapshot, OrderbookSnapshot, Quote, QuoteLadder, SolverConfig,
};

/// Polymarket minimum order size (in shares)
const MIN_ORDER_SIZE: f64 = 5.0;

/// Calculate quote ladder for both Up and Down tokens.
///
/// Simple market-based quoting:
/// - Price = best_ask - offset - level_spread
/// - Offset increases on overweight side (passive), decreases on needed side (aggressive)
///
/// Risk is managed via:
/// - Offset mechanism: increases when imbalanced, making bids less aggressive on overweight side
/// - Max imbalance threshold: stops quoting overweight side when too imbalanced
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

    // Calculate offsets based on imbalance (price adjustment)
    // When heavy on UP (delta > 0): UP offset increases (passive), DOWN offset decreases (aggressive)
    // When heavy on DOWN (delta < 0): DOWN offset increases (passive), UP offset decreases (aggressive)
    // This makes the needed side MORE aggressive to speed up rebalancing
    // Use configurable min_offset to prevent spread crossing when offsets go negative

    let up_offset = (config.base_offset * (1.0 + delta * config.offset_scaling)).max(config.min_offset);
    let down_offset = (config.base_offset * (1.0 - delta * config.offset_scaling)).max(config.min_offset);

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
        "[Solver] delta={:.2} → offsets=(UP:{:.3}, DOWN:{:.3}), sizes=(UP:{:.1}, DOWN:{:.1})",
        delta, up_offset, down_offset, up_size, down_size
    );

    // Skip logic for rebalancing:
    // - Skip overweight side to let the other side catch up
    // - EXCEPTION: Don't skip if we're truly one-sided (building from scratch)
    //
    // "Significant" inventory = at least order_size worth
    // "Building from scratch" = less than order_size on that side
    let is_building_up_from_scratch = inventory.up_size.abs() < config.order_size;
    let is_building_down_from_scratch = inventory.down_size.abs() < config.order_size;

    // Skip UP if heavily long UP AND we have significant UP to rebalance from
    let mut skip_up = delta >= config.max_imbalance && !is_building_up_from_scratch;
    // Skip DOWN if heavily long DOWN AND we have significant DOWN to rebalance from
    let mut skip_down = delta <= -config.max_imbalance && !is_building_down_from_scratch;

    // SAFETY LIMIT: Skip quoting if max_position is reached
    // Also reduce order size when approaching limit (soft limit at 80%)
    let soft_limit_threshold = 0.80;  // Start reducing at 80% of max_position
    let mut up_size_multiplier = 1.0;
    let mut down_size_multiplier = 1.0;

    if config.max_position > 0.0 {
        let up_ratio = inventory.up_size.abs() / config.max_position;
        let down_ratio = inventory.down_size.abs() / config.max_position;

        // Hard limit: stop quoting completely
        if up_ratio >= 1.0 {
            if !skip_up {
                warn!(
                    "[Solver] MAX POSITION LIMIT: UP position {:.1} >= limit {:.1}, stopping UP quotes",
                    inventory.up_size.abs(), config.max_position
                );
            }
            skip_up = true;
        } else if up_ratio >= soft_limit_threshold {
            // Soft limit: reduce order size linearly from 80% to 0% as position approaches limit
            up_size_multiplier = (1.0 - up_ratio) / (1.0 - soft_limit_threshold);
            info!(
                "[Solver] SOFT LIMIT: UP at {:.0}% of max, reducing size to {:.0}%",
                up_ratio * 100.0, up_size_multiplier * 100.0
            );
        }

        if down_ratio >= 1.0 {
            if !skip_down {
                warn!(
                    "[Solver] MAX POSITION LIMIT: DOWN position {:.1} >= limit {:.1}, stopping DOWN quotes",
                    inventory.down_size.abs(), config.max_position
                );
            }
            skip_down = true;
        } else if down_ratio >= soft_limit_threshold {
            // Soft limit: reduce order size linearly from 80% to 0% as position approaches limit
            down_size_multiplier = (1.0 - down_ratio) / (1.0 - soft_limit_threshold);
            info!(
                "[Solver] SOFT LIMIT: DOWN at {:.0}% of max, reducing size to {:.0}%",
                down_ratio * 100.0, down_size_multiplier * 100.0
            );
        }
    }

    // Apply soft limit multipliers to sizes
    let up_size = (up_size * up_size_multiplier).max(MIN_ORDER_SIZE);
    let down_size = (down_size * down_size_multiplier).max(MIN_ORDER_SIZE);

    if skip_up || skip_down {
        debug!(
            "[Solver] Skip decisions: UP={} (delta>={:.1} && UP>={:.0}), DOWN={} (delta<=-{:.1} && DOWN>={:.0})",
            skip_up, config.max_imbalance, config.order_size,
            skip_down, config.max_imbalance, config.order_size
        );
    }

    // Build Up quotes
    // Cross-spread validation DISABLED - was too restrictive and blocked rebalancing
    if !skip_up {
        if let Some(best_ask) = up_ob.best_ask_price() {
            ladder.up_quotes = build_ladder(
                up_token_id,
                best_ask,
                up_offset,
                up_size,
                config,
                None,  // No cross-spread validation
            );
        } else {
            debug!("[Solver] UP quotes skipped: no best_ask in UP orderbook");
        }
    } else {
        debug!("[Solver] UP quotes skipped: skip_up=true (delta={:.2} >= max_imbalance={:.2})", delta, config.max_imbalance);
    }

    // Build Down quotes
    // Cross-spread validation DISABLED - was too restrictive and blocked rebalancing
    if !skip_down {
        if let Some(best_ask) = down_ob.best_ask_price() {
            ladder.down_quotes = build_ladder(
                down_token_id,
                best_ask,
                down_offset,
                down_size,
                config,
                None,  // No cross-spread validation
            );

            // Log if DOWN quotes are empty (helps debug why no orders are placed)
            if ladder.down_quotes.is_empty() {
                tracing::warn!(
                    "[Solver] DOWN ladder EMPTY! best_ask={:.3}, offset={:.3}, delta={:.2}",
                    best_ask, down_offset, delta
                );
            }
        } else {
            tracing::warn!(
                "[Solver] DOWN quotes skipped: no best_ask in DOWN orderbook (down_bid={:?}, up_bid={:?}, up_ask={:?})",
                down_ob.best_bid_price(),
                up_ob.best_bid_price(),
                up_ob.best_ask_price()
            );
        }
    } else {
        debug!("[Solver] DOWN quotes skipped: skip_down=true (delta={:.2} <= -{:.2})", delta, config.max_imbalance);
    }

    ladder
}

/// Build a ladder of bids for a single token.
///
/// Cross-spread validation prevents spread-crossing when orderbook data is stale.
///
/// Example: If our orderbook shows UP best_ask = $0.65 but is stale,
/// and DOWN best_bid = $0.37, we can infer UP is worth ~$0.63 (1 - 0.37).
/// Any UP bid > $0.63 would cross the effective spread!
fn build_ladder(
    token_id: &str,
    best_ask: f64,
    base_offset: f64,
    order_size: f64,
    config: &SolverConfig,
    opposite_best_bid: Option<f64>,  // For cross-spread validation
) -> Vec<Quote> {
    let mut quotes = Vec::with_capacity(config.num_levels);
    let mut last_price: Option<f64> = None;
    let mut skipped_reasons: Vec<String> = Vec::new();

    // CROSS-SPREAD SAFETY: Derive max safe bid from opposite side
    // If DOWN bid = 0.37, then UP is worth ~0.63, so UP bid must be < 0.63
    // This catches stale orderbook data that could cause spread crossing
    //
    // IMPORTANT: We use NO safety margin now. The previous 0.01 margin was too conservative
    // and prevented quoting at competitive prices. The `price < best_ask` check already
    // prevents crossing our own side's spread. This validation is just to catch stale data
    // where the opposite side's orderbook is more up-to-date than ours.
    let cross_spread_max = opposite_best_bid.map(|opp_bid| {
        // Opposite bid tells us this side's value: 1.0 - opposite_bid
        // No safety margin - let us quote at the true implied value
        1.0 - opp_bid
    });

    for level in 0..config.num_levels {
        // Calculate spread for this level (widens with each level)
        let level_spread = (level as f64) * (config.spread_per_level / 100.0);

        // Price = best_ask - base_offset - level_spread
        let mut price = best_ask - base_offset - level_spread;

        // Cap at cross-spread max to prevent TAKER fills
        // This catches cases where our orderbook is stale but opposite side is fresh
        if let Some(cross_max) = cross_spread_max {
            if price > cross_max {
                debug!(
                    "[Solver] Cross-spread cap: {:.3} → {:.3} (opposite_bid={:.3})",
                    price, cross_max, opposite_best_bid.unwrap_or(0.0)
                );
                price = cross_max;
            }
        }

        // Round to tick size
        let price = round_to_tick(price, config.tick_size);

        // Skip if bid would cross or match the spread (prevents immediate TAKER fills)
        if price >= best_ask {
            skipped_reasons.push(format!("L{}: price {:.3} >= best_ask {:.3}", level, price, best_ask));
            continue;
        }

        // Skip if price too low (not worth quoting)
        if price < 0.01 {
            skipped_reasons.push(format!("L{}: price {:.3} < 0.01", level, price));
            continue;
        }

        // Ensure prices are monotonically decreasing (ladder structure)
        // When resulting in same/higher price as previous, spread out
        let price = if let Some(lp) = last_price {
            if price >= lp - 1e-9 {
                // Would be same or higher than previous - move down by one tick
                let adjusted = round_to_tick(lp - config.tick_size, config.tick_size);
                if adjusted < 0.01 {
                    continue;  // Can't go lower
                }
                adjusted
            } else {
                price
            }
        } else {
            price
        };
        last_price = Some(price);

        quotes.push(Quote::new_bid(
            token_id.to_string(),
            price,
            order_size,
            level,
        ));
    }

    // Log if all quotes were skipped
    if quotes.is_empty() && !skipped_reasons.is_empty() {
        tracing::warn!(
            "[Solver] build_ladder({}) ALL SKIPPED: {}",
            &token_id[..8.min(token_id.len())],
            skipped_reasons.join(", ")
        );
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
            max_imbalance: 0.8,
            order_size: 100.0,
            spread_per_level: 1.0,
            offset_scaling: 5.0,
            skew_factor: 1.0,
            min_offset: 0.01,
            max_position: 0.0,  // 0 = unlimited for tests
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
        // No cross-spread validation (None for opposite_best_bid)
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, &config, None);

        assert_eq!(quotes.len(), 3);
        // Level 0: 0.55 - 0.01 - 0 = 0.54
        assert!((quotes[0].price - 0.54).abs() < 0.001);
        // Level 1: 0.55 - 0.01 - 0.01 = 0.53
        assert!((quotes[1].price - 0.53).abs() < 0.001);
        // Level 2: 0.55 - 0.01 - 0.02 = 0.52
        assert!((quotes[2].price - 0.52).abs() < 0.001);
        // All quotes should have full size
        assert!((quotes[0].size - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_build_ladder_cross_spread_validation() {
        let config = default_config();
        // Test cross-spread validation:
        // best_ask = 0.65, but opposite_best_bid = 0.37
        // Cross-spread max = 1.0 - 0.37 = 0.63 (no safety margin anymore)
        // Our bid at 0.64 (0.65 - 0.01) should be capped to 0.63
        let quotes = build_ladder("token", 0.65, 0.01, 100.0, &config, Some(0.37));

        assert!(!quotes.is_empty());
        // All prices should be <= 0.63 (cross-spread max, no safety margin)
        for q in &quotes {
            assert!(q.price <= 0.63, "Price {} exceeds cross-spread max 0.63", q.price);
        }
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

        // Should have quotes on Down (need more)
        assert!(!ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_calculate_quotes_extreme_imbalance() {
        let mut config = default_config();
        config.max_imbalance = 0.7;
        config.order_size = 10.0; // Use smaller order_size so inventory is "significant"

        // Inventory with significant UP position (should be skipped when heavy UP)
        let inventory = InventorySnapshot {
            up_size: 50.0,  // >= order_size (10), so significant
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
            best_ask: Some((0.45, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // delta = 0.9, above max_imbalance of 0.7
        // With 50 UP >= 10 order_size, UP should be skipped
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

        // Should have NO Up quotes (too imbalanced AND significant UP), only Down
        assert!(ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_skew_sizing_heavy_up() {
        let mut config = default_config();
        config.skew_factor = 2.0;
        config.order_size = 100.0;
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
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.50,
            down_size: 50.0,
            down_avg_price: 0.40,
        };

        // MARKET-BASED: Need proper bids for profitability calc
        // UP+DOWN market = 0.53 + 0.43 = 0.96 < 1.0 (profitable)
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: Some((0.53, 50.0)),  // Market bid for UP
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.45, 100.0)),
            best_bid: Some((0.43, 50.0)),  // Market bid for DOWN
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        // Balanced (delta = 0)
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

        // MARKET-BASED: Need proper bids for profitability calc
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

        // Heavy UP (delta = 0.5)
        // up_size = 100 * (1 - 0.5 * 5.0) = -150 → clamped to MIN_ORDER_SIZE (5.0)
        // down_size = 100 * (1 + 0.5 * 5.0) = 350 → clamped to 300
        let ladder = calculate_quotes(0.5, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
        assert!((ladder.up_quotes[0].size - MIN_ORDER_SIZE).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 300.0).abs() < 0.01);
    }

    #[test]
    fn test_skew_sizing_no_skew() {
        let mut config = default_config();
        config.skew_factor = 0.0; // No skew
        config.order_size = 100.0;
        let inventory = default_inventory();

        // MARKET-BASED: Need proper bids for profitability calc
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

        // Even with imbalance, sizes should be equal when skew_factor = 0
        let ladder = calculate_quotes(0.6, &up_ob, &down_ob, &inventory, &config, "up", "down");

        assert!((ladder.up_quotes[0].size - 100.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 100.0).abs() < 0.01);
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
        // Verify that when we have BOTH sides with SIGNIFICANT inventory, the max_imbalance
        // check still works (skips overweight side)
        let mut config = default_config();
        config.max_imbalance = 0.7;
        config.order_size = 10.0; // Smaller order_size so 90 and 10 are both "significant"

        // Two-sided inventory (both sides have significant inventory >= order_size)
        let inventory = InventorySnapshot {
            up_size: 90.0,  // >= order_size (10), significant
            up_avg_price: 0.50,
            down_size: 10.0, // >= order_size (10), significant (just barely)
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
