//! Orderbook domain entities
//!
//! High-performance orderbook data structure optimized for speed with:
//! - Integer price representation (micros) for fast comparison
//! - Sorted Vec for cache-friendly access
//! - Binary search for O(log n) updates

use serde::{Deserialize, Serialize};

// =============================================================================
// Price Level - Basic unit of orderbook
// =============================================================================

/// Price level in order book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: String,  // String to avoid float precision issues
    pub size: String,   // String to avoid float precision issues
}

impl PriceLevel {
    pub fn price_f64(&self) -> f64 {
        self.price.parse().unwrap_or(0.0)
    }

    pub fn size_f64(&self) -> f64 {
        self.size.parse().unwrap_or(0.0)
    }
}

// =============================================================================
// Price Conversion Utilities
// =============================================================================

/// Convert string price (e.g., "0.75") to integer micros (750000)
/// Uses 6 decimal places for precision
#[inline]
pub fn price_to_micros(price: &str) -> u64 {
    (price.parse::<f64>().unwrap_or(0.0) * 1_000_000.0) as u64
}

/// Convert integer micros back to f64 for display
#[inline]
pub fn micros_to_f64(micros: u64) -> f64 {
    micros as f64 / 1_000_000.0
}

// =============================================================================
// OrderbookSide - One side of the orderbook (bids or asks)
// =============================================================================

/// A single side of the orderbook (bids or asks)
/// Uses sorted Vec for cache-friendly access with small N (~20-100 levels)
#[derive(Debug, Clone)]
pub struct OrderbookSide {
    /// Price levels as (price_micros, size_micros)
    /// Bids: sorted descending (highest first)
    /// Asks: sorted ascending (lowest first)
    levels: Vec<(u64, u64)>,
    /// True for bids (descending), false for asks (ascending)
    is_bid: bool,
}

impl OrderbookSide {
    /// Create a new empty orderbook side
    pub fn new(is_bid: bool) -> Self {
        Self {
            levels: Vec::with_capacity(64), // Pre-allocate for typical depth
            is_bid,
        }
    }

    /// Replace entire side with snapshot data
    pub fn process_snapshot(&mut self, levels: &[PriceLevel]) {
        self.levels.clear();
        self.levels.reserve(levels.len());

        for level in levels {
            let price = price_to_micros(&level.price);
            let size = price_to_micros(&level.size);
            if size > 0 {
                self.levels.push((price, size));
            }
        }

        // Sort based on side type
        if self.is_bid {
            // Bids: highest price first (descending)
            self.levels.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        } else {
            // Asks: lowest price first (ascending)
            self.levels.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        }
    }

    /// Update a single price level
    /// If size == 0, remove the level
    pub fn process_update(&mut self, price: u64, size: u64) {
        // Binary search for the price
        let search_result = self.levels.binary_search_by(|(p, _)| {
            if self.is_bid {
                // Descending order for bids
                p.cmp(&price).reverse()
            } else {
                // Ascending order for asks
                p.cmp(&price)
            }
        });

        match search_result {
            Ok(idx) => {
                // Price exists
                if size == 0 {
                    self.levels.remove(idx);
                } else {
                    self.levels[idx].1 = size;
                }
            }
            Err(idx) => {
                // Price doesn't exist
                if size > 0 {
                    self.levels.insert(idx, (price, size));
                }
            }
        }
    }

    /// Get best price level (first element)
    #[inline]
    pub fn best(&self) -> Option<(u64, u64)> {
        self.levels.first().copied()
    }

    /// Get all levels as slice
    #[inline]
    pub fn levels(&self) -> &[(u64, u64)] {
        &self.levels
    }

    /// Get number of price levels
    #[inline]
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// Check if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// Get total liquidity (sum of all sizes)
    pub fn total_liquidity(&self) -> u64 {
        self.levels.iter().map(|(_, s)| s).sum()
    }
}

// =============================================================================
// Orderbook - Complete orderbook for one asset
// =============================================================================

/// Complete orderbook for one asset (Yes or No outcome)
#[derive(Debug, Clone)]
pub struct Orderbook {
    pub asset_id: String,
    pub bids: OrderbookSide,
    pub asks: OrderbookSide,
}

impl Orderbook {
    /// Create a new empty orderbook
    pub fn new(asset_id: String) -> Self {
        Self {
            asset_id,
            bids: OrderbookSide::new(true),
            asks: OrderbookSide::new(false),
        }
    }

    /// Process a full orderbook snapshot
    pub fn process_snapshot(&mut self, bids: &[PriceLevel], asks: &[PriceLevel]) {
        self.bids.process_snapshot(bids);
        self.asks.process_snapshot(asks);
    }

    /// Process a price update
    /// side: "BUY" or "SELL" (or "buy"/"sell")
    pub fn process_update(&mut self, side: &str, price: &str, size: &str) {
        let price_micros = price_to_micros(price);
        let size_micros = price_to_micros(size);

        match side.to_uppercase().as_str() {
            "BUY" => self.bids.process_update(price_micros, size_micros),
            "SELL" => self.asks.process_update(price_micros, size_micros),
            _ => {} // Ignore unknown sides
        }
    }

