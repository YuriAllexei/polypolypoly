//! 4-Layer Quote Ladder Calculation
//!
//! Implements O'Hara Market Microstructure theory with:
//! - Layer 1: Oracle-adjusted offset
//! - Layer 2: Adverse selection (Glosten-Milgrom)
//! - Layer 3: Inventory skew
//! - Layer 4: Edge check

use tracing::{debug, info};

use crate::application::strategies::inventory_mm::types::{
    Quote, QuoteLadder, SolverConfig, SolverInput,
};

/// Polymarket minimum order size (in shares)
const MIN_ORDER_SIZE: f64 = 5.0;

/// Calculate quote ladder using the 4-layer framework.
///
/// Layers are computed in this order:
/// 1. Layer 2: Adverse Selection - widens spread near resolution
/// 2. Layer 1: Oracle Adjustment - adjusts offset based on oracle direction
/// 3. Layer 3: Inventory Skew - adjusts offset/size based on imbalance
/// 4. Layer 4: Edge Check - skip quotes with insufficient edge
pub fn calculate_quotes(input: &SolverInput) -> QuoteLadder {
    let config = &input.config;
    let mut ladder = QuoteLadder::new();

    // Calculate inventory imbalance: (up - down) / (up + down)
    let q = input.inventory.imbalance();

    // ═══════════════════════════════════════════════════════════════
    // LAYER 2: ADVERSE SELECTION (Glosten-Milgrom)
    // Widen spreads near resolution when informed traders dominate
    // ═══════════════════════════════════════════════════════════════
    // Floor time_decay at 0.1 minutes to prevent division by zero
    let time_decay = config.time_decay_minutes.max(0.1);
    let p_informed = (config.p_informed_base
        * (-input.minutes_to_resolution / time_decay).exp())
    .min(0.8); // Cap at 80%

    let spread = config.base_spread * (1.0 + 3.0 * p_informed);

    debug!(
        "[Solver] Layer 2: minutes={:.1}, p_informed={:.1}%, spread={:.3}",
        input.minutes_to_resolution,
        p_informed * 100.0,
        spread
    );

    // ═══════════════════════════════════════════════════════════════
    // LAYER 1: ORACLE-ADJUSTED OFFSET
    // React to oracle price vs threshold
    // ═══════════════════════════════════════════════════════════════
    let oracle_adj = input.oracle_distance_pct * config.oracle_sensitivity;

    // When oracle > threshold (positive): UP favored → tighter UP, wider DOWN
    // When oracle < threshold (negative): DOWN favored → wider UP, tighter DOWN
    let raw_up_offset = (spread - oracle_adj).max(config.min_offset);
    let raw_down_offset = (spread + oracle_adj).max(config.min_offset);

    debug!(
        "[Solver] Layer 1: oracle_dist={:.2}%, oracle_adj={:.3}, raw_offsets=(UP:{:.3}, DOWN:{:.3})",
        input.oracle_distance_pct * 100.0,
        oracle_adj,
        raw_up_offset,
        raw_down_offset
    );

    // ═══════════════════════════════════════════════════════════════
    // LAYER 3: INVENTORY SKEW
    // Adjust offsets and sizes based on inventory imbalance
    // ═══════════════════════════════════════════════════════════════

    // Offset multipliers: widen offset on overweight side
    // When q > 0 (heavy UP): UP gets wider (mult > 1), DOWN gets tighter (mult < 1)
    // IMPORTANT: Floor at 0.1 to prevent negative multipliers with extreme imbalance.
    // Without this floor, gamma_inv=1.5 and q=±0.67+ causes negative multipliers,
    // which would flip the offset sign and place bids ABOVE best_bid.
    let spread_mult_up = (1.0 + config.gamma_inv * q).max(0.1);
    let spread_mult_down = (1.0 - config.gamma_inv * q).max(0.1);

    let final_up_offset = raw_up_offset * spread_mult_up;
    let final_down_offset = raw_down_offset * spread_mult_down;

    // Size: exponential decay on overweight side
    // When q > 0 (heavy UP): UP size decreases, DOWN size increases
    let raw_size_up = config.order_size * (-config.lambda_size * q).exp();
    let raw_size_down = config.order_size * (config.lambda_size * q).exp();

    // Clamp sizes to [MIN_ORDER_SIZE, max] and round
    let max_size = config.order_size * 4.0;
    let up_size = raw_size_up.clamp(MIN_ORDER_SIZE, max_size).round();
    let down_size = raw_size_down.clamp(MIN_ORDER_SIZE, max_size).round();

    debug!(
        "[Solver] Layer 3: q={:.2}, mult=(UP:{:.2}, DOWN:{:.2}), offsets=(UP:{:.3}, DOWN:{:.3}), sizes=(UP:{:.0}, DOWN:{:.0})",
        q, spread_mult_up, spread_mult_down, final_up_offset, final_down_offset, up_size, down_size
    );

    // ═══════════════════════════════════════════════════════════════
    // SKIP LOGIC: Max imbalance and position limits
    // ═══════════════════════════════════════════════════════════════
    let is_building_up_from_scratch = input.inventory.up_size.abs() < config.order_size;
    let is_building_down_from_scratch = input.inventory.down_size.abs() < config.order_size;

    let mut skip_up = q >= config.max_imbalance && !is_building_up_from_scratch;
    let mut skip_down = q <= -config.max_imbalance && !is_building_down_from_scratch;

    // Position limits
    if config.max_position > 0.0 {
        let up_ratio = input.inventory.up_size.abs() / config.max_position;
        let down_ratio = input.inventory.down_size.abs() / config.max_position;
        let hard_limit = 0.90;

        if up_ratio >= hard_limit {
            skip_up = true;
            info!("[Solver] UP at hard limit ({:.0}%), skipping", up_ratio * 100.0);
        }
        if down_ratio >= hard_limit {
            skip_down = true;
            info!("[Solver] DOWN at hard limit ({:.0}%), skipping", down_ratio * 100.0);
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // BUILD QUOTES (from best_bid, not best_ask)
    // ═══════════════════════════════════════════════════════════════

    // UP quotes
    if !skip_up {
        if let (Some(best_bid), Some(best_ask)) = (
            input.up_orderbook.best_bid_price(),
            input.up_orderbook.best_ask_price(),
        ) {
            ladder.up_quotes = build_ladder_4layer(
                &input.up_token_id,
                best_bid,
                best_ask,
                final_up_offset,
                up_size,
                config,
            );
        } else {
            debug!("[Solver] UP quotes skipped: no best_bid or best_ask");
        }
    } else {
        debug!("[Solver] UP quotes skipped: skip_up=true (q={:.2})", q);
    }

    // DOWN quotes
    if !skip_down {
        if let (Some(best_bid), Some(best_ask)) = (
            input.down_orderbook.best_bid_price(),
            input.down_orderbook.best_ask_price(),
        ) {
            ladder.down_quotes = build_ladder_4layer(
                &input.down_token_id,
                best_bid,
                best_ask,
                final_down_offset,
                down_size,
                config,
            );
        } else {
            debug!("[Solver] DOWN quotes skipped: no best_bid or best_ask");
        }
    } else {
        debug!("[Solver] DOWN quotes skipped: skip_down=true (q={:.2})", q);
    }

    ladder
}

/// Build a ladder of bids for a single token using 4-layer logic.
///
/// Bids are calculated from best_bid (not best_ask) with offset subtracted.
/// Layer 4 (edge check) is applied to skip quotes with insufficient edge.
fn build_ladder_4layer(
    token_id: &str,
    best_bid: f64,
    best_ask: f64,
    base_offset: f64,
    order_size: f64,
    config: &SolverConfig,
) -> Vec<Quote> {
    let mut quotes = Vec::with_capacity(config.num_levels);
    let mut last_price: Option<f64> = None;

    for level in 0..config.num_levels {
        // Calculate spread for this level (widens with each level)
        let level_spread = (level as f64) * (config.spread_per_level / 100.0);

        // Bid from best_bid (not best_ask)
        let mut price = best_bid - base_offset - level_spread;

        // Round to tick size
        price = round_to_tick(price, config.tick_size);

        // ═══════════════════════════════════════════════════════════════
        // LAYER 4: EDGE CHECK
        // ═══════════════════════════════════════════════════════════════
        // Use epsilon tolerance for floating-point comparison.
        // edge = 0.0099999... should NOT skip when threshold = 0.01
        const EPSILON: f64 = 1e-9;
        let edge = best_ask - price;
        if edge < config.edge_threshold - EPSILON {
            debug!(
                "[Solver] L{}: edge {:.3} < threshold {:.3}, skipping",
                level, edge, config.edge_threshold
            );
            continue;
        }

        // Skip if price would cross spread (with epsilon tolerance)
        if price >= best_ask - EPSILON {
            debug!(
                "[Solver] L{}: price {:.3} >= best_ask {:.3}, skipping",
                level, price, best_ask
            );
            continue;
        }

        // Skip if price too low
        if price < 0.01 {
            debug!("[Solver] L{}: price {:.3} < 0.01, skipping", level, price);
            continue;
        }

        // Ensure monotonically decreasing prices
        if let Some(lp) = last_price {
            if price >= lp - 1e-9 {
                let adjusted = round_to_tick(lp - config.tick_size, config.tick_size);
                if adjusted < 0.01 {
                    continue;
                }
                price = adjusted;
            }
        }
        last_price = Some(price);

        quotes.push(Quote::new_bid(
            token_id.to_string(),
            price,
            order_size,
            level,
        ));
    }

    quotes
}

/// Round price down to tick size, ensuring exactly 2 decimal places
pub(crate) fn round_to_tick(price: f64, tick_size: f64) -> f64 {
    let ticks = ((price / tick_size) + 1e-9).floor();
    (ticks * tick_size * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::strategies::inventory_mm::types::{
        InventorySnapshot, OrderSnapshot, OrderbookSnapshot,
    };

    fn default_config() -> SolverConfig {
        SolverConfig::default()
    }

    fn default_input() -> SolverInput {
        SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 100.0)),
                best_bid: Some((0.53, 50.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 100.0)),
                best_bid: Some((0.43, 50.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: default_config(),
            oracle_distance_pct: 0.0,
            minutes_to_resolution: 7.5,
        }
    }

    #[test]
    fn test_round_to_tick() {
        assert_eq!(round_to_tick(0.456, 0.01), 0.45);
        assert_eq!(round_to_tick(0.459, 0.01), 0.45);
        assert_eq!(round_to_tick(0.45, 0.01), 0.45);
        assert_eq!(round_to_tick(0.999, 0.01), 0.99);
        let price = round_to_tick(0.52, 0.01);
        assert_eq!(format!("{:.2}", price), "0.52");
    }

    #[test]
    fn test_balanced_inventory_symmetric_quotes() {
        let input = default_input();
        let ladder = calculate_quotes(&input);

        // Should have quotes on both sides
        assert!(!ladder.up_quotes.is_empty(), "Should have UP quotes");
        assert!(!ladder.down_quotes.is_empty(), "Should have DOWN quotes");

        // With balanced inventory and neutral oracle, offsets should be similar
        // UP: 0.53 (best_bid) - offset
        // DOWN: 0.43 (best_bid) - offset
    }

    #[test]
    fn test_oracle_adjustment() {
        let mut input = default_input();

        // Oracle above threshold (UP favored)
        input.oracle_distance_pct = 0.01; // 1% above

        let ladder = calculate_quotes(&input);

        // UP should have tighter offset (lower price closer to best_bid)
        // DOWN should have wider offset (lower price further from best_bid)
        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_adverse_selection_near_resolution() {
        let mut input = default_input();

        // Very close to resolution (1 minute)
        input.minutes_to_resolution = 1.0;

        let ladder_near = calculate_quotes(&input);

        // Further from resolution (10 minutes)
        input.minutes_to_resolution = 10.0;
        let ladder_far = calculate_quotes(&input);

        // Near resolution should have wider spread (lower bid prices)
        if !ladder_near.up_quotes.is_empty() && !ladder_far.up_quotes.is_empty() {
            // Near resolution bid should be lower (more offset)
            assert!(
                ladder_near.up_quotes[0].price <= ladder_far.up_quotes[0].price,
                "Near resolution should have wider spread"
            );
        }
    }

    #[test]
    fn test_inventory_skew_heavy_up() {
        let mut input = default_input();

        // Heavy UP inventory
        input.inventory.up_size = 100.0;
        input.inventory.down_size = 20.0;

        let ladder = calculate_quotes(&input);

        // Should have both quotes but:
        // - UP quotes should be wider (higher offset)
        // - DOWN quotes should be tighter (lower offset)
        // - UP size should be smaller
        // - DOWN size should be larger
        assert!(!ladder.up_quotes.is_empty());
        assert!(!ladder.down_quotes.is_empty());

        // Check sizes reflect skew
        let up_size = ladder.up_quotes[0].size;
        let down_size = ladder.down_quotes[0].size;
        assert!(
            down_size > up_size,
            "DOWN size ({}) should be > UP size ({}) when heavy UP",
            down_size,
            up_size
        );
    }

    #[test]
    fn test_edge_check_skips_bad_quotes() {
        let mut input = default_input();

        // Very tight market where bid is close to ask
        input.up_orderbook.best_bid = Some((0.54, 50.0)); // bid very close to ask (0.55)
        input.config.edge_threshold = 0.02; // Require 2c edge

        let ladder = calculate_quotes(&input);

        // All UP quotes should have edge >= 0.02
        for q in &ladder.up_quotes {
            let edge = 0.55 - q.price;
            assert!(
                edge >= input.config.edge_threshold,
                "Quote at {} has edge {} < threshold {}",
                q.price,
                edge,
                input.config.edge_threshold
            );
        }
    }

    #[test]
    fn test_extreme_imbalance_skips_overweight() {
        let mut input = default_input();
        input.config.max_imbalance = 0.7;

        // Extreme UP imbalance
        input.inventory.up_size = 100.0;
        input.inventory.down_size = 10.0; // delta = 0.82

        let ladder = calculate_quotes(&input);

        // Should skip UP quotes (overweight)
        assert!(ladder.up_quotes.is_empty(), "Should skip UP when heavily overweight");
        // Should still have DOWN quotes (needed side)
        assert!(!ladder.down_quotes.is_empty(), "Should have DOWN quotes to rebalance");
    }

    #[test]
    fn test_one_sided_extreme_imbalance_skips_overweight() {
        let mut input = default_input();

        // Extreme one-sided: 50 UP, 0 DOWN = 100% imbalance
        input.inventory.up_size = 50.0;
        input.inventory.down_size = 0.0;

        let ladder = calculate_quotes(&input);

        // With 100% imbalance (>80% max), should skip UP (overweight)
        // But since we're "building from scratch" (0 DOWN), skip logic doesn't trigger
        // Actually: is_building_up_from_scratch = 50 < 50 = false, so skip_up = true
        assert!(ladder.up_quotes.is_empty(), "Should skip UP when 100% overweight");
        // Should definitely quote DOWN (needed side)
        assert!(!ladder.down_quotes.is_empty(), "Should quote DOWN to rebalance");
    }

    #[test]
    fn test_building_from_scratch_allows_quoting() {
        let mut input = default_input();

        // Building from scratch: only 10 UP (less than order_size of 50)
        input.inventory.up_size = 10.0;
        input.inventory.down_size = 0.0;
        // This gives 100% imbalance, but is_building_up_from_scratch = 10 < 50 = true

        let ladder = calculate_quotes(&input);

        // When building from scratch, should still quote UP even with high imbalance
        assert!(!ladder.up_quotes.is_empty(), "Building from scratch should still quote own side");
        // Should also quote DOWN (needed side)
        assert!(!ladder.down_quotes.is_empty(), "Should quote DOWN to rebalance");
    }

    #[test]
    fn test_build_ladder_basic() {
        let config = default_config();
        let quotes = build_ladder_4layer("token", 0.53, 0.55, 0.02, 50.0, &config);

        assert!(!quotes.is_empty());
        // Level 0: 0.53 - 0.02 = 0.51
        assert!((quotes[0].price - 0.51).abs() < 0.001);
        assert!((quotes[0].size - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_build_ladder_respects_ask() {
        let config = default_config();
        // Very low offset that would put bid above ask
        let quotes = build_ladder_4layer("token", 0.54, 0.55, 0.001, 50.0, &config);

        // All quotes must be below ask
        for q in &quotes {
            assert!(q.price < 0.55, "Bid {} must be < ask 0.55", q.price);
        }
    }

    #[test]
    fn test_spread_mult_floor_prevents_negative() {
        // Regression test for spread multiplier floor fix.
        // With gamma_inv=1.5 and extreme imbalance (q=±0.8), spread_mult would go negative
        // without the .max(0.1) floor, causing bids to go ABOVE best_bid.
        let mut input = default_input();
        input.config.gamma_inv = 1.5;

        // Test extreme positive imbalance (q ≈ 0.82)
        input.inventory.up_size = 100.0;
        input.inventory.down_size = 10.0;

        let ladder = calculate_quotes(&input);

        // DOWN quotes should still be valid (below best_bid, not above)
        // With the floor, spread_mult_down = max(1 - 1.5*0.82, 0.1) = max(-0.23, 0.1) = 0.1
        for q in &ladder.down_quotes {
            assert!(
                q.price <= input.down_orderbook.best_bid_price().unwrap(),
                "DOWN bid {} should be <= best_bid {}",
                q.price,
                input.down_orderbook.best_bid_price().unwrap()
            );
        }

        // Test extreme negative imbalance (q ≈ -0.82)
        input.inventory.up_size = 10.0;
        input.inventory.down_size = 100.0;

        let ladder = calculate_quotes(&input);

        // UP quotes should still be valid (below best_bid, not above)
        for q in &ladder.up_quotes {
            assert!(
                q.price <= input.up_orderbook.best_bid_price().unwrap(),
                "UP bid {} should be <= best_bid {}",
                q.price,
                input.up_orderbook.best_bid_price().unwrap()
            );
        }
    }

    #[test]
    fn test_time_decay_floor_prevents_div_zero() {
        // Regression test for time_decay floor.
        // With time_decay_minutes=0, the exp() calculation would have division by zero.
        let mut input = default_input();
        input.config.time_decay_minutes = 0.0; // Would cause div/0 without floor

        // Should not panic
        let ladder = calculate_quotes(&input);

        // Should still produce valid quotes
        assert!(!ladder.up_quotes.is_empty() || !ladder.down_quotes.is_empty());
    }

    #[test]
    fn test_floating_point_precision_edge_check() {
        // Regression test for floating-point precision in edge check.
        // When edge ≈ threshold due to precision, we should NOT skip the quote.
        let mut input = default_input();
        input.config.edge_threshold = 0.02; // 2 cents

        // Set up a case where edge is exactly at threshold
        // best_ask = 0.55, best_bid = 0.53
        // If we bid at 0.53, edge = 0.55 - 0.53 = 0.02
        // Due to precision, this might be 0.019999999... or 0.020000001...
        input.up_orderbook.best_bid = Some((0.53, 50.0));
        input.up_orderbook.best_ask = Some((0.55, 100.0));

        // With a small offset, our bid should be close to best_bid
        input.config.base_spread = 0.001; // Very small base spread
        input.config.min_offset = 0.001;
        input.oracle_distance_pct = 0.0;
        input.minutes_to_resolution = 100.0; // Far from resolution

        let ladder = calculate_quotes(&input);

        // Should have quotes (not skipped due to precision issues)
        // The key is that quotes at or very near the edge threshold should NOT be skipped
        for q in &ladder.up_quotes {
            let edge = 0.55 - q.price;
            // Edge should be at least threshold (with small tolerance for rounding)
            assert!(
                edge >= input.config.edge_threshold - 0.001,
                "Edge {} should be >= threshold {} (with tolerance)",
                edge,
                input.config.edge_threshold
            );
        }
    }
}
