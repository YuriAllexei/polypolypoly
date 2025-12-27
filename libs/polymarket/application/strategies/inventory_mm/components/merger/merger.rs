//! Merger component - monitors inventory and triggers merges.

use tracing::{info, debug};

use crate::application::strategies::inventory_mm::types::InventorySnapshot;

/// Configuration for the Merger
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MergerConfig {
    /// Minimum pairs before considering a merge
    pub min_merge_size: f64,

    /// Maximum imbalance allowed for merge (e.g., 0.3 = 30%)
    /// If delta is too high, wait for more balance
    pub max_merge_imbalance: f64,

    /// Minimum profit margin required (e.g., 0.01 = 1 cent per pair)
    pub min_profit_margin: f64,

    /// Maximum combined avg cost (1.0 - min_profit_margin)
    pub max_combined_cost: f64,
}

impl Default for MergerConfig {
    fn default() -> Self {
        Self {
            min_merge_size: 10.0,
            max_merge_imbalance: 0.3,
            min_profit_margin: 0.01,
            max_combined_cost: 0.99,
        }
    }
}

/// Floating point epsilon for comparisons
const EPSILON: f64 = 1e-9;

impl MergerConfig {
    /// Create new MergerConfig with validation.
    ///
    /// # Panics
    /// Panics if min_merge_size <= 0 or min_profit_margin is not in (0, 1).
    pub fn new(min_merge_size: f64, min_profit_margin: f64) -> Self {
        assert!(
            min_merge_size > 0.0,
            "min_merge_size must be positive, got {}",
            min_merge_size
        );
        assert!(
            min_profit_margin > 0.0 && min_profit_margin < 1.0,
            "min_profit_margin must be in (0, 1), got {}",
            min_profit_margin
        );

        Self {
            min_merge_size,
            min_profit_margin,
            max_combined_cost: 1.0 - min_profit_margin,
            ..Default::default()
        }
    }

    /// Validate config values. Returns error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_merge_size <= 0.0 {
            return Err(format!(
                "min_merge_size must be positive, got {}",
                self.min_merge_size
            ));
        }
        if self.min_profit_margin <= 0.0 || self.min_profit_margin >= 1.0 {
            return Err(format!(
                "min_profit_margin must be in (0, 1), got {}",
                self.min_profit_margin
            ));
        }
        if self.max_merge_imbalance <= 0.0 || self.max_merge_imbalance > 1.0 {
            return Err(format!(
                "max_merge_imbalance must be in (0, 1], got {}",
                self.max_merge_imbalance
            ));
        }
        Ok(())
    }
}

/// Result of merge decision check
#[derive(Debug, Clone)]
pub struct MergeDecision {
    /// Should we merge?
    pub should_merge: bool,

    /// Number of pairs to merge
    pub pairs_to_merge: f64,

    /// Expected profit from merge
    pub expected_profit: f64,

    /// Reason for decision (for logging)
    pub reason: String,
}

impl MergeDecision {
    pub fn no_merge(reason: impl Into<String>) -> Self {
        Self {
            should_merge: false,
            pairs_to_merge: 0.0,
            expected_profit: 0.0,
            reason: reason.into(),
        }
    }

    pub fn merge(pairs: f64, profit: f64) -> Self {
        Self {
            should_merge: true,
            pairs_to_merge: pairs,
            expected_profit: profit,
            reason: format!("Merge {} pairs for ${:.4} profit", pairs, profit),
        }
    }
}

/// Merger component - pure decision logic for when to merge YES+NO tokens.
/// Stateless: does not store market-specific info, only config.
pub struct Merger {
    config: MergerConfig,
}

impl Merger {
    pub fn new(config: MergerConfig) -> Self {
        Self { config }
    }

    /// Check if we should merge based on current inventory.
    pub fn check_merge(&self, inventory: &InventorySnapshot) -> MergeDecision {
        let delta = inventory.imbalance();
        let pairs = inventory.pairs_available();
        let combined_cost = inventory.combined_avg_cost();

        debug!(
            "[Merger] Checking: delta={:.3}, pairs={:.1}, combined_cost={:.4}",
            delta, pairs, combined_cost
        );

        // Check 1: Enough pairs?
        if pairs < self.config.min_merge_size {
            return MergeDecision::no_merge(format!(
                "Not enough pairs: {:.1} < {:.1}",
                pairs, self.config.min_merge_size
            ));
        }

        // Check 2: Delta within threshold?
        if delta.abs() > self.config.max_merge_imbalance {
            return MergeDecision::no_merge(format!(
                "Imbalance too high: {:.3} > {:.3}",
                delta.abs(), self.config.max_merge_imbalance
            ));
        }

        // Check 3: Profitable? (use epsilon for floating point comparison)
        if combined_cost >= self.config.max_combined_cost - EPSILON {
            return MergeDecision::no_merge(format!(
                "Not profitable: combined {:.4} >= max {:.4}",
                combined_cost, self.config.max_combined_cost
            ));
        }

        // All checks pass - calculate profit and merge
        let profit_per_pair = 1.0 - combined_cost;
        let total_profit = pairs * profit_per_pair;

        info!(
            "[Merger] Merge opportunity: {} pairs @ ${:.4} combined = ${:.4} profit",
            pairs, combined_cost, total_profit
        );

        MergeDecision::merge(pairs, total_profit)
    }

    /// Get config reference
    pub fn config(&self) -> &MergerConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_merger() -> Merger {
        Merger::new(MergerConfig::default())
    }

    #[test]
    fn test_check_merge_profitable_balanced() {
        let merger = default_merger();
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };

        let decision = merger.check_merge(&inventory);

        // Combined = 0.98, pairs = 50, delta = 0
        assert!(decision.should_merge);
        assert!((decision.pairs_to_merge - 50.0).abs() < 0.01);
        assert!((decision.expected_profit - 1.0).abs() < 0.01); // 50 * 0.02
    }

    #[test]
    fn test_check_merge_not_enough_pairs() {
        let merger = default_merger();
        let inventory = InventorySnapshot {
            up_size: 5.0,
            up_avg_price: 0.52,
            down_size: 5.0,
            down_avg_price: 0.46,
        };

        let decision = merger.check_merge(&inventory);

        // Only 5 pairs, need 10
        assert!(!decision.should_merge);
        assert!(decision.reason.contains("Not enough pairs"));
    }

    #[test]
    fn test_check_merge_too_imbalanced() {
        let merger = default_merger();
        let inventory = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.46,
        };

        let decision = merger.check_merge(&inventory);

        // Delta = 0.6, max = 0.3
        assert!(!decision.should_merge);
        assert!(decision.reason.contains("Imbalance too high"));
    }

    #[test]
    fn test_check_merge_not_profitable() {
        let merger = default_merger();
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.48, // Combined = 1.00
        };

        let decision = merger.check_merge(&inventory);

        // Combined = 1.00, not profitable
        assert!(!decision.should_merge);
        assert!(decision.reason.contains("Not profitable"));
    }

    #[test]
    fn test_check_merge_barely_profitable() {
        let merger = default_merger();
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.51,
            down_size: 50.0,
            down_avg_price: 0.47, // Combined = 0.98
        };

        let decision = merger.check_merge(&inventory);

        // Combined = 0.98 < 0.99, profitable
        assert!(decision.should_merge);
        assert!((decision.expected_profit - 1.0).abs() < 0.01); // 50 * 0.02
    }
}
