//! Raw input types for the solver.

/// Raw snapshot of our current open orders for a single token
#[derive(Debug, Clone, Default)]
pub struct OrderSnapshot {
    /// Our open bids (BUY orders) sorted by price descending
    pub bids: Vec<OpenOrder>,
    /// Our open asks (SELL orders) sorted by price ascending
    pub asks: Vec<OpenOrder>,
}

/// A single open order from OMS
#[derive(Debug, Clone)]
pub struct OpenOrder {
    pub order_id: String,
    pub price: f64,
    pub original_size: f64,
    pub remaining_size: f64,
    /// Timestamp for queue priority (older = better position)
    pub created_at: i64,
}

impl OpenOrder {
    pub fn new(order_id: String, price: f64, original_size: f64, remaining_size: f64) -> Self {
        Self {
            order_id,
            price,
            original_size,
            remaining_size,
            created_at: 0,
        }
    }

    pub fn with_created_at(
        order_id: String,
        price: f64,
        original_size: f64,
        remaining_size: f64,
        created_at: i64,
    ) -> Self {
        Self {
            order_id,
            price,
            original_size,
            remaining_size,
            created_at,
        }
    }
}

/// Raw snapshot of our inventory for a token pair
#[derive(Debug, Clone, Default)]
pub struct InventorySnapshot {
    pub up_size: f64,
    pub up_avg_price: f64,
    pub down_size: f64,
    pub down_avg_price: f64,
}

impl InventorySnapshot {
    /// Calculate imbalance ratio: (up - down) / (up + down)
    /// Returns value between -1.0 (all down) and +1.0 (all up)
    ///
    /// IMPORTANT: Uses signed values to correctly handle short positions.
    /// - Positive size = long position (we own tokens)
    /// - Negative size = short position (we owe tokens)
    ///
    /// Example: up_size=100, down_size=-80 (short 80 DOWN)
    /// Imbalance = (100 - (-80)) / (|100| + |-80|) = 180/180 = 1.0 (heavily long UP)
    pub fn imbalance(&self) -> f64 {
        let total = self.up_size.abs() + self.down_size.abs();
        if total < 1e-9 {
            return 0.0;
        }
        // Use signed values in numerator to correctly handle shorts
        (self.up_size - self.down_size) / total
    }

    /// Combined average cost for merge profitability
    pub fn combined_avg_cost(&self) -> f64 {
        self.up_avg_price + self.down_avg_price
    }

    /// Minimum of both sides (max mergeable pairs)
    pub fn pairs_available(&self) -> f64 {
        self.up_size.abs().min(self.down_size.abs())
    }
}

/// Raw snapshot of orderbook for a single token
#[derive(Debug, Clone, Default)]
pub struct OrderbookSnapshot {
    /// Best ask (lowest sell price, size)
    pub best_ask: Option<(f64, f64)>,
    /// Best bid (highest buy price, size)
    pub best_bid: Option<(f64, f64)>,
    /// Our orders at best bid? (for taker logic - only take if NOT ours)
    pub best_bid_is_ours: bool,
    /// Our orders at best ask? (for taker logic)
    pub best_ask_is_ours: bool,
}

impl OrderbookSnapshot {
    pub fn best_ask_price(&self) -> Option<f64> {
        self.best_ask.map(|(p, _)| p)
    }

    pub fn best_bid_price(&self) -> Option<f64> {
        self.best_bid.map(|(p, _)| p)
    }

    pub fn best_ask_size(&self) -> Option<f64> {
        self.best_ask.map(|(_, s)| s)
    }

    pub fn best_bid_size(&self) -> Option<f64> {
        self.best_bid.map(|(_, s)| s)
    }

    /// Check if orderbook has valid data
    pub fn is_valid(&self) -> bool {
        self.best_ask.is_some() || self.best_bid.is_some()
    }

    /// Get the spread if both bid and ask exist
    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid_price(), self.best_ask_price()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }
}

/// Complete input for the solver - all raw types
#[derive(Debug, Clone)]
pub struct SolverInput {
    /// Token identifiers
    pub up_token_id: String,
    pub down_token_id: String,

    /// Our current open orders
    pub up_orders: OrderSnapshot,
    pub down_orders: OrderSnapshot,

    /// Our inventory
    pub inventory: InventorySnapshot,

    /// Current orderbook state
    pub up_orderbook: OrderbookSnapshot,
    pub down_orderbook: OrderbookSnapshot,

    /// Configuration
    pub config: SolverConfig,
}

