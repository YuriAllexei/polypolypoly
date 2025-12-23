//! Quote price calculation for the Market Merger strategy

use crate::application::strategies::market_merger::config::MarketMergerConfig;
use crate::application::strategies::market_merger::types::{MarketContext, MarketState, Quote, QuoteLadder};
use crate::domain::orderbook::Orderbook;

/// Calculates bid prices for the multi-level quote ladder
pub struct QuoteCalculator {
    config: MarketMergerConfig,
}

impl QuoteCalculator {
    /// Create a new quote calculator
    pub fn new(config: &MarketMergerConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Calculate bid prices for multi-level ladder
    /// Key constraint: up_bid + down_bid < $1.00 (STP enforces this)
    pub fn calculate_bids(
        &self,
        ctx: &MarketContext,
        state: &MarketState,
        up_ob: &Orderbook,
        down_ob: &Orderbook,
    ) -> QuoteLadder {
        // Max bid we can place while staying profitable
        // If we have no position on the other side, use a conservative max
        let max_up_bid = if state.down_avg_cost > 0.0 {
            1.0 - state.down_avg_cost - self.config.min_profit_margin
        } else {
            0.49 - self.config.min_profit_margin // Conservative: assume other side ~0.49
        };

        let max_down_bid = if state.up_avg_cost > 0.0 {
            1.0 - state.up_avg_cost - self.config.min_profit_margin
        } else {
            0.49 - self.config.min_profit_margin
        };

        // Market best bids
        let up_best = up_ob.best_bid().map(|(p, _)| p).unwrap_or(0.40);
        let down_best = down_ob.best_bid().map(|(p, _)| p).unwrap_or(0.40);

        // Calculate ladder for each token
        let up_bids = self.build_ladder(
            &ctx.up_token_id,
            up_best,
            max_up_bid,
            ctx.tick_size,
            state.imbalance(),
            state.up_size > state.down_size, // is overweight?
        );

        let down_bids = self.build_ladder(
            &ctx.down_token_id,
            down_best,
            max_down_bid,
            ctx.tick_size,
            state.imbalance(),
            state.down_size > state.up_size,
        );

        QuoteLadder { up_bids, down_bids }
    }

    /// Build a ladder of bids for a single token
    fn build_ladder(
        &self,
        token_id: &str,
        market_best: f64,
        max_bid: f64,
        tick_size: f64,
        imbalance: f64,
        is_overweight: bool,
    ) -> Vec<Quote> {
        let mut bids = Vec::new();

        // If too imbalanced on this side, don't quote at all
        if is_overweight && imbalance > self.config.max_imbalance_halt {
            return bids;
        }

        for level in 0..self.config.num_levels {
            let spread_cents = self.config.spread_for_level(level);

            // Base price: market_best - spread
            let mut price = market_best - (spread_cents / 100.0);

            // Inventory adjustment: if overweight, widen bids (buy cheaper)
            if is_overweight && imbalance > self.config.spread_adjust_threshold {
                // Push bid down by up to 5 cents based on imbalance
                price -= imbalance * 0.05;
            }

            // Cap at profitability limit
            price = price.min(max_bid);

            // Round to tick size
            price = round_to_tick(price, tick_size);

            // Skip if price too low (not worth quoting)
            if price < 0.30 {
                continue;
            }

            bids.push(Quote::new(token_id.to_string(), price, 0.0, level));
        }

        bids
    }

    /// Calculate the maximum viable combined cost
    pub fn max_combined_cost(&self) -> f64 {
        1.0 - self.config.min_profit_margin
    }
}

/// Round price to tick size
fn round_to_tick(price: f64, tick_size: f64) -> f64 {
    (price / tick_size).floor() * tick_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_to_tick() {
        assert_eq!(round_to_tick(0.456, 0.01), 0.45);
        assert_eq!(round_to_tick(0.459, 0.01), 0.45);
        assert_eq!(round_to_tick(0.45, 0.01), 0.45);
    }
}
