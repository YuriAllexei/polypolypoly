//! Configuration for the Inventory MM strategy

use serde::{Deserialize, Serialize};

use super::components::merger::MergerConfig;
use super::components::taker::TakerConfig;
use super::types::SolverConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSpec {
    pub symbol: String,
    pub timeframe: String,
    #[serde(default = "default_market_count")]
    pub count: usize,
}

fn default_market_count() -> usize { 3 }

impl MarketSpec {
    pub fn new(symbol: impl Into<String>, timeframe: impl Into<String>, count: usize) -> Self {
        Self { symbol: symbol.into(), timeframe: timeframe.into(), count }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InventoryMMConfig {
    // === Market Selection ===
    pub markets: Vec<MarketSpec>,

    // === Timing ===
    pub poll_interval_secs: u64,
    pub tick_interval_ms: u64,
    pub snapshot_timeout_secs: u64,
    pub merge_cooldown_secs: u64,

    // === Solver ===
    pub solver: SolverConfig,

    // === Merger ===
    pub merger: MergerConfig,

    // === Taker ===
    #[serde(default)]
    pub taker: TakerConfig,
}

impl Default for InventoryMMConfig {
    fn default() -> Self {
        Self {
            markets: vec![
                MarketSpec::new("BTC", "15M", 3),
                MarketSpec::new("ETH", "15M", 3),
            ],
            poll_interval_secs: 30,
            tick_interval_ms: 100,
            snapshot_timeout_secs: 30,
            merge_cooldown_secs: 120,
            solver: SolverConfig::default(),
            merger: MergerConfig::default(),
            taker: TakerConfig::default(),
        }
    }
}

impl InventoryMMConfig {
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

    pub fn with_order_size(mut self, order_size: f64) -> Self {
        self.solver.order_size = order_size;
        self
    }

    pub fn with_base_offset(mut self, base_offset: f64) -> Self {
        self.solver.base_offset = base_offset;
        self
    }

    pub fn with_max_imbalance(mut self, max_imbalance: f64) -> Self {
        self.solver.max_imbalance = max_imbalance;
        self
    }

    pub fn with_min_merge_size(mut self, min_merge_size: f64) -> Self {
        self.merger.min_merge_size = min_merge_size;
        self
    }

    pub fn with_skew_factor(mut self, skew_factor: f64) -> Self {
        self.solver.skew_factor = skew_factor;
        self
    }

    pub fn is_symbol_enabled(&self, symbol: &str) -> bool {
        self.markets.iter().any(|m| m.symbol.eq_ignore_ascii_case(symbol))
    }

    pub fn is_timeframe_enabled(&self, timeframe: &str) -> bool {
        self.markets.iter().any(|m| m.timeframe.eq_ignore_ascii_case(timeframe))
    }

    pub fn get_count(&self, symbol: &str, timeframe: &str) -> Option<usize> {
        self.markets
            .iter()
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
        assert_eq!(config.snapshot_timeout_secs, 30);
        assert_eq!(config.merge_cooldown_secs, 120);
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
