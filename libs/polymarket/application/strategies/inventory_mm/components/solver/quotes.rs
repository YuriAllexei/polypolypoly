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

    // POSITION LIMITS with IMBALANCE-AWARE logic
    //
    // Key insight: For inventory MM, the goal is BALANCED inventory (UP ≈ DOWN).
    // When imbalanced, we must let the lagging side catch up.
    //
    // Rules:
    // 1. Hard limit at 90% - stop quoting the AHEAD side (buffer for in-flight fills)
    // 2. Soft limit at 60% - start reducing size for the AHEAD side
    // 3. EXCEPTION: The lagging side is NOT limited when trying to catch up
    // 4. If reduced size < MIN_ORDER_SIZE (5), skip quoting entirely
    let soft_limit_threshold = 0.60;
    let hard_limit_threshold = 0.90;
    let mut up_size_multiplier = 1.0;
    let mut down_size_multiplier = 1.0;

    if config.max_position > 0.0 {
        let up_pos = inventory.up_size.abs();
        let down_pos = inventory.down_size.abs();
        let up_ratio = up_pos / config.max_position;
        let down_ratio = down_pos / config.max_position;

        // Determine which side is ahead (for imbalance-aware limiting)
        let imbalance = up_pos - down_pos;  // positive = UP ahead, negative = DOWN ahead
        let imbalance_threshold = config.order_size;  // significant if diff > one order size
        let up_is_ahead = imbalance > imbalance_threshold;
        let down_is_ahead = imbalance < -imbalance_threshold;

        // Apply limits only to the AHEAD side (or both if balanced)
        // The lagging side can quote freely to catch up
        let apply_up_limit = !down_is_ahead;  // limit UP unless DOWN is ahead
        let apply_down_limit = !up_is_ahead;  // limit DOWN unless UP is ahead

        if apply_up_limit {
            if up_ratio >= hard_limit_threshold {
                warn!(
                    "[Solver] UP HARD LIMIT: {:.0} >= {:.0}% of {:.0}, stopping UP quotes",
                    up_pos, hard_limit_threshold * 100.0, config.max_position
                );
                skip_up = true;
            } else if up_ratio >= soft_limit_threshold {
                up_size_multiplier = (hard_limit_threshold - up_ratio) / (hard_limit_threshold - soft_limit_threshold);
                debug!(
                    "[Solver] UP SOFT LIMIT: {:.0}% of max, size multiplier {:.0}%",
                    up_ratio * 100.0, up_size_multiplier * 100.0
                );
            }
        } else {
            debug!(
                "[Solver] UP limit BYPASSED: DOWN is ahead by {:.0}, letting UP catch up",
                -imbalance
            );
        }

        if apply_down_limit {
            if down_ratio >= hard_limit_threshold {
                warn!(
                    "[Solver] DOWN HARD LIMIT: {:.0} >= {:.0}% of {:.0}, stopping DOWN quotes",
                    down_pos, hard_limit_threshold * 100.0, config.max_position
                );
                skip_down = true;
            } else if down_ratio >= soft_limit_threshold {
                down_size_multiplier = (hard_limit_threshold - down_ratio) / (hard_limit_threshold - soft_limit_threshold);
                debug!(
                    "[Solver] DOWN SOFT LIMIT: {:.0}% of max, size multiplier {:.0}%",
                    down_ratio * 100.0, down_size_multiplier * 100.0
                );
            }
        } else {
            debug!(
                "[Solver] DOWN limit BYPASSED: UP is ahead by {:.0}, letting DOWN catch up",
                imbalance
            );
        }

        // Log imbalance status
        if up_is_ahead || down_is_ahead {
            info!(
                "[Solver] IMBALANCE: UP={:.0}, DOWN={:.0}, diff={:.0} → {} catching up",
                up_pos, down_pos, imbalance.abs(),
                if up_is_ahead { "DOWN" } else { "UP" }
            );
        }
    }

    // Apply multipliers to sizes
    let up_size_adjusted = up_size * up_size_multiplier;
    let down_size_adjusted = down_size * down_size_multiplier;

    // If reduced size is below minimum, skip quoting entirely (don't clamp to 5)
    // This prevents placing tiny orders that don't help with rebalancing
    if up_size_adjusted < MIN_ORDER_SIZE && !skip_up {
        debug!(
            "[Solver] UP size {:.1} < min {:.0} after multiplier, skipping UP quotes",
            up_size_adjusted, MIN_ORDER_SIZE
        );
        skip_up = true;
    }
    if down_size_adjusted < MIN_ORDER_SIZE && !skip_down {
        debug!(
            "[Solver] DOWN size {:.1} < min {:.0} after multiplier, skipping DOWN quotes",
            down_size_adjusted, MIN_ORDER_SIZE
        );
        skip_down = true;
    }

    // Round to whole numbers - Polymarket requires integer sizes
    let up_size = round_size(up_size_adjusted.max(MIN_ORDER_SIZE));
    let down_size = round_size(down_size_adjusted.max(MIN_ORDER_SIZE));

    if skip_up || skip_down {
        debug!(
            "[Solver] Skip decisions: UP={} (delta>={:.1} && UP>={:.0}), DOWN={} (delta<=-{:.1} && DOWN>={:.0})",
            skip_up, config.max_imbalance, config.order_size,
            skip_down, config.max_imbalance, config.order_size
        );
    }

    // Calculate profitability caps for weighted bid approach
    // Only apply cap when ALL conditions are met:
    // 1. We have position on OTHER side (to maintain combined avg < 1.0)
    // 2. prof_weight > 0 (feature is enabled)
    // 3. |delta| <= prof_cap_delta_threshold (we're relatively balanced)
    //
    // When imbalanced (|delta| > threshold), SKIP the cap to allow aggressive rebalancing.
    // The offset/skew mechanisms handle imbalance - the cap should not interfere.
    let delta_abs = delta.abs();
    let apply_prof_cap = delta_abs <= config.prof_cap_delta_threshold;

    if !apply_prof_cap && config.prof_weight > 0.0 {
        debug!(
            "[Solver] Profitability cap SKIPPED: |delta|={:.2} > threshold={:.2}, allowing aggressive rebalancing",
            delta_abs, config.prof_cap_delta_threshold
        );
    }

    let profitability_cap_up = if inventory.down_avg_price > 0.0 && config.prof_weight > 0.0 && apply_prof_cap {
        let prof_bid = 1.0 - inventory.down_avg_price;
        up_ob.best_bid_price().map(|imbalance_bid| {
            calculate_generated_bid(prof_bid, imbalance_bid, config.prof_weight, config.imbalance_weight)
        })
    } else {
        None // No DOWN position, cap disabled, or imbalanced (rebalancing mode)
    };

    let profitability_cap_down = if inventory.up_avg_price > 0.0 && config.prof_weight > 0.0 && apply_prof_cap {
        let prof_bid = 1.0 - inventory.up_avg_price;
        down_ob.best_bid_price().map(|imbalance_bid| {
            calculate_generated_bid(prof_bid, imbalance_bid, config.prof_weight, config.imbalance_weight)
        })
    } else {
        None // No UP position, cap disabled, or imbalanced (rebalancing mode)
    };

    // Log when profitability caps are active
    if profitability_cap_up.is_some() || profitability_cap_down.is_some() {
        info!(
            "[Solver] Profitability caps: UP={:?}, DOWN={:?} (avg_costs: UP={:.3}, DOWN={:.3})",
            profitability_cap_up, profitability_cap_down,
            inventory.up_avg_price, inventory.down_avg_price
        );
    }

    // Build Up quotes
    // Cross-spread validation DISABLED - was too restrictive and blocked rebalancing
    if !skip_up {
        if let Some(best_ask) = up_ob.best_ask_price() {
            // LOW PRICE FIX: Cap offset to ensure at least one valid price level ($0.01)
            // When best_ask is very low (e.g., $0.05), normal offset could push price below minimum
            let max_safe_offset = (best_ask - 0.01).max(0.001);  // Must leave room for $0.01 min price
            let effective_up_offset = up_offset.min(max_safe_offset);
            if effective_up_offset < up_offset {
                debug!(
                    "[Solver] UP offset capped for low price: {:.3} → {:.3} (best_ask={:.3})",
                    up_offset, effective_up_offset, best_ask
                );
            }

            ladder.up_quotes = build_ladder(
                up_token_id,
                best_ask,
                effective_up_offset,
                up_size,
                config,
                None,  // No cross-spread validation
                profitability_cap_up,
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
            // LOW PRICE FIX: Cap offset to ensure at least one valid price level ($0.01)
            // When best_ask is very low (e.g., $0.03), normal offset could push price below minimum
            let max_safe_offset = (best_ask - 0.01).max(0.001);  // Must leave room for $0.01 min price
            let effective_down_offset = down_offset.min(max_safe_offset);
            if effective_down_offset < down_offset {
                debug!(
                    "[Solver] DOWN offset capped for low price: {:.3} → {:.3} (best_ask={:.3})",
                    down_offset, effective_down_offset, best_ask
                );
            }

            ladder.down_quotes = build_ladder(
                down_token_id,
                best_ask,
                effective_down_offset,
                down_size,
                config,
                None,  // No cross-spread validation
                profitability_cap_down,
            );

            // Log if DOWN quotes are empty (helps debug why no orders are placed)
            if ladder.down_quotes.is_empty() {
                tracing::warn!(
                    "[Solver] DOWN ladder EMPTY! best_ask={:.3}, offset={:.3} (effective={:.3}), delta={:.2}",
                    best_ask, down_offset, effective_down_offset, delta
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
///
/// Profitability cap limits bid prices to maintain combined avg cost < 1.0.
fn build_ladder(
    token_id: &str,
    best_ask: f64,
    base_offset: f64,
    order_size: f64,
    config: &SolverConfig,
    opposite_best_bid: Option<f64>,  // For cross-spread validation
    profitability_cap: Option<f64>,  // For maintaining avg_up + avg_down < 1.0
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

        // Apply profitability cap to maintain combined avg cost < 1.0
        if let Some(cap) = profitability_cap {
            if price > cap {
                debug!(
                    "[Solver] Profitability cap: {:.3} → {:.3}",
                    price, cap
                );
                price = cap;
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

/// Round price down to tick size, ensuring exactly 2 decimal places
pub(crate) fn round_to_tick(price: f64, tick_size: f64) -> f64 {
    // Add small epsilon to handle floating point precision errors
    // e.g., 0.47/0.01 = 46.9999... should floor to 47, not 46
    let ticks = ((price / tick_size) + 1e-9).floor();
    // CRITICAL: Round to 2 decimal places to avoid floating-point representation issues
    // e.g., 52 * 0.01 = 0.5200000000000001 in floating point
    // Polymarket API requires exactly 2 decimal places
    (ticks * tick_size * 100.0).round() / 100.0
}

/// Round size to whole number (Polymarket requires integer sizes)
fn round_size(size: f64) -> f64 {
    size.round()
}

/// Calculate weighted bid for profitability cap.
///
/// Combines profitability constraint with market competitiveness:
/// - prof_bid: Maximum price to keep combined avg cost < 1.0 (1.0 - other_side_avg)
/// - imbalance_bid: Market best_bid (competitive pricing)
///
/// The weighted average balances staying profitable (prof_weight) with
/// being competitive in the market (imbalance_weight).
pub(crate) fn calculate_generated_bid(
    prof_bid: f64,
    imbalance_bid: f64,
    prof_weight: f64,
    imbalance_weight: f64,
) -> f64 {
    let total_weight = prof_weight + imbalance_weight;
    if total_weight == 0.0 {
        return imbalance_bid;
    }
    let weighted = (prof_bid * prof_weight + imbalance_bid * imbalance_weight) / total_weight;
    round_to_tick(weighted, 0.01)
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
            prof_weight: 0.3,
            imbalance_weight: 0.7,
            prof_cap_delta_threshold: 0.3,  // Only apply cap when |delta| <= 0.3
        }
    }

    #[test]
    fn test_round_to_tick() {
        assert_eq!(round_to_tick(0.456, 0.01), 0.45);
        assert_eq!(round_to_tick(0.459, 0.01), 0.45);
        assert_eq!(round_to_tick(0.45, 0.01), 0.45);
        assert_eq!(round_to_tick(0.999, 0.01), 0.99);
        // Verify exact 2 decimal places (no floating-point precision issues)
        let price = round_to_tick(0.52, 0.01);
        assert_eq!(format!("{:.2}", price), "0.52");
        let price = round_to_tick(0.47, 0.01);
        assert_eq!(format!("{:.2}", price), "0.47");
    }

    #[test]
    fn test_round_size() {
        assert_eq!(round_size(41.0), 41.0);
        assert_eq!(round_size(41.4), 41.0);
        assert_eq!(round_size(41.5), 42.0);
        assert_eq!(round_size(41.9), 42.0);
        assert_eq!(round_size(5.1), 5.0);
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
        // No cross-spread validation (None for opposite_best_bid), no profitability cap
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, &config, None, None);

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
        let quotes = build_ladder("token", 0.65, 0.01, 100.0, &config, Some(0.37), None);

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

    #[test]
    fn test_calculate_generated_bid() {
        // Test the weighted bid calculation
        // prof_bid = 0.51, imbalance_bid = 0.55
        // weights: 0.3 and 0.7 (from notebook example)
        // expected = (0.51 * 0.3 + 0.55 * 0.7) / 1.0 = 0.153 + 0.385 = 0.538
        // round_to_tick floors to 0.53 (conservative for bids)
        let result = calculate_generated_bid(0.51, 0.55, 0.3, 0.7);
        assert!((result - 0.53).abs() < 0.001, "Expected 0.53, got {}", result);

        // Test with equal weights
        let result = calculate_generated_bid(0.50, 0.60, 0.5, 0.5);
        assert!((result - 0.55).abs() < 0.001, "Expected 0.55, got {}", result);

        // Test with zero total weight (edge case)
        let result = calculate_generated_bid(0.50, 0.60, 0.0, 0.0);
        assert!((result - 0.60).abs() < 0.001, "Expected imbalance_bid 0.60, got {}", result);

        // Test exact result (no rounding needed)
        // (0.52 * 0.3 + 0.54 * 0.7) = 0.156 + 0.378 = 0.534 → 0.53
        let result = calculate_generated_bid(0.52, 0.54, 0.3, 0.7);
        assert!((result - 0.53).abs() < 0.001, "Expected 0.53, got {}", result);
    }

    #[test]
    fn test_build_ladder_with_profitability_cap() {
        let config = default_config();
        // best_ask = 0.55, offset = 0.01
        // Normal price would be 0.54, 0.53, 0.52
        // With profitability cap at 0.52, all should be capped
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, &config, None, Some(0.52));

        assert_eq!(quotes.len(), 3);
        // All prices should be <= 0.52 (the profitability cap)
        for q in &quotes {
            assert!(q.price <= 0.52, "Price {} exceeds cap 0.52", q.price);
        }
        // First quote should be exactly at cap
        assert!((quotes[0].price - 0.52).abs() < 0.001, "First quote should be at cap");
    }

    #[test]
    fn test_build_ladder_cap_below_offset_price() {
        let config = default_config();
        // best_ask = 0.55, offset = 0.01
        // Normal price would be 0.54
        // With very low cap at 0.45, all should be at 0.45
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, &config, None, Some(0.45));

        assert!(!quotes.is_empty());
        // All prices should be 0.45 (same price due to low cap)
        // Note: ladder logic ensures monotonically decreasing, so may have fewer quotes
        for q in &quotes {
            assert!(q.price <= 0.45, "Price {} exceeds cap 0.45", q.price);
        }
    }

    #[test]
    fn test_profitability_cap_with_position() {
        // Test that profitability cap is applied when we have position on OTHER side
        let config = default_config();

        // Inventory with DOWN position (avg 0.48), no UP position
        // profBid_UP = 1.0 - 0.48 = 0.52
        let inventory = InventorySnapshot {
            up_size: 0.0,
            up_avg_price: 0.0,
            down_size: 100.0,
            down_avg_price: 0.48,
        };

        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.60, 100.0)),
            best_bid: Some((0.55, 50.0)),  // imbalanceBid = 0.55
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.40, 100.0)),
            best_bid: Some((0.38, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };

        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

        // UP quotes should have profitability cap applied
        // generated_bid = (0.52 * 0.3 + 0.55 * 0.7) / 1.0 = 0.541 → floors to 0.54
        assert!(!ladder.up_quotes.is_empty());
        for q in &ladder.up_quotes {
            assert!(q.price <= 0.54, "UP price {} exceeds profitability cap 0.54", q.price);
        }

        // DOWN quotes should NOT have cap (no UP position)
        assert!(!ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_no_profitability_cap_without_position() {
        // Test that profitability cap is NOT applied when no position on other side
        let config = default_config();

        // No inventory at all - market making from scratch
        let inventory = InventorySnapshot {
            up_size: 0.0,
            up_avg_price: 0.0,
            down_size: 0.0,
            down_avg_price: 0.0,
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

        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

        // Both sides should quote without cap (normal market making)
        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());

        // UP quote should be at normal offset price (0.55 - 0.01 = 0.54)
        assert!((ladder.up_quotes[0].price - 0.54).abs() < 0.001,
            "UP price {} should be 0.54 without cap", ladder.up_quotes[0].price);

        // DOWN quote should be at normal offset price (0.45 - 0.01 = 0.44)
        assert!((ladder.down_quotes[0].price - 0.44).abs() < 0.001,
            "DOWN price {} should be 0.44 without cap", ladder.down_quotes[0].price);
    }
}
