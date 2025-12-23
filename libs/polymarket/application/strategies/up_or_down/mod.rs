//! Up or Down Strategy
//!
//! Monitors recurring crypto price prediction markets
//! with tags: 'Up or Down', 'Crypto Prices', 'Recurring', 'Crypto'
//!
//! When a market enters the delta_t window (time before end), this strategy
//! spawns a WebSocket tracker to monitor the orderbook in real-time.

mod services;
mod strategy;
pub mod tracker;
pub mod types;

pub use strategy::UpOrDownStrategy;
pub use types::{CryptoAsset, Timeframe};
