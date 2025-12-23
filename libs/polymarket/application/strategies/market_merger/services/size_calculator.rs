//! Size calculation for the Market Merger strategy

use crate::application::strategies::market_merger::config::MarketMergerConfig;
use crate::application::strategies::market_merger::types::{MarketState, QuoteLadder, SizingPhase};

/// Calculates bid sizes based on phase and inventory balance
pub struct SizeCalculator {
    config: MarketMergerConfig,
}

impl SizeCalculator {
    /// Create a new size calculator
    pub fn new(config: &MarketMergerConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Calculate sizes based on phase and inventory balance
    pub fn calculate_sizes(
        &self,
        state: &MarketState,
        balance: f64,
        ladder: &mut QuoteLadder,
    ) {
        // Determine size percentage based on phase
        let base_pct = match state.phase {
            SizingPhase::Bootstrap => self.config.bootstrap_size_pct, // 1%
            SizingPhase::Confirmed => self.config.confirmed_size_pct, // 3%
            SizingPhase::Scaled => self.config.scaled_size_pct,       // 5%
        };

        let base_size = balance * base_pct;

        // Inventory-based multipliers
        let imbalance = state.imbalance();
        let (up_mult, down_mult) = if state.up_size > state.down_size {
            // Overweight Up: reduce Up sizes, increase Down sizes
            (1.0 / (1.0 + imbalance), 1.0 + imbalance)
        } else if state.down_size > state.up_size {
            // Overweight Down: reduce Down sizes, increase Up sizes
            (1.0 + imbalance, 1.0 / (1.0 + imbalance))
        } else {
            (1.0, 1.0)
        };

        // Apply sizes to Up bids
        for bid in &mut ladder.up_bids {
            let level_mult = self.config.size_multiplier_for_level(bid.level);
            let size = base_size * up_mult * level_mult;
            // Size is in tokens, convert from USD: tokens = USD / price
            if bid.price > 0.0 {
                bid.size = (size / bid.price).min(self.config.max_quote_size_usd / bid.price);
            }
        }

        // Apply sizes to Down bids
        for bid in &mut ladder.down_bids {
            let level_mult = self.config.size_multiplier_for_level(bid.level);
            let size = base_size * down_mult * level_mult;
            // Size is in tokens, convert from USD: tokens = USD / price
            if bid.price > 0.0 {
                bid.size = (size / bid.price).min(self.config.max_quote_size_usd / bid.price);
            }
        }
    }

    /// Update phase based on current position value
    pub fn update_phase(&self, state: &mut MarketState) {
        let position_value = state.total_position_value();

        state.phase = if position_value < self.config.bootstrap_threshold_usd {
            SizingPhase::Bootstrap
        } else if position_value < self.config.confirmed_threshold_usd {
            SizingPhase::Confirmed
        } else {
            SizingPhase::Scaled
        };
    }

    /// Get the current sizing percentage for a phase
    pub fn size_pct_for_phase(&self, phase: SizingPhase) -> f64 {
        match phase {
            SizingPhase::Bootstrap => self.config.bootstrap_size_pct,
            SizingPhase::Confirmed => self.config.confirmed_size_pct,
            SizingPhase::Scaled => self.config.scaled_size_pct,
        }
    }
}
