//! Application Layer
//!
//! Contains use cases and application services.
//! This layer depends on domain and infrastructure layers.

pub mod facade;
pub mod sniper;
pub mod strategies;
pub mod sync;

// Re-export application facade for binaries
pub use facade::{init_logging, init_logging_with_level, to_sniper_market, EventSyncApp};

// Re-export sniper use cases
pub use sniper::ConfigService;

// Re-export sync services
pub use sync::{EventSyncService, MarketSyncService};

// Re-export pluggable strategies system
pub use strategies::{
    create_strategy, Strategy, StrategyContext, StrategyError, StrategyResult,
    StrategyType, UpOrDownStrategy,
};

// Re-export infrastructure managers
pub use crate::infrastructure::{BalanceManager, PositionManager};
