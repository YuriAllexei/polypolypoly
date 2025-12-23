//! Sniper Strategies Module
//!
//! Pluggable strategy system for the market sniper.

pub mod market_merger;
pub mod sports_sniping;
pub mod traits;
pub mod up_or_down;

// Re-exports
pub use market_merger::MarketMergerStrategy;
pub use sports_sniping::SportsSnipingStrategy;
pub use traits::{Strategy, StrategyContext, StrategyError, StrategyResult};
pub use up_or_down::UpOrDownStrategy;

use crate::infrastructure::config::StrategiesConfig;

/// Available strategy types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyType {
    UpOrDown,
    SportsSniping,
    MarketMerger,
}

impl StrategyType {
    /// Parse strategy type from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "upordown" => Some(Self::UpOrDown),
            "sportssniping" => Some(Self::SportsSniping),
            "marketmerger" => Some(Self::MarketMerger),
            _ => None,
        }
    }

    /// Get the strategy name
    pub fn name(&self) -> &str {
        match self {
            Self::UpOrDown => "up_or_down",
            Self::SportsSniping => "sports_sniping",
            Self::MarketMerger => "market_merger",
        }
    }

    /// List all available strategy names
    pub fn available() -> Vec<&'static str> {
        vec!["up_or_down", "sports_sniping", "market_merger"]
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
        StrategyType::MarketMerger => {
            Box::new(MarketMergerStrategy::new(config.market_merger.clone()))
        }
    }
}
