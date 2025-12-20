//! Order Manager - State Management for User Orders and Trades
//!
//! Provides dual-indexed storage for orders (by asset_id and order_id)
//! and storage for trades (by trade_id).

use super::types::{OrderMessage, OrderType, TradeMessage, TradeStatus};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Shared order manager accessible across threads
pub type SharedOrderManager = Arc<RwLock<OrderManager>>;

/// Order Manager with dual indexing
///
/// Maintains order and trade state received from the user WebSocket channel.
#[derive(Debug, Default)]
pub struct OrderManager {
    /// Orders indexed by order_id -> OrderState
    orders_by_id: HashMap<String, OrderState>,
    /// Orders indexed by asset_id -> Vec<order_id>
    /// (stores order IDs, actual orders are in orders_by_id)
    orders_by_asset: HashMap<String, Vec<String>>,
    /// Trades indexed by trade_id -> TradeState
    trades_by_id: HashMap<String, TradeState>,
    /// Trades indexed by asset_id -> Vec<trade_id>
    trades_by_asset: HashMap<String, Vec<String>>,
}

/// State of an order
#[derive(Debug, Clone)]
pub struct OrderState {
    pub order_id: String,
    pub asset_id: String,
    pub market: String,
    pub side: String,
    pub outcome: String,
    pub price: f64,
    pub original_size: f64,
    pub size_matched: f64,
    pub status: OrderStatus,
    pub associate_trades: Vec<String>,
    pub timestamp: String,
    pub owner: String,
}

/// Order status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    /// Order is active and open
    Open,
    /// Order is partially filled
    PartiallyFilled,
    /// Order is fully filled
    Filled,
    /// Order was cancelled
    Cancelled,
}

/// State of a trade
#[derive(Debug, Clone)]
pub struct TradeState {
    pub trade_id: String,
    pub asset_id: String,
    pub market: String,
    pub side: String,
    pub outcome: String,
    pub price: f64,
    pub size: f64,
    pub status: TradeStatus,
    pub taker_order_id: Option<String>,
    pub maker_orders: Vec<MakerOrderState>,
    pub timestamp: String,
    pub owner: String,
}

/// Maker order info within a trade
#[derive(Debug, Clone)]
pub struct MakerOrderState {
    pub order_id: String,
    pub asset_id: String,
    pub matched_amount: f64,
    pub price: f64,
    pub owner: String,
}

impl OrderManager {
    /// Create a new empty order manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Process an order message and update state
    pub fn process_order(&mut self, msg: &OrderMessage) {
        match msg.order_type() {
            OrderType::Placement => self.handle_placement(msg),
            OrderType::Update => self.handle_update(msg),
            OrderType::Cancellation => self.handle_cancellation(msg),
        }
    }

    /// Process a trade message and update state
    pub fn process_trade(&mut self, msg: &TradeMessage) {
        let trade_state = TradeState {
            trade_id: msg.id.clone(),
            asset_id: msg.asset_id.clone(),
            market: msg.market.clone(),
            side: msg.side.clone(),
            outcome: msg.outcome.clone(),
            price: msg.price.parse().unwrap_or(0.0),
            size: msg.size.parse().unwrap_or(0.0),
            status: msg.trade_status(),
            taker_order_id: msg.taker_order_id.clone(),
            maker_orders: msg
                .maker_orders
                .iter()
                .map(|m| MakerOrderState {
                    order_id: m.order_id.clone(),
                    asset_id: m.asset_id.clone(),
                    matched_amount: m.matched_amount.parse().unwrap_or(0.0),
                    price: m.price.parse().unwrap_or(0.0),
                    owner: m.owner.clone(),
                })
                .collect(),
            timestamp: msg.timestamp.clone(),
            owner: msg.owner.clone(),
        };

        // Update asset index if new trade
        if !self.trades_by_id.contains_key(&msg.id) {
            self.trades_by_asset
                .entry(msg.asset_id.clone())
                .or_default()
                .push(msg.id.clone());
        }

        self.trades_by_id.insert(msg.id.clone(), trade_state);
    }

    /// Handle order placement
    fn handle_placement(&mut self, msg: &OrderMessage) {
        let order_state = OrderState {
            order_id: msg.id.clone(),
            asset_id: msg.asset_id.clone(),
            market: msg.market.clone(),
            side: msg.side.clone(),
            outcome: msg.outcome.clone(),
            price: msg.price.parse().unwrap_or(0.0),
            original_size: msg.original_size.parse().unwrap_or(0.0),
            size_matched: msg.size_matched.parse().unwrap_or(0.0),
            status: OrderStatus::Open,
            associate_trades: msg.associate_trades.clone(),
            timestamp: msg.timestamp.clone(),
            owner: msg.owner.clone(),
        };

        // Add to asset index
        self.orders_by_asset
            .entry(msg.asset_id.clone())
            .or_default()
            .push(msg.id.clone());

        // Add to order index
        self.orders_by_id.insert(msg.id.clone(), order_state);
    }

