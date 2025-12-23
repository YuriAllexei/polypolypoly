//! Binance Direct Price Tracking Module
//!
//! Provides real-time crypto price tracking directly from Binance WebSocket
//! for lower latency than the Polymarket relay.
//!
//! # Latency Comparison
//!
//! - **Oracle module** (via Polymarket relay): ~50-100ms extra latency
//! - **Binance module** (direct connection): ~10-30ms latency
//!
//! # Supported Assets
//!
//! - BTC (Bitcoin)
//! - ETH (Ethereum)
//! - SOL (Solana)
//! - XRP (Ripple)
//!
//! # Usage
//!
//! ```rust,ignore
//! use polymarket::infrastructure::client::binance::{spawn_binance_tracker, BinanceAsset};
//! use std::sync::Arc;
//! use std::sync::atomic::AtomicBool;
//! use std::time::Duration;
//!
//! // Start the tracker
//! let shutdown = Arc::new(AtomicBool::new(true));
//! let prices = spawn_binance_tracker(shutdown).await?;
//!
//! // Read prices from shared state
//! let manager = prices.read();
//!
//! // Get price by symbol
//! if let Some(btc_price) = manager.get_price("BTC") {
//!     println!("BTC: ${:.2} (latency: {}ms)", btc_price.value, btc_price.latency_ms);
//! }
//!
//! // Get price by asset enum
//! if let Some(eth_price) = manager.get_price_by_asset(BinanceAsset::ETH) {
//!     println!("ETH: ${:.2}", eth_price.value);
//! }
//!
//! // Check connection health
//! if !manager.is_healthy(Duration::from_secs(5)) {
//!     eprintln!("Warning: Binance feed is stale!");
//! }
//!
//! // Get latency statistics
//! println!("Avg latency: {:.1}ms", manager.avg_latency_ms());
//! ```
//!
//! # Comparison with Oracle Module
//!
//! The existing `oracle` module connects to Polymarket's relay which aggregates
//! ChainLink and Binance prices. This `binance` module connects directly to
//! Binance for lower latency:
//!
//! | Feature | Oracle Module | Binance Module |
//! |---------|--------------|----------------|
//! | Connection | Polymarket relay | Direct Binance |
//! | Latency | ~50-100ms | ~10-30ms |
//! | ChainLink prices | Yes | No |
//! | Binance prices | Yes | Yes |
//! | HFT suitable | No | Yes |
//!
//! Use the `binance` module when latency is critical (HFT strategies).
//! Use the `oracle` module for ChainLink prices or when Polymarket's
//! timestamp normalization is preferred.

mod price_manager;
mod types;
mod websocket;

// Re-export main types and functions
pub use price_manager::{
    BinanceHealthState, BinancePriceEntry, BinancePriceManager, SharedBinancePrices,
};
pub use types::{
    BinanceAsset, BinanceMessage, BinanceRoute, BinanceStreamWrapper, BinanceTradeData,
};
pub use websocket::spawn_binance_tracker;
