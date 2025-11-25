//! CLOB (Central Limit Order Book) API client and types
//!
//! Provides REST and WebSocket clients for trading on Polymarket.

pub mod rest;
pub mod types;
pub mod sniper_ws_types;
pub mod sniper_ws;
pub mod orderbook;

pub use rest::RestClient;
pub use hypersockets::WebSocketClient;
pub use types::*;
pub use sniper_ws::spawn_market_tracker;
