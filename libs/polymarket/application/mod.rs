//! Application Layer
//!
//! Contains use cases and application services.
//! This layer depends on domain and infrastructure layers.

pub mod facade;
pub mod filter;
pub mod sniper;
pub mod strategy;
pub mod sync;

// Re-export application facade for binaries
pub use facade::{EventSyncApp, SniperApp, init_logging, to_sniper_market};

// Re-export sniper use cases
pub use sniper::{MarketTrackerService, ConfigService};

// Re-export sync services
pub use sync::{EventSyncService, MarketSyncService};

// Re-export filter service
pub use filter::LLMFilter;

// Re-export strategy services
pub use strategy::{OrderExecutor, ResolutionMonitor, RiskManager};