    /// Handle order update (partial fill)
    fn handle_update(&mut self, msg: &OrderMessage) {
        if let Some(order) = self.orders_by_id.get_mut(&msg.id) {
            let size_matched: f64 = msg.size_matched.parse().unwrap_or(0.0);
            order.size_matched = size_matched;
            order.associate_trades = msg.associate_trades.clone();
            order.timestamp = msg.timestamp.clone();

            // Update status based on fill
            if size_matched >= order.original_size {
                order.status = OrderStatus::Filled;
            } else if size_matched > 0.0 {
                order.status = OrderStatus::PartiallyFilled;
            }
        } else {
            // Order not found, treat as new placement
            self.handle_placement(msg);
        }
    }

    /// Handle order cancellation
    fn handle_cancellation(&mut self, msg: &OrderMessage) {
        if let Some(order) = self.orders_by_id.get_mut(&msg.id) {
            order.status = OrderStatus::Cancelled;
            order.timestamp = msg.timestamp.clone();
        }
    }

    // =========================================================================
    // Query Methods
    // =========================================================================

    /// Get order by order_id
    pub fn get_order(&self, order_id: &str) -> Option<&OrderState> {
        self.orders_by_id.get(order_id)
    }

    /// Get all orders for an asset
    pub fn get_orders_by_asset(&self, asset_id: &str) -> Vec<&OrderState> {
        self.orders_by_asset
            .get(asset_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.orders_by_id.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all open orders for an asset
    pub fn get_open_orders_by_asset(&self, asset_id: &str) -> Vec<&OrderState> {
        self.get_orders_by_asset(asset_id)
            .into_iter()
            .filter(|o| o.status == OrderStatus::Open || o.status == OrderStatus::PartiallyFilled)
            .collect()
    }

    /// Get trade by trade_id
    pub fn get_trade(&self, trade_id: &str) -> Option<&TradeState> {
        self.trades_by_id.get(trade_id)
    }

    /// Get all trades for an asset
    pub fn get_trades_by_asset(&self, asset_id: &str) -> Vec<&TradeState> {
        self.trades_by_asset
            .get(asset_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.trades_by_id.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all orders
    pub fn all_orders(&self) -> impl Iterator<Item = &OrderState> {
        self.orders_by_id.values()
    }

    /// Get all trades
    pub fn all_trades(&self) -> impl Iterator<Item = &TradeState> {
        self.trades_by_id.values()
    }

    /// Count of tracked orders
    pub fn order_count(&self) -> usize {
        self.orders_by_id.len()
    }

    /// Count of tracked trades
    pub fn trade_count(&self) -> usize {
        self.trades_by_id.len()
    }

    /// Count of unique assets with orders
    pub fn asset_count(&self) -> usize {
        self.orders_by_asset.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order_msg(id: &str, asset_id: &str, msg_type: &str, size_matched: &str) -> OrderMessage {
        OrderMessage {
            asset_id: asset_id.to_string(),
            associate_trades: vec![],
            event_type: "order".to_string(),
            id: id.to_string(),
            market: "market-1".to_string(),
            order_owner: None,
            original_size: "100".to_string(),
            outcome: "YES".to_string(),
            owner: "owner-1".to_string(),
            price: "0.5".to_string(),
            side: "BUY".to_string(),
            size_matched: size_matched.to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            msg_type: msg_type.to_string(),
        }
    }

    #[test]
    fn test_order_placement() {
        let mut mgr = OrderManager::new();
        let msg = make_order_msg("order-1", "asset-1", "PLACEMENT", "0");
        mgr.process_order(&msg);

        assert_eq!(mgr.order_count(), 1);
        let order = mgr.get_order("order-1").unwrap();
        assert_eq!(order.status, OrderStatus::Open);
        assert_eq!(order.original_size, 100.0);
    }

    #[test]
    fn test_order_update() {
        let mut mgr = OrderManager::new();
        mgr.process_order(&make_order_msg("order-1", "asset-1", "PLACEMENT", "0"));
        mgr.process_order(&make_order_msg("order-1", "asset-1", "UPDATE", "50"));

        let order = mgr.get_order("order-1").unwrap();
        assert_eq!(order.status, OrderStatus::PartiallyFilled);
        assert_eq!(order.size_matched, 50.0);
    }

    #[test]
    fn test_order_cancellation() {
        let mut mgr = OrderManager::new();
        mgr.process_order(&make_order_msg("order-1", "asset-1", "PLACEMENT", "0"));
        mgr.process_order(&make_order_msg("order-1", "asset-1", "CANCELLATION", "0"));

        let order = mgr.get_order("order-1").unwrap();
        assert_eq!(order.status, OrderStatus::Cancelled);
    }

    #[test]
    fn test_dual_indexing() {
        let mut mgr = OrderManager::new();
        mgr.process_order(&make_order_msg("order-1", "asset-1", "PLACEMENT", "0"));
        mgr.process_order(&make_order_msg("order-2", "asset-1", "PLACEMENT", "0"));
        mgr.process_order(&make_order_msg("order-3", "asset-2", "PLACEMENT", "0"));

        // By order_id
        assert!(mgr.get_order("order-1").is_some());
        assert!(mgr.get_order("order-2").is_some());

        // By asset_id
        assert_eq!(mgr.get_orders_by_asset("asset-1").len(), 2);
        assert_eq!(mgr.get_orders_by_asset("asset-2").len(), 1);
    }
}
