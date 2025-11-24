//! Polymarket API clients
//!
//! Provides clients for both the Gamma API (market data) and CLOB API (trading).

pub mod auth;
pub mod gamma;
pub mod clob;

pub use auth::PolymarketAuth;
pub use gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters};
pub use clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs};
