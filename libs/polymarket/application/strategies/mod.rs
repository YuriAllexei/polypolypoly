//! Sniper Strategies Module
//!
//! Pluggable strategy system for the market sniper.

pub mod inventory_mm;
pub mod sports_sniping;
pub mod traits;
pub mod up_or_down;

// Re-exports
pub use inventory_mm::InventoryMMStrategy;
pub use sports_sniping::SportsSnipingStrategy;
pub use traits::{Strategy, StrategyContext, StrategyError, StrategyResult};
pub use up_or_down::UpOrDownStrategy;

use crate::infrastructure::config::StrategiesConfig;

/// Available strategy types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyType {
    UpOrDown,
    SportsSniping,
    InventoryMM,
}

impl StrategyType {
    /// Parse strategy type from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "upordown" => Some(Self::UpOrDown),
            "sportssniping" => Some(Self::SportsSniping),
            "inventorymm" => Some(Self::InventoryMM),
            _ => None,
        }
    }

    /// Get the strategy name
    pub fn name(&self) -> &str {
        match self {
            Self::UpOrDown => "up_or_down",
            Self::SportsSniping => "sports_sniping",
            Self::InventoryMM => "inventory_mm",
        }
    }

    /// List all available strategy names
    pub fn available() -> Vec<&'static str> {
        vec!["up_or_down", "sports_sniping", "inventory_mm"]
    }
}

/// Factory function to create strategies based on type
pub fn create_strategy(
    strategy_type: &StrategyType,
    config: &StrategiesConfig,
) -> Box<dyn Strategy> {
    match strategy_type {
        StrategyType::UpOrDown => Box::new(UpOrDownStrategy::new(config.up_or_down.clone())),
        StrategyType::SportsSniping => {
            Box::new(SportsSnipingStrategy::new(config.sports_sniping.clone()))
        }
        StrategyType::InventoryMM => {
            Box::new(InventoryMMStrategy::new(config.inventory_mm.clone()))
        }
    }
}
