//! Orderbook domain entities
//!
//! Simple orderbook data structure using floats for readability.

use serde::{Deserialize, Serialize};

// =============================================================================
// Price Level - Basic unit of orderbook
// =============================================================================

/// Price level in order book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: String,
    pub size: String,
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
// OrderbookSide - One side of the orderbook (bids or asks)
// =============================================================================

/// A single side of the orderbook (bids or asks)
#[derive(Debug, Clone)]
pub struct OrderbookSide {
    /// Price levels as (price, size)
    /// Bids: sorted descending (highest first)
    /// Asks: sorted ascending (lowest first)
    levels: Vec<(f64, f64)>,
    /// True for bids (descending), false for asks (ascending)
    is_bid: bool,
}

impl OrderbookSide {
    /// Create a new empty orderbook side
    pub fn new(is_bid: bool) -> Self {
        Self {
            levels: Vec::with_capacity(64),
            is_bid,
        }
    }

    /// Replace entire side with snapshot data
    pub fn process_snapshot(&mut self, levels: &[PriceLevel]) {
        self.levels.clear();
        self.levels.reserve(levels.len());

        for level in levels {
            let price = level.price_f64();
            let size = level.size_f64();
            if size > 0.0 {
                self.levels.push((price, size));
            }
        }

        // Sort based on side type
        if self.is_bid {
            // Bids: highest price first (descending)
            self.levels
                .sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        } else {
            // Asks: lowest price first (ascending)
            self.levels
                .sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        }
    }

    /// Update a single price level
    /// If size == 0, remove the level
    pub fn process_update(&mut self, price: f64, size: f64) {
        // Use wider tolerance for floating-point comparison (prices are typically 0.01-0.99)
        const PRICE_TOLERANCE: f64 = 1e-6;

        // For removals (size=0), search entire orderbook to find and remove the price
        if size == 0.0 {
            if let Some(idx) = self.levels.iter().position(|(p, _)| (p - price).abs() < PRICE_TOLERANCE) {
                self.levels.remove(idx);
            }
            return;
        }

        // For additions/updates, find correct position
        let pos = self.levels.iter().position(|(p, _)| {
            if self.is_bid {
                *p <= price
            } else {
                *p >= price
            }
        });

        match pos {
            Some(idx) if (self.levels[idx].0 - price).abs() < PRICE_TOLERANCE => {
                // Price exists at idx - update size
                self.levels[idx].1 = size;
            }
            Some(idx) => {
                // Price doesn't exist, insert at idx
                self.levels.insert(idx, (price, size));
            }
            None => {
                // Insert at end
                self.levels.push((price, size));
            }
        }
    }

    /// Get best price level (first element)
    #[inline]
    pub fn best(&self) -> Option<(f64, f64)> {
        self.levels.first().copied()
    }

