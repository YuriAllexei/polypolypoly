//! Strategies configuration
//!
//! Configuration for the pluggable strategy system.

use super::{ConfigError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

/// Main strategies configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategiesConfig {
    /// Log level (error, warn, info, debug, trace)
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Components configuration (shared infrastructure)
    #[serde(default)]
    pub components: ComponentsConfig,

    /// Up or Down strategy configuration
    #[serde(default)]
    pub up_or_down: UpOrDownConfig,

    /// Sports Sniping strategy configuration
    #[serde(default)]
    pub sports_sniping: SportsSnipingConfig,
}

/// Components configuration (shared infrastructure)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentsConfig {
    /// Balance manager configuration
    #[serde(default)]
    pub balance_manager: BalanceManagerConfig,
}

impl Default for ComponentsConfig {
    fn default() -> Self {
        Self {
            balance_manager: BalanceManagerConfig::default(),
        }
    }
}

/// Balance manager configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceManagerConfig {
    /// Halt threshold as fraction (e.g., 0.10 = 10% of peak)
    #[serde(default = "default_balance_threshold")]
    pub threshold: f64,
}

fn default_balance_threshold() -> f64 {
    0.10 // 10%
}

impl Default for BalanceManagerConfig {
    fn default() -> Self {
        Self {
            threshold: default_balance_threshold(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Configuration for the Up or Down strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpOrDownConfig {
    /// Time window in seconds before market ends to trigger alert
    #[serde(default = "default_delta_t")]
    pub delta_t_seconds: f64,

    /// How often to poll the database for new markets (seconds)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: f64,

    /// Minimum price difference in basis points between oracle and market for trade signal
    #[serde(default = "default_oracle_bps_threshold")]
    pub oracle_bps_price_threshold: f64,

    /// Minimum threshold in seconds for no-asks condition (when close to market end)
    #[serde(default = "default_threshold_min")]
    pub threshold_min: f64,

    /// Maximum threshold in seconds for no-asks condition (when far from market end)
    #[serde(default = "default_threshold_max")]
    pub threshold_max: f64,

    /// Decay time constant in seconds for exponential threshold decay
    #[serde(default = "default_threshold_tau")]
    pub threshold_tau: f64,

    /// Fraction of collateral to use per order (e.g., 0.10 = 10%)
    #[serde(default = "default_order_pct")]
    pub order_pct_of_collateral: f64,

    /// Guardian safety threshold in basis points - cancels orders if oracle is within this
    /// distance of price_to_beat. Never bypassed, runs until market timer ends.
    #[serde(default = "default_guardian_safety_bps")]
    pub guardian_safety_bps: f64,
}

fn default_order_pct() -> f64 {
    0.10 // 10% default
}

fn default_guardian_safety_bps() -> f64 {
    2.0 // 2 basis points (0.02%)
}

fn default_delta_t() -> f64 {
    300.0 // 5 minutes
}

fn default_poll_interval() -> f64 {
    60.0 // 1 minute
}

fn default_oracle_bps_threshold() -> f64 {
    50.0 // 50 basis points (0.5%)
}

fn default_threshold_min() -> f64 {
    0.5 // 0.5 seconds (aggressive, near market end)
}

fn default_threshold_max() -> f64 {
    10.0 // 10 seconds (conservative, far from market end)
}

fn default_threshold_tau() -> f64 {
    30.0 // 30 seconds decay time constant
}

// Sports Sniping defaults
fn default_sports_poll_interval() -> f64 {
    1.0 // 1 second
}

fn default_sports_order_pct() -> f64 {
    0.10 // 10% of collateral per order
}

fn default_sports_bid_threshold() -> f64 {
    0.80 // Minimum best_bid price to execute
}

/// Configuration for the Sports Sniping strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SportsSnipingConfig {
    /// How often to poll for updates (seconds)
    #[serde(default = "default_sports_poll_interval")]
    pub poll_interval_secs: f64,

    /// Enable/disable the strategy (placeholder for future use)
    #[serde(default)]
    pub enabled: bool,

    /// Percentage of collateral to use per order (e.g., 0.10 = 10%)
    #[serde(default = "default_sports_order_pct")]
    pub order_pct_of_collateral: f64,

    /// Minimum best_bid price required to execute an order
    #[serde(default = "default_sports_bid_threshold")]
    pub bid_threshold: f64,
}

impl Default for SportsSnipingConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_sports_poll_interval(),
            enabled: true,
            order_pct_of_collateral: default_sports_order_pct(),
            bid_threshold: default_sports_bid_threshold(),
        }
    }
}

