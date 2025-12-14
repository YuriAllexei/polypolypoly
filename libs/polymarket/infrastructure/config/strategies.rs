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

    /// Up or Down strategy configuration
    #[serde(default)]
    pub up_or_down: UpOrDownConfig,
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

impl Default for UpOrDownConfig {
    fn default() -> Self {
        Self {
            delta_t_seconds: default_delta_t(),
            poll_interval_secs: default_poll_interval(),
            oracle_bps_price_threshold: default_oracle_bps_threshold(),
            threshold_min: default_threshold_min(),
            threshold_max: default_threshold_max(),
            threshold_tau: default_threshold_tau(),
        }
    }
}

impl Default for StrategiesConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            up_or_down: UpOrDownConfig::default(),
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

        Ok(())
    }

    /// Log configuration summary
    pub fn log(&self) {
        info!("Strategies Configuration:");
        info!("  Log level: {}", self.log_level);
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
