pub mod executor;
pub mod monitor;
pub mod risk;

// Re-export services
pub use executor::OrderExecutor;
pub use monitor::ResolutionMonitor;
pub use risk::RiskManager;

// Re-export domain entities (for backward compatibility)
pub use crate::domain::strategy::{
    DailyStats, ExecutorError, MonitoredMarket, RiskConfig, TradingConfig,
};
