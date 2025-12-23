//! Binance Price Manager
//!
//! Manages direct Binance crypto prices with latency tracking for HFT.
//! Similar to OraclePriceManager but optimized for direct Binance feed.

use super::types::BinanceAsset;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// =============================================================================
// SharedBinancePrices
// =============================================================================

/// Shared price manager accessible by handlers and consumers
pub type SharedBinancePrices = Arc<RwLock<BinancePriceManager>>;

// =============================================================================
// BinancePriceEntry
// =============================================================================

/// A single price entry with value, timestamp, latency, and local receive time
#[derive(Debug, Clone, Copy)]
pub struct BinancePriceEntry {
    /// The price value in USD
    pub value: f64,

    /// Binance event timestamp (ms since epoch)
    pub binance_timestamp: u64,

    /// Trade ID (for sequencing)
    pub trade_id: u64,

    /// When we locally received this update (for staleness detection)
    pub received_at: Instant,

    /// Latency in milliseconds (local_time - binance_event_time)
    /// Negative means our clock is behind Binance
    pub latency_ms: i64,

    /// Trade direction indicator (true = sell/maker, false = buy/taker)
    pub is_sell: bool,
}

impl BinancePriceEntry {
    /// Create a new price entry, calculating latency from Binance timestamp
    pub fn new(value: f64, binance_timestamp: u64, trade_id: u64, is_sell: bool) -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Calculate latency: positive means we're behind, negative means Binance timestamp is in the future
        let latency_ms = (now_ms as i64) - (binance_timestamp as i64);

