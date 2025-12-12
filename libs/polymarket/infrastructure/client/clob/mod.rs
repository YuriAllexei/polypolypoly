//! CLOB (Central Limit Order Book) API client and types
//!
//! Provides REST and WebSocket clients for trading on Polymarket.
//!
//! # Module Structure
//!
//! - `constants`: Exchange addresses, signature types, decimal constants
//! - `types`: Market, Order, Position and other API types
//! - `helpers`: Shared HTTP helper functions
//! - `rest/`: REST API client (split into mod, orders, auth)
//! - `order_builder/`: EIP-712 order signing (split into mod, types, signing, encoding, payload)
//! - `trading`: High-level trading client with simplified API
//! - `sniper_ws`: WebSocket market tracker

pub mod constants;
mod helpers;
pub mod order_builder;
pub mod orderbook;
pub mod rest;
pub mod sniper_ws;
pub mod sniper_ws_types;
pub mod trading;
pub mod types;

// Re-export main types
pub use constants::*;
pub use hypersockets::WebSocketClient;
pub use order_builder::{Order, OrderBuilder, SignedOrder};
pub use rest::RestClient;
pub use sniper_ws::spawn_market_tracker;
pub use trading::{TradingClient, TradingError};
pub use types::*;
