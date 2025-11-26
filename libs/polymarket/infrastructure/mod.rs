//! Infrastructure Layer
//!
//! Contains implementations of external interfaces (database, API clients, etc.)
//! This layer depends on the domain layer but not on the application layer.

pub mod cache;
pub mod client;
pub mod config;
pub mod database;
pub mod heartbeat;
pub mod logging;
pub mod ollama;
pub mod shutdown;

// Re-export commonly used types from client
pub use client::{
    PolymarketAuth,
    gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters},
    clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs},
};

// Re-export database types
pub use database::{MarketDatabase, DatabaseError, Result};

// Re-export config types
pub use config::{BotConfig, SniperConfig};

// Re-export infrastructure services
pub use cache::MarketCache;
pub use heartbeat::Heartbeat;
pub use logging::init_tracing;
pub use ollama::OllamaClient;
pub use shutdown::ShutdownManager;
