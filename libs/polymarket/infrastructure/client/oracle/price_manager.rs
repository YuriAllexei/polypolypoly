//! Oracle Price Manager
//!
//! Manages crypto prices from multiple oracle sources (ChainLink and Binance).
//! Provides thread-safe access to current prices via shared state.

use super::types::OracleType;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Shared price manager accessible by handlers and consumers
pub type SharedOraclePrices = Arc<RwLock<OraclePriceManager>>;

/// A single price entry with value and timestamp
#[derive(Debug, Clone, Copy)]
pub struct PriceEntry {
    pub value: f64,
    pub timestamp: u64,
}

impl PriceEntry {
    pub fn new(value: f64, timestamp: u64) -> Self {
        Self { value, timestamp }
    }
}

/// Manages crypto prices from multiple oracle sources
#[derive(Debug, Default)]
pub struct OraclePriceManager {
    /// ChainLink oracle prices (symbol -> price entry)
    pub chainlink: HashMap<String, PriceEntry>,
    /// Binance oracle prices (symbol -> price entry)
    pub binance: HashMap<String, PriceEntry>,
}

impl OraclePriceManager {
    /// Create a new empty price manager
    pub fn new() -> Self {
        Self {
            chainlink: HashMap::new(),
            binance: HashMap::new(),
        }
    }

    /// Update a price for the given oracle and symbol
    pub fn update_price(&mut self, oracle: OracleType, symbol: &str, value: f64, timestamp: u64) {
        let entry = PriceEntry::new(value, timestamp);
        let prices = match oracle {
            OracleType::ChainLink => &mut self.chainlink,
            OracleType::Binance => &mut self.binance,
        };
        prices.insert(symbol.to_uppercase(), entry);
    }

    /// Get a price for the given oracle and symbol
    pub fn get_price(&self, oracle: OracleType, symbol: &str) -> Option<PriceEntry> {
        let prices = match oracle {
            OracleType::ChainLink => &self.chainlink,
            OracleType::Binance => &self.binance,
        };
        prices.get(&symbol.to_uppercase()).copied()
    }

    /// Get all prices for a given oracle
    pub fn get_all_prices(&self, oracle: OracleType) -> &HashMap<String, PriceEntry> {
        match oracle {
            OracleType::ChainLink => &self.chainlink,
            OracleType::Binance => &self.binance,
        }
    }

    /// Get the number of tracked symbols for an oracle
    pub fn symbol_count(&self, oracle: OracleType) -> usize {
        match oracle {
            OracleType::ChainLink => self.chainlink.len(),
            OracleType::Binance => self.binance.len(),
        }
    }

    /// Get the total number of tracked symbols across all oracles
    pub fn total_symbol_count(&self) -> usize {
        self.chainlink.len() + self.binance.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_and_get_price() {
        let mut manager = OraclePriceManager::new();

        manager.update_price(OracleType::ChainLink, "ETH", 3456.78, 1000);

        let price = manager.get_price(OracleType::ChainLink, "ETH").unwrap();
        assert!((price.value - 3456.78).abs() < 0.001);
        assert_eq!(price.timestamp, 1000);
    }

    #[test]
    fn test_case_insensitive_lookup() {
        let mut manager = OraclePriceManager::new();

        manager.update_price(OracleType::Binance, "sol", 189.55, 2000);

        // Should find regardless of case
        assert!(manager.get_price(OracleType::Binance, "SOL").is_some());
        assert!(manager.get_price(OracleType::Binance, "sol").is_some());
        assert!(manager.get_price(OracleType::Binance, "Sol").is_some());
    }

    #[test]
    fn test_separate_oracle_storage() {
        let mut manager = OraclePriceManager::new();

        // Same symbol, different oracles
        manager.update_price(OracleType::ChainLink, "BTC", 100000.0, 1000);
        manager.update_price(OracleType::Binance, "BTC", 100001.0, 1000);

        let chainlink_price = manager.get_price(OracleType::ChainLink, "BTC").unwrap();
        let binance_price = manager.get_price(OracleType::Binance, "BTC").unwrap();

        assert!((chainlink_price.value - 100000.0).abs() < 0.001);
        assert!((binance_price.value - 100001.0).abs() < 0.001);
    }

    #[test]
    fn test_get_all_prices() {
        let mut manager = OraclePriceManager::new();

        manager.update_price(OracleType::ChainLink, "ETH", 3456.78, 1000);
        manager.update_price(OracleType::ChainLink, "BTC", 100000.0, 1001);

        let all_chainlink = manager.get_all_prices(OracleType::ChainLink);
        assert_eq!(all_chainlink.len(), 2);
        assert!(all_chainlink.contains_key("ETH"));
        assert!(all_chainlink.contains_key("BTC"));
    }

    #[test]
    fn test_symbol_count() {
        let mut manager = OraclePriceManager::new();

        manager.update_price(OracleType::ChainLink, "ETH", 3456.78, 1000);
        manager.update_price(OracleType::ChainLink, "BTC", 100000.0, 1001);
        manager.update_price(OracleType::Binance, "SOL", 189.55, 1002);

        assert_eq!(manager.symbol_count(OracleType::ChainLink), 2);
        assert_eq!(manager.symbol_count(OracleType::Binance), 1);
        assert_eq!(manager.total_symbol_count(), 3);
    }

    #[test]
    fn test_price_not_found() {
        let manager = OraclePriceManager::new();
        assert!(manager.get_price(OracleType::ChainLink, "XYZ").is_none());
    }
}
