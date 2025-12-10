//! Sniper Use Cases
//!
//! Application layer use cases for market sniping.
//! Encapsulates business logic and orchestrates infrastructure.

use crate::domain::SniperMarket;
use crate::infrastructure::{SharedOraclePrices, SniperConfig, BotConfig, EventsConfig};
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Market Tracker Service
///
/// Orchestrates market tracking.
/// This is the application layer abstraction over infrastructure.
#[derive(Clone, Default)]
pub struct MarketTrackerService;

impl MarketTrackerService {
    /// Create a new market tracker service
    pub fn new() -> Self {
        Self
    }

    /// Spawn a tracker for a specific market
    ///
    /// This is the use case for tracking a market's orderbook.
    /// When best_ask = 1 is detected, it logs the event with market details.
    pub async fn track_market(
        &self,
        market: &SniperMarket,
        shutdown_flag: Arc<AtomicBool>,
        oracle_prices: Option<SharedOraclePrices>,
    ) -> Result<()> {
        // Delegate to infrastructure layer for WebSocket connection
        crate::infrastructure::spawn_market_tracker(
            market.id.clone(),
            market.question.clone(),
            market.slug.clone(),
            market.token_ids.clone(),
            market.outcomes.clone(),
            market.resolution_time_str.clone(),
            shutdown_flag,
            oracle_prices,
        )
        .await
    }
}

/// Configuration Service.
///
/// Handles loading and validation of YAML configuration files.
/// Provides methods to load sniper, events, and bot configurations.
pub struct ConfigService;

impl ConfigService {
    /// Load sniper configuration
    pub fn load_sniper_config(path: &str) -> Result<SniperConfig> {
        Ok(SniperConfig::load(path)?)
    }

    /// Load events configuration
    pub fn load_events_config(path: &str) -> Result<EventsConfig> {
        Ok(EventsConfig::load(path)?)
    }

    /// Load bot configuration.
    ///
    /// **Deprecated**: Use `load_sniper_config` or `load_events_config` instead.
    /// This method exists for backward compatibility with older configuration files.
    #[deprecated(since = "0.2.0", note = "Use load_sniper_config or load_events_config instead")]
    pub fn load_bot_config(path: &str) -> Result<BotConfig> {
        Ok(BotConfig::load(path)?)
    }
}
