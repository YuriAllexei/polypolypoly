//! Domain Layer
//!
//! Contains pure business entities and domain models.
//! This layer has no dependencies on infrastructure or application layers.

pub mod filter;
pub mod models;
pub mod orderbook;
pub mod sniper_market;
pub mod strategy;

// Re-export domain models
pub use models::{
    DbEvent, DbLLMCache, DbMarket, DbOpportunity, MarketFilters, SyncStats,
};

// Re-export domain entities
pub use sniper_market::SniperMarket;

// Re-export strategy domain entities
pub use strategy::{
    DailyStats, ExecutorError, MonitoredMarket, RiskConfig, RiskError, TradingConfig,
};

// Re-export filter domain entities
pub use filter::{
    CacheEntry, CacheError, CacheStats, FilterError, MarketInfo, OllamaError,
};
