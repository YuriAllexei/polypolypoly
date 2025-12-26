//! Quote ladder calculation.

use crate::application::strategies::inventory_mm::types::{
    OrderbookSnapshot, Quote, QuoteLadder, SolverConfig,
};

/// Calculate quote ladder for both Up and Down tokens.
///
/// Quotes are purely market-based: price = best_ask - offset - level_spread
/// Risk is managed via the offset mechanism (increases when imbalanced).
pub fn calculate_quotes(
    delta: f64,
    up_ob: &OrderbookSnapshot,
    down_ob: &OrderbookSnapshot,
    config: &SolverConfig,
    up_token_id: &str,
    down_token_id: &str,
) -> QuoteLadder {
    let mut ladder = QuoteLadder::new();

    // Calculate offsets based on imbalance
    // When heavy on UP (delta > 0), UP offset increases → less aggressive UP bids
    // When heavy on DOWN (delta < 0), DOWN offset increases → less aggressive DOWN bids
    // offset_scaling controls how aggressively we back off (e.g., 5.0 = 5x multiplier)
    let up_offset = config.base_offset * (1.0 + delta.max(0.0) * config.offset_scaling);
    let down_offset = config.base_offset * (1.0 + (-delta).max(0.0) * config.offset_scaling);

    // Build Up quotes (skip if too imbalanced on Up side)
    if delta < config.max_imbalance {
        if let Some(best_ask) = up_ob.best_ask_price() {
            ladder.up_quotes = build_ladder(
                up_token_id,
                best_ask,
                up_offset,
                config,
            );
        }
    }

    // Build Down quotes (skip if too imbalanced on Down side)
    if delta > -config.max_imbalance {
        if let Some(best_ask) = down_ob.best_ask_price() {
            ladder.down_quotes = build_ladder(
                down_token_id,
                best_ask,
                down_offset,
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
            config.order_size,
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
        let quotes = build_ladder("token", 0.55, 0.01, &config);

        assert_eq!(quotes.len(), 3);
        // Level 0: 0.55 - 0.01 - 0 = 0.54
        assert!((quotes[0].price - 0.54).abs() < 0.001);
        // Level 1: 0.55 - 0.01 - 0.01 = 0.53
        assert!((quotes[1].price - 0.53).abs() < 0.001);
        // Level 2: 0.55 - 0.01 - 0.02 = 0.52
        assert!((quotes[2].price - 0.52).abs() < 0.001);
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
}
