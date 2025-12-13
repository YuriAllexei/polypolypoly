//! Sniper Use Cases
//!
//! Application layer use cases for market sniping.
//! Encapsulates business logic and orchestrates infrastructure.

use crate::infrastructure::{BotConfig, EventsConfig, SniperConfig};
use anyhow::Result;

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
    #[deprecated(
        since = "0.2.0",
        note = "Use load_sniper_config or load_events_config instead"
    )]
    pub fn load_bot_config(path: &str) -> Result<BotConfig> {
        Ok(BotConfig::load(path)?)
    }
}
