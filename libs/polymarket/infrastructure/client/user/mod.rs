//! User Channel - Order State Management and WebSocket Tracking
//!
//! This module provides production-ready order/trade state management with:
//! - Real-time updates via WebSocket
//! - REST API hydration on startup
//! - Bid/Ask separation per asset
//! - Callback system for event notifications
//! - Memory management via pruning
//! - Position tracking with P&L calculation
//! - Merge/split detection for Up/Down token pairs
//!
//! ## Usage
//!
//! ```ignore
//! use polymarket::infrastructure::client::user::*;
//!
//! // Start with REST hydration
//! let state = spawn_user_order_tracker(
//!     shutdown_flag,
//!     &rest_client,
//!     &auth,
//!     None, // or Some(callback)
//! ).await?;
//!
//! // Query orders
//! let open_orders = state.read().get_open_orders("asset-123");
//! let bids = state.read().get_bids("asset-123");
//!
//! // Position tracking with merge detection
//! let tracker = Arc::new(RwLock::new(PositionTracker::new()));
//! tracker.write().register_token_pair("yes_token", "no_token", "condition_123");
//! let bridge = Arc::new(PositionTrackerBridge::new(tracker.clone()));
//! // Pass bridge to OrderStateStore to receive fills
//! ```

mod order_manager;
mod position_tracker;
mod reconciliation;
mod types;
mod user_ws;

// Re-export types for WebSocket messages
pub use types::{
    AuthPayload, MakerOrder, MessageType, OrderMessage, TradeMessage, TradeStatus as WsTradeStatus,
    UserMessage, UserSubscription,
};

// Re-export order manager types
pub use order_manager::{
    parse_timestamp_to_i64, AssetOrderBook, Fill, MakerOrderInfo, NoOpCallback, Order, OrderEvent,
    OrderEventCallback, OrderReconciliationResult, OrderStateStore, OrderStatus, OrderType,
    SharedOrderState, Side, StpCheckResult, TokenPairRegistry, TradeStatus,
};

// Re-export WebSocket functions
pub use user_ws::{
    spawn_user_order_tracker, spawn_user_order_tracker_ws_only, UserConfig, UserHandler, UserRoute,
    UserRouter,
};

// Re-export position tracker types
pub use position_tracker::{
    MergeOpportunity, NoOpPositionCallback, Position, PositionDiscrepancy, PositionEvent,
    PositionEventCallback, PositionTracker, PositionTrackerBridge, ReconciliationResult,
    SharedPositionTracker,
};

// Re-export reconciliation tasks
pub use reconciliation::{
    spawn_order_reconciliation_task, spawn_position_reconciliation_task, ReconciliationConfig,
};
