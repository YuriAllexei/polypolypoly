//! Taker opportunity detection.

use crate::application::strategies::inventory_mm::types::{
    InventorySnapshot, OrderbookSnapshot, OrderSnapshot, TakerOrder, SolverConfig,
};

/// Find taker opportunity for rebalancing.
/// Checks mirrored orderbooks to prevent self-trading (UP bid at P = DOWN ask at 1-P).
pub fn find_taker_opportunity(
    delta: f64,
    up_ob: &OrderbookSnapshot,
    down_ob: &OrderbookSnapshot,
    up_orders: &OrderSnapshot,
    down_orders: &OrderSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    up_token_id: &str,
    down_token_id: &str,
) -> Option<TakerOrder> {
    if delta.abs() < 0.1 {
        return None;
    }

    if delta > 0.0 {
        find_down_taker(down_ob, up_orders, inventory, config, down_token_id)
    } else {
        find_up_taker(up_ob, down_orders, inventory, config, up_token_id)
    }
}

fn find_down_taker(
    down_ob: &OrderbookSnapshot,
    up_orders: &OrderSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    token_id: &str,
) -> Option<TakerOrder> {
    if down_ob.best_ask_is_ours {
        return None;
    }

    let (ask_price, ask_size) = down_ob.best_ask?;

    // Self-trade prevention: DOWN ask at P = UP bid at (1-P)
    let mirror_price = 1.0 - ask_price;
    if up_orders.bids.iter().any(|o| (o.price - mirror_price).abs() < 0.0001) {
        return None;
    }

    if inventory.up_size <= 0.0 || inventory.up_avg_price <= 0.0 {
        return None;
    }

    let take_size = ask_size.min(config.order_size);

    let new_down_avg = if inventory.down_size > 0.0 {
        let old_cost = inventory.down_size * inventory.down_avg_price;
        let new_cost = take_size * ask_price;
        (old_cost + new_cost) / (inventory.down_size + take_size)
    } else {
        ask_price
    };

    let combined_cost = inventory.up_avg_price + new_down_avg;
    if combined_cost > 1.0 - config.min_profit_margin {
        return None;
    }

    let profit_margin = 1.0 - combined_cost;
    Some(TakerOrder::buy(token_id.to_string(), ask_price, take_size, profit_margin * 100.0))
}

fn find_up_taker(
    up_ob: &OrderbookSnapshot,
    down_orders: &OrderSnapshot,
    inventory: &InventorySnapshot,
    config: &SolverConfig,
    token_id: &str,
) -> Option<TakerOrder> {
    if up_ob.best_ask_is_ours {
        return None;
    }

    let (ask_price, ask_size) = up_ob.best_ask?;

    // Self-trade prevention: UP ask at P = DOWN bid at (1-P)
    let mirror_price = 1.0 - ask_price;
    if down_orders.bids.iter().any(|o| (o.price - mirror_price).abs() < 0.0001) {
        return None;
    }

    if inventory.down_size <= 0.0 || inventory.down_avg_price <= 0.0 {
        return None;
    }

    let take_size = ask_size.min(config.order_size);

    let new_up_avg = if inventory.up_size > 0.0 {
        let old_cost = inventory.up_size * inventory.up_avg_price;
        let new_cost = take_size * ask_price;
        (old_cost + new_cost) / (inventory.up_size + take_size)
    } else {
        ask_price
    };

    let combined_cost = new_up_avg + inventory.down_avg_price;
    if combined_cost > 1.0 - config.min_profit_margin {
        return None;
    }

    let profit_margin = 1.0 - combined_cost;
    Some(TakerOrder::buy(token_id.to_string(), ask_price, take_size, profit_margin * 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::strategies::inventory_mm::types::OpenOrder;

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
            skew_factor: 1.0,
        }
    }

    fn empty_orders() -> OrderSnapshot {
        OrderSnapshot::default()
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
            &empty_orders(),
            &empty_orders(),
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
            best_ask: Some((0.44, 50.0)),
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

        let result = find_taker_opportunity(
            inventory.imbalance(),
            &up_ob,
            &down_ob,
            &empty_orders(),
            &empty_orders(),
            &inventory,
            &default_config(),
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
            best_ask_is_ours: true,
        };
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.46,
        };

        let result = find_taker_opportunity(
            inventory.imbalance(),
            &up_ob,
            &down_ob,
            &empty_orders(),
            &empty_orders(),
            &inventory,
            &default_config(),
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
            best_ask: Some((0.50, 50.0)),
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

        let result = find_taker_opportunity(
            inventory.imbalance(),
            &up_ob,
            &down_ob,
            &empty_orders(),
            &empty_orders(),
            &inventory,
            &default_config(),
            "up",
            "down",
        );

        assert!(result.is_none());
    }

    #[test]
    fn test_no_taker_when_mirrored_order_is_ours() {
        // UP bid at 0.56 mirrors to DOWN ask at 0.44 - would self-trade
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
            best_ask_is_ours: false,
        };
        let up_orders = OrderSnapshot {
            bids: vec![OpenOrder::new("order-1".to_string(), 0.56, 100.0, 100.0)],
            asks: vec![],
        };
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.46,
        };

        let result = find_taker_opportunity(
            inventory.imbalance(),
            &up_ob,
            &down_ob,
            &up_orders,
            &empty_orders(),
            &inventory,
            &default_config(),
            "up",
            "down",
        );

        assert!(result.is_none());
    }

    #[test]
    fn test_taker_allowed_when_no_mirrored_order() {
        // UP bid at 0.54 mirrors to DOWN ask at 0.46, not 0.44 - no conflict
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
            best_ask_is_ours: false,
        };
        let up_orders = OrderSnapshot {
            bids: vec![OpenOrder::new("order-1".to_string(), 0.54, 100.0, 100.0)],
            asks: vec![],
        };
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.46,
        };

        let result = find_taker_opportunity(
            inventory.imbalance(),
            &up_ob,
            &down_ob,
            &up_orders,
            &empty_orders(),
            &inventory,
            &default_config(),
            "up",
            "down",
        );

        assert!(result.is_some());
        let taker = result.unwrap();
        assert_eq!(taker.token_id, "down");
        assert!((taker.price - 0.44).abs() < 0.001);
    }
}