        Self {
            value,
            binance_timestamp,
            trade_id,
            received_at: Instant::now(),
            latency_ms,
            is_sell,
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

// =============================================================================
// BinanceHealthState
// =============================================================================

/// Health state for the Binance connection
#[derive(Debug)]
pub struct BinanceHealthState {
    /// When we last received ANY update
    pub last_update: Instant,

    /// Total number of trades received
    pub trade_count: u64,

    /// Running average latency (exponential moving average)
    pub avg_latency_ms: f64,

    /// Min latency seen (useful for clock sync detection)
    pub min_latency_ms: i64,

    /// Max latency seen (useful for spike detection)
    pub max_latency_ms: i64,
}

impl Default for BinanceHealthState {
    fn default() -> Self {
        Self {
            last_update: Instant::now(),
            trade_count: 0,
            avg_latency_ms: 0.0,
            min_latency_ms: i64::MAX,
            max_latency_ms: i64::MIN,
        }
    }
}

impl BinanceHealthState {
    /// Record a new trade update
    pub fn record_update(&mut self, latency_ms: i64) {
        self.last_update = Instant::now();
        self.trade_count += 1;

        // Update min/max
        self.min_latency_ms = self.min_latency_ms.min(latency_ms);
        self.max_latency_ms = self.max_latency_ms.max(latency_ms);

        // Exponential moving average (alpha = 0.1 for smooth updates)
        let alpha = 0.1;
        self.avg_latency_ms = alpha * (latency_ms as f64) + (1.0 - alpha) * self.avg_latency_ms;
    }

    /// Time since last update
    pub fn time_since_update(&self) -> Duration {
        self.last_update.elapsed()
    }

    /// Reset health state (call after reconnection)
    pub fn reset(&mut self) {
        self.last_update = Instant::now();
        self.min_latency_ms = i64::MAX;
        self.max_latency_ms = i64::MIN;
        // Keep trade_count and avg_latency for cumulative stats
    }
}

// =============================================================================
// BinancePriceManager
// =============================================================================

/// Manages direct Binance crypto prices
#[derive(Debug)]
pub struct BinancePriceManager {
    /// Prices per symbol (symbol -> price entry)
    pub prices: HashMap<String, BinancePriceEntry>,

    /// Connection health state
    pub health: BinanceHealthState,
}

impl Default for BinancePriceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BinancePriceManager {
    /// Create a new empty price manager
    pub fn new() -> Self {
        Self {
            prices: HashMap::with_capacity(4), // BTC, ETH, SOL, XRP
            health: BinanceHealthState::default(),
        }
    }

    /// Update price for a symbol
    pub fn update_price(
        &mut self,
        symbol: &str,
        value: f64,
        binance_timestamp: u64,
        trade_id: u64,
        is_sell: bool,
    ) {
        let entry = BinancePriceEntry::new(value, binance_timestamp, trade_id, is_sell);
        self.health.record_update(entry.latency_ms);
        self.prices.insert(symbol.to_uppercase(), entry);
    }

    /// Get price for a symbol
    pub fn get_price(&self, symbol: &str) -> Option<BinancePriceEntry> {
        self.prices.get(&symbol.to_uppercase()).copied()
    }

    /// Get price by asset enum
    pub fn get_price_by_asset(&self, asset: BinanceAsset) -> Option<BinancePriceEntry> {
        self.get_price(asset.symbol())
    }

    /// Check if connection is healthy (received data recently)
    pub fn is_healthy(&self, max_age: Duration) -> bool {
        self.health.time_since_update() < max_age
    }

    /// Get time since last update
    pub fn age(&self) -> Duration {
        self.health.time_since_update()
    }

    /// Get average latency
    pub fn avg_latency_ms(&self) -> f64 {
        self.health.avg_latency_ms
    }

    /// Get min latency
    pub fn min_latency_ms(&self) -> i64 {
        self.health.min_latency_ms
    }

    /// Get max latency
    pub fn max_latency_ms(&self) -> i64 {
        self.health.max_latency_ms
    }

    /// Get trade count
    pub fn trade_count(&self) -> u64 {
        self.health.trade_count
    }

    /// Reset health state (call after reconnection)
    pub fn reset_health(&mut self) {
        self.health.reset();
    }

    /// Get all current prices
    pub fn get_all_prices(&self) -> &HashMap<String, BinancePriceEntry> {
        &self.prices
    }

    /// Get number of tracked symbols
    pub fn symbol_count(&self) -> usize {
        self.prices.len()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_price_manager() {
        let manager = BinancePriceManager::new();
        assert_eq!(manager.symbol_count(), 0);
        assert_eq!(manager.trade_count(), 0);
    }

    #[test]
    fn test_update_and_get_price() {
        let mut manager = BinancePriceManager::new();

        // Simulate receiving a trade
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        manager.update_price("BTC", 100000.50, now_ms, 12345, false);

        let price = manager.get_price("BTC").unwrap();
        assert!((price.value - 100000.50).abs() < 0.001);
        assert_eq!(price.trade_id, 12345);
        assert!(!price.is_sell);
        assert_eq!(manager.trade_count(), 1);
    }

    #[test]
    fn test_get_price_by_asset() {
        let mut manager = BinancePriceManager::new();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        manager.update_price("ETH", 3500.00, now_ms, 1, false);

        let price = manager.get_price_by_asset(BinanceAsset::ETH).unwrap();
        assert!((price.value - 3500.00).abs() < 0.001);
    }

    #[test]
    fn test_case_insensitive_lookup() {
        let mut manager = BinancePriceManager::new();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        manager.update_price("xrp", 2.50, now_ms, 1, true);

        assert!(manager.get_price("XRP").is_some());
        assert!(manager.get_price("xrp").is_some());
        assert!(manager.get_price("Xrp").is_some());
    }

    #[test]
    fn test_latency_tracking() {
        let mut manager = BinancePriceManager::new();

        // Simulate a trade with known timestamp (100ms ago)
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let binance_ts = now_ms.saturating_sub(100);

        manager.update_price("SOL", 150.0, binance_ts, 1, false);

        // Latency should be approximately 100ms
        // Allow wide variance because system clock and test timing can vary
        let latency = manager.avg_latency_ms();
        // Just verify it's positive (we simulated a past timestamp)
        // and not absurdly high (< 1 second)
        assert!(latency >= 0.0 && latency < 1000.0, "latency was {}", latency);
    }

    #[test]
    fn test_health_tracking() {
        let mut manager = BinancePriceManager::new();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Add multiple updates
        manager.update_price("BTC", 100000.0, now_ms, 1, false);
        manager.update_price("ETH", 3500.0, now_ms, 2, true);
        manager.update_price("SOL", 150.0, now_ms, 3, false);

        assert_eq!(manager.trade_count(), 3);
        assert!(manager.is_healthy(Duration::from_secs(1)));
    }

    #[test]
    fn test_price_entry_age() {
        let entry = BinancePriceEntry::new(100000.0, 0, 1, false);

        // Age should be very small (just created)
        assert!(entry.age() < Duration::from_millis(100));
        assert!(!entry.is_stale(Duration::from_secs(1)));
    }

    #[test]
    fn test_health_reset() {
        let mut manager = BinancePriceManager::new();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        manager.update_price("BTC", 100000.0, now_ms.saturating_sub(50), 1, false);
        manager.update_price("BTC", 100001.0, now_ms.saturating_sub(30), 2, false);

        let min_before = manager.min_latency_ms();
        let max_before = manager.max_latency_ms();

        manager.reset_health();

        // After reset, min/max should be reset
        assert_eq!(manager.min_latency_ms(), i64::MAX);
        assert_eq!(manager.max_latency_ms(), i64::MIN);
        // But trade count and avg should be preserved
        assert_eq!(manager.trade_count(), 2);

        // Verify values were different before
        assert_ne!(min_before, i64::MAX);
        assert_ne!(max_before, i64::MIN);
    }

    #[test]
    fn test_get_all_prices() {
        let mut manager = BinancePriceManager::new();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        manager.update_price("BTC", 100000.0, now_ms, 1, false);
        manager.update_price("ETH", 3500.0, now_ms, 2, false);

        let all = manager.get_all_prices();
        assert_eq!(all.len(), 2);
        assert!(all.contains_key("BTC"));
        assert!(all.contains_key("ETH"));
    }
}