impl SportsSnipingConfig {
    fn validate(&self) -> Result<()> {
        if self.poll_interval_secs <= 0.0 {
            return Err(ConfigError::ValidationError(
                "sports_sniping.poll_interval_secs must be greater than 0".to_string(),
            ));
        }
        if self.order_pct_of_collateral <= 0.0 || self.order_pct_of_collateral > 1.0 {
            return Err(ConfigError::ValidationError(
                "sports_sniping.order_pct_of_collateral must be between 0 and 1".to_string(),
            ));
        }
        if self.bid_threshold <= 0.0 || self.bid_threshold >= 1.0 {
            return Err(ConfigError::ValidationError(
                "sports_sniping.bid_threshold must be between 0 and 1".to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for UpOrDownConfig {
    fn default() -> Self {
        Self {
            delta_t_seconds: default_delta_t(),
            poll_interval_secs: default_poll_interval(),
            oracle_bps_price_threshold: default_oracle_bps_threshold(),
            threshold_min: default_threshold_min(),
            threshold_max: default_threshold_max(),
            threshold_tau: default_threshold_tau(),
            order_pct_of_collateral: default_order_pct(),
            guardian_safety_bps: default_guardian_safety_bps(),
        }
    }
}

impl Default for StrategiesConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            components: ComponentsConfig::default(),
            up_or_down: UpOrDownConfig::default(),
            sports_sniping: SportsSnipingConfig::default(),
        }
    }
}

impl StrategiesConfig {
    /// Load configuration from YAML file
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        let yaml_content = std::fs::read_to_string(config_path)?;
        let config: StrategiesConfig = serde_yaml::from_str(&yaml_content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values
    fn validate(&self) -> Result<()> {
        // Validate log_level
        let valid_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_levels.contains(&self.log_level.to_lowercase().as_str()) {
            return Err(ConfigError::ValidationError(format!(
                "log_level must be one of: {}",
                valid_levels.join(", ")
            )));
        }

        // Validate up_or_down config
        self.up_or_down.validate()?;

        // Validate sports_sniping config
        self.sports_sniping.validate()?;

        Ok(())
    }

    /// Log configuration summary
    pub fn log(&self) {
        info!("Strategies Configuration:");
        info!("  Log level: {}", self.log_level);
        info!("Components:");
        info!(
            "  Balance manager threshold: {:.0}%",
            self.components.balance_manager.threshold * 100.0
        );
        info!("Up or Down Strategy:");
        info!("  Delta T: {} seconds", self.up_or_down.delta_t_seconds);
        info!(
            "  Poll interval: {} seconds",
            self.up_or_down.poll_interval_secs
        );
        info!(
            "  Oracle BPS threshold: {} bps",
            self.up_or_down.oracle_bps_price_threshold
        );
        info!(
            "  Threshold min: {} seconds",
            self.up_or_down.threshold_min
        );
        info!(
            "  Threshold max: {} seconds",
            self.up_or_down.threshold_max
        );
        info!(
            "  Threshold tau: {} seconds",
            self.up_or_down.threshold_tau
        );
        info!(
            "  Order pct of collateral: {:.0}%",
            self.up_or_down.order_pct_of_collateral * 100.0
        );
        info!("Sports Sniping Strategy:");
        info!(
            "  Poll interval: {} seconds",
            self.sports_sniping.poll_interval_secs
        );
        info!("  Enabled: {}", self.sports_sniping.enabled);
    }
}

impl UpOrDownConfig {
    fn validate(&self) -> Result<()> {
        if self.delta_t_seconds <= 0.0 {
            return Err(ConfigError::ValidationError(
                "up_or_down.delta_t_seconds must be greater than 0".to_string(),
            ));
        }

        if self.poll_interval_secs <= 0.0 {
            return Err(ConfigError::ValidationError(
                "up_or_down.poll_interval_secs must be greater than 0".to_string(),
            ));
        }

        if self.oracle_bps_price_threshold < 0.0 {
            return Err(ConfigError::ValidationError(
                "up_or_down.oracle_bps_price_threshold must be >= 0".to_string(),
            ));
        }

        if self.threshold_min <= 0.0 {
            return Err(ConfigError::ValidationError(
                "up_or_down.threshold_min must be greater than 0".to_string(),
            ));
        }

        if self.threshold_max <= 0.0 {
            return Err(ConfigError::ValidationError(
                "up_or_down.threshold_max must be greater than 0".to_string(),
            ));
        }

        if self.threshold_min >= self.threshold_max {
            return Err(ConfigError::ValidationError(
                "up_or_down.threshold_min must be less than threshold_max".to_string(),
            ));
        }

        if self.threshold_tau <= 0.0 {
            return Err(ConfigError::ValidationError(
                "up_or_down.threshold_tau must be greater than 0".to_string(),
            ));
        }

        Ok(())
    }
}
