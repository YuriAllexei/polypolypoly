//! Polymarket Trading Bot
//!
//! Complete suite for trading on Polymarket prediction markets.

pub mod client;
pub mod config;
pub mod database;
pub mod filter;
pub mod sniper;
pub mod strategy;
pub mod utils;

// Re-export commonly used items
pub use client::{
    PolymarketAuth,
    gamma::{GammaClient, GammaEvent, GammaMarket, GammaTag, GammaFilters},
    clob::{RestClient, WebSocketClient, Market, Outcome, OrderBook, PriceLevel, Side, OrderType, OrderArgs},
};

pub use config::BotConfig;
pub use database::{MarketDatabase, MarketSyncService};
pub use filter::LLMFilter;
pub use sniper::SniperMarket;
pub use strategy::{OrderExecutor, ResolutionMonitor, RiskManager};
pub use utils::{init_tracing, Heartbeat, ShutdownManager};
