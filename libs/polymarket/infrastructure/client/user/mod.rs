//! User Channel Types (Deprecated)
//!
//! This module is deprecated. Use `polymarket::infrastructure::OrderManager` instead.
//!
//! The order tracking functionality has been moved to:
//! - `libs/polymarket/infrastructure/order_manager.rs`

// Keep types.rs for backwards compatibility if needed
mod types;

// Re-export types for backwards compatibility
pub use types::{
    AuthPayload, MakerOrder, OrderMessage, OrderType, TradeMessage, TradeStatus, UserMessage,
    UserSubscription,
};
