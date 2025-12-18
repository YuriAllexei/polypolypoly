//! Polymarket API clients
//!
//! Provides clients for both the Gamma API (market data) and CLOB API (trading).

pub mod auth;
pub mod clob;
pub mod data;
pub mod gamma;
pub mod oracle;
pub mod sports;
pub mod user;

pub use auth::PolymarketAuth;
pub use clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs, TradingClient, TradingError};
pub use data::{DataApiClient, Position, PositionFilters, PositionSortBy, SortDirection};
pub use gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters};
pub use oracle::{spawn_oracle_trackers, OraclePriceManager, SharedOraclePrices, OracleType, PriceEntry};
pub use sports::{spawn_sports_live_data_tracker, SportsLiveData, SportsLiveDataMessage, SportsRoute};
pub use user::{spawn_user_order_tracker, OrderManager, SharedOrderManager, OrderState, OrderStatus, TradeState};
