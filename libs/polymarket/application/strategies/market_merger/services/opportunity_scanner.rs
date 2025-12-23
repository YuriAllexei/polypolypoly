//! Opportunity-based taker scanner for the Market Merger strategy
//!
//! Instead of threshold-based rebalancing, we actively scan for good taker opportunities
//! based on a multi-factor scoring system.

use crate::application::strategies::market_merger::config::MarketMergerConfig;
use crate::application::strategies::market_merger::types::{MarketContext, MarketState, TakerOpportunity};
use crate::domain::orderbook::Orderbook;

/// Scans orderbooks for profitable taker opportunities
pub struct OpportunityScanner {
    config: MarketMergerConfig,
}

impl OpportunityScanner {
    /// Create a new opportunity scanner
    pub fn new(config: &MarketMergerConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Scan both orderbooks for taker opportunities
    /// Returns the best opportunity if score exceeds minimum threshold
    pub fn scan(
        &self,
        ctx: &MarketContext,
        state: &MarketState,
        up_ob: &Orderbook,
        down_ob: &Orderbook,
    ) -> Option<TakerOpportunity> {
        let mut best: Option<TakerOpportunity> = None;

        // Scan Up asks
        if let Some((ask_price, ask_size)) = up_ob.best_ask() {
            if let Some(opp) = self.evaluate(
                &ctx.up_token_id,
                true, // is_up
                ask_price,
                ask_size,
                state.up_avg_cost,
                state.down_avg_cost,
                state.up_size,
                state.down_size,
                state.best_up_bid_price(),
            ) {
                if best.as_ref().map(|b| opp.score > b.score).unwrap_or(true) {
                    best = Some(opp);
                }
            }
        }

        // Scan Down asks
        if let Some((ask_price, ask_size)) = down_ob.best_ask() {
            if let Some(opp) = self.evaluate(
                &ctx.down_token_id,
                false, // is_up
                ask_price,
                ask_size,
                state.down_avg_cost,
                state.up_avg_cost,
                state.down_size,
                state.up_size,
                state.best_down_bid_price(),
            ) {
                if best.as_ref().map(|b| opp.score > b.score).unwrap_or(true) {
                    best = Some(opp);
                }
            }
        }

        // Only return if score meets minimum threshold
        best.filter(|o| o.is_viable(self.config.min_opportunity_score))
    }

    /// Evaluate a potential taker opportunity
    fn evaluate(
        &self,
        token_id: &str,
        is_up: bool,
        ask_price: f64,
        ask_size: f64,
        our_avg: f64,
        other_avg: f64,
        our_size: f64,
        other_size: f64,
        our_bid: Option<f64>,
    ) -> Option<TakerOpportunity> {
        let mut opp = TakerOpportunity::new(token_id.to_string(), is_up, ask_price, ask_size);

        // === HARD CONSTRAINT: Must be profitable ===
        // Combined cost after taking this fill must be < 0.98
        let combined_after = if our_size > 0.0 {
            // Calculate new weighted average after fill
            let new_avg = ((our_size * our_avg) + (ask_size * ask_price)) / (our_size + ask_size);
            new_avg + other_avg
        } else {
            ask_price + other_avg
        };

        // Must be under merge profit threshold (with buffer for existing positions)
        if other_avg > 0.0 && combined_after >= 0.98 {
            return None;
        }

        // If no other side position yet, be conservative
        if other_avg == 0.0 && ask_price >= 0.49 {
            return None;
        }

        // === Factor 1: Profit Margin (higher = better) ===
        if other_avg > 0.0 {
            let profit_margin = 1.0 - combined_after;
            let margin_score = profit_margin * self.config.profit_margin_weight;
            opp.add_score(margin_score, &format!("margin:{:.1}%", profit_margin * 100.0));
        }

        // === Factor 2: Price vs Our Bid ===
        if let Some(bid) = our_bid {
            let advantage = bid - ask_price;
            if advantage >= 0.0 {
                // Ask AT or BELOW our bid - excellent!
                let score = 10.0 + (advantage * self.config.price_vs_bid_weight);
                opp.add_score(score, &format!("ask≤bid+{:.0}c", advantage * 100.0));
            } else if advantage >= -0.02 {
                // Within 2 cents of our bid - still good
                opp.add_score(3.0, "near_bid");
            }
        }

        // === Factor 3: Delta Coverage ===
        let delta = other_size - our_size; // Positive = need more of this token
        if delta > 0.0 {
            let fill_size = ask_size.min(delta);
            let coverage = fill_size / delta;
            let coverage_score = coverage * self.config.delta_coverage_weight;
            opp.add_score(coverage_score, &format!("covers:{:.0}%", coverage * 100.0));
        }

        // === Factor 4: Improves Average Cost ===
        if our_size > 0.0 && ask_price < our_avg {
            let improvement = our_avg - ask_price;
            let improvement_score = improvement * self.config.avg_improvement_weight;
            opp.add_score(improvement_score, &format!("improves_avg:{:.0}c", improvement * 100.0));
        }

        // Calculate fill size (cap by delta if positive, or max_taker_size)
        let size = if delta > 0.0 {
            ask_size.min(delta).min(self.config.max_taker_size)
        } else {
            ask_size.min(self.config.max_taker_size)
        };
        opp.size = size;

        Some(opp)
    }

    /// Get the minimum opportunity score threshold
    pub fn min_score(&self) -> f64 {
        self.config.min_opportunity_score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opportunity_scoring() {
        let config = MarketMergerConfig::default();
        let scanner = OpportunityScanner::new(&config);

        // Create a mock opportunity
        let opp = scanner.evaluate(
            "token_up",
            true,
            0.46,   // ask_price (good)
            30.0,   // ask_size
            0.47,   // our_avg (ask is below our avg - good)
            0.48,   // other_avg
            100.0,  // our_size
            80.0,   // other_size (we need more Up to balance)
            Some(0.47), // our_bid (ask is below - excellent)
        );

        assert!(opp.is_some());
        let opp = opp.unwrap();

        // Should have good score due to:
        // - Profit margin (1.0 - 0.94 = 6%)
        // - Ask below our bid (1 cent)
        // - Covers delta (need 20 more)
        // - Below avg cost
        assert!(opp.score > 10.0, "Score {} should be > 10", opp.score);
    }
}
