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

    /// Oracle distance from threshold: (oracle_price - threshold) / threshold
    /// Positive = above threshold (UP favored), negative = below (DOWN favored)
    pub oracle_distance_pct: f64,

    /// Minutes remaining until market resolution
    pub minutes_to_resolution: f64,
}

/// Solver configuration parameters for 4-layer quoter
///
/// Implements O'Hara Market Microstructure theory with:
/// - Layer 1: Oracle-adjusted offset
/// - Layer 2: Adverse selection (Glosten-Milgrom)
/// - Layer 3: Inventory skew
/// - Layer 4: Edge check
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SolverConfig {
    // ═══════════════════════════════════════════════════════════════
    // GENERAL PARAMETERS
    // ═══════════════════════════════════════════════════════════════

    /// Number of bid levels per side (e.g., 3)
    pub num_levels: usize,

    /// Tick size for price rounding (e.g., 0.01)
    pub tick_size: f64,

    /// Base order size when inventory is balanced
    pub order_size: f64,

    /// Spread adjustment per level (cents)
    /// Level 0 = base, Level 1 = base + spread_per_level, etc.
    pub spread_per_level: f64,

    /// Minimum offset from best_bid to prevent crossing spread
    pub min_offset: f64,

    /// Maximum position size per side (UP/DOWN) - stops quoting when reached.
    /// Set to 0.0 for unlimited.
    pub max_position: f64,

    /// Maximum imbalance before stopping quotes on overweight side
    pub max_imbalance: f64,

    /// Maximum inventory delta (absolute difference between UP and DOWN tokens)
    /// When |up_qty - down_qty| exceeds this, stop quoting the overweight side
    /// This is a HARD LIMIT on directional exposure. Set to 0.0 for unlimited.
    pub max_delta: f64,

    // ═══════════════════════════════════════════════════════════════
    // DEFENSIVE LAYERS
    // ═══════════════════════════════════════════════════════════════

    /// Maximum combined average cost (quote_price + other_side_avg).
    /// Block quotes that would exceed this ceiling.
    /// Set to 0.0 to disable. Default: 0.93 (7% minimum margin)
    pub max_combined_avg: f64,

    /// Enable profitable imbalance check.
    /// When imbalanced, ensure the overweight side is profitable if it wins.
    /// Default: true
    pub profitable_imbalance_check: bool,

    /// Minimum minutes to resolution before stopping all quotes.
    /// In final minutes, adverse selection is extreme - stop quoting entirely.
    /// Set to 0.0 to disable. Default: 4.0 minutes
    pub min_minutes_to_quote: f64,

    // ═══════════════════════════════════════════════════════════════
    // LAYER 1: ORACLE-ADJUSTED OFFSET
    // ═══════════════════════════════════════════════════════════════

    /// Sensitivity to oracle price distance from threshold
    /// Higher = more aggressive adjustment based on oracle
    /// Formula: oracle_adj = oracle_distance_pct * oracle_sensitivity
    pub oracle_sensitivity: f64,

    // ═══════════════════════════════════════════════════════════════
    // LAYER 2: ADVERSE SELECTION (Glosten-Milgrom)
    // ═══════════════════════════════════════════════════════════════

    /// Base spread before adverse selection adjustment (e.g., 0.02 = 2c)
    pub base_spread: f64,

    /// Base probability of informed trader (e.g., 0.2 = 20%)
    /// Used in: p_informed = p_informed_base * exp(-minutes / time_decay)
    pub p_informed_base: f64,

    /// Time decay constant in minutes for informed trader probability
    /// Lower = faster increase in p_informed near resolution
    pub time_decay_minutes: f64,

    // ═══════════════════════════════════════════════════════════════
    // LAYER 3: INVENTORY SKEW
    // ═══════════════════════════════════════════════════════════════

    /// Inventory skew sensitivity for offset multiplier
    /// Formula: spread_mult = 1 + gamma_inv * q
    /// Higher = more aggressive offset adjustment with inventory
    pub gamma_inv: f64,

    /// Inventory skew sensitivity for size decay
    /// Formula: size = base_size * exp(-lambda_size * q)
    /// Higher = more aggressive size reduction when overweight
    pub lambda_size: f64,

    // ═══════════════════════════════════════════════════════════════
    // LAYER 4: EDGE CHECK
    // ═══════════════════════════════════════════════════════════════

    /// Minimum edge (ask - bid) required to place a quote
    /// Skip quoting if edge < edge_threshold
    pub edge_threshold: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            // General
            num_levels: 3,
            tick_size: 0.01,
            order_size: 50.0,            // Base size (from notebook)
            spread_per_level: 1.0,       // 1 cent wider per level
            min_offset: 0.01,            // Min 1c offset
            max_position: 0.0,           // 0 = unlimited
            max_imbalance: 0.8,          // Stop at 80% imbalance
            max_delta: 30.0,             // Stop quoting overweight side at 30 token delta

            // Defensive Layers
            max_combined_avg: 0.93,          // Block if quote_price + other_avg > 93%
            profitable_imbalance_check: true, // Enable profitable imbalance check
            min_minutes_to_quote: 4.0,       // Stop quoting in final 4 minutes

            // Layer 1: Oracle
            oracle_sensitivity: 5.0,     // 5x multiplier on oracle distance %

            // Layer 2: Adverse Selection
            base_spread: 0.02,           // 2c base spread
            p_informed_base: 0.2,        // 20% base informed probability
            time_decay_minutes: 5.0,     // 5 minute time constant

            // Layer 3: Inventory Skew
            gamma_inv: 1.5,              // Offset multiplier sensitivity
            lambda_size: 1.5,            // Size decay sensitivity

            // Layer 4: Edge Check
            edge_threshold: 0.01,        // Minimum 1c edge required
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
