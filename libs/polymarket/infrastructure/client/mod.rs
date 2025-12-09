//! Polymarket API clients
//!
//! Provides clients for both the Gamma API (market data) and CLOB API (trading).

pub mod auth;
pub mod gamma;
pub mod clob;
pub mod oracle;
pub mod user;

pub use auth::PolymarketAuth;
pub use gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters};
pub use clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs};
pub use oracle::{spawn_oracle_trackers, OraclePriceManager, SharedOraclePrices, OracleType, PriceEntry};
pub use user::{spawn_user_order_tracker, OrderManager, SharedOrderManager, OrderState, OrderStatus, TradeState};
