//! Oracle Price Manager
//!
//! Manages crypto prices from multiple oracle sources (ChainLink and Binance).
//! Provides thread-safe access to current prices via shared state.
//!
//! ## Health Tracking
//!
//! The price manager tracks the health of each oracle connection:
//! - `received_at` on each price entry tracks when we received data
//! - `OracleHealthState` tracks the last update time for each oracle
//!
//! This allows strategies to detect stale data even when the WebSocket
//! appears connected (zombie connection detection).

use super::types::OracleType;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Shared price manager accessible by handlers and consumers
pub type SharedOraclePrices = Arc<RwLock<OraclePriceManager>>;

/// A single price entry with value, timestamp, and local receive time
#[derive(Debug, Clone, Copy)]
pub struct PriceEntry {
    /// The price value
    pub value: f64,
    /// Server-side timestamp (from oracle)
    pub timestamp: u64,
    /// When we locally received this update (for staleness detection)
    pub received_at: Instant,
}

impl PriceEntry {
    pub fn new(value: f64, timestamp: u64) -> Self {
        Self {
            value,
            timestamp,
            received_at: Instant::now(),
        }
    }

    /// Get the age of this price entry (time since we received it)
    pub fn age(&self) -> Duration {
        self.received_at.elapsed()
    }

    /// Check if this price entry is stale (older than max_age)
    pub fn is_stale(&self, max_age: Duration) -> bool {
        self.age() > max_age
    }
}

/// Health state for a single oracle connection
#[derive(Debug)]
pub struct OracleHealthState {
    /// When we last received ANY update from this oracle
    pub last_update: Instant,
    /// Total number of messages received from this oracle
    pub message_count: u64,
}

impl Default for OracleHealthState {
    fn default() -> Self {
        Self {
            last_update: Instant::now(),
            message_count: 0,
        }
    }
}

impl OracleHealthState {
    /// Create a new health state
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that we received an update
    pub fn record_update(&mut self) {
        self.last_update = Instant::now();
        self.message_count += 1;
    }

    /// Get time since last update
    pub fn time_since_update(&self) -> Duration {
        self.last_update.elapsed()
    }

    /// Reset health state (call after reconnection)
    pub fn reset(&mut self) {
        self.last_update = Instant::now();
        // Keep message_count for cumulative stats
    }
}

/// Manages crypto prices from multiple oracle sources
#[derive(Debug)]
pub struct OraclePriceManager {
    /// ChainLink oracle prices (symbol -> price entry)
    pub chainlink: HashMap<String, PriceEntry>,
    /// Binance oracle prices (symbol -> price entry)
    pub binance: HashMap<String, PriceEntry>,
    /// Health state for ChainLink oracle connection
    pub chainlink_health: OracleHealthState,
    /// Health state for Binance oracle connection
    pub binance_health: OracleHealthState,
}

impl Default for OraclePriceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl OraclePriceManager {
    /// Create a new empty price manager
    pub fn new() -> Self {
        Self {
            chainlink: HashMap::new(),
            binance: HashMap::new(),
            chainlink_health: OracleHealthState::new(),
            binance_health: OracleHealthState::new(),
        }
    }

    /// Update a price for the given oracle and symbol
    /// Also updates the health state for that oracle
    pub fn update_price(&mut self, oracle: OracleType, symbol: &str, value: f64, timestamp: u64) {
        let entry = PriceEntry::new(value, timestamp);
        let (prices, health) = match oracle {
            OracleType::ChainLink => (&mut self.chainlink, &mut self.chainlink_health),
            OracleType::Binance => (&mut self.binance, &mut self.binance_health),
        };
        prices.insert(symbol.to_uppercase(), entry);
        health.record_update();
    }

    /// Check if a specific oracle has received data recently
    pub fn is_oracle_healthy(&self, oracle: OracleType, max_age: Duration) -> bool {
        let health = match oracle {
            OracleType::ChainLink => &self.chainlink_health,
            OracleType::Binance => &self.binance_health,
        };
        health.time_since_update() < max_age
    }

    /// Get age of last update for a specific oracle
    pub fn oracle_age(&self, oracle: OracleType) -> Duration {
        match oracle {
            OracleType::ChainLink => self.chainlink_health.time_since_update(),
            OracleType::Binance => self.binance_health.time_since_update(),
        }
    }

    /// Get the message count for a specific oracle
    pub fn oracle_message_count(&self, oracle: OracleType) -> u64 {
        match oracle {
            OracleType::ChainLink => self.chainlink_health.message_count,
            OracleType::Binance => self.binance_health.message_count,
        }
    }

    /// Reset health state for a specific oracle (call after reconnection)
    pub fn reset_oracle_health(&mut self, oracle: OracleType) {
        match oracle {
            OracleType::ChainLink => self.chainlink_health.reset(),
            OracleType::Binance => self.binance_health.reset(),
        }
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
