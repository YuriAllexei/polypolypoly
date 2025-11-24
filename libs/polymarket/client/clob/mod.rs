//! CLOB (Central Limit Order Book) API client and types
//!
//! Provides REST and WebSocket clients for trading on Polymarket.

pub mod rest;
pub mod websocket;
pub mod types;

pub use rest::RestClient;
pub use hypersockets::WebSocketClient;
pub use types::*;
