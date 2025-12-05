//! Infrastructure Layer
//!
//! Contains implementations of external interfaces (database, API clients, etc.)
//! This layer depends on the domain layer but not on the application layer.

pub mod client;
pub mod config;
pub mod database;
pub mod heartbeat;
pub mod logging;
pub mod shutdown;

// Re-export commonly used types from client
pub use client::{
    clob::{
        spawn_market_tracker, Market, OrderArgs, OrderBook, OrderType, Outcome, PriceLevel,
        RestClient, Side, WebSocketClient,
    },
    gamma::{GammaClient, GammaEvent, GammaFilters, GammaMarket, GammaTag},
    PolymarketAuth,
};

// Re-export database types
pub use database::{DatabaseError, MarketDatabase, Result};

// Re-export config types
pub use config::{BotConfig, EventsConfig, SniperConfig};

// Re-export infrastructure services
pub use heartbeat::Heartbeat;
pub use logging::{init_tracing, init_tracing_with_level};
pub use shutdown::ShutdownManager;
