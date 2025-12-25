//! Configuration for the Inventory MM strategy

use super::types::SolverConfig;
use super::components::merger::MergerConfig;

/// Complete configuration for Inventory MM strategy
#[derive(Debug, Clone)]
pub struct InventoryMMConfig {
    /// Solver configuration
    pub solver: SolverConfig,

    /// Merger configuration
    pub merger: MergerConfig,

    /// Update interval for solver loop (milliseconds)
    pub update_interval_ms: u64,

    /// Token IDs
    pub up_token_id: String,
    pub down_token_id: String,
    pub condition_id: String,
}

impl InventoryMMConfig {
    pub fn new(
        up_token_id: String,
        down_token_id: String,
        condition_id: String,
    ) -> Self {
        Self {
            solver: SolverConfig::default(),
            merger: MergerConfig::default(),
            update_interval_ms: 100, // 100ms per senior dev
            up_token_id,
            down_token_id,
            condition_id,
        }
    }

    /// Builder-style setters
    pub fn with_num_levels(mut self, num_levels: usize) -> Self {
        self.solver.num_levels = num_levels;
        self
    }

    pub fn with_tick_size(mut self, tick_size: f64) -> Self {
        self.solver.tick_size = tick_size;
        self
    }

    pub fn with_base_offset(mut self, base_offset: f64) -> Self {
        self.solver.base_offset = base_offset;
        self
    }

    pub fn with_min_profit_margin(mut self, margin: f64) -> Self {
        self.solver.min_profit_margin = margin;
        self.merger.min_profit_margin = margin;
        self.merger.max_combined_cost = 1.0 - margin;
        self
    }

    pub fn with_max_imbalance(mut self, max_imbalance: f64) -> Self {
        self.solver.max_imbalance = max_imbalance;
        self
    }

    pub fn with_order_size(mut self, order_size: f64) -> Self {
        self.solver.order_size = order_size;
        self
    }

    pub fn with_min_merge_size(mut self, min_merge_size: f64) -> Self {
        self.merger.min_merge_size = min_merge_size;
        self
    }

    pub fn with_update_interval_ms(mut self, interval_ms: u64) -> Self {
        self.update_interval_ms = interval_ms;
        self
    }
}

impl Default for InventoryMMConfig {
    fn default() -> Self {
        Self {
            solver: SolverConfig::default(),
            merger: MergerConfig::default(),
            update_interval_ms: 100,
            up_token_id: String::new(),
            down_token_id: String::new(),
            condition_id: String::new(),
        }
    }
}