    /// Get all levels as slice
    #[inline]
    pub fn levels(&self) -> &[(f64, f64)] {
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
    pub fn total_liquidity(&self) -> f64 {
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
        let price_f64 = price.parse().unwrap_or(0.0);
        let size_f64 = size.parse().unwrap_or(0.0);

        match side.to_uppercase().as_str() {
            "BUY" => self.bids.process_update(price_f64, size_f64),
            "SELL" => self.asks.process_update(price_f64, size_f64),
            _ => {}
        }
    }

    /// Get best bid (highest buy price)
    #[inline]
    pub fn best_bid(&self) -> Option<(f64, f64)> {
        self.bids.best()
    }

    /// Get best ask (lowest sell price)
    #[inline]
    pub fn best_ask(&self) -> Option<(f64, f64)> {
        self.asks.best()
    }

    /// Calculate spread (best_ask - best_bid)
    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid, _)), Some((ask, _))) => Some(ask - bid),
            _ => None,
        }
    }

    /// Calculate mid price
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some((bid, _)), Some((ask, _))) => Some((bid + ask) / 2.0),
            _ => None,
        }
    }

    /// Format orderbook for logging
    pub fn format_summary(&self) -> String {
        let bid_str = self
            .best_bid()
            .map(|(p, s)| format!("{:.4} ({:.2})", p, s))
            .unwrap_or_else(|| "N/A".to_string());

        let ask_str = self
            .best_ask()
            .map(|(p, s)| format!("{:.4} ({:.2})", p, s))
            .unwrap_or_else(|| "N/A".to_string());

        let spread_str = self
            .spread()
            .map(|s| format!("{:.4}", s))
            .unwrap_or_else(|| "N/A".to_string());

        format!(
            "Bid: {} | Ask: {} | Spread: {}",
            bid_str, ask_str, spread_str
        )
    }

    /// Format full orderbook depth for logging (top N levels)
    pub fn format_depth(&self, max_levels: usize) -> String {
        let mut output = String::new();

        output.push_str("  Bids: ");
        let bid_levels: Vec<String> = self
            .bids
            .levels()
            .iter()
            .take(max_levels)
            .map(|(p, s)| format!("{:.4}({:.2})", p, s))
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
            .map(|(p, s)| format!("{:.4}({:.2})", p, s))
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

    const TEST_TOLERANCE: f64 = 1e-6;

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
        let best = bids.best().unwrap();
        assert!((best.0 - 0.75).abs() < TEST_TOLERANCE);
        assert!((best.1 - 200.0).abs() < TEST_TOLERANCE);
    }

    #[test]
    fn test_orderbook_side_update() {
        let mut bids = OrderbookSide::new(true);
        bids.process_snapshot(&[make_level("0.75", "200"), make_level("0.74", "150")]);

        // Update existing level
        bids.process_update(0.75, 300.0);
        let best = bids.best().unwrap();
        assert!((best.1 - 300.0).abs() < TEST_TOLERANCE);

        // Add new level
        bids.process_update(0.76, 100.0);
        let best = bids.best().unwrap();
        assert!((best.0 - 0.76).abs() < TEST_TOLERANCE);

        // Remove level (size = 0)
        bids.process_update(0.76, 0.0);
        let best = bids.best().unwrap();
        assert!((best.0 - 0.75).abs() < TEST_TOLERANCE);
    }

    #[test]
    fn test_orderbook_removal_clears_all_asks() {
        let mut asks = OrderbookSide::new(false);
        asks.process_snapshot(&[
            make_level("0.76", "100"),
            make_level("0.77", "200"),
        ]);
        assert_eq!(asks.len(), 2);

        // Remove all asks one by one
        asks.process_update(0.76, 0.0);
        assert_eq!(asks.len(), 1);

        asks.process_update(0.77, 0.0);
        assert!(asks.is_empty());
    }

    #[test]
    fn test_orderbook_spread() {
        let mut ob = Orderbook::new("test".to_string());
        ob.process_snapshot(
            &[make_level("0.74", "100"), make_level("0.73", "200")],
            &[make_level("0.76", "100"), make_level("0.77", "200")],
        );

        let best_bid = ob.best_bid().unwrap();
        let best_ask = ob.best_ask().unwrap();
        assert!((best_bid.0 - 0.74).abs() < TEST_TOLERANCE);
        assert!((best_ask.0 - 0.76).abs() < TEST_TOLERANCE);

        let spread = ob.spread().unwrap();
        assert!((spread - 0.02).abs() < TEST_TOLERANCE);
    }

    #[test]
    fn test_orderbook_update_sides() {
        let mut ob = Orderbook::new("test".to_string());
        ob.process_snapshot(&[make_level("0.74", "100")], &[make_level("0.76", "100")]);

        // Update bid
        ob.process_update("BUY", "0.75", "200");
        let best_bid = ob.best_bid().unwrap();
        assert!((best_bid.0 - 0.75).abs() < TEST_TOLERANCE);

        // Update ask
        ob.process_update("SELL", "0.755", "150");
        let best_ask = ob.best_ask().unwrap();
        assert!((best_ask.0 - 0.755).abs() < TEST_TOLERANCE);
    }
}
