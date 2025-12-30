//! Configuration for the TakerTask.

use serde::{Deserialize, Serialize};

/// Configuration for taker order execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TakerConfig {
    /// Whether taker functionality is enabled
    pub enabled: bool,
    /// Tick interval in milliseconds for checking taker opportunities
    pub tick_interval_ms: u64,
    /// Minimum delta (imbalance) threshold to trigger taker check
    /// Range: 0.0 to 1.0 (e.g., 0.1 = 10% imbalance required)
    pub min_delta_threshold: f64,
    /// Maximum size per taker order
    pub max_take_size: f64,
    /// Minimum size per taker order (orders below this are skipped)
    pub min_take_size: f64,
    /// Maximum combined average cost for profitability (e.g., 0.99 = require 1% profit)
    /// Trades with combined_avg >= this value will be rejected
    pub max_combined_avg: f64,
}

impl Default for TakerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tick_interval_ms: 100,
            min_delta_threshold: 0.1,
            max_take_size: 100.0,
            min_take_size: 1.0,
            max_combined_avg: 0.99, // Require at least 1% profit margin
        }
    }
}

impl TakerConfig {
    /// Create a new TakerConfig with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a disabled TakerConfig.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TakerConfig::default();
        assert!(config.enabled);
        assert_eq!(config.tick_interval_ms, 100);
        assert!((config.min_delta_threshold - 0.1).abs() < 1e-9);
        assert!((config.max_take_size - 100.0).abs() < 1e-9);
        assert!((config.min_take_size - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_min_take_size_configurable() {
        let config = TakerConfig {
            min_take_size: 5.0,
            ..TakerConfig::default()
        };
        assert!((config.min_take_size - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_disabled_config() {
        let config = TakerConfig::disabled();
        assert!(!config.enabled);
    }
}
