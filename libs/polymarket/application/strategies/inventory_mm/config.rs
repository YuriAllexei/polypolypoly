//! Configuration for the Inventory MM strategy

use serde::{Deserialize, Serialize};

use super::types::SolverConfig;
use super::components::merger::MergerConfig;

/// Specifies which markets to track for a given symbol/timeframe combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSpec {
    /// Crypto symbol (e.g., "BTC", "ETH")
    pub symbol: String,
    /// Timeframe (e.g., "15M", "1H", "4H")
    pub timeframe: String,
    /// Number of upcoming markets to track for this spec
    #[serde(default = "default_market_count")]
    pub count: usize,
}

fn default_market_count() -> usize {
    3
}

impl MarketSpec {
    pub fn new(symbol: impl Into<String>, timeframe: impl Into<String>, count: usize) -> Self {
        Self {
            symbol: symbol.into(),
            timeframe: timeframe.into(),
            count,
        }
    }
}

/// Complete configuration for Inventory MM strategy (multi-market).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryMMConfig {
    /// Markets to track - each spec defines (symbol, timeframe, count)
    #[serde(default = "default_markets")]
    pub markets: Vec<MarketSpec>,

    /// How often to poll DB for new markets (seconds)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    /// Tick interval for quoter loop (milliseconds)
    #[serde(default = "default_tick_interval")]
    pub tick_interval_ms: u64,

    /// Solver configuration (shared across all quoters)
    #[serde(default)]
    pub solver: SolverConfig,

    /// Merger configuration (shared across all quoters)
    #[serde(default)]
    pub merger: MergerConfig,
}

fn default_markets() -> Vec<MarketSpec> {
    vec![
        MarketSpec::new("BTC", "15M", 3),
        MarketSpec::new("ETH", "15M", 3),
    ]
}

fn default_poll_interval() -> u64 {
    30 // 30 seconds
}

fn default_tick_interval() -> u64 {
    100 // 100ms
}

impl Default for InventoryMMConfig {
    fn default() -> Self {
        Self {
            markets: default_markets(),
            poll_interval_secs: default_poll_interval(),
            tick_interval_ms: default_tick_interval(),
            solver: SolverConfig::default(),
            merger: MergerConfig::default(),
        }
    }
}

impl InventoryMMConfig {
    /// Builder-style setters
    pub fn with_markets(mut self, markets: Vec<MarketSpec>) -> Self {
        self.markets = markets;
        self
    }

    pub fn with_poll_interval_secs(mut self, secs: u64) -> Self {
        self.poll_interval_secs = secs;
        self
    }

    pub fn with_tick_interval_ms(mut self, ms: u64) -> Self {
        self.tick_interval_ms = ms;
        self
    }

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

    /// Check if a symbol is configured for trading
    pub fn is_symbol_enabled(&self, symbol: &str) -> bool {
        self.markets.iter().any(|m| m.symbol.eq_ignore_ascii_case(symbol))
    }

    /// Check if a timeframe is configured for trading
    pub fn is_timeframe_enabled(&self, timeframe: &str) -> bool {
        self.markets.iter().any(|m| m.timeframe.eq_ignore_ascii_case(timeframe))
    }

    /// Get the count for a specific (symbol, timeframe) combination
    pub fn get_count(&self, symbol: &str, timeframe: &str) -> Option<usize> {
        self.markets.iter()
            .find(|m| m.symbol.eq_ignore_ascii_case(symbol) && m.timeframe.eq_ignore_ascii_case(timeframe))
            .map(|m| m.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = InventoryMMConfig::default();
        assert_eq!(config.markets.len(), 2);
        assert_eq!(config.poll_interval_secs, 30);
        assert_eq!(config.tick_interval_ms, 100);
    }

    #[test]
    fn test_market_spec() {
        let spec = MarketSpec::new("SOL", "4H", 2);
        assert_eq!(spec.symbol, "SOL");
        assert_eq!(spec.timeframe, "4H");
        assert_eq!(spec.count, 2);
    }

    #[test]
    fn test_is_symbol_enabled() {
        let config = InventoryMMConfig::default();
        assert!(config.is_symbol_enabled("btc"));
        assert!(config.is_symbol_enabled("ETH"));
        assert!(!config.is_symbol_enabled("SOL"));
    }
}
