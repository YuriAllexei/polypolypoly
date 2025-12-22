//! User Channel - Order State Management and WebSocket Tracking
//!
//! This module provides production-ready order/trade state management with:
//! - Real-time updates via WebSocket
//! - REST API hydration on startup
//! - Bid/Ask separation per asset
//! - Callback system for event notifications
//! - Memory management via pruning
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
//! ```

mod order_manager;
mod types;
mod user_ws;

// Re-export types for WebSocket messages
pub use types::{
    AuthPayload, MakerOrder, MessageType, OrderMessage, TradeMessage, TradeStatus as WsTradeStatus,
    UserMessage, UserSubscription,
};

// Re-export order manager types
pub use order_manager::{
    AssetOrderBook, Fill, MakerOrderInfo, NoOpCallback, Order, OrderEvent, OrderEventCallback,
    OrderStateStore, OrderStatus, OrderType, SharedOrderState, Side, StpCheckResult,
    TokenPairRegistry, TradeStatus,
};

// Re-export WebSocket functions
pub use user_ws::{
    spawn_user_order_tracker, spawn_user_order_tracker_ws_only, UserConfig, UserHandler, UserRoute,
    UserRouter,
};
