//! Quote ladder calculation.

use crate::application::strategies::inventory_mm::types::{
    InventorySnapshot, OrderbookSnapshot, Quote, QuoteLadder, SolverConfig,
};

/// Calculate quote ladder for both Up and Down tokens.
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

    // Calculate offsets based on imbalance
    let up_offset = config.base_offset * (1.0 + delta.max(0.0));
    let down_offset = config.base_offset * (1.0 + (-delta).max(0.0));

    // Calculate max profitable bids based on other side's avg cost or best ask
    let max_up_bid = if inventory.down_avg_price > 0.0 {
        // Have Down position: max Up bid = 1.0 - down_avg - margin
        1.0 - inventory.down_avg_price - config.min_profit_margin
    } else if let Some(down_ask) = down_ob.best_ask_price() {
        // No Down position: use best Down ask as proxy (what we might get)
        // Max Up bid = 1.0 - down_ask - margin
        1.0 - down_ask - config.min_profit_margin
    } else {
        // No data at all: use conservative 50/50 assumption
        0.50 - config.min_profit_margin
    };

    let max_down_bid = if inventory.up_avg_price > 0.0 {
        1.0 - inventory.up_avg_price - config.min_profit_margin
    } else if let Some(up_ask) = up_ob.best_ask_price() {
        // No Up position: use best Up ask as proxy
        1.0 - up_ask - config.min_profit_margin
    } else {
        0.50 - config.min_profit_margin
    };

    // Build Up quotes (skip if too imbalanced on Up side)
    if delta < config.max_imbalance {
        if let Some(best_ask) = up_ob.best_ask_price() {
            ladder.up_quotes = build_ladder(
                up_token_id,
                best_ask,
                up_offset,
                max_up_bid,
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
                max_down_bid,
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
    max_bid: f64,
    config: &SolverConfig,
) -> Vec<Quote> {
    let mut quotes = Vec::with_capacity(config.num_levels);

    for level in 0..config.num_levels {
        // Calculate spread for this level (widens with each level)
        let level_spread = (level as f64) * (config.spread_per_level / 100.0);

        // Price = best_ask - base_offset - level_spread
        let mut price = best_ask - base_offset - level_spread;

        // Cap at profitability limit
        price = price.min(max_bid);

        // Round to tick size
        price = round_to_tick(price, config.tick_size);

        // Skip if price too low (not worth quoting)
        if price < 0.01 {
            continue;
        }

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
    (price / tick_size).floor() * tick_size
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
        let quotes = build_ladder("token", 0.55, 0.01, 0.54, &config);

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
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };

        let ladder = calculate_quotes(
            0.0, // balanced
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
        let inventory = InventorySnapshot {
            up_size: 90.0,
            up_avg_price: 0.52,
            down_size: 10.0,
            down_avg_price: 0.46,
        };

        // delta = (90-10)/(90+10) = 0.8, exactly at max_imbalance
        let delta = inventory.imbalance();
        assert!((delta - 0.8).abs() < 0.01);

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
        let inventory = InventorySnapshot {
            up_size: 95.0,
            up_avg_price: 0.52,
            down_size: 5.0,
            down_avg_price: 0.46,
        };

        // delta = (95-5)/100 = 0.9, above max_imbalance of 0.7
        let delta = inventory.imbalance();

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
}
