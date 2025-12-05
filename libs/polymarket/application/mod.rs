//! Application Layer
//!
//! Contains use cases and application services.
//! This layer depends on domain and infrastructure layers.

pub mod facade;
pub mod sniper;
pub mod strategy;
pub mod sync;

// Re-export application facade for binaries
pub use facade::{init_logging, init_logging_with_level, to_sniper_market, EventSyncApp, SniperApp};

// Re-export sniper use cases
pub use sniper::{ConfigService, MarketTrackerService};

// Re-export sync services
pub use sync::{EventSyncService, MarketSyncService};

// Re-export strategy services
pub use strategy::{OrderExecutor, ResolutionMonitor, RiskManager};
