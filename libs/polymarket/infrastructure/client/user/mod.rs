//! User Channel WebSocket Client
//!
//! Real-time order and trade tracking via Polymarket's user WebSocket channel.
//!
//! # Overview
//!
//! This module provides a WebSocket client that connects to the Polymarket user
//! channel to receive real-time updates about your orders and trades.
//!
//! # Usage
//!
//! ```ignore
//! use polymarket::infrastructure::spawn_user_order_tracker;
//!
//! // Spawn the tracker (requires API_KEY, API_SECRET, API_PASSPHRASE env vars)
//! let orders = spawn_user_order_tracker(shutdown_flag).await?;
//!
//! // Query order state
//! let mgr = orders.read().unwrap();
//! for order in mgr.all_orders() {
//!     println!("{}: {} @ {}", order.order_id, order.side, order.price);
//! }
//! ```
//!
//! # Environment Variables
//!
//! - `API_KEY` - Polymarket API key
//! - `API_SECRET` - Polymarket API secret
//! - `API_PASSPHRASE` - Polymarket API passphrase
//!
//! # Message Types
//!
//! The user channel provides two message types:
//!
//! - **Order**: Placement, update, or cancellation of orders
//! - **Trade**: Trade executions and status updates

mod order_manager;
mod types;
mod user_ws;

// Re-export types
pub use order_manager::{
    MakerOrderState, OrderManager, OrderState, OrderStatus, SharedOrderManager, TradeState,
};
pub use types::{
    AuthPayload, MakerOrder, OrderMessage, OrderType, TradeMessage, TradeStatus, UserMessage,
    UserSubscription,
};
pub use user_ws::{spawn_user_order_tracker, UserConfig, UserHandler, UserRoute, UserRouter};
