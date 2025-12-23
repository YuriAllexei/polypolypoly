//! Merge decision logic for the Market Merger strategy

use crate::application::strategies::market_merger::config::MarketMergerConfig;
use crate::application::strategies::market_merger::types::MarketState;

/// Checks conditions for merging positions
pub struct MergeChecker {
    config: MarketMergerConfig,
}

/// Result of a merge check
#[derive(Debug, Clone)]
pub struct MergeDecision {
    /// Whether to proceed with merge
    pub should_merge: bool,
    /// Number of pairs to merge
    pub pairs: u64,
    /// Expected profit from merge
    pub expected_profit: f64,
    /// Reason for decision
    pub reason: String,
}

impl MergeDecision {
    /// Create a positive merge decision
    pub fn yes(pairs: u64, expected_profit: f64) -> Self {
        Self {
            should_merge: true,
            pairs,
            expected_profit,
            reason: "All conditions met".to_string(),
        }
    }

    /// Create a negative merge decision with reason
    pub fn no(reason: &str) -> Self {
        Self {
            should_merge: false,
            pairs: 0,
            expected_profit: 0.0,
            reason: reason.to_string(),
        }
    }
}

impl MergeChecker {
    /// Create a new merge checker
    pub fn new(config: &MarketMergerConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Check if conditions are met for merging positions
    pub fn should_merge(&self, state: &MarketState) -> MergeDecision {
        let mergeable = state.mergeable_pairs();

        // Minimum pairs check
        if mergeable < self.config.min_merge_pairs as f64 {
            return MergeDecision::no(&format!(
                "Below minimum pairs: {:.0} < {}",
                mergeable, self.config.min_merge_pairs
            ));
        }

        // Profitability check (with fee buffer)
        let combined = state.combined_cost();
        if combined >= self.config.merge_profit_threshold {
            return MergeDecision::no(&format!(
                "Not profitable: {:.4} >= {:.2}",
                combined, self.config.merge_profit_threshold
            ));
        }

        // Balance check - positions should be roughly equal
        let imbalance = state.imbalance();
        if imbalance > self.config.max_merge_imbalance {
            return MergeDecision::no(&format!(
                "Too imbalanced: {:.1}% > {:.1}%",
                imbalance * 100.0,
                self.config.max_merge_imbalance * 100.0
            ));
        }

        // Cost spread check - wait for similar avg costs on both sides
        let cost_spread = (state.up_avg_cost - state.down_avg_cost).abs();
        if cost_spread > self.config.max_cost_spread {
            return MergeDecision::no(&format!(
                "Cost spread too large: ${:.4} > ${:.2}",
                cost_spread, self.config.max_cost_spread
            ));
        }

        // All conditions met!
        let pairs = mergeable as u64;
        let profit = pairs as f64 * (1.0 - combined);

        MergeDecision::yes(pairs, profit)
    }

    /// Calculate the potential profit from merging current positions
    pub fn potential_profit(&self, state: &MarketState) -> f64 {
        if !state.is_profitable() {
            return 0.0;
        }

        let pairs = state.mergeable_pairs();
        pairs * (1.0 - state.combined_cost())
    }

    /// Get the minimum pairs required for merge
    pub fn min_pairs(&self) -> u64 {
        self.config.min_merge_pairs
    }
}

impl std::fmt::Display for MergeDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.should_merge {
            write!(
                f,
                "MERGE {} pairs for ${:.2} profit",
                self.pairs, self.expected_profit
            )
        } else {
            write!(f, "NO MERGE: {}", self.reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(up_size: f64, up_avg: f64, down_size: f64, down_avg: f64) -> MarketState {
        let mut state = MarketState::new();
        state.up_size = up_size;
        state.up_avg_cost = up_avg;
        state.down_size = down_size;
        state.down_avg_cost = down_avg;
        state
    }

    #[test]
    fn test_should_merge_profitable() {
        let config = MarketMergerConfig::default();
        let checker = MergeChecker::new(&config);

        // Good case: 100 pairs, combined cost $0.95
        let state = make_state(100.0, 0.47, 100.0, 0.48);
        let decision = checker.should_merge(&state);

        assert!(decision.should_merge);
        assert_eq!(decision.pairs, 100);
        assert!((decision.expected_profit - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_should_not_merge_unprofitable() {
        let config = MarketMergerConfig::default();
        let checker = MergeChecker::new(&config);

        // Bad case: combined cost $0.99
        let state = make_state(100.0, 0.50, 100.0, 0.49);
        let decision = checker.should_merge(&state);

        assert!(!decision.should_merge);
        assert!(decision.reason.contains("Not profitable"));
    }

    #[test]
    fn test_should_not_merge_too_few() {
        let config = MarketMergerConfig::default();
        let checker = MergeChecker::new(&config);

        // Bad case: only 5 pairs (below min of 10)
        let state = make_state(5.0, 0.47, 5.0, 0.48);
        let decision = checker.should_merge(&state);

        assert!(!decision.should_merge);
        assert!(decision.reason.contains("minimum pairs"));
    }

    #[test]
    fn test_should_not_merge_imbalanced() {
        let mut config = MarketMergerConfig::default();
        config.max_merge_imbalance = 0.05; // 5%
        let checker = MergeChecker::new(&config);

        // Bad case: 100 Up, 80 Down (20% imbalance)
        let state = make_state(100.0, 0.47, 80.0, 0.48);
        let decision = checker.should_merge(&state);

        assert!(!decision.should_merge);
        assert!(decision.reason.contains("imbalanced"));
    }
}
