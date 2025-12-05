//! Sniper Use Cases
//!
//! Application layer use cases for market sniping.
//! Encapsulates business logic and orchestrates infrastructure.

use crate::domain::SniperMarket;
use crate::infrastructure::{MarketDatabase, SniperConfig, BotConfig, EventsConfig};
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Market Tracker Service
///
/// Orchestrates market tracking and opportunity detection.
/// This is the application layer abstraction over infrastructure.
#[derive(Clone)]
pub struct MarketTrackerService {
    database: Arc<MarketDatabase>,
    probability_threshold: f64,
}

impl MarketTrackerService {
    /// Create a new market tracker service
    pub fn new(database: Arc<MarketDatabase>, probability_threshold: f64) -> Self {
        Self {
            database,
            probability_threshold,
        }
    }

    /// Spawn a tracker for a specific market
    ///
    /// This is the use case for tracking a market's orderbook
    /// and detecting arbitrage opportunities.
    pub async fn track_market(
        &self,
        market: &SniperMarket,
        shutdown_flag: Arc<AtomicBool>,
        event_id: Option<String>,
    ) -> Result<()> {
        // Delegate to infrastructure layer for WebSocket connection
        crate::infrastructure::spawn_market_tracker(
            market.id.clone(),
            market.token_ids.clone(),
            market.outcomes.clone(),
            market.resolution_time_str.clone(),
            shutdown_flag,
            Arc::clone(&self.database),
            self.probability_threshold,
            event_id,
        )
        .await
    }
}

/// Configuration Service
///
/// Handles loading and validation of application configuration.
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

    /// Load bot configuration (legacy - use sniper or events config instead)
    pub fn load_bot_config(path: &str) -> Result<BotConfig> {
        Ok(BotConfig::load(path)?)
    }
}
