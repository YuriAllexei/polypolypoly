//! Quote ladder calculation.

use tracing::debug;

use crate::application::strategies::inventory_mm::types::{
    OrderbookSnapshot, Quote, QuoteLadder, SolverConfig,
};

/// Polymarket minimum order size (in shares)
const MIN_ORDER_SIZE: f64 = 5.0;

/// Calculate quote ladder for both Up and Down tokens.
///
/// Quotes are purely market-based: price = best_ask - offset - level_spread
/// Risk is managed via offset (price) and skew (size) mechanisms.
pub fn calculate_quotes(
    delta: f64,
    up_ob: &OrderbookSnapshot,
    down_ob: &OrderbookSnapshot,
    config: &SolverConfig,
    up_token_id: &str,
    down_token_id: &str,
) -> QuoteLadder {
    let mut ladder = QuoteLadder::new();

    // Calculate offsets based on imbalance (price adjustment)
    // When heavy on UP (delta > 0), UP offset increases → less aggressive UP bids
    // When heavy on DOWN (delta < 0), DOWN offset increases → less aggressive DOWN bids
    let up_offset = config.base_offset * (1.0 + delta.max(0.0) * config.offset_scaling);
    let down_offset = config.base_offset * (1.0 + (-delta).max(0.0) * config.offset_scaling);

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

    // Build Up quotes (skip only if TOO imbalanced on Up side - use <= to include boundary)
    if delta <= config.max_imbalance {
        if let Some(best_ask) = up_ob.best_ask_price() {
            ladder.up_quotes = build_ladder(
                up_token_id,
                best_ask,
                up_offset,
                up_size,
                config,
            );
        }
    }

    // Build Down quotes (skip only if TOO imbalanced on Down side - use >= to include boundary)
    if delta >= -config.max_imbalance {
        if let Some(best_ask) = down_ob.best_ask_price() {
            ladder.down_quotes = build_ladder(
                down_token_id,
                best_ask,
                down_offset,
                down_size,
                config,
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
    config: &SolverConfig,
) -> Vec<Quote> {
    let mut quotes = Vec::with_capacity(config.num_levels);
    let mut last_price: Option<f64> = None;

    for level in 0..config.num_levels {
        // Calculate spread for this level (widens with each level)
        let level_spread = (level as f64) * (config.spread_per_level / 100.0);

        // Price = best_ask - base_offset - level_spread
        let price = best_ask - base_offset - level_spread;

        // Round to tick size
        let price = round_to_tick(price, config.tick_size);

        // Skip if price too low (not worth quoting)
        if price < 0.01 {
            continue;
        }

        // Skip if same as previous level (shouldn't happen without capping, but keep for safety)
        if last_price.map_or(false, |lp| (lp - price).abs() < 1e-9) {
            continue;
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
        }
    }

    #[test]
    fn test_round_to_tick() {
        assert_eq!(round_to_tick(0.456, 0.01), 0.45);
        assert_eq!(round_to_tick(0.459, 0.01), 0.45);
        assert_eq!(round_to_tick(0.45, 0.01), 0.45);
        assert_eq!(round_to_tick(0.999, 0.01), 0.99);
    }

    #[test]
    fn test_build_ladder_basic() {
        let config = default_config();
        let quotes = build_ladder("token", 0.55, 0.01, 100.0, &config);

        assert_eq!(quotes.len(), 3);
        // Level 0: 0.55 - 0.01 - 0 = 0.54
        assert!((quotes[0].price - 0.54).abs() < 0.001);
        // Level 1: 0.55 - 0.01 - 0.01 = 0.53
        assert!((quotes[1].price - 0.53).abs() < 0.001);
        // Level 2: 0.55 - 0.01 - 0.02 = 0.52
        assert!((quotes[2].price - 0.52).abs() < 0.001);
        // All quotes should have the passed size
        assert!((quotes[0].size - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_calculate_quotes_balanced() {
        let config = default_config();
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
        let ladder = calculate_quotes(0.4, &up_ob, &down_ob, &config, "up", "down");

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
        let ladder = calculate_quotes(-0.4, &up_ob, &down_ob, &config, "up", "down");

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
        let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &config, "up", "down");

        assert!((ladder.up_quotes[0].size - 100.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_skew_sizing_clamped() {
        let mut config = default_config();
        config.skew_factor = 5.0; // Very aggressive
        config.order_size = 100.0;

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
        let ladder = calculate_quotes(0.5, &up_ob, &down_ob, &config, "up", "down");

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
        let ladder = calculate_quotes(0.6, &up_ob, &down_ob, &config, "up", "down");

        assert!((ladder.up_quotes[0].size - 100.0).abs() < 0.01);
        assert!((ladder.down_quotes[0].size - 100.0).abs() < 0.01);
    }
}