/// Solver configuration parameters
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SolverConfig {
    /// Number of bid levels per side (e.g., 3)
    pub num_levels: usize,

    /// Tick size for price rounding (e.g., 0.01)
    pub tick_size: f64,

    /// Base offset from best ask when delta=0 (aggressive)
    pub base_offset: f64,

    /// Maximum imbalance before stopping quotes on overweight side
    pub max_imbalance: f64,

    /// Order size per level
    pub order_size: f64,

    /// Spread adjustment per level (cents)
    /// Level 0 = base, Level 1 = base + spread_per_level, etc.
    pub spread_per_level: f64,

    /// Multiplier for offset scaling with imbalance (e.g., 5.0)
    /// offset = base_offset * (1 + |delta| * offset_scaling)
    /// Higher = more aggressive backing off on overweight side
    pub offset_scaling: f64,

    /// Skew factor for size adjustment based on imbalance
    /// up_size = order_size * (1 - delta * skew_factor)
    /// down_size = order_size * (1 + delta * skew_factor)
    /// 0.0 = no skew, 1.0 = moderate, 2.0 = aggressive
    pub skew_factor: f64,

    /// Minimum offset from best_ask to prevent spread crossing
    /// When offsets go negative due to extreme imbalance, this floor
    /// prevents bids from crossing the ask. Must be >= tick_size (0.01).
    pub min_offset: f64,

    /// Maximum position size per side (UP/DOWN) - stops quoting when reached.
    /// Set to 0.0 for unlimited. This is a safety limit for testing.
    pub max_position: f64,

    /// Weight for profitability bid in weighted average (default: 0.3)
    /// Used to cap bid prices for maintaining avg_up + avg_down < 1.0
    pub prof_weight: f64,

    /// Weight for imbalance/market bid in weighted average (default: 0.7)
    /// Higher weight means bids stay closer to market best_bid
    pub imbalance_weight: f64,

    /// Maximum |delta| at which profitability cap is applied (default: 0.3)
    /// When |delta| > this threshold, the profitability cap is DISABLED to allow
    /// aggressive rebalancing. The offset/skew mechanisms handle imbalance instead.
    /// Set to 1.0 to always apply cap, 0.0 to never apply cap.
    pub prof_cap_delta_threshold: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            num_levels: 3,
            tick_size: 0.01,
            base_offset: 0.01,           // Aggressive when delta=0
            max_imbalance: 0.8,          // Stop quoting at 80% imbalance
            order_size: 100.0,
            spread_per_level: 1.0,       // 1 cent wider per level
            offset_scaling: 5.0,         // Scale offset 5x with imbalance
            skew_factor: 1.0,            // Moderate size skew based on delta
            min_offset: 0.01,            // Min offset = tick_size to prevent spread crossing
            max_position: 0.0,           // 0 = unlimited (set in config for testing)
            prof_weight: 0.3,            // 30% weight on profitability constraint
            imbalance_weight: 0.7,       // 70% weight on market competitiveness
            prof_cap_delta_threshold: 0.15, // Only apply cap when |delta| <= 0.15
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_imbalance_balanced() {
        let inv = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.47,
        };
        assert!((inv.imbalance() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_imbalance_heavy_up() {
        let inv = InventorySnapshot {
            up_size: 80.0,
            up_avg_price: 0.52,
            down_size: 20.0,
            down_avg_price: 0.47,
        };
        assert!((inv.imbalance() - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_imbalance_heavy_down() {
        let inv = InventorySnapshot {
            up_size: 20.0,
            up_avg_price: 0.52,
            down_size: 80.0,
            down_avg_price: 0.47,
        };
        assert!((inv.imbalance() - (-0.6)).abs() < 0.001);
    }

    #[test]
    fn test_imbalance_empty() {
        let inv = InventorySnapshot::default();
        assert!((inv.imbalance() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_imbalance_with_short_position() {
        // Long 100 UP, Short 80 DOWN (negative = short/owe tokens)
        // This is heavily imbalanced toward UP
        // Imbalance = (100 - (-80)) / (|100| + |-80|) = 180 / 180 = 1.0
        let inv = InventorySnapshot {
            up_size: 100.0,
            up_avg_price: 0.55,
            down_size: -80.0, // SHORT position
            down_avg_price: 0.45,
        };
        assert!((inv.imbalance() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_imbalance_with_both_shorts() {
        // Short 50 UP, Short 50 DOWN
        // Imbalance = (-50 - (-50)) / (50 + 50) = 0 / 100 = 0.0
        let inv = InventorySnapshot {
            up_size: -50.0, // SHORT
            up_avg_price: 0.55,
            down_size: -50.0, // SHORT
            down_avg_price: 0.45,
        };
        assert!((inv.imbalance() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_imbalance_long_down_short_up() {
        // Short 30 UP, Long 70 DOWN
        // Imbalance = (-30 - 70) / (30 + 70) = -100 / 100 = -1.0
        let inv = InventorySnapshot {
            up_size: -30.0, // SHORT
            up_avg_price: 0.55,
            down_size: 70.0, // LONG
            down_avg_price: 0.45,
        };
        assert!((inv.imbalance() - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_combined_avg_cost() {
        let inv = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };
        assert!((inv.combined_avg_cost() - 0.98).abs() < 0.001);
    }
}
