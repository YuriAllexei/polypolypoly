//! Oracle Price Tracking Module
//!
//! Provides real-time crypto price tracking from ChainLink and Binance oracles
//! via Polymarket's live data WebSocket.
//!
//! # Usage
//!
//! ```rust,ignore
//! use polymarket::infrastructure::oracle::{spawn_oracle_trackers, OracleType};
//! use std::sync::Arc;
//! use std::sync::atomic::AtomicBool;
//!
//! let shutdown = Arc::new(AtomicBool::new(true));
//! let prices = spawn_oracle_trackers(shutdown).await?;
//!
//! // Read prices from shared state
//! let manager = prices.read().unwrap();
//! if let Some(eth_price) = manager.get_price(OracleType::ChainLink, "ETH") {
//!     println!("ETH price: ${}", eth_price.value);
//! }
//! ```

mod oracle_ws;
mod price_manager;
mod types;

// Re-export main types and functions
pub use oracle_ws::{
    parse_binance_symbol, parse_chainlink_symbol, spawn_oracle_trackers, OracleRoute,
};
pub use price_manager::{OracleHealthState, OraclePriceManager, PriceEntry, SharedOraclePrices};
pub use types::{OracleMessage, OraclePricePayload, OraclePriceUpdate, OracleSubscription, OracleType};
