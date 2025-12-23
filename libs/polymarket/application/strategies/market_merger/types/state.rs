//! Market state for the Market Merger strategy

use std::collections::HashMap;
use std::time::Instant;

/// Mutable state for market making on a single market
#[derive(Debug)]
pub struct MarketState {
    // === Position Tracking ===
    /// Current Up token position size
    pub up_size: f64,
    /// Average cost of Up tokens
    pub up_avg_cost: f64,
    /// Current Down token position size
    pub down_size: f64,
    /// Average cost of Down tokens
    pub down_avg_cost: f64,

    // === Active Bids (only bids, no asks) ===
    /// Active bids on Up token: level -> bid info
    pub up_bids: HashMap<u8, BidInfo>,
    /// Active bids on Down token: level -> bid info
    pub down_bids: HashMap<u8, BidInfo>,

    // === Sizing Phase ===
    /// Current sizing phase
    pub phase: SizingPhase,

    // === Metrics ===
    /// Total fills received
    pub fill_count: u64,
    /// Total pairs merged
    pub merged_pairs: u64,
    /// Realized profit from merges
    pub realized_profit: f64,
}

/// Information about a placed bid
#[derive(Debug, Clone)]
pub struct BidInfo {
    /// Order ID from Polymarket
    pub order_id: String,
    /// Bid price
    pub price: f64,
    /// Bid size (in tokens)
    pub size: f64,
    /// When the bid was placed
    pub placed_at: Instant,
    /// Bid level (0, 1, 2)
    pub level: u8,
}

/// Sizing phase determines how much capital to deploy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingPhase {
    /// Position < $100, use 1% sizes (conservative)
    Bootstrap,
    /// $100-500, use 3% sizes (confirmed profitability)
    Confirmed,
    /// > $500, use 5% sizes (scaled up)
    Scaled,
}

impl Default for MarketState {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketState {
    /// Create a new empty state
    pub fn new() -> Self {
        Self {
            up_size: 0.0,
            up_avg_cost: 0.0,
            down_size: 0.0,
            down_avg_cost: 0.0,
            up_bids: HashMap::new(),
            down_bids: HashMap::new(),
            phase: SizingPhase::Bootstrap,
            fill_count: 0,
            merged_pairs: 0,
            realized_profit: 0.0,
        }
    }

    /// Combined cost of Up + Down (must be < $1.00 to be profitable)
    pub fn combined_cost(&self) -> f64 {
        self.up_avg_cost + self.down_avg_cost
    }

    /// Check if current positions are profitable when merged
    pub fn is_profitable(&self) -> bool {
        self.combined_cost() < 1.0
    }

    /// Number of pairs that can be merged (limited by smaller position)
    pub fn mergeable_pairs(&self) -> f64 {
        self.up_size.min(self.down_size)
    }

    /// Calculate position imbalance as a ratio (0.0 = balanced, 1.0 = fully one-sided)
    pub fn imbalance(&self) -> f64 {
        let total = self.up_size + self.down_size;
        if total == 0.0 {
            return 0.0;
        }
        (self.up_size - self.down_size).abs() / total
    }

    /// Calculate the Up/Down ratio (0.5 = balanced)
    pub fn up_ratio(&self) -> f64 {
        let total = self.up_size + self.down_size;
        if total == 0.0 {
            return 0.5;
        }
        self.up_size / total
    }

    /// Get the best (highest) Up bid price
    pub fn best_up_bid_price(&self) -> Option<f64> {
        self.up_bids.values().map(|b| b.price).reduce(f64::max)
    }

    /// Get the best (highest) Down bid price
    pub fn best_down_bid_price(&self) -> Option<f64> {
        self.down_bids.values().map(|b| b.price).reduce(f64::max)
    }

    /// Total position value in USD
    pub fn total_position_value(&self) -> f64 {
        (self.up_size * self.up_avg_cost) + (self.down_size * self.down_avg_cost)
    }

    /// Apply a fill to update position
    pub fn apply_fill(&mut self, _token_id: &str, is_up: bool, price: f64, size: f64) {
        if is_up {
            self.apply_up_fill(price, size);
        } else {
            self.apply_down_fill(price, size);
        }
        self.fill_count += 1;
    }

    /// Apply a fill on the Up token
    fn apply_up_fill(&mut self, price: f64, size: f64) {
        if self.up_size == 0.0 {
            self.up_avg_cost = price;
            self.up_size = size;
        } else {
            // Volume-weighted average
            let total_cost = (self.up_size * self.up_avg_cost) + (size * price);
            self.up_size += size;
            self.up_avg_cost = total_cost / self.up_size;
        }
    }

    /// Apply a fill on the Down token
    fn apply_down_fill(&mut self, price: f64, size: f64) {
        if self.down_size == 0.0 {
            self.down_avg_cost = price;
            self.down_size = size;
        } else {
            // Volume-weighted average
            let total_cost = (self.down_size * self.down_avg_cost) + (size * price);
            self.down_size += size;
            self.down_avg_cost = total_cost / self.down_size;
        }
    }

    /// Record a merge operation
    pub fn record_merge(&mut self, pairs: u64, profit: f64) {
        self.merged_pairs += pairs;
        self.realized_profit += profit;

        // Reduce positions by merged amount
        let pairs_f64 = pairs as f64;
        self.up_size -= pairs_f64;
        self.down_size -= pairs_f64;

        // Clamp to zero
        self.up_size = self.up_size.max(0.0);
        self.down_size = self.down_size.max(0.0);
    }

    /// Clear all bids from state
    pub fn clear_bids(&mut self) {
        self.up_bids.clear();
        self.down_bids.clear();
    }

    /// Hydrate state from API data
    pub fn hydrate(&mut self, up_size: f64, up_avg: f64, down_size: f64, down_avg: f64) {
        self.up_size = up_size;
        self.up_avg_cost = up_avg;
        self.down_size = down_size;
        self.down_avg_cost = down_avg;
    }
}

impl BidInfo {
    /// Create a new bid info
    pub fn new(order_id: String, price: f64, size: f64, level: u8) -> Self {
        Self {
            order_id,
            price,
            size,
            placed_at: Instant::now(),
            level,
        }
    }

    /// Check if this bid is stale (older than given seconds)
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        self.placed_at.elapsed().as_secs() > max_age_secs
    }
}

impl std::fmt::Display for SizingPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SizingPhase::Bootstrap => write!(f, "Bootstrap (1%)"),
            SizingPhase::Confirmed => write!(f, "Confirmed (3%)"),
            SizingPhase::Scaled => write!(f, "Scaled (5%)"),
        }
    }
}
