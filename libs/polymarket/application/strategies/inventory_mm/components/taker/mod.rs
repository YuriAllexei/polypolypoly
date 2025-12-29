//! Taker component - handles immediate FOK order execution for rebalancing.

mod config;
mod task;

pub use config::TakerConfig;
pub use task::TakerTask;
