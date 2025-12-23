//! Position Tracker - Real-time Position Management for Market Making
//!
//! Provides:
//! - Real-time position updates from WebSocket fills
//! - Cost basis and average entry price tracking
//! - Realized and unrealized PnL calculation
//! - Merge/split awareness for Up/Down token pairs
//! - Optional REST hydration on startup
//! - Callback system for position change notifications
//!
//! ## Usage
//!
//! ```ignore
//! use polymarket::infrastructure::client::user::*;
//!
//! // Create position tracker with callback
//! let tracker = Arc::new(RwLock::new(
//!     PositionTracker::with_callback(Arc::new(MyPositionHandler))
//! ));
//!
//! // Register token pairs for merge detection
//! tracker.write().register_token_pair("yes_token", "no_token", "condition_123");
//!
//! // Create bridge to receive fills from OrderStateStore
//! let bridge = Arc::new(PositionTrackerBridge::new(tracker.clone()));
//!
//! // Query positions
//! let pos = tracker.read().get_position("yes_token");
//! let merges = tracker.read().get_merge_opportunities();
//! ```

use super::order_manager::{Fill, Order, OrderEventCallback, Side, TokenPairRegistry};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Epsilon for floating point comparisons
const POSITION_EPSILON: f64 = 1e-9;

// =============================================================================
// Position
// =============================================================================

/// Represents a position in a single token
#[derive(Debug, Clone)]
pub struct Position {
    /// Token ID (asset_id)
    pub token_id: String,
    /// Net position size (positive = long, negative = short)
    pub size: f64,
    /// Volume-weighted average entry price
    pub avg_entry_price: f64,
    /// Total cost basis (size * avg_price for longs)
    pub cost_basis: f64,
    /// Realized P&L from closed positions
    pub realized_pnl: f64,
    /// Cumulative buy volume
    pub total_bought: f64,
    /// Cumulative sell volume
    pub total_sold: f64,
    /// Cumulative fees paid
    pub total_fees: f64,
    /// Number of fills processed
    pub fill_count: u64,
    /// Timestamp of last update
    pub last_fill_time: String,
}

impl Position {
    /// Create a new empty position
    pub fn new(token_id: String) -> Self {
        Self {
            token_id,
            size: 0.0,
            avg_entry_price: 0.0,
            cost_basis: 0.0,
            realized_pnl: 0.0,
            total_bought: 0.0,
            total_sold: 0.0,
            total_fees: 0.0,
            fill_count: 0,
            last_fill_time: String::new(),
        }
    }

    /// Check if position is flat (no exposure)
    pub fn is_flat(&self) -> bool {
        self.size.abs() < POSITION_EPSILON
    }

    /// Check if position is long
    pub fn is_long(&self) -> bool {
        self.size > POSITION_EPSILON
    }

    /// Check if position is short
    pub fn is_short(&self) -> bool {
        self.size < -POSITION_EPSILON
    }

    /// Calculate unrealized P&L at a given mark price
    pub fn unrealized_pnl(&self, mark_price: f64) -> f64 {
        if self.is_flat() {
            return 0.0;
        }

        if self.is_long() {
            // Long: profit when price goes up
            self.size * (mark_price - self.avg_entry_price)
        } else {
            // Short: profit when price goes down
            self.size.abs() * (self.avg_entry_price - mark_price)
        }
    }

    /// Calculate total P&L (realized + unrealized)
    pub fn total_pnl(&self, mark_price: f64) -> f64 {
        self.realized_pnl + self.unrealized_pnl(mark_price)
    }
}

// =============================================================================
// MergeOpportunity
// =============================================================================

/// Represents an opportunity to merge Up/Down tokens for USDC
#[derive(Debug, Clone)]
pub struct MergeOpportunity {
    /// First token ID (e.g., YES token)
    pub token_a: String,
    /// Second token ID (e.g., NO token)
    pub token_b: String,
    /// Market/condition ID
    pub condition_id: String,
    /// Number of pairs that can be merged (min of both positions)
    pub mergeable_pairs: f64,
    /// Value received from merging (mergeable_pairs * $1.00)
    pub merge_value: f64,
    /// Total cost basis of the mergeable portion
    pub total_cost: f64,
    /// Potential profit from merging (merge_value - total_cost)
    pub potential_profit: f64,
    /// Average cost per pair
    pub avg_cost_per_pair: f64,
}

