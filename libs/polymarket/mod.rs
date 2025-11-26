//! Polymarket Trading Bot
//!
//! Complete suite for trading on Polymarket prediction markets.

// Core layers (Clean Architecture)
pub mod domain;
pub mod infrastructure;
pub mod application;

// Legacy modules (re-export from new locations for backward compatibility)
pub mod client;
pub mod config;
pub mod database;
pub mod filter;
pub mod sniper;
pub mod strategy;

// Re-export commonly used items from infrastructure
pub use infrastructure::{
    PolymarketAuth,
    BotConfig,
    client::{
        gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters},
        clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs},
    },
    database::MarketDatabase,
};

// Re-export from application layer
pub use application::{
    EventSyncService, 
    MarketSyncService,
    LLMFilter,
    OrderExecutor, 
    ResolutionMonitor, 
    RiskManager,
};

// Re-export from domain layer
pub use domain::SniperMarket;

// Re-export utils from infrastructure for backward compatibility
pub use infrastructure::{init_tracing, Heartbeat, ShutdownManager};
