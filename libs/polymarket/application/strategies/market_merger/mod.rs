//! Market Merger Strategy
//!
//! Accumulates balanced positions in Up and Down tokens at combined cost < $1.00,
//! then merges for guaranteed profit.
//!
//! Key mechanics:
//! - Only places BID orders (accumulation)
//! - Multi-level bid ladder (3 levels per token)
//! - Dynamic sizing based on position growth
//! - Opportunity-based taker (score-based, not threshold-based)
//! - Continuous merging when positions are balanced and profitable

mod config;
mod strategy;
pub mod tracker;
pub mod services;
pub mod types;

pub use config::MarketMergerConfig;
pub use strategy::MarketMergerStrategy;
