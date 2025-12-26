//! Taker opportunity detection.

use crate::application::strategies::inventory_mm::types::{
    InventorySnapshot, OrderbookSnapshot, TakerOrder, SolverConfig,
};

/// Find taker opportunity for rebalancing
///
/// Returns a taker order if:
/// 1. There's liquidity at BBO that isn't ours
/// 2. Taking improves our delta (moves toward balance)
/// 3. Taking maintains profitability (combined avg cost < threshold)
pub fn find_taker_opportunity(
    delta: f64,
    up_ob: &OrderbookSnapshot,
    down_ob: &OrderbookSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    up_token_id: &str,
    down_token_id: &str,
) -> Option<TakerOrder> {
    // If balanced, no need to take
    if delta.abs() < 0.1 {
        return None;
    }

    if delta > 0.0 {
        // Heavy on Up -> need Down
        find_down_taker(down_ob, inventory, config, down_token_id)
    } else {
        // Heavy on Down -> need Up
        find_up_taker(up_ob, inventory, config, up_token_id)
    }
}

/// Find opportunity to take Down liquidity (when we need Down)
fn find_down_taker(
    down_ob: &OrderbookSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    token_id: &str,
) -> Option<TakerOrder> {
    // Skip if best ask is ours
    if down_ob.best_ask_is_ours {
        return None;
    }

    // Need existing Up position to calculate profitability
    if inventory.up_size <= 0.0 || inventory.up_avg_price <= 0.0 {
        return None;
    }

    let (ask_price, ask_size) = down_ob.best_ask?;

    // Use the actual size we'll order (capped at config.order_size)
    let take_size = ask_size.min(config.order_size);

    // Check if taking maintains profitability
    let new_down_avg = if inventory.down_size > 0.0 {
        // VWAP: (old_cost + new_cost) / (old_size + new_size)
        let old_cost = inventory.down_size * inventory.down_avg_price;
        let new_cost = take_size * ask_price;
        (old_cost + new_cost) / (inventory.down_size + take_size)
    } else {
        ask_price
    };

    let combined_cost = inventory.up_avg_price + new_down_avg;

    // Must stay profitable after taking
    if combined_cost > 1.0 - config.min_profit_margin {
        return None;
    }

    // Calculate score based on how good this opportunity is
    let profit_margin = 1.0 - combined_cost;
    let score = profit_margin * 100.0; // Higher margin = higher score

    Some(TakerOrder::buy(
        token_id.to_string(),
        ask_price,
        take_size,
        score,
    ))
}

/// Find opportunity to take Up liquidity (when we need Up)
fn find_up_taker(
    up_ob: &OrderbookSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    token_id: &str,
) -> Option<TakerOrder> {
    // Skip if best ask is ours
    if up_ob.best_ask_is_ours {
        return None;
    }

    // Need existing Down position to calculate profitability
    if inventory.down_size <= 0.0 || inventory.down_avg_price <= 0.0 {
        return None;
    }

    let (ask_price, ask_size) = up_ob.best_ask?;

    // Use the actual size we'll order (capped at config.order_size)
    let take_size = ask_size.min(config.order_size);

    // Check if taking maintains profitability
    let new_up_avg = if inventory.up_size > 0.0 {
        let old_cost = inventory.up_size * inventory.up_avg_price;
        let new_cost = take_size * ask_price;
        (old_cost + new_cost) / (inventory.up_size + take_size)
    } else {
        ask_price
    };

    let combined_cost = new_up_avg + inventory.down_avg_price;

    // Must stay profitable after taking
    if combined_cost > 1.0 - config.min_profit_margin {
        return None;
    }

    let profit_margin = 1.0 - combined_cost;
    let score = profit_margin * 100.0;

    Some(TakerOrder::buy(
        token_id.to_string(),
        ask_price,
        take_size,
        score,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> SolverConfig {
        SolverConfig {
            num_levels: 3,
            tick_size: 0.01,
            base_offset: 0.01,
            min_profit_margin: 0.01, // 1 cent
            max_imbalance: 0.8,
            order_size: 100.0,
            spread_per_level: 1.0,
            offset_scaling: 5.0,
        }
    }

    #[test]
    fn test_no_taker_when_balanced() {
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
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };
        let config = default_config();

        let result = find_taker_opportunity(
            0.0, // balanced
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up",
            "down",
        );

        assert!(result.is_none());
    }

    #[test]
    fn test_taker_when_need_down() {
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.44, 50.0)), // Good price for Down
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.46,
        };
        let config = default_config();

        // delta = (80-20)/100 = 0.6 (need Down)
        let delta = inventory.imbalance();

        let result = find_taker_opportunity(
            delta,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up",
            "down",
        );

        assert!(result.is_some());
        let taker = result.unwrap();
        assert_eq!(taker.token_id, "down");
        assert!((taker.price - 0.44).abs() < 0.001);
    }

    #[test]
    fn test_no_taker_when_ask_is_ours() {
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.44, 50.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: true, // Our order!
        };
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.46,
        };
        let config = default_config();
        let delta = inventory.imbalance();

        let result = find_taker_opportunity(
            delta,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up",
            "down",
        );

        assert!(result.is_none());
    }

    #[test]
    fn test_no_taker_when_unprofitable() {
        let up_ob = OrderbookSnapshot {
            best_ask: Some((0.55, 100.0)),
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let down_ob = OrderbookSnapshot {
            best_ask: Some((0.50, 50.0)), // Too expensive
            best_bid: None,
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        };
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52, // Combined would be 0.52 + 0.50 = 1.02 (loss!)
            down_size: 20.0,
            down_avg_price: 0.46,
        };
        let config = default_config();
        let delta = inventory.imbalance();

        let result = find_taker_opportunity(
            delta,
            &up_ob,
            &down_ob,
            &inventory,
            &config,
            "up",
            "down",
        );

        // Taking at 0.50 would make new down avg close to 0.50
        // Combined = 0.52 + ~0.48 = 1.00 (barely profitable, but check exact math)
        // With VWAP: new_down = (20*0.46 + 50*0.50) / 70 = (9.2 + 25) / 70 = 0.489
        // Combined = 0.52 + 0.489 = 1.009 > 0.99 (unprofitable)
        assert!(result.is_none());
    }
}
