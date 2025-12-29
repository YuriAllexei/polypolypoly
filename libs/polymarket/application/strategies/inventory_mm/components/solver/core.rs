//! Core solver function.

use crate::application::strategies::inventory_mm::types::{
    SolverInput, SolverOutput,
};

use super::quotes::calculate_quotes;
use super::diff::diff_orders;

/// Main solver function.
///
/// Quotes are purely market-based. Risk is managed via:
/// - Offset mechanism: increases when imbalanced, making bids less aggressive
/// - Max imbalance threshold: stops quoting entirely when too imbalanced
pub fn solve(input: &SolverInput) -> SolverOutput {
    let mut output = SolverOutput::new();

    let delta = input.inventory.imbalance();

    // 1. Calculate desired quote ladder for both sides
    let ladder = calculate_quotes(
        delta,
        &input.up_orderbook,
        &input.down_orderbook,
        &input.config,
        &input.up_token_id,
        &input.down_token_id,
    );

    // 2. Diff Up orders: current vs desired
    let (cancel_up, place_up) = diff_orders(
        &input.up_orders.bids,
        &ladder.up_quotes,
        &input.up_token_id,
    );
    output.cancellations.extend(cancel_up);
    output.limit_orders.extend(place_up);

    // 3. Diff Down orders: current vs desired
    let (cancel_down, place_down) = diff_orders(
        &input.down_orders.bids,
        &ladder.down_quotes,
        &input.down_token_id,
    );
    output.cancellations.extend(cancel_down);
    output.limit_orders.extend(place_down);

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::strategies::inventory_mm::types::{
        SolverConfig, InventorySnapshot, OrderbookSnapshot, OrderSnapshot,
    };

    fn make_input(
        up_ask: f64,
        down_ask: f64,
        up_size: f64,
        down_size: f64,
    ) -> SolverInput {
        SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size,
                up_avg_price: 0.50,
                down_size,
                down_avg_price: 0.48,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((up_ask, 100.0)),
                best_bid: Some((up_ask - 0.02, 50.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((down_ask, 100.0)),
                best_bid: Some((down_ask - 0.02, 50.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: SolverConfig::default(),
        }
    }

    #[test]
    fn test_solve_balanced_inventory() {
        let input = make_input(0.55, 0.45, 50.0, 50.0);
        let output = solve(&input);

        // Should generate quotes for both sides
        assert!(output.limit_orders.len() > 0);
    }

    #[test]
    fn test_solve_heavy_up_inventory() {
        let input = make_input(0.55, 0.45, 90.0, 10.0);
        let output = solve(&input);

        // With max_imbalance = 0.8, delta = 0.8, should stop Up quotes
        // Check that we have fewer Up orders or more Down orders
        let up_orders: Vec<_> = output.limit_orders.iter()
            .filter(|o| o.token_id == "up_token")
            .collect();
        let down_orders: Vec<_> = output.limit_orders.iter()
            .filter(|o| o.token_id == "down_token")
            .collect();

        // Should be aggressive on Down (have quotes), passive on Up (fewer/no quotes)
        assert!(down_orders.len() >= up_orders.len());
    }

    #[test]
    fn test_solve_with_existing_inventory() {
        // Even with "unprofitable" inventory, we still quote
        // Risk is managed via offset mechanism, not profitability checks
        let mut input = make_input(0.55, 0.46, 50.0, 50.0);
        input.inventory.up_avg_price = 0.52;
        input.inventory.down_avg_price = 0.49;

        let output = solve(&input);

        // Should still generate quotes (no profitability cap anymore)
        assert!(!output.limit_orders.is_empty());
    }

    #[test]
    fn test_solve_tight_spread() {
        // Market with tight spread - solver still quotes at market prices
        let input = make_input(0.51, 0.50, 50.0, 50.0);

        let output = solve(&input);

        // Should generate quotes based on market prices
        // No special handling for tight spreads anymore
        assert!(!output.limit_orders.is_empty());
    }
}