impl MergeOpportunity {
    /// Check if this merge would be profitable
    pub fn is_profitable(&self) -> bool {
        self.potential_profit > 0.0
    }

    /// Calculate profit percentage
    pub fn profit_percentage(&self) -> f64 {
        if self.total_cost > 0.0 {
            (self.potential_profit / self.total_cost) * 100.0
        } else {
            0.0
        }
    }
}

// =============================================================================
// PositionEvent
// =============================================================================

/// Events emitted by the position tracker
#[derive(Debug, Clone)]
pub enum PositionEvent {
    /// Position was updated due to a fill
    Updated {
        token_id: String,
        old_position: Option<Position>,
        new_position: Position,
        fill: Fill,
    },
    /// A merge opportunity was detected
    MergeOpportunity(MergeOpportunity),
}

// =============================================================================
// PositionEventCallback
// =============================================================================

/// Callback trait for position events
///
/// ## Important
/// - Callbacks are fired synchronously, keep them fast
/// - Avoid acquiring locks on SharedPositionTracker within callbacks (deadlock risk)
/// - For expensive operations, queue work to a background task
pub trait PositionEventCallback: Send + Sync {
    /// Called when a position is updated
    fn on_position_updated(&self, event: &PositionEvent);
}

/// No-op callback implementation for when callbacks aren't needed
pub struct NoOpPositionCallback;

impl PositionEventCallback for NoOpPositionCallback {
    fn on_position_updated(&self, _: &PositionEvent) {}
}

// =============================================================================
// PositionTracker
// =============================================================================

/// Real-time position tracker for market making
pub struct PositionTracker {
    /// Positions by token_id
    positions: HashMap<String, Position>,
    /// Token pair registry for merge detection
    token_pairs: TokenPairRegistry,
    /// Callback for position events
    callback: Arc<dyn PositionEventCallback>,
}

/// Thread-safe shared position tracker
pub type SharedPositionTracker = Arc<RwLock<PositionTracker>>;