    /// Get best bid (highest buy price)
    #[inline]
    pub fn best_bid(&self) -> Option<(u64, u64)> {
        self.bids.best()
    }

    /// Get best ask (lowest sell price)
    #[inline]
    pub fn best_ask(&self) -> Option<(u64, u64)> {
        self.asks.best()
    }

    /// Calculate spread in micros (best_ask - best_bid)
    pub fn spread(&self) -> Option<i64> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid, _)), Some((ask, _))) => Some(ask as i64 - bid as i64),
            _ => None,
        }
    }

    /// Calculate mid price in micros
    pub fn mid_price(&self) -> Option<u64> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid, _)), Some((ask, _))) => Some((bid + ask) / 2),
            _ => None,
        }
    }

    /// Format orderbook for logging
    pub fn format_summary(&self) -> String {
        let bid_str = self
            .best_bid()
            .map(|(p, s)| format!("{:.4} ({:.2})", micros_to_f64(p), micros_to_f64(s)))
            .unwrap_or_else(|| "N/A".to_string());

        let ask_str = self
            .best_ask()
            .map(|(p, s)| format!("{:.4} ({:.2})", micros_to_f64(p), micros_to_f64(s)))
            .unwrap_or_else(|| "N/A".to_string());

        let spread_str = self
            .spread()
            .map(|s| format!("{:.4}", s as f64 / 1_000_000.0))
            .unwrap_or_else(|| "N/A".to_string());

        format!(
            "Bid: {} | Ask: {} | Spread: {}",
            bid_str, ask_str, spread_str
        )
    }

    /// Format full orderbook depth for logging (top N levels)
    pub fn format_depth(&self, max_levels: usize) -> String {
        let mut output = String::new();

        // Bids
        output.push_str("  Bids: ");
        let bid_levels: Vec<String> = self
            .bids
            .levels()
            .iter()
            .take(max_levels)
            .map(|(p, s)| format!("{:.4}({:.2})", micros_to_f64(*p), micros_to_f64(*s)))
            .collect();
        output.push_str(&bid_levels.join(", "));
        if bid_levels.is_empty() {
            output.push_str("(empty)");
        }

        output.push_str("\n  Asks: ");
        let ask_levels: Vec<String> = self
            .asks
            .levels()
            .iter()
            .take(max_levels)
            .map(|(p, s)| format!("{:.4}({:.2})", micros_to_f64(*p), micros_to_f64(*s)))
            .collect();
        output.push_str(&ask_levels.join(", "));
        if ask_levels.is_empty() {
            output.push_str("(empty)");
        }

        output
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_level(price: &str, size: &str) -> PriceLevel {
        PriceLevel {
            price: price.to_string(),
            size: size.to_string(),
        }
    }

    #[test]
    fn test_price_conversion() {
        assert_eq!(price_to_micros("0.75"), 750000);
        assert_eq!(price_to_micros("1.0"), 1000000);
        assert_eq!(price_to_micros("0.123456"), 123456);
        assert_eq!(micros_to_f64(750000), 0.75);
    }

    #[test]
    fn test_orderbook_side_snapshot() {
        let mut bids = OrderbookSide::new(true);
        bids.process_snapshot(&[
            make_level("0.70", "100"),
            make_level("0.75", "200"),
            make_level("0.72", "150"),
        ]);

        // Bids should be sorted descending
        assert_eq!(bids.levels().len(), 3);
        assert_eq!(bids.best(), Some((750000, 200000000))); // 0.75 is best bid
    }

    #[test]
    fn test_orderbook_side_update() {
        let mut bids = OrderbookSide::new(true);
        bids.process_snapshot(&[
            make_level("0.75", "200"),
            make_level("0.74", "150"),
        ]);

        // Update existing level
        bids.process_update(750000, 300000000);
        assert_eq!(bids.best(), Some((750000, 300000000)));

        // Add new level
        bids.process_update(760000, 100000000);
        assert_eq!(bids.best(), Some((760000, 100000000))); // New best

        // Remove level (size = 0)
        bids.process_update(760000, 0);
        assert_eq!(bids.best(), Some((750000, 300000000)));
    }

    #[test]
    fn test_orderbook_spread() {
        let mut ob = Orderbook::new("test".to_string());
        ob.process_snapshot(
            &[make_level("0.74", "100"), make_level("0.73", "200")],
            &[make_level("0.76", "100"), make_level("0.77", "200")],
        );

        assert_eq!(ob.best_bid(), Some((740000, 100000000)));
        assert_eq!(ob.best_ask(), Some((760000, 100000000)));
        assert_eq!(ob.spread(), Some(20000)); // 0.02 spread
    }

    #[test]
    fn test_orderbook_update_sides() {
        let mut ob = Orderbook::new("test".to_string());
        ob.process_snapshot(&[make_level("0.74", "100")], &[make_level("0.76", "100")]);

        // Update bid
        ob.process_update("BUY", "0.75", "200");
        assert_eq!(ob.best_bid(), Some((750000, 200000000)));

        // Update ask
        ob.process_update("SELL", "0.755", "150");
        assert_eq!(ob.best_ask(), Some((755000, 150000000)));
    }
}