impl PositionTracker {
    /// Create a new position tracker without callbacks
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            token_pairs: TokenPairRegistry::new(),
            callback: Arc::new(NoOpPositionCallback),
        }
    }

    /// Create a new position tracker with a callback
    pub fn with_callback(callback: Arc<dyn PositionEventCallback>) -> Self {
        Self {
            positions: HashMap::new(),
            token_pairs: TokenPairRegistry::new(),
            callback,
        }
    }

    // =========================================================================
    // Position Updates
    // =========================================================================

    /// Apply a fill to update positions
    ///
    /// This is the main entry point for position updates. It:
    /// 1. Updates position size and average price
    /// 2. Calculates realized P&L on closes
    /// 3. Tracks fees
    /// 4. Checks for merge opportunities
    ///
    /// Returns a tuple of (PositionEvent, Option<MergeOpportunity>) for callback firing.
    /// **IMPORTANT**: Callbacks should be fired OUTSIDE the lock scope to prevent deadlocks.
    /// Use `fire_callback()` after releasing the write lock.
    pub fn apply_fill(&mut self, fill: &Fill) -> (PositionEvent, Option<MergeOpportunity>) {
        let pos = self
            .positions
            .entry(fill.asset_id.clone())
            .or_insert_with(|| Position::new(fill.asset_id.clone()));

        let old_pos = Some(pos.clone());

        // Calculate fee
        let fee = fill.size * fill.price * (fill.fee_rate_bps / 10000.0);

        match fill.side {
            Side::Buy => Self::apply_buy(pos, fill),
            Side::Sell => Self::apply_sell(pos, fill),
        }

        pos.total_fees += fee;
        pos.fill_count += 1;
        pos.last_fill_time = fill.timestamp.clone();

        let new_pos = pos.clone();

        // Create the update event
        let update_event = PositionEvent::Updated {
            token_id: fill.asset_id.clone(),
            old_position: old_pos,
            new_position: new_pos,
            fill: fill.clone(),
        };

        // Check for merge opportunity (don't fire callback here - return for external firing)
        let merge_opportunity = self.check_merge_opportunity(&fill.asset_id);

        (update_event, merge_opportunity)
    }

    /// Fire callbacks for position events. Call this OUTSIDE the lock scope.
    ///
    /// ## Important
    /// This method should be called after releasing the write lock to prevent deadlocks.
    /// The callback may attempt to read the position tracker, which would deadlock if
    /// called while holding the write lock.
    pub fn fire_callback(&self, event: &PositionEvent) {
        self.callback.on_position_updated(event);
    }

    /// Apply a buy fill
    fn apply_buy(pos: &mut Position, fill: &Fill) {
        if pos.size >= -POSITION_EPSILON {
            // Opening or adding to long position (size >= 0 within epsilon)
            // Update average price: (old_cost + new_cost) / new_size
            let new_cost = fill.size * fill.price;
            pos.cost_basis += new_cost;
            pos.size += fill.size;
            if pos.size > POSITION_EPSILON {
                pos.avg_entry_price = pos.cost_basis / pos.size;
            }
        } else {
            // Closing short position
            let close_size = fill.size.min(pos.size.abs());
            // Short P&L: (entry - exit) * size
            let pnl = close_size * (pos.avg_entry_price - fill.price);
            pos.realized_pnl += pnl;
            pos.size += fill.size;

            // If flipped to long, recalculate
            if pos.size > 0.0 {
                let remaining_buy = fill.size - close_size;
                pos.cost_basis = remaining_buy * fill.price;
                pos.avg_entry_price = fill.price;
            } else if pos.is_flat() {
                pos.cost_basis = 0.0;
                pos.avg_entry_price = 0.0;
            } else {
                // Still short, reduce cost basis proportionally
                let remaining_ratio = pos.size.abs() / (pos.size.abs() + close_size);
                pos.cost_basis *= remaining_ratio;
            }
        }
        pos.total_bought += fill.size;
    }

    /// Apply a sell fill
    fn apply_sell(pos: &mut Position, fill: &Fill) {
        if pos.size > POSITION_EPSILON {
            // Closing long position
            let close_size = fill.size.min(pos.size);
            // Long P&L: (exit - entry) * size
            let pnl = close_size * (fill.price - pos.avg_entry_price);
            pos.realized_pnl += pnl;
            pos.size -= fill.size;

            // If flipped to short, recalculate
            if pos.size < -POSITION_EPSILON {
                let remaining_sell = fill.size - close_size;
                pos.cost_basis = remaining_sell * fill.price;
                pos.avg_entry_price = fill.price;
            } else if pos.is_flat() {
                pos.cost_basis = 0.0;
                pos.avg_entry_price = 0.0;
            } else {
                // Still long, reduce cost basis proportionally
                pos.cost_basis = pos.size * pos.avg_entry_price;
            }
        } else {
            // Opening or adding to short position (size <= 0 within epsilon)
            let new_cost = fill.size * fill.price;
            pos.cost_basis += new_cost;
            pos.size -= fill.size;
            if pos.size.abs() > POSITION_EPSILON {
                pos.avg_entry_price = pos.cost_basis / pos.size.abs();
            }
        }
        pos.total_sold += fill.size;
    }

    // =========================================================================
    // Token Pair Registration
    // =========================================================================

    /// Register a token pair for merge detection
    pub fn register_token_pair(&mut self, token_a: &str, token_b: &str, condition_id: &str) {
        self.token_pairs.register_pair(token_a, token_b, condition_id);
    }

    /// Register token IDs from a market (array of token IDs)
    pub fn register_token_ids(&mut self, token_ids: &[String], condition_id: &str) {
        if token_ids.len() == 2 {
            self.token_pairs
                .register_pair(&token_ids[0], &token_ids[1], condition_id);
        } else if token_ids.len() > 2 {
            // Multi-outcome: register all pairwise
            for i in 0..token_ids.len() {
                for j in (i + 1)..token_ids.len() {
                    self.token_pairs
                        .register_pair(&token_ids[i], &token_ids[j], condition_id);
                }
            }
        }
    }

    /// Get the complement token for a given token
    pub fn get_complement_token(&self, token_id: &str) -> Option<&String> {
        self.token_pairs.get_complement(token_id)
    }

    // =========================================================================
    // Merge Detection
    // =========================================================================

    /// Check for a merge opportunity for a given token
    fn check_merge_opportunity(&self, token_id: &str) -> Option<MergeOpportunity> {
        let complement_id = self.token_pairs.get_complement(token_id)?;
        let condition_id = self.token_pairs.get_market(token_id)?;

        let pos_a = self.positions.get(token_id)?;
        let pos_b = self.positions.get(complement_id)?;

        // Both must be long (positive size) to merge
        if pos_a.size <= POSITION_EPSILON || pos_b.size <= POSITION_EPSILON {
            return None;
        }

        let mergeable = pos_a.size.min(pos_b.size);
        if mergeable <= POSITION_EPSILON {
            return None;
        }

        // Calculate cost of the mergeable portion
        let cost_a = mergeable * pos_a.avg_entry_price;
        let cost_b = mergeable * pos_b.avg_entry_price;
        let total_cost = cost_a + cost_b;

        // Merge value is $1.00 per pair
        let merge_value = mergeable * 1.0;

        Some(MergeOpportunity {
            token_a: token_id.to_string(),
            token_b: complement_id.clone(),
            condition_id: condition_id.clone(),
            mergeable_pairs: mergeable,
            merge_value,
            total_cost,
            potential_profit: merge_value - total_cost,
            avg_cost_per_pair: total_cost / mergeable,
        })
    }

    /// Get all current merge opportunities
    pub fn get_merge_opportunities(&self) -> Vec<MergeOpportunity> {
        let mut opportunities = Vec::new();
        let mut checked = std::collections::HashSet::new();

        for token_id in self.positions.keys() {
            if checked.contains(token_id) {
                continue;
            }

            if let Some(complement_id) = self.token_pairs.get_complement(token_id) {
                checked.insert(token_id.clone());
                checked.insert(complement_id.clone());

                if let Some(opp) = self.check_merge_opportunity(token_id) {
                    opportunities.push(opp);
                }
            }
        }

        opportunities
    }

    /// Get merge opportunity for a specific token (and its complement)
    pub fn get_merge_opportunity_for(&self, token_id: &str) -> Option<MergeOpportunity> {
        self.check_merge_opportunity(token_id)
    }

    // =========================================================================
    // Queries
    // =========================================================================

    /// Get a position by token ID
    pub fn get_position(&self, token_id: &str) -> Option<&Position> {
        self.positions.get(token_id)
    }

    /// Get all positions
    pub fn get_all_positions(&self) -> Vec<&Position> {
        self.positions.values().collect()
    }

    /// Get net position size for a token
    pub fn get_net_size(&self, token_id: &str) -> f64 {
        self.positions
            .get(token_id)
            .map(|p| p.size)
            .unwrap_or(0.0)
    }

    /// Get unrealized P&L for a token at a given mark price
    pub fn get_unrealized_pnl(&self, token_id: &str, mark_price: f64) -> f64 {
        self.positions
            .get(token_id)
            .map(|p| p.unrealized_pnl(mark_price))
            .unwrap_or(0.0)
    }

    /// Get total realized P&L across all positions
    pub fn get_total_realized_pnl(&self) -> f64 {
        self.positions.values().map(|p| p.realized_pnl).sum()
    }

    /// Get total fees paid across all positions
    pub fn get_total_fees(&self) -> f64 {
        self.positions.values().map(|p| p.total_fees).sum()
    }

    /// Get number of tracked positions
    pub fn position_count(&self) -> usize {
        self.positions.len()
    }

    /// Check if there are any open positions
    pub fn has_open_positions(&self) -> bool {
        self.positions.values().any(|p| !p.is_flat())
    }

    // =========================================================================
    // Hydration
    // =========================================================================

    /// Hydrate a position from REST API data
    ///
    /// Use this to initialize positions from the `/positions` endpoint on startup.
    pub fn hydrate_position(&mut self, token_id: &str, size: f64, avg_price: f64) {
        let pos = self
            .positions
            .entry(token_id.to_string())
            .or_insert_with(|| Position::new(token_id.to_string()));

        pos.size = size;
        pos.avg_entry_price = avg_price;
        pos.cost_basis = size.abs() * avg_price;
    }

    /// Clear all positions (use with caution)
    pub fn clear(&mut self) {
        self.positions.clear();
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// PositionTrackerBridge
// =============================================================================

/// Bridge that forwards fills from OrderStateStore to PositionTracker
///
/// Implements `OrderEventCallback` to receive fill events from the order manager
/// and forwards them to the position tracker.
pub struct PositionTrackerBridge {
    tracker: SharedPositionTracker,
}

impl PositionTrackerBridge {
    /// Create a new bridge
    pub fn new(tracker: SharedPositionTracker) -> Self {
        Self { tracker }
    }

    /// Get a reference to the underlying tracker
    pub fn tracker(&self) -> &SharedPositionTracker {
        &self.tracker
    }
}

impl OrderEventCallback for PositionTrackerBridge {
    fn on_order_placed(&self, _order: &Order) {
        // Not relevant for position tracking
    }

    fn on_order_updated(&self, _order: &Order) {
        // Not relevant for position tracking
    }

    fn on_order_cancelled(&self, _order: &Order) {
        // Not relevant for position tracking
    }

    fn on_order_filled(&self, _order: &Order) {
        // Order fill event doesn't contain fill details
        // We track via on_trade instead
    }

    fn on_trade(&self, fill: &Fill) {
        // Apply fill and get events (acquire write lock)
        let (event, merge_opportunity) = self.tracker.write().apply_fill(fill);

        // Log the fill for visibility
        if let PositionEvent::Updated { ref new_position, .. } = event {
            info!(
                "[PositionTracker] {} {} {:.2} @ ${:.4} | New position: {:.2} shares @ ${:.4} avg | Realized P&L: ${:.2}",
                fill.side,
                &fill.asset_id[..8.min(fill.asset_id.len())],
                fill.size,
                fill.price,
                new_position.size,
                new_position.avg_entry_price,
                new_position.realized_pnl
            );
        }

        // Fire callbacks OUTSIDE the lock scope to prevent deadlocks
        // This allows callbacks to safely read the tracker
        {
            let tracker = self.tracker.read();
            tracker.fire_callback(&event);

            if let Some(ref merge) = merge_opportunity {
                info!(
                    "[PositionTracker] MERGE OPPORTUNITY: {:.2} pairs available, profit: ${:.2} ({:.1}%)",
                    merge.mergeable_pairs,
                    merge.potential_profit,
                    merge.profit_percentage()
                );
                let merge_event = PositionEvent::MergeOpportunity(merge.clone());
                tracker.fire_callback(&merge_event);
            }
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TRADE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_fill(asset_id: &str, side: Side, price: f64, size: f64) -> Fill {
        let id = TRADE_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        Fill {
            trade_id: format!("trade-{}", id),
            asset_id: asset_id.to_string(),
            market: "market-1".to_string(),
            side,
            outcome: if side == Side::Buy { "YES" } else { "NO" }.to_string(),
            price,
            size,
            status: super::super::order_manager::TradeStatus::Confirmed,
            taker_order_id: "taker-1".to_string(),
            trader_side: "TAKER".to_string(),
            fee_rate_bps: 0.0, // No fees for simpler testing
            transaction_hash: None,
            maker_orders: vec![],
            match_time: "2024-01-01T00:00:00Z".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            owner: "owner-1".to_string(),
        }
    }

    // =========================================================================
    // Position Creation Tests
    // =========================================================================

    #[test]
    fn test_position_new() {
        let pos = Position::new("token-1".to_string());
        assert_eq!(pos.token_id, "token-1");
        assert_eq!(pos.size, 0.0);
        assert!(pos.is_flat());
    }

    // =========================================================================
    // Buy Tests
    // =========================================================================

    #[test]
    fn test_buy_opens_long() {
        let mut tracker = PositionTracker::new();

        let fill = make_fill("token-1", Side::Buy, 0.50, 100.0);
        tracker.apply_fill(&fill);

        let pos = tracker.get_position("token-1").unwrap();
        assert_eq!(pos.size, 100.0);
        assert_eq!(pos.avg_entry_price, 0.50);
        assert_eq!(pos.cost_basis, 50.0);
        assert_eq!(pos.total_bought, 100.0);
        assert!(pos.is_long());
    }

    #[test]
    fn test_buy_adds_to_long() {
        let mut tracker = PositionTracker::new();

        // First buy: 100 @ 0.50
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.50, 100.0));

        // Second buy: 100 @ 0.60
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.60, 100.0));

        let pos = tracker.get_position("token-1").unwrap();
        assert_eq!(pos.size, 200.0);
        // Avg price: (100*0.50 + 100*0.60) / 200 = 110/200 = 0.55
        assert!((pos.avg_entry_price - 0.55).abs() < 1e-9);
        assert_eq!(pos.cost_basis, 110.0);
        assert_eq!(pos.total_bought, 200.0);
    }

    #[test]
    fn test_buy_closes_short() {
        let mut tracker = PositionTracker::new();

        // Open short: sell 100 @ 0.60
        tracker.apply_fill(&make_fill("token-1", Side::Sell, 0.60, 100.0));

        // Close short: buy 100 @ 0.50 (profit!)
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.50, 100.0));

        let pos = tracker.get_position("token-1").unwrap();
        assert!(pos.is_flat());
        // Short P&L: (0.60 - 0.50) * 100 = 10.0
        assert!((pos.realized_pnl - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_buy_flips_short_to_long() {
        let mut tracker = PositionTracker::new();

        // Open short: sell 100 @ 0.60
        tracker.apply_fill(&make_fill("token-1", Side::Sell, 0.60, 100.0));

        // Flip to long: buy 150 @ 0.50
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.50, 150.0));

        let pos = tracker.get_position("token-1").unwrap();
        assert_eq!(pos.size, 50.0); // 150 - 100 = 50 long
        assert!(pos.is_long());
        // Short P&L on first 100: (0.60 - 0.50) * 100 = 10.0
        assert!((pos.realized_pnl - 10.0).abs() < 1e-9);
        // New long position at 0.50
        assert_eq!(pos.avg_entry_price, 0.50);
    }

    // =========================================================================
    // Sell Tests
    // =========================================================================

    #[test]
    fn test_sell_opens_short() {
        let mut tracker = PositionTracker::new();

        let fill = make_fill("token-1", Side::Sell, 0.60, 100.0);
        tracker.apply_fill(&fill);

        let pos = tracker.get_position("token-1").unwrap();
        assert_eq!(pos.size, -100.0);
        assert_eq!(pos.avg_entry_price, 0.60);
        assert!(pos.is_short());
    }

    #[test]
    fn test_sell_closes_long_profit() {
        let mut tracker = PositionTracker::new();

        // Open long: buy 100 @ 0.40
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.40, 100.0));

        // Close long: sell 100 @ 0.60 (profit!)
        tracker.apply_fill(&make_fill("token-1", Side::Sell, 0.60, 100.0));

        let pos = tracker.get_position("token-1").unwrap();
        assert!(pos.is_flat());
        // Long P&L: (0.60 - 0.40) * 100 = 20.0
        assert!((pos.realized_pnl - 20.0).abs() < 1e-9);
    }

    #[test]
    fn test_sell_closes_long_loss() {
        let mut tracker = PositionTracker::new();

        // Open long: buy 100 @ 0.60
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.60, 100.0));

        // Close long: sell 100 @ 0.40 (loss!)
        tracker.apply_fill(&make_fill("token-1", Side::Sell, 0.40, 100.0));

        let pos = tracker.get_position("token-1").unwrap();
        assert!(pos.is_flat());
        // Long P&L: (0.40 - 0.60) * 100 = -20.0
        assert!((pos.realized_pnl - (-20.0)).abs() < 1e-9);
    }

    #[test]
    fn test_partial_close() {
        let mut tracker = PositionTracker::new();

        // Open long: buy 100 @ 0.40
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.40, 100.0));

        // Partial close: sell 50 @ 0.60
        tracker.apply_fill(&make_fill("token-1", Side::Sell, 0.60, 50.0));

        let pos = tracker.get_position("token-1").unwrap();
        assert_eq!(pos.size, 50.0);
        assert_eq!(pos.avg_entry_price, 0.40); // Unchanged
        // Long P&L on 50: (0.60 - 0.40) * 50 = 10.0
        assert!((pos.realized_pnl - 10.0).abs() < 1e-9);
    }

    // =========================================================================
    // Unrealized P&L Tests
    // =========================================================================

    #[test]
    fn test_unrealized_pnl_long() {
        let mut tracker = PositionTracker::new();

        // Open long: buy 100 @ 0.40
        tracker.apply_fill(&make_fill("token-1", Side::Buy, 0.40, 100.0));

        // Mark at 0.60 -> unrealized = (0.60 - 0.40) * 100 = 20
        let unrealized = tracker.get_unrealized_pnl("token-1", 0.60);
        assert!((unrealized - 20.0).abs() < 1e-9);

        // Mark at 0.30 -> unrealized = (0.30 - 0.40) * 100 = -10
        let unrealized = tracker.get_unrealized_pnl("token-1", 0.30);
        assert!((unrealized - (-10.0)).abs() < 1e-9);
    }

    #[test]
    fn test_unrealized_pnl_short() {
        let mut tracker = PositionTracker::new();

        // Open short: sell 100 @ 0.60
        tracker.apply_fill(&make_fill("token-1", Side::Sell, 0.60, 100.0));

        // Mark at 0.40 -> unrealized = (0.60 - 0.40) * 100 = 20 (profit)
        let unrealized = tracker.get_unrealized_pnl("token-1", 0.40);
        assert!((unrealized - 20.0).abs() < 1e-9);

        // Mark at 0.80 -> unrealized = (0.60 - 0.80) * 100 = -20 (loss)
        let unrealized = tracker.get_unrealized_pnl("token-1", 0.80);
        assert!((unrealized - (-20.0)).abs() < 1e-9);
    }

    // =========================================================================
    // Merge Detection Tests
    // =========================================================================

    #[test]
    fn test_merge_opportunity_detection() {
        let mut tracker = PositionTracker::new();

        // Register token pair
        tracker.register_token_pair("yes-token", "no-token", "condition-1");

        // Buy YES @ 0.40
        tracker.apply_fill(&make_fill("yes-token", Side::Buy, 0.40, 100.0));

        // Buy NO @ 0.50
        tracker.apply_fill(&make_fill("no-token", Side::Buy, 0.50, 100.0));

        let opportunities = tracker.get_merge_opportunities();
        assert_eq!(opportunities.len(), 1);

        let opp = &opportunities[0];
        assert_eq!(opp.mergeable_pairs, 100.0);
        assert_eq!(opp.merge_value, 100.0); // 100 * $1.00
        assert_eq!(opp.total_cost, 90.0); // 100*0.40 + 100*0.50
        assert!((opp.potential_profit - 10.0).abs() < 1e-9); // 100 - 90 = 10
        assert!(opp.is_profitable());
    }

    #[test]
    fn test_merge_opportunity_unequal_sizes() {
        let mut tracker = PositionTracker::new();

        tracker.register_token_pair("yes-token", "no-token", "condition-1");

        // Buy YES @ 0.40 (100 shares)
        tracker.apply_fill(&make_fill("yes-token", Side::Buy, 0.40, 100.0));

        // Buy NO @ 0.55 (50 shares - less than YES)
        tracker.apply_fill(&make_fill("no-token", Side::Buy, 0.55, 50.0));

        let opp = tracker.get_merge_opportunity_for("yes-token").unwrap();
        assert_eq!(opp.mergeable_pairs, 50.0); // min(100, 50)
        assert_eq!(opp.merge_value, 50.0);
        // Cost: 50*0.40 + 50*0.55 = 20 + 27.5 = 47.5
        assert!((opp.total_cost - 47.5).abs() < 1e-9);
        // Profit: 50 - 47.5 = 2.5
        assert!((opp.potential_profit - 2.5).abs() < 1e-9);
    }

    #[test]
    fn test_merge_opportunity_unprofitable() {
        let mut tracker = PositionTracker::new();

        tracker.register_token_pair("yes-token", "no-token", "condition-1");

        // Buy YES @ 0.55
        tracker.apply_fill(&make_fill("yes-token", Side::Buy, 0.55, 100.0));

        // Buy NO @ 0.50
        tracker.apply_fill(&make_fill("no-token", Side::Buy, 0.50, 100.0));

        let opp = tracker.get_merge_opportunity_for("yes-token").unwrap();
        // Cost: 100*0.55 + 100*0.50 = 105
        // Merge value: 100
        // Profit: 100 - 105 = -5 (loss)
        assert!((opp.potential_profit - (-5.0)).abs() < 1e-9);
        assert!(!opp.is_profitable());
    }

    #[test]
    fn test_no_merge_with_short() {
        let mut tracker = PositionTracker::new();

        tracker.register_token_pair("yes-token", "no-token", "condition-1");

        // Buy YES @ 0.40
        tracker.apply_fill(&make_fill("yes-token", Side::Buy, 0.40, 100.0));

        // Short NO (sell)
        tracker.apply_fill(&make_fill("no-token", Side::Sell, 0.55, 100.0));

        // Can't merge long + short
        let opportunities = tracker.get_merge_opportunities();
        assert!(opportunities.is_empty());
    }

    // =========================================================================
    // Fee Tracking Tests
    // =========================================================================

    #[test]
    fn test_fee_tracking() {
        let mut tracker = PositionTracker::new();

        let mut fill = make_fill("token-1", Side::Buy, 0.50, 100.0);
        fill.fee_rate_bps = 50.0; // 0.5% fee

        tracker.apply_fill(&fill);

        let pos = tracker.get_position("token-1").unwrap();
        // Fee: 100 * 0.50 * (50/10000) = 50 * 0.005 = 0.25
        assert!((pos.total_fees - 0.25).abs() < 1e-9);
    }

    // =========================================================================
    // Hydration Tests
    // =========================================================================

    #[test]
    fn test_hydrate_position() {
        let mut tracker = PositionTracker::new();

        tracker.hydrate_position("token-1", 500.0, 0.45);

        let pos = tracker.get_position("token-1").unwrap();
        assert_eq!(pos.size, 500.0);
        assert_eq!(pos.avg_entry_price, 0.45);
        assert_eq!(pos.cost_basis, 225.0);
    }

    // =========================================================================
    // Bridge Tests
    // =========================================================================

    #[test]
    fn test_bridge_forwards_fills() {
        let tracker = Arc::new(RwLock::new(PositionTracker::new()));
        let bridge = PositionTrackerBridge::new(tracker.clone());

        let fill = make_fill("token-1", Side::Buy, 0.50, 100.0);
        bridge.on_trade(&fill);

        let pos = tracker.read().get_position("token-1").unwrap().clone();
        assert_eq!(pos.size, 100.0);
    }
}
